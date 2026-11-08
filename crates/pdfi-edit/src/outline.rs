//! The outline (bookmarks) and page labels, replaced as a whole.

use std::collections::HashSet;

use pdfi_model::DocModel;
use pdfi_syntax::{Dict, DictExt, Name, Node, ObjId, Object};

use crate::value::text_string;
use crate::{EditError, Result, Tx, err};

/// Outline items deleted with an old outline at most (more are left).
const MAX_OLD: usize = 1_000_000;

/// Where an outline item goes.
#[derive(Clone, Debug, PartialEq)]
pub enum OutlineTarget {
    /// A page of this document by index, viewed as `fit` (`XYZ`, `Fit`,
    /// `FitH`, ...) with its parameters (`None` written as null).
    Page {
        index: u64,
        fit: String,
        params: Vec<Option<f64>>,
    },
    /// A named destination.
    Named(String),
    Uri(String),
}

/// One outline item, in a list in reading order (an item's children
/// follow it, one level deeper).
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineSpec {
    /// 0 for the top level.
    pub level: u32,
    pub title: String,
    pub target: Option<OutlineTarget>,
    /// Its children are shown.
    pub open: bool,
    /// 1 italic, 2 bold.
    pub flags: i64,
    /// RGB, 0..1.
    pub color: Option<[f64; 3]>,
}

/// How page numbers of a label range are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelStyle {
    Decimal,
    UpperRoman,
    LowerRoman,
    UpperLetters,
    LowerLetters,
}

impl LabelStyle {
    fn name(self) -> &'static str {
        match self {
            LabelStyle::Decimal => "D",
            LabelStyle::UpperRoman => "R",
            LabelStyle::LowerRoman => "r",
            LabelStyle::UpperLetters => "A",
            LabelStyle::LowerLetters => "a",
        }
    }
}

/// Labels of the pages from `start` (0-based) to the next range.
#[derive(Clone, Debug, PartialEq)]
pub struct LabelSpec {
    pub start: u64,
    /// `None`: only the prefix.
    pub style: Option<LabelStyle>,
    pub prefix: Option<String>,
    /// The number of the first page (1 when `None`).
    pub first: Option<i64>,
}

fn dict(entries: Vec<(&str, Object)>) -> Dict {
    let mut d = Dict::new();
    for (k, v) in entries {
        d.insert(Name::from(k), Node::synthetic(v));
    }
    d
}

fn arr(v: Vec<Object>) -> Object {
    Object::Array(v.into_iter().map(Node::synthetic).collect())
}

/// The items of an outline (and its root), following `/First` and `/Next`.
fn old_items(tx: &Tx, root: ObjId) -> Vec<ObjId> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut todo = vec![root];
    seen.insert(root);
    while let Some(id) = todo.pop() {
        out.push(id);
        if out.len() >= MAX_OLD {
            break;
        }
        let Ok(o) = tx.get(id) else { continue };
        let Some(d) = o.node.obj.as_dict() else {
            continue;
        };
        for k in ["First", "Next"] {
            if let Some(next) = d.get_key(k).and_then(Object::as_ref_id) {
                if seen.insert(next) {
                    todo.push(next);
                }
            }
        }
    }
    out
}

