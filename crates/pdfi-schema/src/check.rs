//! Type inference and validation against the Arlington tables.
//!
//! Types flow along references: the trailer is a `FileTrailer` (or
//! `XRefStream`), its `/Root` links to `Catalog`, and so on. Where a key
//! links to several candidate types the object's own entries decide
//! ([`Checker::choose`]). Objects reached that way are remembered per
//! document; any other object is typed by a guess from its `/Type`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use pdfi_syntax::xref::XrefKind;
use pdfi_syntax::{
    self as syn, DictExt, IndirectObject, Name, Node, ObjId, Object, PdfFile, Severity, Span,
};

use crate::expr::{self, Env};
use crate::tables::*;

pub fn type_def(id: TypeId) -> &'static TypeDef {
    &TYPES[id as usize]
}

pub fn type_by_name(name: &str) -> Option<TypeId> {
    static INDEX: OnceLock<HashMap<&'static str, TypeId>> = OnceLock::new();
    INDEX
        .get_or_init(|| {
            TYPES
                .iter()
                .enumerate()
                .map(|(i, t)| (t.name, i as TypeId))
                .collect()
        })
        .get(name)
        .copied()
}

/// Dictionary types by the value of their `/Type` entry.
fn by_type_key(name: &[u8]) -> &'static [TypeId] {
    static INDEX: OnceLock<HashMap<&'static [u8], Vec<TypeId>>> = OnceLock::new();
    INDEX
        .get_or_init(|| {
            let mut m: HashMap<&'static [u8], Vec<TypeId>> = HashMap::new();
            for (i, t) in TYPES.iter().enumerate().filter(|(_, t)| !t.array) {
                let Some(row) = t.keys.iter().find(|k| k.key == "Type") else {
                    continue;
                };
                for s in row.slots {
                    for l in s.values.iter().flat_map(|v| v.lits) {
                        if let Lit::Name(n) = l.lit {
                            m.entry(n.as_bytes()).or_default().push(i as TypeId);
                        }
                    }
                }
            }
            m
        })
        .get(name)
        .map_or(&[], Vec::as_slice)
}

pub fn version_text(v: Ver) -> String {
    format!("{}.{}", v / 10, v % 10)
}

fn parse_version(s: &[u8]) -> Option<Ver> {
    let s = std::str::from_utf8(s).ok()?;
    let (a, b) = s.trim().split_once('.')?;
    Some(a.parse::<u8>().ok()? * 10 + b.get(..1)?.parse::<u8>().ok()?)
}

/// A slot of a key, identifying the leaf types of a name or number tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SlotRef {
    pub ty: TypeId,
    pub key: u16,
    pub slot: u8,
}

impl SlotRef {
    pub fn slot(self) -> &'static Slot {
        &type_def(self.ty).keys[self.key as usize].slots[self.slot as usize]
    }
}

/// The schema type of a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    Def(TypeId),
    /// A node of a name tree (`number: false`) or number tree whose values
    /// are typed by the links of `leaf`.
    Tree {
        number: bool,
        leaf: SlotRef,
    },
    /// `/Kids` of a tree node.
    TreeKids {
        number: bool,
        leaf: SlotRef,
    },
    /// `/Names` or `/Nums` of a tree node: key, value, key, value, …
    TreeEntries {
        number: bool,
        leaf: SlotRef,
    },
}

impl Ty {
    pub fn name(self) -> String {
        let tree = |n: bool| if n { "NumberTree" } else { "NameTree" };
        match self {
            Ty::Def(t) => type_def(t).name.to_string(),
            Ty::Tree { number, leaf } => match leaf.slot().links.first() {
                Some(l) => format!("{}<{}>", tree(number), type_def(l.ty).name),
                None => tree(number).to_string(),
            },
            Ty::TreeKids { number, .. } => format!("{} Kids", tree(number)),
            Ty::TreeEntries { number, .. } => {
                format!("{} {}", tree(number), if number { "Nums" } else { "Names" })
            }
        }
    }

    /// Reference to the specification, when the model gives one.
    pub fn note(self) -> &'static str {
        match self {
            Ty::Def(t) => type_def(t).keys.first().map_or("", |k| k.note),
            _ => "",
        }
    }

    pub fn def(self) -> Option<&'static TypeDef> {
        match self {
            Ty::Def(t) => Some(type_def(t)),
            _ => None,
        }
    }
}

