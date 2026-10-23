//! Editing (docs/DESIGN.md §4.7): commands applied as a transaction over a
//! [`PdfFile`] snapshot, the undo / redo history, and saving (incremental
//! update or full rewrite, produced in chunks).
//!
//! Edits never change the file. A transaction ([`Tx`]) starts from the
//! edits of a snapshot, applies [`Command`]s to its own copy and yields
//! the new [`Edits`]; [`PdfFile::with_edits`] turns them into the next
//! snapshot, which shares the source and caches with the first.

mod meta;
mod pages;
pub mod save;
mod value;

use std::collections::HashSet;
use std::sync::Arc;

use pdfi_syntax::file::{Edited, Edits};
use pdfi_syntax::filters::{canonical_name, is_image_codec};
use pdfi_syntax::{
    Dict, DictExt, IndirectObject, Name, Node, ObjId, ObjLocation, Object, PdfFile, Stream,
    StreamData,
};

pub use meta::{info_text, pdf_date_text};
pub use save::{FullOptions, SaveMode, SaveReport, Saver, signature_fields};
pub use value::{parse_value, text_string};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditError(pub String);

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EditError {}

pub type Result<T> = std::result::Result<T, EditError>;

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(EditError(msg.into()))
}

/// The object or the trailer a node path starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Root {
    Trailer,
    Object(ObjId),
}

/// A node: the root, then child indices through dictionaries (entry
/// order), streams (their dictionary) and arrays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeAt {
    pub root: Root,
    pub path: Vec<usize>,
}

/// How [`Command::SetStreamData`] encodes the data. Image codecs at the end
/// of the stream's filter list (DCT, JPX, JBIG2, CCITT) are kept: the data
/// is their input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamFilter {
    None,
    Flate,
}

#[derive(Clone, Debug)]
pub enum Command<'a> {
    /// Replace the node. An empty path replaces the whole object (a stream
    /// given a dictionary keeps its data) or the trailer.
    SetValue {
        at: NodeAt,
        value: Object,
    },
    /// Add or replace an entry of the dictionary (or stream) at `at`.
    SetEntry {
        at: NodeAt,
        key: Name,
        value: Object,
    },
    RemoveEntry {
        at: NodeAt,
        key: Name,
    },
    /// Insert into the array at `at` (`index` may be its length).
    Insert {
        at: NodeAt,
        index: usize,
        value: Object,
    },
    /// Remove the child at `index` of the array or dictionary at `at`.
    RemoveChild {
        at: NodeAt,
        index: usize,
    },
    /// A new object (a stream when `data` is given, stored as is).
    AddObject {
        value: Object,
        data: Option<Vec<u8>>,
    },
    DeleteObject {
        id: ObjId,
    },
    /// Replace the (decoded) data of a stream.
    SetStreamData {
        id: ObjId,
        data: Vec<u8>,
        filter: StreamFilter,
    },
    /// Re-encode the data of a stream.
    SetStreamFilter {
        id: ObjId,
        filter: StreamFilter,
    },
    DeletePages {
        pages: Vec<u64>,
    },
    /// Move pages so that they start at index `to` of the result.
    MovePages {
        pages: Vec<u64>,
        to: u64,
    },
    /// Add a multiple of 90 degrees to the pages' `/Rotate`.
    RotatePages {
        pages: Vec<u64>,
        by: i64,
    },
    InsertBlankPage {
        at: u64,
        width: f64,
        height: f64,
    },
    /// Copy pages (with everything they use) from another file, or from
    /// this one (`None`: resources are shared, not copied).
    InsertPages {
        at: u64,
        from: Option<&'a PdfFile>,
        pages: Vec<u64>,
    },
    /// Set (or remove) an Info entry given as text; dates may be ISO 8601.
    /// `sync_xmp` updates the matching XMP property if there is metadata.
    SetInfo {
        key: String,
        value: Option<String>,
        sync_xmp: bool,
    },
    /// Replace (or add) the catalog's XMP metadata.
    SetXmp {
        xml: Vec<u8>,
    },
}

