//! Image decoding to premultiplied RGBA: samples of 1–16 bits in the
//! color spaces of [`crate::color`], `/Decode`, stencil masks
//! (`/ImageMask`), soft masks (`/SMask`, and `/SMaskInData` of JPEG 2000),
//! and `/Mask` as a stencil or color key ranges. Codecs: JPEG (zune-jpeg),
//! JPEG 2000, JBIG2 (with `/JBIG2Globals`) and CCITT fax (the hayro
//! decoders); JBIG2 and CCITT give 1-bit samples like any other image.

use pdfi_content::interp::{Lookup, Resources};
use pdfi_model::resolve;
use pdfi_syntax::filters::{self, canonical_name};
use pdfi_syntax::{Dict, DictExt, Name, ObjId, Object, PdfFile, Stream};
use tiny_skia::{IntSize, Pixmap};
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace as JpegSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

use crate::color::{self, Approx, Profiles, Space};

/// Largest image decoded (pixels).
pub const MAX_PIXELS: u64 = 1 << 25;
/// Largest decoded sample data.
const MAX_DATA: usize = 256 << 20;

/// A decoded image.
#[derive(Debug)]
pub struct Decoded {
    /// Premultiplied RGBA; for stencil masks only the alpha matters.
    pub pixmap: Pixmap,
    /// `/ImageMask`: painted with the fill color.
    pub stencil: bool,
    pub interpolate: bool,
    /// Colors were converted approximately (tint transforms).
    pub approximate: bool,
}

impl Decoded {
    pub fn bytes(&self) -> usize {
        self.pixmap.data().len()
    }
}

/// Why an image is not drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// A filter not decoded yet (`JPXDecode`, ...).
    Filter(String),
    TooLarge,
    Invalid(String),
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::Filter(n) => write!(f, "/{n} images are not decoded yet"),
            ImageError::TooLarge => write!(f, "image too large (over {MAX_PIXELS} pixels)"),
            ImageError::Invalid(m) => f.write_str(m),
        }
    }
}

/// Image dictionary entries, from a stream or an inline image.
struct Params {
    width: u32,
    height: u32,
    bpc: u32,
    space: Option<Space>,
    decode: Option<Vec<f64>>,
    stencil: bool,
    interpolate: bool,
    /// `/SMaskInData` (JPEG 2000): use the image's own opacity.
    smask_in_data: bool,
}

/// The image codec a stream's filters stop at, with its parameters.
struct Codec<'a> {
    name: Vec<u8>,
    parms: Option<Dict>,
    file: &'a PdfFile,
}

impl<'a> Codec<'a> {
    fn of(
        file: &'a PdfFile,
        specs: &[filters::FilterSpec],
        stopped: Option<&Name>,
    ) -> Option<Self> {
        let name = stopped?;
        let parms = specs
            .iter()
            .rev()
            .find(|f| f.name == *name)
            .and_then(|f| f.parms.clone());
        Some(Codec {
            name: canonical_name(name.as_bytes()).to_vec(),
            parms,
            file,
        })
    }

    fn parm(&self, k: &str) -> Option<Object> {
        let v = self.parms.as_ref()?.get_key(k)?;
        Some((*resolve(self.file, v).1).clone())
    }
}

/// Look up `long` (or, in inline images, `short`), resolved.
fn entry(file: &PdfFile, d: &Dict, long: &str, short: Option<&str>) -> Option<Object> {
    let o = d
        .get_key(long)
        .or_else(|| short.and_then(|s| d.get_key(s)))?;
    Some((*resolve(file, o).1).clone())
}

