//! Color spaces and their conversion to RGB.
//!
//! Device spaces convert directly (CMYK naively). CalGray, CalRGB and
//! Lab convert through XYZ (white points scaled to D65). ICC profiles are
//! read with moxcms and turned into transforms to sRGB (kept per
//! document); a profile that cannot be used falls back to `/Alternate`,
//! else to the device space of its component count.
//! Separation and DeviceN follow their tint transforms (any function
//! type); a transform that cannot be read is approximated by the darkest
//! tint.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use moxcms::{ColorProfile, DataColorSpace, Layout, TransformF32Executor, TransformOptions};
use pdfi_model::resolve;
use pdfi_syntax::{DictExt, Node, ObjId, Object, PdfFile};

use crate::function::{Function, nums};

/// Color spaces nested in each other (Indexed → ICCBased → ...).
const MAX_DEPTH: u32 = 8;
/// Largest lookup table of an Indexed space.
const MAX_LOOKUP: usize = 256 * 32;
/// Largest ICC profile read.
const MAX_PROFILE: usize = 16 << 20;
/// D65, the white point of sRGB.
const D65: [f64; 3] = [0.9505, 1.0, 1.089];

/// A transform from an ICC profile's data to sRGB.
#[derive(Clone)]
pub struct Icc {
    pub n: usize,
    xf: Arc<TransformF32Executor>,
}

impl std::fmt::Debug for Icc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Icc({})", self.n)
    }
}

impl PartialEq for Icc {
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.xf, &o.xf)
    }
}

impl Icc {
    /// A transform from an ICC profile for `n` components.
    pub fn from_profile(data: &[u8], n: usize) -> Option<Icc> {
        let p = ColorProfile::new_from_slice(data).ok()?;
        let layout = match (p.color_space, n) {
            (DataColorSpace::Gray, 1) => Layout::Gray,
            (DataColorSpace::Rgb, 3) => Layout::Rgb,
            (DataColorSpace::Cmyk, 4) => Layout::Rgba,
            _ => return None,
        };
        let xf = p
            .create_transform_f32(
                layout,
                &ColorProfile::new_srgb(),
                Layout::Rgb,
                TransformOptions::default(),
            )
            .ok()?;
        Some(Icc { n, xf })
    }

    /// Convert pixels of `n` components (0..1) to RGB triples.
    pub fn convert(&self, src: &[f32], dst: &mut [f32]) -> bool {
        self.xf.transform(src, dst).is_ok()
    }
}

/// ICC profiles of a document, by stream (`None`: not usable).
#[derive(Default)]
pub struct Profiles {
    map: Mutex<HashMap<ObjId, Option<Icc>>>,
}

impl Profiles {
    fn get(
        &self,
        file: &PdfFile,
        id: Option<ObjId>,
        s: &pdfi_syntax::Stream,
        n: usize,
    ) -> Option<Icc> {
        let load = || {
            let d = file.decode_stream(id.unwrap_or(ObjId::new(0, 0)), s, Some(MAX_PROFILE));
            if d.stopped_at.is_some() {
                return None;
            }
            Icc::from_profile(&d.data, n)
        };
        let Some(id) = id else { return load() };
        let mut g = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = g.get(&id) {
            return v.clone();
        }
        let v = load();
        g.insert(id, v.clone());
        v
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Space {
    Gray,
    Rgb,
    Cmyk,
    CalGray {
        white: [f64; 3],
        gamma: f64,
    },
    CalRgb {
        white: [f64; 3],
        gamma: [f64; 3],
        /// Column-major as in the dictionary: X, Y, Z of A, then B, then C.
        matrix: [f64; 9],
    },
    Icc(Icc),
    Lab {
        white: [f64; 3],
        /// `a` and `b` ranges.
        range: [f64; 4],
    },
    Indexed {
        base: Box<Space>,
        hival: usize,
        /// `(hival + 1) × base components` bytes.
        lookup: Vec<u8>,
    },
    /// Separation (n = 1) or DeviceN: tints through a transform.
    Tint {
        n: usize,
        /// Painted nothing (`/None`).
        none: bool,
        alternate: Box<Space>,
        transform: Option<Function>,
    },
    /// With the base space of uncolored tiling patterns.
    Pattern(Option<Box<Space>>),
    /// Unknown: drawn as gray from the first component.
    Unknown(usize),
}

/// Whether a color was converted only approximately.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Approx {
    pub tint: bool,
}

impl Space {
    pub fn components(&self) -> usize {
        match self {
            Space::Gray | Space::CalGray { .. } | Space::Indexed { .. } => 1,
            Space::Rgb | Space::CalRgb { .. } | Space::Lab { .. } => 3,
            Space::Icc(i) => i.n,
            Space::Cmyk => 4,
            Space::Tint { n, .. } => *n,
            Space::Pattern(base) => base.as_ref().map_or(0, |b| b.components()),
            Space::Unknown(n) => *n,
        }
    }

