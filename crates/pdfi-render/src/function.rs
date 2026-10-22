//! PDF functions (ISO 32000 7.10): sampled (type 0), exponential
//! (type 2), stitching (type 3) and PostScript calculator (type 4), and
//! arrays of one-output functions. Tint transforms and shadings use them.

use pdfi_model::resolve;
use pdfi_syntax::{DictExt, ObjId, Object, PdfFile};

/// Functions nested in each other (stitching, arrays).
const MAX_DEPTH: u32 = 8;
/// Largest sample table of a sampled function (values).
const MAX_SAMPLES: usize = 1 << 22;
/// Inputs of a sampled function (its corners are 2^m).
const MAX_INPUTS: usize = 8;
/// Operand stack of a calculator function (as the spec's limit).
const MAX_STACK: usize = 100;
/// Instructions of a calculator function.
const MAX_CODE: usize = 1 << 16;

#[derive(Clone, Debug, PartialEq)]
pub enum Function {
    /// Type 0: a table of samples, interpolated (multilinearly).
    Sampled {
        domain: Vec<(f64, f64)>,
        range: Vec<(f64, f64)>,
        size: Vec<usize>,
        encode: Vec<(f64, f64)>,
        decode: Vec<(f64, f64)>,
        /// Samples scaled to 0..1, output by output for each point.
        samples: Vec<f32>,
    },
    /// Type 2: `C0 + x^N × (C1 − C0)`.
    Exponential {
        domain: (f64, f64),
        range: Option<Vec<(f64, f64)>>,
        c0: Vec<f64>,
        c1: Vec<f64>,
        n: f64,
    },
    /// Type 3: one function per subdomain.
    Stitching {
        domain: (f64, f64),
        range: Option<Vec<(f64, f64)>>,
        functions: Vec<Function>,
        bounds: Vec<f64>,
        encode: Vec<(f64, f64)>,
    },
    /// Type 4: a PostScript calculator program.
    Calculator {
        domain: Vec<(f64, f64)>,
        range: Vec<(f64, f64)>,
        code: Vec<Ins>,
    },
    /// One function per output (an array of functions).
    Each(Vec<Function>),
}

fn num(file: &PdfFile, o: &Object) -> Option<f64> {
    resolve(file, o).1.as_f64()
}

pub(crate) fn nums(file: &PdfFile, o: Option<&Object>) -> Option<Vec<f64>> {
    let (_, r) = resolve(file, o?);
    r.as_array()?
        .iter()
        .map(|n| num(file, &n.obj))
        .collect::<Option<Vec<_>>>()
}

fn pairs(v: &[f64]) -> Vec<(f64, f64)> {
    v.chunks_exact(2).map(|c| (c[0], c[1])).collect()
}

fn clip(v: f64, (a, b): (f64, f64)) -> f64 {
    v.clamp(a.min(b), a.max(b))
}

fn clip_range(out: &mut [f64], range: Option<&[(f64, f64)]>) {
    if let Some(r) = range {
        for (v, r) in out.iter_mut().zip(r) {
            *v = clip(*v, *r);
        }
    }
}

impl Function {
    pub fn parse(file: &PdfFile, o: &Object) -> Option<Function> {
        Self::parse_at(file, o, 0)
    }

