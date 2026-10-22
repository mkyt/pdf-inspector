//! The raster device: paints what the interpreter hands over onto a
//! tiny-skia pixmap.
//!
//! Three modes share it: painting a content, painting a highlight layer
//! (only some operations, in one opaque color), and probing (which
//! operations cover a few pixels around a point, for hit testing).

use std::collections::BTreeMap;
use std::sync::Arc;

use pdfi_content::device::{ClipPath, Device, GlyphRun, ImageData, PathEl};
use pdfi_content::font::GlyphSource;
use pdfi_content::geom::{Matrix, apply, mul, scale_of};
use pdfi_content::interp::{Color, GState, Resources};
use pdfi_syntax::{Object, PdfFile};
use tiny_skia::{
    FillRule, FilterQuality, LineCap, LineJoin, Mask, Paint, PathBuilder, Pattern, Pixmap,
    SpreadMode, Stroke, StrokeDash, Transform,
};

use crate::DocRender;
use crate::color::{self, Approx, Space};
use crate::image::{self, Decoded};

/// Clip masks kept per pass (saved states switch between a few).
const MASKS: usize = 8;
/// Color spaces kept per pass.
const SPACES: usize = 32;
/// Opacity of glyph boxes (fonts without glyphs).
const TEXT_ALPHA: f32 = 0.35;
/// Strokes of a highlight are at least this wide (pixels).
const HIGHLIGHT_WIDTH: f64 = 3.0;

pub enum Mode {
    Paint,
    /// Only these operations (sorted), in one color.
    Highlight(Vec<u32>),
    /// Record the operations that cover any pixel.
    Probe,
}

pub struct Raster<'a> {
    file: &'a PdfFile,
    cache: &'a DocRender,
    pub pix: Pixmap,
    /// The content's space to pixels.
    base: Matrix,
    /// The pixels kept (`x0, y0, x1, y1`); the rest is a margin that is
    /// dropped, so clips covering this need no mask.
    kept: (f64, f64, f64, f64),
    mode: Mode,
    /// The current operation is painted (highlight filter).
    active: bool,
    op: u32,
    /// Probe: operations that covered a pixel, in painting order.
    pub hits: Vec<u32>,
    /// `None`: the clip covers the whole pixmap.
    masks: Vec<(Arc<ClipPath>, Option<Mask>)>,
    spaces: Vec<(Arc<Object>, Arc<Space>)>,
    /// What was approximated or left out, and how often.
    pub notes: BTreeMap<String, u32>,
}

pub fn to_transform(m: &Matrix) -> Transform {
    Transform::from_row(
        m[0] as f32,
        m[1] as f32,
        m[2] as f32,
        m[3] as f32,
        m[4] as f32,
        m[5] as f32,
    )
}

fn invert(m: &Matrix) -> Option<Matrix> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)])
}

/// A tiny-skia path from path elements, transformed by `m`.
fn build(els: &[PathEl], m: Option<&Matrix>) -> Option<tiny_skia::Path> {
    let p = |x: f64, y: f64| {
        let (x, y) = m.map_or((x, y), |m| apply(m, x, y));
        (x as f32, y as f32)
    };
    let mut b = PathBuilder::with_capacity(els.len(), els.len() * 3);
    for e in els {
        match *e {
            PathEl::Move(x, y) => {
                let (x, y) = p(x, y);
                b.move_to(x, y);
            }
            PathEl::Line(x, y) => {
                let (x, y) = p(x, y);
                b.line_to(x, y);
            }
            PathEl::Curve(x1, y1, x2, y2, x3, y3) => {
                let ((x1, y1), (x2, y2), (x3, y3)) = (p(x1, y1), p(x2, y2), p(x3, y3));
                b.cubic_to(x1, y1, x2, y2, x3, y3);
            }
            PathEl::Close => b.close(),
        }
    }
    b.finish()
}

