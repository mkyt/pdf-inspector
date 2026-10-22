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
    /// Reduce by this in both directions while decoding, if the codec
    /// can (JBIG2 and CCITT); 1: full size.
    reduce: u32,
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
    if width as u64 * height as u64 > MAX_PIXELS && !(reduces(file, d) && !inline) {
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
        reduce: 1,
    })
}

/// Whether an image's codec decodes it reduced (bilevel JBIG2 and CCITT
/// images of any size).
pub fn reduces(file: &PdfFile, d: &Dict) -> bool {
    file.filter_specs(d).last().is_some_and(|f| {
        matches!(
            canonical_name(f.name.as_bytes()),
            b"JBIG2Decode" | b"CCITTFaxDecode"
        )
    })
}

/// Decode an image XObject (and its `/SMask` or `/Mask`), reduced by
/// about `k` in both directions (bilevel codecs reduce while decoding,
/// and also when the image would be too large otherwise).
pub fn decode_xobject(
    file: &PdfFile,
    profiles: &Profiles,
    id: Option<ObjId>,
    s: &Stream,
    k: u32,
) -> Result<Decoded, ImageError> {
    let d = &s.dict;
    let mut p = params(file, d, false, |o| color::parse(file, profiles, o))?;
    p.reduce = k.max(1);
    let raw = file.decode_stream(id.unwrap_or(ObjId::new(0, 0)), s, Some(MAX_DATA));
    let specs = file.filter_specs(d);
    let codec = Codec::of(file, &specs, raw.stopped_at.as_ref());
    let (out, reduced) = samples(&p, raw.data, codec.as_ref())?;
    let mut out = if reduced {
        out
    } else {
        crate::raster::shrink(out, k)
    };
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
                if let Ok(m) = decode_xobject(file, profiles, mid, ms, 1) {
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
    let (mut out, _) = samples(&p, data.clone(), codec.as_ref())?;
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
    let (img, _) = samples(&p, raw.data, codec.as_ref())?;
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

/// Turn sample data (all filters applied up to `codec`) into a pixmap;
/// says whether the codec reduced it by `p.reduce`.
fn samples(
    p: &Params,
    data: Vec<u8>,
    codec: Option<&Codec>,
) -> Result<(Decoded, bool), ImageError> {
    let mut reduced = false;
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
                let b = jbig2(&data, globals.as_deref(), p.reduce)?;
                (width, height, data, bpc) = (b.width, b.height, b.data, b.bits);
                reduced = true;
            }
            b"CCITTFaxDecode" => {
                let b = ccitt(&data, c, height, p.reduce)?;
                (width, height, data, bpc) = (b.width, b.height, b.data, b.bits);
                reduced = true;
            }
            b"DCTDecode" if !p.stencil => {
                let j = jpeg(&data)?;
                (width, height, data) = (j.width, j.height, j.data);
                bpc = 8;
                // The dictionary's space when it has as many components
                // (Separation, DeviceN, ICCBased, Indexed …), else the
                // device space of the data.
                if space
                    .as_ref()
                    .is_none_or(|s| s.components() != j.components)
                {
                    space = Some(match j.components {
                        1 => Space::Gray,
                        3 => Space::Rgb,
                        _ => Space::Cmyk,
                    });
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
    Ok((out, reduced))
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
        if bpc == 8 {
            // Reduced while decoding: the share of 1 samples.
            for (o, &v) in out.chunks_exact_mut(4).zip(data) {
                o[3] = if paint_on == 1 { v } else { 255 - v };
            }
            return Ok(Decoded {
                pixmap: pixmap(width, height, out)?,
                stencil: true,
                interpolate: true,
                approximate: false,
            });
        }
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

/// Decoded bilevel samples: 1 bit each, or (reduced) the share of 1
/// bits in each `k × k` block as a byte.
struct Bilevel {
    width: u32,
    height: u32,
    bits: u32,
    data: Vec<u8>,
}

/// Where a bilevel decoder's pixels go, in runs.
enum Sink {
    /// 1-bit rows.
    Full {
        data: Vec<u8>,
        stride: usize,
        /// What unpainted rows hold.
        fill: u8,
    },
    /// Counts of 1 bits per block of `k × k`, a band of `k` rows at a
    /// time.
    Reduced {
        k: usize,
        counts: Vec<u32>,
        data: Vec<u8>,
    },
}

struct Rows {
    sink: Sink,
    width: usize,
    x: usize,
    row: usize,
    /// Rows started (pixels were pushed into them).
    rows: usize,
}

impl Rows {
    /// Rows of `width` 1-bit samples, `height` expected; reduced by `k`
    /// (raised to fit [`MAX_PIXELS`]). `ones`: what missing samples are.
    fn new(width: usize, height: usize, k: u32, ones: bool) -> Self {
        let need = ((width as f64 * height as f64 / MAX_PIXELS as f64)
            .sqrt()
            .ceil()) as usize;
        let k = (k as usize).max(need).max(1);
        let sink = if k == 1 {
            let stride = width.div_ceil(8);
            let fill = if ones { 0xFF } else { 0 };
            Sink::Full {
                data: vec![fill; (stride * height).min(MAX_DATA)],
                stride,
                fill,
            }
        } else {
            Sink::Reduced {
                k,
                counts: vec![0; width.div_ceil(k)],
                data: Vec::with_capacity((width.div_ceil(k) * height.div_ceil(k)).min(MAX_DATA)),
            }
        };
        Rows {
            sink,
            width,
            x: 0,
            row: 0,
            rows: 0,
        }
    }

    /// `n` samples of the same value.
    fn run(&mut self, one: bool, n: usize) {
        let (start, end) = (self.x.min(self.width), (self.x + n).min(self.width));
        self.x += n;
        if start >= end {
            return;
        }
        self.rows = self.rows.max(self.row + 1);
        match &mut self.sink {
            Sink::Full { data, stride, fill } => {
                let need = (self.row + 1) * *stride;
                if need > data.len() {
                    if need > MAX_DATA {
                        return;
                    }
                    data.resize(need, *fill);
                }
                let line = &mut data[self.row * *stride..need];
                for x in start..end {
                    let bit = 0x80 >> (x % 8);
                    if one {
                        line[x / 8] |= bit;
                    } else {
                        line[x / 8] &= !bit;
                    }
                }
            }
            Sink::Reduced { k, counts, .. } => {
                if !one {
                    return;
                }
                let mut x = start;
                while x < end {
                    let cell = x / *k;
                    let stop = ((cell + 1) * *k).min(end);
                    counts[cell] += (stop - x) as u32;
                    x = stop;
                }
            }
        }
    }

    fn next_line(&mut self) {
        // A short row: the rest is missing (as the fill).
        self.row += 1;
        self.x = 0;
        if let Sink::Reduced { k, .. } = &self.sink {
            if self.row % *k == 0 {
                self.flush(*k);
            }
        }
    }

    /// Turn a band of `n` rows of counts into bytes.
    fn flush(&mut self, n: usize) {
        let width = self.width;
        if let Sink::Reduced { k, counts, data } = &mut self.sink {
            for (cell, c) in counts.iter_mut().enumerate() {
                let w = (*k).min(width - cell * *k);
                let v = (*c as u64 * 255 / (w * n).max(1) as u64).min(255) as u8;
                if data.len() < MAX_DATA {
                    data.push(v);
                }
                *c = 0;
            }
        }
    }

    /// The samples of `height` rows (missing ones as `ones` says).
    fn finish(mut self, height: usize, ones: bool) -> Bilevel {
        match self.sink {
            Sink::Full {
                mut data,
                stride,
                fill,
            } => {
                data.resize(stride * height, fill);
                Bilevel {
                    width: self.width as u32,
                    height: height as u32,
                    bits: 1,
                    data,
                }
            }
            Sink::Reduced { k, .. } => {
                // The last band, and a last row without its end.
                if self.row % k != 0 || self.x > 0 {
                    self.flush(self.row % k + usize::from(self.x > 0));
                }
                let Sink::Reduced { data: mut d, .. } = self.sink else {
                    unreachable!()
                };
                let (w, h) = (self.width.div_ceil(k), height.div_ceil(k));
                d.resize(w * h, if ones { 255 } else { 0 });
                Bilevel {
                    width: w as u32,
                    height: h as u32,
                    bits: 8,
                    data: d,
                }
            }
        }
    }
}

/// JBIG2 (embedded organization) to bilevel samples, 0 for black.
fn jbig2(data: &[u8], globals: Option<&[u8]>, k: u32) -> Result<Bilevel, ImageError> {
    struct Out(Rows);
    impl hayro_jbig2::Decoder for Out {
        fn push_pixel(&mut self, black: bool) {
            self.0.run(!black, 1);
        }
        fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
            self.0.run(!black, chunk_count as usize * 8);
        }
        fn next_line(&mut self) {
            self.0.next_line();
        }
    }
    let bad = |e: hayro_jbig2::DecodeError| ImageError::Invalid(format!("JBIG2: {e}"));
    let img = hayro_jbig2::Image::new_embedded(data, globals).map_err(bad)?;
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut out = Out(Rows::new(w, h, k, true));
    img.decode(&mut out).map_err(bad)?;
    Ok(out.0.finish(h, true))
}

/// CCITT fax data to bilevel samples (`/BlackIs1` decides which is 1);
/// `height` is used when `/Rows` is not given.
fn ccitt(data: &[u8], c: &Codec, height: u32, reduce: u32) -> Result<Bilevel, ImageError> {
    use hayro_ccitt::{DecodeSettings, DecoderContext, EncodingMode};
    struct Out(Rows, bool);
    impl hayro_ccitt::Decoder for Out {
        fn push_pixel(&mut self, white: bool) {
            self.0.run(white != self.1, 1);
        }
        fn push_pixel_chunk(&mut self, white: bool, chunk_count: u32) {
            self.0.run(white != self.1, chunk_count as usize * 8);
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
    let rows = int("Rows", 0.0).clamp(0.0, 1e7) as u32;
    let black_is_1 = flag("BlackIs1", false);
    let want = if rows > 0 { rows } else { height };
    let settings = DecodeSettings {
        columns,
        rows: want,
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
    // Missing samples are white.
    let white = !black_is_1;
    let mut out = Out(
        Rows::new(columns as usize, want as usize, reduce, white),
        black_is_1,
    );
    let mut ctx = DecoderContext::new(settings);
    // Damaged data: keep the rows decoded before the error.
    let result = hayro_ccitt::decode(data, &mut out, &mut ctx);
    let got = out.0.rows;
    if let (Err(e), 0) = (result, got) {
        return Err(ImageError::Invalid(format!("CCITT: {e}")));
    }
    let h = if rows > 0 { rows as usize } else { got.max(1) };
    Ok(out.0.finish(h, white))
}

struct Jpeg {
    width: u32,
    height: u32,
    components: usize,
    data: Vec<u8>,
}

fn jpeg(data: &[u8]) -> Result<Jpeg, ImageError> {
    let bad = |e: zune_jpeg::errors::DecodeErrors| ImageError::Invalid(format!("JPEG: {e:?}"));
    let mut d = JpegDecoder::new(ZCursor::new(data));
    d.decode_headers().map_err(bad)?;
    let input = d.input_colorspace().unwrap_or(JpegSpace::YCbCr);
    let (out, components) = match input {
        JpegSpace::Luma => (JpegSpace::Luma, 1),
        JpegSpace::CMYK => (JpegSpace::CMYK, 4),
        // Converted to CMYK below.
        JpegSpace::YCCK => (JpegSpace::YCCK, 4),
        _ => (JpegSpace::RGB, 3),
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
    let mut pixels = d.decode().map_err(bad)?;
    if input == JpegSpace::YCCK {
        // As libjpeg: YCbCr to RGB, whose complement is CMY; K as stored.
        // The samples are then used as stored (PDF writers compensate
        // inverted Adobe JPEGs with /Decode).
        for px in pixels.chunks_exact_mut(4) {
            let (y, cb, cr) = (px[0] as f32, px[1] as f32 - 128.0, px[2] as f32 - 128.0);
            let q = |v: f32| 255 - (v.round().clamp(0.0, 255.0) as u8);
            px[0] = q(y + 1.402 * cr);
            px[1] = q(y - 0.344_136 * cb - 0.714_136 * cr);
            px[2] = q(y + 1.772 * cb);
        }
    }
    Ok(Jpeg {
        width: info.width as u32,
        height: info.height as u32,
        components,
        data: pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bilevel_rows() {
        // Full size: runs set bits, missing rows are the fill.
        let mut r = Rows::new(10, 2, 1, false);
        r.run(true, 3);
        r.run(false, 5);
        r.run(true, 4); // 2 past the row's end
        r.next_line();
        let b = r.finish(3, false);
        assert_eq!((b.width, b.height, b.bits), (10, 3, 1));
        assert_eq!(b.data, [0b1110_0000, 0b1100_0000, 0, 0, 0, 0]);
        // Reduced by 2: the share of 1 bits per 2 × 2 block; a last band
        // of one row, and blocks one sample wide at the right edge.
        let mut r = Rows::new(5, 3, 2, true);
        for _ in 0..2 {
            r.run(true, 1);
            r.run(false, 4);
            r.next_line();
        }
        r.run(true, 5);
        let b = r.finish(3, true);
        assert_eq!((b.width, b.height, b.bits), (3, 2, 8));
        assert_eq!(b.data, [127, 0, 0, 255, 255, 255]);
        // Too large for full size: reduced anyway.
        let r = Rows::new(1 << 13, 1 << 13, 1, true);
        assert!(matches!(r.sink, Sink::Reduced { k: 2, .. }));
    }

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
