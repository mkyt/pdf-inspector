//! Page operations as small edits of the page tree: kids are removed from
//! and inserted into their parents' `/Kids`, and the nodes on the way get
//! their `/Count` recomputed. Nodes left without kids are removed.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use pdfi_model::DocModel;
use pdfi_model::pages::PageRef;
use pdfi_syntax::{Dict, DictExt, Name, Node, ObjId, Object, PdfFile, Stream, StreamData};

use crate::{EditError, Result, Tx, err};

/// Attributes a page may inherit from its ancestors.
const INHERITED: [&str; 4] = ["Resources", "MediaBox", "CropBox", "Rotate"];

/// Pages looked up by index in the transaction's current state.
fn refs(tx: &Tx, pages: &[u64]) -> Result<(Vec<PageRef>, u64, Option<ObjId>)> {
    let view = tx.view();
    let dm = DocModel::new();
    let m = dm.on(&view);
    let count = m.page_count();
    let root = m.page_root();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut sorted = pages.to_vec();
    sorted.sort_unstable();
    for i in sorted {
        if !seen.insert(i) {
            continue;
        }
        if i >= count {
            return err(format!("no page {} (the document has {count})", i + 1));
        }
        out.push(m.page(i).map_err(EditError)?);
    }
    Ok((out, count, root))
}

/// Where new kids go: before the page at `at`, or after the last page.
struct Spot {
    parent: ObjId,
    /// Insert before this kid (`None`: at the end of `/Kids`).
    before: Option<ObjId>,
    /// After this kid (when appending after the last page).
    after: Option<ObjId>,
    /// Nodes whose `/Count` grows: the root down to `parent`.
    ancestors: Vec<ObjId>,
}

fn spot(tx: &Tx, at: u64, skip: &HashSet<ObjId>) -> Result<Spot> {
    let view = tx.view();
    let dm = DocModel::new();
    let m = dm.on(&view);
    let root = m
        .page_root()
        .ok_or_else(|| EditError("the catalog has no page tree".into()))?;
    let count = m.page_count();
    // The first page at or after `at` that is not being moved.
    let mut i = at.min(count);
    while i < count {
        let p = m.page(i).map_err(EditError)?;
        if !skip.contains(&p.id) {
            let parent = *p.ancestors.last().unwrap_or(&root);
            return Ok(Spot {
                parent,
                before: Some(p.id),
                after: None,
                ancestors: p.ancestors,
            });
        }
        i += 1;
    }
    // At the end: after the last page that stays.
    let mut j = count.min(at);
    while j > 0 {
        j -= 1;
        let p = m.page(j).map_err(EditError)?;
        if !skip.contains(&p.id) {
            let parent = *p.ancestors.last().unwrap_or(&root);
            return Ok(Spot {
                parent,
                before: None,
                after: Some(p.id),
                ancestors: p.ancestors,
            });
        }
    }
    Ok(Spot {
        parent: root,
        before: None,
        after: None,
        ancestors: vec![root],
    })
}

fn dict_of(tx: &Tx, id: ObjId) -> Result<Dict> {
    match &tx.get(id)?.node.obj {
        Object::Dict(d) => Ok(d.clone()),
        _ => err(format!("page tree node {id} is not a dictionary")),
    }
}

/// Change the `/Kids` of `node` (following an indirect array).
fn edit_kids(tx: &mut Tx, node: ObjId, f: impl FnOnce(&mut Vec<Node>)) -> Result<()> {
    let mut d = dict_of(tx, node)?;
    match d.get_key("Kids") {
        Some(Object::Ref(r)) => {
            let r = *r;
            let mut kids = match &tx.get(r)?.node.obj {
                Object::Array(a) => a.clone(),
                _ => Vec::new(),
            };
            f(&mut kids);
            tx.put(r, Object::Array(kids));
        }
        other => {
            let mut kids = match other {
                Some(Object::Array(a)) => a.clone(),
                _ => Vec::new(),
            };
            f(&mut kids);
            d.insert(Name::from("Kids"), Node::synthetic(Object::Array(kids)));
            tx.put(node, Object::Dict(d));
        }
    }
    Ok(())
}