/// Bounding box of path elements after `m` (control points included).
fn bounds(els: &[PathEl], m: &Matrix) -> Option<(f64, f64, f64, f64)> {
    let mut b: Option<(f64, f64, f64, f64)> = None;
    let mut add = |x: f64, y: f64| {
        let (x, y) = apply(m, x, y);
        b = Some(match b {
            None => (x, y, x, y),
            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        });
    };
    for e in els {
        match *e {
            PathEl::Move(x, y) | PathEl::Line(x, y) => add(x, y),
            PathEl::Curve(x1, y1, x2, y2, x3, y3) => {
                add(x1, y1);
                add(x2, y2);
                add(x3, y3);
            }
            PathEl::Close => {}
        }
    }
    b
}

/// An axis-aligned rectangle (after `m`), as `(x0, y0, x1, y1)`.
fn as_rect(els: &[PathEl], m: &Matrix) -> Option<(f64, f64, f64, f64)> {
    let mut pts = Vec::with_capacity(5);
    for e in els {
        match *e {
            PathEl::Move(x, y) | PathEl::Line(x, y) => {
                if !pts.is_empty() && matches!(e, PathEl::Move(..)) {
                    return None;
                }
                pts.push(apply(m, x, y));
            }
            PathEl::Curve(..) => return None,
            PathEl::Close => {}
        }
    }
    if pts.len() == 5 && pts[4] == pts[0] {
        pts.pop();
    }
    if pts.len() != 4 {
        return None;
    }
    let xs = (pts[0].0, pts[2].0);
    let ys = (pts[0].1, pts[2].1);
    let corner = |(x, y): (f64, f64)| (x == xs.0 || x == xs.1) && (y == ys.0 || y == ys.1);
    let edges = pts.iter().all(|&p| corner(p))
        && (pts[1].0 == pts[0].0 || pts[1].1 == pts[0].1)
        && (pts[3].0 == pts[0].0 || pts[3].1 == pts[0].1);
    edges.then(|| {
        (
            xs.0.min(xs.1),
            ys.0.min(ys.1),
            xs.0.max(xs.1),
            ys.0.max(ys.1),
        )
    })
}

const UNIT: [PathEl; 4] = [
    PathEl::Move(0.0, 0.0),
    PathEl::Line(1.0, 0.0),
    PathEl::Line(1.0, 1.0),
    PathEl::Line(0.0, 1.0),
];

fn rgb_color(rgb: [f32; 3], alpha: f32) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba(rgb[0], rgb[1], rgb[2], alpha.clamp(0.0, 1.0))
        .unwrap_or(tiny_skia::Color::BLACK)
}

fn highlight() -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(255, 64, 0, 255)
}

impl<'a> Raster<'a> {
    pub fn new(
        file: &'a PdfFile,
        cache: &'a DocRender,
        pix: Pixmap,
        base: Matrix,
        margin: u32,
        mode: Mode,
    ) -> Self {
        let active = !matches!(mode, Mode::Highlight(_));
        let m = margin as f64;
        let kept = (m, m, pix.width() as f64 - m, pix.height() as f64 - m);
        Raster {
            file,
            cache,
            pix,
            base,
            kept,
            mode,
            active,
            op: 0,
            hits: Vec::new(),
            masks: Vec::new(),
            spaces: Vec::new(),
            notes: BTreeMap::new(),
        }
    }

    /// Whether a device-space box misses the bitmap (drawing it would
    /// only cost time: tiles skip most of a page this way).
    fn misses(&self, b: Option<(f64, f64, f64, f64)>, grow: f64) -> bool {
        let (w, h) = (self.pix.width() as f64, self.pix.height() as f64);
        match b {
            Some((x0, y0, x1, y1)) => {
                x1 + grow < 0.0 || y1 + grow < 0.0 || x0 - grow > w || y0 - grow > h
            }
            None => true,
        }
    }

    fn note(&mut self, what: impl Into<String>) {
        *self.notes.entry(what.into()).or_default() += 1;
    }

