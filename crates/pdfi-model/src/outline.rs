//! The document outline (bookmarks).

use std::collections::HashSet;
use std::sync::Arc;

use pdfi_syntax::{DictExt, ObjId};

use crate::dest::Link;
use crate::util::{get, int, text};
use crate::{Issue, Model, lock};

/// Items followed along one `/First` … `/Next` chain.
const MAX_SIBLINGS: usize = 100_000;

#[derive(Clone, Debug, PartialEq)]
pub struct OutlineItem {
    pub id: ObjId,
    pub title: String,
    /// `/Count`: open items below when positive, closed when negative.
    pub count: Option<i64>,
    pub has_children: bool,
    pub link: Option<Link>,
    /// `/F`: 1 italic, 2 bold.
    pub flags: i64,
    pub color: Option<[f64; 3]>,
}

impl Model<'_> {
    /// The outline dictionary (`/Root /Outlines`).
    pub fn outline_root(&self) -> Option<ObjId> {
        let (_, cat) = self.catalog()?;
        cat.as_dict()?.get_key("Outlines")?.as_ref_id()
    }

    /// Items directly under `parent` (`None`: the top level), in order.
    pub fn outline_children(&self, parent: Option<ObjId>) -> Arc<Vec<ObjId>> {
        if let Some(c) = lock(&self.m.outline).get(&parent) {
            return c.clone();
        }
        let c = Arc::new(self.read_siblings(parent));
        lock(&self.m.outline).insert(parent, c.clone());
        c
    }

    fn read_siblings(&self, parent: Option<ObjId>) -> Vec<ObjId> {
        let f = self.file;
        let Some(pid) = parent.or_else(|| self.outline_root()) else {
            return Vec::new();
        };
        let first = f
            .get(pid)
            .ok()
            .and_then(|o| o.node.obj.as_dict()?.get_key("First")?.as_ref_id());
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut cur = first;
        while let Some(id) = cur {
            if !seen.insert(id) {
                self.report(Issue::error(
                    "Outline",
                    format!("outline items under {pid} loop back to {id}"),
                    Some(pid),
                ));
                break;
            }
            if out.len() >= MAX_SIBLINGS {
                self.report(Issue::warning(
                    "Outline",
                    format!(
                        "more than {MAX_SIBLINGS} outline items under {pid}; the rest are not shown"
                    ),
                    Some(pid),
                ));
                break;
            }
            let Ok(o) = f.get(id) else {
                self.report(Issue::error(
                    "Outline",
                    format!("outline item {id} is missing"),
                    Some(pid),
                ));
                break;
            };
            out.push(id);
            cur = o
                .node
                .obj
                .as_dict()
                .and_then(|d| d.get_key("Next")?.as_ref_id());
        }
        out
    }

    pub fn outline_item(&self, id: ObjId) -> Result<OutlineItem, String> {
        let f = self.file;
        let o = f.get(id).map_err(|e| e.to_string())?;
        let d = o
            .node
            .obj
            .as_dict()
            .ok_or_else(|| format!("outline item {id} is not a dictionary"))?;
        let color = get(f, d, "C").and_then(|c| {
            let a = c.as_array()?;
            match a {
                [r, g, b] => Some([r.obj.as_f64()?, g.obj.as_f64()?, b.obj.as_f64()?]),
                _ => None,
            }
        });
        Ok(OutlineItem {
            id,
            title: text(f, d, "Title").unwrap_or_default(),
            count: int(f, d, "Count"),
            has_children: d.get_key("First").is_some(),
            link: self.link(d),
            flags: int(f, d, "F").unwrap_or(0),
            color,
        })
    }
}
