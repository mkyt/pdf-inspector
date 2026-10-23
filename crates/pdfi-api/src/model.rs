//! Requests about the semantic layer (pdfi-model): document, pages,
//! outline, annotations and the inventory.

use std::sync::Arc;

use pdfi_model::inventory::{self, Inventory, TRAILER};
use pdfi_model::{Issue, dest, pages};
use pdfi_syntax::{DictExt, ObjId, Object};

use crate::convert::name_text;
use crate::dto::*;
use crate::protocol::{ApiError, ErrorCode};
use crate::session::{Doc, lock, page};

/// Resources listed in a [`PageDetail`].
const MAX_RESOURCES: usize = 500;
/// Pages listed in a [`FontInfo`] or [`ImageInfo`].
const MAX_USAGE: usize = 100;
/// Paths listed in a [`Referrer`].
const MAX_PATHS: usize = 8;

pub(crate) fn rect(r: pdfi_model::Rect) -> Rect {
    Rect {
        x0: r.x0,
        y0: r.y0,
        x1: r.x1,
        y1: r.y1,
    }
}

fn origin(o: pages::Origin) -> AttrOrigin {
    match o {
        pages::Origin::Page => AttrOrigin::Page,
        pages::Origin::Inherited(id) => AttrOrigin::Inherited { from: id.into() },
        pages::Origin::Default(of) => AttrOrigin::Default { of: of.into() },
        pages::Origin::Missing => AttrOrigin::Missing,
    }
}

pub fn issue(i: &Issue) -> DiagnosticInfo {
    DiagnosticInfo {
        severity: match i.severity {
            pdfi_syntax::Severity::Info => Severity::Info,
            pdfi_syntax::Severity::Warning => Severity::Warning,
            pdfi_syntax::Severity::Error => Severity::Error,
        },
        code: i.code.into(),
        message: i.message.clone(),
        obj: i.obj.map(Into::into),
        path: None,
        span: None,
    }
}

fn index32(i: u64) -> u32 {
    i.min(u32::MAX as u64) as u32
}

fn link(l: dest::Link) -> LinkTarget {
    match l {
        dest::Link::Dest(d) => LinkTarget::Dest {
            dest: Destination {
                named: d.named,
                page: d.page.map(Into::into),
                page_index: d.page_index.map(index32),
                remote_page: d.remote_page,
                fit: d.fit,
                params: d.params,
                error: d.error,
            },
        },
        dest::Link::Uri(uri) => LinkTarget::Uri { uri },
        dest::Link::Action { kind, detail } => LinkTarget::Action {
            action: kind,
            detail,
        },
    }
}

fn not_found(msg: impl Into<String>) -> ApiError {
    ApiError::new(ErrorCode::NotFound, msg)
}

pub fn document(d: &Doc) -> DocumentView {
    let m = d.model.on(&d.file);
    let o = m.overview();
    DocumentView {
        catalog: o.catalog.map(Into::into),
        info: o.info.map(Into::into),
        info_entries: o
            .info_entries
            .into_iter()
            .map(|e| InfoEntry {
                key: e.key,
                text: e.text,
                date: e.date.map(|d| d.iso()),
            })
            .collect(),
        metadata: o.metadata.map(Into::into),
        pages: index32(o.page_count),
        features: o
            .features
            .into_iter()
            .map(|f| Feature {
                key: f.key,
                text: f.text,
                obj: f.obj.map(Into::into),
            })
            .collect(),
        issues: o.issues.iter().map(issue).collect(),
    }
}