    /// The index of the mask for `clip` in `masks` (built if needed, from
    /// the nearest ancestor that has one: long chains of clips are built
    /// one path at a time).
    fn mask(&mut self, clip: &Option<Arc<ClipPath>>) -> Option<usize> {
        let clip = clip.as_ref()?;
        let cached = |c: &ClipPath, masks: &[(Arc<ClipPath>, Option<Mask>)]| {
            masks.iter().position(|(m, _)| std::ptr::eq(&**m, c))
        };
        if let Some(k) = cached(clip, &self.masks) {
            return Some(k);
        }
        // The paths to apply, innermost first, and the mask to start from.
        let mut todo = Vec::new();
        let mut start: Option<Mask> = None;
        let mut c = Some(&**clip);
        while let Some(p) = c {
            if let Some(k) = cached(p, &self.masks) {
                start = self.masks[k].1.clone();
                break;
            }
            todo.push(p);
            c = p.parent.as_deref();
        }
        let (w, h) = (self.pix.width(), self.pix.height());
        let ts = to_transform(&self.base);
        let mut mask = start;
        for p in todo.into_iter().rev() {
            if let Some((x0, y0, x1, y1)) = as_rect(&p.path, &self.base) {
                let k = self.kept;
                if x0 <= k.0 && y0 <= k.1 && x1 >= k.2 && y1 >= k.3 {
                    continue;
                }
            }
            let rule = if p.even_odd {
                FillRule::EvenOdd
            } else {
                FillRule::Winding
            };
            let m = mask.get_or_insert_with(|| {
                let mut m = Mask::new(w, h).expect("pixmap size");
                m.data_mut().fill(255);
                m
            });
            match build(&p.path, None) {
                Some(path) => m.intersect_path(&path, rule, true, ts),
                None => m.clear(),
            }
        }
        if self.masks.len() >= MASKS {
            self.masks.remove(0);
        }
        self.masks.push((clip.clone(), mask));
        Some(self.masks.len() - 1)
    }

    /// Paint with the clip of `g`; in probe mode, record whether anything
    /// was covered.
    fn draw(&mut self, g: &GState, f: impl FnOnce(&mut Pixmap, Option<&Mask>)) {
        if let Mode::Paint = self.mode {
            if g.soft_mask.is_some() {
                self.note("soft masks are ignored");
            }
            if g.blend != "Normal" && g.blend != "Compatible" {
                self.note(format!("blend mode /{} is drawn as /Normal", g.blend));
            }
        }
        let k = self.mask(&g.clip_path);
        let probe = matches!(self.mode, Mode::Probe);
        if probe {
            self.pix.fill(tiny_skia::Color::TRANSPARENT);
        }
        let mask = k.and_then(|k| self.masks[k].1.as_ref());
        f(&mut self.pix, mask);
        if probe
            && self.pix.pixels().iter().any(|p| p.alpha() > 0)
            && self.hits.last() != Some(&self.op)
        {
            self.hits.push(self.op);
        }
    }

    fn space(&mut self, def: &Arc<Object>) -> Arc<Space> {
        if let Some((_, s)) = self.spaces.iter().find(|(d, _)| Arc::ptr_eq(d, def)) {
            return s.clone();
        }
        let s = Arc::new(color::parse(self.file, def));
        if self.spaces.len() >= SPACES {
            self.spaces.remove(0);
        }
        self.spaces.push((def.clone(), s.clone()));
        s
    }

    /// A color as RGB (`None`: paints nothing).
    fn rgb(&mut self, c: &Color) -> Option<[f32; 3]> {
        if c.family == "Pattern" {
            self.note("pattern colors are drawn as gray");
            return Some([0.5; 3]);
        }
        let space = match c.family.as_str() {
            "DeviceGray" => Arc::new(Space::Gray),
            "DeviceRGB" => Arc::new(Space::Rgb),
            "DeviceCMYK" => Arc::new(Space::Cmyk),
            _ => match &c.def {
                Some(d) => self.space(d),
                None => Arc::new(match c.n {
                    Some(3) => Space::Rgb,
                    Some(4) => Space::Cmyk,
                    _ => Space::Gray,
                }),
            },
        };
        let mut approx = Approx::default();
        let out = space.to_rgb(&c.components, &mut approx);
        if approx.tint {
            self.note("Separation / DeviceN colors are approximated");
        }
        out
    }