    fn parse_at(file: &PdfFile, o: &Object, depth: u32) -> Option<Function> {
        if depth > MAX_DEPTH {
            return None;
        }
        let (id, r) = resolve(file, o);
        if let Some(a) = r.as_array() {
            let fs = a
                .iter()
                .map(|n| Self::parse_at(file, &n.obj, depth + 1))
                .collect::<Option<Vec<_>>>()?;
            return Some(Function::Each(fs));
        }
        let (d, stream) = match &*r {
            Object::Dict(d) => (d, None),
            Object::Stream(s) => (&s.dict, Some(s)),
            _ => return None,
        };
        let domain = pairs(&nums(file, d.get_key("Domain")).unwrap_or_default());
        let range = nums(file, d.get_key("Range")).map(|r| pairs(&r));
        let data = || {
            let s = stream?;
            let d = file.decode_stream(id.unwrap_or(ObjId::new(0, 0)), s, Some(64 << 20));
            d.stopped_at.is_none().then_some(d.data)
        };
        match d.get_key("FunctionType").and_then(|t| num(file, t))? as i64 {
            0 => {
                let range = range?;
                let m = domain.len();
                if m == 0 || m > MAX_INPUTS || range.is_empty() {
                    return None;
                }
                let size: Vec<usize> = nums(file, d.get_key("Size"))?
                    .iter()
                    .map(|&s| s.clamp(1.0, 1e9) as usize)
                    .collect();
                if size.len() != m {
                    return None;
                }
                let n = range.len();
                let points = size
                    .iter()
                    .try_fold(1usize, |a, &s| a.checked_mul(s))
                    .filter(|&p| p.saturating_mul(n) <= MAX_SAMPLES)?;
                let bps = d.get_key("BitsPerSample").and_then(|b| num(file, b))? as u32;
                if !matches!(bps, 1 | 2 | 4 | 8 | 12 | 16 | 24 | 32) {
                    return None;
                }
                let encode = nums(file, d.get_key("Encode"))
                    .map(|e| pairs(&e))
                    .filter(|e| e.len() == m)
                    .unwrap_or_else(|| size.iter().map(|&s| (0.0, (s - 1) as f64)).collect());
                let decode = nums(file, d.get_key("Decode"))
                    .map(|e| pairs(&e))
                    .filter(|e| e.len() == n)
                    .unwrap_or_else(|| range.clone());
                let bytes = data()?;
                let max = ((1u64 << bps) - 1) as f64;
                let mut bits = Bits::new(&bytes);
                let mut samples = Vec::with_capacity(points * n);
                for _ in 0..points * n {
                    samples.push((bits.read(bps).unwrap_or(0) as f64 / max) as f32);
                }
                Some(Function::Sampled {
                    domain,
                    range,
                    size,
                    encode,
                    decode,
                    samples,
                })
            }
            2 => Some(Function::Exponential {
                domain: domain.first().copied().unwrap_or((0.0, 1.0)),
                range,
                c0: nums(file, d.get_key("C0")).unwrap_or_else(|| vec![0.0]),
                c1: nums(file, d.get_key("C1")).unwrap_or_else(|| vec![1.0]),
                n: d.get_key("N").and_then(|n| num(file, n)).unwrap_or(1.0),
            }),
            3 => {
                let (_, fs) = resolve(file, d.get_key("Functions")?);
                let functions = fs
                    .as_array()?
                    .iter()
                    .map(|n| Self::parse_at(file, &n.obj, depth + 1))
                    .collect::<Option<Vec<_>>>()?;
                let bounds = nums(file, d.get_key("Bounds")).unwrap_or_default();
                let encode = pairs(&nums(file, d.get_key("Encode")).unwrap_or_default());
                (bounds.len() + 1 == functions.len() && encode.len() == functions.len()).then(
                    || Function::Stitching {
                        domain: domain.first().copied().unwrap_or((0.0, 1.0)),
                        range,
                        functions,
                        bounds,
                        encode,
                    },
                )
            }
            4 => {
                let range = range?;
                let code = compile(&data()?)?;
                (!domain.is_empty()).then_some(Function::Calculator {
                    domain,
                    range,
                    code,
                })
            }
            _ => None,
        }
    }

    /// Number of outputs, when the function says.
    pub fn outputs(&self) -> Option<usize> {
        match self {
            Function::Sampled { range, .. } | Function::Calculator { range, .. } => {
                Some(range.len())
            }
            Function::Exponential { range, c0, .. } => {
                Some(range.as_ref().map_or(c0.len(), Vec::len))
            }
            Function::Stitching {
                range, functions, ..
            } => range
                .as_ref()
                .map(Vec::len)
                .or_else(|| functions.first()?.outputs()),
            Function::Each(fs) => Some(fs.len()),
        }
    }