fn kids_len(tx: &Tx, node: ObjId) -> Result<usize> {
    let d = dict_of(tx, node)?;
    Ok(match d.get_key("Kids").map(|k| tx.resolve(k)) {
        Some(Object::Array(a)) => a.len(),
        _ => 0,
    })
}

fn is_ref(n: &Node, id: ObjId) -> bool {
    n.obj.as_ref_id() == Some(id)
}

fn is_leaf(d: &Dict) -> bool {
    d.type_is("Page") || (!d.type_is("Pages") && d.get_key("Kids").is_none())
}

/// Set the `/Count` of the touched nodes below `node` from their kids
/// (untouched nodes are trusted when they have a count). Returns the
/// number of pages under `node`.
fn recount(tx: &mut Tx, node: ObjId, touched: &BTreeSet<ObjId>, depth: u32) -> Result<i64> {
    if depth > 64 {
        return err("the page tree is too deep");
    }
    let mut d = dict_of(tx, node)?;
    let kids = match d.get_key("Kids").map(|k| tx.resolve(k)) {
        Some(Object::Array(a)) => a,
        _ => Vec::new(),
    };
    let mut n = 0;
    for k in &kids {
        let Some(id) = k.obj.as_ref_id() else {
            continue;
        };
        let Ok(Object::Dict(kd)) = tx.get(id).map(|o| o.node.obj.clone()) else {
            continue;
        };
        n += if is_leaf(&kd) {
            1
        } else {
            match kd.get_key("Count").map(|c| tx.resolve(c).as_i64()) {
                Some(Some(c)) if !touched.contains(&id) => c,
                _ => recount(tx, id, touched, depth + 1)?,
            }
        };
    }
    if touched.contains(&node) {
        d.insert(Name::from("Count"), Node::synthetic(Object::Integer(n)));
        tx.put(node, Object::Dict(d));
    }
    Ok(n)
}

fn finish(tx: &mut Tx, root: Option<ObjId>, touched: &BTreeSet<ObjId>) -> Result<()> {
    match root.or_else(|| touched.first().copied()) {
        Some(r) => recount(tx, r, touched, 0).map(|_| ()),
        None => Ok(()),
    }
}

/// Detach pages from their parents (counts not adjusted).
fn detach(tx: &mut Tx, pages: &[PageRef], touched: &mut BTreeSet<ObjId>) -> Result<()> {
    for p in pages {
        let parent = *p
            .ancestors
            .last()
            .ok_or_else(|| EditError(format!("page {} has no parent", p.index + 1)))?;
        let id = p.id;
        edit_kids(tx, parent, |kids| kids.retain(|k| !is_ref(k, id)))?;
        touched.extend(p.ancestors.iter().copied());
    }
    Ok(())
}

/// Remove intermediate nodes left without kids, bottom up.
fn prune(tx: &mut Tx, pages: &[PageRef], root: Option<ObjId>) -> Result<()> {
    let mut done = HashSet::new();
    for p in pages {
        for k in (1..p.ancestors.len()).rev() {
            let node = p.ancestors[k];
            if Some(node) == root || !done.insert(node) {
                break;
            }
            if kids_len(tx, node)? > 0 {
                break;
            }
            let parent = p.ancestors[k - 1];
            edit_kids(tx, parent, |kids| kids.retain(|n| !is_ref(n, node)))?;
        }
    }
    Ok(())
}

pub(crate) fn delete(tx: &mut Tx, pages: &[u64]) -> Result<()> {
    let (refs, _, root) = refs(tx, pages)?;
    let mut touched = BTreeSet::new();
    detach(tx, &refs, &mut touched)?;
    prune(tx, &refs, root)?;
    finish(tx, root, &touched)
}

