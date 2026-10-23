//! Writing a snapshot out: an incremental update (the file's bytes, then
//! the edited objects and a cross-reference section) or a full rewrite.
//!
//! [`Saver`] produces the output in chunks ([`Saver::next_chunk`]) so the
//! caller decides where it goes and when: a job writing a file, or the
//! browser awaiting a `FileSystemWritableFileStream` between chunks. Data
//! the file already holds (its bytes for an update, stream data for a
//! rewrite) is copied from the source a chunk at a time.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::io::Read;
use std::ops::Deref;

use md5::{Digest, Md5};
use pdfi_syntax::file::Edited;
use pdfi_syntax::source::read_vec;
use pdfi_syntax::write::{deflate, write_object};
use pdfi_syntax::xref::{XrefEntry, XrefKind};
use pdfi_syntax::{
    Dict, DictExt, Name, Node, ObjId, Object, PdfFile, PdfString, StrKind, Stream, StreamData,
};

use crate::{EditError, Result, err};

/// Size of the chunks [`Saver::next_chunk`] returns (roughly).
pub const CHUNK: usize = 1 << 20;
/// Objects per object stream.
const OBJSTM_SIZE: usize = 100;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FullOptions {
    /// A cross-reference stream instead of a table (needed for object streams).
    pub xref_stream: bool,
    /// Pack objects other than streams into object streams.
    pub object_streams: bool,
    /// Flate-compress streams that have no filter.
    pub compress: bool,
    /// Leave out objects the trailer does not reach.
    pub remove_unused: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveMode {
    /// Append the edits to the unchanged file (keeps signatures valid).
    Incremental,
    /// Write every object anew (decrypted, without the old revisions).
    Full(FullOptions),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SaveReport {
    pub bytes: u64,
    /// Objects written (in object streams too).
    pub objects: u64,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy)]
enum Entry {
    InUse { offset: u64, generation: u16 },
    Compressed { stream: u32, index: u32 },
    Free { generation: u16 },
}

enum Piece {
    Bytes(Vec<u8>),
    /// Bytes of the source.
    Copy {
        start: u64,
        end: u64,
    },
    Object(ObjId),
    /// Flush the objects waiting for an object stream.
    FlushObjStm,
    Xref,
}

pub struct Saver<F: Deref<Target = PdfFile>> {
    file: F,
    mode: SaveMode,
    pieces: VecDeque<Piece>,
    pos: u64,
    entries: BTreeMap<u32, Entry>,
    /// Object numbers available for new objects (object streams, the xref stream).
    next_num: u32,
    /// Objects waiting to go into an object stream.
    pending: Vec<(ObjId, Vec<u8>)>,
    /// Hash of what an update adds (for the second part of `/ID`).
    hash: Md5,
    total: u64,
    report: SaveReport,
    finished: bool,
}

fn too_big(e: impl std::fmt::Display) -> EditError {
    EditError(format!("cannot write: {e}"))
}

impl<F: Deref<Target = PdfFile>> Saver<F> {
    pub fn new(file: F, mode: SaveMode) -> Result<Self> {
        let f: &PdfFile = &file;
        let mut report = SaveReport::default();
        let signed = signature_fields(f);
        let mut pieces = VecDeque::new();
        let mut size = f.xref().size().max(1);
        let total;
        match mode {
            SaveMode::Incremental => {
                if f.security().is_some() {
                    return err(
                        "an encrypted file can only be saved as a full (decrypted) rewrite",
                    );
                }
                if f.is_recovered() {
                    return err(
                        "the file's cross-reference data is broken; save a full rewrite instead",
                    );
                }
                let len = f.source().len();
                pieces.push_back(Piece::Copy { start: 0, end: len });
                let last = read_vec(f.source(), len.saturating_sub(1), 1).map_err(too_big)?;
                if !matches!(last.first(), Some(b'\n' | b'\r')) {
                    pieces.push_back(Piece::Bytes(b"\n".to_vec()));
                }
                if let Some(e) = f.edits() {
                    for (&num, x) in &e.objects {
                        if let Edited::Object(o) = x {
                            pieces.push_back(Piece::Object(o.id));
                        }
                        size = size.max(num + 1);
                    }
                }
                total = len;
                if signed > 0 {
                    report.warnings.push(format!(
                        "{signed} signature field(s): the update keeps the signed bytes, \
                         but viewers may report the changes made after signing"
                    ));
                }
            }
            SaveMode::Full(o) => {
                if f.security().is_some_and(|s| !s.is_authenticated()) {
                    return err("the file is encrypted and no valid password was given");
                }
                let version = f.header().version.clone().unwrap_or_else(|| "1.4".into());
                let version = if (o.xref_stream || o.object_streams) && version.as_str() < "1.5" {
                    "1.5".to_string()
                } else {
                    version
                };
                pieces.push_back(Piece::Bytes(
                    format!("%PDF-{version}\n%\u{e2}\u{e3}\u{cf}\u{d3}\n").into_bytes(),
                ));
                let ids: Vec<ObjId> = if o.remove_unused {
                    reachable(f)
                } else {
                    f.xref().live_ids().collect()
                };
                pieces.extend(ids.into_iter().map(Piece::Object));
                if o.object_streams {
                    pieces.push_back(Piece::FlushObjStm);
                }
                total = f.source().len();
                if signed > 0 {
                    report.warnings.push(format!(
                        "{signed} signature field(s): a full rewrite invalidates the signatures"
                    ));
                }
                if f.security().is_some() {
                    report
                        .warnings
                        .push("the file is written without encryption".into());
                }
            }
        }
        pieces.push_back(Piece::Xref);
        Ok(Self {
            file,
            mode,
            pieces,
            pos: 0,
            entries: BTreeMap::new(),
            next_num: size,
            pending: Vec::new(),
            hash: Md5::new(),
            total,
            report,
            finished: false,
        })
    }

    /// Bytes written so far, and an estimate of the total.
    pub fn progress(&self) -> (u64, u64) {
        (self.pos, self.total.max(self.pos))
    }

    pub fn report(&self) -> &SaveReport {
        &self.report
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    /// The next chunk of output (`None` when everything was produced).
    pub fn next_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        let mut out = Vec::new();
        while out.len() < CHUNK {
            let Some(p) = self.pieces.pop_front() else {
                break;
            };
            match p {
                Piece::Bytes(b) => self.emit(&mut out, &b, true),
                Piece::Copy { start, end } => {
                    let n = (end - start).min((CHUNK - out.len()).max(64 << 10) as u64);
                    let data = read_vec(self.file.source(), start, n as usize).map_err(too_big)?;
                    if data.is_empty() && n > 0 {
                        return err("the source ended early");
                    }
                    let got = data.len() as u64;
                    self.emit(&mut out, &data, false);
                    if start + got < end {
                        self.pieces.push_front(Piece::Copy {
                            start: start + got,
                            end,
                        });
                    }
                }
                Piece::Object(id) => self.object(id, &mut out)?,
                Piece::FlushObjStm => self.flush_objstm(&mut out)?,
                Piece::Xref => self.xref(&mut out)?,
            }
        }
        if out.is_empty() {
            self.finished = true;
            return Ok(None);
        }
        Ok(Some(out))
    }

    fn emit(&mut self, out: &mut Vec<u8>, b: &[u8], hashed: bool) {
        if hashed {
            self.hash.update(b);
        }
        out.extend_from_slice(b);
        self.pos += b.len() as u64;
    }

    fn full(&self) -> Option<FullOptions> {
        match self.mode {
            SaveMode::Full(o) => Some(o),
            SaveMode::Incremental => None,
        }
    }

    fn object(&mut self, id: ObjId, out: &mut Vec<u8>) -> Result<()> {
        let f: &PdfFile = &self.file;
        let o = match f.get(id) {
            Ok(o) => o,
            Err(e) => {
                self.report
                    .warnings
                    .push(format!("object {id} left out: {e}"));
                return Ok(());
            }
        };
        let full = self.full();
        if full.is_some() {
            if Some(id) == f.encrypt_ref() {
                return Ok(());
            }
            match &o.node.obj {
                Object::Stream(s) if s.dict.type_is("XRef") || s.dict.type_is("ObjStm") => {
                    return Ok(());
                }
                // Stale once the file is rewritten.
                Object::Dict(d) if d.contains_key(b"Linearized".as_slice()) => return Ok(()),
                _ => {}
            }
        }
        self.report.objects += 1;
        let Object::Stream(s) = &o.node.obj else {
            let mut body = Vec::new();
            write_object(&o.node.obj, &mut body);
            if full.is_some_and(|o| o.object_streams) && id.generation == 0 {
                self.pending.push((id, body));
                if self.pending.len() >= OBJSTM_SIZE {
                    self.flush_objstm(out)?;
                }
                return Ok(());
            }
            self.entries.insert(
                id.num,
                Entry::InUse {
                    offset: self.pos,
                    generation: id.generation,
                },
            );
            let mut b = format!("{} {} obj\n", id.num, id.generation).into_bytes();
            b.extend(body);
            b.extend_from_slice(b"\nendobj\n");
            self.emit(out, &b, true);
            return Ok(());
        };
        self.entries.insert(
            id.num,
            Entry::InUse {
                offset: self.pos,
                generation: id.generation,
            },
        );
        let mut dict = s.dict.clone();
        let encrypted = f.crypt_decoder(id, s, &f.filter_specs(&s.dict)).is_some();
        let compress = full.is_some_and(|o| o.compress)
            && !dict.contains_key(b"Filter".as_slice())
            && !dict.type_is("Metadata");
        let data: Option<Vec<u8>> = match (&s.data, encrypted, compress) {
            // Stream data the file has: copied as is.
            (StreamData::Source(_), false, false) => None,
            (StreamData::Owned(d), _, false) => Some(d.to_vec()),
            _ => {
                let mut d = Vec::new();
                f.decrypted_reader(id, s)
                    .read_to_end(&mut d)
                    .map_err(|e| EditError(format!("object {id}: {e}")))?;
                if compress && d.len() > 64 {
                    let z = deflate(&d);
                    if z.len() < d.len() {
                        dict.insert(
                            Name::from("Filter"),
                            Node::synthetic(Object::Name(Name::from("FlateDecode"))),
                        );
                        dict.shift_remove(b"DecodeParms".as_slice());
                        d = z;
                    }
                }
                Some(d)
            }
        };
        let len = match (&data, &s.data) {
            (Some(d), _) => d.len() as u64,
            (None, StreamData::Source(sp)) => sp.len(),
            (None, StreamData::Owned(d)) => d.len() as u64,
        };
        dict.insert(
            Name::from("Length"),
            Node::synthetic(Object::Integer(len as i64)),
        );
        let mut b = format!("{} {} obj\n", id.num, id.generation).into_bytes();
        write_object(&Object::Dict(dict), &mut b);
        b.extend_from_slice(b"\nstream\n");
        self.emit(out, &b, true);
        let tail = Piece::Bytes(b"\nendstream\nendobj\n".to_vec());
        match (data, &s.data) {
            (Some(d), _) => {
                self.emit(out, &d, false);
                self.pieces.push_front(tail);
            }
            (None, StreamData::Source(sp)) => {
                self.pieces.push_front(tail);
                self.pieces.push_front(Piece::Copy {
                    start: sp.start,
                    end: sp.end,
                });
            }
            (None, StreamData::Owned(_)) => unreachable!("owned data is read above"),
        }
        Ok(())
    }

    fn flush_objstm(&mut self, out: &mut Vec<u8>) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let num = self.next_num;
        self.next_num += 1;
        let mut head = Vec::new();
        let mut body = Vec::new();
        for (i, (id, b)) in self.pending.iter().enumerate() {
            head.extend_from_slice(format!("{} {} ", id.num, body.len()).as_bytes());
            body.extend_from_slice(b);
            body.push(b'\n');
            self.entries.insert(
                id.num,
                Entry::Compressed {
                    stream: num,
                    index: i as u32,
                },
            );
        }
        let n = self.pending.len();
        self.pending.clear();
        head.push(b'\n');
        let first = head.len();
        head.extend(body);
        let data = deflate(&head);
        let mut dict = Dict::new();
        let mut set = |k: &str, v: Object| {
            dict.insert(Name::from(k), Node::synthetic(v));
        };
        set("Type", Object::Name(Name::from("ObjStm")));
        set("N", Object::Integer(n as i64));
        set("First", Object::Integer(first as i64));
        set("Filter", Object::Name(Name::from("FlateDecode")));
        set("Length", Object::Integer(data.len() as i64));
        self.entries.insert(
            num,
            Entry::InUse {
                offset: self.pos,
                generation: 0,
            },
        );
        let mut b = format!("{num} 0 obj\n").into_bytes();
        write_object(&Object::Dict(dict), &mut b);
        b.extend_from_slice(b"\nstream\n");
        b.extend(data);
        b.extend_from_slice(b"\nendstream\nendobj\n");
        self.emit(out, &b, true);
        Ok(())
    }

    fn xref(&mut self, out: &mut Vec<u8>) -> Result<()> {
        let f: &PdfFile = &self.file;
        let incremental = self.full().is_none();
        // Free entries: deleted objects (update), every gap (rewrite).
        if incremental {
            if let Some(e) = f.edits() {
                for (&num, x) in &e.objects {
                    if let Edited::Deleted { generation } = x {
                        self.entries.insert(
                            num,
                            Entry::Free {
                                generation: *generation,
                            },
                        );
                    }
                }
            }
        }
        let use_stream = match self.mode {
            SaveMode::Incremental => matches!(
                f.revisions().first().map(|r| r.kind),
                Some(XrefKind::Stream | XrefKind::Hybrid)
            ),
            SaveMode::Full(o) => o.xref_stream || o.object_streams,
        };
        let mut trailer = Dict::new();
        let edited = f.trailer();
        for key in ["Root", "Info"] {
            if let Some(v) = edited.get(key.as_bytes()) {
                trailer.insert(Name::from(key), v.clone());
            }
        }
        if incremental {
            // Everything else the trailer has stays (but what describes this
            // section is new).
            for (k, v) in edited {
                if !matches!(
                    k.as_bytes(),
                    b"Size"
                        | b"Prev"
                        | b"XRefStm"
                        | b"Root"
                        | b"Info"
                        | b"ID"
                        | b"Type"
                        | b"Index"
                        | b"W"
                        | b"Length"
                        | b"Filter"
                        | b"DecodeParms"
                ) {
                    trailer.insert(k.clone(), v.clone());
                }
            }
            if let Some(r) = f.revisions().first() {
                trailer.insert(
                    Name::from("Prev"),
                    Node::synthetic(Object::Integer(r.offset as i64)),
                );
            }
        }
        let id0 = match edited.get_key("ID") {
            Some(Object::Array(a)) => a.first().and_then(|n| n.obj.as_string()).cloned(),
            _ => None,
        };
        let digest = self.hash.clone().finalize().to_vec();
        let id1 = PdfString {
            bytes: digest.clone(),
            kind: StrKind::Hex,
        };
        let ids = match (incremental, id0) {
            (true, Some(a)) => [a, id1],
            (false, Some(a)) => {
                let b = match edited.get_key("ID") {
                    Some(Object::Array(x)) => x.get(1).and_then(|n| n.obj.as_string()).cloned(),
                    _ => None,
                };
                [a.clone(), b.unwrap_or(a)]
            }
            (_, None) => [id1.clone(), id1],
        };
        trailer.insert(
            Name::from("ID"),
            Node::synthetic(Object::Array(
                ids.into_iter()
                    .map(|s| Node::synthetic(Object::String(s)))
                    .collect(),
            )),
        );
        let xref_pos = self.pos;
        let mut b = Vec::new();
        if use_stream {
            let num = self.next_num;
            self.next_num += 1;
            self.entries.insert(
                num,
                Entry::InUse {
                    offset: xref_pos,
                    generation: 0,
                },
            );
            let size = self.next_num;
            let rows = self.rows(size, incremental);
            let w2 = if xref_pos > u32::MAX as u64 { 8 } else { 4 };
            let mut data = Vec::new();
            let mut index = Vec::new();
            for (start, entries) in &rows {
                index.push(Node::synthetic(Object::Integer(*start as i64)));
                index.push(Node::synthetic(Object::Integer(entries.len() as i64)));
                for e in entries {
                    let (t, a, c) = match *e {
                        Entry::Free { generation } => (0u8, 0u64, generation as u32),
                        Entry::InUse { offset, generation } => (1, offset, generation as u32),
                        Entry::Compressed { stream, index } => (2, stream as u64, index),
                    };
                    data.push(t);
                    data.extend_from_slice(&a.to_be_bytes()[8 - w2..]);
                    data.extend_from_slice(&(c as u16).to_be_bytes());
                }
            }
            let data = deflate(&data);
            let mut set = |k: &str, v: Object| {
                trailer.insert(Name::from(k), Node::synthetic(v));
            };
            set("Type", Object::Name(Name::from("XRef")));
            set("Size", Object::Integer(size as i64));
            set("Index", Object::Array(index));
            set(
                "W",
                Object::Array(
                    [1, w2 as i64, 2]
                        .map(|v| Node::synthetic(Object::Integer(v)))
                        .to_vec(),
                ),
            );
            set("Filter", Object::Name(Name::from("FlateDecode")));
            set("Length", Object::Integer(data.len() as i64));
            b.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
            write_object(&Object::Dict(trailer), &mut b);
            b.extend_from_slice(b"\nstream\n");
            b.extend(data);
            b.extend_from_slice(b"\nendstream\nendobj\n");
        } else {
            let size = self.next_num;
            let rows = self.rows(size, incremental);
            b.extend_from_slice(b"xref\n");
            for (start, entries) in &rows {
                b.extend_from_slice(format!("{start} {}\n", entries.len()).as_bytes());
                for e in entries {
                    let line = match *e {
                        Entry::Free { generation } => format!("0000000000 {generation:05} f\r\n"),
                        Entry::InUse { offset, generation } => {
                            format!("{offset:010} {generation:05} n\r\n")
                        }
                        Entry::Compressed { .. } => {
                            unreachable!("tables have no compressed entries")
                        }
                    };
                    b.extend_from_slice(line.as_bytes());
                }
            }
            trailer.insert(
                Name::from("Size"),
                Node::synthetic(Object::Integer(size as i64)),
            );
            b.extend_from_slice(b"trailer\n");
            write_object(&Object::Dict(trailer), &mut b);
            b.push(b'\n');
        }
        b.extend_from_slice(format!("startxref\n{xref_pos}\n%%EOF\n").as_bytes());
        self.emit(out, &b, true);
        self.report.bytes = self.pos;
        Ok(())
    }

    /// Subsections of the cross-reference data: every number below `size`
    /// for a rewrite, runs of the written numbers for an update.
    fn rows(&self, size: u32, incremental: bool) -> Vec<(u32, Vec<Entry>)> {
        if !incremental {
            let free_gen = |num: u32| match self.file.xref().get(num) {
                _ if num == 0 => 65535,
                Some(XrefEntry::Free { generation, .. }) => generation,
                _ => 0,
            };
            let all = (0..size)
                .map(|n| {
                    self.entries.get(&n).copied().unwrap_or(Entry::Free {
                        generation: free_gen(n),
                    })
                })
                .collect();
            return vec![(0, all)];
        }
        let mut rows: Vec<(u32, Vec<Entry>)> = Vec::new();
        for (&n, &e) in &self.entries {
            match rows.last_mut() {
                Some((start, v)) if *start + v.len() as u32 == n => v.push(e),
                _ => rows.push((n, vec![e])),
            }
        }
        rows
    }

    /// Produce everything into `w`.
    pub fn write_all(&mut self, w: &mut impl std::io::Write) -> Result<SaveReport> {
        while let Some(c) = self.next_chunk()? {
            w.write_all(&c).map_err(too_big)?;
        }
        Ok(self.report.clone())
    }
}