/// What a transaction did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    /// Objects added, in order.
    pub created: Vec<ObjId>,
    /// Object numbers changed, added or deleted.
    pub changed: Vec<u32>,
    pub trailer_changed: bool,
    /// Things worth telling (an Info key with no XMP counterpart, ...).
    pub notes: Vec<String>,
}

/// A transaction over a snapshot.
pub struct Tx<'f> {
    file: &'f PdfFile,
    edits: Edits,
    next_num: u32,
    applied: Applied,
    changed: HashSet<u32>,
}

impl<'f> Tx<'f> {
    pub fn new(file: &'f PdfFile) -> Self {
        let edits = file.edits().cloned().unwrap_or_default();
        let next_num = file
            .xref()
            .size()
            .max(edits.objects.keys().next_back().map_or(0, |n| n + 1))
            .max(1);
        Self {
            file,
            edits,
            next_num,
            applied: Applied::default(),
            changed: HashSet::new(),
        }
    }

    /// Apply the commands in order; any failure abandons the transaction.
    pub fn apply_all<'c>(
        file: &'f PdfFile,
        cmds: impl IntoIterator<Item = Command<'c>>,
    ) -> Result<(Edits, Applied)> {
        let mut tx = Tx::new(file);
        for (i, c) in cmds.into_iter().enumerate() {
            tx.apply(c)
                .map_err(|e| EditError(format!("command {}: {e}", i + 1)))?;
        }
        Ok(tx.finish())
    }

    pub fn finish(mut self) -> (Edits, Applied) {
        let mut changed: Vec<u32> = self.changed.into_iter().collect();
        changed.sort_unstable();
        self.applied.changed = changed;
        (self.edits, self.applied)
    }

    /// The file with this transaction's edits so far.
    pub fn view(&self) -> PdfFile {
        self.file.with_edits(self.edits.clone())
    }

    pub fn get(&self, id: ObjId) -> Result<Arc<IndirectObject>> {
        match self.edits.objects.get(&id.num) {
            Some(Edited::Object(o)) if o.id == id => Ok(o.clone()),
            Some(_) => err(format!("object {id} does not exist")),
            None => self
                .file
                .get_original(id)
                .map_err(|e| EditError(e.to_string())),
        }
    }

    /// The live object with this number.
    pub fn get_num(&self, num: u32) -> Result<Arc<IndirectObject>> {
        let generation = match self.edits.objects.get(&num) {
            Some(Edited::Object(o)) => o.id.generation,
            Some(Edited::Deleted { .. }) => return err(format!("object {num} is deleted")),
            None => self.file.file_xref().get(num).map_or(0, |e| e.generation()),
        };
        self.get(ObjId::new(num, generation))
    }

    /// Follow references (missing objects are null).
    pub fn resolve(&self, o: &Object) -> Object {
        let mut cur = o.clone();
        for _ in 0..32 {
            match cur {
                Object::Ref(id) => match self.get(id) {
                    Ok(x) => cur = x.node.obj.clone(),
                    Err(_) => return Object::Null,
                },
                other => return other,
            }
        }
        Object::Null
    }

    pub fn trailer(&self) -> &Dict {
        self.edits
            .trailer
            .as_ref()
            .unwrap_or_else(|| self.file.file_trailer())
    }

    pub fn set_trailer(&mut self, d: Dict) {
        self.edits.trailer = Some(d);
        self.applied.trailer_changed = true;
    }

    /// Store `obj` as object `id` (replacing it).
    pub fn put(&mut self, id: ObjId, obj: Object) {
        let mut node = Node::synthetic(obj);
        strip_spans(&mut node);
        self.edits.objects.insert(
            id.num,
            Edited::Object(Arc::new(IndirectObject {
                id,
                node,
                location: ObjLocation::Edited,
            })),
        );
        self.changed.insert(id.num);
    }

    pub fn add(&mut self, obj: Object) -> ObjId {
        let id = ObjId::new(self.next_num, 0);
        self.next_num += 1;
        self.put(id, obj);
        self.applied.created.push(id);
        id
    }

    pub fn delete(&mut self, id: ObjId) -> Result<()> {
        self.get(id)?;
        match self.file.file_xref().get(id.num) {
            Some(e) if e.is_live() && e.generation() == id.generation => {
                self.edits.objects.insert(
                    id.num,
                    Edited::Deleted {
                        generation: id.generation.saturating_add(1),
                    },
                );
            }
            // Added by an edit: forget it.
            _ => {
                self.edits.objects.remove(&id.num);
            }
        }
        self.changed.insert(id.num);
        Ok(())
    }

    pub fn note(&mut self, s: impl Into<String>) {
        self.applied.notes.push(s.into());
    }

    pub fn apply(&mut self, cmd: Command<'_>) -> Result<()> {
        match cmd {
            Command::SetValue { at, value } => {
                if at.path.is_empty() {
                    return self.replace_root(at.root, value);
                }
                self.modify(&at.root, |root| {
                    let n = node_mut(root, &at.path)?;
                    *n = value;
                    Ok(())
                })
            }
            Command::SetEntry { at, key, value } => {
                self.modify(&at.root, |root| match node_mut(root, &at.path)? {
                    Object::Dict(d) | Object::Stream(Stream { dict: d, .. }) => {
                        d.insert(key, Node::synthetic(value));
                        Ok(())
                    }
                    o => err(format!("{} is not a dictionary", o.type_name())),
                })
            }
            Command::RemoveEntry { at, key } => {
                self.modify(&at.root, |root| match node_mut(root, &at.path)? {
                    Object::Dict(d) | Object::Stream(Stream { dict: d, .. }) => d
                        .shift_remove(&key)
                        .map(|_| ())
                        .ok_or_else(|| EditError(format!("no key {key:?}"))),
                    o => err(format!("{} is not a dictionary", o.type_name())),
                })
            }
            Command::Insert { at, index, value } => {
                self.modify(&at.root, |root| match node_mut(root, &at.path)? {
                    Object::Array(a) if index <= a.len() => {
                        a.insert(index, Node::synthetic(value));
                        Ok(())
                    }
                    Object::Array(a) => err(format!("index {index} beyond {} items", a.len())),
                    o => err(format!("{} is not an array", o.type_name())),
                })
            }
            Command::RemoveChild { at, index } => {
                self.modify(&at.root, |root| match node_mut(root, &at.path)? {
                    Object::Array(a) if index < a.len() => {
                        a.remove(index);
                        Ok(())
                    }
                    Object::Dict(d) | Object::Stream(Stream { dict: d, .. }) if index < d.len() => {
                        d.shift_remove_index(index);
                        Ok(())
                    }
                    o => err(format!("{} has no child {index}", o.type_name())),
                })
            }
            Command::AddObject { value, data } => {
                let obj = match data {
                    Some(data) => {
                        let Object::Dict(dict) = value else {
                            return err("a stream needs a dictionary");
                        };
                        Object::Stream(Stream {
                            dict,
                            data: StreamData::Owned(data.into()),
                        })
                    }
                    None => value,
                };
                self.add(obj);
                Ok(())
            }
            Command::DeleteObject { id } => self.delete(id),
            Command::SetStreamData { id, data, filter } => self.set_stream_data(id, data, filter),
            Command::SetStreamFilter { id, filter } => {
                let o = self.get(id)?;
                let Object::Stream(s) = &o.node.obj else {
                    return err(format!("object {id} is not a stream"));
                };
                let view = self.view();
                let d = view.decode_stream(id, s, None);
                if let Some(e) = d.errors.first() {
                    return err(format!("cannot decode the data: {e}"));
                }
                if d.unsupported || d.stopped_at.as_ref().is_some_and(|n| !is_image_codec(&n.0)) {
                    return err("the stream uses a filter that cannot be decoded");
                }
                self.set_stream_data(id, d.data, filter)
            }
            Command::DeletePages { pages } => pages::delete(self, &pages),
            Command::MovePages { pages, to } => pages::move_to(self, &pages, to),
            Command::RotatePages { pages, by } => pages::rotate(self, &pages, by),
            Command::InsertBlankPage { at, width, height } => {
                pages::insert_blank(self, at, width, height)
            }
            Command::InsertPages { at, from, pages } => {
                pages::insert_copies(self, at, from, &pages)
            }
            Command::SetInfo {
                key,
                value,
                sync_xmp,
            } => meta::set_info(self, &key, value.as_deref(), sync_xmp),
            Command::SetXmp { xml } => meta::set_xmp(self, xml),
        }
    }

    /// Apply `f` to the root's object (a copy), then store it.
    fn modify(&mut self, root: &Root, f: impl FnOnce(&mut Object) -> Result<()>) -> Result<()> {
        match *root {
            Root::Trailer => {
                let mut o = Object::Dict(self.trailer().clone());
                f(&mut o)?;
                let Object::Dict(d) = o else { unreachable!() };
                self.set_trailer(d);
            }
            Root::Object(id) => {
                let mut o = self.get(id)?.node.obj.clone();
                f(&mut o)?;
                self.put(id, o);
            }
        }
        Ok(())
    }

    fn replace_root(&mut self, root: Root, value: Object) -> Result<()> {
        match root {
            Root::Trailer => match value {
                Object::Dict(d) => {
                    self.set_trailer(d);
                    Ok(())
                }
                _ => err("the trailer must be a dictionary"),
            },
            Root::Object(id) => {
                let old = self.get(id)?;
                let new = match (&old.node.obj, value) {
                    (Object::Stream(s), Object::Dict(dict)) => Object::Stream(Stream {
                        dict,
                        data: s.data.clone(),
                    }),
                    (_, v) => v,
                };
                self.put(id, new);
                Ok(())
            }
        }
    }

    fn set_stream_data(&mut self, id: ObjId, data: Vec<u8>, filter: StreamFilter) -> Result<()> {
        let o = self.get(id)?;
        let Object::Stream(s) = &o.node.obj else {
            return err(format!("object {id} is not a stream"));
        };
        let mut dict = s.dict.clone();
        // Keep the image codecs at the end of the filter list.
        let specs = self.view().filter_specs(&dict);
        let keep = specs
            .iter()
            .rev()
            .take_while(|f| is_image_codec(&f.name.0))
            .count();
        let tail = &specs[specs.len() - keep..];
        let mut names: Vec<Object> = Vec::new();
        let mut parms: Vec<Object> = Vec::new();
        let encoded = match filter {
            StreamFilter::None => data,
            StreamFilter::Flate => {
                names.push(Object::Name(Name::from("FlateDecode")));
                parms.push(Object::Null);
                pdfi_syntax::write::deflate(&data)
            }
        };
        for f in tail {
            names.push(Object::Name(Name::new(canonical_name(&f.name.0).to_vec())));
            parms.push(f.parms.clone().map_or(Object::Null, Object::Dict));
        }
        for k in [
            "Filter",
            "DecodeParms",
            "DL",
            "F",
            "DP",
            "FFilter",
            "FDecodeParms",
        ] {
            dict.shift_remove(k.as_bytes());
        }
        let one = |mut v: Vec<Object>| {
            if v.len() == 1 {
                v.remove(0)
            } else {
                Object::Array(v.into_iter().map(Node::synthetic).collect())
            }
        };
        if !names.is_empty() {
            dict.insert(Name::from("Filter"), Node::synthetic(one(names)));
        }
        if parms.iter().any(|p| !matches!(p, Object::Null)) {
            dict.insert(Name::from("DecodeParms"), Node::synthetic(one(parms)));
        }
        dict.insert(
            Name::from("Length"),
            Node::synthetic(Object::Integer(encoded.len() as i64)),
        );
        self.put(
            id,
            Object::Stream(Stream {
                dict,
                data: StreamData::Owned(encoded.into()),
            }),
        );
        Ok(())
    }

    /// The catalog's object number and dictionary.
    pub(crate) fn catalog(&self) -> Result<(ObjId, Dict)> {
        let id = self
            .trailer()
            .get_key("Root")
            .and_then(Object::as_ref_id)
            .ok_or_else(|| EditError("the trailer has no /Root reference".into()))?;
        match &self.get(id)?.node.obj {
            Object::Dict(d) => Ok((id, d.clone())),
            _ => err("the catalog is not a dictionary"),
        }
    }
}

