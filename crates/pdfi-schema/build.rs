//! Compiles the vendored Arlington PDF Model (`schema/arlington/tsv`) into
//! static tables (`$OUT_DIR/arlington.rs`, included by `src/tables.rs`).
//!
//! Predicates (`fn:...`) are reduced to what the runtime understands:
//! version wrappers become version fields, literal value lists become
//! [`Lit`]s, `fn:Eval(...)` and conditional `Required` / `IndirectReference`
//! keep their expression text for `src/expr.rs`, and anything else marks the
//! constraint as open (not checked).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

const DIR: &str = "../../schema/arlington/tsv";

const COLUMNS: [&str; 12] = [
    "Key",
    "Type",
    "SinceVersion",
    "DeprecatedIn",
    "Required",
    "IndirectReference",
    "Inheritable",
    "DefaultValue",
    "PossibleValues",
    "SpecialCase",
    "Link",
    "Note",
];

fn main() {
    println!("cargo:rerun-if-changed={DIR}");
    let mut files: Vec<_> = std::fs::read_dir(DIR)
        .expect("schema/arlington/tsv")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "tsv"))
        .collect();
    files.sort();

    let mut defs = Vec::new();
    for p in &files {
        let name = p.file_stem().unwrap().to_str().unwrap().to_string();
        let text = std::fs::read_to_string(p).unwrap();
        defs.push(parse_file(&name, &text));
    }
    let ids: BTreeMap<String, usize> = defs
        .iter()
        .enumerate()
        .map(|(i, d)| (d.name.clone(), i))
        .collect();

    // A file with only a `*` row is an array or a map; decide by how it is
    // linked.
    let mut from_array = BTreeSet::new();
    for d in &defs {
        for r in &d.rows {
            for s in &r.slots {
                if matches!(s.base.as_str(), "array" | "rectangle" | "matrix") {
                    for l in &s.links {
                        from_array.insert(l.name.clone());
                    }
                }
            }
        }
    }

    let mut out = String::new();
    writeln!(out, "pub static TYPES: [TypeDef; {}] = [", defs.len()).unwrap();
    for d in &defs {
        let indexed = d
            .rows
            .iter()
            .any(|r| r.key.starts_with(|c: char| c.is_ascii_digit()));
        let array =
            indexed || (d.rows.iter().all(|r| r.key == "*") && from_array.contains(&d.name));
        writeln!(
            out,
            "TypeDef {{ name: {:?}, array: {array}, keys: &[",
            d.name
        )
        .unwrap();
        for r in &d.rows {
            emit_row(&mut out, r, &ids, &d.name);
        }
        writeln!(out, "] }},").unwrap();
    }
    writeln!(out, "];").unwrap();

    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("arlington.rs");
    std::fs::write(dest, out).unwrap();
}

struct FileDef {
    name: String,
    rows: Vec<Row>,
}

struct Row {
    key: String,
    slots: Vec<SlotDef>,
    since: u8,
    extension: String,
    deprecated: u8,
    required: String,
    inheritable: bool,
    default: String,
    note: String,
}

struct SlotDef {
    base: String,
    since: u8,
    deprecated: u8,
    indirect: String,
    values: Option<ValuesDef>,
    links: Vec<LinkDef>,
}

#[derive(Default)]
struct ValuesDef {
    lits: Vec<(String, u8, u8)>,
    evals: Vec<String>,
    open: bool,
}

struct LinkDef {
    name: String,
    since: u8,
    only: u8,
}

fn parse_file(name: &str, text: &str) -> FileDef {
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap_or_default().split('\t').collect();
    assert_eq!(header.len(), COLUMNS.len(), "{name}: unexpected columns");
    let mut rows = Vec::new();
    for line in lines.filter(|l| !l.trim().is_empty()) {
        let mut c: Vec<&str> = line.split('\t').map(str::trim).collect();
        c.resize(COLUMNS.len(), "");
        rows.push(parse_row(name, &c));
    }
    FileDef {
        name: name.to_string(),
        rows,
    }
}

fn parse_row(file: &str, c: &[&str]) -> Row {
    let types = split_top(c[1], ';');
    let n = types.len();
    let indirect = per_slot(c[5], n);
    let values = per_slot(c[8], n);
    let links = per_slot(c[10], n);
    let slots = types
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let (base, since, deprecated) = unwrap_versioned(t);
            let values = values[i]
                .as_deref()
                .filter(|v| !v.is_empty())
                .map(|v| parse_values(v, &base));
            SlotDef {
                indirect: parse_indirect(indirect[i].as_deref().unwrap_or("FALSE")),
                links: links[i]
                    .as_deref()
                    .map(|l| {
                        split_top(l, ',')
                            .iter()
                            .filter(|x| !x.is_empty())
                            .filter_map(|x| parse_link(x))
                            .collect()
                    })
                    .unwrap_or_default(),
                base,
                since,
                deprecated,
                values,
            }
        })
        .collect();
    let (since, extension) = parse_since(c[2]);
    Row {
        key: c[0].to_string(),
        slots,
        since,
        extension,
        deprecated: version(c[3]).unwrap_or(0),
        required: match c[4] {
            "TRUE" => "Req::Yes".into(),
            "FALSE" | "" => "Req::No".into(),
            r => match call(r) {
                Some(("IsRequired", args)) if args.len() == 1 => {
                    format!("Req::Cond({:?})", args[0])
                }
                _ => panic!("{file}: unknown Required {r}"),
            },
        },
        inheritable: c[6] == "TRUE",
        default: c[7].to_string(),
        note: c[11].to_string(),
    }
}