pub fn xmp(d: &Doc, range: Range) -> Result<XmpView, ApiError> {
    let m = d.model.on(&d.file);
    let x = m
        .metadata()
        .ok_or_else(|| not_found("the catalog has no /Metadata stream"))?;
    let nodes = &x.xmp.nodes;
    Ok(XmpView {
        id: x.id.into(),
        len: x.len,
        nodes: page(nodes.len() as u64, range, |o, n| {
            nodes[o..o + n]
                .iter()
                .map(|n| XmpNode {
                    depth: n.depth,
                    kind: match n.kind {
                        pdfi_model::xmp::NodeKind::Element => XmpNodeKind::Element,
                        pdfi_model::xmp::NodeKind::Attribute => XmpNodeKind::Attribute,
                    },
                    name: n.name.clone(),
                    namespace: n.namespace.clone(),
                    value: n.value.clone(),
                })
                .collect()
        }),
        truncated: x.xmp.truncated,
        properties: x
            .xmp
            .properties
            .iter()
            .map(|p| XmpProperty {
                name: p.name.clone(),
                namespace: p.namespace.clone(),
                value: p.value.clone(),
            })
            .collect(),
        error: x.xmp.error.clone().or_else(|| x.decode_error.clone()),
    })
}

pub fn pages(d: &Doc, range: Range) -> Page<PageSummary> {
    let m = d.model.on(&d.file);
    page(m.page_count(), range, |o, n| {
        (o..o + n)
            .map(|i| match m.page_summary(i as u64) {
                Ok((p, size, rotate, annots)) => PageSummary {
                    index: i as u32,
                    id: Some(p.id.into()),
                    label: m.label(i as u64),
                    size: size.map(rect),
                    rotate,
                    annots: annots as u32,
                    error: None,
                },
                Err(e) => PageSummary {
                    index: i as u32,
                    id: None,
                    label: m.label(i as u64),
                    size: None,
                    rotate: 0,
                    annots: 0,
                    error: Some(e),
                },
            })
            .collect()
    })
}

pub fn page_detail(d: &Doc, index: u32) -> Result<PageDetail, ApiError> {
    let f = &d.file;
    let m = d.model.on(f);
    let p = m.page_info(index as u64).map_err(not_found)?;
    let (items, truncated) = resource_items(f, p.resources.as_ref());
    Ok(PageDetail {
        index,
        id: p.page.id.into(),
        label: p.label.clone(),
        ancestors: p.page.ancestors.iter().map(|&a| a.into()).collect(),
        boxes: p
            .boxes
            .iter()
            .map(|b| PageBox {
                name: b.name.into(),
                rect: b.rect.map(rect),
                origin: origin(b.origin),
            })
            .collect(),
        rotate: p.rotate,
        rotate_origin: origin(p.rotate_origin),
        user_unit: p.user_unit,
        resources: p.resources_id.map(Into::into),
        resources_origin: origin(p.resources_origin),
        resource_items: items,
        resources_truncated: truncated,
        contents: p
            .contents
            .iter()
            .map(|c| ContentInfo {
                id: c.id.map(Into::into),
                raw_len: c.raw_len,
                filters: c.filters.clone(),
                error: c.error.clone(),
            })
            .collect(),
        annots: p.annots as u32,
        issues: p.issues.iter().map(issue).collect(),
    })
}

/// The named resources of a resource dictionary (the first
/// [`MAX_RESOURCES`]), and whether there are more.
pub(crate) fn resource_items(
    f: &pdfi_syntax::PdfFile,
    res: Option<&Object>,
) -> (Vec<ResourceItem>, bool) {
    let mut items = Vec::new();
    if let Some(res) = res.and_then(Object::as_dict) {
        for (cat, v) in res {
            let (_, r) = pdfi_model::resolve(f, &v.obj);
            let Some(entries) = r.as_dict() else { continue };
            for (name, v) in entries {
                if items.len() >= MAX_RESOURCES {
                    return (items, true);
                }
                items.push(ResourceItem {
                    category: name_text(cat),
                    name: name_text(name),
                    id: v.obj.as_ref_id().map(Into::into),
                    detail: inventory::describe(f, &v.obj),
                });
            }
        }
    }
    (items, false)
}

pub fn page_index(d: &Doc, id: ObjRef) -> Option<u32> {
    d.model.on(&d.file).page_index(id.into()).map(index32)
}

pub fn annotations(d: &Doc, page_index: u32, range: Range) -> Result<Page<AnnotInfo>, ApiError> {
    let m = d.model.on(&d.file);
    let limit = range.limit.min(crate::MAX_PAGE) as usize;
    let offset = range.offset.min(usize::MAX as u64) as usize;
    let (total, items) = m
        .annots(page_index as u64, offset, limit)
        .map_err(not_found)?;
    Ok(Page {
        total: total as u64,
        offset: (offset.min(total)) as u64,
        items: items.into_iter().map(annot_info).collect(),
    })
}

