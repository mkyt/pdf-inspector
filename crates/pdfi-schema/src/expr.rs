//! A subset of Arlington's predicate language, evaluated with three-valued
//! logic: anything not understood (paths through other objects, most
//! `fn:` functions) is unknown, and an unknown condition is never reported.
//!
//! Supported: `@Key` (value of a key or array element of the current
//! object), literals (numbers, names, `'strings'`, `true` / `false`),
//! `== != < <= > >=`, `&& ||`, `+ - * mod`, parentheses and
//! `fn:IsPresent(Key)`, `fn:Not`, `fn:Eval`, `fn:SinceVersion`,
//! `fn:BeforeVersion`, `fn:IsPDFVersion`, `fn:ArrayLength`,
//! `fn:StringLength`.

use pdfi_syntax::Object;

use crate::tables::Ver;

/// What an expression can see.
pub trait Env {
    /// PDF version of the document.
    fn version(&self) -> Ver;
    /// Value of a key (or array index) of the current object, references
    /// resolved. `None` when absent.
    fn lookup(&self, key: &str) -> Option<Object>;
}

#[derive(Clone, Debug, PartialEq)]
enum V {
    Num(f64),
    Bool(bool),
    Name(String),
    Str(Vec<u8>),
    Unknown,
}

/// Evaluate a condition (`Required`, `IndirectReference`). Version-gated
/// parts are conjunctions: `fn:SinceVersion(1.6,X)` holds when the
/// document is 1.6 or later and X holds. `None` when it cannot be decided.
pub fn eval(expr: &str, env: &dyn Env) -> Option<bool> {
    truth(&value(expr, env, false))
}

/// Evaluate a constraint (`PossibleValues`). Version-gated parts are
/// implications: `fn:BeforeVersion(1.3,X)` holds in 1.3 and later, and
/// before that only when X holds.
pub fn holds(expr: &str, env: &dyn Env) -> Option<bool> {
    truth(&value(expr, env, true))
}

fn value(expr: &str, env: &dyn Env, implies: bool) -> V {
    let Some(toks) = tokenize(expr) else {
        return V::Unknown;
    };
    let mut p = Parser {
        toks,
        pos: 0,
        env,
        implies,
    };
    let v = p.or();
    if p.pos == p.toks.len() { v } else { V::Unknown }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Str(Vec<u8>),
    /// Bare word: a name, a key, `true` / `false`, `mod`.
    Word(String),
    /// `@Key`.
    At(String),
    Call(String, Vec<String>),
    Op(&'static str),
    Open,
    Close,
}

fn tokenize(s: &str) -> Option<Vec<Tok>> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    let word_char = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b':');
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if s[i..].starts_with("fn:") {
            let open = i + s[i..].find('(')?;
            let close = matching(b, open)?;
            out.push(Tok::Call(
                s[i + 3..open].to_string(),
                split_args(&s[open + 1..close]),
            ));
            i = close + 1;
            continue;
        }
        if let Some(op) = ["==", "!=", "<=", ">=", "&&", "||", "<", ">", "+", "*"]
            .into_iter()
            .find(|op| s[i..].starts_with(op))
        {
            out.push(Tok::Op(op));
            i += op.len();
            continue;
        }
        match c {
            b'(' => {
                out.push(Tok::Open);
                i += 1;
            }
            b')' => {
                out.push(Tok::Close);
                i += 1;
            }
            b'\'' => {
                let end = i + 1 + s[i + 1..].find('\'')?;
                out.push(Tok::Str(b[i + 1..end].to_vec()));
                i = end + 1;
            }
            b'@' => {
                let start = i + 1;
                i = start;
                while i < b.len() && word_char(b[i]) {
                    i += 1;
                }
                // Repeating array element: `@1*`.
                if i > start
                    && b.get(i) == Some(&b'*')
                    && b[start..i].iter().all(u8::is_ascii_digit)
                {
                    i += 1;
                }
                out.push(Tok::At(s[start..i].to_string()));
            }
            b'-' if !matches!(out.last(), Some(Tok::Num(_) | Tok::At(_) | Tok::Close)) => {
                out.push(Tok::Op("neg"));
                i += 1;
            }
            b'-' => {
                out.push(Tok::Op("-"));
                i += 1;
            }
            c if word_char(c) => {
                let start = i;
                while i < b.len() && word_char(b[i]) {
                    i += 1;
                }
                let w = &s[start..i];
                out.push(match w.parse::<f64>() {
                    Ok(n) => Tok::Num(n),
                    Err(_) => Tok::Word(w.to_string()),
                });
            }
            _ => return None,
        }
    }
    Some(out)
}

