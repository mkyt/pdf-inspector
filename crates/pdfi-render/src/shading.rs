//! Shadings (ISO 32000 8.7.4.5): function-based, axial, radial, and the
//! mesh types (free-form and lattice triangles, Coons and tensor-product
//! patches), painted into a bitmap of device pixels.
//!
//! Axial and radial shadings and meshes with a function look colors up in
//! a table over the parametric variable. Meshes without a function
//! interpolate RGB between their vertices; patches are cut into triangles
//! a few pixels across.

use pdfi_content::geom::{Matrix, apply};
use pdfi_model::resolve;
use pdfi_syntax::{DictExt, ObjId, Object, PdfFile};
use tiny_skia::{IntSize, Pixmap};

use crate::color::{self, Approx, Profiles, Space};
use crate::function::{Bits, Function, nums};

/// Entries of a color table over the parametric variable.
const LUT: usize = 1024;
/// Largest mesh data read.
const MAX_DATA: usize = 64 << 20;
/// Triangles of one mesh.
const MAX_TRIANGLES: usize = 1 << 21;
/// Subdivisions of a patch side.
const MAX_SPLIT: usize = 48;

/// A mesh vertex: a point, and its parametric value (with a function) or
/// its color.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Vertex {
    x: f64,
    y: f64,
    t: f64,
    rgb: [f32; 3],
}

impl Vertex {
    fn lerp(a: &Vertex, b: &Vertex, k: f64) -> Vertex {
        let kf = k as f32;
        Vertex {
            x: a.x + (b.x - a.x) * k,
            y: a.y + (b.y - a.y) * k,
            t: a.t + (b.t - a.t) * k,
            rgb: [0, 1, 2].map(|i| a.rgb[i] + (b.rgb[i] - a.rgb[i]) * kf),
        }
    }
}

/// A tensor-product patch: `p[i][j]` (i along u, j along v) and the
/// colors of the corners (u, v) = (0, 0), (0, 1), (1, 1), (1, 0).
#[derive(Clone, Debug, PartialEq)]
struct Patch {
    p: [[(f64, f64); 4]; 4],
    c: [Vertex; 4],
}

#[derive(Clone, Debug, PartialEq)]
enum Geom {
    Function {
        domain: [f64; 4],
        /// Domain to shading space.
        matrix: Matrix,
    },
    Axial {
        c: [f64; 4],
        t: (f64, f64),
        extend: (bool, bool),
    },
    Radial {
        c: [f64; 6],
        t: (f64, f64),
        extend: (bool, bool),
    },
    Triangles(Vec<[Vertex; 3]>),
    Patches(Vec<Patch>),
}

/// Halvings of the parametric range when sampling gradient stops.
const GRADIENT_DEPTH: u32 = 6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GradientKind {
    /// `/Coords`: x0 y0 x1 y1.
    Axial([f64; 4]),
    /// `/Coords`: x0 y0 r0 x1 y1 r1.
    Radial([f64; 6]),
}

/// A shading as color stops over its parametric variable.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub kind: GradientKind,
    pub extend: (bool, bool),
    /// From 0 (the start circle or point) to 1; RGBA, alpha 0 where the
    /// function failed.
    pub stops: Vec<(f64, [u8; 4])>,
    pub approximate: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shading {
    space: Space,
    function: Option<Function>,
    /// Range of the parametric variable of meshes with a function.
    t_range: (f64, f64),
    background: Option<[u8; 4]>,
    bbox: Option<[f64; 4]>,
    geom: Geom,
    /// Colors were converted approximately.
    pub approximate: bool,
}

fn rgba(c: Option<[f32; 3]>) -> [u8; 4] {
    match c {
        Some(c) => {
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            [q(c[0]), q(c[1]), q(c[2]), 255]
        }
        None => [0; 4],
    }
}

fn invert(m: &Matrix) -> Option<Matrix> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)])
}