/// The node at `path` below `root`.
fn node_mut<'n>(root: &'n mut Object, path: &[usize]) -> Result<&'n mut Object> {
    let mut cur = root;
    for &i in path {
        cur = match cur {
            Object::Dict(d) | Object::Stream(Stream { dict: d, .. }) => d
                .get_index_mut(i)
                .map(|(_, v)| &mut v.obj)
                .ok_or_else(|| EditError(format!("no entry {i}")))?,
            Object::Array(a) => a
                .get_mut(i)
                .map(|v| &mut v.obj)
                .ok_or_else(|| EditError(format!("no item {i}")))?,
            o => return err(format!("{} has no children", o.type_name())),
        };
    }
    Ok(cur)
}

/// Spans of a stored edit would point into the file; drop them.
fn strip_spans(n: &mut Node) {
    n.span = None;
    match &mut n.obj {
        Object::Array(a) => a.iter_mut().for_each(strip_spans),
        Object::Dict(d) | Object::Stream(Stream { dict: d, .. }) => {
            d.values_mut().for_each(strip_spans)
        }
        _ => {}
    }
}

/// Undo / redo over whole edit states. Entry 0 is the file as opened.
#[derive(Clone, Debug)]
pub struct History {
    states: Vec<(String, Arc<Edits>)>,
    pos: usize,
}