pub(crate) fn set_outline(tx: &mut Tx, items: &[OutlineSpec]) -> Result<()> {
    // The parent of each item (`None`: the top level).
    let mut parents: Vec<Option<usize>> = Vec::with_capacity(items.len());
    let mut stack: Vec<usize> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let level = it.level as usize;
        if level > stack.len() {
            return err(format!(
                "outline item {} ({:?}) is at level {level} after level {}",
                i + 1,
                it.title,
                stack.len().saturating_sub(1)
            ));
        }
        stack.truncate(level);
        parents.push(stack.last().copied());
        stack.push(i);
    }
    let view = tx.view();
    let dm = DocModel::new();
    let m = dm.on(&view);
    let count = m.page_count();
    let mut pages = Vec::with_capacity(items.len());
    for (i, it) in items.iter().enumerate() {
        pages.push(match &it.target {
            Some(OutlineTarget::Page { index, .. }) => {
                if *index >= count {
                    return err(format!(
                        "outline item {} ({:?}) goes to page {} (the document has {count})",
                        i + 1,
                        it.title,
                        index + 1
                    ));
                }
                Some(m.page(*index).map_err(EditError)?.id)
            }
            _ => None,
        });
    }

    let (cid, mut cat) = tx.catalog()?;
    if let Some(old) = cat.get_key("Outlines").and_then(Object::as_ref_id) {
        for id in old_items(tx, old) {
            // Objects shared with something else stay (best effort: only
            // outline dictionaries are deleted).
            let is_outline = tx.get(id).is_ok_and(|o| {
                o.node.obj.as_dict().is_some_and(|d| {
                    d.get_key("Title").is_some() || d.type_is("Outlines") || id == old
                })
            });
            if is_outline {
                tx.delete(id)?;
            }
        }
    }
    if items.is_empty() {
        cat.shift_remove(b"Outlines".as_slice());
        tx.put(cid, Object::Dict(cat));
        return Ok(());
    }

    let root = tx.add(Object::Null);
    let ids: Vec<ObjId> = items.iter().map(|_| tx.add(Object::Null)).collect();
    // Children of each item, and of the root (last).
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); items.len() + 1];
    for (i, p) in parents.iter().enumerate() {
        children[p.unwrap_or(items.len())].push(i);
    }
    // Visible descendants when open, deepest items first.
    let mut visible = vec![0i64; items.len() + 1];
    for i in (0..items.len()).rev() {
        let v: i64 = children[i]
            .iter()
            .map(|&c| 1 + if items[c].open { visible[c] } else { 0 })
            .sum();
        visible[i] = v;
    }
    visible[items.len()] = children[items.len()]
        .iter()
        .map(|&c| 1 + if items[c].open { visible[c] } else { 0 })
        .sum();
    let node_ref = |k: usize| {
        if k == items.len() { root } else { ids[k] }
    };
    for (i, it) in items.iter().enumerate() {
        let parent = parents[i].unwrap_or(items.len());
        let sib = &children[parent];
        let pos = sib.iter().position(|&c| c == i).expect("listed above");
        let mut e: Vec<(&str, Object)> = vec![
            ("Title", Object::String(text_string(&it.title))),
            ("Parent", Object::Ref(node_ref(parent))),
        ];
        if pos > 0 {
            e.push(("Prev", Object::Ref(ids[sib[pos - 1]])));
        }
        if let Some(&n) = sib.get(pos + 1) {
            e.push(("Next", Object::Ref(ids[n])));
        }
        if let (Some(&first), Some(&last)) = (children[i].first(), children[i].last()) {
            e.push(("First", Object::Ref(ids[first])));
            e.push(("Last", Object::Ref(ids[last])));
            let n = visible[i];
            e.push(("Count", Object::Integer(if it.open { n } else { -n })));
        }
        match &it.target {
            Some(OutlineTarget::Page { fit, params, .. }) => {
                let page = pages[i].expect("looked up above");
                let mut d = vec![
                    Object::Ref(page),
                    Object::Name(Name::from(if fit.is_empty() {
                        "Fit"
                    } else {
                        fit.as_str()
                    })),
                ];
                d.extend(params.iter().map(|p| match p {
                    Some(x) => Object::Real(*x),
                    None => Object::Null,
                }));
                e.push(("Dest", arr(d)));
            }
            Some(OutlineTarget::Named(n)) => {
                e.push(("Dest", Object::String(text_string(n))));
            }
            Some(OutlineTarget::Uri(u)) => {
                let a = dict(vec![
                    ("S", Object::Name(Name::from("URI"))),
                    (
                        "URI",
                        Object::String(pdfi_syntax::PdfString::literal(u.as_bytes().to_vec())),
                    ),
                ]);
                e.push(("A", Object::Dict(a)));
            }
            None => {}
        }
        if let Some(c) = it.color {
            if c.iter().any(|&x| !(0.0..=1.0).contains(&x)) {
                return err(format!(
                    "outline item {} ({:?}): color components are 0..1",
                    i + 1,
                    it.title
                ));
            }
            e.push(("C", arr(c.iter().map(|&x| Object::Real(x)).collect())));
        }
        if it.flags != 0 {
            e.push(("F", Object::Integer(it.flags & 3)));
        }
        tx.put(ids[i], Object::Dict(dict(e)));
    }
    let top = &children[items.len()];
    let r = dict(vec![
        ("Type", Object::Name(Name::from("Outlines"))),
        ("First", Object::Ref(ids[top[0]])),
        ("Last", Object::Ref(ids[*top.last().expect("not empty")])),
        ("Count", Object::Integer(visible[items.len()])),
    ]);
    tx.put(root, Object::Dict(r));
    cat.insert(Name::from("Outlines"), Node::synthetic(Object::Ref(root)));
    tx.put(cid, Object::Dict(cat));
    Ok(())
}

pub(crate) fn set_page_labels(tx: &mut Tx, ranges: &[LabelSpec]) -> Result<()> {
    let mut ranges = ranges.to_vec();
    ranges.sort_by_key(|r| r.start);
    if let Some(w) = ranges.windows(2).find(|w| w[0].start == w[1].start) {
        return err(format!("two label ranges start at page {}", w[0].start + 1));
    }
    let view = tx.view();
    let count = DocModel::new().on(&view).page_count();
    if let Some(r) = ranges.iter().find(|r| r.start >= count.max(1)) {
        return err(format!(
            "a label range starts at page {} (the document has {count})",
            r.start + 1
        ));
    }
    if ranges.first().is_some_and(|r| r.start != 0) {
        return err("the first label range must start at the first page (0)");
    }
    let (cid, mut cat) = tx.catalog()?;
    if ranges.is_empty() {
        cat.shift_remove(b"PageLabels".as_slice());
        tx.put(cid, Object::Dict(cat));
        return Ok(());
    }
    let mut nums = Vec::new();
    for r in &ranges {
        let mut e: Vec<(&str, Object)> = Vec::new();
        if let Some(s) = r.style {
            e.push(("S", Object::Name(Name::from(s.name()))));
        }
        if let Some(p) = r.prefix.as_deref().filter(|p| !p.is_empty()) {
            e.push(("P", Object::String(text_string(p))));
        }
        if let Some(st) = r.first.filter(|&n| n != 1) {
            if st < 1 {
                return err(format!(
                    "label range at page {}: numbers start at 1 or more",
                    r.start + 1
                ));
            }
            e.push(("St", Object::Integer(st)));
        }
        nums.push(Object::Integer(r.start as i64));
        nums.push(Object::Dict(dict(e)));
    }
    let tree = Object::Dict(dict(vec![("Nums", arr(nums))]));
    match cat.get_key("PageLabels").and_then(Object::as_ref_id) {
        // Kept where it is (other nodes of an old tree are left).
        Some(id) if tx.get(id).is_ok() => tx.put(id, tree),
        _ => {
            cat.insert(Name::from("PageLabels"), Node::synthetic(tree));
            tx.put(cid, Object::Dict(cat));
        }
    }
    Ok(())
}