impl Shading {
    /// Read a shading dictionary or stream.
    pub fn parse(
        file: &PdfFile,
        profiles: &Profiles,
        id: Option<ObjId>,
        o: &Object,
    ) -> Result<Shading, String> {
        let (rid, r) = resolve(file, o);
        let id = rid.or(id);
        let (d, stream) = match &*r {
            Object::Dict(d) => (d, None),
            Object::Stream(s) => (&s.dict, Some(s)),
            other => {
                return Err(format!(
                    "a shading is a dictionary, not {}",
                    other.type_name()
                ));
            }
        };
        let num = |k: &str| d.get_key(k).and_then(|v| resolve(file, v).1.as_f64());
        let kind = num("ShadingType").ok_or("shading without /ShadingType")? as i64;
        let space = d
            .get_key("ColorSpace")
            .map(|cs| color::parse(file, profiles, cs))
            .ok_or("shading without /ColorSpace")?;
        let function = d.get_key("Function").and_then(|f| Function::parse(file, f));
        if d.get_key("Function").is_some() && function.is_none() {
            return Err("the shading's /Function cannot be read".into());
        }
        let mut approx = Approx::default();
        let background = nums(file, d.get_key("Background"))
            .filter(|b| b.len() == space.components())
            .map(|b| rgba(space.to_rgb(&b, &mut approx)));
        let bbox = nums(file, d.get_key("BBox"))
            .filter(|b| b.len() == 4)
            .map(|b| {
                [
                    b[0].min(b[2]),
                    b[1].min(b[3]),
                    b[0].max(b[2]),
                    b[1].max(b[3]),
                ]
            });
        let domain = nums(file, d.get_key("Domain")).unwrap_or_default();
        let extend = d
            .get_key("Extend")
            .map(|e| resolve(file, e).1)
            .and_then(|e| {
                let a = e.as_array()?;
                let b = |k: usize| matches!(a.get(k).map(|n| &n.obj), Some(Object::Bool(true)));
                Some((b(0), b(1)))
            })
            .unwrap_or((false, false));
        let t = (
            domain.first().copied().unwrap_or(0.0),
            domain.get(1).copied().unwrap_or(1.0),
        );
        let coords = nums(file, d.get_key("Coords")).unwrap_or_default();
        let need_function = |g: Geom| match function {
            Some(_) => Ok(g),
            None => Err("the shading has no /Function".to_string()),
        };
        let mut sh = Shading {
            space,
            function: None,
            t_range: (0.0, 1.0),
            background,
            bbox,
            geom: Geom::Triangles(Vec::new()),
            approximate: false,
        };
        sh.geom = match kind {
            1 => need_function(Geom::Function {
                domain: <[f64; 4]>::try_from(domain).unwrap_or([0.0, 1.0, 0.0, 1.0]),
                matrix: nums(file, d.get_key("Matrix"))
                    .and_then(|m| <[f64; 6]>::try_from(m).ok())
                    .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            })?,
            2 => need_function(Geom::Axial {
                c: <[f64; 4]>::try_from(coords).map_err(|_| "axial /Coords needs 4 numbers")?,
                t,
                extend,
            })?,
            3 => need_function(Geom::Radial {
                c: <[f64; 6]>::try_from(coords).map_err(|_| "radial /Coords needs 6 numbers")?,
                t,
                extend,
            })?,
            4..=7 => {
                let s = stream.ok_or("a mesh shading is a stream")?;
                let data = file.decode_stream(id.unwrap_or(ObjId::new(0, 0)), s, Some(MAX_DATA));
                if let Some(f) = &data.stopped_at {
                    return Err(format!("filter /{} cannot be decoded", f.as_str_lossy()));
                }
                sh.function = function.clone();
                let mesh = Mesh::new(file, d, &sh, kind)?;
                if let Some(t) = mesh.t_range {
                    sh.t_range = t;
                }
                mesh.read(&data.data, &mut sh)?
            }
            k => return Err(format!("unknown /ShadingType {k}")),
        };
        sh.function = function;
        sh.approximate |= approx.tint;
        Ok(sh)
    }