impl Default for History {
    fn default() -> Self {
        Self {
            states: vec![(String::new(), Arc::default())],
            pos: 0,
        }
    }
}

/// Edit states kept for undo.
const MAX_UNDO: usize = 200;

impl History {
    /// Record the state after an edit (dropping anything that was undone).
    pub fn push(&mut self, label: impl Into<String>, edits: Edits) {
        self.states.truncate(self.pos + 1);
        self.states.push((label.into(), Arc::new(edits)));
        if self.states.len() > MAX_UNDO + 1 {
            // The oldest kept state becomes the floor.
            self.states.remove(0);
            self.states[0].0.clear();
        }
        self.pos = self.states.len() - 1;
    }

    pub fn current(&self) -> &Edits {
        &self.states[self.pos].1
    }

    /// Step back; returns the state to show.
    pub fn undo(&mut self) -> Option<&Edits> {
        if self.pos == 0 {
            return None;
        }
        self.pos -= 1;
        Some(self.current())
    }

    pub fn redo(&mut self) -> Option<&Edits> {
        if self.pos + 1 >= self.states.len() {
            return None;
        }
        self.pos += 1;
        Some(self.current())
    }

    /// Label of the edit `undo` would revert.
    pub fn undo_label(&self) -> Option<&str> {
        (self.pos > 0).then(|| self.states[self.pos].0.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.states.get(self.pos + 1).map(|s| s.0.as_str())
    }

    /// Edits that can be undone.
    pub fn depth(&self) -> usize {
        self.pos
    }

    /// Whether the current state differs from the file.
    pub fn dirty(&self) -> bool {
        !self.current().is_empty()
    }
}
