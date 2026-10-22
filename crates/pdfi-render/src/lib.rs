//! The raster renderer: runs a content again through the interpreter and
//! paints paths, strokes, clips, images and text (glyph outlines of the
//! embedded or substituted font program; Type 3 glyphs through their
//! procedures) with tiny-skia. Shadings, patterns, soft masks and blend
//! modes come later (what a pass leaves out or approximates is reported as
//! notes).
//!
//! Coordinates: a content's *view* is what is shown, in points with the
//! origin at the top left and y going down (pages rotated by `/Rotate`).
//! A render request asks for a tile of the view at some scale (pixels per
//! point). [`hit_test`] answers which operations paint at a view point.
//!
//! [`DocRender`] keeps decoded images per document; tiles of one page can
//! be rendered on several threads at once.

pub mod color;
pub mod function;
pub mod image;
mod raster;
pub mod shading;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use pdfi_content::geom::{Matrix, mul, translate};
use pdfi_content::interp::{ElementKind, Kind};
use pdfi_content::{Analysis, Content};
use pdfi_model::Rect;
use pdfi_syntax::{ObjId, PdfFile, Stream};
use tiny_skia::Pixmap;

use crate::image::{Decoded, ImageError};
use crate::raster::{Mode, Raster, shrink};
use crate::shading::Shading;

/// Longest side of a rendered bitmap.
pub const MAX_SIDE: u32 = 16384;
/// Pixels of one rendered bitmap.
pub const MAX_PIXELS: u64 = 1 << 24;
/// Shadings kept parsed per document.
const SHADINGS: usize = 256;
/// Decoded image bytes kept per document.
const IMAGE_CACHE: usize = 192 << 20;
/// Size of a content without bounds that paints nothing.
const EMPTY_SIDE: f64 = 100.0;
/// Pixels painted around a tile and dropped: anti-aliasing leaks coverage
/// into the last pixels where a path is cut by the bitmap's edge, and
/// tiles must join seamlessly.
const MARGIN: u32 = 2;

/// What is shown of a content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// In points.
    pub width: f64,
    pub height: f64,
    /// The content's space to the view (top left origin, y down).
    pub matrix: Matrix,
}

/// The view of an analyzed content: a page's visible area turned by its
/// `/Rotate`, a form's `/BBox`, or what a stream paints.
pub fn view(a: &Analysis) -> View {
    let b = a.bounds.unwrap_or_else(|| {
        a.elements
            .iter()
            .filter(|e| !e.hidden && e.kind != ElementKind::Clip)
            .fold(None, |r: Option<Rect>, e| {
                Some(r.map_or(e.bbox, |r| Rect {
                    x0: r.x0.min(e.bbox.x0),
                    y0: r.y0.min(e.bbox.y0),
                    x1: r.x1.max(e.bbox.x1),
                    y1: r.y1.max(e.bbox.y1),
                }))
            })
            .unwrap_or(Rect {
                x0: 0.0,
                y0: 0.0,
                x1: EMPTY_SIDE,
                y1: EMPTY_SIDE,
            })
    });
    let (w, h) = ((b.x1 - b.x0).max(1.0), (b.y1 - b.y0).max(1.0));
    let rotate = if a.kind == Kind::Page { a.rotate } else { 0 };
    let (matrix, width, height) = match rotate {
        90 => ([0.0, 1.0, 1.0, 0.0, -b.y0, -b.x0], h, w),
        180 => ([-1.0, 0.0, 0.0, 1.0, b.x1, -b.y0], w, h),
        270 => ([0.0, -1.0, -1.0, 0.0, b.y1, b.x1], h, w),
        _ => ([1.0, 0.0, 0.0, -1.0, -b.x0, b.y1], w, h),
    };
    View {
        width,
        height,
        matrix,
    }
}

impl View {
    /// Size in pixels at `scale`.
    pub fn pixels(&self, scale: f64) -> (u64, u64) {
        let px = |v: f64| ((v * scale).ceil().max(1.0)).min(u32::MAX as f64) as u64;
        (px(self.width), px(self.height))
    }
}