    /// The color of components (or of the function's input).
    fn color(&self, input: &[f64], approx: &mut Approx) -> [u8; 4] {
        match &self.function {
            Some(f) => match f.eval(input) {
                Some(c) => rgba(self.space.to_rgb(&c, approx)),
                None => [0; 4],
            },
            None => rgba(self.space.to_rgb(input, approx)),
        }
    }

    fn table(&self, (t0, t1): (f64, f64), approx: &mut Approx) -> Vec<[u8; 4]> {
        (0..LUT)
            .map(|k| self.color(&[t0 + (t1 - t0) * k as f64 / (LUT - 1) as f64], approx))
            .collect()
    }

    /// An axial shading, or a radial one whose circles lie one inside the
    /// other, as a gradient of color stops (what an SVG gradient can
    /// say); `None` for the others and for shadings with a `/BBox`.
    pub fn gradient(&self) -> Option<Gradient> {
        if self.bbox.is_some() {
            return None;
        }
        let (kind, (t0, t1), extend) = match &self.geom {
            Geom::Axial { c, t, extend } => (GradientKind::Axial(*c), *t, *extend),
            Geom::Radial { c, t, extend } => {
                let d = (c[3] - c[0]).hypot(c[4] - c[1]);
                if d + c[2] > c[5] + 1e-9 && d + c[5] > c[2] + 1e-9 {
                    return None;
                }
                (GradientKind::Radial(*c), *t, *extend)
            }
            _ => return None,
        };
        let mut approx = Approx::default();
        let at = |s: f64, approx: &mut Approx| self.color(&[t0 + (t1 - t0) * s], approx);
        // Stops where the colors stop changing linearly (stitched and
        // sampled functions), at most 2^GRADIENT_DEPTH + 1.
        let mut stops = vec![(0.0, at(0.0, &mut approx))];
        let mut todo = vec![(0.0, 1.0, stops[0].1, at(1.0, &mut approx), 0)];
        while let Some((a, b, ca, cb, depth)) = todo.pop() {
            let m = (a + b) / 2.0;
            let cm = at(m, &mut approx);
            let far = (0..4).any(|k| (cm[k] as i32 - (ca[k] as i32 + cb[k] as i32) / 2).abs() > 2);
            if far && depth < GRADIENT_DEPTH {
                // The right half after the left one (a stack).
                todo.push((m, b, cm, cb, depth + 1));
                todo.push((a, m, ca, cm, depth + 1));
            } else {
                stops.push((b, cb));
            }
        }
        Some(Gradient {
            kind,
            extend,
            stops,
            approximate: approx.tint || self.approximate,
        })
    }

