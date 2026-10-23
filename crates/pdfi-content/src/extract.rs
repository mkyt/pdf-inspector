//! Cutting a region out of a content: a one-page PDF whose page is the
//! region and whose content keeps only the operations painting there.
//!
//! Operations that set state (graphics and text state, clipping, marked
//! content) are all kept, so what remains is painted exactly as before.
//! What paints outside goes: a path is ended with `n` instead (its clip, if
//! any, still applies) or dropped with its construction, an image, shading
//! or form is dropped, text is dropped (keeping the line it moves to, when
//! later text of the text object needs it) or, when later text continues
//! from where it ends, shown invisibly. A form XObject only partly inside
//! is cut the same way into a new form.
//!
//! Kept operations are copied byte for byte from the decoded content, and
//! the resources they name are copied with everything they refer to,
//! streams still encoded as they are in the file: nothing is re-encoded.

use std::collections::{HashMap, HashSet};
use std::io::Read;

use pdfi_model::{Rect, resolve};
use pdfi_syntax::write::{deflate, write_object};
use pdfi_syntax::{Dict, DictExt, Name, Node, ObjId, Object, PdfFile};

use crate::geom::{IDENTITY, Matrix, intersect, mul, transform, union};
use crate::interp::{ElementKind, Env, GState, Interp, Lookup, Rec, Resources, State};
use crate::op::{Category, Op, Operator};
use crate::parse::{OpReader, SourceData};
use crate::{Analysis, Content, Issues, Target};

/// Forms cut inside each other.
const MAX_DEPTH: u32 = 12;
/// Slack when testing that a box is inside the region, in points.
const EPSILON: f64 = 1e-3;
/// Resource categories pruned to the names the kept operations use.
const PRUNED: [&str; 7] = [
    "Font",
    "XObject",
    "ExtGState",
    "ColorSpace",
    "Pattern",
    "Shading",
    "Properties",
];
/// Color spaces that change how device colors paint: kept when present.
const DEFAULT_SPACES: [&str; 3] = ["DefaultGray", "DefaultRGB", "DefaultCMYK"];

/// Which painting operations a region takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Select {
    /// What they paint lies inside it.
    Inside,
    /// What they paint touches it.
    Overlaps,
}

#[derive(Clone, Copy, Debug)]
pub struct Region {
    /// In the content's space (default user space for pages).
    pub rect: Rect,
    pub select: Select,
    /// Shrink the page to what is kept (within `rect`).
    pub fit: bool,
}

/// The PDF cut out, and what went into it.
#[derive(Debug)]
pub struct Extract {
    pub pdf: Vec<u8>,
    /// The page's `/MediaBox`, in the content's space.
    pub media_box: Rect,
    /// Painting operations kept and dropped (also those inside forms that
    /// were cut).
    pub kept: u64,
    pub dropped: u64,
    /// Form XObjects cut because they were partly inside.
    pub forms: u64,
    /// Objects written.
    pub objects: u64,
}

/// What becomes of one operation.
#[derive(Clone, Debug)]
enum Act {
    Keep,
    Drop,
    /// A path painted outside: end it without painting.
    EndPath,
    BeginText,
    EndText,
    /// Text positioning; kept when later text of the text object is shown.
    Position(bool),
    /// Text shown inside (`kept`), or outside but shown invisibly (`hold`)
    /// for the position it advances to, or for the clip of a clipping mode.
    Text {
        kept: bool,
        hold: bool,
        /// `'` or `"`: moves to the next line first.
        line: bool,
        /// Later text of the text object is shown (so the line matters).
        moves: bool,
        render: i64,
        font: Option<String>,
    },
    /// `Tf`: kept when some text shown uses the font.
    Font(String),
    /// `Do` of a form partly inside: paint its cut instead.
    Cut(u32),
}

/// A cut content: its operations and the resources they use.
struct Cutout {
    data: Vec<u8>,
    resources: Dict,
    kept: u64,
    dropped: u64,
    forms: u64,
    /// What is kept, in page space, within the region.
    bbox: Option<Rect>,
}

/// An object of the new file.
enum Obj {
    Plain(Object),
    /// Dictionary and data as stored.
    Stream(Dict, Vec<u8>),
}