fn params(
    file: &PdfFile,
    d: &Dict,
    inline: bool,
    space: impl FnOnce(&Object) -> Space,
) -> Result<Params, ImageError> {
    let ab = |s: &'static str| inline.then_some(s);
    let int = |long, short| entry(file, d, long, ab(short)).and_then(|o| o.as_f64());
    let width = int("Width", "W").unwrap_or(0.0);
    let height = int("Height", "H").unwrap_or(0.0);
    if !(width >= 1.0 && height >= 1.0) {
        return Err(ImageError::Invalid("image without a valid size".into()));
    }
    let (width, height) = (width.min(1e9) as u32, height.min(1e9) as u32);
    if width as u64 * height as u64 > MAX_PIXELS {
        return Err(ImageError::TooLarge);
    }
    let stencil = matches!(
        entry(file, d, "ImageMask", ab("IM")),
        Some(Object::Bool(true))
    );
    let bpc = if stencil {
        1
    } else {
        int("BitsPerComponent", "BPC").map_or(8, |b| b as u32)
    };
    if !matches!(bpc, 1 | 2 | 4 | 8 | 16) {
        return Err(ImageError::Invalid(format!(
            "{bpc} bits per component is not 1, 2, 4, 8 or 16"
        )));
    }
    let space = match (stencil, entry(file, d, "ColorSpace", ab("CS"))) {
        (true, _) => None,
        (false, Some(cs)) => Some(space(&cs)),
        (false, None) => None,
    };
    let decode = entry(file, d, "Decode", ab("D")).and_then(|o| {
        o.as_array()?
            .iter()
            .map(|n| resolve(file, &n.obj).1.as_f64())
            .collect::<Option<Vec<_>>>()
    });
    let interpolate = matches!(
        entry(file, d, "Interpolate", ab("I")),
        Some(Object::Bool(true))
    );
    let smask_in_data = entry(file, d, "SMaskInData", None)
        .and_then(|v| v.as_f64())
        .is_some_and(|v| v != 0.0);
    Ok(Params {
        width,
        height,
        bpc,
        space,
        decode,
        stencil,
        interpolate,
        smask_in_data,
    })
}