    /// Paint pixels `area` (`x0, y0, x1, y1`) of device space; `m` maps
    /// the shading's space to device pixels. `background`: fill what the
    /// shading leaves (shading patterns). `None` when nothing is painted.
    pub fn paint(
        &self,
        m: &Matrix,
        (x0, y0, x1, y1): (i32, i32, i32, i32),
        background: bool,
    ) -> Option<Pixmap> {
        let (w, h) = ((x1 - x0).max(0) as usize, (y1 - y0).max(0) as usize);
        let size = IntSize::from_wh(w as u32, h as u32)?;
        let inv = invert(m)?;
        let mut buf = vec![0u8; w * h * 4];
        if let (true, Some(bg)) = (background, self.background) {
            for px in buf.chunks_exact_mut(4) {
                px.copy_from_slice(&bg);
            }
        }
        let inside = |x: f64, y: f64| {
            self.bbox
                .is_none_or(|b| x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3])
        };
        let mut approx = Approx::default();
        // Each pixel by its center, in the shading's space.
        let mut each = |f: &mut dyn FnMut(f64, f64) -> Option<[u8; 4]>| {
            for py in 0..h {
                for px in 0..w {
                    let (x, y) = apply(
                        &inv,
                        (x0 + px as i32) as f64 + 0.5,
                        (y0 + py as i32) as f64 + 0.5,
                    );
                    if !inside(x, y) {
                        continue;
                    }
                    if let Some(c) = f(x, y) {
                        let i = (py * w + px) * 4;
                        buf[i..i + 4].copy_from_slice(&c);
                    }
                }
            }
        };
        match &self.geom {
            Geom::Axial { c, t, extend } => {
                let lut = self.table(*t, &mut approx);
                let (dx, dy) = (c[2] - c[0], c[3] - c[1]);
                let len = dx * dx + dy * dy;
                if len == 0.0 {
                    return None;
                }
                each(&mut |x, y| {
                    let s = ((x - c[0]) * dx + (y - c[1]) * dy) / len;
                    lookup(&lut, s, *extend)
                });
            }
            Geom::Radial { c, t, extend } => {
                let lut = self.table(*t, &mut approx);
                let (cx, cy, r0) = (c[0], c[1], c[2]);
                let (dx, dy, dr) = (c[3] - c[0], c[4] - c[1], c[5] - c[2]);
                let a = dx * dx + dy * dy - dr * dr;
                each(&mut |x, y| {
                    let (px, py) = (x - cx, y - cy);
                    let b = px * dx + py * dy + r0 * dr;
                    let cc = px * px + py * py - r0 * r0;
                    let ok = |s: f64| {
                        let within = (0.0..=1.0).contains(&s)
                            || (s < 0.0 && extend.0)
                            || (s > 1.0 && extend.1);
                        within && r0 + s * dr >= 0.0
                    };
                    let s = if a.abs() < 1e-12 {
                        // One root: the circles' radii grow as fast as
                        // their centers move.
                        if b == 0.0 {
                            return None;
                        }
                        let s = cc / (2.0 * b);
                        ok(s).then_some(s)?
                    } else {
                        let disc = b * b - a * cc;
                        if disc < 0.0 {
                            return None;
                        }
                        let r = disc.sqrt();
                        let (s1, s2) = ((b + r) / a, (b - r) / a);
                        let (hi, lo) = (s1.max(s2), s1.min(s2));
                        if ok(hi) {
                            hi
                        } else if ok(lo) {
                            lo
                        } else {
                            return None;
                        }
                    };
                    lookup(&lut, s, (true, true))
                });
            }
            Geom::Function { domain, matrix } => {
                let inv_f = invert(matrix)?;
                let f = self.function.as_ref()?;
                let space = &self.space;
                each(&mut |x, y| {
                    let (u, v) = apply(&inv_f, x, y);
                    if u < domain[0] || u > domain[1] || v < domain[2] || v > domain[3] {
                        return None;
                    }
                    let c = f.eval(&[u, v])?;
                    Some(rgba(space.to_rgb(&c, &mut approx)))
                });
            }
            Geom::Triangles(tris) => {
                let lut = self
                    .function
                    .as_ref()
                    .map(|_| self.table(self.t_range, &mut approx));
                let mut r = Tris::new(&mut buf, w, h, (x0, y0), lut, self.t_range);
                for t in tris {
                    r.fill(&t.map(|v| at(m, &v)), &inside, &inv);
                }
            }
            Geom::Patches(ps) => {
                let lut = self
                    .function
                    .as_ref()
                    .map(|_| self.table(self.t_range, &mut approx));
                let mut r = Tris::new(&mut buf, w, h, (x0, y0), lut, self.t_range);
                for p in ps {
                    r.patch(p, m, &inside, &inv);
                }
            }
        }
        Pixmap::from_vec(buf, size)
    }
}

/// A vertex moved to device space.
fn at(m: &Matrix, v: &Vertex) -> Vertex {
    let (x, y) = apply(m, v.x, v.y);
    Vertex { x, y, ..*v }
}