/// Copies objects of the source file into the new one, numbering them
/// anew. Pages, the page tree and the catalog are not followed (a
/// reference to them becomes `null`).
struct Copier<'f> {
    file: &'f PdfFile,
    map: HashMap<ObjId, u32>,
    /// Object `n` at `n - 1`.
    objects: Vec<Option<Obj>>,
    pending: Vec<ObjId>,
}

impl<'f> Copier<'f> {
    fn alloc(&mut self) -> u32 {
        self.objects.push(None);
        self.objects.len() as u32
    }

    fn set(&mut self, num: u32, o: Obj) {
        self.objects[num as usize - 1] = Some(o);
    }

    fn reference(&mut self, id: ObjId) -> Object {
        if let Some(&n) = self.map.get(&id) {
            return Object::Ref(ObjId::new(n, 0));
        }
        let Ok(h) = self.file.get(id) else {
            return Object::Null;
        };
        let skip = match &h.node.obj {
            Object::Dict(d) => ["Page", "Pages", "Catalog"].iter().any(|t| d.type_is(t)),
            Object::Ref(_) => true,
            _ => false,
        };
        if skip {
            return Object::Null;
        }
        let n = self.alloc();
        self.map.insert(id, n);
        self.pending.push(id);
        Object::Ref(ObjId::new(n, 0))
    }

    fn copy(&mut self, o: &Object) -> Object {
        match o {
            Object::Ref(id) => self.reference(*id),
            Object::Array(a) => Object::Array(
                a.iter()
                    .map(|n| Node::synthetic(self.copy(&n.obj)))
                    .collect(),
            ),
            Object::Dict(d) => Object::Dict(self.copy_dict(d)),
            // Streams are only reached through references.
            Object::Stream(_) => Object::Null,
            o => o.clone(),
        }
    }

    fn copy_dict(&mut self, d: &Dict) -> Dict {
        d.iter()
            .map(|(k, v)| (k.clone(), Node::synthetic(self.copy(&v.obj))))
            .collect()
    }

    /// Copy everything referred to so far.
    fn finish(&mut self) -> Result<(), String> {
        while let Some(id) = self.pending.pop() {
            let n = self.map[&id];
            let h = self.file.get(id).map_err(|e| e.to_string())?;
            let o = match &h.node.obj {
                Object::Stream(s) => {
                    let mut data = Vec::new();
                    self.file
                        .decrypted_reader(id, s)
                        .read_to_end(&mut data)
                        .map_err(|e| format!("{id} obj: {e}"))?;
                    let mut dict = self.copy_dict(&s.dict);
                    dict.insert(
                        Name::from("Length"),
                        Node::synthetic(Object::Integer(data.len() as i64)),
                    );
                    Obj::Stream(dict, data)
                }
                o => Obj::Plain(self.copy(o)),
            };
            self.set(n, o);
        }
        Ok(())
    }
}