    /// The `/Decode` default for `bpc`-bit samples.
    pub fn default_decode(&self, bpc: u32) -> Vec<(f64, f64)> {
        match self {
            Space::Indexed { .. } => vec![(0.0, ((1u32 << bpc.min(16)) - 1) as f64)],
            Space::Lab { range, .. } => {
                vec![(0.0, 100.0), (range[0], range[1]), (range[2], range[3])]
            }
            s => vec![(0.0, 1.0); s.components()],
        }
    }

    /// RGB in 0..1; `None` paints nothing (Separation `/None`).
    pub fn to_rgb(&self, c: &[f64], approx: &mut Approx) -> Option<[f32; 3]> {
        let at = |i: usize| c.get(i).copied().unwrap_or(0.0);
        let u = |v: f64| v.clamp(0.0, 1.0) as f32;
        Some(match self {
            Space::Gray => {
                let g = u(at(0));
                [g, g, g]
            }
            Space::Rgb => [u(at(0)), u(at(1)), u(at(2))],
            Space::Cmyk => cmyk(at(0), at(1), at(2), at(3)),
            Space::CalGray { white, gamma } => {
                let y = at(0).clamp(0.0, 1.0).powf(*gamma);
                xyz_to_srgb([white[0] * y, white[1] * y, white[2] * y], white)
            }
            Space::CalRgb {
                white,
                gamma,
                matrix: m,
            } => {
                let [a, b, c] = [0, 1, 2].map(|k| at(k).clamp(0.0, 1.0).powf(gamma[k]));
                xyz_to_srgb(
                    [
                        m[0] * a + m[3] * b + m[6] * c,
                        m[1] * a + m[4] * b + m[7] * c,
                        m[2] * a + m[5] * b + m[8] * c,
                    ],
                    white,
                )
            }
            Space::Icc(icc) => {
                let src: Vec<f32> = (0..icc.n).map(|k| u(at(k))).collect();
                let mut dst = [0f32; 3];
                if !icc.convert(&src, &mut dst) {
                    return None;
                }
                dst.map(|v| v.clamp(0.0, 1.0))
            }
            Space::Lab { white, .. } => lab(at(0), at(1), at(2), white),
            Space::Indexed {
                base,
                hival,
                lookup,
            } => {
                let i = (at(0).round().max(0.0) as usize).min(*hival);
                let n = base.components();
                let v: Vec<f64> = (0..n)
                    .map(|k| lookup.get(i * n + k).copied().unwrap_or(0) as f64 / 255.0)
                    .collect();
                // Lab lookups hold bytes spread over the component ranges.
                let v = match **base {
                    Space::Lab { range, .. } => lab_bytes(&v, &range),
                    _ => v,
                };
                return base.to_rgb(&v, approx);
            }
            Space::Tint {
                n,
                none,
                alternate,
                transform,
            } => {
                if *none {
                    return None;
                }
                let tints: Vec<f64> = (0..*n).map(at).collect();
                if let Some(out) = transform.as_ref().and_then(|f| f.eval(&tints)) {
                    return alternate.to_rgb(&out, approx);
                }
                approx.tint = true;
                let g = u(1.0 - tints.iter().copied().fold(0.0, f64::max));
                [g, g, g]
            }
            Space::Pattern(base) => match base {
                Some(b) => return b.to_rgb(c, approx),
                None => [0.5, 0.5, 0.5],
            },
            Space::Unknown(_) => {
                let g = u(at(0));
                [g, g, g]
            }
        })
    }
}

fn lab_bytes(v: &[f64], range: &[f64; 4]) -> Vec<f64> {
    let at = |i: usize| v.get(i).copied().unwrap_or(0.0);
    vec![
        at(0) * 100.0,
        range[0] + at(1) * (range[1] - range[0]),
        range[2] + at(2) * (range[3] - range[2]),
    ]
}

pub fn cmyk(c: f64, m: f64, y: f64, k: f64) -> [f32; 3] {
    let u = |v: f64| v.clamp(0.0, 1.0);
    let k = u(k);
    [
        ((1.0 - u(c)) * (1.0 - k)) as f32,
        ((1.0 - u(m)) * (1.0 - k)) as f32,
        ((1.0 - u(y)) * (1.0 - k)) as f32,
    ]
}

fn lab(l: f64, a: f64, b: f64, white: &[f64; 3]) -> [f32; 3] {
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let g = |t: f64| {
        if t >= 6.0 / 29.0 {
            t * t * t
        } else {
            108.0 / 841.0 * (t - 4.0 / 29.0)
        }
    };
    xyz_to_srgb(
        [white[0] * g(fx), white[1] * g(fy), white[2] * g(fz)],
        white,
    )
}

/// CIE XYZ relative to `white` to sRGB (the white point scaled to D65).
fn xyz_to_srgb(xyz: [f64; 3], white: &[f64; 3]) -> [f32; 3] {
    let k = |i: usize| {
        if white[i] > 0.0 {
            D65[i] / white[i]
        } else {
            1.0
        }
    };
    let (x, y, z) = (xyz[0] * k(0), xyz[1] * k(1), xyz[2] * k(2));
    let r = 3.2406 * x - 1.5372 * y - 0.4986 * z;
    let gg = -0.9689 * x + 1.8758 * y + 0.0415 * z;
    let bb = 0.0557 * x - 0.2040 * y + 1.0570 * z;
    let gamma = |v: f64| {
        let v = v.clamp(0.0, 1.0);
        (if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }) as f32
    };
    [gamma(r), gamma(gg), gamma(bb)]
}

