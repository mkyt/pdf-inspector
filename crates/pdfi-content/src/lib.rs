//! Content streams: the instruction list ([`op`]), its parser ([`parse`]),
//! and an interpreter ([`interp`]) that checks the operations and tracks
//! the graphics state, recording what each operation paints with its
//! bounding box.
//!
//! [`DocContent`] keeps the analyses of recently viewed contents (pages,
//! form XObjects, other streams) and the fonts they use. Pair it with a
//! document through [`DocContent::on`]. [`Content::draw`] runs a content
//! again to paint it on a [`device::Device`].

pub mod device;
pub mod extract;
pub mod geom;
pub mod interp;
pub mod op;
pub mod parse;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use pdfi_model::{Model, Rect, resolve};
use pdfi_syntax::parser::Parser;
use pdfi_syntax::{Dict, DictExt, ObjId, Object, PdfFile, Severity, Span, Stream};

pub use pdfi_font as font;

use crate::device::Device;
use crate::font::Font;
use crate::geom::IDENTITY;
use crate::interp::{Element, Env, GState, Interp, Kind, Rec, Resources, Show, State};
use crate::op::{ContentSource, Op, OpOrigin};
use crate::parse::{OpReader, ReadPoint, SourceData};

/// Decoded bytes read from one content stream.
pub const MAX_STREAM: usize = 64 << 20;
/// Issues kept per content.
const MAX_ISSUES: usize = 100_000;
/// Analyses kept per document.
const ANALYSES: usize = 4;
/// Decoded bytes of form XObjects kept per document.
const FORM_DATA: usize = 64 << 20;
/// Fonts kept per document (each may hold its program once drawn).
const FONTS: usize = 1024;
/// Interpreter state saved every this many operations.
pub const CHECKPOINT: usize = 1024;

/// A problem of a content.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    /// The operation it concerns (`None`: the content as a whole).
    pub op: Option<u32>,
    /// Where in a source stream, for syntax problems.
    pub at: Option<OpOrigin>,
}

impl Issue {
    pub fn new(
        severity: Severity,
        code: &'static str,
        message: impl Into<String>,
        op: Option<u32>,
    ) -> Self {
        Issue {
            severity,
            code,
            message: message.into(),
            op,
            at: None,
        }
    }

    pub fn error(code: &'static str, message: impl Into<String>, op: Option<u32>) -> Self {
        Self::new(Severity::Error, code, message, op)
    }

    pub fn warning(code: &'static str, message: impl Into<String>, op: Option<u32>) -> Self {
        Self::new(Severity::Warning, code, message, op)
    }

    pub fn at(mut self, source: u32, span: Span) -> Self {
        self.at = Some(OpOrigin { source, span });
        self
    }
}

/// Issues collected while parsing and interpreting.
#[derive(Debug, Default)]
pub struct Issues {
    items: Vec<Issue>,
    /// Operation that issues without one are attached to.
    pub(crate) cursor: Option<u32>,
    pub dropped: u64,
}

impl Issues {
    pub fn push(&mut self, mut i: Issue) {
        if i.op.is_none() {
            i.op = self.cursor;
        }
        if self.items.len() < MAX_ISSUES {
            self.items.push(i);
        } else {
            self.dropped += 1;
        }
    }

    /// Move the lexer's diagnostics over, attached to operation `op`.
    pub(crate) fn flush_lexer(&mut self, p: &mut Parser<'_>, source: u32, op: usize) {
        for d in p.take_diags() {
            let mut i = Issue::new(d.severity, "Syntax", d.message, Some(op as u32));
            if let Some(s) = d.span {
                i = i.at(source, s);
            }
            self.push(i);
        }
    }