fn appearance_info(p: pdfi_model::annot::Appearance) -> AppearanceInfo {
    AppearanceInfo {
        kind: p.kind,
        id: p.id.map(Into::into),
        state: p.state,
        states: p.states,
        bbox: p.bbox.map(rect),
        matrix: p.matrix.to_vec(),
        to_page: p.to_page.map(|m| m.to_vec()),
        error: p.error,
    }
}

fn annot_info(a: pdfi_model::annot::Annot) -> AnnotInfo {
    let shown = a.shown();
    AnnotInfo {
        index: a.index as u32,
        id: a.id.map(Into::into),
        subtype: a.subtype,
        rect: a.rect.map(rect),
        contents: a.contents,
        name: a.name,
        flags: a.flags,
        flag_names: pdfi_model::annot::FLAG_NAMES
            .iter()
            .enumerate()
            .filter(|(i, _)| a.flags & (1 << i) != 0)
            .map(|(_, n)| (*n).into())
            .collect(),
        shown,
        author: a.author,
        subject: a.subject,
        modified: a.modified,
        color: a.color,
        interior: a.interior,
        opacity: a.opacity,
        border: a.border,
        quads: a.quads.iter().map(|q| q.to_vec()).collect(),
        paths: a
            .paths
            .iter()
            .map(|p| p.iter().flat_map(|&(x, y)| [x, y]).collect())
            .collect(),
        appearances: a.appearances,
        appearance: a.appearance.map(appearance_info),
        variants: a.variants.into_iter().map(appearance_info).collect(),
        as_state: a.r#as,
        link: a.link.map(link),
        parent: a.parent.map(Into::into),
        popup: a.popup.map(Into::into),
        error: a.error,
    }
}

pub fn outline(d: &Doc, parent: Option<ObjRef>, range: Range) -> Page<OutlineItem> {
    let m = d.model.on(&d.file);
    let ids = m.outline_children(parent.map(Into::into));
    page(ids.len() as u64, range, |o, n| {
        ids[o..o + n]
            .iter()
            .map(|&id| match m.outline_item(id) {
                Ok(i) => OutlineItem {
                    id: id.into(),
                    title: i.title,
                    count: i.count,
                    has_children: i.has_children,
                    link: i.link.map(link),
                    flags: i.flags,
                    color: i.color.map(|c| c.to_vec()),
                },
                Err(e) => OutlineItem {
                    id: id.into(),
                    title: String::new(),
                    count: None,
                    has_children: false,
                    link: Some(LinkTarget::Action {
                        action: "error".into(),
                        detail: Some(e),
                    }),
                    flags: 0,
                    color: None,
                },
            })
            .collect()
    })
}

fn inventory(d: &Doc) -> Result<Arc<Inventory>, ApiError> {
    lock(&d.inventory).clone().ok_or_else(|| {
        ApiError::new(
            ErrorCode::NotReady,
            "the inventory has not been built; run the inventory job",
        )
    })
}

fn usage(pages: &[u32]) -> (u32, Vec<u32>) {
    (
        pages.len() as u32,
        pages.iter().take(MAX_USAGE).copied().collect(),
    )
}

pub fn fonts(d: &Doc, range: Range) -> Result<Page<FontInfo>, ApiError> {
    let inv = inventory(d)?;
    Ok(page(inv.fonts.len() as u64, range, |o, n| {
        inv.fonts[o..o + n]
            .iter()
            .map(|f| {
                let (page_count, pages) = usage(&f.pages);
                FontInfo {
                    id: f.id.into(),
                    base_font: f.base_font.clone(),
                    subtype: f.subtype.clone(),
                    encoding: f.encoding.clone(),
                    embedded: f.embedded.clone(),
                    font_file: f.font_file.map(Into::into),
                    to_unicode: f.to_unicode,
                    descendant: f.descendant.map(Into::into),
                    subset: f.subset,
                    standard: f.standard,
                    page_count,
                    pages,
                }
            })
            .collect()
    }))
}