/// The color at parametric position `s` (0..1 between the ends).
fn lookup(lut: &[[u8; 4]], s: f64, extend: (bool, bool)) -> Option<[u8; 4]> {
    if !s.is_finite() || (s < 0.0 && !extend.0) || (s > 1.0 && !extend.1) {
        return None;
    }
    let k = (s.clamp(0.0, 1.0) * (lut.len() - 1) as f64).round() as usize;
    Some(lut[k])
}

/// Gouraud-shaded triangles onto a buffer.
struct Tris<'a> {
    buf: &'a mut [u8],
    w: usize,
    h: usize,
    origin: (i32, i32),
    lut: Option<Vec<[u8; 4]>>,
    t_range: (f64, f64),
}

impl<'a> Tris<'a> {
    fn new(
        buf: &'a mut [u8],
        w: usize,
        h: usize,
        origin: (i32, i32),
        lut: Option<Vec<[u8; 4]>>,
        t_range: (f64, f64),
    ) -> Self {
        Tris {
            buf,
            w,
            h,
            origin,
            lut,
            t_range,
        }
    }

    /// Fill a triangle given in device space.
    fn fill(&mut self, v: &[Vertex; 3], inside: &dyn Fn(f64, f64) -> bool, inv: &Matrix) {
        let [a, b, c] = v;
        let d = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        if d.abs() < 1e-12 || !d.is_finite() {
            return;
        }
        let (ox, oy) = (self.origin.0 as f64, self.origin.1 as f64);
        let lo = |p: f64, o: f64| ((p - o - 0.5).floor().max(0.0)) as usize;
        let hi = |p: f64, o: f64, n: usize| ((p - o + 0.5).ceil().max(0.0) as usize).min(n);
        let (xa, xb) = (
            lo(a.x.min(b.x).min(c.x), ox),
            hi(a.x.max(b.x).max(c.x), ox, self.w),
        );
        let (ya, yb) = (
            lo(a.y.min(b.y).min(c.y), oy),
            hi(a.y.max(b.y).max(c.y), oy, self.h),
        );
        let (t0, t1) = self.t_range;
        let eps = -1e-9;
        for py in ya..yb {
            let y = py as f64 + oy + 0.5;
            for px in xa..xb {
                let x = px as f64 + ox + 0.5;
                let wa = ((b.x - x) * (c.y - y) - (b.y - y) * (c.x - x)) / d;
                let wb = ((c.x - x) * (a.y - y) - (c.y - y) * (a.x - x)) / d;
                let wc = 1.0 - wa - wb;
                if wa < eps || wb < eps || wc < eps {
                    continue;
                }
                let (sx, sy) = apply(inv, x, y);
                if !inside(sx, sy) {
                    continue;
                }
                let color = match &self.lut {
                    Some(lut) => {
                        let t = wa * a.t + wb * b.t + wc * c.t;
                        let s = if t1 != t0 { (t - t0) / (t1 - t0) } else { 0.0 };
                        lookup(lut, s, (true, true)).unwrap_or([0; 4])
                    }
                    None => {
                        let (wa, wb, wc) = (wa as f32, wb as f32, wc as f32);
                        rgba(Some(
                            [0, 1, 2].map(|k| wa * a.rgb[k] + wb * b.rgb[k] + wc * c.rgb[k]),
                        ))
                    }
                };
                let i = (py * self.w + px) * 4;
                self.buf[i..i + 4].copy_from_slice(&color);
            }
        }
    }

