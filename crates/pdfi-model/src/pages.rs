//! The page tree: pages by index in O(depth) through `/Count` (never by
//! listing every page), inherited attributes and where they come from,
//! boxes, contents, resources and page labels.

use std::ops::ControlFlow;
use std::sync::Arc;

use pdfi_syntax::{Dict, DictExt, ObjId, Object};

use crate::util::{Rect, get, int, resolve};
use crate::{Issue, Model, lock, tree};

/// Depth of page trees we follow.
const MAX_DEPTH: usize = 64;
/// Page label ranges read from `/PageLabels`.
const MAX_LABELS: usize = 100_000;

/// The kids of one page tree node with the pages they hold.
pub(crate) struct NodeIndex {
    kids: Vec<Kid>,
    /// `ends[i]`: pages in kids `0..=i`.
    ends: Vec<u64>,
}

impl NodeIndex {
    fn total(&self) -> u64 {
        self.ends.last().copied().unwrap_or(0)
    }
}

#[derive(Clone, Copy)]
struct Kid {
    id: ObjId,
    leaf: bool,
}

/// A page found by index.
#[derive(Clone, Debug, PartialEq)]
pub struct PageRef {
    pub index: u64,
    pub id: ObjId,
    /// Page tree nodes from the root down to the page's parent.
    pub ancestors: Vec<ObjId>,
}