pub fn images(d: &Doc, range: Range) -> Result<Page<ImageInfo>, ApiError> {
    let inv = inventory(d)?;
    Ok(page(inv.images.len() as u64, range, |o, n| {
        inv.images[o..o + n]
            .iter()
            .map(|i| {
                let (page_count, pages) = usage(&i.pages);
                ImageInfo {
                    id: i.id.into(),
                    width: i.width,
                    height: i.height,
                    bits: i.bits,
                    color_space: i.color_space.clone(),
                    filters: i.filters.clone(),
                    raw_len: i.raw_len,
                    image_mask: i.image_mask,
                    smask: i.smask.map(Into::into),
                    mask: i.mask,
                    interpolate: i.interpolate,
                    page_count,
                    pages,
                }
            })
            .collect()
    }))
}

pub fn referrers(d: &Doc, id: ObjRef, range: Range) -> Result<Page<Referrer>, ApiError> {
    let inv = inventory(d)?;
    let target: ObjId = id.into();
    let srcs = inv.refs.referrers(id.num);
    let ck = d.schema.checker(&d.file);
    Ok(page(srcs.len() as u64, range, |o, n| {
        srcs[o..o + n]
            .iter()
            .map(|&s| {
                if s == TRAILER {
                    let t = Object::Dict(d.file.trailer().clone());
                    return Referrer {
                        from: NodeRoot::Trailer,
                        type_name: Some(ck.trailer_type().name()),
                        paths: inventory::ref_paths(&t, target, MAX_PATHS),
                    };
                }
                let generation = d.file.xref().get(s).map_or(0, |e| e.generation());
                let sid = ObjId::new(s, generation);
                let (type_name, paths) = match d.file.get(sid) {
                    Ok(o) => (
                        ck.type_of(o.id).map(|(t, _)| t.name()).or_else(|| {
                            o.node
                                .obj
                                .as_dict()
                                .and_then(|d| d.get_name("Type"))
                                .map(|n| format!("/{}", name_text(n)))
                        }),
                        inventory::ref_paths(&o.node.obj, target, MAX_PATHS),
                    ),
                    Err(_) => (None, Vec::new()),
                };
                Referrer {
                    from: NodeRoot::Object { id: sid.into() },
                    type_name,
                    paths,
                }
            })
            .collect()
    }))
}

pub fn unreferenced(d: &Doc, range: Range) -> Result<Page<ObjRef>, ApiError> {
    let inv = inventory(d)?;
    let ids = &inv.unreferenced;
    Ok(page(ids.len() as u64, range, |o, n| {
        ids[o..o + n].iter().map(|&i| i.into()).collect()
    }))
}

pub fn object_classes(d: &Doc) -> Result<Vec<ObjectClass>, ApiError> {
    let inv = inventory(d)?;
    Ok(inv
        .classes
        .iter()
        .map(|c| ObjectClass {
            kind: c.kind.into(),
            type_name: c.type_name.clone(),
            subtype: c.subtype.clone(),
            schema_type: c.schema_type.clone(),
            count: c.ids.len() as u64,
        })
        .collect())
}

/// Object numbers `f` picks, ascending.
pub fn filter_objects(d: &Doc, f: &ObjectFilter) -> Result<Vec<u32>, ApiError> {
    let inv = inventory(d)?;
    let mut nums: Vec<u32> = match &f.classes {
        Some(want) => inv
            .classes
            .iter()
            .filter(|c| {
                want.iter().any(|w| {
                    w.kind == c.kind
                        && w.type_name == c.type_name
                        && w.subtype == c.subtype
                        && w.schema_type == c.schema_type
                })
            })
            .flat_map(|c| c.ids.iter().map(|i| i.num))
            .collect(),
        None if f.unreferenced => inv.unreferenced.iter().map(|i| i.num).collect(),
        None => d.file.xref().live_ids().map(|i| i.num).collect(),
    };
    nums.sort_unstable();
    nums.dedup();
    if f.unreferenced && f.classes.is_some() {
        let orphans: std::collections::HashSet<u32> =
            inv.unreferenced.iter().map(|i| i.num).collect();
        nums.retain(|n| orphans.contains(n));
    }
    Ok(nums)
}