/// A `[a];[b]` column split per type slot. A single unbracketed value
/// applies to every slot.
fn per_slot(s: &str, n: usize) -> Vec<Option<String>> {
    if s.is_empty() {
        return vec![None; n];
    }
    let parts = split_top(s, ';');
    let bracketed = parts.iter().all(|p| p.starts_with('[') && p.ends_with(']'));
    if !bracketed {
        return vec![Some(s.to_string()); n];
    }
    let inner: Vec<Option<String>> = parts
        .iter()
        .map(|p| Some(p[1..p.len() - 1].to_string()))
        .collect();
    if inner.len() == n {
        inner
    } else if inner.len() == 1 {
        vec![inner[0].clone(); n]
    } else {
        vec![None; n]
    }
}

fn parse_indirect(s: &str) -> String {
    match s {
        "TRUE" => "Indirect::Must".into(),
        "FALSE" | "" => "Indirect::Any".into(),
        _ => match call(s) {
            Some(("MustBeDirect", a)) if a.is_empty() => "Indirect::MustNot".into(),
            Some(("MustBeIndirect", a)) if a.is_empty() => "Indirect::Must".into(),
            Some(("MustBeDirect", a)) => format!("Indirect::MustNotIf({:?})", a[0]),
            Some(("MustBeIndirect", a)) => format!("Indirect::MustIf({:?})", a[0]),
            _ => "Indirect::Any".into(),
        },
    }
}

/// `type`, `fn:SinceVersion(v,type)`, `fn:Deprecated(v,type)`,
/// `fn:Extension(e,type)`.
fn unwrap_versioned(s: &str) -> (String, u8, u8) {
    match call(s) {
        Some((f, a)) if a.len() == 2 => {
            let (inner, since, dep) = unwrap_versioned(&a[1]);
            let v = version(&a[0]).unwrap_or(0);
            match f {
                "SinceVersion" => (inner, v.max(since), dep),
                "Deprecated" => (inner, since, v),
                _ => (inner, since, dep),
            }
        }
        _ => (s.to_string(), 0, 0),
    }
}

fn parse_link(s: &str) -> Option<LinkDef> {
    match call(s) {
        Some((f, a)) if a.len() == 2 => {
            let mut l = parse_link(&a[1])?;
            let v = version(&a[0]).unwrap_or(0);
            match f {
                "SinceVersion" => l.since = v,
                "IsPDFVersion" => l.only = v,
                _ => {}
            }
            Some(l)
        }
        Some(_) => None,
        None => Some(LinkDef {
            name: s.to_string(),
            since: 0,
            only: 0,
        }),
    }
}

fn parse_values(s: &str, base: &str) -> ValuesDef {
    let mut v = ValuesDef::default();
    for item in split_top(s, ',').iter().filter(|x| !x.is_empty()) {
        add_value(&mut v, item, 0, 0, base);
    }
    v
}

fn add_value(v: &mut ValuesDef, item: &str, since: u8, dep: u8, base: &str) {
    if item == "*" || item.starts_with('[') {
        v.open = true;
        return;
    }
    let Some((f, a)) = call(item) else {
        if let Some(lit) = literal(item, base) {
            v.lits.push((lit, since, dep));
        } else {
            v.open = true;
        }
        return;
    };
    match (f, a.len()) {
        ("Eval", 1) => v.evals.push(a[0].clone()),
        ("SinceVersion", 2) => add_value(v, &a[1], version(&a[0]).unwrap_or(0), dep, base),
        ("Deprecated", 2) => add_value(v, &a[1], since, version(&a[0]).unwrap_or(0), base),
        ("IsPDFVersion" | "BeforeVersion" | "Extension" | "RequiredValue", 2) => {
            add_value(v, &a[1], since, dep, base)
        }
        _ => v.open = true,
    }
}

/// Rust expression of a `Lit` for a value of a slot of type `base`.
fn literal(s: &str, base: &str) -> Option<String> {
    Some(match base {
        "name" => format!("Lit::Name({s:?})"),
        "integer" | "bitmask" => format!("Lit::Int({})", s.parse::<i64>().ok()?),
        "number" => format!("Lit::Num({:?})", s.parse::<f64>().ok()?),
        "boolean" => format!("Lit::Bool({})", s.parse::<bool>().ok()?),
        b if b.starts_with("string") || b == "date" => {
            let t = s.strip_prefix('\'')?.strip_suffix('\'')?;
            format!("Lit::Str({t:?})")
        }
        _ => return None,
    })
}