/// Where a page attribute comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Origin {
    Page,
    Inherited(ObjId),
    /// Not given: the default, or the value of the named box.
    Default(&'static str),
    /// Required but given nowhere.
    Missing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageBox {
    pub name: &'static str,
    pub rect: Option<Rect>,
    pub origin: Origin,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Content {
    pub id: Option<ObjId>,
    pub raw_len: Option<u64>,
    pub filters: Vec<String>,
    pub error: Option<String>,
}

/// One page with its inherited attributes resolved.
#[derive(Clone, Debug)]
pub struct PageInfo {
    pub page: PageRef,
    pub label: Option<String>,
    /// MediaBox, CropBox, BleedBox, TrimBox, ArtBox.
    pub boxes: Vec<PageBox>,
    /// Normalized to 0, 90, 180 or 270.
    pub rotate: i64,
    pub rotate_origin: Origin,
    pub user_unit: f64,
    /// The resource dictionary (resolved) and the object holding it.
    pub resources: Option<Object>,
    pub resources_id: Option<ObjId>,
    pub resources_origin: Origin,
    pub contents: Vec<Content>,
    pub annots: usize,
    pub issues: Vec<Issue>,
}

impl PageInfo {
    /// The visible area: the crop box clipped to the media box.
    pub fn visible(&self) -> Option<Rect> {
        let media = self.boxes[0].rect?;
        Some(self.boxes[1].rect.map_or(media, |c| c.intersect(&media)))
    }
}

/// A `/PageLabels` range.
#[derive(Clone, Debug)]
pub(crate) struct LabelRange {
    start: u64,
    style: Option<u8>,
    prefix: String,
    first: i64,
}

fn is_leaf(d: &Dict) -> bool {
    if d.type_is("Page") {
        return true;
    }
    if d.type_is("Pages") {
        return false;
    }
    !matches!(d.get_key("Kids"), Some(Object::Array(_) | Object::Ref(_)))
}

impl<'a> Model<'a> {
    /// The root of the page tree (`/Root /Pages`).
    pub fn page_root(&self) -> Option<ObjId> {
        let (_, cat) = self.catalog()?;
        cat.as_dict()?.get_key("Pages")?.as_ref_id()
    }

    pub fn page_count(&self) -> u64 {
        self.page_root()
            .map_or(0, |r| self.node(r, &mut Vec::new()).total())
    }

    /// Kids of a page tree node, counting pages by `/Count` (or, where it
    /// is missing, by looking inside). Built once per node.
    fn node(&self, id: ObjId, stack: &mut Vec<ObjId>) -> Arc<NodeIndex> {
        if let Some(n) = lock(&self.m.nodes).get(&id) {
            return n.clone();
        }
        let n = Arc::new(self.build_node(id, stack));
        lock(&self.m.nodes).insert(id, n.clone());
        n
    }

    fn build_node(&self, id: ObjId, stack: &mut Vec<ObjId>) -> NodeIndex {
        let f = self.file;
        let mut n = NodeIndex {
            kids: Vec::new(),
            ends: Vec::new(),
        };
        let Ok(o) = f.get(id) else {
            self.report(Issue::error(
                "PageTree",
                format!("page tree node {id} is missing"),
                Some(id),
            ));
            return n;
        };
        let Some(d) = o.node.obj.as_dict() else {
            self.report(Issue::error(
                "PageTree",
                format!("page tree node {id} is not a dictionary"),
                Some(id),
            ));
            return n;
        };
        let kids = get(f, d, "Kids");
        let mut total = 0;
        for kid in kids
            .as_deref()
            .and_then(Object::as_array)
            .unwrap_or_default()
        {
            let Some(kid_id) = kid.obj.as_ref_id() else {
                self.report(Issue::error(
                    "PageTree",
                    format!("/Kids of {id} holds a direct object; kids must be indirect"),
                    Some(id),
                ));
                continue;
            };
            if kid_id == id || stack.contains(&kid_id) {
                self.report(Issue::error(
                    "PageTree",
                    format!("/Kids of {id} refers back to {kid_id}"),
                    Some(id),
                ));
                continue;
            }
            let Some(k) = f.get(kid_id).ok() else {
                self.report(Issue::error(
                    "PageTree",
                    format!("/Kids of {id}: {kid_id} is missing"),
                    Some(id),
                ));
                continue;
            };
            let Some(kd) = k.node.obj.as_dict() else {
                self.report(Issue::error(
                    "PageTree",
                    format!("/Kids of {id}: {kid_id} is not a dictionary"),
                    Some(id),
                ));
                continue;
            };
            let leaf = is_leaf(kd);
            let count = if leaf {
                1
            } else {
                match int(f, kd, "Count") {
                    Some(c) if c >= 0 => c as u64,
                    _ => {
                        self.report(Issue::warning(
                            "PageTree",
                            format!("page tree node {kid_id} has no valid /Count"),
                            Some(kid_id),
                        ));
                        stack.push(id);
                        let c = self.node(kid_id, stack).total();
                        stack.pop();
                        c
                    }
                }
            };
            total += count;
            n.kids.push(Kid { id: kid_id, leaf });
            n.ends.push(total);
        }
        if let Some(c) = int(f, d, "Count").filter(|&c| c as u64 != total) {
            self.report(Issue::warning(
                "PageCount",
                format!("/Count of {id} is {c}, but its kids hold {total} pages"),
                Some(id),
            ));
        }
        n
    }

    /// The page at `index` (0-based), descending by the kids' counts.
    pub fn page(&self, index: u64) -> Result<PageRef, String> {
        let root = self.page_root().ok_or("the catalog has no page tree")?;
        let mut cur = root;
        let mut i = index;
        let mut ancestors = Vec::new();
        for _ in 0..MAX_DEPTH {
            ancestors.push(cur);
            let n = self.node(cur, &mut Vec::new());
            let k = n.ends.partition_point(|&e| e <= i);
            let Some(kid) = n.kids.get(k).copied() else {
                return Err(if cur == root {
                    format!("no page {index}; the document has {}", n.total())
                } else {
                    format!(
                        "page {index} should be under {cur}, which has fewer pages than its /Count says"
                    )
                });
            };
            i -= if k == 0 { 0 } else { n.ends[k - 1] };
            if kid.leaf {
                return Ok(PageRef {
                    index,
                    id: kid.id,
                    ancestors,
                });
            }
            if ancestors.contains(&kid.id) {
                return Err(format!("the page tree has a cycle at {}", kid.id));
            }
            cur = kid.id;
        }
        Err("the page tree is too deep".into())
    }

    /// Index of a page object, found by walking up its `/Parent` chain.
    pub fn page_index(&self, id: ObjId) -> Option<u64> {
        let root = self.page_root()?;
        let mut cur = id;
        let mut index = 0;
        for _ in 0..MAX_DEPTH {
            let o = self.file.get(cur).ok()?;
            let parent = o.node.obj.as_dict()?.get_key("Parent")?.as_ref_id()?;
            let n = self.node(parent, &mut Vec::new());
            let k = n.kids.iter().position(|k| k.id == cur)?;
            index += if k == 0 { 0 } else { n.ends[k - 1] };
            if parent == root {
                // A wrong /Parent would give a wrong index: check it.
                return (self.page(index).ok()?.id == id).then_some(index);
            }
            cur = parent;
        }
        None
    }

    /// Value of an inheritable attribute: on the page or the nearest
    /// ancestor that has it.
    fn inherited(
        &self,
        p: &PageRef,
        page: &Dict,
        key: &str,
    ) -> Option<(Object, Option<ObjId>, Origin)> {
        if let Some(v) = page.get_key(key) {
            let (id, r) = resolve(self.file, v);
            return Some(((*r).clone(), id, Origin::Page));
        }
        for &a in p.ancestors.iter().rev() {
            let Ok(o) = self.file.get(a) else { continue };
            if let Some(v) = o.node.obj.as_dict().and_then(|d| d.get_key(key)) {
                let (id, r) = resolve(self.file, v);
                return Some(((*r).clone(), id, Origin::Inherited(a)));
            }
        }
        None
    }

    /// The resource dictionary of a page (inherited if need be), resolved.
    pub fn resources_of(&self, p: &PageRef) -> Option<Object> {
        let o = self.file.get(p.id).ok()?;
        let d = o.node.obj.as_dict()?;
        self.inherited(p, d, "Resources").map(|(v, _, _)| v)
    }

    /// The page at `index` with its attributes resolved.
    pub fn page_info(&self, index: u64) -> Result<PageInfo, String> {
        let p = self.page(index)?;
        let f = self.file;
        let held = f.get(p.id).map_err(|e| e.to_string())?;
        let d = held
            .node
            .obj
            .as_dict()
            .ok_or_else(|| format!("page {} is not a dictionary", p.id))?;
        let mut issues = Vec::new();
        let rect_of = |key: &'static str, v: &Object, issues: &mut Vec<Issue>| {
            let r = Rect::read(f, v);
            if r.is_none() {
                issues.push(Issue::error(
                    "PageBox",
                    format!("/{key} is not a rectangle"),
                    Some(p.id),
                ));
            }
            r
        };

        let mut boxes = Vec::new();
        let media = match self.inherited(&p, d, "MediaBox") {
            Some((v, _, origin)) => PageBox {
                name: "MediaBox",
                rect: rect_of("MediaBox", &v, &mut issues),
                origin,
            },
            None => {
                issues.push(Issue::error(
                    "PageBox",
                    "no /MediaBox on the page or its ancestors",
                    Some(p.id),
                ));
                PageBox {
                    name: "MediaBox",
                    rect: None,
                    origin: Origin::Missing,
                }
            }
        };
        let crop = match self.inherited(&p, d, "CropBox") {
            Some((v, _, origin)) => PageBox {
                name: "CropBox",
                rect: rect_of("CropBox", &v, &mut issues),
                origin,
            },
            None => PageBox {
                name: "CropBox",
                rect: media.rect,
                origin: Origin::Default("MediaBox"),
            },
        };
        let crop_rect = crop.rect;
        boxes.push(media);
        boxes.push(crop);
        for name in ["BleedBox", "TrimBox", "ArtBox"] {
            boxes.push(match d.get_key(name) {
                Some(v) => PageBox {
                    name,
                    rect: rect_of(name, v, &mut issues),
                    origin: Origin::Page,
                },
                None => PageBox {
                    name,
                    rect: crop_rect,
                    origin: Origin::Default("CropBox"),
                },
            });
        }

        let (rotate, rotate_origin) = match self.inherited(&p, d, "Rotate") {
            Some((v, _, origin)) => match v.as_i64() {
                Some(r) => {
                    if r % 90 != 0 {
                        issues.push(Issue::error(
                            "Rotate",
                            format!("/Rotate {r} is not a multiple of 90"),
                            Some(p.id),
                        ));
                    }
                    (r.rem_euclid(360) / 90 * 90, origin)
                }
                None => {
                    issues.push(Issue::error(
                        "Rotate",
                        "/Rotate is not an integer",
                        Some(p.id),
                    ));
                    (0, origin)
                }
            },
            None => (0, Origin::Default("0")),
        };

        let (resources, resources_id, resources_origin) = match self.inherited(&p, d, "Resources") {
            Some((v, id, origin)) => {
                if v.as_dict().is_none() {
                    issues.push(Issue::error(
                        "Resources",
                        "/Resources is not a dictionary",
                        Some(p.id),
                    ));
                }
                (Some(v), id, origin)
            }
            None => {
                issues.push(Issue::error(
                    "Resources",
                    "no /Resources on the page or its ancestors",
                    Some(p.id),
                ));
                (None, None, Origin::Missing)
            }
        };

        let user_unit = get(f, d, "UserUnit")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0);
        let annots = get(f, d, "Annots")
            .and_then(|a| a.as_array().map(<[_]>::len))
            .unwrap_or(0);

        Ok(PageInfo {
            label: self.label(index),
            page: p,
            boxes,
            rotate,
            rotate_origin,
            user_unit,
            resources,
            resources_id,
            resources_origin,
            contents: self.contents(d),
            annots,
            issues,
        })
    }

    /// The streams of `/Contents`, in order.
    fn contents(&self, page: &Dict) -> Vec<Content> {
        let f = self.file;
        let Some(c) = page.get_key("Contents") else {
            return Vec::new();
        };
        let one = |o: &Object| {
            let (id, r) = resolve(f, o);
            match r.as_stream() {
                Some(s) => Content {
                    id,
                    raw_len: Some(match &s.data {
                        pdfi_syntax::StreamData::Source(sp) => sp.len(),
                        pdfi_syntax::StreamData::Owned(d) => d.len() as u64,
                    }),
                    filters: f
                        .filter_specs(&s.dict)
                        .iter()
                        .map(|x| x.name.as_str_lossy().into_owned())
                        .collect(),
                    error: None,
                },
                None => Content {
                    id,
                    raw_len: None,
                    filters: Vec::new(),
                    error: Some(format!("not a stream ({})", r.type_name())),
                },
            }
        };
        let (_, r) = resolve(f, c);
        match &*r {
            Object::Array(a) => a.iter().map(|n| one(&n.obj)).collect(),
            _ => vec![one(c)],
        }
    }

    fn labels(&self) -> &'a [LabelRange] {
        self.m.labels.get_or_init(|| {
            let mut out = Vec::new();
            let Some(root) = self
                .catalog()
                .and_then(|(_, c)| c.as_dict()?.get_key("PageLabels").cloned())
            else {
                return out;
            };
            let f = self.file;
            tree::walk(f, &root, tree::Kind::Number, &mut |k, v| {
                let (Some(start), r) = (k.as_i64(), resolve(f, v).1) else {
                    return ControlFlow::Continue(());
                };
                if let (true, Some(d)) = (start >= 0, r.as_dict()) {
                    out.push(LabelRange {
                        start: start as u64,
                        style: crate::util::name(f, d, "S").and_then(|n| n.0.first().copied()),
                        prefix: crate::util::text(f, d, "P").unwrap_or_default(),
                        first: get(f, d, "St").and_then(|v| v.as_i64()).unwrap_or(1),
                    });
                }
                if out.len() >= MAX_LABELS {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            });
            out.sort_by_key(|r| r.start);
            out
        })
    }

    /// The page label (`/PageLabels`) of page `index`, if the document has
    /// labels.
    pub fn label(&self, index: u64) -> Option<String> {
        let labels = self.labels();
        let k = labels.partition_point(|r| r.start <= index);
        let r = labels.get(k.checked_sub(1)?)?;
        let n = r.first.saturating_add((index - r.start) as i64);
        let num = match r.style {
            Some(b'D') => n.to_string(),
            Some(b'R') => roman(n),
            Some(b'r') => roman(n).to_lowercase(),
            Some(b'A') => letters(n),
            Some(b'a') => letters(n).to_lowercase(),
            _ => String::new(),
        };
        Some(format!("{}{num}", r.prefix))
    }

    /// Summary used by page lists: the visible size, rotation and number of
    /// annotations.
    pub fn page_summary(&self, index: u64) -> Result<(PageRef, Option<Rect>, i64, usize), String> {
        let p = self.page_info(index)?;
        let visible = p.visible();
        Ok((p.page, visible, p.rotate, p.annots))
    }
}

fn roman(n: i64) -> String {
    if !(1..=100_000).contains(&n) {
        return n.to_string();
    }
    const T: [(i64, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut n = n;
    let mut s = String::new();
    for (v, t) in T {
        while n >= v {
            s.push_str(t);
            n -= v;
        }
    }
    s
}

/// A, B, …, Z, AA, BB, …, ZZ, AAA, …
fn letters(n: i64) -> String {
    if !(1..=10_000).contains(&n) {
        return n.to_string();
    }
    let c = (b'A' + ((n - 1) % 26) as u8) as char;
    std::iter::repeat_n(c, ((n - 1) / 26 + 1) as usize).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_styles() {
        assert_eq!(roman(1994), "MCMXCIV");
        assert_eq!(letters(1), "A");
        assert_eq!(letters(26), "Z");
        assert_eq!(letters(28), "BB");
    }
}
