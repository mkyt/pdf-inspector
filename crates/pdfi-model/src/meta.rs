//! Document-level information: the Info dictionary, the XMP metadata (and
//! whether the two agree) and what the catalog declares.

use std::sync::Arc;

use pdfi_syntax::{DictExt, ObjId, Object};

use crate::dest::Link;
use crate::text::{self, Date};
use crate::util::{get, name_str, resolve};
use crate::xmp::{self, Xmp};
use crate::{Issue, Model, tree};

/// Decoded metadata streams larger than this are not parsed.
const MAX_XMP: usize = 16 << 20;
/// Nodes of the XMP tree kept.
pub const MAX_XMP_NODES: usize = 20_000;
/// Entries counted in a name tree.
const MAX_COUNT: usize = 1_000_000;

#[derive(Clone, Debug, PartialEq)]
pub struct InfoEntry {
    pub key: String,
    /// The value as text (strings decoded, names with `/`).
    pub text: String,
    pub date: Option<Date>,
}

/// Something the catalog declares, summarized.
#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub key: String,
    pub text: String,
    pub obj: Option<ObjId>,
}

#[derive(Clone, Debug)]
pub struct Overview {
    pub catalog: Option<ObjId>,
    pub info: Option<ObjId>,
    pub info_entries: Vec<InfoEntry>,
    /// The catalog's `/Metadata` stream.
    pub metadata: Option<ObjId>,
    pub page_count: u64,
    pub features: Vec<Feature>,
    pub issues: Vec<Issue>,
}

/// The parsed `/Metadata` of the catalog.
#[derive(Clone, Debug)]
pub struct Metadata {
    pub id: ObjId,
    pub len: u64,
    pub xmp: Xmp,
    pub decode_error: Option<String>,
}

impl Link {
    pub fn describe(&self) -> String {
        match self {
            Link::Dest(d) => {
                let page = match (d.page_index, d.remote_page) {
                    (Some(i), _) => format!("page {}", i + 1),
                    (None, Some(p)) => format!("page {} of another file", p + 1),
                    _ => d.error.clone().unwrap_or_else(|| "?".into()),
                };
                match &d.named {
                    Some(n) => format!("“{n}” → {page}"),
                    None => page,
                }
            }
            Link::Uri(u) => u.clone(),
            Link::Action { kind, detail } => match detail {
                Some(d) => format!("{kind}: {d}"),
                None => kind.clone(),
            },
        }
    }
}

/// Info keys and the XMP properties that should carry the same value.
const INFO_XMP: [(&str, &str, &str); 8] = [
    ("Title", xmp::DC, "title"),
    ("Author", xmp::DC, "creator"),
    ("Subject", xmp::DC, "description"),
    ("Keywords", xmp::PDF, "Keywords"),
    ("Creator", xmp::XMP, "CreatorTool"),
    ("Producer", xmp::PDF, "Producer"),
    ("CreationDate", xmp::XMP, "CreateDate"),
    ("ModDate", xmp::XMP, "ModifyDate"),
];