/// Live objects the trailer reaches (`/Encrypt` aside), in number order.
fn reachable(f: &PdfFile) -> Vec<ObjId> {
    let mut seen = HashSet::new();
    let mut stack: Vec<ObjId> = Vec::new();
    let refs = |o: &Object, stack: &mut Vec<ObjId>| {
        let mut todo = vec![o];
        while let Some(o) = todo.pop() {
            match o {
                Object::Ref(r) => stack.push(*r),
                Object::Array(a) => todo.extend(a.iter().map(|n| &n.obj)),
                Object::Dict(d) | Object::Stream(Stream { dict: d, .. }) => {
                    todo.extend(d.values().map(|n| &n.obj))
                }
                _ => {}
            }
        }
    };
    for (k, v) in f.trailer() {
        if k != "Encrypt" {
            refs(&v.obj, &mut stack);
        }
    }
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Ok(o) = f.get(id) {
            refs(&o.node.obj, &mut stack);
        }
    }
    let mut ids: Vec<ObjId> = seen.into_iter().filter(|&id| f.get(id).is_ok()).collect();
    ids.sort_unstable();
    ids
}

/// Signature fields with a value (signed), found from the catalog's
/// `/AcroForm /Fields`.
pub fn signature_fields(f: &PdfFile) -> usize {
    let Some(Object::Dict(cat)) = f
        .trailer()
        .get_key("Root")
        .map(|r| f.resolve(r).unwrap_or(Object::Null))
    else {
        return 0;
    };
    let Some(Object::Dict(form)) = cat
        .get_key("AcroForm")
        .map(|r| f.resolve(r).unwrap_or(Object::Null))
    else {
        return 0;
    };
    let mut stack: Vec<Object> = match form
        .get_key("Fields")
        .map(|r| f.resolve(r).unwrap_or(Object::Null))
    {
        Some(Object::Array(a)) => a.into_iter().map(|n| n.obj).collect(),
        _ => return 0,
    };
    let mut seen = HashSet::new();
    let mut n = 0;
    while let Some(o) = stack.pop() {
        if seen.len() > 100_000 {
            break;
        }
        if let Object::Ref(r) = o {
            if !seen.insert(r) {
                continue;
            }
        }
        let Ok(Object::Dict(d)) = f.resolve(&o) else {
            continue;
        };
        if d.get_name("FT").is_some_and(|t| t == "Sig") && d.get_key("V").is_some() {
            n += 1;
        }
        if let Some(Object::Array(k)) = d
            .get_key("Kids")
            .map(|k| f.resolve(k).unwrap_or(Object::Null))
        {
            stack.extend(k.into_iter().map(|n| n.obj));
        }
    }
    n
}