/// Decode an image XObject (and its `/SMask` or `/Mask`).
pub fn decode_xobject(
    file: &PdfFile,
    profiles: &Profiles,
    id: Option<ObjId>,
    s: &Stream,
) -> Result<Decoded, ImageError> {
    let d = &s.dict;
    let p = params(file, d, false, |o| color::parse(file, profiles, o))?;
    let raw = file.decode_stream(id.unwrap_or(ObjId::new(0, 0)), s, Some(MAX_DATA));
    let specs = file.filter_specs(d);
    let codec = Codec::of(file, &specs, raw.stopped_at.as_ref());
    let mut out = samples(&p, raw.data, codec.as_ref())?;
    if p.stencil {
        return Ok(out);
    }
    if let Some(sm) = d.get_key("SMask") {
        let (sid, so) = resolve(file, sm);
        if let Some(ss) = so.as_stream() {
            if let Ok(m) = decode_gray(file, sid, ss) {
                apply_alpha(&mut out.pixmap, &m.0, m.1, m.2);
            }
        }
    } else if let Some(mask) = d.get_key("Mask") {
        let (mid, mo) = resolve(file, mask);
        match &*mo {
            Object::Stream(ms) => {
                if let Ok(m) = decode_xobject(file, profiles, mid, ms) {
                    let (w, h) = (m.pixmap.width(), m.pixmap.height());
                    let a: Vec<u8> = m.pixmap.pixels().iter().map(|c| c.alpha()).collect();
                    apply_alpha(&mut out.pixmap, &a, w, h);
                }
            }
            Object::Array(a) => {
                let ranges: Vec<f64> = a.iter().filter_map(|n| n.obj.as_f64()).collect();
                let raw = file.decode_stream(id.unwrap_or(ObjId::new(0, 0)), s, Some(MAX_DATA));
                if raw.stopped_at.is_none() {
                    color_key(&p, &raw.data, &ranges, &mut out.pixmap);
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Decode an inline image; named color spaces come from `res`.
pub fn decode_inline(
    file: &PdfFile,
    profiles: &Profiles,
    d: &Dict,
    data: &[u8],
    res: &Resources,
) -> Result<Decoded, ImageError> {
    let p = params(file, d, true, |o| match o {
        Object::Name(n) if !is_inline_space(n) => match res.lookup(file, "ColorSpace", n) {
            Lookup::Found(_, cs) => color::parse(file, profiles, &cs),
            Lookup::Missing => Space::Unknown(1),
        },
        o => color::parse(file, profiles, o),
    })?;
    let specs = filters::filter_specs(d);
    let chain = filters::build_chain(&specs);
    let stopped = chain.stopped_at.clone();
    let (data, _) = filters::decode_all(data, chain.decoders, Some(MAX_DATA));
    let codec = Codec::of(file, &specs, stopped.as_ref());
    let mut out = samples(&p, data.clone(), codec.as_ref())?;
    if let Some(Object::Array(a)) = entry(file, d, "Mask", None) {
        let ranges: Vec<f64> = a.iter().filter_map(|n| n.obj.as_f64()).collect();
        if stopped.is_none() {
            color_key(&p, &data, &ranges, &mut out.pixmap);
        }
    }
    Ok(out)
}

fn is_inline_space(n: &Name) -> bool {
    matches!(
        n.as_bytes(),
        b"G" | b"RGB" | b"CMYK" | b"I" | b"DeviceGray" | b"DeviceRGB" | b"DeviceCMYK" | b"Indexed"
    )
}

/// A soft mask: its gray samples as alpha.
fn decode_gray(
    file: &PdfFile,
    id: Option<ObjId>,
    s: &Stream,
) -> Result<(Vec<u8>, u32, u32), ImageError> {
    let mut p = params(file, &s.dict, false, |_| Space::Gray)?;
    p.space = Some(Space::Gray);
    p.stencil = false;
    let raw = file.decode_stream(id.unwrap_or(ObjId::new(0, 0)), s, Some(MAX_DATA));
    let specs = file.filter_specs(&s.dict);
    let codec = Codec::of(file, &specs, raw.stopped_at.as_ref());
    let img = samples(&p, raw.data, codec.as_ref())?;
    let (w, h) = (img.pixmap.width(), img.pixmap.height());
    Ok((img.pixmap.pixels().iter().map(|c| c.red()).collect(), w, h))
}

/// Multiply the alpha of `pix` by a `w × h` mask (scaled to fit).
fn apply_alpha(pix: &mut Pixmap, mask: &[u8], w: u32, h: u32) {
    let (pw, ph) = (pix.width(), pix.height());
    let data = pix.data_mut();
    for y in 0..ph {
        let my = (y as u64 * h as u64 / ph as u64) as u32;
        for x in 0..pw {
            let mx = (x as u64 * w as u64 / pw as u64) as u32;
            let a = mask.get((my * w + mx) as usize).copied().unwrap_or(255) as u32;
            let k = ((y * pw + x) * 4) as usize;
            for c in &mut data[k..k + 4] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
    }
}

/// Unpack `bpc`-bit samples of one row.
fn unpack(row: &[u8], bpc: u32, out: &mut [u16]) {
    match bpc {
        8 => {
            for (o, &b) in out.iter_mut().zip(row) {
                *o = b as u16;
            }
        }
        16 => {
            for (o, c) in out.iter_mut().zip(row.chunks_exact(2)) {
                *o = u16::from_be_bytes([c[0], c[1]]);
            }
        }
        _ => {
            let per = 8 / bpc as usize;
            let mask = (1u16 << bpc) - 1;
            for (k, o) in out.iter_mut().enumerate().take(row.len() * per) {
                let b = row[k / per] as u16;
                let shift = 8 - bpc as usize * (k % per + 1);
                *o = (b >> shift) & mask;
            }
        }
    }
}

/// Pixels whose samples are all within the `/Mask` ranges become
/// transparent.
fn color_key(p: &Params, data: &[u8], ranges: &[f64], pix: &mut Pixmap) {
    let n = p.space.as_ref().map_or(1, Space::components).max(1);
    if ranges.len() < 2 * n {
        return;
    }
    let stride = (p.width as usize * n * p.bpc as usize).div_ceil(8);
    let mut row = vec![0u16; p.width as usize * n];
    let w = p.width as usize;
    let out = pix.data_mut();
    for y in 0..p.height as usize {
        let Some(r) = data.get(y * stride..(y + 1) * stride) else {
            break;
        };
        unpack(r, p.bpc, &mut row);
        for x in 0..w {
            let px = &row[x * n..x * n + n];
            let keyed = px
                .iter()
                .enumerate()
                .all(|(k, &v)| ranges[2 * k] <= v as f64 && v as f64 <= ranges[2 * k + 1]);
            if keyed {
                out[(y * w + x) * 4..(y * w + x) * 4 + 4].fill(0);
            }
        }
    }
}

fn pixmap(w: u32, h: u32, data: Vec<u8>) -> Result<Pixmap, ImageError> {
    let size = IntSize::from_wh(w, h).ok_or(ImageError::TooLarge)?;
    Pixmap::from_vec(data, size).ok_or(ImageError::TooLarge)
}

/// Turn sample data (all filters applied up to `codec`) into a pixmap.
fn samples(p: &Params, data: Vec<u8>, codec: Option<&Codec>) -> Result<Decoded, ImageError> {
    let mut space = p.space.clone();
    let mut bpc = p.bpc;
    let (mut width, mut height) = (p.width, p.height);
    let mut decode = p.decode.clone();
    let mut data = data;
    // Opacity that comes with the samples (JPEG 2000).
    let mut alpha = None;
    if let Some(c) = codec {
        match c.name.as_slice() {
            b"JPXDecode" => {
                let indexed = matches!(space, Some(Space::Indexed { .. }));
                let j = jpx(&data, indexed)?;
                (width, height, data) = (j.width, j.height, j.data);
                bpc = 8;
                decode = None;
                if p.smask_in_data {
                    alpha = j.alpha;
                }
                // The image's own space, unless the dictionary gives one
                // that fits.
                if space.as_ref().is_none_or(|s| s.components() != j.n) {
                    space = Some(j.space);
                }
            }
            b"JBIG2Decode" => {
                let globals = c
                    .parms
                    .as_ref()
                    .and_then(|d| d.get_key("JBIG2Globals"))
                    .and_then(|o| {
                        let (gid, g) = resolve(c.file, o);
                        let gs = g.as_stream()?;
                        let d = c.file.decode_stream(
                            gid.unwrap_or(ObjId::new(0, 0)),
                            gs,
                            Some(MAX_DATA),
                        );
                        d.stopped_at.is_none().then_some(d.data)
                    });
                let (w, h, bits) = jbig2(&data, globals.as_deref())?;
                (width, height, data) = (w, h, bits);
                bpc = 1;
            }
            b"CCITTFaxDecode" => {
                let (w, h, bits) = ccitt(&data, c, height)?;
                (width, height, data) = (w, h, bits);
                bpc = 1;
            }
            b"DCTDecode" if !p.stencil => {
                let j = jpeg(&data)?;
                (width, height, data) = (j.width, j.height, j.data);
                bpc = 8;
                match j.components {
                    1 => space = Some(Space::Gray),
                    3 if j.converted => {
                        space = Some(Space::Rgb);
                        decode = None;
                    }
                    3 => {
                        if space.as_ref().is_none_or(|s| s.components() != 3) {
                            space = Some(Space::Rgb);
                        }
                    }
                    _ => {
                        if space.as_ref().is_none_or(|s| s.components() != 4) {
                            space = Some(Space::Cmyk);
                        }
                    }
                }
            }
            other => {
                return Err(ImageError::Filter(
                    String::from_utf8_lossy(other).into_owned(),
                ));
            }
        }
    }
    let mut out = pixels(p, space, bpc, width, height, decode, &data)?;
    if let Some(a) = alpha {
        apply_alpha(&mut out.pixmap, &a, width, height);
    }
    Ok(out)
}

/// Samples of `bpc` bits in `space` (or a stencil mask) to a pixmap.
fn pixels(
    p: &Params,
    space: Option<Space>,
    bpc: u32,
    width: u32,
    height: u32,
    decode: Option<Vec<f64>>,
    data: &[u8],
) -> Result<Decoded, ImageError> {
    if width as u64 * height as u64 > MAX_PIXELS {
        return Err(ImageError::TooLarge);
    }
    let (w, h) = (width as usize, height as usize);
    let mut out = vec![0u8; w * h * 4];
    let mut approx = Approx::default();
    if p.stencil {
        // Sample 0 paints, unless /Decode is [1 0].
        let paint_on = u16::from(decode.as_ref().is_some_and(|d| d.first() == Some(&1.0)));
        let stride = w.div_ceil(8);
        let mut row = vec![0u16; w];
        for y in 0..h {
            let r = data.get(y * stride..(y + 1) * stride).unwrap_or(&[]);
            row.fill(1 - paint_on);
            unpack(r, 1, &mut row);
            for x in 0..w {
                if row[x] == paint_on {
                    out[(y * w + x) * 4 + 3] = 255;
                }
            }
        }
        return Ok(Decoded {
            pixmap: pixmap(width, height, out)?,
            stencil: true,
            interpolate: p.interpolate,
            approximate: false,
        });
    }
    let space = space.unwrap_or(Space::Gray);
    let n = space.components().max(1);
    let max = ((1u32 << bpc) - 1) as f64;
    let ranges: Vec<(f64, f64)> = match &decode {
        Some(d) if d.len() >= 2 * n => d.chunks(2).take(n).map(|c| (c[0], c[1])).collect(),
        _ => space.default_decode(bpc),
    };
    let value = |k: usize, s: u16| {
        let (lo, hi) = ranges.get(k).copied().unwrap_or((0.0, 1.0));
        lo + s as f64 * (hi - lo) / max
    };
    // One component of at most 8 bits: a table of every color.
    let table: Option<Vec<[u8; 4]>> = (n == 1 && bpc <= 8).then(|| {
        (0..=max as u16)
            .map(|s| rgba(space.to_rgb(&[value(0, s)], &mut approx)))
            .collect()
    });
    let stride = (w * n * bpc as usize).div_ceil(8);
    let mut row = vec![0u16; w * n];
    let mut c = vec![0.0; n];
    // ICC profiles convert a row at a time.
    let icc = match (&space, &table) {
        (Space::Icc(icc), None) => Some(icc),
        _ => None,
    };
    let (mut src, mut dst) = match icc {
        Some(_) => (vec![0f32; w * n], vec![0f32; w * 3]),
        None => (Vec::new(), Vec::new()),
    };
    for y in 0..h {
        let Some(r) = data.get(y * stride..(y + 1) * stride) else {
            // Missing rows stay transparent.
            break;
        };
        unpack(r, bpc, &mut row);
        let line = &mut out[y * w * 4..(y + 1) * w * 4];
        if let Some(t) = &table {
            for (px, &s) in line.chunks_exact_mut(4).zip(&row) {
                px.copy_from_slice(&t[s as usize]);
            }
            continue;
        }
        if let Some(icc) = icc {
            for (k, (v, &s)) in src.iter_mut().zip(&row).enumerate() {
                *v = value(k % n, s).clamp(0.0, 1.0) as f32;
            }
            if icc.convert(&src, &mut dst) {
                for (px, c) in line.chunks_exact_mut(4).zip(dst.chunks_exact(3)) {
                    px.copy_from_slice(&rgba(Some([c[0], c[1], c[2]].map(|v| v.clamp(0.0, 1.0)))));
                }
            }
            continue;
        }
        for (px, s) in line.chunks_exact_mut(4).zip(row.chunks_exact(n)) {
            for k in 0..n {
                c[k] = value(k, s[k]);
            }
            px.copy_from_slice(&rgba(space.to_rgb(&c, &mut approx)));
        }
    }
    Ok(Decoded {
        pixmap: pixmap(width, height, out)?,
        stencil: false,
        interpolate: p.interpolate,
        approximate: approx.tint,
    })
}

fn rgba(c: Option<[f32; 3]>) -> [u8; 4] {
    match c {
        Some([r, g, b]) => {
            let q = |v: f32| (v * 255.0 + 0.5) as u8;
            [q(r), q(g), q(b), 255]
        }
        None => [0, 0, 0, 0],
    }
}

struct Jpx {
    width: u32,
    height: u32,
    /// Color components (without opacity).
    n: usize,
    space: Space,
    data: Vec<u8>,
    alpha: Option<Vec<u8>>,
}

/// Decode JPEG 2000 data to 8-bit samples (palette indices kept as they
/// are for an `/Indexed` space).
fn jpx(data: &[u8], indexed: bool) -> Result<Jpx, ImageError> {
    use hayro_jpeg2000::{ColorSpace as J, DecodeSettings, Image};
    let bad = |e: hayro_jpeg2000::DecodeError| ImageError::Invalid(format!("JPEG 2000: {e}"));
    let settings = DecodeSettings {
        resolve_palette_indices: !indexed,
        ..DecodeSettings::default()
    };
    let img = Image::new(data, &settings).map_err(bad)?;
    let (width, height) = (img.width(), img.height());
    if width as u64 * height as u64 > MAX_PIXELS || width == 0 || height == 0 {
        return Err(ImageError::TooLarge);
    }
    let n = img.color_space().num_channels() as usize;
    let has_alpha = img.has_alpha();
    let depth = img.original_bit_depth();
    let space = match img.color_space() {
        J::Gray => Space::Gray,
        J::RGB => Space::Rgb,
        J::CMYK => Space::Cmyk,
        J::Icc { profile, .. } => match crate::color::Icc::from_profile(profile, n) {
            Some(icc) => Space::Icc(icc),
            None => device(n),
        },
        J::Unknown { .. } => device(n),
    };
    let raw = img.decode().map_err(bad)?;
    let k = n + usize::from(has_alpha);
    let (mut data, alpha) = if has_alpha {
        let mut d = Vec::with_capacity(raw.len() / k * n);
        let mut a = Vec::with_capacity(raw.len() / k);
        for px in raw.chunks_exact(k) {
            d.extend_from_slice(&px[..n]);
            a.push(px[n]);
        }
        (d, Some(a))
    } else {
        (raw, None)
    };
    // Indices of fewer bits were scaled to 8 bits: undo it.
    if indexed && depth < 8 {
        let max = ((1u32 << depth) - 1) as f32;
        for v in &mut data {
            *v = (*v as f32 * max / 255.0).round() as u8;
        }
    }
    Ok(Jpx {
        width,
        height,
        n,
        space,
        data,
        alpha,
    })
}

/// A device space of `n` components.
fn device(n: usize) -> Space {
    match n {
        1 => Space::Gray,
        4 => Space::Cmyk,
        3 => Space::Rgb,
        n => Space::Unknown(n),
    }
}

/// 1-bit rows being filled pixel by pixel.
struct Packed {
    data: Vec<u8>,
    width: usize,
    stride: usize,
    /// What unpainted rows hold.
    fill: u8,
    x: usize,
    row: usize,
    /// Rows started (pixels were pushed into them).
    rows: usize,
}

impl Packed {
    fn new(width: usize, rows: usize, white: bool) -> Self {
        let stride = width.div_ceil(8);
        let fill = if white { 0xFF } else { 0 };
        Packed {
            data: vec![fill; (stride * rows).min(MAX_DATA)],
            width,
            stride,
            fill,
            x: 0,
            row: 0,
            rows: 0,
        }
    }

    fn put(&mut self, one: bool) {
        if self.x < self.width {
            let end = (self.row + 1) * self.stride;
            if end > self.data.len() && end <= MAX_DATA {
                self.data.resize(end, self.fill);
            }
            if let Some(b) = self.data.get_mut(self.row * self.stride + self.x / 8) {
                let bit = 0x80 >> (self.x % 8);
                if one {
                    *b |= bit;
                } else {
                    *b &= !bit;
                }
            }
            self.rows = self.rows.max(self.row + 1);
        }
        self.x += 1;
    }

    fn next_line(&mut self) {
        self.row += 1;
        self.x = 0;
    }
}

/// JBIG2 (embedded organization) to 1-bit samples, 0 for black.
fn jbig2(data: &[u8], globals: Option<&[u8]>) -> Result<(u32, u32, Vec<u8>), ImageError> {
    struct Out(Packed);
    impl hayro_jbig2::Decoder for Out {
        fn push_pixel(&mut self, black: bool) {
            self.0.put(!black);
        }
        fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
            for _ in 0..chunk_count * 8 {
                self.0.put(!black);
            }
        }
        fn next_line(&mut self) {
            self.0.next_line();
        }
    }
    let bad = |e: hayro_jbig2::DecodeError| ImageError::Invalid(format!("JBIG2: {e}"));
    let img = hayro_jbig2::Image::new_embedded(data, globals).map_err(bad)?;
    let (w, h) = (img.width(), img.height());
    if w as u64 * h as u64 > MAX_PIXELS {
        return Err(ImageError::TooLarge);
    }
    let mut out = Out(Packed::new(w as usize, h as usize, true));
    img.decode(&mut out).map_err(bad)?;
    Ok((w, h, out.0.data))
}

/// CCITT fax data to 1-bit samples (`/BlackIs1` decides which is 1);
/// `height` is used when `/Rows` is not given.
fn ccitt(data: &[u8], c: &Codec, height: u32) -> Result<(u32, u32, Vec<u8>), ImageError> {
    use hayro_ccitt::{DecodeSettings, DecoderContext, EncodingMode};
    struct Out(Packed, bool);
    impl hayro_ccitt::Decoder for Out {
        fn push_pixel(&mut self, white: bool) {
            self.0.put(white != self.1);
        }
        fn push_pixel_chunk(&mut self, white: bool, chunk_count: u32) {
            for _ in 0..chunk_count * 8 {
                self.0.put(white != self.1);
            }
        }
        fn next_line(&mut self) {
            self.0.next_line();
        }
    }
    let int = |k: &str, d: f64| c.parm(k).and_then(|v| v.as_f64()).unwrap_or(d);
    let flag = |k: &str, d: bool| match c.parm(k) {
        Some(Object::Bool(b)) => b,
        _ => d,
    };
    let k = int("K", 0.0) as i64;
    let columns = int("Columns", 1728.0).clamp(1.0, 1e6) as u32;
    let rows = int("Rows", 0.0).max(0.0) as u32;
    let black_is_1 = flag("BlackIs1", false);
    let settings = DecodeSettings {
        columns,
        rows: if rows > 0 { rows } else { height },
        end_of_block: flag("EndOfBlock", true),
        end_of_line: flag("EndOfLine", false),
        rows_are_byte_aligned: flag("EncodedByteAlign", false),
        encoding: match k {
            k if k < 0 => EncodingMode::Group4,
            0 => EncodingMode::Group3_1D,
            k => EncodingMode::Group3_2D { k: k as u32 },
        },
        invert_black: false,
    };
    let want = if rows > 0 { rows } else { height };
    if columns as u64 * want as u64 > MAX_PIXELS {
        return Err(ImageError::TooLarge);
    }
    let mut out = Out(
        Packed::new(columns as usize, want as usize, !black_is_1),
        black_is_1,
    );
    let mut ctx = DecoderContext::new(settings);
    // Damaged data: keep the rows decoded before the error.
    let result = hayro_ccitt::decode(data, &mut out, &mut ctx);
    let got = out.0.rows as u32;
    if result.is_err() && got == 0 {
        return Err(ImageError::Invalid(format!(
            "CCITT: {}",
            result.err().map(|e| e.to_string()).unwrap_or_default()
        )));
    }
    let h = if rows > 0 { rows } else { got.max(1) };
    let mut bits = out.0.data;
    bits.resize(columns.div_ceil(8) as usize * h as usize, out.0.fill);
    Ok((columns, h, bits))
}

struct Jpeg {
    width: u32,
    height: u32,
    components: usize,
    /// Converted to RGB by the decoder (YCCK).
    converted: bool,
    data: Vec<u8>,
}

fn jpeg(data: &[u8]) -> Result<Jpeg, ImageError> {
    let bad = |e: zune_jpeg::errors::DecodeErrors| ImageError::Invalid(format!("JPEG: {e:?}"));
    let mut d = JpegDecoder::new(ZCursor::new(data));
    d.decode_headers().map_err(bad)?;
    let input = d.input_colorspace().unwrap_or(JpegSpace::YCbCr);
    let (out, components, converted) = match input {
        JpegSpace::Luma => (JpegSpace::Luma, 1, false),
        JpegSpace::CMYK => (JpegSpace::CMYK, 4, false),
        JpegSpace::YCCK => (JpegSpace::RGB, 3, true),
        _ => (JpegSpace::RGB, 3, false),
    };
    let info = d
        .info()
        .ok_or_else(|| ImageError::Invalid("JPEG: no header".into()))?;
    if info.width as u64 * info.height as u64 > MAX_PIXELS {
        return Err(ImageError::TooLarge);
    }
    let mut d = JpegDecoder::new_with_options(
        ZCursor::new(data),
        DecoderOptions::default()
            .jpeg_set_out_colorspace(out)
            .set_max_width(1 << 16)
            .set_max_height(1 << 16),
    );
    let pixels = d.decode().map_err(bad)?;
    Ok(Jpeg {
        width: info.width as u32,
        height: info.height as u32,
        components,
        converted,
        data: pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpack_bits() {
        let mut out = [0u16; 8];
        unpack(&[0b1011_0010], 1, &mut out);
        assert_eq!(out, [1, 0, 1, 1, 0, 0, 1, 0]);
        let mut out = [0u16; 2];
        unpack(&[0xA5], 4, &mut out);
        assert_eq!(out, [0xA, 0x5]);
    }
}