    /// Cut a patch into triangles a few pixels across and fill them.
    fn patch(&mut self, p: &Patch, m: &Matrix, inside: &dyn Fn(f64, f64) -> bool, inv: &Matrix) {
        // Size in pixels: the span of its control points.
        let pts: Vec<(f64, f64)> = p.p.iter().flatten().map(|&(x, y)| apply(m, x, y)).collect();
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &(x, y) in &pts {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        let (ox, oy) = (self.origin.0 as f64, self.origin.1 as f64);
        if x1 < ox || y1 < oy || x0 > ox + self.w as f64 || y0 > oy + self.h as f64 {
            return;
        }
        let span = (x1 - x0).max(y1 - y0);
        let n = ((span / 4.0).ceil() as usize).clamp(1, MAX_SPLIT);
        let bern = |t: f64| {
            let s = 1.0 - t;
            [s * s * s, 3.0 * t * s * s, 3.0 * t * t * s, t * t * t]
        };
        let mut grid = Vec::with_capacity((n + 1) * (n + 1));
        for i in 0..=n {
            let u = i as f64 / n as f64;
            let bu = bern(u);
            for j in 0..=n {
                let v = j as f64 / n as f64;
                let bv = bern(v);
                let (mut x, mut y) = (0.0, 0.0);
                for (a, bu) in bu.iter().enumerate() {
                    for (b, bv) in bv.iter().enumerate() {
                        let k = bu * bv;
                        x += k * p.p[a][b].0;
                        y += k * p.p[a][b].1;
                    }
                }
                let c0 = Vertex::lerp(&p.c[0], &p.c[1], v);
                let c1 = Vertex::lerp(&p.c[3], &p.c[2], v);
                let c = Vertex::lerp(&c0, &c1, u);
                grid.push(at(m, &Vertex { x, y, ..c }));
            }
        }
        let g = |i: usize, j: usize| grid[i * (n + 1) + j];
        for i in 0..n {
            for j in 0..n {
                self.fill(&[g(i, j), g(i + 1, j), g(i + 1, j + 1)], inside, inv);
                self.fill(&[g(i, j), g(i + 1, j + 1), g(i, j + 1)], inside, inv);
            }
        }
    }
}

/// How a mesh's data is laid out.
struct Mesh {
    kind: i64,
    bits_coord: u32,
    bits_comp: u32,
    bits_flag: u32,
    per_row: usize,
    /// Decode ranges: x, y, then each color value.
    decode: Vec<(f64, f64)>,
    /// Color values per vertex (1 with a function).
    n: usize,
    t_range: Option<(f64, f64)>,
}

impl Mesh {
    fn new(file: &PdfFile, d: &pdfi_syntax::Dict, sh: &Shading, kind: i64) -> Result<Self, String> {
        let int = |k: &str| d.get_key(k).and_then(|v| resolve(file, v).1.as_f64());
        let bits_coord = int("BitsPerCoordinate").unwrap_or(0.0) as u32;
        let bits_comp = int("BitsPerComponent").unwrap_or(0.0) as u32;
        let bits_flag = if matches!(kind, 4 | 6 | 7) {
            int("BitsPerFlag").unwrap_or(8.0) as u32
        } else {
            0
        };
        if !matches!(bits_coord, 1 | 2 | 4 | 8 | 12 | 16 | 24 | 32) {
            return Err(format!("/BitsPerCoordinate {bits_coord} is not valid"));
        }
        if !matches!(bits_comp, 1 | 2 | 4 | 8 | 12 | 16) {
            return Err(format!("/BitsPerComponent {bits_comp} is not valid"));
        }
        let n = if sh.function.is_some() {
            1
        } else {
            sh.space.components()
        };
        let decode: Vec<(f64, f64)> = nums(file, d.get_key("Decode"))
            .unwrap_or_default()
            .chunks_exact(2)
            .map(|c| (c[0], c[1]))
            .collect();
        if decode.len() < 2 + n {
            return Err("/Decode needs ranges for x, y and each color value".into());
        }
        let per_row = int("VerticesPerRow").unwrap_or(0.0) as usize;
        if kind == 5 && per_row < 2 {
            return Err("/VerticesPerRow must be at least 2".into());
        }
        Ok(Mesh {
            kind,
            bits_coord,
            bits_comp,
            bits_flag: bits_flag.min(8),
            per_row,
            t_range: sh.function.is_some().then(|| decode[2]),
            decode,
            n,
        })
    }

