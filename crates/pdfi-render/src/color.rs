//! Color spaces and their conversion to RGB.
//!
//! Device spaces convert directly (CMYK naively); CIE-based spaces are
//! treated as their device counterparts (Lab converts through XYZ); ICC
//! profiles are not read (`/Alternate`, else by component count).
//! Separation and DeviceN follow tint transforms of function types 2 and
//! 3; other tint transforms are approximated by the darkest tint.

use pdfi_model::resolve;
use pdfi_syntax::{DictExt, Node, Object, PdfFile};

/// Color spaces nested in each other (Indexed → ICCBased → ...).
const MAX_DEPTH: u32 = 8;
/// Largest lookup table of an Indexed space.
const MAX_LOOKUP: usize = 256 * 32;

#[derive(Clone, Debug, PartialEq)]
pub enum Space {
    Gray,
    Rgb,
    Cmyk,
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
    Pattern,
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
            Space::Gray | Space::Indexed { .. } => 1,
            Space::Rgb | Space::Lab { .. } => 3,
            Space::Cmyk => 4,
            Space::Tint { n, .. } => *n,
            Space::Pattern => 0,
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
            Space::Pattern => [0.5, 0.5, 0.5],
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
    let (x, y, z) = (white[0] * g(fx), white[1] * g(fy), white[2] * g(fz));
    // XYZ (D65-ish, the white point is not adapted) to linear sRGB.
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

fn nums(file: &PdfFile, o: Option<&Object>) -> Option<Vec<f64>> {
    let (_, r) = resolve(file, o?);
    r.as_array()?
        .iter()
        .map(|n| num(file, &n.obj))
        .collect::<Option<Vec<_>>>()
}

/// Parse a color space object (a name or an array).
pub fn parse(file: &PdfFile, o: &Object) -> Space {
    parse_at(file, o, 0)
}

fn parse_at(file: &PdfFile, o: &Object, depth: u32) -> Space {
    if depth > MAX_DEPTH {
        return Space::Unknown(1);
    }
    let (_, r) = resolve(file, o);
    let (family, args): (&[u8], &[Node]) = match &*r {
        Object::Name(n) => (n.as_bytes(), &[]),
        Object::Array(a) => match a.first().map(|n| resolve(file, &n.obj).1) {
            Some(f) => match f.as_name() {
                Some(n) => return parse_family(file, n.as_bytes(), &a[1..], depth),
                None => return Space::Unknown(1),
            },
            None => return Space::Unknown(1),
        },
        _ => return Space::Unknown(1),
    };
    parse_family(file, family, args, depth)
}

fn parse_family(file: &PdfFile, family: &[u8], args: &[Node], depth: u32) -> Space {
    let arg = |i: usize| args.get(i).map(|n| &n.obj);
    match family {
        b"DeviceGray" | b"G" | b"CalGray" => Space::Gray,
        b"DeviceRGB" | b"RGB" | b"CalRGB" => Space::Rgb,
        b"DeviceCMYK" | b"CMYK" => Space::Cmyk,
        b"Pattern" => Space::Pattern,
        b"Lab" => {
            let d = arg(0).map(|o| resolve(file, o).1);
            let d = d.as_ref().and_then(|d| d.as_dict());
            let white = d
                .and_then(|d| nums(file, d.get_key("WhitePoint")))
                .filter(|w| w.len() == 3)
                .map_or([0.9505, 1.0, 1.089], |w| [w[0], w[1], w[2]]);
            let range = d
                .and_then(|d| nums(file, d.get_key("Range")))
                .filter(|r| r.len() == 4)
                .map_or([-100.0, 100.0, -100.0, 100.0], |r| [r[0], r[1], r[2], r[3]]);
            Space::Lab { white, range }
        }
        b"ICCBased" => {
            let s = arg(0).map(|o| resolve(file, o).1);
            let d = s.as_ref().and_then(|s| s.as_stream()).map(|s| &s.dict);
            if let Some(alt) = d.and_then(|d| d.get_key("Alternate")) {
                let a = parse_at(file, alt, depth + 1);
                if !matches!(a, Space::Unknown(_)) {
                    return a;
                }
            }
            match d.and_then(|d| d.get_key("N")).and_then(|n| num(file, n)) {
                Some(1.0) => Space::Gray,
                Some(4.0) => Space::Cmyk,
                _ => Space::Rgb,
            }
        }
        b"Indexed" | b"I" => {
            let base = arg(0).map_or(Space::Rgb, |b| parse_at(file, b, depth + 1));
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
            let alternate = arg(1).map_or(Space::Gray, |a| parse_at(file, a, depth + 1));
            Space::Tint {
                n: names.len().max(1),
                none: !names.is_empty() && names.iter().all(|n| n == b"None"),
                transform: arg(2).and_then(|f| Function::parse(file, f, 0)),
                alternate: Box::new(alternate),
            }
        }
        _ => Space::Unknown(1),
    }
}

/// A PDF function of the kinds tint transforms commonly use.
#[derive(Clone, Debug, PartialEq)]
pub enum Function {
    /// Type 2: `C0 + x^N × (C1 − C0)`.
    Exponential {
        domain: (f64, f64),
        c0: Vec<f64>,
        c1: Vec<f64>,
        n: f64,
    },
    /// Type 3: one function per subdomain.
    Stitching {
        domain: (f64, f64),
        functions: Vec<Function>,
        bounds: Vec<f64>,
        encode: Vec<(f64, f64)>,
    },
    /// One function per output (an array of functions).
    Each(Vec<Function>),
}

impl Function {
    pub fn parse(file: &PdfFile, o: &Object, depth: u32) -> Option<Function> {
        if depth > MAX_DEPTH {
            return None;
        }
        let (_, r) = resolve(file, o);
        if let Some(a) = r.as_array() {
            let fs = a
                .iter()
                .map(|n| Function::parse(file, &n.obj, depth + 1))
                .collect::<Option<Vec<_>>>()?;
            return Some(Function::Each(fs));
        }
        let d = match &*r {
            Object::Dict(d) => d,
            Object::Stream(s) => &s.dict,
            _ => return None,
        };
        let domain = nums(file, d.get_key("Domain"))
            .filter(|v| v.len() >= 2)
            .map_or((0.0, 1.0), |v| (v[0], v[1]));
        match d.get_key("FunctionType").and_then(|t| num(file, t))? as i64 {
            2 => Some(Function::Exponential {
                domain,
                c0: nums(file, d.get_key("C0")).unwrap_or_else(|| vec![0.0]),
                c1: nums(file, d.get_key("C1")).unwrap_or_else(|| vec![1.0]),
                n: d.get_key("N").and_then(|n| num(file, n)).unwrap_or(1.0),
            }),
            3 => {
                let (_, fs) = resolve(file, d.get_key("Functions")?);
                let functions = fs
                    .as_array()?
                    .iter()
                    .map(|n| Function::parse(file, &n.obj, depth + 1))
                    .collect::<Option<Vec<_>>>()?;
                let bounds = nums(file, d.get_key("Bounds")).unwrap_or_default();
                let e = nums(file, d.get_key("Encode")).unwrap_or_default();
                let encode: Vec<(f64, f64)> = e.chunks_exact(2).map(|c| (c[0], c[1])).collect();
                (bounds.len() + 1 == functions.len() && encode.len() == functions.len()).then_some(
                    Function::Stitching {
                        domain,
                        functions,
                        bounds,
                        encode,
                    },
                )
            }
            _ => None,
        }
    }