/// Copy inherited attributes onto the page so it keeps them under another
/// parent.
fn materialize(tx: &mut Tx, p: &PageRef) -> Result<()> {
    let mut page = dict_of(tx, p.id)?;
    let mut changed = false;
    for key in INHERITED {
        if page.contains_key(key.as_bytes()) {
            continue;
        }
        for &a in p.ancestors.iter().rev() {
            if let Some(v) = dict_of(tx, a)?.get(key.as_bytes()) {
                page.insert(Name::from(key), v.clone());
                changed = true;
                break;
            }
        }
    }
    if changed {
        tx.put(p.id, Object::Dict(page));
    }
    Ok(())
}

/// Give the page its own values of what the ancestors at `s` would
/// otherwise pass on to it (a crop box, a rotation, resources).
fn isolate(tx: &mut Tx, id: ObjId, s: &Spot) -> Result<()> {
    let mut page = dict_of(tx, id)?;
    let mut changed = false;
    for key in INHERITED {
        if page.contains_key(key.as_bytes()) {
            continue;
        }
        let mut inherited = false;
        for &a in &s.ancestors {
            inherited |= dict_of(tx, a)?.contains_key(key.as_bytes());
        }
        if !inherited {
            continue;
        }
        let v = match key {
            "Resources" => Object::Dict(Dict::new()),
            "Rotate" => Object::Integer(0),
            "CropBox" => match page.get_key("MediaBox") {
                Some(m) => m.clone(),
                None => continue,
            },
            _ => continue,
        };
        page.insert(Name::from(key), Node::synthetic(v));
        changed = true;
    }
    if changed {
        tx.put(id, Object::Dict(page));
    }
    Ok(())
}

/// Insert `ids` (pages) at `s` and point their `/Parent` there.
fn attach(tx: &mut Tx, s: &Spot, ids: &[ObjId], touched: &mut BTreeSet<ObjId>) -> Result<()> {
    let new: Vec<Node> = ids
        .iter()
        .map(|&id| Node::synthetic(Object::Ref(id)))
        .collect();
    edit_kids(tx, s.parent, |kids| {
        let at = match (s.before, s.after) {
            (Some(b), _) => kids.iter().position(|k| is_ref(k, b)),
            (_, Some(a)) => kids.iter().position(|k| is_ref(k, a)).map(|i| i + 1),
            _ => None,
        }
        .unwrap_or(kids.len());
        kids.splice(at..at, new);
    })?;
    for &id in ids {
        let mut d = dict_of(tx, id)?;
        let moved = d.get_key("Parent").and_then(Object::as_ref_id) != Some(s.parent);
        d.insert(Name::from("Parent"), Node::synthetic(Object::Ref(s.parent)));
        tx.put(id, Object::Dict(d));
        if moved {
            isolate(tx, id, s)?;
        }
    }
    touched.extend(s.ancestors.iter().copied());
    Ok(())
}

pub(crate) fn move_to(tx: &mut Tx, pages: &[u64], to: u64) -> Result<()> {
    let (refs, count, root) = refs(tx, pages)?;
    if refs.is_empty() {
        return Ok(());
    }
    let moving: HashSet<ObjId> = refs.iter().map(|p| p.id).collect();
    // `to` counts pages of the result: skip that many of those that stay.
    let staying = count - refs.len() as u64;
    let to = to.min(staying);
    let at = {
        let view = tx.view();
        let dm = DocModel::new();
        let m = dm.on(&view);
        let mut left = to;
        let mut i = 0;
        while i < count && left > 0 {
            if !moving.contains(&m.page(i).map_err(EditError)?.id) {
                left -= 1;
            }
            i += 1;
        }
        i
    };
    let s = spot(tx, at, &moving)?;
    for p in &refs {
        if p.ancestors.last() != Some(&s.parent) {
            materialize(tx, p)?;
        }
    }
    let mut touched = BTreeSet::new();
    detach(tx, &refs, &mut touched)?;
    let ids: Vec<ObjId> = refs.iter().map(|p| p.id).collect();
    attach(tx, &s, &ids, &mut touched)?;
    prune(tx, &refs, root)?;
    finish(tx, root, &touched)
}