    fn value(bits: &mut Bits, n: u32, (lo, hi): (f64, f64)) -> Option<f64> {
        let max = ((1u64 << n) - 1) as f64;
        Some(lo + bits.read(n)? as f64 * (hi - lo) / max)
    }

    fn point(&self, bits: &mut Bits) -> Option<(f64, f64)> {
        Some((
            Self::value(bits, self.bits_coord, self.decode[0])?,
            Self::value(bits, self.bits_coord, self.decode[1])?,
        ))
    }

    fn color(&self, bits: &mut Bits, sh: &Shading, approx: &mut Approx) -> Option<Vertex> {
        let mut c = Vec::with_capacity(self.n);
        for k in 0..self.n {
            c.push(Self::value(bits, self.bits_comp, self.decode[2 + k])?);
        }
        Some(if sh.function.is_some() {
            Vertex {
                t: c[0],
                ..Vertex::default()
            }
        } else {
            let rgb = sh.space.to_rgb(&c, approx).unwrap_or([0.0; 3]);
            Vertex {
                rgb,
                ..Vertex::default()
            }
        })
    }

    fn vertex(&self, bits: &mut Bits, sh: &Shading, approx: &mut Approx) -> Option<Vertex> {
        let (x, y) = self.point(bits)?;
        Some(Vertex {
            x,
            y,
            ..self.color(bits, sh, approx)?
        })
    }

