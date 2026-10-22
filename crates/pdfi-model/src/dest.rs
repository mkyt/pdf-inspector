//! Destinations and actions: where outline items and links go.

use pdfi_syntax::{Dict, DictExt, ObjId, Object};

use crate::util::{get, name, name_str, resolve};
use crate::{Model, tree};

/// A destination (ISO 32000-2 §12.3.2), resolved to a page where possible.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dest {
    /// The name it was looked up by, for named destinations.
    pub named: Option<String>,
    pub page: Option<ObjId>,
    pub page_index: Option<u64>,
    /// Page number of a destination in another file (`GoToR`).
    pub remote_page: Option<i64>,
    /// `XYZ`, `Fit`, `FitH`, ...
    pub fit: Option<String>,
    /// Parameters after the fit type; `None` for null.
    pub params: Vec<Option<f64>>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    Dest(Dest),
    Uri(String),
    /// Any other action, by its `/S` type.
    Action {
        kind: String,
        detail: Option<String>,
    },
}

/// Text of a file specification (a string or a dictionary).
fn file_spec(m: &Model, o: &Object) -> Option<String> {
    let r = resolve(m.file, o).1;
    match &*r {
        Object::String(s) => Some(crate::text::decode(&s.bytes)),
        Object::Dict(d) => {
            crate::util::text(m.file, d, "UF").or_else(|| crate::util::text(m.file, d, "F"))
        }
        _ => None,
    }
}

impl Model<'_> {
    /// Look up a named destination: strings in the `/Dests` name tree,
    /// names in the catalog's `/Dests` dictionary (either way as fallback).
    fn named_dest(&self, key: &[u8]) -> Option<Object> {
        let (_, cat) = self.catalog()?;
        let cat = cat.as_dict()?;
        let in_tree = || {
            let names = get(self.file, cat, "Names")?;
            let root = names.as_dict()?.get_key("Dests")?.clone();
            tree::lookup(self.file, &root, tree::Kind::Name, tree::Key::Name(key))
        };
        let in_dict = || {
            let d = get(self.file, cat, "Dests")?;
            d.as_dict()?.get(key).map(|n| n.obj.clone())
        };
        in_tree().or_else(in_dict)
    }

    /// Resolve a destination: an array, a name or a string.
    pub fn dest(&self, o: &Object) -> Dest {
        let f = self.file;
        let r = resolve(f, o).1;
        let mut out = Dest::default();
        let key = match &*r {
            Object::Name(n) => Some(n.0.clone()),
            Object::String(s) => Some(s.bytes.clone()),
            _ => None,
        };
        let target;
        let arr = if let Some(k) = key {
            out.named = Some(crate::text::decode(&k));
            let Some(v) = self.named_dest(&k) else {
                out.error = Some("no such named destination".into());
                return out;
            };
            target = v;
            let v = resolve(f, &target).1;
            // A named destination may be a dictionary with the array at /D.
            match v.as_dict() {
                Some(d) => match d.get_key("D") {
                    Some(dd) => resolve(f, dd).1.clone(),
                    None => Object::Null,
                },
                None => v.clone(),
            }
        } else {
            r.clone()
        };
        let Some(a) = arr.as_array() else {
            out.error = Some(format!("destination is a {}", arr.type_name()));
            return out;
        };
        let Some((page, rest)) = a.split_first() else {
            out.error = Some("empty destination".into());
            return out;
        };
        match &page.obj {
            Object::Ref(id) => {
                out.page = Some(*id);
                out.page_index = self.page_index(*id);
                if out.page_index.is_none() {
                    out.error = Some(format!("{id} is not a page of this document"));
                }
            }
            Object::Integer(i) => out.remote_page = Some(*i),
            o => out.error = Some(format!("page is a {}", o.type_name())),
        }
        if let Some((fit, params)) = rest.split_first() {
            out.fit = fit.obj.as_name().map(name_str);
            out.params = params
                .iter()
                .map(|p| crate::util::number(f, &p.obj))
                .collect();
        }
        out
    }

    /// Where `/Dest` or `/A` of an outline item or link annotation goes.
    pub fn link(&self, d: &Dict) -> Option<Link> {
        if let Some(dest) = d.get_key("Dest") {
            return Some(Link::Dest(self.dest(dest)));
        }
        let a = get(self.file, d, "A")?;
        let a = a.as_dict()?;
        Some(self.action(a))
    }

    pub fn action(&self, a: &Dict) -> Link {
        let f = self.file;
        let kind = name(f, a, "S").map(|n| name_str(&n)).unwrap_or_default();
        match kind.as_str() {
            "GoTo" => match a.get_key("D") {
                Some(d) => Link::Dest(self.dest(d)),
                None => Link::Action {
                    kind,
                    detail: Some("no /D".into()),
                },
            },
            "URI" => match get(f, a, "URI").and_then(|u| u.as_string().cloned()) {
                Some(u) => Link::Uri(String::from_utf8_lossy(&u.bytes).into_owned()),
                None => Link::Action {
                    kind,
                    detail: Some("no /URI".into()),
                },
            },
            "GoToR" | "GoToE" | "Launch" => Link::Action {
                detail: a.get_key("F").and_then(|x| file_spec(self, x)),
                kind,
            },
            "Named" => Link::Action {
                detail: name(f, a, "N").map(|n| name_str(&n)),
                kind,
            },
            _ => Link::Action { kind, detail: None },
        }
    }
}