pub(crate) fn rotate(tx: &mut Tx, pages: &[u64], by: i64) -> Result<()> {
    if by % 90 != 0 {
        return err("pages rotate by multiples of 90 degrees");
    }
    let view = tx.view();
    let dm = DocModel::new();
    let m = dm.on(&view);
    let count = m.page_count();
    let mut infos = Vec::new();
    for &i in pages {
        if i >= count {
            return err(format!("no page {} (the document has {count})", i + 1));
        }
        let info = m.page_info(i).map_err(EditError)?;
        infos.push((info.page.id, info.rotate));
    }
    for (id, rot) in infos {
        let mut d = dict_of(tx, id)?;
        d.insert(
            Name::from("Rotate"),
            Node::synthetic(Object::Integer((rot + by).rem_euclid(360))),
        );
        tx.put(id, Object::Dict(d));
    }
    Ok(())
}

fn rect(w: f64, h: f64) -> Object {
    let n = |v: f64| {
        Node::synthetic(if v.fract() == 0.0 && v.abs() < 1e15 {
            Object::Integer(v as i64)
        } else {
            Object::Real(v)
        })
    };
    Object::Array(vec![n(0.0), n(0.0), n(w), n(h)])
}

pub(crate) fn insert_blank(tx: &mut Tx, at: u64, width: f64, height: f64) -> Result<()> {
    if !(width > 0.0 && height > 0.0 && width <= 14400.0 && height <= 14400.0) {
        return err("page sizes are between 0 and 14400 points");
    }
    let s = spot(tx, at, &HashSet::new())?;
    let mut d = Dict::new();
    d.insert(
        Name::from("Type"),
        Node::synthetic(Object::Name(Name::from("Page"))),
    );
    d.insert(Name::from("MediaBox"), Node::synthetic(rect(width, height)));
    d.insert(
        Name::from("Resources"),
        Node::synthetic(Object::Dict(Dict::new())),
    );
    let id = tx.add(Object::Dict(d));
    let mut touched = BTreeSet::new();
    attach(tx, &s, &[id], &mut touched)?;
    finish(tx, s.ancestors.first().copied(), &touched)
}

/// Keys of a copied page that point into the source document's structures.
const PAGE_DROP: [&str; 4] = ["Parent", "StructParents", "B", "PieceInfo"];

/// Deep copy of pages from `src` into the transaction.
struct Copier<'s> {
    src: &'s PdfFile,
    /// Source page → new page.
    pages: HashMap<ObjId, ObjId>,
    /// Source object → new object.
    map: HashMap<ObjId, ObjId>,
    queue: VecDeque<(ObjId, ObjId)>,
    /// Copying within the document: share everything but pages and their
    /// annotations.
    share: bool,
}

fn kind(o: &Object) -> Option<&Name> {
    o.as_dict().and_then(|d| d.get_name("Type"))
}