    pub fn into_vec(self) -> Vec<Issue> {
        self.items
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A content to analyze.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// A page by index: its `/Contents`, with its (inherited) resources.
    Page(u64),
    /// A stream: form XObject, tiling pattern, glyph procedure, appearance
    /// stream, or one stream of a page's contents (without the page's
    /// resources).
    Stream(ObjId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key {
    Page(ObjId),
    Stream(ObjId),
}

/// Operation counts by category.
pub type Counts = Vec<(op::Category, u64)>;

/// A parsed and interpreted content.
#[derive(Debug)]
pub struct Analysis {
    pub kind: Kind,
    /// The page (for pages) or stream.
    pub id: ObjId,
    /// The streams parsed (their decoded data is kept to read operations
    /// again on request).
    pub sources: Vec<ContentSource>,
    data: Vec<SourceData>,
    /// Number of operations.
    pub ops: usize,
    /// Stopped at the operation limit.
    pub truncated: bool,
    /// Sorted by operation; content-wide issues (`op: None`) first.
    pub issues: Vec<Issue>,
    pub issues_dropped: u64,
    /// Sorted by operation; at most one per operation.
    pub elements: Vec<Element>,
    /// Nesting depth of each operation (saved states, text object, marked
    /// content); closing operators are at their opener's depth.
    pub depth: Vec<u16>,
    /// Decoded text of the text-showing operations, by operation.
    pub texts: Vec<(u32, String)>,
    /// What each text-showing operation shows, by operation (text inside
    /// forms at the `Do` that paints them).
    pub shows: Vec<Show>,
    /// The visible area of a page, or a form's `/BBox` (also the initial
    /// clip).
    pub bounds: Option<Rect>,
    /// A page's `/Rotate` (0, 90, 180 or 270; 0 for streams).
    pub rotate: i64,
    /// The resource dictionary and where it is.
    pub resources: Option<Object>,
    pub resources_id: Option<ObjId>,
    /// Some text bounding boxes rest on guessed glyph widths.
    pub approximate: bool,
    pub counts: Counts,
    /// Reader and interpreter state before every [`CHECKPOINT`]-th
    /// operation.
    checkpoints: Vec<(ReadPoint, State)>,
}

impl Analysis {
    pub fn issues_of(&self, op: u32) -> &[Issue] {
        let lo = self.issues.partition_point(|i| i.op.is_none_or(|o| o < op));
        let hi = self
            .issues
            .partition_point(|i| i.op.is_none_or(|o| o <= op));
        &self.issues[lo..hi]
    }

    pub fn element_of(&self, op: u32) -> Option<&Element> {
        let k = self.elements.partition_point(|e| e.op < op);
        self.elements.get(k).filter(|e| e.op == op)
    }

    /// Operations `from..from + n` (read again from the decoded data).
    pub fn read_ops(&self, from: usize, n: usize) -> Vec<Op> {
        let Some((rp, _)) = self.checkpoints.get(from / CHECKPOINT) else {
            return Vec::new();
        };
        let mut r = OpReader::at(&self.data, rp);
        let mut sink = Issues::default();
        let mut out = Vec::with_capacity(n.min(self.ops.saturating_sub(from)));
        while out.len() < n {
            let at = r.count();
            let Some(op) = r.next(&mut sink) else { break };
            if at >= from {
                out.push(op);
            }
        }
        out
    }

    pub fn text_of(&self, op: u32) -> Option<&str> {
        let k = self.texts.partition_point(|t| t.0 < op);
        self.texts
            .get(k)
            .filter(|t| t.0 == op)
            .map(|t| t.1.as_str())
    }
}

/// Decoded data of form XObjects, and its total size.
#[derive(Default)]
struct FormData {
    data: HashMap<ObjId, Arc<[SourceData]>>,
    bytes: usize,
}

/// Per-document state; pair it with the document through [`DocContent::on`].
#[derive(Default)]
pub struct DocContent {
    analyses: Mutex<Vec<(Key, Arc<Analysis>)>>,
    forms: Mutex<FormData>,
    fonts: Mutex<HashMap<ObjId, Arc<Font>>>,
}

impl DocContent {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on<'a>(&'a self, model: Model<'a>) -> Content<'a> {
        Content { m: model, c: self }
    }

    /// A font, cached by object.
    pub fn font(&self, file: &PdfFile, id: Option<ObjId>, d: &Dict) -> Arc<Font> {
        let Some(id) = id else {
            return Arc::new(font::load(file, None, d));
        };
        if let Some(f) = lock(&self.fonts).get(&id) {
            return f.clone();
        }
        let f = Arc::new(font::load(file, Some(id), d));
        let mut g = lock(&self.fonts);
        if g.len() >= FONTS {
            g.clear();
        }
        g.insert(id, f.clone());
        f
    }

    /// Decoded data of a form XObject (for following it); problems are
    /// reported where the form itself is analyzed.
    pub fn form_data(&self, file: &PdfFile, id: ObjId, s: &Stream) -> Arc<[SourceData]> {
        if let Some(p) = lock(&self.forms).data.get(&id) {
            return p.clone();
        }
        let d = decode(file, Some(id), s);
        let len = d.data.len();
        let p: Arc<[SourceData]> = Arc::new([d]);
        let mut g = lock(&self.forms);
        if g.bytes + len > FORM_DATA {
            g.data.clear();
            g.bytes = 0;
        }
        g.data.insert(id, p.clone());
        g.bytes += len;
        p
    }
}

/// The contents of one document.
pub struct Content<'a> {
    m: Model<'a>,
    c: &'a DocContent,
}

fn decode(file: &PdfFile, id: Option<ObjId>, s: &Stream) -> SourceData {
    let d = file.decode_stream(id.unwrap_or(ObjId::new(0, 0)), s, Some(MAX_STREAM + 1));
    let mut error = d.errors.first().cloned();
    if let Some(f) = &d.stopped_at {
        error = Some(format!("filter /{} cannot be decoded", f.as_str_lossy()));
    }
    let mut data = d.data;
    if data.len() > MAX_STREAM {
        data.truncate(MAX_STREAM);
        error = Some(format!("only the first {} MiB are read", MAX_STREAM >> 20));
    }
    SourceData { id, data, error }
}

impl<'a> Content<'a> {
    pub fn file(&self) -> &'a PdfFile {
        self.m.file
    }

    /// Parse and interpret a content (cached).
    pub fn analyze(&self, t: Target) -> Result<Arc<Analysis>, String> {
        self.analyze_with(t, true)
    }

    /// Like [`Content::analyze`], without keeping the result: for going
    /// through many contents without pushing out those being viewed.
    pub fn analyze_once(&self, t: Target) -> Result<Arc<Analysis>, String> {
        self.analyze_with(t, false)
    }

    fn analyze_with(&self, t: Target, keep: bool) -> Result<Arc<Analysis>, String> {
        let file = self.m.file;
        let key = match t {
            Target::Page(i) => Key::Page(self.m.page(i)?.id),
            Target::Stream(id) => Key::Stream(id),
        };
        {
            let mut g = lock(&self.c.analyses);
            if let Some(k) = g.iter().position(|(k, _)| *k == key) {
                let e = g.remove(k);
                let a = e.1.clone();
                g.push(e);
                return Ok(a);
            }
        }
        let (kind, id, sources, resources, resources_id, bounds, rotate) = match t {
            Target::Page(i) => {
                let info = self.m.page_info(i)?;
                let held = file.get(info.page.id).map_err(|e| e.to_string())?;
                let mut sources = Vec::new();
                if let Some(c) = held.node.obj.as_dict().and_then(|d| d.get_key("Contents")) {
                    let (_, r) = resolve(file, c);
                    let items: Vec<Object> = match &*r {
                        Object::Array(a) => a.iter().map(|n| n.obj.clone()).collect(),
                        _ => vec![c.clone()],
                    };
                    for o in items {
                        let (id, r) = resolve(file, &o);
                        sources.push(match r.as_stream() {
                            Some(s) => decode(file, id, s),
                            None => SourceData {
                                id,
                                data: Vec::new(),
                                error: Some(format!("not a stream ({})", r.type_name())),
                            },
                        });
                    }
                }
                (
                    Kind::Page,
                    info.page.id,
                    sources,
                    info.resources.clone(),
                    info.resources_id,
                    info.visible(),
                    info.rotate,
                )
            }
            Target::Stream(id) => {
                let held = file.get(id).map_err(|e| e.to_string())?;
                let s = held.node.obj.as_stream().ok_or_else(|| {
                    format!("{id} obj is not a stream ({})", held.node.obj.type_name())
                })?;
                let kind = if s.dict.get_name("Subtype").is_some_and(|n| n == "Form") {
                    Kind::Form
                } else if s.dict.get_i64("PatternType") == Some(1) {
                    Kind::Pattern
                } else {
                    Kind::Other
                };
                let bounds = match kind {
                    Kind::Form | Kind::Pattern => {
                        s.dict.get_key("BBox").and_then(|b| Rect::read(file, b))
                    }
                    _ => None,
                };
                let (res_id, res) = match s.dict.get_key("Resources") {
                    Some(r) => {
                        let (rid, rr) = resolve(file, r);
                        (rid, Some((*rr).clone()))
                    }
                    None => (None, None),
                };
                (
                    kind,
                    id,
                    vec![decode(file, Some(id), s)],
                    res,
                    res_id,
                    bounds,
                    0,
                )
            }
        };

        let res = Resources::new(file, resources.as_ref());
        let mut env = Env::new(file, self.c);
        let mut interp = Interp::new(State::new(GState::new(IDENTITY, bounds)), &res, kind);
        let mut reader = OpReader::new(&sources);
        let mut depth = Vec::new();
        let mut checkpoints = Vec::new();
        let mut counts: HashMap<op::Category, u64> = HashMap::new();
        let mut rec = Rec::full();
        loop {
            let i = reader.count();
            if i % CHECKPOINT == 0 {
                checkpoints.push((reader.point(), interp.s.clone()));
            }
            let Some(op) = reader.next(&mut rec.issues) else {
                break;
            };
            *counts.entry(op.operator.category()).or_default() += 1;
            let before = interp.s.nesting();
            interp.step(&mut env, i as u32, &op, &mut rec);
            let after = interp.s.nesting();
            depth.push(before.min(after).min(u16::MAX as usize) as u16);
        }
        let n = reader.count();
        let truncated = reader.truncated;
        let Rec {
            mut issues,
            elements,
            texts,
            shows,
            approximate,
            ..
        } = rec;
        issues.cursor = None;
        interp.finish(&mut issues);
        let dropped = issues.dropped;
        let mut issues = issues.into_vec();
        for i in &mut issues {
            if i.op.is_some_and(|o| o as usize >= n) {
                i.op = None;
            }
        }
        issues.sort_by_key(|i| i.op.map_or(0, |o| o as u64 + 1));
        let a = Arc::new(Analysis {
            kind,
            id,
            sources: sources
                .iter()
                .map(|s| ContentSource {
                    id: s.id,
                    len: s.data.len() as u64,
                    error: s.error.clone(),
                })
                .collect(),
            data: sources,
            ops: n,
            truncated,
            issues,
            issues_dropped: dropped,
            elements,
            depth,
            texts,
            shows,
            bounds,
            rotate,
            resources,
            resources_id,
            approximate,
            counts: op::Category::ALL
                .iter()
                .filter_map(|c| counts.get(c).map(|&n| (*c, n)))
                .collect(),
            checkpoints,
        });
        if !keep {
            return Ok(a);
        }
        let mut g = lock(&self.c.analyses);
        g.retain(|(k, _)| *k != key);
        if g.len() >= ANALYSES {
            g.remove(0);
        }
        g.push((key, a.clone()));
        Ok(a)
    }

    /// The state an operation runs in, and the state after it.
    pub fn state_at(&self, a: &Analysis, op: usize) -> Option<(State, State)> {
        if op >= a.ops {
            return None;
        }
        let (rp, start) = a.checkpoints.get(op / CHECKPOINT)?;
        let res = Resources::new(self.m.file, a.resources.as_ref());
        let mut env = Env::new(self.m.file, self.c);
        let mut interp = Interp::new(start.clone(), &res, a.kind);
        let mut rec = Rec::silent();
        let mut r = OpReader::at(&a.data, rp);
        let mut sink = Issues::default();
        loop {
            let i = r.count();
            let o = r.next(&mut sink)?;
            if i == op {
                let before = interp.s.clone();
                interp.step(&mut env, i as u32, &o, &mut rec);
                return Some((before, interp.s));
            }
            interp.step(&mut env, i as u32, &o, &mut rec);
        }
    }

    /// Paint operations `0..=until` (all of them without `until`) of an
    /// analyzed content on `device`. `cancel` is polled every few hundred
    /// operations.
    pub fn draw(
        &self,
        a: &Analysis,
        until: Option<usize>,
        device: &mut dyn Device,
        cancel: &dyn Fn() -> bool,
    ) -> Drawn {
        let res = Resources::new(self.m.file, a.resources.as_ref());
        let mut env = Env::new(self.m.file, self.c);
        env.cancel = Some(cancel);
        let start = State::new(GState::new(IDENTITY, a.bounds));
        let mut interp = Interp::new(start, &res, a.kind).with_data(&a.data);
        let mut rec = Rec::draw(device);
        let mut r = OpReader::new(&a.data);
        let mut sink = Issues::default();
        let end = until.map_or(a.ops, |u| u.saturating_add(1).min(a.ops));
        let mut cancelled = false;
        while r.count() < end {
            let i = r.count();
            if i % 256 == 0 && cancel() {
                cancelled = true;
                break;
            }
            let Some(op) = r.next(&mut sink) else { break };
            if let Some(d) = rec.device.as_deref_mut() {
                d.op(i as u32);
            }
            interp.step(&mut env, i as u32, &op, &mut rec);
        }
        Drawn {
            ops: r.count().min(end),
            cancelled: cancelled || (env.budget == 0 && cancel()),
        }
    }

    /// Paint a stream that a renderer runs itself (a tiling pattern's
    /// cell, a soft mask's group) from state `g`, with the stream's own
    /// resources. The device's operation index is left as it is.
    pub fn draw_stream(
        &self,
        id: ObjId,
        s: &Stream,
        g: GState,
        kind: Kind,
        device: &mut dyn Device,
        cancel: &dyn Fn() -> bool,
    ) -> Drawn {
        let file = self.m.file;
        let res = Resources::new(file, s.dict.get_key("Resources"));
        let data = self.c.form_data(file, id, s);
        let mut env = Env::new(file, self.c);
        env.cancel = Some(cancel);
        let mut interp = Interp::new(State::new(g), &res, kind).with_data(&data);
        let mut rec = Rec::draw(device);
        let mut r = OpReader::new(&data);
        let mut sink = Issues::default();
        let mut cancelled = false;
        loop {
            let i = r.count();
            if i % 256 == 0 && cancel() {
                cancelled = true;
                break;
            }
            let Some(op) = r.next(&mut sink) else { break };
            interp.step(&mut env, i as u32, &op, &mut rec);
            if env.budget == 0 {
                break;
            }
        }
        Drawn {
            ops: r.count(),
            cancelled,
        }
    }
}

/// What [`Content::draw`] got through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drawn {
    /// Operations run.
    pub ops: usize,
    pub cancelled: bool,
}