    pub fn eval(&self, x: &[f64]) -> Option<Vec<f64>> {
        match self {
            Function::Exponential { domain, c0, c1, n } => {
                let t = x
                    .first()?
                    .clamp(domain.0.min(domain.1), domain.0.max(domain.1));
                let p = if *n == 1.0 { t } else { t.powf(*n) };
                Some(c0.iter().zip(c1).map(|(a, b)| a + p * (b - a)).collect())
            }
            Function::Stitching {
                domain,
                functions,
                bounds,
                encode,
            } => {
                let t = x.first()?.clamp(domain.0, domain.1);
                let k = bounds.partition_point(|b| *b <= t);
                let lo = if k == 0 { domain.0 } else { bounds[k - 1] };
                let hi = bounds.get(k).copied().unwrap_or(domain.1);
                let (e0, e1) = encode[k];
                let u = if hi > lo {
                    e0 + (t - lo) * (e1 - e0) / (hi - lo)
                } else {
                    e0
                };
                functions[k].eval(&[u])
            }
            Function::Each(fs) => {
                let mut out = Vec::with_capacity(fs.len());
                for f in fs {
                    out.push(*f.eval(x)?.first()?);
                }
                Some(out)
            }
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

    #[test]
    fn stitching() {
        let f = Function::Stitching {
            domain: (0.0, 1.0),
            functions: vec![
                Function::Exponential {
                    domain: (0.0, 1.0),
                    c0: vec![0.0],
                    c1: vec![1.0],
                    n: 1.0,
                },
                Function::Exponential {
                    domain: (0.0, 1.0),
                    c0: vec![1.0],
                    c1: vec![0.0],
                    n: 1.0,
                },
            ],
            bounds: vec![0.5],
            encode: vec![(0.0, 1.0), (0.0, 1.0)],
        };
        assert_eq!(f.eval(&[0.25]), Some(vec![0.5]));
        assert_eq!(f.eval(&[0.75]), Some(vec![0.5]));
    }
}