impl Model<'_> {
    pub fn info_entries(&self) -> (Option<ObjId>, Vec<InfoEntry>, Vec<Issue>) {
        let f = self.file;
        let mut issues = Vec::new();
        let Some(info) = f.trailer().get_key("Info") else {
            return (None, Vec::new(), issues);
        };
        let (id, r) = resolve(f, info);
        let Some(d) = r.as_dict() else {
            issues.push(Issue::error("Info", "/Info is not a dictionary", id));
            return (id, Vec::new(), issues);
        };
        let entries = d
            .iter()
            .map(|(k, v)| {
                let key = name_str(k);
                let v = resolve(f, &v.obj).1;
                let (text, date) = match &*v {
                    Object::String(s) => {
                        let t = text::decode(&s.bytes);
                        let date = text::pdf_date(&s.bytes);
                        if date.is_none() && (key == "CreationDate" || key == "ModDate") {
                            issues.push(Issue::warning(
                                "Date",
                                format!("/Info /{key} ({t}) is not a date"),
                                id,
                            ));
                        }
                        (t, date)
                    }
                    Object::Name(n) => (format!("/{}", name_str(n)), None),
                    o => (pdfi_syntax::write::pretty(o), None),
                };
                InfoEntry { key, text, date }
            })
            .collect();
        (id, entries, issues)
    }

    /// The catalog's XMP metadata, parsed once.
    pub fn metadata(&self) -> Option<Arc<Metadata>> {
        self.m
            .metadata
            .get_or_init(|| {
                let (_, cat) = self.catalog()?;
                let r = cat.as_dict()?.get_key("Metadata")?;
                let (id, m) = resolve(self.file, r);
                let (id, s) = (id?, m.as_stream()?);
                let d = self.file.decode_stream(id, s, Some(MAX_XMP + 1));
                let too_big = d.data.len() > MAX_XMP;
                let mut decode_error = d.errors.first().cloned();
                if let Some(stop) = &d.stopped_at {
                    decode_error = Some(format!("cannot decode /{}", stop.as_str_lossy()));
                }
                let xmp = if too_big {
                    Xmp {
                        error: Some(format!("larger than {MAX_XMP} bytes; not parsed")),
                        ..Default::default()
                    }
                } else {
                    xmp::parse(&d.data, MAX_XMP_NODES)
                };
                Some(Arc::new(Metadata {
                    id,
                    len: d.data.len() as u64,
                    xmp,
                    decode_error,
                }))
            })
            .clone()
    }

    pub fn overview(&self) -> Overview {
        let (info, info_entries, mut issues) = self.info_entries();
        let catalog = self.catalog();
        let mut out = Overview {
            catalog: catalog.as_ref().and_then(|c| c.0),
            info,
            metadata: None,
            page_count: self.page_count(),
            features: Vec::new(),
            issues: Vec::new(),
            info_entries,
        };
        if let Some(m) = self.metadata() {
            out.metadata = Some(m.id);
            if let Some(e) = m.xmp.error.as_ref().or(m.decode_error.as_ref()) {
                issues.push(Issue::warning(
                    "Xmp",
                    format!("metadata {}: {e}", m.id),
                    Some(m.id),
                ));
            }
            issues.extend(self.compare_info_xmp(&out.info_entries, &m));
        }
        if let Some(d) = catalog.as_ref().and_then(|(_, c)| c.as_dict()) {
            out.features = self.features(d);
        }
        issues.extend(self.issues());
        out.issues = issues;
        out
    }

    fn compare_info_xmp(&self, info: &[InfoEntry], m: &Metadata) -> Vec<Issue> {
        let mut out = Vec::new();
        for (key, ns, local) in INFO_XMP {
            let (Some(i), Some(x)) = (
                info.iter().find(|e| e.key == key),
                m.xmp.property(ns, local),
            ) else {
                continue;
            };
            let same = match i.date {
                Some(d) => text::xmp_date(x).is_some_and(|x| x.same_as(&d)),
                None => i.text.trim() == x.trim(),
            };
            if !same {
                let prefix = match ns {
                    xmp::DC => "dc",
                    xmp::PDF => "pdf",
                    _ => "xmp",
                };
                out.push(Issue::warning(
                    "InfoXmp",
                    format!(
                        "/Info /{key} ({}) differs from XMP {prefix}:{local} ({x})",
                        i.text
                    ),
                    Some(m.id),
                ));
            }
        }
        out
    }

    fn features(&self, cat: &pdfi_syntax::Dict) -> Vec<Feature> {
        let f = self.file;
        let mut out = Vec::new();
        let mut push = |key: &str, text: String, obj: Option<ObjId>| {
            out.push(Feature {
                key: key.into(),
                text,
                obj,
            })
        };
        for (k, v) in cat {
            let key = name_str(k);
            let (id, r) = resolve(f, &v.obj);
            let len = |o: &Object| match o {
                Object::Array(a) => a.len(),
                Object::Dict(d) => d.len(),
                _ => 0,
            };
            match key.as_str() {
                "Type" | "Pages" | "Metadata" => {}
                "Version" | "PageLayout" | "PageMode" => push(
                    &key,
                    r.as_name().map_or_else(
                        || r.type_name().to_string(),
                        |n| format!("/{}", name_str(n)),
                    ),
                    id,
                ),
                "Lang" => push(
                    &key,
                    r.as_string()
                        .map(|s| text::decode(&s.bytes))
                        .unwrap_or_default(),
                    id,
                ),
                "Outlines" => {
                    let n = self.outline_children(None).len();
                    push(&key, format!("{n} top-level items"), id)
                }
                "PageLabels" => {
                    let (n, _) = tree::count(f, &v.obj, tree::Kind::Number, MAX_COUNT);
                    push(&key, format!("{n} ranges"), id)
                }
                "Names" => {
                    for (cat_name, root) in r.as_dict().into_iter().flatten() {
                        let (n, complete) = tree::count(f, &root.obj, tree::Kind::Name, MAX_COUNT);
                        push(
                            &format!("Names/{}", name_str(cat_name)),
                            format!("{n}{} entries", if complete { "" } else { "+" }),
                            root.obj.as_ref_id().or(id),
                        )
                    }
                }
                "Dests" => push(&key, format!("{} entries", len(&r)), id),
                "AcroForm" => {
                    let d = r.as_dict();
                    let fields = d.and_then(|d| get(f, d, "Fields")).map_or(0, |a| len(&a));
                    let mut t = format!("{fields} top-level fields");
                    if d.is_some_and(|d| d.get_key("XFA").is_some()) {
                        t += " · XFA";
                    }
                    if let Some(s) = d
                        .and_then(|d| get(f, d, "SigFlags"))
                        .and_then(|s| s.as_i64())
                    {
                        t += &format!(" · SigFlags {s}");
                    }
                    push(&key, t, id)
                }
                "StructTreeRoot" => push(&key, "tagged".into(), id),
                "MarkInfo" => {
                    let marked = r
                        .as_dict()
                        .and_then(|d| get(f, d, "Marked"))
                        .is_some_and(|m| matches!(*m, Object::Bool(true)));
                    push(&key, format!("Marked {marked}"), id)
                }
                "OCProperties" => {
                    let n = r
                        .as_dict()
                        .and_then(|d| get(f, d, "OCGs"))
                        .map_or(0, |a| len(&a));
                    push(&key, format!("{n} optional content groups"), id)
                }
                "OpenAction" => {
                    let link = match &*r {
                        Object::Dict(d) => Some(self.action(d)),
                        Object::Array(_) => Some(Link::Dest(self.dest(&r))),
                        _ => None,
                    };
                    push(
                        &key,
                        link.map_or_else(|| r.type_name().into(), |l| l.describe()),
                        id,
                    )
                }
                _ => push(
                    &key,
                    match len(&r) {
                        0 => r.type_name().into(),
                        n => format!("{} ({n} entries)", r.type_name()),
                    },
                    id,
                ),
            }
        }
        out
    }
}
