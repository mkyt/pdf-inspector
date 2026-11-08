//! Finding what a content paints by what it is (operators, kind, place,
//! color, text) and what it sits in (marked content, optional content), to
//! remove it: [`Content::find`].
//!
//! The candidates are the painted elements of the content (a path from its
//! first construction operator to the painting one, a text-showing
//! operation, `Do`, `sh`, an inline image); with [`Unit::Op`] also the other
//! operations, for queries that need nothing an element has. What paints
//! inside a form XObject is searched too, but it is found as the `Do` that
//! paints the form (forms may be shared by several pages).

use std::sync::Arc;

use pdfi_model::{Rect, resolve};
use pdfi_syntax::{DictExt, Name, ObjId, Object, PdfFile};

use crate::geom::{IDENTITY, Matrix, intersect, mul};
use crate::interp::{Color, ElementKind, Env, GState, Interp, Kind, Lookup, Rec, Resources, State};
use crate::op::{Category, Operator};
use crate::parse::{OpReader, SourceData};
use crate::{Analysis, Content, DocContent, Issues};

/// Hits returned at most.
pub const MAX_HITS: usize = 100_000;
/// Form XObjects searched inside each other.
const MAX_DEPTH: usize = 12;
/// Operations run inside form XObjects for one search.
const FORM_BUDGET: u64 = 4_000_000;

/// How [`Query::bbox`] relates an element's bounding box to a rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    Inside,
    /// Not overlapping (touching counts as outside).
    Outside,
    Intersects,
}

/// What a hit covers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Unit {
    /// Single operations.
    Op,
    /// The painted element: removing it leaves the rest as it painted.
    #[default]
    Element,
    /// The `q`…`Q` or `BMC` / `BDC`…`EMC` around the element (the marked
    /// content that matched, when the query names some).
    Block,
}

/// Marked content an element sits in (any level).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarkedQuery {
    /// Tags (`Artifact`, `OC`, ...); empty: any.
    pub tags: Vec<String>,
    /// Entries the property list has: names, text strings, numbers and
    /// booleans compared as text.
    pub properties: Vec<(String, String)>,
}

/// The color an element paints with (fill or stroke, as it paints).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorQuery {
    /// Color space families (`DeviceCMYK`, `Separation`, ...); empty: any.
    pub families: Vec<String>,
    /// Colorants of a `Separation` or `DeviceN` space (`All` for
    /// registration marks); empty: any.
    pub separations: Vec<String>,
}

/// What to find; every criterion given must hold.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    /// Operator names: one of the element's operations is one of them.
    pub operators: Option<Vec<String>>,
    pub kinds: Option<Vec<ElementKind>>,
    pub marked: Option<MarkedQuery>,
    /// Optional content groups: the element sits in marked content `/OC`
    /// (or a form or image with `/OC`) naming one of them, directly or by
    /// a membership dictionary.
    pub ocgs: Option<Vec<ObjId>>,
    pub color: Option<ColorQuery>,
    /// In the content's space (default user space for pages).
    pub bbox: Option<(Relation, Rect)>,
    /// The decoded text shown contains it.
    pub text: Option<String>,
    pub hidden: Option<bool>,
    pub unit: Unit,
    /// Only elements whose (last) operation is in `from..to`.
    pub range: Option<(usize, usize)>,
}

impl Query {
    /// Criteria only painted elements can meet.
    fn element_only(&self) -> bool {
        self.kinds.is_some()
            || self.color.is_some()
            || self.bbox.is_some()
            || self.text.is_some()
            || self.hidden.is_some()
    }
}

/// Something found: operations `from..to`.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub from: u32,
    pub to: u32,
    /// The operation that paints (or the one found, for [`Unit::Op`]).
    pub op: u32,
    /// `None` for an operation that paints nothing.
    pub kind: Option<ElementKind>,
    pub bbox: Option<Rect>,
    pub hidden: bool,
    /// The text shown, decoded.
    pub text: Option<String>,
    /// Found inside this form XObject (painted by the `Do` at `op`).
    pub form: Option<ObjId>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    pub hits: Vec<Hit>,
    /// Stopped at [`MAX_HITS`].
    pub truncated: bool,
    /// Stopped following forms at the operation budget.
    pub forms_truncated: bool,
}

