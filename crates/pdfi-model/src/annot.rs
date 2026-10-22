//! Annotations of a page.

use pdfi_syntax::{DictExt, ObjId};

use crate::Model;
use crate::dest::Link;
use crate::util::{Rect, get, int, name, name_str, resolve, text};

#[derive(Clone, Debug, PartialEq)]
pub struct Annot {
    /// Position in the page's `/Annots`.
    pub index: usize,
    pub id: Option<ObjId>,
    pub subtype: Option<String>,
    pub rect: Option<Rect>,
    pub contents: Option<String>,
    /// `/NM`.
    pub name: Option<String>,
    /// `/F` (Invisible, Hidden, Print, ...).
    pub flags: i64,
    /// Has a normal appearance stream (`/AP /N`).
    pub appearance: bool,
    /// Where a link (or a widget's action) goes.
    pub link: Option<Link>,
    /// `/Parent` of a pop-up, `/IRT` of a reply.
    pub parent: Option<ObjId>,
    pub error: Option<String>,
}

impl Model<'_> {
    /// Annotations `offset..offset + limit` of page `index`, and how many the
    /// page has.
    pub fn annots(
        &self,
        index: u64,
        offset: usize,
        limit: usize,
    ) -> Result<(usize, Vec<Annot>), String> {
        let f = self.file;
        let p = self.page(index)?;
        let page = f.get(p.id).map_err(|e| e.to_string())?;
        let Some(arr) = page.node.obj.as_dict().and_then(|d| get(f, d, "Annots")) else {
            return Ok((0, Vec::new()));
        };
        let items = arr.as_array().unwrap_or_default();
        let out = items
            .iter()
            .enumerate()
            .skip(offset)
            .take(limit)
            .map(|(i, n)| {
                let (id, r) = resolve(f, &n.obj);
                let mut a = Annot {
                    index: i,
                    id,
                    subtype: None,
                    rect: None,
                    contents: None,
                    name: None,
                    flags: 0,
                    appearance: false,
                    link: None,
                    parent: None,
                    error: None,
                };
                let Some(d) = r.as_dict() else {
                    a.error = Some(format!("not a dictionary ({})", r.type_name()));
                    return a;
                };
                a.subtype = name(f, d, "Subtype").map(|n| name_str(&n));
                a.rect = d.get_key("Rect").and_then(|r| Rect::read(f, r));
                a.contents = text(f, d, "Contents");
                a.name = text(f, d, "NM");
                a.flags = int(f, d, "F").unwrap_or(0);
                a.appearance = get(f, d, "AP")
                    .is_some_and(|ap| ap.as_dict().is_some_and(|ap| ap.get_key("N").is_some()));
                a.link = self.link(d);
                a.parent = d
                    .get_key("Parent")
                    .or_else(|| d.get_key("IRT"))
                    .and_then(|p| p.as_ref_id());
                a
            })
            .collect();
        Ok((items.len(), out))
    }
}
