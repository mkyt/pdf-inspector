//! Name trees and number trees (ISO 32000-2 §7.9.6, §7.9.7).

use std::collections::HashSet;
use std::ops::ControlFlow;

use pdfi_syntax::{ObjId, Object, PdfFile};

use crate::util::{get, resolve};

const MAX_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Name,
    Number,
}

impl Kind {
    fn entries_key(self) -> &'static str {
        match self {
            Kind::Name => "Names",
            Kind::Number => "Nums",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Key<'k> {
    Name(&'k [u8]),
    Number(i64),
}

impl Key<'_> {
    fn matches(&self, file: &PdfFile, o: &Object) -> bool {
        let r = resolve(file, o).1;
        match (self, &*r) {
            (Key::Name(k), Object::String(s)) => s.bytes == *k,
            (Key::Number(k), Object::Integer(i)) => i == k,
            _ => false,
        }
    }

    /// Whether `/Limits` of a node rule this key out.
    fn outside(&self, file: &PdfFile, limits: &[pdfi_syntax::Node]) -> bool {
        let [lo, hi] = limits else { return false };
        let (lo, hi) = (resolve(file, &lo.obj).1, resolve(file, &hi.obj).1);
        match (self, &*lo, &*hi) {
            (Key::Name(k), Object::String(lo), Object::String(hi)) => {
                *k < lo.bytes.as_slice() || *k > hi.bytes.as_slice()
            }
            (Key::Number(k), Object::Integer(lo), Object::Integer(hi)) => k < lo || k > hi,
            _ => false,
        }
    }
}

/// The value stored under `key`, descending by `/Limits`.
pub fn lookup(file: &PdfFile, root: &Object, kind: Kind, key: Key) -> Option<Object> {
    fn go(
        file: &PdfFile,
        node: &Object,
        kind: Kind,
        key: Key,
        depth: usize,
        seen: &mut HashSet<ObjId>,
    ) -> Option<Object> {
        if depth > MAX_DEPTH {
            return None;
        }
        let (id, r) = resolve(file, node);
        if id.is_some_and(|id| !seen.insert(id)) {
            return None;
        }
        let d = r.as_dict()?;
        if let Some(e) = get(file, d, kind.entries_key()) {
            for pair in e.as_array().unwrap_or_default().chunks(2) {
                if key.matches(file, &pair[0].obj) {
                    return pair.get(1).map(|v| v.obj.clone());
                }
            }
        }
        let kids = get(file, d, "Kids")?;
        for kid in kids.as_array().unwrap_or_default() {
            let kr = resolve(file, &kid.obj).1;
            let ruled_out = kr
                .as_dict()
                .and_then(|kd| get(file, kd, "Limits"))
                .is_some_and(|l| key.outside(file, l.as_array().unwrap_or_default()));
            if ruled_out {
                continue;
            }
            if let Some(v) = go(file, &kid.obj, kind, key, depth + 1, seen) {
                return Some(v);
            }
        }
        None
    }
    go(file, root, kind, key, 0, &mut HashSet::new())
}

/// Visit every entry in key order, as stored. Stops early when `f` breaks.
/// Returns whether the walk got through the whole tree.
pub fn walk(
    file: &PdfFile,
    root: &Object,
    kind: Kind,
    f: &mut dyn FnMut(&Object, &Object) -> ControlFlow<()>,
) -> bool {
    fn go(
        file: &PdfFile,
        node: &Object,
        kind: Kind,
        depth: usize,
        seen: &mut HashSet<ObjId>,
        f: &mut dyn FnMut(&Object, &Object) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        if depth > MAX_DEPTH {
            return ControlFlow::Continue(());
        }
        let (id, r) = resolve(file, node);
        if id.is_some_and(|id| !seen.insert(id)) {
            return ControlFlow::Continue(());
        }
        let Some(d) = r.as_dict() else {
            return ControlFlow::Continue(());
        };
        if let Some(e) = get(file, d, kind.entries_key()) {
            for pair in e.as_array().unwrap_or_default().chunks(2) {
                if let [k, v] = pair {
                    let k = resolve(file, &k.obj).1;
                    f(&k, &v.obj)?;
                }
            }
        }
        if let Some(kids) = get(file, d, "Kids") {
            for kid in kids.as_array().unwrap_or_default() {
                go(file, &kid.obj, kind, depth + 1, seen, f)?;
            }
        }
        ControlFlow::Continue(())
    }
    go(file, root, kind, 0, &mut HashSet::new(), f).is_continue()
}

/// Number of entries, counting at most `limit`.
pub fn count(file: &PdfFile, root: &Object, kind: Kind, limit: usize) -> (usize, bool) {
    let mut n = 0;
    let complete = walk(file, root, kind, &mut |_, _| {
        n += 1;
        if n >= limit {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    (n, complete)
}
