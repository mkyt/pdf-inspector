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

fn rect(r: pdfi_model::Rect) -> Rect {
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
    let mut items = Vec::new();
    let mut truncated = false;
    if let Some(res) = p.resources.as_ref().and_then(Object::as_dict) {
        'outer: for (cat, v) in res {
            let (_, r) = pdfi_model::resolve(f, &v.obj);
            let Some(entries) = r.as_dict() else { continue };
            for (name, v) in entries {
                if items.len() >= MAX_RESOURCES {
                    truncated = true;
                    break 'outer;
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
        items: items
            .into_iter()
            .map(|a| AnnotInfo {
                index: a.index as u32,
                id: a.id.map(Into::into),
                subtype: a.subtype,
                rect: a.rect.map(rect),
                contents: a.contents,
                name: a.name,
                flags: a.flags,
                appearance: a.appearance,
                link: a.link.map(link),
                parent: a.parent.map(Into::into),
                error: a.error,
            })
            .collect(),
    })
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