/// Objects loaded while looking for a [`RefPath`].
const MAX_PATH_LOADS: usize = 20_000;
/// Extra cost of following a back link: a page is reached through `/Kids`
/// rather than from one of its annotations' `/P`.
const BACK_LINK_COST: u32 = 8;

/// Entries that point back up or sideways, or jump to a destination.
fn is_back_link(key: &str) -> bool {
    let last = key.trim_end_matches(|c: char| c == ']' || c == '[' || c.is_ascii_digit());
    let name = last.rsplit('/').next().unwrap_or("");
    matches!(
        name,
        "Parent" | "P" | "Prev" | "Next" | "Last" | "Dest" | "D" | "OpenAction" | "A" | "SE"
    )
}

pub fn ref_path(d: &Doc, id: ObjRef) -> Result<RefPath, ApiError> {
    use std::cmp::Reverse;
    use std::collections::{BinaryHeap, HashMap};

    let inv = inventory(d)?;
    let target: ObjId = id.into();
    let trailer = Object::Dict(d.file.trailer().clone());
    // Walk the reverse graph from the object (Dijkstra): for each node, the
    // cost to reach the object and the next node towards it, with the key.
    let mut best: HashMap<u32, (u32, u32, String)> = HashMap::new();
    let mut heap = BinaryHeap::from([Reverse((0u32, id.num))]);
    let mut done = std::collections::HashSet::new();
    let mut loads = 0;
    let mut found = false;
    while let Some(Reverse((cost, num))) = heap.pop() {
        if loads > MAX_PATH_LOADS {
            break;
        }
        if num == TRAILER {
            found = true;
            break;
        }
        if !done.insert(num) {
            continue;
        }
        let to = ObjId::new(num, d.file.xref().get(num).map_or(0, |e| e.generation()));
        for &src in inv.refs.referrers(num) {
            if done.contains(&src) {
                continue;
            }
            loads += 1;
            if loads > MAX_PATH_LOADS {
                break;
            }
            let key = if src == TRAILER {
                inventory::ref_paths(&trailer, to, 1)
            } else {
                let sid = ObjId::new(src, d.file.xref().get(src).map_or(0, |e| e.generation()));
                d.file
                    .get(sid)
                    .map(|o| inventory::ref_paths(&o.node.obj, to, 1))
                    .unwrap_or_default()
            };
            let Some(key) = key.into_iter().next() else {
                continue;
            };
            let c = cost
                + if is_back_link(&key) {
                    BACK_LINK_COST
                } else {
                    1
                };
            if best.get(&src).is_none_or(|b| c < b.0) {
                best.insert(src, (c, num, key));
                heap.push(Reverse((c, src)));
            }
        }
    }
    if !found {
        return Ok(RefPath {
            reachable: false,
            steps: Vec::new(),
        });
    }
    let ck = d.schema.checker(&d.file);
    let mut steps = Vec::new();
    let mut at = TRAILER;
    while at != target.num {
        let Some((_, next, key)) = best.get(&at) else {
            break;
        };
        let step = if at == TRAILER {
            PathStep {
                from: NodeRoot::Trailer,
                type_name: Some(ck.trailer_type().name()),
                key: key.clone(),
            }
        } else {
            let sid = ObjId::new(at, d.file.xref().get(at).map_or(0, |e| e.generation()));
            PathStep {
                from: NodeRoot::Object { id: sid.into() },
                type_name: ck.type_of(sid).map(|(t, _)| t.name()).or_else(|| {
                    d.file.get(sid).ok().and_then(|o| {
                        o.node
                            .obj
                            .as_dict()
                            .and_then(|d| d.get_name("Type"))
                            .map(|n| format!("/{}", name_text(n)))
                    })
                }),
                key: key.clone(),
            }
        };
        steps.push(step);
        at = *next;
    }
    Ok(RefPath {
        reachable: true,
        steps,
    })
}