/// Called with each hit, the blocks open around it and the matched marked
/// content (among those open); returns false to stop.
type OnHit<'f> = dyn FnMut(Hit, &[Open], Option<usize>) -> bool + 'f;

/// An open marked content (or the `/OC` of a form or image).
#[derive(Clone, Debug)]
struct Mark {
    tag: String,
    props: Option<Arc<Object>>,
    props_id: Option<ObjId>,
}

/// `q` (`None`) or marked content, with its opener.
struct Open {
    opener: u32,
    mark: Option<Mark>,
}

/// A painted element as the query sees it.
struct Elem<'e> {
    kind: ElementKind,
    bbox: Rect,
    hidden: bool,
    ops: &'e [Operator],
    fill: &'e Color,
    stroke: &'e Color,
    render: i64,
    text: Option<&'e str>,
}

struct Scan<'a, 'q> {
    file: &'a PdfFile,
    cache: &'a DocContent,
    q: &'q Query,
    operators: Option<Vec<Operator>>,
    budget: u64,
    active: Vec<ObjId>,
    cancel: &'a dyn Fn() -> bool,
    cancelled: bool,
    forms_truncated: bool,
}

fn text_of(file: &PdfFile, o: &Object) -> Option<String> {
    let (_, r) = resolve(file, o);
    Some(match &*r {
        Object::Name(n) => n.as_str_lossy().into_owned(),
        Object::String(s) => pdfi_model::text::decode(&s.bytes),
        Object::Integer(i) => i.to_string(),
        Object::Real(x) => x.to_string(),
        Object::Bool(b) => b.to_string(),
        _ => return None,
    })
}