/// SinceVersion column: `1.3`, `fn:Extension(X,1.7)`, `fn:Extension(X)` or
/// `fn:Eval(fn:Extension(X,1.7) || 2.0)`.
fn parse_since(s: &str) -> (u8, String) {
    if let Some(v) = version(s) {
        return (v, String::new());
    }
    match call(s) {
        Some(("Extension", a)) => (a.get(1).and_then(|v| version(v)).unwrap_or(0), a[0].clone()),
        Some(("Eval", a)) => {
            let mut since = 0;
            let mut ext = String::new();
            for part in a[0].split("||").map(str::trim) {
                if let Some(v) = version(part) {
                    since = v;
                } else if let Some(("Extension", e)) = call(part) {
                    ext = e[0].clone();
                }
            }
            (since, ext)
        }
        _ => (0, String::new()),
    }
}

fn version(s: &str) -> Option<u8> {
    let (a, b) = s.trim().split_once('.')?;
    Some(a.parse::<u8>().ok()? * 10 + b.parse::<u8>().ok()?)
}

/// `fn:Name(a,b)` → `("Name", ["a", "b"])`, when the call spans all of `s`.
fn call(s: &str) -> Option<(&str, Vec<String>)> {
    let rest = s.strip_prefix("fn:")?;
    let open = rest.find('(')?;
    let body = rest[open..].strip_prefix('(')?.strip_suffix(')')?;
    // `fn:A(x) || fn:B(y)` also starts with a name and ends with `)`.
    let mut depth = 0;
    for ch in body.chars() {
        match ch {
            '(' => depth += 1,
            ')' if depth == 0 => return None,
            ')' => depth -= 1,
            _ => {}
        }
    }
    let args = if body.trim().is_empty() {
        Vec::new()
    } else {
        split_top(body, ',')
    };
    Some((&rest[..open], args))
}

/// Split at `sep` outside parentheses, brackets and quotes.
fn split_top(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quoted = false;
    let mut cur = String::new();
    for ch in s.chars() {
        match ch {
            '\'' => quoted = !quoted,
            '(' | '[' if !quoted => depth += 1,
            ')' | ']' if !quoted => depth -= 1,
            c if c == sep && depth == 0 && !quoted => {
                out.push(cur.trim().to_string());
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    out.push(cur.trim().to_string());
    out
}

fn base_variant(b: &str, file: &str) -> &'static str {
    match b {
        "array" => "Array",
        "bitmask" => "Bitmask",
        "boolean" => "Boolean",
        "date" => "Date",
        "dictionary" => "Dictionary",
        "integer" => "Integer",
        "matrix" => "Matrix",
        "name" => "Name",
        "name-tree" => "NameTree",
        "null" => "Null",
        "number" => "Number",
        "number-tree" => "NumberTree",
        "rectangle" => "Rectangle",
        "stream" => "Stream",
        "string" => "String",
        "string-ascii" => "StringAscii",
        "string-byte" => "StringByte",
        "string-text" => "StringText",
        _ => panic!("{file}: unknown type {b}"),
    }
}

fn emit_row(out: &mut String, r: &Row, ids: &BTreeMap<String, usize>, file: &str) {
    write!(out, "KeyDef {{ key: {:?}, slots: &[", r.key).unwrap();
    for s in &r.slots {
        write!(
            out,
            "Slot {{ base: Base::{}, since: {}, deprecated: {}, indirect: {}, values: ",
            base_variant(&s.base, file),
            s.since,
            s.deprecated,
            s.indirect
        )
        .unwrap();
        match &s.values {
            None => out.push_str("None"),
            Some(v) => {
                out.push_str("Some(Values { lits: &[");
                for (l, since, dep) in &v.lits {
                    write!(
                        out,
                        "LitDef {{ lit: {l}, since: {since}, deprecated: {dep} }},"
                    )
                    .unwrap();
                }
                write!(out, "], evals: &{:?}, open: {} }})", v.evals, v.open).unwrap();
            }
        }
        out.push_str(", links: &[");
        for l in &s.links {
            match ids.get(&l.name) {
                Some(id) => write!(
                    out,
                    "Link {{ ty: {id}, since: {}, only: {} }},",
                    l.since, l.only
                )
                .unwrap(),
                None => println!("cargo:warning={file}: unknown link {}", l.name),
            }
        }
        out.push_str("] },");
    }
    writeln!(
        out,
        "], since: {}, extension: {:?}, deprecated: {}, required: {}, inheritable: {}, default: {:?}, note: {:?} }},",
        r.since, r.extension, r.deprecated, r.required, r.inheritable, r.default, r.note
    )
    .unwrap();
}