    /// The paint for a color with an alpha, by mode.
    fn paint_for(&mut self, c: &Color, alpha: f64) -> Option<Paint<'static>> {
        let color = match self.mode {
            Mode::Probe => tiny_skia::Color::BLACK,
            Mode::Highlight(_) => highlight(),
            Mode::Paint => rgb_color(self.rgb(c)?, alpha as f32),
        };
        let mut p = Paint::default();
        p.set_color(color);
        Some(p)
    }

    fn stroke_style(&self, g: &GState, ts: &Matrix) -> Stroke {
        let scale = scale_of(ts);
        let mut width = g.line_width.abs();
        if let Mode::Highlight(_) = self.mode {
            if scale > 0.0 {
                width = width.max(HIGHLIGHT_WIDTH / scale);
            }
        }
        // Thinner than a pixel: a hairline, as viewers do.
        if width * scale < 1.0 {
            width = 0.0;
        }
        let dash = (!g.dash.is_empty()
            && g.dash.iter().all(|&d| d >= 0.0)
            && g.dash.iter().sum::<f64>() > 0.0)
            .then(|| {
                let mut a: Vec<f32> = g.dash.iter().map(|&d| d as f32).collect();
                if a.len() % 2 == 1 {
                    a.extend_from_within(..);
                }
                StrokeDash::new(a, g.dash_phase as f32)
            })
            .flatten();
        Stroke {
            width: width as f32,
            miter_limit: g.miter_limit.max(1.0) as f32,
            line_cap: match g.line_cap {
                1 => LineCap::Round,
                2 => LineCap::Square,
                _ => LineCap::Butt,
            },
            line_join: match g.line_join {
                1 => LineJoin::Round,
                2 => LineJoin::Bevel,
                _ => LineJoin::Miter,
            },
            dash,
        }
    }

    fn unit_square(&mut self, g: &GState, color: tiny_skia::Color) {
        let ts = to_transform(&mul(&g.ctm, &self.base));
        if self.misses(bounds(&UNIT, &mul(&g.ctm, &self.base)), 1.0) {
            return;
        }
        let mut paint = Paint::default();
        paint.set_color(color);
        let r = tiny_skia::Rect::from_ltrb(0.0, 0.0, 1.0, 1.0).expect("unit rect");
        self.draw(g, |pix, mask| pix.fill_rect(r, &paint, ts, mask));
    }

    fn decoded(
        &mut self,
        img: ImageData<'_>,
        res: &Resources,
        k: u32,
    ) -> Arc<Result<Decoded, image::ImageError>> {
        match img {
            ImageData::XObject {
                id: Some(id),
                stream,
            } => self.cache.image(self.file, id, stream, k),
            ImageData::XObject { id: None, stream } => {
                Arc::new(image::decode_xobject(self.file, None, stream).map(|d| shrink(d, k)))
            }
            ImageData::Inline { dict, data } => {
                Arc::new(image::decode_inline(self.file, dict, data, res).map(|d| shrink(d, k)))
            }
        }
    }
}

/// Reduce an image by `k` in both directions (box filter).
pub fn shrink(d: Decoded, k: u32) -> Decoded {
    if k <= 1 {
        return d;
    }
    let (w, h) = (d.pixmap.width(), d.pixmap.height());
    let (nw, nh) = (w.div_ceil(k), h.div_ceil(k));
    let Some(mut out) = Pixmap::new(nw, nh) else {
        return d;
    };
    let src = d.pixmap.data();
    let dst = out.data_mut();
    for y in 0..nh {
        for x in 0..nw {
            let mut sum = [0u32; 4];
            let mut n = 0;
            for sy in y * k..((y + 1) * k).min(h) {
                for sx in x * k..((x + 1) * k).min(w) {
                    let i = ((sy * w + sx) * 4) as usize;
                    for c in 0..4 {
                        sum[c] += src[i + c] as u32;
                    }
                    n += 1;
                }
            }
            let o = ((y * nw + x) * 4) as usize;
            for c in 0..4 {
                dst[o + c] = (sum[c] / n.max(1)) as u8;
            }
        }
    }
    Decoded { pixmap: out, ..d }
}