fn same_value(have: &str, want: &str) -> bool {
    if have == want {
        return true;
    }
    match (have.parse::<f64>(), want.parse::<f64>()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The colorants of a `Separation` or `DeviceN` color.
fn colorants(file: &PdfFile, c: &Color) -> Vec<String> {
    let Some(def) = &c.def else {
        return Vec::new();
    };
    let (_, r) = resolve(file, def);
    let Some(a) = r.as_array() else {
        return Vec::new();
    };
    let Some(arg) = a.get(1) else {
        return Vec::new();
    };
    let (_, v) = resolve(file, &arg.obj);
    match (c.family.as_str(), &*v) {
        ("Separation", Object::Name(n)) => vec![n.as_str_lossy().into_owned()],
        ("DeviceN", Object::Array(names)) => names
            .iter()
            .filter_map(|n| n.obj.as_name().map(|n| n.as_str_lossy().into_owned()))
            .collect(),
        _ => Vec::new(),
    }
}

impl Scan<'_, '_> {
    fn color_ok(&self, cq: &ColorQuery, c: &Color) -> bool {
        (cq.families.is_empty() || cq.families.contains(&c.family))
            && (cq.separations.is_empty() || {
                let names = colorants(self.file, c);
                names.iter().any(|n| cq.separations.contains(n))
            })
    }

    fn mark_ok(&self, mq: &MarkedQuery, m: &Mark) -> bool {
        if !mq.tags.is_empty() && !mq.tags.contains(&m.tag) {
            return false;
        }
        if mq.properties.is_empty() {
            return true;
        }
        let Some(d) = m.props.as_deref().and_then(Object::as_dict) else {
            return false;
        };
        mq.properties.iter().all(|(k, want)| {
            let k = k.strip_prefix('/').unwrap_or(k);
            d.get_key(k)
                .and_then(|v| text_of(self.file, v))
                .is_some_and(|have| same_value(&have, want))
        })
    }

    /// The marked content names one of `ocgs` (an OCG, or a membership
    /// dictionary listing one).
    fn oc_ok(&self, ocgs: &[ObjId], m: &Mark) -> bool {
        if m.tag != "OC" {
            return false;
        }
        if m.props_id.is_some_and(|id| ocgs.contains(&id)) {
            return true;
        }
        let Some(d) = m.props.as_deref().and_then(Object::as_dict) else {
            return false;
        };
        if !d.type_is("OCMD") {
            return false;
        }
        let Some(v) = d.get_key("OCGs") else {
            return false;
        };
        let (id, r) = resolve(self.file, v);
        if id.is_some_and(|id| ocgs.contains(&id)) {
            return true;
        }
        r.as_array().is_some_and(|a| {
            a.iter()
                .any(|n| n.obj.as_ref_id().is_some_and(|id| ocgs.contains(&id)))
        })
    }

    /// Whether an element (`None`: an operation that paints nothing)
    /// meets the query; `Some(k)`: the innermost mark the query matched
    /// (`usize::MAX` when it names none).
    fn matches(&self, e: Option<&Elem>, op: &Operator, marks: &[&Mark]) -> Option<usize> {
        let q = self.q;
        if let Some(ops) = &self.operators {
            let any = match e {
                Some(e) => e.ops.iter().any(|o| ops.contains(o)),
                None => ops.contains(op),
            };
            if !any {
                return None;
            }
        }
        if let Some(e) = e {
            // A clipping path paints nothing: found only when asked for.
            let kind_ok = match &q.kinds {
                Some(k) => k.contains(&e.kind),
                None => e.kind != ElementKind::Clip,
            };
            if !kind_ok {
                return None;
            }
        }
        if q.element_only() {
            let e = e?;
            if q.hidden.is_some_and(|h| h != e.hidden) {
                return None;
            }
            if let Some((rel, r)) = &q.bbox {
                let b = &e.bbox;
                let ok = match rel {
                    Relation::Inside => {
                        b.x0 >= r.x0 && b.y0 >= r.y0 && b.x1 <= r.x1 && b.y1 <= r.y1
                    }
                    Relation::Intersects => intersect(b, r).is_some(),
                    Relation::Outside => {
                        intersect(b, r).is_none_or(|x| x.x0 == x.x1 || x.y0 == x.y1)
                    }
                };
                if !ok {
                    return None;
                }
            }
            if let Some(t) = &q.text {
                if !e.text.is_some_and(|s| s.contains(t.as_str())) {
                    return None;
                }
            }
            if let Some(cq) = &q.color {
                use ElementKind as K;
                let (fill, stroke) = match e.kind {
                    K::Fill | K::InlineImage => (true, false),
                    K::Stroke => (false, true),
                    K::FillStroke => (true, true),
                    K::Text => match e.render {
                        1 | 5 => (false, true),
                        2 | 6 => (true, true),
                        _ => (true, false),
                    },
                    _ => (false, false),
                };
                if !((fill && self.color_ok(cq, e.fill)) || (stroke && self.color_ok(cq, e.stroke)))
                {
                    return None;
                }
            }
        }
        let mut found = usize::MAX;
        if let Some(mq) = &q.marked {
            found = marks.iter().rposition(|m| self.mark_ok(mq, m))?;
        }
        if let Some(ocgs) = &q.ocgs {
            let k = marks.iter().rposition(|m| self.oc_ok(ocgs, m))?;
            if found == usize::MAX {
                found = k;
            }
        }
        Some(found)
    }

    /// The `/OC` of an XObject a `Do` paints, as a mark.
    fn xobject_oc(
        &self,
        res: &Resources,
        name: &Name,
    ) -> Option<(Option<ObjId>, Object, Option<Mark>)> {
        let Lookup::Found(id, o) = res.lookup(self.file, "XObject", name) else {
            return None;
        };
        let mark = o.as_stream().and_then(|s| s.dict.get_key("OC")).map(|oc| {
            let (oid, r) = resolve(self.file, oc);
            Mark {
                tag: "OC".into(),
                props: Some(Arc::new((*r).clone())),
                props_id: oid,
            }
        });
        Some((id, o, mark))
    }

    /// Run a content from `start`, calling `found` for every candidate that
    /// meets the query with (hit, the marks in scope, the matched mark's
    /// position or `usize::MAX`, the open blocks). Returns false when
    /// `found` asked to stop.
    #[allow(clippy::too_many_arguments)]
    fn run(
        &mut self,
        data: &[SourceData],
        res: &Resources,
        start: State,
        kind: Kind,
        outer: &[Mark],
        top: bool,
        found: &mut OnHit<'_>,
    ) -> bool {
        let mut env = Env::new(self.file, self.cache);
        let mut interp = Interp::new(start, res, kind);
        let mut rec = Rec::elements_only();
        let mut reader = OpReader::new(data);
        let mut sink = Issues::default();
        let mut open: Vec<Open> = Vec::new();
        // Operators of the path being built, and where it began.
        let mut path: Vec<Operator> = Vec::new();
        let mut path_start = 0u32;
        let range = if top { self.q.range } else { None };
        loop {
            let i = reader.count() as u32;
            if i % 256 == 0 && (self.cancel)() {
                self.cancelled = true;
                return false;
            }
            if !top {
                if self.budget == 0 {
                    self.forms_truncated = true;
                    return true;
                }
                self.budget -= 1;
            }
            let Some(op) = reader.next(&mut sink) else {
                return true;
            };
            if let Some((_, to)) = range {
                if i as usize >= to {
                    return true;
                }
            }
            let cat = op.operator.category();
            // Marked content opens before the operation runs, closes after.
            match &op.operator {
                Operator::Save => open.push(Open {
                    opener: i,
                    mark: None,
                }),
                Operator::BeginMarked | Operator::BeginMarkedProps => {
                    let tag = op
                        .operands
                        .first()
                        .and_then(|n| n.obj.as_name())
                        .map(|n| n.as_str_lossy().into_owned())
                        .unwrap_or_default();
                    let (props, props_id) = match op.operands.get(1).map(|n| &n.obj) {
                        Some(Object::Name(n)) if op.operator == Operator::BeginMarkedProps => {
                            match res.lookup(self.file, "Properties", n) {
                                Lookup::Found(id, o) => (Some(Arc::new(o)), id),
                                Lookup::Missing => (None, None),
                            }
                        }
                        Some(o @ Object::Dict(_)) => (Some(Arc::new(o.clone())), None),
                        _ => (None, None),
                    };
                    open.push(Open {
                        opener: i,
                        mark: Some(Mark {
                            tag,
                            props,
                            props_id,
                        }),
                    });
                }
                _ => {}
            }
            if cat == Category::PathConstruction && interp.s.path.is_none() {
                path.clear();
                path_start = i;
            }
            let in_path = interp.s.path.is_some() || cat == Category::PathConstruction;
            if in_path {
                path.push(op.operator.clone());
            }
            // A `Do`: the XObject it paints (for `/OC` and looking inside).
            let xobject = match (&op.operator, op.operands.first().map(|n| &n.obj)) {
                (Operator::XObject, Some(Object::Name(n))) => self.xobject_oc(res, n),
                _ => None,
            };
            let ctm_before: Matrix = interp.s.g.ctm;
            rec.elements.clear();
            rec.shows.clear();
            interp.step(&mut env, i, &op, &mut rec);
            let element = rec.elements.pop();
            let in_range = range.is_none_or(|(from, _)| i as usize >= from);
            let mut keep_going = true;
            if in_range {
                let mut marks: Vec<&Mark> = outer.iter().collect();
                marks.extend(open.iter().filter_map(|o| o.mark.as_ref()));
                if let Some(m) = xobject.as_ref().and_then(|x| x.2.as_ref()) {
                    marks.push(m);
                }
                match element {
                    Some(el) => {
                        let shown: String;
                        let text = match el.kind {
                            ElementKind::Text | ElementKind::Form if !rec.shows.is_empty() => {
                                shown = rec
                                    .shows
                                    .iter()
                                    .map(|s| s.text.as_str())
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                Some(shown.as_str())
                            }
                            _ => None,
                        };
                        let single = [op.operator.clone()];
                        let painting = matches!(cat, Category::PathPainting | Category::Clipping);
                        let ops: &[Operator] = if painting { &path } else { &single };
                        let from = if painting { path_start } else { i };
                        let g = &interp.s.g;
                        let e = Elem {
                            kind: el.kind,
                            bbox: el.bbox,
                            hidden: el.hidden,
                            ops,
                            fill: &g.fill,
                            stroke: &g.stroke,
                            render: g.text.render,
                            text,
                        };
                        let mut hit_mark = self.matches(Some(&e), &op.operator, &marks);
                        let mut inner: Option<(ObjId, Option<String>)> = None;
                        if hit_mark.is_none() && el.kind == ElementKind::Form {
                            if let Some((Some(id), o, _)) = &xobject {
                                // `marks` ends with the form's `/OC`.
                                let m: Vec<Mark> = marks.iter().map(|m| (*m).clone()).collect();
                                if let Some(t) = self.inside_form(*id, o, res, g, ctm_before, &m) {
                                    inner = Some((*id, t));
                                    hit_mark = Some(usize::MAX);
                                }
                            }
                        }
                        if let Some(k) = hit_mark {
                            // A mark from `outer` or the XObject's `/OC` has no
                            // block here.
                            let n_outer = outer.len();
                            let block_mark = (k != usize::MAX && k >= n_outer)
                                .then(|| k - n_outer)
                                .filter(|&k| k < open.iter().filter(|o| o.mark.is_some()).count());
                            let mut ops_found: Vec<(u32, u32)> = Vec::new();
                            if self.q.unit == Unit::Op {
                                for (k, o) in ops.iter().enumerate() {
                                    let at = from + k as u32;
                                    let wanted = match &self.operators {
                                        Some(list) => list.contains(o),
                                        None => at == i,
                                    };
                                    if wanted {
                                        ops_found.push((at, at));
                                    }
                                }
                                if ops_found.is_empty() {
                                    ops_found.push((i, i));
                                }
                            } else {
                                ops_found.push((from, i));
                            }
                            for (a, b) in ops_found {
                                let hit = Hit {
                                    from: a,
                                    to: b,
                                    op: i,
                                    kind: Some(el.kind),
                                    bbox: Some(el.bbox),
                                    hidden: el.hidden,
                                    text: match &inner {
                                        Some((_, t)) => t.clone(),
                                        None => text.map(str::to_owned),
                                    },
                                    form: inner.as_ref().map(|x| x.0),
                                };
                                if !found(hit, &open, block_mark) {
                                    keep_going = false;
                                    break;
                                }
                            }
                        }
                    }
                    None if !in_path
                        && self.q.unit == Unit::Op
                        && !self.q.element_only()
                        && !matches!(cat, Category::PathPainting | Category::Clipping) =>
                    {
                        if let Some(k) = self.matches(None, &op.operator, &marks) {
                            let n_outer = outer.len();
                            let block_mark = (k != usize::MAX && k >= n_outer)
                                .then(|| k - n_outer)
                                .filter(|&k| k < open.iter().filter(|o| o.mark.is_some()).count());
                            let hit = Hit {
                                from: i,
                                to: i,
                                op: i,
                                kind: None,
                                bbox: None,
                                hidden: false,
                                text: None,
                                form: None,
                            };
                            keep_going = found(hit, &open, block_mark);
                        }
                    }
                    None => {}
                }
            }
            if matches!(cat, Category::PathPainting) {
                path.clear();
            }
            match &op.operator {
                Operator::Restore => {
                    if let Some(k) = open.iter().rposition(|o| o.mark.is_none()) {
                        open.truncate(k);
                    }
                }
                Operator::EndMarked => {
                    if let Some(k) = open.iter().rposition(|o| o.mark.is_some()) {
                        open.truncate(k);
                    }
                }
                _ => {}
            }
            if !keep_going || self.cancelled {
                return false;
            }
        }
    }

    /// Whether something a form paints meets the query; the text of what
    /// does.
    #[allow(clippy::too_many_arguments)]
    fn inside_form(
        &mut self,
        id: ObjId,
        o: &Object,
        res: &Resources,
        g: &GState,
        ctm: Matrix,
        marks: &[Mark],
    ) -> Option<Option<String>> {
        let s = o.as_stream()?;
        if self.active.contains(&id) || self.active.len() >= MAX_DEPTH || self.budget == 0 {
            return None;
        }
        let f = self.file;
        let matrix = s
            .dict
            .get_key("Matrix")
            .map(|m| resolve(f, m).1)
            .and_then(|m| {
                m.as_array().filter(|a| a.len() == 6).map(|a| {
                    let mut x = IDENTITY;
                    for (v, n) in x.iter_mut().zip(a.iter()) {
                        *v = n.obj.as_f64().unwrap_or(0.0);
                    }
                    x
                })
            })
            .unwrap_or(IDENTITY);
        let sub_res = match s.dict.get_key("Resources") {
            Some(r) => Resources::new(f, Some(r)),
            None => res.inherit(),
        };
        let mut g = g.clone();
        g.ctm = mul(&matrix, &ctm);
        g.pattern_ctm = g.ctm;
        let data = self.cache.form_data(f, id, s);
        self.active.push(id);
        let mut text: Option<String> = None;
        let mut any = false;
        self.run(
            &data,
            &sub_res,
            State::new(g),
            Kind::Form,
            marks,
            false,
            &mut |hit, _, _| {
                any = true;
                if let Some(t) = hit.text {
                    match &mut text {
                        Some(s) => {
                            s.push(' ');
                            s.push_str(&t);
                        }
                        None => text = Some(t),
                    }
                    // Enough text to show; keep looking only for more.
                    text.as_ref().is_some_and(|s| s.len() < 4096)
                } else {
                    false
                }
            },
        );
        self.active.pop();
        any.then_some(text)
    }
}

impl Content<'_> {
    /// What an analyzed content paints that meets `q`, in operation order
    /// (equal ranges once). `cancel` is polled every few hundred
    /// operations: a cancelled search returns `Err`.
    pub fn find(
        &self,
        a: &Analysis,
        q: &Query,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Found, String> {
        let operators = q.operators.as_ref().map(|v| {
            v.iter()
                .map(|s| Operator::from_bytes(s.as_bytes()))
                .collect()
        });
        let mut scan = Scan {
            file: self.m.file,
            cache: self.c,
            q,
            operators,
            budget: FORM_BUDGET,
            active: Vec::new(),
            cancel,
            cancelled: false,
            forms_truncated: false,
        };
        let res = Resources::new(self.m.file, a.resources.as_ref());
        let start = State::new(GState::new(IDENTITY, a.bounds));
        let mut hits: Vec<Hit> = Vec::new();
        let mut truncated = false;
        scan.run(
            &a.data,
            &res,
            start,
            a.kind,
            &[],
            true,
            &mut |mut hit, open, mark| {
                if q.unit == Unit::Block {
                    let opener = match mark {
                        // The matched marked content.
                        Some(k) => open
                            .iter()
                            .filter(|o| o.mark.is_some())
                            .nth(k)
                            .map(|o| o.opener),
                        None => open.last().map(|o| o.opener),
                    };
                    if let Some(end) = opener.and_then(|s| a.block_end(s).map(|e| (s, e))) {
                        hit.from = end.0;
                        hit.to = end.1.max(hit.to);
                    }
                }
                match hits.last_mut() {
                    Some(last) if last.from == hit.from && last.to == hit.to => {
                        if let Some(t) = hit.text {
                            match &mut last.text {
                                Some(s) if s.len() < 4096 => {
                                    s.push(' ');
                                    s.push_str(&t);
                                }
                                Some(_) => {}
                                None => last.text = Some(t),
                            }
                        }
                    }
                    _ => {
                        if hits.len() >= MAX_HITS {
                            truncated = true;
                            return false;
                        }
                        hits.push(hit);
                    }
                }
                true
            },
        );
        if scan.cancelled {
            return Err("cancelled".into());
        }
        // Ranges end before `to` from here.
        for h in &mut hits {
            h.to += 1;
        }
        Ok(Found {
            hits,
            truncated,
            forms_truncated: scan.forms_truncated,
        })
    }
}