    fn read(&self, data: &[u8], sh: &mut Shading) -> Result<Geom, String> {
        let mut bits = Bits::new(data);
        let mut approx = Approx::default();
        let geom = match self.kind {
            4 => {
                let mut tris: Vec<[Vertex; 3]> = Vec::new();
                let mut pending: Vec<Vertex> = Vec::new();
                while tris.len() < MAX_TRIANGLES {
                    let Some(flag) = bits.read(self.bits_flag) else {
                        break;
                    };
                    let Some(v) = self.vertex(&mut bits, sh, &mut approx) else {
                        break;
                    };
                    bits.align();
                    match (flag, tris.last()) {
                        (1, Some(&[_, b, c])) if pending.is_empty() => tris.push([b, c, v]),
                        (2, Some(&[a, _, c])) if pending.is_empty() => tris.push([a, c, v]),
                        _ => {
                            pending.push(v);
                            if pending.len() == 3 {
                                tris.push([pending[0], pending[1], pending[2]]);
                                pending.clear();
                            }
                        }
                    }
                }
                Geom::Triangles(tris)
            }
            5 => {
                let mut vs = Vec::new();
                while vs.len() < MAX_TRIANGLES {
                    let Some(v) = self.vertex(&mut bits, sh, &mut approx) else {
                        break;
                    };
                    vs.push(v);
                }
                let k = self.per_row;
                let rows = vs.len() / k;
                let mut tris = Vec::new();
                for r in 0..rows.saturating_sub(1) {
                    for c in 0..k - 1 {
                        let v = |r: usize, c: usize| vs[r * k + c];
                        tris.push([v(r, c), v(r, c + 1), v(r + 1, c)]);
                        tris.push([v(r, c + 1), v(r + 1, c + 1), v(r + 1, c)]);
                    }
                }
                Geom::Triangles(tris)
            }
            _ => {
                // Points in stream order of a tensor patch (a Coons patch
                // has the first 12).
                const ORDER: [(usize, usize); 16] = [
                    (0, 0),
                    (0, 1),
                    (0, 2),
                    (0, 3),
                    (1, 3),
                    (2, 3),
                    (3, 3),
                    (3, 2),
                    (3, 1),
                    (3, 0),
                    (2, 0),
                    (1, 0),
                    (1, 1),
                    (1, 2),
                    (2, 2),
                    (2, 1),
                ];
                let count = if self.kind == 6 { 12 } else { 16 };
                let mut patches: Vec<Patch> = Vec::new();
                while patches.len() < MAX_TRIANGLES / 64 {
                    let Some(flag) = bits.read(self.bits_flag) else {
                        break;
                    };
                    let mut pts = [(0.0, 0.0); 16];
                    let mut cs = [Vertex::default(); 4];
                    let (first, first_c) = match (flag, patches.last()) {
                        (1..=3, Some(prev)) => {
                            let edge = |k: usize| prev.p[ORDER[k].0][ORDER[k].1];
                            let (from, c) = match flag {
                                1 => ([3, 4, 5, 6], [1, 2]),
                                2 => ([6, 7, 8, 9], [2, 3]),
                                _ => ([9, 10, 11, 0], [3, 0]),
                            };
                            for (k, f) in from.iter().enumerate() {
                                pts[k] = edge(*f);
                            }
                            cs[0] = prev.c[c[0]];
                            cs[1] = prev.c[c[1]];
                            (4, 2)
                        }
                        _ => (0, 0),
                    };
                    let mut ok = true;
                    for p in pts.iter_mut().take(count).skip(first) {
                        match self.point(&mut bits) {
                            Some(v) => *p = v,
                            None => ok = false,
                        }
                    }
                    for c in cs.iter_mut().skip(first_c) {
                        match self.color(&mut bits, sh, &mut approx) {
                            Some(v) => *c = v,
                            None => ok = false,
                        }
                    }
                    bits.align();
                    if !ok {
                        break;
                    }
                    let mut p = [[(0.0, 0.0); 4]; 4];
                    for (k, &(i, j)) in ORDER.iter().enumerate().take(count) {
                        p[i][j] = pts[k];
                    }
                    if self.kind == 6 {
                        coons_interior(&mut p);
                    }
                    patches.push(Patch { p, c: cs });
                }
                Geom::Patches(patches)
            }
        };
        sh.approximate |= approx.tint;
        Ok(geom)
    }
}

/// The inner control points of a Coons patch as a tensor patch.
fn coons_interior(p: &mut [[(f64, f64); 4]; 4]) {
    let f = |a: (usize, usize),
             b: [(usize, usize); 2],
             c: [(usize, usize); 2],
             d: [(usize, usize); 2],
             e: (usize, usize),
             p: &[[(f64, f64); 4]; 4]| {
        let g = |k: usize| {
            let v = |(i, j): (usize, usize)| if k == 0 { p[i][j].0 } else { p[i][j].1 };
            (-4.0 * v(a) + 6.0 * (v(b[0]) + v(b[1])) - 2.0 * (v(c[0]) + v(c[1]))
                + 3.0 * (v(d[0]) + v(d[1]))
                - v(e))
                / 9.0
        };
        (g(0), g(1))
    };
    let q = *p;
    p[1][1] = f(
        (0, 0),
        [(0, 1), (1, 0)],
        [(0, 3), (3, 0)],
        [(3, 1), (1, 3)],
        (3, 3),
        &q,
    );
    p[1][2] = f(
        (0, 3),
        [(0, 2), (1, 3)],
        [(0, 0), (3, 3)],
        [(3, 2), (1, 0)],
        (3, 0),
        &q,
    );
    p[2][1] = f(
        (3, 0),
        [(3, 1), (2, 0)],
        [(3, 3), (0, 0)],
        [(0, 1), (2, 3)],
        (0, 3),
        &q,
    );
    p[2][2] = f(
        (3, 3),
        [(3, 2), (2, 3)],
        [(3, 0), (0, 3)],
        [(0, 2), (2, 0)],
        (0, 0),
        &q,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coons_of_a_square_is_bilinear() {
        // Straight edges with control points at thirds.
        let mut p = [[(0.0, 0.0); 4]; 4];
        for (i, row) in p.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                if i == 0 || i == 3 || j == 0 || j == 3 {
                    *v = (i as f64, j as f64);
                }
            }
        }
        coons_interior(&mut p);
        assert_eq!(p[1][1], (1.0, 1.0));
        assert_eq!(p[2][2], (2.0, 2.0));
        assert_eq!(p[1][2], (1.0, 2.0));
    }
}