impl Device for Raster<'_> {
    fn op(&mut self, index: u32) {
        self.op = index;
        if let Mode::Highlight(ops) = &self.mode {
            self.active = ops.binary_search(&index).is_ok();
        }
    }

    fn fill(&mut self, path: &[PathEl], even_odd: bool, g: &GState) {
        if !self.active || self.misses(bounds(path, &self.base), 1.0) {
            return;
        }
        let Some(p) = build(path, None) else { return };
        let Some(paint) = self.paint_for(&g.fill, g.fill_alpha) else {
            return;
        };
        let rule = if even_odd {
            FillRule::EvenOdd
        } else {
            FillRule::Winding
        };
        let ts = to_transform(&self.base);
        self.draw(g, |pix, mask| pix.fill_path(&p, &paint, rule, ts, mask));
    }

    fn stroke(&mut self, path: &[PathEl], g: &GState) {
        if !self.active {
            return;
        }
        let Some(inv) = invert(&g.ctm) else { return };
        let Some(p) = build(path, Some(&inv)) else {
            return;
        };
        let Some(paint) = self.paint_for(&g.stroke, g.stroke_alpha) else {
            return;
        };
        let m = mul(&g.ctm, &self.base);
        let stroke = self.stroke_style(g, &m);
        // Half the width, as far as miters reach, in pixels (and a hairline).
        let reach = stroke.width as f64 * scale_of(&m) * g.miter_limit.max(1.0) / 2.0 + 2.0;
        if self.misses(bounds(path, &self.base), reach) {
            return;
        }
        let ts = to_transform(&m);
        self.draw(g, |pix, mask| {
            pix.stroke_path(&p, &paint, &stroke, ts, mask)
        });
    }

    fn text(&mut self, run: &GlyphRun<'_>, g: &GState) {
        if !self.active {
            return;
        }
        let font = &run.font.info;
        let outlines = font.has_outlines(self.file);
        if let Mode::Paint = self.mode {
            let name = match &font.base_font {
                Some(b) => format!("/{} ({b})", run.font.name),
                None => format!("/{}", run.font.name),
            };
            match &font.glyphs(self.file).source {
                GlyphSource::Substitute { name: with, .. } => {
                    self.note(format!("font {name} is drawn with {with}"));
                }
                GlyphSource::None => {
                    self.note(format!("font {name} has no glyphs: text is drawn as boxes"));
                }
                _ => {}
            }
        }
        let mut els = Vec::with_capacity(run.glyphs.len() * 24);
        for gl in &run.glyphs {
            run.glyph_path(self.file, gl, &mut els);
        }
        if els.is_empty() || self.misses(bounds(&els, &self.base), 1.0) {
            return;
        }
        if !outlines {
            // Glyph boxes, faint: where the text is, not what it says.
            let Some(p) = build(&els, None) else { return };
            let (color, alpha) = if matches!(g.text.render, 1 | 5) {
                (&g.stroke, g.stroke_alpha)
            } else {
                (&g.fill, g.fill_alpha)
            };
            let Some(mut paint) = self.paint_for(color, alpha) else {
                return;
            };
            if let (Mode::Paint, tiny_skia::Shader::SolidColor(c)) = (&self.mode, &mut paint.shader)
            {
                c.apply_opacity(TEXT_ALPHA);
            }
            let ts = to_transform(&self.base);
            self.draw(g, |pix, mask| {
                pix.fill_path(&p, &paint, FillRule::Winding, ts, mask)
            });
            return;
        }
        let r = g.text.render;
        if matches!(r, 0 | 2 | 4 | 6) {
            self.fill(&els, false, g);
        }
        if matches!(r, 1 | 2 | 5 | 6) {
            self.stroke(&els, g);
        }
    }

    fn image(&mut self, img: ImageData<'_>, res: &Resources, g: &GState) {
        if !self.active {
            return;
        }
        match self.mode {
            Mode::Probe => return self.unit_square(g, tiny_skia::Color::BLACK),
            Mode::Highlight(_) => return self.unit_square(g, highlight()),
            Mode::Paint => {}
        }
        let m = mul(&g.ctm, &self.base);
        if self.misses(bounds(&UNIT, &m), 1.0) {
            return;
        }
        // Pixels the image covers along each edge of the unit square.
        let (dw, dh) = (m[0].hypot(m[1]), m[2].hypot(m[3]));
        let (iw, ih) = image_size(self.file, img).unwrap_or((1, 1));
        let k = if dw > 0.0 && dh > 0.0 {
            ((iw as f64 / dw).min(ih as f64 / dh).floor() as u32).clamp(1, 64)
        } else {
            1
        };
        let decoded = self.decoded(img, res, k);
        let d = match &*decoded {
            Ok(d) => d,
            Err(e) => {
                self.note(e.to_string());
                return self.unit_square(g, tiny_skia::Color::from_rgba8(128, 128, 128, 64));
            }
        };
        if d.approximate {
            self.note("Separation / DeviceN colors are approximated");
        }
        let colored;
        let pix = if d.stencil {
            let Some(rgb) = self.rgb(&g.fill) else { return };
            let mut p = d.pixmap.clone();
            let q = |v: f32, a: u8| ((v * a as f32) + 0.5) as u8;
            for px in p.data_mut().chunks_exact_mut(4) {
                let a = px[3];
                px[0] = q(rgb[0], a);
                px[1] = q(rgb[1], a);
                px[2] = q(rgb[2], a);
            }
            colored = p;
            &colored
        } else {
            &d.pixmap
        };
        let (w, h) = (pix.width() as f32, pix.height() as f32);
        let upscaled = dw / w as f64 > 1.5 || dh / h as f64 > 1.5;
        let quality = if upscaled && !d.interpolate {
            FilterQuality::Nearest
        } else {
            FilterQuality::Bilinear
        };
        let shader = Pattern::new(
            pix.as_ref(),
            SpreadMode::Pad,
            quality,
            g.fill_alpha as f32,
            Transform::from_row(1.0 / w, 0.0, 0.0, -1.0 / h, 0.0, 1.0),
        );
        let paint = Paint {
            shader,
            ..Paint::default()
        };
        let r = tiny_skia::Rect::from_ltrb(0.0, 0.0, 1.0, 1.0).expect("unit rect");
        let ts = to_transform(&m);
        self.draw(g, |p, mask| p.fill_rect(r, &paint, ts, mask));
    }

    fn shading(&mut self, _shading: &Object, _g: &GState) {
        if let Mode::Paint = self.mode {
            self.note("shadings (sh) are not drawn yet");
        }
    }
}