/// How the type of an object was determined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Reached by following typed references.
    Context,
    /// Guessed from the object's own `/Type`.
    Guess,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IssueCode {
    WrongType,
    MissingKey,
    UnknownKey,
    BadValue,
    PredicateFailed,
    NotIndirect,
    NotDirect,
    TooNew,
    Deprecated,
    ArrayLength,
    BadDate,
    BadString,
    MissingObject,
    BadTree,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seg {
    Key(Name),
    Index(u32),
}

/// `/Pages/Kids[3]`.
pub fn path_text(path: &[Seg]) -> String {
    let mut s = String::new();
    for p in path {
        match p {
            Seg::Key(k) => {
                s.push('/');
                s.push_str(&k.as_str_lossy());
            }
            Seg::Index(i) => s.push_str(&format!("[{i}]")),
        }
    }
    s
}

#[derive(Clone, Debug)]
pub struct Issue {
    pub severity: Severity,
    pub code: IssueCode,
    pub message: String,
    pub obj: Option<ObjId>,
    /// Location within `obj` (or the trailer).
    pub path: Vec<Seg>,
    pub span: Option<Span>,
}

/// A dictionary or array being checked.
#[derive(Clone, Copy)]
pub enum Container<'o> {
    Dict(&'o syn::Dict),
    Array(&'o [Node]),
}

impl<'o> Container<'o> {
    pub fn of(o: &'o Object) -> Option<Self> {
        match o {
            Object::Array(a) => Some(Container::Array(a)),
            o => o.as_dict().map(Container::Dict),
        }
    }

    pub fn len(self) -> usize {
        match self {
            Container::Dict(d) => d.len(),
            Container::Array(a) => a.len(),
        }
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub fn child(self, i: usize) -> Option<(Option<&'o Name>, &'o Node)> {
        match self {
            Container::Dict(d) => d.get_index(i).map(|(k, n)| (Some(k), n)),
            Container::Array(a) => a.get(i).map(|n| (None, n)),
        }
    }

    /// A dictionary key or an array index.
    fn get(self, key: &str) -> Option<&'o Node> {
        match self {
            Container::Dict(d) => d.get(key.as_bytes()),
            Container::Array(a) => a.get(key.parse::<usize>().ok()?),
        }
    }

    fn seg(self, i: usize) -> Seg {
        match self.child(i) {
            Some((Some(k), _)) => Seg::Key(k.clone()),
            _ => Seg::Index(i as u32),
        }
    }
}

/// Schema information about one child of a container.
#[derive(Default)]
pub struct Entry {
    /// The row describing the key or element.
    pub def: Option<&'static KeyDef>,
    /// The alternative the value matched.
    pub slot: Option<&'static Slot>,
    /// Type of the value (a direct container or the referenced object).
    pub ty: Option<Ty>,
    /// Problems with this entry; paths are relative to the entry.
    pub issues: Vec<Issue>,
}

/// Per-document schema state: the version and the types learned so far.
#[derive(Default)]
pub struct DocSchema {
    version: OnceLock<Ver>,
    types: Mutex<HashMap<ObjId, Ty>>,
    seeded: OnceLock<()>,
}

impl DocSchema {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn checker<'a>(&'a self, file: &'a PdfFile) -> Checker<'a> {
        let version = *self.version.get_or_init(|| doc_version(file));
        Checker {
            file,
            state: self,
            version,
        }
    }

    /// Objects with a type learned from context.
    pub fn known(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ObjId, Ty>> {
        self.types.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Header version, or the catalog's `/Version` when later.
fn doc_version(file: &PdfFile) -> Ver {
    let header = file
        .header()
        .version
        .as_deref()
        .and_then(|v| parse_version(v.as_bytes()))
        .unwrap_or(0);
    let catalog = file
        .trailer()
        .get_key("Root")
        .and_then(|r| file.resolve(r).ok())
        .and_then(|c| {
            c.as_dict()
                .and_then(|d| d.get_name("Version"))
                .and_then(|n| parse_version(n.as_bytes()))
        })
        .unwrap_or(0);
    header.max(catalog)
}

pub struct Checker<'a> {
    file: &'a PdfFile,
    state: &'a DocSchema,
    version: Ver,
}

/// A value with references followed. Keeps the referenced object alive.
struct Resolved {
    held: Option<Arc<IndirectObject>>,
    missing: bool,
}

impl Resolved {
    fn obj<'n>(&'n self, node: &'n Object) -> &'n Object {
        match &self.held {
            Some(o) => &o.node.obj,
            None if self.missing => &Object::Null,
            None => node,
        }
    }
}

impl<'a> Checker<'a> {
    /// Document version (0 when unknown).
    pub fn version(&self) -> Ver {
        self.version
    }

    pub fn trailer_type(&self) -> Ty {
        let stream = self
            .file
            .revisions()
            .first()
            .is_some_and(|r| r.kind == XrefKind::Stream);
        let name = if stream { "XRefStream" } else { "FileTrailer" };
        Ty::Def(type_by_name(name).expect("Arlington defines the trailer"))
    }

    /// Type of an indirect object: learned from context, else guessed.
    pub fn type_of(&self, id: ObjId) -> Option<(Ty, Origin)> {
        self.seed();
        if let Some(t) = self.state.lock().get(&id) {
            return Some((*t, Origin::Context));
        }
        let o = self.file.get(id).ok()?;
        self.guess(&o.node.obj).map(|t| (t, Origin::Guess))
    }

    /// Type of an object learned from context only.
    pub fn known_type(&self, id: ObjId) -> Option<Ty> {
        self.state.lock().get(&id).copied()
    }

    pub fn record(&self, id: ObjId, ty: Ty) {
        self.state.lock().entry(id).or_insert(ty);
    }

    /// Type the trailer's and the catalog's references, so the main objects
    /// are known before anything was browsed.
    fn seed(&self) {
        self.state.seeded.get_or_init(|| {
            let tt = self.trailer_type();
            let t = Container::Dict(self.file.trailer());
            for i in 0..t.len() {
                self.entry(tt, t, i, false);
            }
            let Some(root) = self
                .file
                .trailer()
                .get_key("Root")
                .and_then(Object::as_ref_id)
            else {
                return;
            };
            let (Some(ty), Ok(o)) = (self.known_type(root), self.file.get(root)) else {
                return;
            };
            if let Some(c) = Container::of(&o.node.obj) {
                for i in 0..c.len() {
                    self.entry(ty, c, i, false);
                }
            }
        });
    }

    /// Guess a type from `/Type` alone.
    pub fn guess(&self, o: &Object) -> Option<Ty> {
        let d = o.as_dict()?;
        let cands = by_type_key(d.get_name("Type")?.as_bytes());
        let links: Vec<Link> = cands
            .iter()
            .map(|&ty| Link {
                ty,
                since: 0,
                only: 0,
            })
            .collect();
        self.choose(&links, o).map(Ty::Def)
    }

    /// Pick the candidate that fits `o` best.
    pub fn choose(&self, links: &[Link], o: &Object) -> Option<TypeId> {
        let v = self.version;
        let usable: Vec<TypeId> = links
            .iter()
            .filter(|l| v == 0 || ((l.only == 0 || l.only == v) && l.since <= v))
            .map(|l| l.ty)
            .collect();
        let cands: Vec<TypeId> = if usable.is_empty() {
            links.iter().map(|l| l.ty).collect()
        } else {
            usable
        };
        match cands.as_slice() {
            [] => None,
            [one] => Some(*one),
            _ => {
                let c = Container::of(o)?;
                let mut best = None;
                for &t in &cands {
                    let s = score(type_def(t), c);
                    if best.is_none_or(|(b, _)| s > b) {
                        best = Some((s, t));
                    }
                }
                best.map(|(_, t)| t)
            }
        }
    }

    fn resolve(&self, o: &Object) -> Resolved {
        match o {
            Object::Ref(id) => match self.file.get(*id) {
                Ok(held) => Resolved {
                    held: Some(held),
                    missing: false,
                },
                Err(_) => Resolved {
                    held: None,
                    missing: true,
                },
            },
            _ => Resolved {
                held: None,
                missing: false,
            },
        }
    }

    /// Schema information about child `i` of container `c` of type `ty`.
    /// Issues are collected only when `issues` is set.
    pub fn entry(&self, ty: Ty, c: Container, i: usize, issues: bool) -> Entry {
        let Some((key, node)) = c.child(i) else {
            return Entry::default();
        };
        let res = self.resolve(&node.obj);
        let value = res.obj(&node.obj);
        let mut e = Entry::default();
        let mut out = Vec::new();
        match ty {
            Ty::Def(t) => {
                let def = type_def(t);
                let row = match key {
                    Some(k) => find_key(def, k.as_bytes()),
                    None => find_index(def, i),
                };
                let Some(row) = row else {
                    if issues && key.is_some() && !def.keys.iter().any(|k| k.key == "*") {
                        out.push(self.issue(
                            Severity::Info,
                            IssueCode::UnknownKey,
                            format!(
                                "/{} is not a key of {}",
                                key.map(|k| k.as_str_lossy().into_owned()).unwrap_or_default(),
                                def.name
                            ),
                            node,
                        ));
                    }
                    e.issues = out;
                    return e;
                };
                e.def = Some(row);
                // A null entry of a dictionary is the same as no entry.
                if key.is_some() && matches!(value, Object::Null) {
                    if issues && res.missing {
                        out.push(self.missing_object(node));
                    }
                    e.issues = out;
                    return e;
                }
                let key_index = def
                    .keys
                    .iter()
                    .position(|k| std::ptr::eq(k, row))
                    .unwrap_or(0);
                let slot_i = row.slots.iter().position(|s| fits(s.base, value));
                e.slot = slot_i.map(|s| &row.slots[s]);
                e.ty = slot_i.and_then(|si| {
                    let s = &row.slots[si];
                    let leaf = SlotRef {
                        ty: t,
                        key: key_index as u16,
                        slot: si as u8,
                    };
                    match s.base {
                        Base::NameTree => Some(Ty::Tree {
                            number: false,
                            leaf,
                        }),
                        Base::NumberTree => Some(Ty::Tree { number: true, leaf }),
                        _ => self.choose(s.links, value).map(Ty::Def),
                    }
                });
                if issues {
                    self.check_entry(row, e.slot, c, node, value, &res, &mut out);
                }
            }
            Ty::Tree { number, leaf } => {
                let k = key.map(Name::as_bytes);
                e.ty = match k {
                    Some(b"Kids") => Some(Ty::TreeKids { number, leaf }),
                    Some(b"Names") if !number => Some(Ty::TreeEntries { number, leaf }),
                    Some(b"Nums") if number => Some(Ty::TreeEntries { number, leaf }),
                    _ => None,
                };
                if issues && e.ty.is_some() && !matches!(value, Object::Array(_)) {
                    out.push(self.issue(
                        Severity::Error,
                        IssueCode::BadTree,
                        format!("expected an array, got {}", value.type_name()),
                        node,
                    ));
                }
            }
            Ty::TreeKids { number, leaf } => {
                e.ty = Some(Ty::Tree { number, leaf });
                if issues && value.as_dict().is_none() {
                    out.push(self.issue(
                        Severity::Error,
                        IssueCode::BadTree,
                        format!(
                            "tree node kid must be a dictionary, got {}",
                            value.type_name()
                        ),
                        node,
                    ));
                }
            }
            Ty::TreeEntries { number, leaf } => {
                if i % 2 == 1 {
                    e.ty = self.choose(leaf.slot().links, value).map(Ty::Def);
                } else if issues {
                    let ok = if number {
                        matches!(value, Object::Integer(_))
                    } else {
                        matches!(value, Object::String(_))
                    };
                    if !ok {
                        out.push(self.issue(
                            Severity::Error,
                            IssueCode::BadTree,
                            format!(
                                "tree key must be {}, got {}",
                                if number { "an integer" } else { "a string" },
                                value.type_name()
                            ),
                            node,
                        ));
                    }
                }
            }
        }
        if let (Object::Ref(id), Some(t)) = (&node.obj, e.ty) {
            self.record(*id, t);
        }
        e.issues = out;
        e
    }

    fn issue(&self, severity: Severity, code: IssueCode, message: String, node: &Node) -> Issue {
        Issue {
            severity,
            code,
            message,
            obj: None,
            path: Vec::new(),
            span: node.span,
        }
    }

    fn missing_object(&self, node: &Node) -> Issue {
        let id = node
            .obj
            .as_ref_id()
            .map(|i| i.to_string())
            .unwrap_or_default();
        self.issue(
            Severity::Warning,
            IssueCode::MissingObject,
            format!("{id} R refers to a missing object (treated as null)"),
            node,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn check_entry(
        &self,
        row: &'static KeyDef,
        slot: Option<&'static Slot>,
        c: Container,
        node: &Node,
        value: &Object,
        res: &Resolved,
        out: &mut Vec<Issue>,
    ) {
        let v = self.version;
        if v != 0 && row.since > v {
            let (sev, ext) = if row.extension.is_empty() {
                (Severity::Warning, String::new())
            } else {
                (Severity::Info, format!(" (or extension {})", row.extension))
            };
            out.push(self.issue(
                sev,
                IssueCode::TooNew,
                format!(
                    "introduced in PDF {}{ext}; the document is PDF {}",
                    version_text(row.since),
                    version_text(v)
                ),
                node,
            ));
        }
        if v != 0 && row.deprecated != 0 && v >= row.deprecated {
            out.push(self.issue(
                Severity::Info,
                IssueCode::Deprecated,
                format!("deprecated in PDF {}", version_text(row.deprecated)),
                node,
            ));
        }
        if res.missing {
            if !row.slots.iter().any(|s| s.base == Base::Null) {
                out.push(self.missing_object(node));
            }
            return;
        }
        let Some(slot) = slot else {
            let expected: Vec<&str> = row.slots.iter().map(|s| s.base.as_str()).collect();
            out.push(self.issue(
                Severity::Error,
                IssueCode::WrongType,
                format!(
                    "expected {}, got {}",
                    expected.join(" or "),
                    value_kind(value)
                ),
                node,
            ));
            return;
        };
        let env = CEnv {
            c,
            ck: self,
            this: Some((row.key, value)),
        };
        let direct = !matches!(node.obj, Object::Ref(_));
        let must = match slot.indirect {
            Indirect::Any => None,
            Indirect::Must => Some(true),
            Indirect::MustNot => Some(false),
            Indirect::MustIf(e) => (expr::eval(e, &env) == Some(true)).then_some(true),
            Indirect::MustNotIf(e) => (expr::eval(e, &env) == Some(true)).then_some(false),
        };
        match must {
            Some(true) if direct => out.push(self.issue(
                Severity::Warning,
                IssueCode::NotIndirect,
                "must be an indirect reference".into(),
                node,
            )),
            Some(false) if !direct => out.push(self.issue(
                Severity::Warning,
                IssueCode::NotDirect,
                "must be a direct object".into(),
                node,
            )),
            _ => {}
        }
        if let Some(problem) = shape_problem(slot.base, value) {
            out.push(self.issue(Severity::Error, IssueCode::WrongType, problem, node));
            return;
        }
        if let Some(vals) = &slot.values {
            self.check_values(vals, value, &env, node, out);
        }
        match (slot.base, value) {
            (Base::Date, Object::String(s)) => {
                if let Some(p) = date_problem(&s.bytes) {
                    out.push(self.issue(Severity::Warning, IssueCode::BadDate, p, node));
                }
            }
            (Base::StringAscii, Object::String(s)) if !s.bytes.is_ascii() => {
                out.push(self.issue(
                    Severity::Warning,
                    IssueCode::BadString,
                    "must be an ASCII string".into(),
                    node,
                ));
            }
            _ => {}
        }
    }

    fn check_values(
        &self,
        vals: &Values,
        value: &Object,
        env: &CEnv,
        node: &Node,
        out: &mut Vec<Issue>,
    ) {
        if let Some(l) = vals.lits.iter().find(|l| lit_eq(l.lit, value)) {
            let v = self.version;
            if v != 0 && l.since > v {
                out.push(self.issue(
                    Severity::Warning,
                    IssueCode::TooNew,
                    format!(
                        "value {} was introduced in PDF {}",
                        short(value),
                        version_text(l.since)
                    ),
                    node,
                ));
            } else if v != 0 && l.deprecated != 0 && v >= l.deprecated {
                out.push(self.issue(
                    Severity::Info,
                    IssueCode::Deprecated,
                    format!(
                        "value {} is deprecated in PDF {}",
                        short(value),
                        version_text(l.deprecated)
                    ),
                    node,
                ));
            }
            return;
        }
        let results: Vec<Option<bool>> = vals.evals.iter().map(|e| expr::holds(e, env)).collect();
        if results.contains(&Some(true)) || vals.open || results.contains(&None) {
            return;
        }
        let message = if !vals.lits.is_empty() {
            let allowed: Vec<String> = vals.lits.iter().take(12).map(|l| lit_text(l.lit)).collect();
            let more = if vals.lits.len() > 12 { ", …" } else { "" };
            format!(
                "{} is not one of {}{more}",
                short(value),
                allowed.join(", ")
            )
        } else {
            format!(
                "{} does not satisfy {}",
                short(value),
                vals.evals.join(" or ")
            )
        };
        let code = if vals.lits.is_empty() {
            IssueCode::PredicateFailed
        } else {
            IssueCode::BadValue
        };
        out.push(self.issue(Severity::Warning, code, message, node));
    }

    /// Problems of the container as a whole: missing required keys, array
    /// length, tree structure.
    pub fn container_issues(&self, ty: Ty, c: Container) -> Vec<Issue> {
        let mut out = Vec::new();
        let issue = |severity, code, message| Issue {
            severity,
            code,
            message,
            obj: None,
            path: Vec::new(),
            span: None,
        };
        match (ty, c) {
            (Ty::Def(t), Container::Dict(d)) if !type_def(t).array => {
                let env = CEnv {
                    c,
                    ck: self,
                    this: None,
                };
                for row in type_def(t).keys.iter().filter(|k| k.key != "*") {
                    let present = d
                        .get(row.key.as_bytes())
                        .is_some_and(|n| !matches!(n.obj, Object::Null));
                    if present {
                        continue;
                    }
                    // Conditionally required keys are a lesser problem.
                    let severity = match row.required {
                        Req::Yes => Severity::Error,
                        Req::No => continue,
                        Req::Cond(e) if expr::eval(e, &env) == Some(true) => Severity::Warning,
                        Req::Cond(_) => continue,
                    };
                    if row.inheritable && self.inherited(d, row.key) {
                        continue;
                    }
                    let why = match row.required {
                        Req::Cond(e) => format!(" (required when {e})"),
                        _ => String::new(),
                    };
                    out.push(issue(
                        severity,
                        IssueCode::MissingKey,
                        format!("required key /{} is missing{why}", row.key),
                    ));
                }
            }
            (Ty::Def(t), Container::Array(a)) if type_def(t).array => {
                let def = type_def(t);
                let fixed: Vec<&KeyDef> = def
                    .keys
                    .iter()
                    .filter(|k| k.key.bytes().all(|b| b.is_ascii_digit()))
                    .collect();
                let repeating: Vec<&KeyDef> = def
                    .keys
                    .iter()
                    .filter(|k| k.key.len() > 1 && k.key.ends_with('*'))
                    .collect();
                let wildcard = def.keys.iter().any(|k| k.key == "*");
                let required = fixed
                    .iter()
                    .rposition(|k| matches!(k.required, Req::Yes))
                    .map_or(0, |p| p + 1);
                if a.len() < required {
                    out.push(issue(
                        Severity::Error,
                        IssueCode::ArrayLength,
                        format!(
                            "{} elements; {} expects at least {required}",
                            a.len(),
                            def.name
                        ),
                    ));
                } else if !wildcard && repeating.is_empty() && a.len() > fixed.len() {
                    out.push(issue(
                        Severity::Warning,
                        IssueCode::ArrayLength,
                        format!(
                            "{} elements; {} expects at most {}",
                            a.len(),
                            def.name,
                            fixed.len()
                        ),
                    ));
                } else if !repeating.is_empty()
                    && repeating.iter().any(|k| matches!(k.required, Req::Yes))
                    && a.len() >= fixed.len()
                    && (a.len() - fixed.len()) % repeating.len() != 0
                {
                    out.push(issue(
                        Severity::Warning,
                        IssueCode::ArrayLength,
                        format!(
                            "{} elements; {} expects {} + a multiple of {}",
                            a.len(),
                            def.name,
                            fixed.len(),
                            repeating.len()
                        ),
                    ));
                }
            }
            (Ty::Tree { number, .. }, Container::Dict(d)) => {
                let entries: &[u8] = if number { b"Nums" } else { b"Names" };
                if !d.contains_key(b"Kids".as_slice()) && !d.contains_key(entries) {
                    out.push(issue(
                        Severity::Warning,
                        IssueCode::BadTree,
                        format!(
                            "tree node has neither /Kids nor /{}",
                            String::from_utf8_lossy(entries)
                        ),
                    ));
                }
            }
            (Ty::TreeEntries { .. }, Container::Array(a)) if a.len() % 2 != 0 => {
                out.push(issue(
                    Severity::Error,
                    IssueCode::BadTree,
                    format!("odd number of elements ({})", a.len()),
                ));
            }
            (Ty::Def(t), _) => {
                let def = type_def(t);
                out.push(issue(
                    Severity::Error,
                    IssueCode::WrongType,
                    format!(
                        "{} is {}, got {}",
                        def.name,
                        if def.array {
                            "an array"
                        } else {
                            "a dictionary"
                        },
                        match c {
                            Container::Dict(_) => "a dictionary",
                            Container::Array(_) => "an array",
                        }
                    ),
                ));
            }
            _ => {}
        }
        out
    }

    /// Whether `/Parent`'s chain provides `key`.
    fn inherited(&self, d: &syn::Dict, key: &str) -> bool {
        let mut cur = d.get_key("Parent").cloned();
        for _ in 0..64 {
            let Some(p) = cur.and_then(|p| self.file.resolve(&p).ok()) else {
                return false;
            };
            let Some(pd) = p.as_dict() else {
                return false;
            };
            if pd.get_key(key).is_some_and(|o| !matches!(o, Object::Null)) {
                return true;
            }
            cur = pd.get_key("Parent").cloned();
        }
        false
    }

    /// Validate a container and the direct containers inside it. References
    /// with a known type are returned (with the key they were found under)
    /// for the caller to follow. At most `limit` entries per container are
    /// checked.
    pub fn validate(&self, ty: Ty, c: Container, limit: usize) -> Validation {
        let mut v = Validation::default();
        let mut path = Vec::new();
        self.walk(ty, c, limit, &mut path, &mut v, 0);
        v
    }

    fn walk(
        &self,
        ty: Ty,
        c: Container,
        limit: usize,
        path: &mut Vec<Seg>,
        v: &mut Validation,
        depth: u32,
    ) {
        for mut i in self.container_issues(ty, c) {
            i.path = path.clone();
            v.issues.push(i);
        }
        if c.len() > limit {
            v.truncated = true;
        }
        for i in 0..c.len().min(limit) {
            let e = self.entry(ty, c, i, true);
            let seg = c.seg(i);
            for mut is in e.issues {
                is.path = path.clone();
                is.path.push(seg.clone());
                v.issues.push(is);
            }
            let Some((key, node)) = c.child(i) else {
                continue;
            };
            let Some(t) = e.ty else {
                continue;
            };
            match &node.obj {
                Object::Ref(id) => {
                    let via = match path.first() {
                        Some(Seg::Key(k)) => Some(k.clone()),
                        Some(Seg::Index(_)) => None,
                        None => key.cloned(),
                    };
                    v.refs.push((*id, t, via));
                }
                o if depth < 32 => {
                    if let Some(cc) = Container::of(o) {
                        path.push(seg);
                        self.walk(t, cc, limit, path, v, depth + 1);
                        path.pop();
                    }
                }
                _ => {}
            }
        }
    }
}

/// Breadth-first validation along typed references, in bounded steps.
pub struct Walk {
    /// `None` is the trailer.
    queue: VecDeque<(Option<ObjId>, Ty)>,
    seen: HashSet<ObjId>,
    root: Option<ObjId>,
    /// Objects validated so far.
    pub done: u64,
}

impl Walk {
    /// The whole document, from the trailer.
    pub fn document(ck: &Checker) -> Self {
        Self {
            queue: VecDeque::from([(None, ck.trailer_type())]),
            seen: HashSet::new(),
            root: None,
            done: 0,
        }
    }

    /// One object and what it refers to. Upward and sideways links (`/Parent`,
    /// pages reached other than through `/Kids`) are not followed, so a page
    /// does not pull in the whole document.
    pub fn object(id: ObjId, ty: Ty) -> Self {
        Self {
            queue: VecDeque::from([(Some(id), ty)]),
            seen: HashSet::from([id]),
            root: Some(id),
            done: 0,
        }
    }

    /// Nothing to validate.
    pub fn empty() -> Self {
        Self {
            queue: VecDeque::new(),
            seen: HashSet::new(),
            root: None,
            done: 0,
        }
    }

    /// Objects discovered but not validated yet.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// Validate up to `n` objects, appending their issues to `out`. Returns
    /// whether work remains.
    pub fn step(&mut self, ck: &Checker, n: usize, out: &mut Vec<Issue>) -> bool {
        for _ in 0..n {
            let Some((id, ty)) = self.queue.pop_front() else {
                return false;
            };
            self.done += 1;
            let held;
            let (c, in_file) = match id {
                None => (Container::Dict(ck.file.trailer()), true),
                Some(id) => {
                    let Ok(o) = ck.file.get(id) else {
                        continue;
                    };
                    held = o;
                    let Some(c) = Container::of(&held.node.obj) else {
                        continue;
                    };
                    (c, matches!(held.location, syn::ObjLocation::File(_)))
                }
            };
            let v = ck.validate(ty, c, usize::MAX);
            for mut i in v.issues {
                i.obj = id;
                if !in_file {
                    i.span = None;
                }
                out.push(i);
            }
            for (rid, rty, via) in v.refs {
                if self.root.is_some() && !self.follows(rty, via.as_ref()) {
                    continue;
                }
                if self.seen.insert(rid) {
                    self.queue.push_back((Some(rid), rty));
                }
            }
        }
        !self.queue.is_empty()
    }

    fn follows(&self, ty: Ty, via: Option<&Name>) -> bool {
        let via = via.map(Name::as_bytes);
        if via == Some(b"Parent") {
            return false;
        }
        let page_like = ty
            .def()
            .is_some_and(|d| matches!(d.name, "PageObject" | "PageTreeNode" | "PageTreeNodeRoot"));
        !page_like || via == Some(b"Kids")
    }
}

#[derive(Default)]
pub struct Validation {
    pub issues: Vec<Issue>,
    /// Typed references found: target, its type, and the top-level key of
    /// the validated object it was found under (`None` in arrays).
    pub refs: Vec<(ObjId, Ty, Option<Name>)>,
    /// Some container had more entries than were checked.
    pub truncated: bool,
}

struct CEnv<'c, 'a> {
    c: Container<'c>,
    ck: &'c Checker<'a>,
    /// The entry being checked (its own value for `@Key`).
    this: Option<(&'static str, &'c Object)>,
}

impl Env for CEnv<'_, '_> {
    fn version(&self) -> Ver {
        self.ck.version
    }

    fn lookup(&self, key: &str) -> Option<Object> {
        if let Some((k, v)) = self.this {
            if k == key {
                return Some(v.clone());
            }
        }
        let n = self.c.get(key)?;
        self.ck.file.resolve(&n.obj).ok()
    }
}

fn find_key(def: &'static TypeDef, key: &[u8]) -> Option<&'static KeyDef> {
    def.keys
        .iter()
        .find(|k| k.key.as_bytes() == key)
        .or_else(|| def.keys.iter().find(|k| k.key == "*"))
}

fn find_index(def: &'static TypeDef, i: usize) -> Option<&'static KeyDef> {
    let s = i.to_string();
    if let Some(k) = def.keys.iter().find(|k| k.key == s) {
        return Some(k);
    }
    let reps: Vec<(usize, &'static KeyDef)> = def
        .keys
        .iter()
        .filter_map(|k| Some((k.key.strip_suffix('*')?.parse().ok()?, k)))
        .collect();
    if let Some(start) = reps.iter().map(|(p, _)| *p).min() {
        if i >= start {
            let want = start + (i - start) % reps.len();
            return reps.iter().find(|(p, _)| *p == want).map(|(_, k)| *k);
        }
    }
    def.keys.iter().find(|k| k.key == "*")
}

fn fits(base: Base, o: &Object) -> bool {
    match base {
        Base::Array | Base::Rectangle | Base::Matrix => matches!(o, Object::Array(_)),
        Base::Bitmask | Base::Integer => matches!(o, Object::Integer(_)),
        Base::Number => matches!(o, Object::Integer(_) | Object::Real(_)),
        Base::Boolean => matches!(o, Object::Bool(_)),
        Base::Date | Base::String | Base::StringAscii | Base::StringByte | Base::StringText => {
            matches!(o, Object::String(_))
        }
        Base::Name => matches!(o, Object::Name(_)),
        Base::Null => matches!(o, Object::Null),
        Base::Dictionary | Base::NameTree | Base::NumberTree => matches!(o, Object::Dict(_)),
        Base::Stream => matches!(o, Object::Stream(_)),
    }
}

/// Rectangles and matrices are arrays of 4 and 6 numbers.
fn shape_problem(base: Base, o: &Object) -> Option<String> {
    let (n, what) = match base {
        Base::Rectangle => (4, "rectangle"),
        Base::Matrix => (6, "matrix"),
        _ => return None,
    };
    let a = o.as_array()?;
    let numbers = a.iter().all(|x| x.obj.as_f64().is_some());
    (a.len() != n || !numbers).then(|| format!("a {what} is an array of {n} numbers"))
}

fn value_kind(o: &Object) -> &'static str {
    match o {
        Object::Real(_) => "real",
        o => o.type_name(),
    }
}

fn lit_eq(l: Lit, o: &Object) -> bool {
    match (l, o) {
        (Lit::Name(a), Object::Name(b)) => a.as_bytes() == b.as_bytes(),
        (Lit::Int(a), Object::Integer(b)) => a == *b,
        (Lit::Int(a), Object::Real(b)) => a as f64 == *b,
        (Lit::Num(a), o) => o.as_f64() == Some(a),
        (Lit::Str(a), Object::String(b)) => a.as_bytes() == b.bytes,
        (Lit::Bool(a), Object::Bool(b)) => a == *b,
        _ => false,
    }
}

fn lit_text(l: Lit) -> String {
    match l {
        Lit::Name(n) => format!("/{n}"),
        Lit::Int(i) => i.to_string(),
        Lit::Num(n) => n.to_string(),
        Lit::Str(s) => format!("({s})"),
        Lit::Bool(b) => b.to_string(),
    }
}

/// Short rendering of a scalar for messages.
fn short(o: &Object) -> String {
    match o {
        Object::Name(n) => format!("/{}", n.as_str_lossy()),
        Object::Integer(i) => i.to_string(),
        Object::Real(r) => r.to_string(),
        Object::Bool(b) => b.to_string(),
        Object::String(s) => {
            let t = String::from_utf8_lossy(&s.bytes[..s.bytes.len().min(40)]).into_owned();
            format!("({t})")
        }
        o => o.type_name().to_string(),
    }
}

/// Check `D:YYYYMMDDHHmmSSOHH'mm` (everything after the year optional).
fn date_problem(b: &[u8]) -> Option<String> {
    let s = b.strip_prefix(&[0xfe, 0xff]).map_or_else(
        || b.to_vec(),
        |rest| rest.chunks(2).filter_map(|c| c.get(1).copied()).collect(),
    );
    let Some(rest) = s.strip_prefix(b"D:") else {
        return Some("date should start with D:".into());
    };
    let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits < 4 || digits % 2 != 0 || digits > 14 {
        return Some(format!(
            "date has {digits} digits; expected YYYY[MM[DD[HH[mm[SS]]]]]"
        ));
    }
    let num = |a: usize| -> u32 {
        std::str::from_utf8(&rest[a..a + 2])
            .ok()
            .and_then(|x| x.parse().ok())
            .unwrap_or(0)
    };
    let ranges = [(4, 1, 12), (6, 1, 31), (8, 0, 23), (10, 0, 59), (12, 0, 59)];
    for (at, lo, hi) in ranges {
        if digits > at && !(lo..=hi).contains(&num(at)) {
            return Some("date has a field out of range".into());
        }
    }
    let tz = &rest[digits..];
    let ok = match tz.first() {
        None => true,
        Some(b'Z') => tz[1..].iter().all(|c| c.is_ascii_digit() || *c == b'\''),
        Some(b'+' | b'-') => {
            tz.len() >= 3
                && tz[1..3].iter().all(u8::is_ascii_digit)
                && tz[3..].iter().all(|c| c.is_ascii_digit() || *c == b'\'')
        }
        _ => false,
    };
    (!ok).then(|| "date has an invalid time zone".into())
}

/// How well `c` fits `def`: matching discriminators (required keys with a
/// fixed value set, like `/Type` and `/Subtype`) weigh most.
fn score(def: &'static TypeDef, c: Container) -> i32 {
    match c {
        Container::Dict(_) if def.array => return i32::MIN / 2,
        Container::Array(_) if !def.array => return i32::MIN / 2,
        _ => {}
    }
    let mut s = 0;
    match c {
        Container::Dict(d) => {
            for row in def.keys.iter().filter(|k| k.key != "*") {
                s += score_entry(row, d.get(row.key.as_bytes()));
            }
            if !def.keys.iter().any(|k| k.key == "*") {
                let unknown = d
                    .keys()
                    .filter(|k| !def.keys.iter().any(|r| r.key.as_bytes() == k.as_bytes()))
                    .count();
                s -= unknown as i32;
            }
        }
        Container::Array(a) => {
            for (i, n) in a.iter().enumerate().take(8) {
                s += match find_index(def, i) {
                    Some(row) => score_entry(row, Some(n)),
                    None => -2,
                };
            }
            for row in def
                .keys
                .iter()
                .filter(|k| k.key.bytes().all(|b| b.is_ascii_digit()))
            {
                let i: usize = row.key.parse().unwrap_or(0);
                if i >= a.len() && matches!(row.required, Req::Yes) {
                    s -= 4;
                }
            }
        }
    }
    s
}

fn score_entry(row: &KeyDef, node: Option<&Node>) -> i32 {
    let required = matches!(row.required, Req::Yes);
    let Some(n) = node.filter(|n| !matches!(n.obj, Object::Null)) else {
        return if required { -4 } else { 0 };
    };
    if matches!(n.obj, Object::Ref(_)) {
        return 1;
    }
    let Some(slot) = row.slots.iter().find(|sl| fits(sl.base, &n.obj)) else {
        return -1;
    };
    match &slot.values {
        Some(v) if !v.lits.is_empty() && v.evals.is_empty() && !v.open => {
            let hit = v.lits.iter().any(|l| lit_eq(l.lit, &n.obj));
            1 + match (hit, required) {
                (true, true) => 10,
                (true, false) => 3,
                (false, true) => -30,
                (false, false) => -6,
            }
        }
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_link_and_index() {
        let cat = type_by_name("Catalog").unwrap();
        let pages = find_key(type_def(cat), b"Pages").unwrap();
        assert!(matches!(pages.required, Req::Yes));
        let link = pages.slots[0].links[0];
        assert_eq!(type_def(link.ty).name, "PageTreeNodeRoot");
        assert!(by_type_key(b"Catalog").contains(&cat));
        let kids = type_def(type_by_name("ArrayOfPageTreeNodeKids").unwrap());
        assert!(kids.array);
    }

    #[test]
    fn repeating_array_elements() {
        let def = type_def(type_by_name("ArrayOfAttributeRevisions").unwrap());
        assert_eq!(find_index(def, 0).unwrap().key, "0*");
        assert_eq!(find_index(def, 3).unwrap().key, "1*");
        assert_eq!(find_index(def, 4).unwrap().key, "0*");
    }

    #[test]
    fn dates() {
        assert_eq!(date_problem(b"D:20240131120000+09'00'"), None);
        assert_eq!(date_problem(b"D:2024"), None);
        assert_eq!(
            date_problem(b"D:20241301"),
            Some("date has a field out of range".into())
        );
        assert!(date_problem(b"2024").is_some());
        assert!(date_problem(b"D:202401311200000").is_some());
    }

    #[test]
    fn choose_by_discriminator() {
        let mut d = syn::Dict::new();
        d.insert(
            Name::from("Type"),
            Node::synthetic(Object::Name(Name::from("Annot"))),
        );
        d.insert(
            Name::from("Subtype"),
            Node::synthetic(Object::Name(Name::from("Link"))),
        );
        let rect = Object::Array(
            [0, 0, 10, 10]
                .iter()
                .map(|&n| Node::synthetic(Object::Integer(n)))
                .collect(),
        );
        d.insert(Name::from("Rect"), Node::synthetic(rect));
        let cands = by_type_key(b"Annot");
        assert!(cands.len() > 10);
        let best = cands
            .iter()
            .max_by_key(|&&t| score(type_def(t), Container::Dict(&d)))
            .unwrap();
        assert_eq!(type_def(*best).name, "AnnotLink");
    }
}