impl Copier<'_> {
    fn value(&mut self, tx: &mut Tx, o: &Object, annots: bool) -> Object {
        match o {
            Object::Ref(r) => {
                if let Some(&n) = self.pages.get(r).or_else(|| self.map.get(r)) {
                    return Object::Ref(n);
                }
                let Ok(x) = self.src.get(*r) else {
                    return Object::Null;
                };
                // Pages not copied, and the page tree, stay behind.
                if kind(&x.node.obj).is_some_and(|t| t == "Page" || t == "Pages") {
                    return Object::Null;
                }
                if self.share && !annots && !is_annot(&x.node.obj) {
                    return Object::Ref(*r);
                }
                let n = tx.add(Object::Null);
                self.map.insert(*r, n);
                self.queue.push_back((*r, n));
                Object::Ref(n)
            }
            Object::Array(a) => Object::Array(
                a.iter()
                    .map(|n| Node::synthetic(self.value(tx, &n.obj, annots)))
                    .collect(),
            ),
            Object::Dict(d) => Object::Dict(self.dict(tx, d, false)),
            Object::Stream(s) => Object::Stream(Stream {
                dict: self.dict(tx, &s.dict, false),
                data: s.data.clone(),
            }),
            o => o.clone(),
        }
    }

    fn dict(&mut self, tx: &mut Tx, d: &Dict, annots: bool) -> Dict {
        d.iter()
            .map(|(k, v)| {
                let annots = annots && k == "Annots";
                (k.clone(), Node::synthetic(self.value(tx, &v.obj, annots)))
            })
            .collect()
    }

    /// Copy the queued objects (and what they refer to).
    fn drain(&mut self, tx: &mut Tx) -> Result<()> {
        while let Some((from, to)) = self.queue.pop_front() {
            let x = self.src.get(from).map_err(|e| EditError(e.to_string()))?;
            let obj = match &x.node.obj {
                Object::Stream(s) => {
                    // Decrypted, still encoded: stored in memory.
                    let mut data = Vec::new();
                    std::io::Read::read_to_end(&mut self.src.decrypted_reader(from, s), &mut data)
                        .map_err(|e| EditError(format!("object {from}: {e}")))?;
                    Object::Stream(Stream {
                        dict: self.dict(tx, &s.dict, false),
                        data: StreamData::Owned(data.into()),
                    })
                }
                o => self.value(tx, o, false),
            };
            tx.put(to, obj);
        }
        Ok(())
    }
}

fn is_annot(o: &Object) -> bool {
    o.as_dict().is_some_and(|d| {
        d.type_is("Annot") || (d.get_key("Rect").is_some() && d.get_key("Subtype").is_some())
    })
}

pub(crate) fn insert_copies(
    tx: &mut Tx,
    at: u64,
    from: Option<&PdfFile>,
    pages: &[u64],
) -> Result<()> {
    let own;
    let src = match from {
        Some(f) => f,
        None => {
            own = tx.view();
            &own
        }
    };
    let dm = DocModel::new();
    let m = dm.on(src);
    let count = m.page_count();
    let mut found = Vec::new();
    for &i in pages {
        if i >= count {
            return err(format!("no page {} in the source (it has {count})", i + 1));
        }
        found.push(m.page(i).map_err(EditError)?);
    }
    let s = spot(tx, at, &HashSet::new())?;
    let mut c = Copier {
        src,
        pages: HashMap::new(),
        map: HashMap::new(),
        queue: VecDeque::new(),
        share: from.is_none(),
    };
    // Allocate every page first so links between copied pages hold.
    let mut ids = Vec::new();
    for p in &found {
        let id = *c.pages.entry(p.id).or_insert_with(|| tx.add(Object::Null));
        ids.push((p, id));
    }
    let mut new_ids = Vec::new();
    for (p, id) in ids {
        if new_ids.contains(&id) {
            // The same page twice: a second copy.
            let id2 = tx.add(Object::Null);
            let d = dict_of(tx, id)?;
            tx.put(id2, Object::Dict(d));
            new_ids.push(id2);
            continue;
        }
        let o = src.get(p.id).map_err(|e| EditError(e.to_string()))?;
        let Some(d) = o.node.obj.as_dict() else {
            return err(format!("page {} is not a dictionary", p.index + 1));
        };
        let mut d = d.clone();
        for k in PAGE_DROP {
            d.shift_remove(k.as_bytes());
        }
        for key in INHERITED {
            if d.contains_key(key.as_bytes()) {
                continue;
            }
            for &a in p.ancestors.iter().rev() {
                let v = src.get(a).ok().and_then(|x| {
                    x.node
                        .obj
                        .as_dict()
                        .and_then(|n| n.get(key.as_bytes()))
                        .cloned()
                });
                if let Some(v) = v {
                    d.insert(Name::from(key), v);
                    break;
                }
            }
        }
        let copied = c.dict(tx, &d, true);
        tx.put(id, Object::Dict(copied));
        c.drain(tx)?;
        new_ids.push(id);
    }
    let mut touched = BTreeSet::new();
    attach(tx, &s, &new_ids, &mut touched)?;
    finish(tx, s.ancestors.first().copied(), &touched)
}