/// `/Width` and `/Height` of an image, without decoding it.
fn image_size(file: &PdfFile, img: ImageData<'_>) -> Option<(u32, u32)> {
    use pdfi_syntax::DictExt;
    let (d, inline) = match img {
        ImageData::XObject { stream, .. } => (&stream.dict, false),
        ImageData::Inline { dict, .. } => (dict, true),
    };
    let get = |long: &str, short: &str| {
        let o = d
            .get_key(long)
            .or_else(|| inline.then(|| d.get_key(short)).flatten())?;
        pdfi_model::resolve(file, o).1.as_f64()
    };
    let (w, h) = (get("Width", "W")?, get("Height", "H")?);
    (w >= 1.0 && h >= 1.0).then(|| (w.min(1e9) as u32, h.min(1e9) as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse() {
        let m = [2.0, 0.0, 0.0, 4.0, 10.0, 20.0];
        let i = invert(&m).unwrap();
        assert_eq!(mul(&m, &i), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn rectangles() {
        let r = [
            PathEl::Move(0.0, 0.0),
            PathEl::Line(10.0, 0.0),
            PathEl::Line(10.0, 5.0),
            PathEl::Line(0.0, 5.0),
            PathEl::Close,
        ];
        let id = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        assert_eq!(as_rect(&r, &id), Some((0.0, 0.0, 10.0, 5.0)));
        let skew = [1.0, 0.5, 0.0, 1.0, 0.0, 0.0];
        assert_eq!(as_rect(&r, &skew), None);
    }
}