    pub fn eval(&self, x: &[f64]) -> Option<Vec<f64>> {
        match self {
            Function::Sampled {
                domain,
                range,
                size,
                encode,
                decode,
                samples,
            } => {
                let m = domain.len();
                let n = range.len();
                // Position in the table along each input, and its cell.
                let mut lo = [0usize; MAX_INPUTS];
                let mut frac = [0f64; MAX_INPUTS];
                for k in 0..m {
                    let (d0, d1) = domain[k];
                    let v = clip(x.get(k).copied().unwrap_or(d0), domain[k]);
                    let (e0, e1) = encode[k];
                    let e = if d1 != d0 {
                        e0 + (v - d0) * (e1 - e0) / (d1 - d0)
                    } else {
                        e0
                    };
                    let e = e.clamp(0.0, (size[k] - 1) as f64);
                    lo[k] = (e.floor() as usize).min(size[k].saturating_sub(2));
                    frac[k] = if size[k] > 1 { e - lo[k] as f64 } else { 0.0 };
                }
                let mut out = vec![0.0; n];
                for corner in 0..1usize << m {
                    let mut w = 1.0;
                    let mut index = 0;
                    let mut stride = 1;
                    for k in 0..m {
                        let up = corner >> k & 1 == 1;
                        if up && size[k] == 1 {
                            w = 0.0;
                            break;
                        }
                        w *= if up { frac[k] } else { 1.0 - frac[k] };
                        index += (lo[k] + usize::from(up)) * stride;
                        stride *= size[k];
                    }
                    if w == 0.0 {
                        continue;
                    }
                    for (j, o) in out.iter_mut().enumerate() {
                        *o += w * samples.get(index * n + j).copied().unwrap_or(0.0) as f64;
                    }
                }
                for (j, o) in out.iter_mut().enumerate() {
                    let (a, b) = decode[j];
                    *o = clip(a + *o * (b - a), range[j]);
                }
                Some(out)
            }
            Function::Exponential {
                domain,
                range,
                c0,
                c1,
                n,
            } => {
                let t = clip(*x.first()?, *domain);
                let p = if *n == 1.0 { t } else { t.powf(*n) };
                let mut out: Vec<f64> = c0.iter().zip(c1).map(|(a, b)| a + p * (b - a)).collect();
                clip_range(&mut out, range.as_deref());
                Some(out)
            }
            Function::Stitching {
                domain,
                range,
                functions,
                bounds,
                encode,
            } => {
                let t = clip(*x.first()?, *domain);
                let k = bounds.partition_point(|b| *b <= t);
                let lo = if k == 0 { domain.0 } else { bounds[k - 1] };
                let hi = bounds.get(k).copied().unwrap_or(domain.1);
                let (e0, e1) = encode[k];
                let u = if hi > lo {
                    e0 + (t - lo) * (e1 - e0) / (hi - lo)
                } else {
                    e0
                };
                let mut out = functions[k].eval(&[u])?;
                clip_range(&mut out, range.as_deref());
                Some(out)
            }
            Function::Calculator {
                domain,
                range,
                code,
            } => {
                let mut stack: Vec<Val> = domain
                    .iter()
                    .enumerate()
                    .map(|(k, d)| Val::Num(clip(x.get(k).copied().unwrap_or(d.0), *d)))
                    .collect();
                run(code, &mut stack)?;
                let n = range.len();
                if stack.len() < n {
                    return None;
                }
                let base = stack.len() - n;
                Some(
                    (0..n)
                        .map(|j| clip(stack[base + j].num(), range[j]))
                        .collect(),
                )
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

/// Big-endian bit reader.
pub(crate) struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Bits { data, pos: 0 }
    }

    /// Read `n` (≤ 32) bits; `None` past the end.
    pub fn read(&mut self, n: u32) -> Option<u32> {
        if self.pos + n as usize > self.data.len() * 8 {
            self.pos = self.data.len() * 8;
            return None;
        }
        let mut v = 0u64;
        let mut left = n;
        while left > 0 {
            let byte = self.data[self.pos / 8];
            let off = (self.pos % 8) as u32;
            let take = (8 - off).min(left);
            let bits = (byte >> (8 - off - take)) & ((1u16 << take) - 1) as u8;
            v = (v << take) | bits as u64;
            self.pos += take as usize;
            left -= take;
        }
        Some(v as u32)
    }

    /// Skip to the next byte.
    pub fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Val {
    Num(f64),
    Bool(bool),
}

impl Val {
    fn num(self) -> f64 {
        match self {
            Val::Num(v) => v,
            Val::Bool(b) => f64::from(u8::from(b)),
        }
    }

    fn int(self) -> i64 {
        self.num() as i64
    }
}

/// A calculator instruction; `If` / `IfElse` jump within the code.
#[derive(Clone, Debug, PartialEq)]
pub enum Ins {
    Push(f64),
    Bool(bool),
    Op(&'static str),
    /// Pop a boolean; when false, jump to the index.
    JumpUnless(usize),
    Jump(usize),
}

const OPS: &[&str] = &[
    "abs", "add", "atan", "ceiling", "cos", "cvi", "cvr", "div", "exp", "floor", "idiv", "ln",
    "log", "mod", "mul", "neg", "round", "sin", "sqrt", "sub", "truncate", "and", "bitshift", "eq",
    "ge", "gt", "le", "lt", "ne", "not", "or", "xor", "copy", "dup", "exch", "index", "pop",
    "roll",
];

#[derive(Debug, PartialEq)]
enum Tok<'a> {
    Open,
    Close,
    Word(&'a str),
}

fn tokens(src: &[u8]) -> Option<Vec<Tok<'_>>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let c = src[i];
        match c {
            b'{' => out.push(Tok::Open),
            b'}' => out.push(Tok::Close),
            b'%' => {
                while i < src.len() && src[i] != b'\n' && src[i] != b'\r' {
                    i += 1;
                }
            }
            c if c.is_ascii_whitespace() || c == 0 => {}
            _ => {
                let start = i;
                while i < src.len()
                    && !src[i].is_ascii_whitespace()
                    && !matches!(src[i], b'{' | b'}' | b'%' | 0)
                {
                    i += 1;
                }
                out.push(Tok::Word(std::str::from_utf8(&src[start..i]).ok()?));
                continue;
            }
        }
        i += 1;
    }
    Some(out)
}

/// Compile a calculator program (`{ … }`) to flat code.
fn compile(src: &[u8]) -> Option<Vec<Ins>> {
    let toks = tokens(src)?;
    let mut at = 0;
    if toks.first() != Some(&Tok::Open) {
        return None;
    }
    at += 1;
    let mut code = Vec::new();
    block(&toks, &mut at, &mut code, 0)?;
    Some(code)
}

/// Compile until the matching `}` (consumed).
fn block(toks: &[Tok], at: &mut usize, code: &mut Vec<Ins>, depth: u32) -> Option<()> {
    if depth > 64 {
        return None;
    }
    // Procedures waiting for `if` / `ifelse`: their code.
    let mut procs: Vec<Vec<Ins>> = Vec::new();
    loop {
        if code.len() > MAX_CODE {
            return None;
        }
        let t = toks.get(*at)?;
        *at += 1;
        match t {
            Tok::Close => {
                for p in procs {
                    code.extend(p);
                }
                return Some(());
            }
            Tok::Open => {
                let mut p = Vec::new();
                block(toks, at, &mut p, depth + 1)?;
                procs.push(p);
            }
            Tok::Word(w) => {
                let w = *w;
                if w == "if" || w == "ifelse" {
                    let n = if w == "if" { 1 } else { 2 };
                    if procs.len() < n {
                        return None;
                    }
                    let mut ps = procs.split_off(procs.len() - n);
                    // Earlier procedures are pushed operands of nothing.
                    for p in procs.drain(..) {
                        code.extend(p);
                    }
                    let base = code.len();
                    if n == 1 {
                        let a = ps.pop()?;
                        code.push(Ins::JumpUnless(base + 1 + a.len()));
                        code.extend(relocate(a, base + 1));
                    } else {
                        let b = ps.pop()?;
                        let a = ps.pop()?;
                        let else_at = base + 2 + a.len();
                        code.push(Ins::JumpUnless(else_at));
                        code.extend(relocate(a, base + 1));
                        code.push(Ins::Jump(else_at + b.len()));
                        code.extend(relocate(b, else_at));
                    }
                    continue;
                }
                for p in procs.drain(..) {
                    code.extend(p);
                }
                code.push(match w {
                    "true" => Ins::Bool(true),
                    "false" => Ins::Bool(false),
                    _ => match OPS.iter().find(|&&o| o == w) {
                        Some(o) => Ins::Op(o),
                        None => Ins::Push(w.parse().ok()?),
                    },
                });
            }
        }
    }
}

/// Code compiled at 0, moved to `base`.
fn relocate(code: Vec<Ins>, base: usize) -> impl Iterator<Item = Ins> {
    code.into_iter().map(move |i| match i {
        Ins::JumpUnless(t) => Ins::JumpUnless(t + base),
        Ins::Jump(t) => Ins::Jump(t + base),
        i => i,
    })
}

fn run(code: &[Ins], s: &mut Vec<Val>) -> Option<()> {
    let mut pc = 0;
    let mut steps = 0u32;
    while pc < code.len() {
        steps += 1;
        if steps > 1 << 20 || s.len() > MAX_STACK {
            return None;
        }
        match &code[pc] {
            Ins::Push(v) => s.push(Val::Num(*v)),
            Ins::Bool(b) => s.push(Val::Bool(*b)),
            Ins::JumpUnless(t) => {
                if let Val::Bool(false) = s.pop()? {
                    pc = *t;
                    continue;
                }
            }
            Ins::Jump(t) => {
                pc = *t;
                continue;
            }
            Ins::Op(o) => op(o, s)?,
        }
        pc += 1;
    }
    Some(())
}

fn op(o: &str, s: &mut Vec<Val>) -> Option<()> {
    match o {
        "dup" => {
            let v = *s.last()?;
            s.push(v);
        }
        "pop" => {
            s.pop()?;
        }
        "exch" => {
            let b = s.pop()?;
            let a = s.pop()?;
            s.extend([b, a]);
        }
        "copy" => {
            let n = s.pop()?.int();
            if n < 0 || n as usize > s.len() {
                return None;
            }
            let from = s.len() - n as usize;
            s.extend_from_within(from..);
        }
        "index" => {
            let n = s.pop()?.int();
            if n < 0 || n as usize >= s.len() {
                return None;
            }
            let v = s[s.len() - 1 - n as usize];
            s.push(v);
        }
        "roll" => {
            let j = s.pop()?.int();
            let n = s.pop()?.int();
            if n < 0 || n as usize > s.len() {
                return None;
            }
            if n > 0 {
                let from = s.len() - n as usize;
                let k = j.rem_euclid(n) as usize;
                s[from..].rotate_right(k);
            }
        }
        "abs" | "ceiling" | "cos" | "cvi" | "cvr" | "exp" | "floor" | "ln" | "log" | "neg"
        | "round" | "sin" | "sqrt" | "truncate" | "not" => {
            let a = s.pop()?;
            s.push(match (o, a) {
                ("not", Val::Bool(b)) => Val::Bool(!b),
                ("not", v) => Val::Num(!v.int() as f64),
                (_, v) => {
                    let x = v.num();
                    Val::Num(match o {
                        "abs" => x.abs(),
                        "ceiling" => x.ceil(),
                        "cos" => x.to_radians().cos(),
                        "cvi" | "truncate" => x.trunc(),
                        "cvr" => x,
                        "exp" => x.exp(),
                        "floor" => x.floor(),
                        "ln" => x.ln(),
                        "log" => x.log10(),
                        "neg" => -x,
                        "round" => (x + 0.5).floor(),
                        "sin" => x.to_radians().sin(),
                        _ => x.sqrt(),
                    })
                }
            });
        }
        _ => {
            let b = s.pop()?;
            let a = s.pop()?;
            let (x, y) = (a.num(), b.num());
            s.push(match o {
                "add" => Val::Num(x + y),
                "sub" => Val::Num(x - y),
                "mul" => Val::Num(x * y),
                "div" => Val::Num(if y == 0.0 { 0.0 } else { x / y }),
                "idiv" => Val::Num(if b.int() == 0 {
                    0.0
                } else {
                    (a.int() / b.int()) as f64
                }),
                "mod" => Val::Num(if b.int() == 0 {
                    0.0
                } else {
                    (a.int() % b.int()) as f64
                }),
                "atan" => {
                    let d = x.atan2(y).to_degrees();
                    Val::Num(if d < 0.0 { d + 360.0 } else { d })
                }
                "exp" => Val::Num(x.powf(y)),
                "bitshift" => {
                    let (v, k) = (a.int(), b.int().clamp(-63, 63));
                    Val::Num(if k >= 0 { v << k } else { v >> -k } as f64)
                }
                "and" | "or" | "xor" => match (a, b) {
                    (Val::Bool(p), Val::Bool(q)) => Val::Bool(match o {
                        "and" => p && q,
                        "or" => p || q,
                        _ => p ^ q,
                    }),
                    _ => {
                        let (p, q) = (a.int(), b.int());
                        Val::Num(match o {
                            "and" => p & q,
                            "or" => p | q,
                            _ => p ^ q,
                        } as f64)
                    }
                },
                "eq" | "ne" => {
                    let same = match (a, b) {
                        (Val::Bool(p), Val::Bool(q)) => p == q,
                        (Val::Num(_), Val::Num(_)) => x == y,
                        _ => false,
                    };
                    Val::Bool(same == (o == "eq"))
                }
                "ge" => Val::Bool(x >= y),
                "gt" => Val::Bool(x > y),
                "le" => Val::Bool(x <= y),
                "lt" => Val::Bool(x < y),
                _ => return None,
            });
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calc(src: &str, x: &[f64], n: usize) -> Option<Vec<f64>> {
        Function::Calculator {
            domain: vec![(-1e9, 1e9); x.len()],
            range: vec![(-1e9, 1e9); n],
            code: compile(src.as_bytes())?,
        }
        .eval(x)
    }

    #[test]
    fn calculator() {
        assert_eq!(calc("{ 2 mul 1 add }", &[3.0], 1), Some(vec![7.0]));
        assert_eq!(
            calc("{ dup 0.5 gt { pop 1 } { pop 0 } ifelse }", &[0.7], 1),
            Some(vec![1.0])
        );
        assert_eq!(
            calc("{ dup 0.5 gt { pop 1 } { pop 0 } ifelse }", &[0.2], 1),
            Some(vec![0.0])
        );
        assert_eq!(calc("{ dup 0 lt { neg } if }", &[-4.0], 1), Some(vec![4.0]));
        // A CMYK tint transform: 1 input, 4 outputs.
        assert_eq!(
            calc(
                "{ dup 0.2 mul exch dup 0 mul exch dup 1 mul exch 0 mul }",
                &[0.5],
                4
            ),
            Some(vec![0.1, 0.0, 0.5, 0.0])
        );
        assert_eq!(
            calc("{ 1 2 3 3 1 roll }", &[], 3),
            Some(vec![3.0, 1.0, 2.0])
        );
        assert_eq!(
            calc("{ 1 2 3 2 index }", &[], 4),
            Some(vec![1.0, 2.0, 3.0, 1.0])
        );
        assert_eq!(
            calc("{ 1 2 2 copy }", &[], 4),
            Some(vec![1.0, 2.0, 1.0, 2.0])
        );
        assert_eq!(calc("{ 0 1 atan }", &[], 1), Some(vec![0.0]));
        assert_eq!(calc("{ 7 2 idiv 7 2 mod }", &[], 2), Some(vec![3.0, 1.0]));
        assert_eq!(calc("{ pop }", &[], 1), None);
    }

    #[test]
    fn sampled() {
        // 1 input, 2 outputs, 2 samples per output: (0, 1) and (1, 0).
        let f = Function::Sampled {
            domain: vec![(0.0, 1.0)],
            range: vec![(0.0, 1.0); 2],
            size: vec![2],
            encode: vec![(0.0, 1.0)],
            decode: vec![(0.0, 1.0); 2],
            samples: vec![0.0, 1.0, 1.0, 0.0],
        };
        assert_eq!(f.eval(&[0.25]), Some(vec![0.25, 0.75]));
        // 2 inputs: bilinear.
        let f = Function::Sampled {
            domain: vec![(0.0, 1.0); 2],
            range: vec![(0.0, 1.0)],
            size: vec![2, 2],
            encode: vec![(0.0, 1.0); 2],
            decode: vec![(0.0, 1.0)],
            samples: vec![0.0, 1.0, 1.0, 1.0],
        };
        assert_eq!(f.eval(&[0.5, 0.5]), Some(vec![0.75]));
    }

    #[test]
    fn stitching() {
        let e = |c0: f64, c1: f64| Function::Exponential {
            domain: (0.0, 1.0),
            range: None,
            c0: vec![c0],
            c1: vec![c1],
            n: 1.0,
        };
        let f = Function::Stitching {
            domain: (0.0, 1.0),
            range: None,
            functions: vec![e(0.0, 1.0), e(1.0, 0.0)],
            bounds: vec![0.5],
            encode: vec![(0.0, 1.0), (0.0, 1.0)],
        };
        assert_eq!(f.eval(&[0.25]), Some(vec![0.5]));
        assert_eq!(f.eval(&[0.75]), Some(vec![0.5]));
    }

    #[test]
    fn bits() {
        let mut b = Bits::new(&[0b1010_1100, 0xFF]);
        assert_eq!(b.read(3), Some(0b101));
        assert_eq!(b.read(7), Some(0b0110011));
        assert_eq!(b.read(7), None);
    }
}