/// Resource names the kept operations use, by category.
#[derive(Default)]
struct Used(HashMap<&'static str, HashSet<Name>>);

impl Used {
    fn add(&mut self, category: &'static str, o: Option<&Node>) {
        if let Some(n) = o.and_then(|n| n.obj.as_name()) {
            self.0.entry(category).or_default().insert(n.clone());
        }
    }

    fn note(&mut self, op: &Op) {
        let a = &op.operands;
        match op.operator {
            Operator::Font => self.add("Font", a.first()),
            Operator::ExtGState => self.add("ExtGState", a.first()),
            Operator::StrokeSpace | Operator::FillSpace => self.add("ColorSpace", a.first()),
            Operator::StrokeColorN | Operator::FillColorN => self.add("Pattern", a.last()),
            Operator::Shade => self.add("Shading", a.first()),
            Operator::XObject => self.add("XObject", a.first()),
            Operator::MarkPointProps | Operator::BeginMarkedProps => {
                self.add("Properties", a.get(1))
            }
            Operator::InlineImage => {
                if let Some(img) = &op.inline_image {
                    self.add("ColorSpace", img.dict.get(b"CS".as_slice()));
                    self.add("ColorSpace", img.dict.get(b"ColorSpace".as_slice()));
                }
            }
            _ => {}
        }
    }

    fn has(&self, category: &str, name: &Name) -> bool {
        self.0.get(category).is_some_and(|s| s.contains(name))
    }
}

/// An operation as written: its bytes in the decoded content when they
/// hold all of it, otherwise written anew.
fn write_op(data: &[SourceData], op: &Op, out: &mut Vec<u8>) {
    if let Some(o) = op.origin {
        // Operands read from an earlier stream are not in the span.
        let whole = op.operands.is_empty()
            || op.inline_image.is_some()
            || (o.span.end - o.span.start) as usize > op.operator.name().len();
        let bytes = data
            .get(o.source as usize)
            .and_then(|s| s.data.get(o.span.start as usize..o.span.end as usize));
        if let (true, Some(b)) = (whole, bytes) {
            out.extend_from_slice(b);
            out.push(b'\n');
            return;
        }
    }
    for n in &op.operands {
        write_object(&n.obj, out);
        out.push(b' ');
    }
    out.extend_from_slice(op.operator.name().as_bytes());
    out.push(b'\n');
}

fn contains(outer: &Rect, r: &Rect) -> bool {
    r.x0 >= outer.x0 - EPSILON
        && r.y0 >= outer.y0 - EPSILON
        && r.x1 <= outer.x1 + EPSILON
        && r.y1 <= outer.y1 + EPSILON
}

fn matrix(file: &PdfFile, o: Option<&Object>) -> Matrix {
    let Some(o) = o else { return IDENTITY };
    let (_, r) = resolve(file, o);
    let Some(a) = r.as_array().filter(|a| a.len() == 6) else {
        return IDENTITY;
    };
    let mut m = IDENTITY;
    for (x, n) in m.iter_mut().zip(a) {
        *x = match n.obj {
            Object::Integer(i) => i as f64,
            Object::Real(r) => r,
            _ => return IDENTITY,
        };
    }
    m
}

fn real(x: f64) -> Node {
    Node::synthetic(Object::Real((x * 1000.0).round() / 1000.0))
}

struct Cutter<'c, 'a> {
    content: &'c Content<'a>,
    region: Region,
    out: Copier<'a>,
    /// Forms being cut (against cycles).
    active: Vec<ObjId>,
    cancel: &'c dyn Fn() -> bool,
}

impl<'a> Cutter<'_, 'a> {
    /// Cut content `a`, whose space maps to page space by `m`.
    fn cut(&mut self, a: &Analysis, m: &Matrix, depth: u32) -> Result<Cutout, String> {
        let content = self.content;
        let file = content.file();
        let rect = self.region.rect;
        let res = Resources::new(file, a.resources.as_ref());
        let mut env = Env::new(file, content.c);
        let mut interp = Interp::new(State::new(GState::new(IDENTITY, a.bounds)), &res, a.kind)
            .with_data(&a.data);
        let mut rec = Rec::silent();
        let mut sink = Issues::default();
        let mut r = OpReader::new(&a.data);
        let mut acts = Vec::with_capacity(a.ops);
        // Path construction since the last painting operation.
        let mut path = Vec::new();
        let mut cut = Cutout {
            data: Vec::new(),
            resources: Dict::new(),
            kept: 0,
            dropped: 0,
            forms: 0,
            bbox: None,
        };
        let mut cuts: Vec<(u32, Name)> = Vec::new();
        loop {
            let i = r.count();
            if i % 256 == 0 && (self.cancel)() {
                return Err("cancelled".into());
            }
            let Some(op) = r.next(&mut sink) else { break };
            let s = &interp.s;
            let el = a.element_of(i as u32);
            // What the operation paints that can be seen, in page space.
            let seen = el.and_then(|e| match s.g.clip {
                Some(c) => intersect(&c, &e.bbox),
                None => Some(e.bbox),
            });
            let seen = seen.map(|b| transform(&b, m));
            let inside = seen.is_some_and(|b| contains(&rect, &b));
            let touches = seen.and_then(|b| intersect(&rect, &b));
            let take = match self.region.select {
                Select::Inside => inside,
                Select::Overlaps => touches.is_some(),
            };
            let painting = el.is_some_and(|e| e.kind != ElementKind::Clip);
            if painting {
                if take {
                    cut.kept += 1;
                    if let Some(t) = touches {
                        cut.bbox = Some(union(cut.bbox, t));
                    }
                } else {
                    cut.dropped += 1;
                }
            }
            let act = match op.operator.category() {
                Category::PathConstruction => {
                    path.push(acts.len());
                    Act::Keep
                }
                Category::PathPainting => {
                    let p = std::mem::take(&mut path);
                    if !painting || take {
                        Act::Keep
                    } else if s.pending_clip.is_some() {
                        Act::EndPath
                    } else {
                        for k in p {
                            acts[k] = Act::Drop;
                        }
                        Act::Drop
                    }
                }
                Category::TextObject => match op.operator {
                    Operator::BeginText => Act::BeginText,
                    _ => Act::EndText,
                },
                Category::TextPositioning => Act::Position(true),
                Category::TextShowing => {
                    let render = s.g.text.render;
                    Act::Text {
                        kept: !painting || take,
                        // Text in a clipping mode clips at `ET`.
                        hold: render >= 4,
                        line: matches!(
                            op.operator,
                            Operator::NextLineShow | Operator::NextLineSpacingShow
                        ),
                        moves: false,
                        render,
                        font: s.g.text.font.as_ref().map(|f| f.name.clone()),
                    }
                }
                Category::TextState if op.operator == Operator::Font => {
                    match op.operands.first().and_then(|n| n.obj.as_name()) {
                        Some(n) => Act::Font(n.as_str_lossy().into_owned()),
                        None => Act::Keep,
                    }
                }
                Category::XObject | Category::Shading | Category::InlineImage => {
                    let form = el.is_some_and(|e| e.kind == ElementKind::Form);
                    if form && !inside && touches.is_some() && depth < MAX_DEPTH {
                        // Undo the count above: the form's operations count.
                        if take {
                            cut.kept -= 1;
                        } else {
                            cut.dropped -= 1;
                        }
                        let ctm = s.g.ctm;
                        match self.cut_form(&res, &op, &mul(&ctm, m), depth)? {
                            Some((num, inner)) => {
                                cut.kept += inner.kept;
                                cut.dropped += inner.dropped;
                                cut.forms += inner.forms + u64::from(num.is_some());
                                if let Some(b) = inner.bbox {
                                    cut.bbox = Some(union(cut.bbox, b));
                                }
                                match num {
                                    Some(n) => Act::Cut(n),
                                    None if inner.kept > 0 => Act::Keep,
                                    None => Act::Drop,
                                }
                            }
                            None if take => {
                                cut.kept += 1;
                                Act::Keep
                            }
                            None => {
                                cut.dropped += 1;
                                Act::Drop
                            }
                        }
                    } else if take {
                        Act::Keep
                    } else {
                        Act::Drop
                    }
                }
                _ => Act::Keep,
            };
            acts.push(act);
            interp.step(&mut env, i as u32, &op, &mut rec);
        }

        // Going backwards: `any`, some later text of the text object is
        // shown (so positioning and line moves matter); `after`, later text
        // goes on from the position text leaves (no new line in between),
        // so text outside is needed invisibly.
        let (mut any, mut after) = (false, false);
        for act in acts.iter_mut().rev() {
            match act {
                Act::BeginText | Act::EndText => (any, after) = (false, false),
                Act::Position(keep) => {
                    *keep = any;
                    after = false;
                }
                Act::Text {
                    kept,
                    hold,
                    line,
                    moves,
                    ..
                } => {
                    *hold |= !*kept && after;
                    *moves = any;
                    any |= *kept || *hold;
                    after = !*line && (*kept || *hold);
                }
                _ => {}
            }
        }
        let mut fonts = HashSet::new();
        let mut every_font = false;
        for act in &acts {
            if let Act::Text {
                kept, hold, font, ..
            } = act
                && (*kept || *hold)
            {
                match font {
                    Some(f) => {
                        fonts.insert(f.clone());
                    }
                    None => every_font = true,
                }
            }
        }

        let mut used = Used::default();
        let mut r = OpReader::new(&a.data);
        let out = &mut cut.data;
        for act in &acts {
            let Some(op) = r.next(&mut sink) else { break };
            let keep = match act {
                Act::Keep | Act::BeginText | Act::EndText | Act::Position(true) => true,
                Act::Drop => false,
                Act::Position(false) => {
                    // `TD` also sets the leading, for text after it.
                    if op.operator == Operator::TextMoveLeading
                        && let Some(ty) = op.operands.get(1).and_then(|n| n.obj.as_f64())
                    {
                        write_object(&Object::Real(-ty), out);
                        out.extend_from_slice(b" TL\n");
                    }
                    false
                }
                Act::EndPath => {
                    out.extend_from_slice(b"n\n");
                    false
                }
                Act::Text { kept: true, .. } => true,
                Act::Text {
                    hold: true, render, ..
                } => {
                    if matches!(render, 3 | 7) {
                        true
                    } else {
                        // Invisible, or clipping only.
                        let mode = if *render >= 4 { 7 } else { 3 };
                        out.extend_from_slice(format!("{mode} Tr\n").as_bytes());
                        write_op(&a.data, &op, out);
                        out.extend_from_slice(format!("{render} Tr\n").as_bytes());
                        false
                    }
                }
                Act::Text { line, moves, .. } => {
                    // `"` also sets the spacing, for text after it.
                    if op.operator == Operator::NextLineSpacingShow
                        && let [aw, ac, _] = op.operands.as_slice()
                    {
                        write_object(&aw.obj, out);
                        out.extend_from_slice(b" Tw ");
                        write_object(&ac.obj, out);
                        out.extend_from_slice(b" Tc\n");
                    }
                    if *line && *moves {
                        out.extend_from_slice(b"T*\n");
                    }
                    false
                }
                Act::Font(name) => every_font || fonts.contains(name),
                Act::Cut(num) => {
                    let mut k = cuts.len();
                    let name = loop {
                        let n = Name::from(format!("PdfiCut{k}").as_str());
                        if matches!(res.lookup(file, "XObject", &n), Lookup::Missing) {
                            break n;
                        }
                        k += 1;
                    };
                    let mut b = Vec::new();
                    pdfi_syntax::write::write_name(&name, &mut b);
                    out.extend_from_slice(&b);
                    out.extend_from_slice(b" Do\n");
                    cuts.push((*num, name));
                    false
                }
            };
            if keep {
                used.note(&op);
                write_op(&a.data, &op, out);
            }
        }

        cut.resources = self.resources(a.resources.as_ref(), &used);
        if !cuts.is_empty() {
            let mut xo = match cut.resources.get(b"XObject".as_slice()).map(|n| &n.obj) {
                Some(Object::Dict(d)) => d.clone(),
                _ => Dict::new(),
            };
            for (num, name) in cuts {
                xo.insert(name, Node::synthetic(Object::Ref(ObjId::new(num, 0))));
            }
            cut.resources
                .insert(Name::from("XObject"), Node::synthetic(Object::Dict(xo)));
        }
        Ok(cut)
    }

    /// Cut the form a `Do` paints (`m`: its parent's CTM to page space).
    /// `None` when it is not a form that can be cut; a number when a new
    /// form was made (not when all or nothing of it is kept).
    fn cut_form(
        &mut self,
        res: &Resources,
        op: &Op,
        m: &Matrix,
        depth: u32,
    ) -> Result<Option<(Option<u32>, Cutout)>, String> {
        let file = self.content.file();
        let Some(name) = op.operands.first().and_then(|n| n.obj.as_name()) else {
            return Ok(None);
        };
        let Lookup::Found(Some(id), Object::Stream(s)) = res.lookup(file, "XObject", name) else {
            return Ok(None);
        };
        if self.active.contains(&id) || s.dict.get_name("Subtype").is_none_or(|n| n != "Form") {
            return Ok(None);
        }
        let Ok(a) = self.content.analyze_once(Target::Stream(id)) else {
            return Ok(None);
        };
        let fm = matrix(file, s.dict.get_key("Matrix"));
        self.active.push(id);
        let inner = self.cut(&a, &mul(&fm, m), depth + 1);
        self.active.pop();
        let mut inner = inner?;
        if inner.kept == 0 || inner.dropped == 0 {
            return Ok(Some((None, inner)));
        }
        let mut dict = Dict::new();
        for (k, v) in &s.dict {
            if !matches!(
                k.as_bytes(),
                b"Length" | b"Filter" | b"DecodeParms" | b"Resources"
            ) {
                dict.insert(k.clone(), Node::synthetic(self.out.copy(&v.obj)));
            }
        }
        let data = deflate(&inner.data);
        dict.insert(
            Name::from("Resources"),
            Node::synthetic(Object::Dict(std::mem::take(&mut inner.resources))),
        );
        dict.insert(
            Name::from("Filter"),
            Node::synthetic(Object::Name(Name::from("FlateDecode"))),
        );
        dict.insert(
            Name::from("Length"),
            Node::synthetic(Object::Integer(data.len() as i64)),
        );
        let num = self.out.alloc();
        self.out.set(num, Obj::Stream(dict, data));
        Ok(Some((Some(num), inner)))
    }

    /// The resources of a content pruned to what `used` names, copied.
    fn resources(&mut self, o: Option<&Object>, used: &Used) -> Dict {
        let file = self.content.file();
        let mut out = Dict::new();
        let Some(o) = o else { return out };
        let (_, r) = resolve(file, o);
        let Some(d) = r.as_dict() else { return out };
        for (k, v) in d {
            let Some(cat) = PRUNED.into_iter().find(|c| k.as_bytes() == c.as_bytes()) else {
                out.insert(k.clone(), Node::synthetic(self.out.copy(&v.obj)));
                continue;
            };
            let (_, c) = resolve(file, &v.obj);
            let Some(c) = c.as_dict() else { continue };
            let picked: Dict = c
                .iter()
                .filter(|(n, _)| {
                    used.has(cat, n)
                        || (cat == "ColorSpace"
                            && DEFAULT_SPACES.iter().any(|s| n.as_bytes() == s.as_bytes()))
                })
                .map(|(n, v)| (n.clone(), Node::synthetic(self.out.copy(&v.obj))))
                .collect();
            if !picked.is_empty() {
                out.insert(k.clone(), Node::synthetic(Object::Dict(picked)));
            }
        }
        out
    }
}