fn num(file: &PdfFile, o: &Object) -> Option<f64> {
    resolve(file, o).1.as_f64()
}

/// Parse a color space object (a name or an array).
pub fn parse(file: &PdfFile, profiles: &Profiles, o: &Object) -> Space {
    Parser { file, profiles }.parse(o, 0)
}

struct Parser<'a> {
    file: &'a PdfFile,
    profiles: &'a Profiles,
}

impl Parser<'_> {
    fn parse(&self, o: &Object, depth: u32) -> Space {
        let file = self.file;
        if depth > MAX_DEPTH {
            return Space::Unknown(1);
        }
        let (_, r) = resolve(file, o);
        let (family, args): (&[u8], &[Node]) = match &*r {
            Object::Name(n) => (n.as_bytes(), &[]),
            Object::Array(a) => match a.first().map(|n| resolve(file, &n.obj).1) {
                Some(f) => match f.as_name() {
                    Some(n) => return self.family(n.as_bytes(), &a[1..], depth),
                    None => return Space::Unknown(1),
                },
                None => return Space::Unknown(1),
            },
            _ => return Space::Unknown(1),
        };
        self.family(family, args, depth)
    }

    fn family(&self, family: &[u8], args: &[Node], depth: u32) -> Space {
        let file = self.file;
        let arg = |i: usize| args.get(i).map(|n| &n.obj);
        let dict = || {
            let d = arg(0).map(|o| resolve(file, o).1);
            d.and_then(|d| d.as_dict().cloned())
        };
        let white = |d: &Option<pdfi_syntax::Dict>| {
            d.as_ref()
                .and_then(|d| nums(file, d.get_key("WhitePoint")))
                .filter(|w| w.len() == 3 && w[1] > 0.0)
                .map_or(D65, |w| [w[0], w[1], w[2]])
        };
        match family {
            b"DeviceGray" | b"G" => Space::Gray,
            b"DeviceRGB" | b"RGB" => Space::Rgb,
            b"CalGray" => {
                let d = dict();
                Space::CalGray {
                    white: white(&d),
                    gamma: d
                        .as_ref()
                        .and_then(|d| d.get_key("Gamma"))
                        .and_then(|g| num(file, g))
                        .filter(|g| *g > 0.0)
                        .unwrap_or(1.0),
                }
            }
            b"CalRGB" => {
                let d = dict();
                let gamma = d
                    .as_ref()
                    .and_then(|d| nums(file, d.get_key("Gamma")))
                    .filter(|g| g.len() == 3 && g.iter().all(|v| *v > 0.0))
                    .map_or([1.0; 3], |g| [g[0], g[1], g[2]]);
                let matrix = d
                    .as_ref()
                    .and_then(|d| nums(file, d.get_key("Matrix")))
                    .and_then(|m| <[f64; 9]>::try_from(m).ok())
                    .unwrap_or([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
                Space::CalRgb {
                    white: white(&d),
                    gamma,
                    matrix,
                }
            }
            b"DeviceCMYK" | b"CMYK" => Space::Cmyk,
            b"Pattern" => Space::Pattern(arg(0).map(|b| Box::new(self.parse(b, depth + 1)))),
            b"Lab" => {
                let d = dict();
                let white = white(&d);
                let range = d
                    .as_ref()
                    .and_then(|d| nums(file, d.get_key("Range")))
                    .filter(|r| r.len() == 4)
                    .map_or([-100.0, 100.0, -100.0, 100.0], |r| [r[0], r[1], r[2], r[3]]);
                Space::Lab { white, range }
            }
            b"ICCBased" => {
                let (id, s) = match arg(0).map(|o| resolve(file, o)) {
                    Some((id, s)) => (id, Some(s)),
                    None => (None, None),
                };
                let stream = s.as_ref().and_then(|s| s.as_stream());
                let d = stream.map(|s| &s.dict);
                let n = d
                    .and_then(|d| d.get_key("N"))
                    .and_then(|n| num(file, n))
                    .unwrap_or(3.0) as usize;
                if let Some(icc) = stream.and_then(|s| self.profiles.get(file, id, s, n)) {
                    return Space::Icc(icc);
                }
                if let Some(alt) = d.and_then(|d| d.get_key("Alternate")) {
                    let a = self.parse(alt, depth + 1);
                    if !matches!(a, Space::Unknown(_)) {
                        return a;
                    }
                }
                match n {
                    1 => Space::Gray,
                    4 => Space::Cmyk,
                    _ => Space::Rgb,
                }
            }
            b"Indexed" | b"I" => {
                let base = arg(0).map_or(Space::Rgb, |b| self.parse(b, depth + 1));
                let hival = arg(1)
                    .and_then(|h| num(file, h))
                    .unwrap_or(0.0)
                    .clamp(0.0, 255.0) as usize;
                let lookup = match arg(2).map(|l| resolve(file, l)) {
                    Some((id, l)) => match &*l {
                        Object::String(s) => s.bytes.clone(),
                        Object::Stream(s) => {
                            let id = id.unwrap_or(pdfi_syntax::ObjId::new(0, 0));
                            file.decode_stream(id, s, Some(MAX_LOOKUP)).data
                        }
                        _ => Vec::new(),
                    },
                    None => Vec::new(),
                };
                Space::Indexed {
                    base: Box::new(base),
                    hival,
                    lookup,
                }
            }
            b"Separation" | b"DeviceN" => {
                let separation = family == b"Separation";
                let names: Vec<Vec<u8>> = if separation {
                    arg(0)
                        .and_then(|n| resolve(file, n).1.as_name().map(|n| n.as_bytes().to_vec()))
                        .into_iter()
                        .collect()
                } else {
                    arg(0)
                        .map(|a| resolve(file, a).1)
                        .and_then(|a| {
                            a.as_array().map(|a| {
                                a.iter()
                                    .filter_map(|n| n.obj.as_name().map(|n| n.as_bytes().to_vec()))
                                    .collect()
                            })
                        })
                        .unwrap_or_default()
                };
                let alternate = arg(1).map_or(Space::Gray, |a| self.parse(a, depth + 1));
                Space::Tint {
                    n: names.len().max(1),
                    none: !names.is_empty() && names.iter().all(|n| n == b"None"),
                    transform: arg(2).and_then(|f| Function::parse(file, f)),
                    alternate: Box::new(alternate),
                }
            }
            _ => Space::Unknown(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_colors() {
        let mut a = Approx::default();
        assert_eq!(Space::Gray.to_rgb(&[0.5], &mut a), Some([0.5; 3]));
        assert_eq!(
            Space::Cmyk.to_rgb(&[1.0, 0.0, 0.0, 0.0], &mut a),
            Some([0.0, 1.0, 1.0])
        );
        let white = Space::Lab {
            white: [0.9505, 1.0, 1.089],
            range: [-100.0, 100.0, -100.0, 100.0],
        }
        .to_rgb(&[100.0, 0.0, 0.0], &mut a)
        .unwrap();
        assert!(white.iter().all(|&v| v > 0.99), "{white:?}");
    }
}