/// A region of the bitmap of the whole view, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Layer {
    /// The content, on white for pages and transparent for streams.
    Content,
    /// Only these operations, in one opaque color on transparent.
    Highlight(Vec<u32>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Pixels per point.
    pub scale: f64,
    /// `None`: the whole view.
    pub tile: Option<Tile>,
    /// Run operations up to and including this one.
    pub until: Option<usize>,
    pub layer: Layer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub width: u32,
    pub height: u32,
    /// RGBA, not premultiplied, row by row.
    pub rgba: Vec<u8>,
    /// Operations run.
    pub ops: usize,
    pub cancelled: bool,
    /// What was approximated or left out, with how often.
    pub notes: Vec<(String, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid(String),
    TooLarge(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Invalid(m) | Error::TooLarge(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

/// A shading as parsed (or why it cannot be).
pub(crate) type Parsed = Arc<Result<Shading, String>>;

type ImageCell = Arc<OnceLock<Arc<Result<Decoded, ImageError>>>>;

#[derive(Default)]
struct Images {
    cells: HashMap<(ObjId, u32), ImageCell>,
    /// Insertion order, for eviction.
    order: VecDeque<((ObjId, u32), usize)>,
    bytes: usize,
}

/// Per-document renderer state: decoded images (whole and reduced), ICC
/// profiles.
#[derive(Default)]
pub struct DocRender {
    images: Mutex<Images>,
    pub(crate) profiles: color::Profiles,
    /// By object (`true`: the shading inside a pattern object).
    shadings: Mutex<HashMap<(ObjId, bool), Parsed>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl DocRender {
    pub fn new() -> Self {
        Self::default()
    }

    /// A shading, parsed once.
    pub(crate) fn shading(&self, key: (ObjId, bool), parse: impl FnOnce() -> Parsed) -> Parsed {
        if let Some(s) = lock(&self.shadings).get(&key) {
            return s.clone();
        }
        let s = parse();
        let mut g = lock(&self.shadings);
        if g.len() >= SHADINGS {
            g.clear();
        }
        g.insert(key, s.clone());
        s
    }

    /// An image XObject decoded and reduced by `k` (cached; decoded once
    /// even when several threads ask at the same time).
    pub(crate) fn image(
        &self,
        file: &PdfFile,
        id: ObjId,
        s: &Stream,
        k: u32,
    ) -> Arc<Result<Decoded, ImageError>> {
        let cell = lock(&self.images).cells.entry((id, k)).or_default().clone();
        let mut fresh = false;
        let v = cell
            .get_or_init(|| {
                fresh = true;
                Arc::new(if k <= 1 || image::reduces(file, &s.dict) {
                    image::decode_xobject(file, &self.profiles, Some(id), s, k)
                } else {
                    match &*self.image(file, id, s, 1) {
                        Ok(d) => Ok(shrink(
                            Decoded {
                                pixmap: d.pixmap.clone(),
                                ..*d
                            },
                            k,
                        )),
                        Err(e) => Err(e.clone()),
                    }
                })
            })
            .clone();
        if fresh {
            let size = v.as_ref().as_ref().map_or(0, Decoded::bytes);
            let mut g = lock(&self.images);
            g.order.push_back(((id, k), size));
            g.bytes += size;
            while g.bytes > IMAGE_CACHE && g.order.len() > 1 {
                let Some((key, n)) = g.order.pop_front() else {
                    break;
                };
                g.cells.remove(&key);
                g.bytes -= n;
            }
        }
        v
    }
}

/// Render (part of) an analyzed content.
pub fn render(
    cache: &DocRender,
    content: &Content<'_>,
    a: &Analysis,
    o: &Options,
    cancel: &dyn Fn() -> bool,
) -> Result<Output, Error> {
    if !(o.scale > 0.0 && o.scale.is_finite()) {
        return Err(Error::Invalid("scale must be a positive number".into()));
    }
    let v = view(a);
    let (fw, fh) = v.pixels(o.scale);
    let t = o.tile.unwrap_or(Tile {
        x: 0,
        y: 0,
        width: fw.min(u32::MAX as u64) as u32,
        height: fh.min(u32::MAX as u64) as u32,
    });
    if t.x as u64 >= fw || t.y as u64 >= fh || t.width == 0 || t.height == 0 {
        return Err(Error::Invalid(format!(
            "the tile is outside the view ({fw} × {fh} pixels)"
        )));
    }
    let width = t.width.min((fw - t.x as u64) as u32);
    let height = t.height.min((fh - t.y as u64) as u32);
    if width > MAX_SIDE || height > MAX_SIDE || width as u64 * height as u64 > MAX_PIXELS {
        return Err(Error::TooLarge(format!(
            "at most {MAX_SIDE} pixels a side and {MAX_PIXELS} pixels per bitmap"
        )));
    }
    let pix = Pixmap::new(width + 2 * MARGIN, height + 2 * MARGIN)
        .ok_or_else(|| Error::TooLarge("bitmap".into()))?;
    // Pages are painted on a transparent backdrop and then put on white
    // paper (blend modes see nothing where nothing was painted).
    let paper = o.layer == Layer::Content && a.kind == Kind::Page;
    let mode = match &o.layer {
        Layer::Content => Mode::Paint,
        Layer::Highlight(ops) => {
            let mut ops = ops.clone();
            ops.sort_unstable();
            Mode::Highlight(ops)
        }
    };
    let base = mul(
        &mul(&v.matrix, &[o.scale, 0.0, 0.0, o.scale, 0.0, 0.0]),
        &translate(MARGIN as f64 - t.x as f64, MARGIN as f64 - t.y as f64),
    );
    let mut dev = Raster::new(content, cache, cancel, pix, base, MARGIN, mode);
    let drawn = content.draw(a, o.until, &mut dev, cancel);
    let notes = dev.notes.into_iter().collect();
    if paper {
        for p in dev.pix.pixels_mut() {
            let w = 255 - p.alpha();
            *p = tiny_skia::PremultipliedColorU8::from_rgba(
                p.red() + w,
                p.green() + w,
                p.blue() + w,
                255,
            )
            .unwrap_or(*p);
        }
    }
    let full = dev.pix.take_demultiplied();
    let stride = (width + 2 * MARGIN) as usize * 4;
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for row in full
        .chunks_exact(stride)
        .skip(MARGIN as usize)
        .take(height as usize)
    {
        rgba.extend_from_slice(&row[MARGIN as usize * 4..][..width as usize * 4]);
    }
    Ok(Output {
        width,
        height,
        rgba,
        ops: drawn.ops,
        cancelled: drawn.cancelled,
        notes,
    })
}

/// Operations that paint within `radius` of view point `(x, y)`, topmost
/// first (forms count as the `Do` that calls them).
pub fn hit_test(
    cache: &DocRender,
    content: &Content<'_>,
    a: &Analysis,
    (x, y): (f64, f64),
    radius: f64,
    until: Option<usize>,
    cancel: &dyn Fn() -> bool,
) -> Vec<u32> {
    // A 3 × 3 pixmap over the square around the point.
    let r = if radius > 0.0 && radius.is_finite() {
        radius
    } else {
        1.0
    };
    let s = 3.0 / (2.0 * r);
    let base = mul(
        &mul(&view(a).matrix, &translate(-(x - r), -(y - r))),
        &[s, 0.0, 0.0, s, 0.0, 0.0],
    );
    let pix = Pixmap::new(3, 3).expect("probe pixmap");
    let mut dev = Raster::new(content, cache, cancel, pix, base, 0, Mode::Probe);
    content.draw(a, until, &mut dev, cancel);
    let mut hits = dev.hits;
    hits.reverse();
    let mut seen = std::collections::HashSet::new();
    hits.retain(|h| seen.insert(*h));
    hits
}

#[cfg(feature = "png")]
impl Output {
    /// The bitmap as a PNG file.
    pub fn png(&self) -> Result<Vec<u8>, String> {
        let size = tiny_skia::IntSize::from_wh(self.width, self.height).ok_or("empty bitmap")?;
        let mut data = self.rgba.clone();
        for px in data.chunks_exact_mut(4) {
            let a = px[3] as u32;
            for c in &mut px[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
        let pix = Pixmap::from_vec(data, size).ok_or("bitmap size")?;
        pix.encode_png().map_err(|e| e.to_string())
    }
}