fn matching(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut quoted = false;
    for (i, &c) in b.iter().enumerate().skip(open) {
        match c {
            b'\'' => quoted = !quoted,
            b'(' if !quoted => depth += 1,
            b')' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut quoted = false;
    let mut cur = String::new();
    for ch in s.chars() {
        match ch {
            '\'' => quoted = !quoted,
            '(' | '[' if !quoted => depth += 1,
            ')' | ']' if !quoted => depth -= 1,
            ',' if depth == 0 && !quoted => {
                out.push(cur.trim().to_string());
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    if !cur.trim().is_empty() || !out.is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

struct Parser<'a> {
    toks: Vec<Tok>,
    pos: usize,
    env: &'a dyn Env,
    /// Version gates are implications (see [`holds`]).
    implies: bool,
}

impl Parser<'_> {
    fn peek_op(&self, op: &str) -> bool {
        matches!(self.toks.get(self.pos), Some(Tok::Op(o)) if *o == op)
            || (op == "mod" && matches!(self.toks.get(self.pos), Some(Tok::Word(w)) if w == "mod"))
    }

    fn or(&mut self) -> V {
        let mut v = self.and();
        while self.peek_op("||") {
            self.pos += 1;
            let r = self.and();
            v = match (truth(&v), truth(&r)) {
                (Some(true), _) | (_, Some(true)) => V::Bool(true),
                (Some(false), Some(false)) => V::Bool(false),
                _ => V::Unknown,
            };
        }
        v
    }

    fn and(&mut self) -> V {
        let mut v = self.cmp();
        while self.peek_op("&&") {
            self.pos += 1;
            let r = self.cmp();
            v = match (truth(&v), truth(&r)) {
                (Some(false), _) | (_, Some(false)) => V::Bool(false),
                (Some(true), Some(true)) => V::Bool(true),
                _ => V::Unknown,
            };
        }
        v
    }

    fn cmp(&mut self) -> V {
        let l = self.add();
        for op in ["==", "!=", "<=", ">=", "<", ">"] {
            if self.peek_op(op) {
                self.pos += 1;
                let r = self.add();
                return compare(op, &l, &r);
            }
        }
        l
    }

    fn add(&mut self) -> V {
        let mut v = self.mul();
        loop {
            let op = if self.peek_op("+") {
                "+"
            } else if self.peek_op("-") {
                "-"
            } else {
                return v;
            };
            self.pos += 1;
            let r = self.mul();
            v = arith(op, &v, &r);
        }
    }

    fn mul(&mut self) -> V {
        let mut v = self.unary();
        loop {
            let op = if self.peek_op("*") {
                "*"
            } else if self.peek_op("mod") {
                "mod"
            } else {
                return v;
            };
            self.pos += 1;
            let r = self.unary();
            v = arith(op, &v, &r);
        }
    }

    fn unary(&mut self) -> V {
        if self.peek_op("neg") {
            self.pos += 1;
            return match self.unary() {
                V::Num(n) => V::Num(-n),
                _ => V::Unknown,
            };
        }
        self.primary()
    }

    fn primary(&mut self) -> V {
        let Some(t) = self.toks.get(self.pos).cloned() else {
            return V::Unknown;
        };
        self.pos += 1;
        match t {
            Tok::Num(n) => V::Num(n),
            Tok::Str(s) => V::Str(s),
            Tok::Word(w) => match w.as_str() {
                "true" => V::Bool(true),
                "false" => V::Bool(false),
                _ => V::Name(w),
            },
            Tok::At(k) => self.at(&k),
            Tok::Call(f, args) => self.call(&f, &args),
            Tok::Open => {
                let v = self.or();
                if self.toks.get(self.pos) == Some(&Tok::Close) {
                    self.pos += 1;
                    v
                } else {
                    self.pos = self.toks.len() + 1;
                    V::Unknown
                }
            }
            Tok::Close | Tok::Op(_) => {
                self.pos = self.toks.len() + 1;
                V::Unknown
            }
        }
    }

    fn at(&self, key: &str) -> V {
        if key.contains("::") {
            return V::Unknown;
        }
        match self.env.lookup(key) {
            Some(Object::Integer(i)) => V::Num(i as f64),
            Some(Object::Real(r)) => V::Num(r),
            Some(Object::Bool(b)) => V::Bool(b),
            Some(Object::Name(n)) => V::Name(n.as_str_lossy().into_owned()),
            Some(Object::String(s)) => V::Str(s.bytes),
            _ => V::Unknown,
        }
    }

    fn call(&self, f: &str, args: &[String]) -> V {
        let version = |a: &str| -> Option<Ver> {
            let (x, y) = a.trim().split_once('.')?;
            Some(x.parse::<u8>().ok()? * 10 + y.parse::<u8>().ok()?)
        };
        let ver = self.env.version();
        let gated = |cond: bool| -> V {
            match (cond, args.get(1)) {
                (false, _) => V::Bool(self.implies && args.len() == 2),
                (true, None) => V::Bool(true),
                (true, Some(e)) => value(e, self.env, self.implies),
            }
        };
        match (f, args.len()) {
            ("Eval", 1) => value(&args[0], self.env, self.implies),
            ("Not", 1) => match truth(&value(&args[0], self.env, self.implies)) {
                Some(b) => V::Bool(!b),
                None => V::Unknown,
            },
            ("IsPresent", 1) if is_local_key(&args[0]) => {
                V::Bool(self.env.lookup(&args[0]).is_some_and(|o| o != Object::Null))
            }
            ("SinceVersion" | "BeforeVersion" | "IsPDFVersion", 1 | 2) if ver != 0 => {
                let Some(v) = version(&args[0]) else {
                    return V::Unknown;
                };
                gated(match f {
                    "SinceVersion" => ver >= v,
                    "BeforeVersion" => ver < v,
                    _ => ver == v,
                })
            }
            ("ArrayLength", 1) if is_local_key(&args[0]) => match self.env.lookup(&args[0]) {
                Some(Object::Array(a)) => V::Num(a.len() as f64),
                _ => V::Unknown,
            },
            ("StringLength", 1) if is_local_key(&args[0]) => match self.env.lookup(&args[0]) {
                Some(Object::String(s)) => V::Num(s.bytes.len() as f64),
                _ => V::Unknown,
            },
            _ => V::Unknown,
        }
    }
}

fn is_local_key(k: &str) -> bool {
    !k.is_empty() && !k.contains("::") && !k.contains('@') && !k.contains('(')
}

fn truth(v: &V) -> Option<bool> {
    match v {
        V::Bool(b) => Some(*b),
        _ => None,
    }
}

fn arith(op: &str, l: &V, r: &V) -> V {
    let (V::Num(a), V::Num(b)) = (l, r) else {
        return V::Unknown;
    };
    V::Num(match op {
        "+" => a + b,
        "-" => a - b,
        "*" => a * b,
        _ if *b == 0.0 => return V::Unknown,
        _ => a % b,
    })
}

fn compare(op: &str, l: &V, r: &V) -> V {
    use std::cmp::Ordering;
    let ord = match (l, r) {
        (V::Num(a), V::Num(b)) => a.partial_cmp(b),
        (V::Name(a), V::Name(b)) => Some(a.cmp(b)),
        (V::Str(a), V::Str(b)) => Some(a.cmp(b)),
        (V::Bool(a), V::Bool(b)) => Some(a.cmp(b)),
        // `@Version==1.4`: a name that looks like a number.
        (V::Name(a), V::Num(b)) => a.parse::<f64>().ok().and_then(|a| a.partial_cmp(b)),
        (V::Num(a), V::Name(b)) => b.parse::<f64>().ok().and_then(|b| a.partial_cmp(&b)),
        (V::Unknown, _) | (_, V::Unknown) => None,
        // Different kinds: only (in)equality is meaningful.
        _ => {
            return match op {
                "==" => V::Bool(false),
                "!=" => V::Bool(true),
                _ => V::Unknown,
            };
        }
    };
    let Some(o) = ord else {
        return V::Unknown;
    };
    V::Bool(match op {
        "==" => o == Ordering::Equal,
        "!=" => o != Ordering::Equal,
        "<" => o == Ordering::Less,
        "<=" => o != Ordering::Greater,
        ">" => o == Ordering::Greater,
        _ => o != Ordering::Less,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdfi_syntax::{Dict, Name, Node};

    struct D(Dict, Ver);

    impl Env for D {
        fn version(&self) -> Ver {
            self.1
        }
        fn lookup(&self, key: &str) -> Option<Object> {
            self.0.get(key.as_bytes()).map(|n| n.obj.clone())
        }
    }

    fn env(pairs: &[(&str, Object)], v: Ver) -> D {
        let mut d = Dict::new();
        for (k, o) in pairs {
            d.insert(Name::from(*k), Node::synthetic(o.clone()));
        }
        D(d, v)
    }

    #[test]
    fn comparisons_and_logic() {
        let e = env(
            &[
                ("Count", Object::Integer(3)),
                ("Rotate", Object::Integer(270)),
                ("Type", Object::Name(Name::from("Page"))),
                ("CA", Object::Real(0.5)),
            ],
            17,
        );
        assert_eq!(eval("@Count>=0", &e), Some(true));
        assert_eq!(eval("(@Rotate mod 90)==0", &e), Some(true));
        assert_eq!(eval("(@CA>=0.0) && (@CA<=1.0)", &e), Some(true));
        assert_eq!(eval("@Type!=Template", &e), Some(true));
        assert_eq!(eval("@Type==Template", &e), Some(false));
        assert_eq!(eval("(@Volume>=-1) && (@Volume<=1)", &e), None);
        assert_eq!(eval("fn:IsPresent(Count)", &e), Some(true));
        assert_eq!(eval("fn:Not(fn:IsPresent(Kids))", &e), Some(true));
        assert_eq!(
            eval("fn:SinceVersion(2.0) || fn:IsPresent(Encrypt)", &e),
            Some(false)
        );
        assert_eq!(
            eval("fn:SinceVersion(1.6,fn:IsPresent(Count))", &e),
            Some(true)
        );
        assert_eq!(eval("fn:IsPresent(trailer::Catalog::PieceInfo)", &e), None);
        // Unknown on one side of `||` stays unknown unless the other is true.
        assert_eq!(eval("fn:FontHasLatinChars() || @Count==3", &e), Some(true));
        assert_eq!(eval("fn:FontHasLatinChars() && @Count==3", &e), None);
        assert_eq!(eval("fn:FontHasLatinChars() && @Count==4", &e), Some(false));
    }

    #[test]
    fn version_gates() {
        let e = env(&[("Colors", Object::Integer(5))], 17);
        let c = "(@Colors>=1) && fn:BeforeVersion(1.3,fn:Eval(@Colors<=4))";
        assert_eq!(holds(c, &e), Some(true));
        assert_eq!(eval(c, &e), Some(false));
        let old = env(&[("Colors", Object::Integer(5))], 12);
        assert_eq!(holds(c, &old), Some(false));
    }

    #[test]
    fn arithmetic() {
        let e = env(&[("Length", Object::Integer(128))], 20);
        assert_eq!(
            eval(
                "(@Length>=40) && (@Length<=128) && ((@Length mod 8)==0)",
                &e
            ),
            Some(true)
        );
        assert_eq!(eval("@Length-100>20", &e), Some(true));
        assert_eq!(eval("@Length*2==256", &e), Some(true));
    }
}