fn put(d: &mut Dict, k: &str, o: Object) {
    d.insert(Name::from(k), Node::synthetic(o));
}

fn write_file(version: &str, objects: &[Option<Obj>], root: u32) -> Vec<u8> {
    let mut w = format!("%PDF-{version}\n%\u{e2}\u{e3}\u{cf}\u{d3}\n").into_bytes();
    let mut offsets = Vec::with_capacity(objects.len());
    for (k, o) in objects.iter().enumerate() {
        offsets.push(w.len());
        w.extend_from_slice(format!("{} 0 obj\n", k + 1).as_bytes());
        match o {
            Some(Obj::Plain(o)) => write_object(o, &mut w),
            Some(Obj::Stream(d, data)) => {
                write_object(&Object::Dict(d.clone()), &mut w);
                w.extend_from_slice(b"\nstream\n");
                w.extend_from_slice(data);
                w.extend_from_slice(b"\nendstream");
            }
            None => w.extend_from_slice(b"null"),
        }
        w.extend_from_slice(b"\nendobj\n");
    }
    let xref = w.len();
    w.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f\r\n", objects.len() + 1).as_bytes(),
    );
    for off in offsets {
        w.extend_from_slice(format!("{off:010} 00000 n\r\n").as_bytes());
    }
    w.extend_from_slice(
        format!(
            "trailer\n<</Size {}/Root {root} 0 R>>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    w
}

/// The higher of two `x.y` versions.
fn later_version<'v>(a: &'v str, b: &'v str) -> &'v str {
    let key = |v: &str| {
        let (x, y) = v.split_once('.').unwrap_or((v, "0"));
        (x.parse::<u32>().unwrap_or(0), y.parse::<u32>().unwrap_or(0))
    };
    if key(b) > key(a) { b } else { a }
}

impl<'a> Content<'a> {
    /// Cut `region` out of an analyzed content into a one-page PDF.
    /// `cancel` is polled every few hundred operations.
    pub fn extract(
        &self,
        a: &Analysis,
        region: &Region,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Extract, String> {
        let file = self.file();
        let rect = region.rect;
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return Err("the region is empty".into());
        }
        let mut c = Cutter {
            content: self,
            region: *region,
            out: Copier {
                file,
                map: HashMap::new(),
                objects: Vec::new(),
                pending: Vec::new(),
            },
            active: vec![a.id],
            cancel,
        };
        // Catalog, page tree, page, contents.
        for _ in 0..4 {
            c.out.alloc();
        }
        let cut = c.cut(a, &IDENTITY, 0)?;
        if cut.kept == 0 {
            return Err("nothing is painted in the region".into());
        }
        let media_box = match (region.fit, cut.bbox) {
            (true, Some(b)) => b,
            _ => rect,
        };

        let held = file.get(a.id).map_err(|e| e.to_string())?;
        let src = match &held.node.obj {
            Object::Dict(d) => Some(d),
            Object::Stream(s) => Some(&s.dict),
            _ => None,
        };
        let mut page = Dict::new();
        put(&mut page, "Type", Object::Name(Name::from("Page")));
        put(&mut page, "Parent", Object::Ref(ObjId::new(2, 0)));
        put(
            &mut page,
            "MediaBox",
            Object::Array(vec![
                real(media_box.x0),
                real(media_box.y0),
                real(media_box.x1),
                real(media_box.y1),
            ]),
        );
        if a.rotate != 0 {
            put(&mut page, "Rotate", Object::Integer(a.rotate));
        }
        put(&mut page, "Resources", Object::Dict(cut.resources));
        put(&mut page, "Contents", Object::Ref(ObjId::new(4, 0)));
        for k in ["Group", "UserUnit"] {
            if let Some(v) = src.and_then(|d| d.get_key(k)) {
                let v = c.out.copy(v);
                put(&mut page, k, v);
            }
        }
        let data = deflate(&cut.data);
        let mut cs = Dict::new();
        cs.insert(
            Name::from("Filter"),
            Node::synthetic(Object::Name(Name::from("FlateDecode"))),
        );
        cs.insert(
            Name::from("Length"),
            Node::synthetic(Object::Integer(data.len() as i64)),
        );
        c.out.set(4, Obj::Stream(cs, data));
        c.out.set(3, Obj::Plain(Object::Dict(page)));
        let mut pages = Dict::new();
        put(&mut pages, "Type", Object::Name(Name::from("Pages")));
        put(
            &mut pages,
            "Kids",
            Object::Array(vec![Node::synthetic(Object::Ref(ObjId::new(3, 0)))]),
        );
        put(&mut pages, "Count", Object::Integer(1));
        c.out.set(2, Obj::Plain(Object::Dict(pages)));

        let catalog = file
            .trailer()
            .get_key("Root")
            .map(|r| resolve(file, r).1.as_dict().cloned());
        let catalog = catalog.flatten();
        let mut root = Dict::new();
        put(&mut root, "Type", Object::Name(Name::from("Catalog")));
        put(&mut root, "Pages", Object::Ref(ObjId::new(2, 0)));
        // Optional content the kept operations may be marked with.
        if let Some(oc) = catalog.as_ref().and_then(|d| d.get_key("OCProperties")) {
            let oc = c.out.copy(oc);
            put(&mut root, "OCProperties", oc);
        }
        c.out.set(1, Obj::Plain(Object::Dict(root)));
        c.out.finish()?;

        let header = file
            .header()
            .version
            .clone()
            .unwrap_or_else(|| "1.7".into());
        let declared = catalog
            .as_ref()
            .and_then(|d| d.get_name("Version"))
            .map(|n| n.as_str_lossy().into_owned())
            .unwrap_or_default();
        let version = later_version(&header, &declared).to_owned();
        Ok(Extract {
            pdf: write_file(&version, &c.out.objects, 1),
            media_box,
            kept: cut.kept,
            dropped: cut.dropped,
            forms: cut.forms,
            objects: c.out.objects.len() as u64,
        })
    }
}
