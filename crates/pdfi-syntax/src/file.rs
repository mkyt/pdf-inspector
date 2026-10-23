//! Reading a PDF file: header, cross-reference chain, lazy object loading,
//! object streams, stream decoding, decryption and recovery.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read};
use std::sync::{Arc, Mutex, OnceLock};

use crate::crypt::{CryptMethod, SecurityHandler};
use crate::diag::{DiagCode, DiagSink, Diagnostic};
use crate::filters::{self, DecodeReader, Decoder, FilterSpec};
use crate::lexer::{Lexer, Tok, is_whitespace};
use crate::object::*;
use crate::parser::{ParseError, ParsedIndirect, Parser};
use crate::scan::{ChunkScanner, rfind_tail};
use crate::source::{ByteSource, RangeReader, read_vec};
use crate::xref::{self, Revision, XrefEntry, XrefKind, XrefTable};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("object {0} not found")]
    Missing(ObjId),
    #[error("object {0}: {1}")]
    Object(ObjId, String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("invalid file: {0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Recovery {
    /// Reconstruct the cross-reference data by scanning when it is unusable.
    #[default]
    Auto,
    Never,
    /// Always scan, ignoring the cross-reference data.
    Force,
}

#[derive(Clone, Debug)]
pub struct OpenOptions {
    pub password: Option<Vec<u8>>,
    pub recovery: Recovery,
    /// Maximum number of parsed objects kept in memory.
    pub object_cache: usize,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            password: None,
            recovery: Recovery::Auto,
            object_cache: 50_000,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Header {
    /// Version from `%PDF-x.y`, if present.
    pub version: Option<String>,
    /// Offset of `%PDF-` (non-zero when the file has leading garbage).
    pub offset: u64,
}

/// Decoded object stream.
#[derive(Debug)]
pub struct ObjStm {
    pub id: ObjId,
    pub data: Arc<[u8]>,
    pub first: usize,
    /// `(object number, offset relative to /First)` in stream order.
    pub entries: Vec<(u32, usize)>,
}

#[derive(Debug, Default)]
pub struct DecodedStream {
    pub data: Vec<u8>,
    /// First filter left undecoded (image codecs or unsupported filters).
    pub stopped_at: Option<Name>,
    pub unsupported: bool,
    pub errors: Vec<String>,
}

const MAX_WINDOW: usize = 256 << 20;
const MAX_OBJSTM: usize = 512 << 20;

struct Cache<V> {
    map: HashMap<ObjId, (V, u64)>,
    tick: u64,
    cap: usize,
}

impl<V: Clone> Cache<V> {
    fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            tick: 0,
            cap: cap.max(16),
        }
    }

    fn get(&mut self, id: ObjId) -> Option<V> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(&id).map(|(v, used)| {
            *used = t;
            v.clone()
        })
    }

    fn insert(&mut self, id: ObjId, v: V) {
        self.tick += 1;
        if self.map.len() >= self.cap {
            // Drop the least recently used half (amortized O(1)).
            let mut ticks: Vec<u64> = self.map.values().map(|(_, t)| *t).collect();
            let mid = ticks.len() / 2;
            let (_, &mut cut, _) = ticks.select_nth_unstable(mid);
            self.map.retain(|_, (_, t)| *t > cut);
        }
        self.map.insert(id, (v, self.tick));
    }
}

/// Changes over the objects of the file: a snapshot made with
/// [`PdfFile::with_edits`] reads these instead of the file.
#[derive(Clone, Debug, Default)]
pub struct Edits {
    /// By object number.
    pub objects: std::collections::BTreeMap<u32, Edited>,
    /// The trailer (`None`: the file's).
    pub trailer: Option<Dict>,
}

#[derive(Clone, Debug)]
pub enum Edited {
    /// Replaced or added. Strings and stream data are not encrypted.
    Object(Arc<IndirectObject>),
    /// Removed; the number is free with this generation.
    Deleted { generation: u16 },
}

impl Edits {
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty() && self.trailer.is_none()
    }
}

/// A PDF file, or a snapshot of it with [`Edits`] applied. Snapshots share
/// the source, the parsed cross-reference data and the object caches.
pub struct PdfFile {
    src: Arc<dyn ByteSource>,
    header: Header,
    revisions: Arc<Vec<Revision>>,
    xref: XrefTable,
    trailer: Dict,
    recovered: bool,
    recovery_mode: Recovery,
    security: Option<SecurityHandler>,
    encrypt_ref: Option<ObjId>,
    cache: Arc<Mutex<Cache<Arc<IndirectObject>>>>,
    objstms: Arc<Mutex<Cache<Arc<ObjStm>>>>,
    /// Object offsets found by scanning, built on first need.
    scan_index: Arc<OnceLock<HashMap<u32, (u64, u16)>>>,
    diags: Arc<DiagSink>,
    /// The file's own table and trailer, under `edits`.
    base: Option<Arc<(XrefTable, Dict)>>,
    edits: Option<Arc<Edits>>,
}

impl std::fmt::Debug for PdfFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PdfFile")
            .field("len", &self.src.len())
            .field("header", &self.header)
            .field("revisions", &self.revisions.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReadMode {
    Normal,
    /// Do not resolve an indirect `/Length` (xref streams: no table yet).
    NoLengthRef,
    /// Scanning for recovery: like `NoLengthRef`, and record no diagnostics.
    Scan,
}

/// Raw object read at an offset, before decryption.
struct RawIndirect {
    id: ObjId,
    node: Node,
    span: Span,
}

impl PdfFile {
    pub fn open(src: Arc<dyn ByteSource>, opts: OpenOptions) -> Result<Self> {
        let diags = DiagSink::default();
        let header = read_header(&*src, &diags)?;
        let mut f = PdfFile {
            src,
            header,
            revisions: Arc::default(),
            xref: XrefTable::default(),
            trailer: Dict::new(),
            recovered: false,
            recovery_mode: opts.recovery,
            security: None,
            encrypt_ref: None,
            cache: Arc::new(Mutex::new(Cache::new(opts.object_cache))),
            objstms: Arc::new(Mutex::new(Cache::new(64))),
            scan_index: Arc::default(),
            diags: Arc::new(diags),
            base: None,
            edits: None,
        };
        let chain = if opts.recovery == Recovery::Force {
            Err(Error::Invalid("recovery forced".into()))
        } else {
            f.load_xref_chain()
        };
        match chain {
            Ok(revs) => {
                f.xref = XrefTable::from_revisions(&revs);
                f.revisions = Arc::new(revs);
                f.trailer = merged_trailer(&f.revisions);
            }
            Err(e) => {
                if opts.recovery == Recovery::Never {
                    return Err(e);
                }
                if opts.recovery != Recovery::Force {
                    f.diags.push(Diagnostic::error(
                        DiagCode::XrefInvalid,
                        format!("cross-reference data unusable: {e}"),
                    ));
                }
                f.recover()?;
            }
        }
        f.setup_security(opts.password.as_deref());
        if !f.recovered && opts.recovery == Recovery::Auto && !f.root_loads() {
            f.diags.push(Diagnostic::error(
                DiagCode::XrefInvalid,
                "document catalog cannot be loaded via cross-reference data",
            ));
            f.recover()?;
            f.setup_security(opts.password.as_deref());
        }
        Ok(f)
    }

    pub fn source(&self) -> &dyn ByteSource {
        &*self.src
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Cross-reference sections, newest first.
    pub fn revisions(&self) -> &[Revision] {
        &self.revisions
    }

    pub fn xref(&self) -> &XrefTable {
        &self.xref
    }

    /// The trailer (with edits applied).
    pub fn trailer(&self) -> &Dict {
        &self.trailer
    }

    /// The edits this snapshot applies over the file.
    pub fn edits(&self) -> Option<&Edits> {
        self.edits.as_deref()
    }

    /// The file's own merged cross-reference table, without edits.
    pub fn file_xref(&self) -> &XrefTable {
        self.base.as_ref().map_or(&self.xref, |b| &b.0)
    }

    /// The file's own trailer, without edits.
    pub fn file_trailer(&self) -> &Dict {
        self.base.as_ref().map_or(&self.trailer, |b| &b.1)
    }

    /// A snapshot of the file with `edits` applied (replacing any edits of
    /// this one). Shares the source and caches with `self`.
    pub fn with_edits(&self, edits: Edits) -> PdfFile {
        let base = self
            .base
            .clone()
            .unwrap_or_else(|| Arc::new((self.xref.clone(), self.trailer.clone())));
        let mut xref = base.0.clone();
        for (&num, e) in &edits.objects {
            xref.set(
                num,
                match e {
                    Edited::Object(o) => XrefEntry::Edited {
                        generation: o.id.generation,
                    },
                    &Edited::Deleted { generation } => XrefEntry::Free {
                        next: 0,
                        generation,
                    },
                },
            );
        }
        let trailer = edits.trailer.clone().unwrap_or_else(|| base.1.clone());
        PdfFile {
            src: self.src.clone(),
            header: self.header.clone(),
            revisions: self.revisions.clone(),
            xref,
            trailer,
            recovered: self.recovered,
            recovery_mode: self.recovery_mode,
            security: self.security.clone(),
            encrypt_ref: self.encrypt_ref,
            cache: self.cache.clone(),
            objstms: self.objstms.clone(),
            scan_index: self.scan_index.clone(),
            diags: self.diags.clone(),
            edits: (!edits.is_empty()).then(|| Arc::new(edits)),
            base: Some(base),
        }
    }

    pub fn is_recovered(&self) -> bool {
        self.recovered
    }

    pub fn security(&self) -> Option<&SecurityHandler> {
        self.security.as_ref()
    }

    pub fn encrypt_ref(&self) -> Option<ObjId> {
        self.encrypt_ref
    }

    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.diags.snapshot()
    }

    pub fn diag_count(&self) -> usize {
        self.diags.len()
    }

    fn root_loads(&self) -> bool {
        match self.trailer.get_key("Root") {
            Some(Object::Ref(id)) => self.get(*id).is_ok_and(|o| o.node.obj.as_dict().is_some()),
            Some(Object::Dict(_)) => true,
            _ => false,
        }
    }

    // ---------------------------------------------------------------- objects

    pub fn get(&self, id: ObjId) -> Result<Arc<IndirectObject>> {
        if let Some(e) = self.edits.as_ref().and_then(|e| e.objects.get(&id.num)) {
            return match e {
                Edited::Object(o) if o.id == id => Ok(o.clone()),
                _ => Err(Error::Missing(id)),
            };
        }
        self.get_depth(id, 0)
    }

    /// The object as stored in the file, ignoring edits.
    pub fn get_original(&self, id: ObjId) -> Result<Arc<IndirectObject>> {
        self.get_depth(id, 0)
    }

    /// Look up by object number using the generation from the xref table.
    pub fn get_num(&self, num: u32) -> Result<Arc<IndirectObject>> {
        let generation = self.xref.get(num).map_or(0, |e| e.generation());
        self.get(ObjId::new(num, generation))
    }

    fn get_depth(&self, id: ObjId, depth: u32) -> Result<Arc<IndirectObject>> {
        if let Some(o) = self.lock_cache().get(id) {
            return Ok(o);
        }
        if depth > 16 {
            return Err(Error::Object(id, "reference chain too deep".into()));
        }
        let obj = match self.file_xref().get(id.num) {
            Some(XrefEntry::InUse { offset, generation }) => {
                if generation != id.generation {
                    return Err(Error::Missing(id));
                }
                if offset == 0 {
                    // A common writer bug; treated as a null reference like other readers do.
                    self.diags.push(
                        Diagnostic::warning(
                            DiagCode::XrefEntryMismatch,
                            "in-use entry with offset 0",
                        )
                        .with_obj(id),
                    );
                    return Err(Error::Missing(id));
                }
                self.load_in_use(id, offset, depth)?
            }
            Some(XrefEntry::Compressed { stream, index }) => {
                if id.generation != 0 {
                    return Err(Error::Missing(id));
                }
                self.load_compressed(id, stream, index, depth)?
            }
            Some(XrefEntry::Free { .. } | XrefEntry::Edited { .. }) | None => {
                return Err(Error::Missing(id));
            }
        };
        let obj = Arc::new(obj);
        self.lock_cache().insert(id, obj.clone());
        Ok(obj)
    }

    fn lock_cache(&self) -> std::sync::MutexGuard<'_, Cache<Arc<IndirectObject>>> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn load_in_use(&self, id: ObjId, offset: u64, depth: u32) -> Result<IndirectObject> {
        let mut candidates = vec![offset];
        if self.header.offset > 0 {
            candidates.push(offset + self.header.offset);
        }
        let mut last_err = None;
        for (i, &off) in candidates.iter().enumerate() {
            match self.read_indirect_at(off, depth, ReadMode::Normal) {
                Ok(raw) if raw.id == id => {
                    if i > 0 {
                        self.diags.push(
                            Diagnostic::warning(
                                DiagCode::XrefOffsetAdjusted,
                                "object offset is relative to the header",
                            )
                            .with_obj(id),
                        );
                    }
                    return Ok(self.finish_loaded(raw));
                }
                Ok(raw) => last_err = Some(format!("xref points to object {} instead", raw.id)),
                Err(e) => last_err = Some(e.to_string()),
            }
        }
        let msg = last_err.unwrap_or_default();
        // Fall back to the object position found by scanning the file.
        if self.recovery_mode != Recovery::Never {
            if let Some(&(off, generation)) = self.scan_index().get(&id.num) {
                if generation == id.generation {
                    if let Ok(raw) = self.read_indirect_at(off, depth, ReadMode::Normal) {
                        if raw.id == id {
                            self.diags.push(
                                Diagnostic::warning(
                                    DiagCode::XrefEntryMismatch,
                                    format!("wrong xref offset {offset} ({msg}); found at {off} by scanning"),
                                )
                                .with_obj(id),
                            );
                            return Ok(self.finish_loaded(raw));
                        }
                    }
                }
            }
        }
        self.diags
            .push(Diagnostic::error(DiagCode::XrefEntryMismatch, msg.clone()).with_obj(id));
        Err(Error::Object(id, msg))
    }

    fn finish_loaded(&self, raw: RawIndirect) -> IndirectObject {
        let mut node = raw.node;
        if Some(raw.id) != self.encrypt_ref {
            if let Some(sec) = self.security.as_ref().filter(|s| s.is_authenticated()) {
                let is_xref = node.obj.as_stream().is_some_and(|s| s.dict.type_is("XRef"));
                if !is_xref {
                    decrypt_strings(sec, raw.id, &mut node);
                }
            }
        }
        IndirectObject {
            id: raw.id,
            node,
            location: ObjLocation::File(raw.span),
        }
    }

    fn load_compressed(
        &self,
        id: ObjId,
        stream: u32,
        index: u32,
        depth: u32,
    ) -> Result<IndirectObject> {
        let os = self.objstm(stream, depth)?;
        let pos = match os.entries.get(index as usize) {
            Some(&(n, _)) if n == id.num => index as usize,
            _ => os
                .entries
                .iter()
                .position(|&(n, _)| n == id.num)
                .ok_or_else(|| Error::Object(id, format!("not found in object stream {stream}")))?,
        };
        let start = os.first + os.entries[pos].1;
        let end = os
            .entries
            .get(pos + 1)
            .map_or(os.data.len(), |&(_, o)| os.first + o)
            .max(start);
        if start > os.data.len() {
            return Err(Error::Object(
                id,
                "object stream offset out of range".into(),
            ));
        }
        let end = end.min(os.data.len());
        let mut p = Parser::new(&os.data[start..end], start as u64, true);
        let node = p
            .parse_object()
            .map_err(|e| Error::Object(id, e.to_string()))?;
        self.diags
            .extend(p.take_diags().into_iter().map(|d| d.with_obj(id)));
        Ok(IndirectObject {
            id,
            node,
            location: ObjLocation::ObjStm {
                stream: os.id,
                index: pos as u32,
            },
        })
    }

    pub fn objstm(&self, num: u32, depth: u32) -> Result<Arc<ObjStm>> {
        let generation = self.file_xref().get(num).map_or(0, |e| e.generation());
        let sid = ObjId::new(num, generation);
        if let Some(o) = self
            .objstms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(sid)
        {
            return Ok(o);
        }
        let obj = self.get_depth(sid, depth + 1)?;
        let Object::Stream(s) = &obj.node.obj else {
            return Err(Error::Object(sid, "object stream is not a stream".into()));
        };
        let dec = self.decode_stream(sid, s, Some(MAX_OBJSTM));
        if !dec.errors.is_empty() {
            self.diags.push(
                Diagnostic::warning(
                    DiagCode::ObjStmInvalid,
                    format!("object stream decoding: {}", dec.errors.join("; ")),
                )
                .with_obj(sid),
            );
        }
        let n = s.dict.get_i64("N").unwrap_or(0).max(0) as usize;
        let first = s.dict.get_i64("First").unwrap_or(0).max(0) as usize;
        let data: Arc<[u8]> = dec.data.into();
        let mut lex = Lexer::new_complete(&data[..first.min(data.len())], 0);
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            match (lex.next_token(), lex.next_token()) {
                (Ok(a), Ok(b)) => match (a.tok, b.tok) {
                    (Tok::Int(num), Tok::Int(off)) if num >= 0 && off >= 0 => {
                        entries.push((num as u32, off as usize));
                    }
                    _ => break,
                },
                _ => break,
            }
        }
        if entries.len() != n {
            self.diags.push(
                Diagnostic::warning(
                    DiagCode::ObjStmInvalid,
                    format!(
                        "object stream header lists {} of {n} objects",
                        entries.len()
                    ),
                )
                .with_obj(sid),
            );
        }
        let os = Arc::new(ObjStm {
            id: sid,
            data,
            first,
            entries,
        });
        self.objstms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(sid, os.clone());
        Ok(os)
    }

    /// Follow references (missing objects resolve to null, as the spec says).
    pub fn resolve(&self, obj: &Object) -> Result<Object> {
        let mut cur = obj.clone();
        for _ in 0..32 {
            match cur {
                Object::Ref(id) => match self.get(id) {
                    Ok(o) => cur = o.node.obj.clone(),
                    Err(Error::Missing(_)) => return Ok(Object::Null),
                    Err(e) => return Err(e),
                },
                other => return Ok(other),
            }
        }
        Err(Error::Parse("reference cycle".into()))
    }

    // ---------------------------------------------------------------- reading

    fn emit(&self, mode: ReadMode, d: Diagnostic) {
        if mode != ReadMode::Scan {
            self.diags.push(d);
        }
    }

    /// Parse `N G obj ... endobj` at `offset`, determining stream extents.
    fn read_indirect_at(&self, offset: u64, depth: u32, mode: ReadMode) -> Result<RawIndirect> {
        let len = self.src.len();
        if offset >= len {
            return Err(Error::Parse(format!("offset {offset} beyond end of file")));
        }
        let mut win = 4096usize;
        loop {
            let buf = read_vec(&*self.src, offset, win)?;
            let complete = offset + buf.len() as u64 >= len;
            let mut p = Parser::new(&buf, offset, complete);
            match p.parse_indirect() {
                Ok(pi) => {
                    let id = pi.id;
                    for d in p.take_diags() {
                        self.emit(mode, d.with_obj(id));
                    }
                    return self.finish_indirect(pi, depth, mode);
                }
                Err(ParseError::Eof) if !complete && win < MAX_WINDOW => win *= 4,
                Err(e) => return Err(Error::Parse(e.to_string())),
            }
        }
    }

    fn finish_indirect(
        &self,
        pi: ParsedIndirect,
        depth: u32,
        mode: ReadMode,
    ) -> Result<RawIndirect> {
        let id = pi.id;
        let Some(data_start) = pi.stream_data_start else {
            return Ok(RawIndirect {
                id,
                node: pi.node,
                span: Span::new(pi.start, pi.end),
            });
        };
        let dict_span = pi.node.span.unwrap_or_default();
        let Object::Dict(dict) = pi.node.obj else {
            unreachable!("parser checked stream dict")
        };
        let len = self.src.len();
        let length = match dict.get_key("Length") {
            Some(Object::Integer(n)) => Some(*n),
            Some(Object::Ref(r)) if mode == ReadMode::Normal && *r != id => self
                .get_depth(*r, depth + 1)
                .ok()
                .and_then(|o| o.node.obj.as_i64()),
            _ => None,
        };
        let mut sc = ChunkScanner::new(&*self.src);
        let mut data_end = None;
        let mut kw_end = 0;
        if let Some(l) = length.filter(|&l| l >= 0 && data_start + l as u64 <= len) {
            let end = data_start + l as u64;
            let (base, bytes) = sc.around(end, 0, 64)?;
            let mut i = 0;
            while i < bytes.len() && is_whitespace(bytes[i]) {
                i += 1;
            }
            if bytes[i..].starts_with(b"endstream") {
                data_end = Some(end);
                kw_end = base + (i + 9) as u64;
            }
        }
        if data_end.is_none() {
            let found = sc.find(data_start, b"endstream")?;
            let (end, kend) = match found {
                Some(p) => {
                    let (base, before) = sc.around(p, 2, 0)?;
                    let mut e = p;
                    if before.ends_with(b"\r\n") && p - base >= 2 {
                        e -= 2;
                    } else if before.ends_with(b"\n") || before.ends_with(b"\r") {
                        e -= 1;
                    }
                    (e.max(data_start), p + 9)
                }
                None => {
                    self.emit(
                        mode,
                        Diagnostic::error(DiagCode::MissingEndstream, "missing 'endstream'")
                            .with_obj(id),
                    );
                    (len, len)
                }
            };
            self.emit(
                mode,
                Diagnostic::warning(
                    DiagCode::StreamLengthInvalid,
                    match length {
                        Some(l) => format!(
                            "stream /Length {l} is wrong; actual data length {}",
                            end - data_start
                        ),
                        None => format!(
                            "stream /Length missing or unresolvable; actual data length {}",
                            end - data_start
                        ),
                    },
                )
                .with_obj(id)
                .with_span(Span::new(data_start, end)),
            );
            data_end = Some(end);
            kw_end = kend;
        }
        let data_end = data_end.unwrap_or(data_start);
        // Expect `endobj` after `endstream`.
        let (base, bytes) = sc.around(kw_end, 0, 64)?;
        let mut i = 0;
        while i < bytes.len() && is_whitespace(bytes[i]) {
            i += 1;
        }
        let obj_end = if bytes[i..].starts_with(b"endobj") {
            base + (i + 6) as u64
        } else {
            self.emit(
                mode,
                Diagnostic::warning(DiagCode::MissingEndobj, "missing 'endobj' after stream")
                    .with_obj(id),
            );
            kw_end
        };
        let stream = Stream {
            dict,
            data: StreamData::Source(Span::new(data_start, data_end)),
        };
        let node = Node::new(Object::Stream(stream), Span::new(dict_span.start, kw_end));
        Ok(RawIndirect {
            id,
            node,
            span: Span::new(pi.start, obj_end),
        })
    }

    // ---------------------------------------------------------------- streams

    /// Stream data as stored in the file (still encrypted and encoded).
    pub fn raw_reader<'a>(&'a self, s: &'a Stream) -> Box<dyn Read + Send + 'a> {
        match &s.data {
            StreamData::Source(span) => {
                Box::new(RangeReader::new(&*self.src, span.start, span.end))
            }
            StreamData::Owned(d) => Box::new(io::Cursor::new(d.clone())),
        }
    }

    pub fn raw_data(&self, s: &Stream, limit: Option<usize>) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let r = self.raw_reader(s);
        match limit {
            Some(l) => r.take(l as u64).read_to_end(&mut out)?,
            None => {
                let mut r = r;
                r.read_to_end(&mut out)?
            }
        };
        Ok(out)
    }

    /// Filter list with indirect `/Filter` / `/DecodeParms` values resolved.
    pub fn filter_specs(&self, dict: &Dict) -> Vec<FilterSpec> {
        let mut d = Dict::new();
        for key in ["Filter", "F", "DecodeParms", "DP"] {
            if let Some(n) = dict.get(key.as_bytes()) {
                let mut v = self.resolve(&n.obj).unwrap_or(Object::Null);
                if let Object::Array(items) = &mut v {
                    for it in items.iter_mut() {
                        it.obj = self.resolve(&it.obj).unwrap_or(Object::Null);
                    }
                }
                if let Object::Dict(pd) = &mut v {
                    for it in pd.values_mut() {
                        if matches!(it.obj, Object::Ref(_)) {
                            it.obj = self.resolve(&it.obj).unwrap_or(Object::Null);
                        }
                    }
                }
                d.insert(Name::from(key), Node::synthetic(v));
            }
        }
        // `/F` in a stream dictionary means an external file; the
        // abbreviation is only valid in inline images.
        if dict.contains_key(b"Filter".as_slice()) {
            d.shift_remove(b"F".as_slice());
            d.shift_remove(b"DP".as_slice());
        }
        filters::filter_specs(&d)
    }

    /// Decryption stage for stream `id`, if the file is encrypted.
    pub fn crypt_decoder(
        &self,
        id: ObjId,
        s: &Stream,
        specs: &[FilterSpec],
    ) -> Option<Box<dyn Decoder>> {
        let sec = self.security.as_ref()?;
        // Data held in memory comes from edits, which are not encrypted.
        if matches!(s.data, StreamData::Owned(_)) {
            return None;
        }
        if Some(id) == self.encrypt_ref || s.dict.type_is("XRef") {
            return None;
        }
        if s.dict.type_is("Metadata") && !sec.info.encrypt_metadata {
            return None;
        }
        let method = specs
            .iter()
            .find(|f| f.name == "Crypt")
            .map(|f| {
                let name = f
                    .parms
                    .as_ref()
                    .and_then(|p| p.get_name("Name"))
                    .cloned()
                    .unwrap_or_else(|| Name::from("Identity"));
                sec.method_for_filter(&name)
            })
            .unwrap_or(sec.info.stream_method);
        if method == CryptMethod::None {
            return None;
        }
        sec.stream_decoder(id, method)
    }

    /// Reader over decrypted and decoded data. Also returns the filter at
    /// which decoding stopped, if any.
    pub fn decoded_reader<'a>(
        &'a self,
        id: ObjId,
        s: &'a Stream,
    ) -> (DecodeReader<Box<dyn Read + Send + 'a>>, filters::Chain) {
        let specs = self.filter_specs(&s.dict);
        let mut chain = filters::build_chain(&specs);
        let mut decoders = std::mem::take(&mut chain.decoders);
        if let Some(c) = self.crypt_decoder(id, s, &specs) {
            decoders.insert(0, c);
        }
        (DecodeReader::new(self.raw_reader(s), decoders), chain)
    }

    /// Decrypted but still filter-encoded data (what a decrypting writer emits).
    pub fn decrypted_reader<'a>(
        &'a self,
        id: ObjId,
        s: &'a Stream,
    ) -> DecodeReader<Box<dyn Read + Send + 'a>> {
        let specs = self.filter_specs(&s.dict);
        let decoders = self.crypt_decoder(id, s, &specs).into_iter().collect();
        DecodeReader::new(self.raw_reader(s), decoders)
    }

    pub fn decode_stream(&self, id: ObjId, s: &Stream, limit: Option<usize>) -> DecodedStream {
        let (mut r, chain) = self.decoded_reader(id, s);
        let mut data = Vec::new();
        let res = match limit {
            Some(l) => (&mut r).take(l as u64).read_to_end(&mut data),
            None => r.read_to_end(&mut data),
        };
        let mut errors = r.errors().to_vec();
        if let Err(e) = res {
            errors.push(e.to_string());
        }
        if self
            .security
            .as_ref()
            .is_some_and(|s| !s.is_authenticated())
        {
            errors.push("file is encrypted and no valid password was given".into());
        }
        DecodedStream {
            data,
            stopped_at: chain.stopped_at,
            unsupported: chain.unsupported,
            errors,
        }
    }

    // ---------------------------------------------------------------- xref

    fn find_startxref(&self) -> Result<u64> {
        let pos = match rfind_tail(&*self.src, b"startxref", 4096)? {
            Some(p) => p,
            None => rfind_tail(&*self.src, b"startxref", 1 << 20)?
                .ok_or_else(|| Error::Invalid("'startxref' not found".into()))?,
        };
        let buf = read_vec(&*self.src, pos + 9, 64)?;
        let mut lex = Lexer::new_complete(&buf, pos + 9);
        match lex.next_token().map(|t| t.tok) {
            Ok(Tok::Int(n)) if n >= 0 => Ok(n as u64),
            _ => Err(Error::Invalid("invalid 'startxref' value".into())),
        }
    }

    fn load_xref_chain(&self) -> Result<Vec<Revision>> {
        let start = self.find_startxref().inspect_err(|_| {
            self.diags.push(Diagnostic::error(
                DiagCode::StartXrefMissing,
                "'startxref' not found",
            ));
        })?;
        let mut revs = Vec::new();
        let mut visited = HashSet::new();
        let mut off = start;
        loop {
            if !visited.insert(off) {
                self.diags.push(Diagnostic::warning(
                    DiagCode::XrefLoop,
                    format!("/Prev loop at offset {off}"),
                ));
                break;
            }
            let rev = match self.load_section(off) {
                Ok(r) => r,
                Err(e) if revs.is_empty() => return Err(e),
                Err(e) => {
                    self.diags.push(Diagnostic::error(
                        DiagCode::XrefInvalid,
                        format!("previous xref section at {off}: {e}"),
                    ));
                    break;
                }
            };
            let prev = rev.trailer.get_i64("Prev");
            revs.push(rev);
            match prev {
                Some(p) if p >= 0 => off = p as u64,
                _ => break,
            }
        }
        Ok(revs)
    }

    fn load_section(&self, off: u64) -> Result<Revision> {
        let mut candidates = vec![off];
        if self.header.offset > 0 {
            candidates.push(off + self.header.offset);
        }
        let mut err = None;
        for (i, &c) in candidates.iter().enumerate() {
            match self.load_section_at(c) {
                Ok(r) => {
                    if i > 0 {
                        self.diags.push(Diagnostic::warning(
                            DiagCode::XrefOffsetAdjusted,
                            "xref offset is relative to the header",
                        ));
                    }
                    return Ok(r);
                }
                Err(e) => err = Some(e),
            }
        }
        // Look for a nearby `xref` keyword (offsets slightly off are common).
        let near = off.saturating_sub(1024);
        let mut sc = ChunkScanner::new(&*self.src);
        if let Some(p) = sc.find(near, b"xref")? {
            if p.abs_diff(off) <= 1024 && !candidates.contains(&p) {
                if let Ok(r) = self.load_section_at(p) {
                    self.diags.push(Diagnostic::warning(
                        DiagCode::XrefOffsetAdjusted,
                        format!("xref section expected at {off}, found at {p}"),
                    ));
                    return Ok(r);
                }
            }
        }
        Err(err.unwrap_or_else(|| Error::Invalid("xref section not found".into())))
    }

    fn load_section_at(&self, off: u64) -> Result<Revision> {
        let head = read_vec(&*self.src, off, 32)?;
        let skip = head.iter().take_while(|b| is_whitespace(**b)).count();
        if head[skip..].starts_with(b"xref") {
            self.load_table(off + skip as u64)
        } else {
            self.load_xref_stream(off)
        }
    }

    fn load_table(&self, off: u64) -> Result<Revision> {
        let len = self.src.len();
        let mut win = 64 * 1024usize;
        loop {
            let buf = read_vec(&*self.src, off, win)?;
            let complete = off + buf.len() as u64 >= len;
            let mut lex = if complete {
                Lexer::new_complete(&buf, off)
            } else {
                Lexer::new(&buf, off)
            };
            lex.set_pos(4);
            let res = xref::parse_table(&mut lex).and_then(|(entries, warns)| {
                match lex.next_token()?.tok {
                    Tok::Keyword(b"trailer") => {}
                    _ => return Ok(Err("'trailer' not found after xref table".to_string())),
                }
                let mut p = Parser::new(&buf[lex.pos()..], lex.abs_pos(), complete);
                match p.parse_object() {
                    Ok(Node {
                        obj: Object::Dict(d),
                        span,
                    }) => {
                        self.diags.extend(p.take_diags());
                        Ok(Ok((entries, warns, d, span.map_or(off, |s| s.end))))
                    }
                    Ok(_) => Ok(Err("trailer is not a dictionary".to_string())),
                    Err(ParseError::Eof) => Err(crate::lexer::LexError::Eof),
                    Err(e) => Ok(Err(e.to_string())),
                }
            });
            match res {
                Ok(Ok((entries, warns, trailer, end))) => {
                    for w in warns {
                        self.diags.push(
                            Diagnostic::warning(DiagCode::XrefInvalid, w)
                                .with_span(Span::new(off, end)),
                        );
                    }
                    let mut rev = Revision {
                        kind: XrefKind::Table,
                        offset: off,
                        span: Span::new(off, end),
                        trailer,
                        entries,
                        stream_entries: Vec::new(),
                        xref_stream: None,
                    };
                    if let Some(xs) = rev.trailer.get_i64("XRefStm") {
                        match self.load_xref_stream(xs.max(0) as u64) {
                            Ok(s) => {
                                rev.kind = XrefKind::Hybrid;
                                rev.stream_entries = s.entries;
                                rev.xref_stream = s.xref_stream;
                            }
                            Err(e) => self.diags.push(Diagnostic::warning(
                                DiagCode::XrefInvalid,
                                format!("/XRefStm: {e}"),
                            )),
                        }
                    }
                    return Ok(rev);
                }
                Ok(Err(msg)) => return Err(Error::Invalid(msg)),
                Err(_) if !complete && win < MAX_WINDOW => win *= 4,
                Err(_) => return Err(Error::Invalid("truncated xref table".into())),
            }
        }
    }

    fn load_xref_stream(&self, off: u64) -> Result<Revision> {
        let raw = self.read_indirect_at(off, 0, ReadMode::NoLengthRef)?;
        let Object::Stream(s) = &raw.node.obj else {
            return Err(Error::Invalid(format!(
                "no xref table or stream at offset {off}"
            )));
        };
        if !s.dict.type_is("XRef") {
            if s.dict.get_key("W").is_none() {
                return Err(Error::Invalid(format!(
                    "object at {off} is not an xref stream"
                )));
            }
            self.diags.push(
                Diagnostic::warning(DiagCode::XrefInvalid, "xref stream without /Type /XRef")
                    .with_obj(raw.id),
            );
        }
        let specs = filters::filter_specs(&s.dict);
        let chain = filters::build_chain(&specs);
        let raw_data = self.raw_data(s, None)?;
        let (data, errs) = filters::decode_all(&raw_data, chain.decoders, None);
        for e in errs {
            self.diags
                .push(Diagnostic::warning(DiagCode::FilterError, e).with_obj(raw.id));
        }
        let (entries, warns) =
            xref::parse_stream_entries(&s.dict, &data).map_err(Error::Invalid)?;
        for w in warns {
            self.diags
                .push(Diagnostic::warning(DiagCode::XrefInvalid, w).with_obj(raw.id));
        }
        Ok(Revision {
            kind: XrefKind::Stream,
            offset: off,
            span: raw.span,
            trailer: s.dict.clone(),
            entries,
            stream_entries: Vec::new(),
            xref_stream: Some(raw.id),
        })
    }

    // ---------------------------------------------------------------- recovery

    fn scan_index(&self) -> &HashMap<u32, (u64, u16)> {
        self.scan_index.get_or_init(|| {
            let found = self.scan_objects().unwrap_or_default();
            found
                .objects
                .into_iter()
                .map(|(id, off)| (id.num, (off, id.generation)))
                .collect()
        })
    }

    fn scan_objects(&self) -> Result<ScanResult> {
        let mut res = ScanResult::default();
        let mut sc = ChunkScanner::new(&*self.src);
        let mut pos = self.header.offset;
        while let Some((p, which)) = sc.find_any(pos, &[b"obj", b"trailer"])? {
            pos = p + 1;
            if which == 1 {
                if let Ok(Some(d)) = self.parse_dict_after(p + 7) {
                    res.trailers.push(d);
                }
                continue;
            }
            let Some(start) = header_start(&mut sc, p)? else {
                continue;
            };
            let Ok(raw) = self.read_indirect_at(start, 0, ReadMode::Scan) else {
                continue;
            };
            if let Object::Stream(s) = &raw.node.obj {
                if s.dict.type_is("ObjStm") {
                    res.objstms.push(raw.id);
                }
                if s.dict.type_is("XRef") {
                    res.trailers.push(s.dict.clone());
                }
            } else if raw.node.obj.as_dict().is_some_and(|d| d.type_is("Catalog")) {
                res.catalogs.push(raw.id);
            }
            res.objects.push((raw.id, start));
            pos = raw.span.end.max(pos);
        }
        Ok(res)
    }

    fn parse_dict_after(&self, off: u64) -> Result<Option<Dict>> {
        let buf = read_vec(&*self.src, off, 64 * 1024)?;
        let complete = off + buf.len() as u64 >= self.src.len();
        let mut p = Parser::new(&buf, off, complete);
        Ok(match p.parse_object() {
            Ok(Node {
                obj: Object::Dict(d),
                ..
            }) => Some(d),
            _ => None,
        })
    }

    fn recover(&mut self) -> Result<()> {
        // Drop anything loaded through the broken table.
        let cap = self.lock_cache().cap;
        *self.lock_cache() = Cache::new(cap);
        *self.objstms.lock().unwrap_or_else(|e| e.into_inner()) = Cache::new(64);
        let scan = self.scan_objects()?;
        let mut table = XrefTable::default();
        let mut entries = Vec::new();
        // Later definitions win, like incremental updates.
        for &(id, off) in &scan.objects {
            table.set(
                id.num,
                XrefEntry::InUse {
                    offset: off,
                    generation: id.generation,
                },
            );
        }
        self.xref = table;
        // Objects in object streams, unless also stored directly.
        for &sid in &scan.objstms {
            if let Ok(os) = self.objstm(sid.num, 0) {
                for (i, &(num, _)) in os.entries.iter().enumerate() {
                    if self.xref.get(num).is_none() {
                        self.xref.set(
                            num,
                            XrefEntry::Compressed {
                                stream: sid.num,
                                index: i as u32,
                            },
                        );
                    }
                }
            }
        }
        entries.extend(self.xref.iter());
        let mut trailer = scan
            .trailers
            .iter()
            .rev()
            .find(|t| t.get_key("Root").is_some())
            .cloned()
            .unwrap_or_default();
        if trailer.get_key("Root").is_none() {
            if let Some(&cat) = scan.catalogs.last() {
                trailer.insert(Name::from("Root"), Node::synthetic(Object::Ref(cat)));
            }
        }
        // Keep /Info, /ID, /Encrypt from any trailer when the chosen one lacks them.
        for key in ["Info", "ID", "Encrypt"] {
            if trailer.get_key(key).is_none() {
                if let Some(v) = scan
                    .trailers
                    .iter()
                    .rev()
                    .find_map(|t| t.get(key.as_bytes()))
                {
                    trailer.insert(Name::from(key), v.clone());
                }
            }
        }
        trailer.shift_remove(b"Prev".as_slice());
        trailer.shift_remove(b"XRefStm".as_slice());
        self.diags.push(Diagnostic::warning(
            DiagCode::RecoveryUsed,
            format!(
                "cross-reference data reconstructed by scanning ({} objects)",
                entries.len()
            ),
        ));
        let len = self.src.len();
        Arc::make_mut(&mut self.revisions).insert(
            0,
            Revision {
                kind: XrefKind::Recovered,
                offset: 0,
                span: Span::new(0, len),
                trailer: trailer.clone(),
                entries,
                stream_entries: Vec::new(),
                xref_stream: None,
            },
        );
        self.trailer = trailer;
        self.recovered = true;
        let _ = self.scan_index.set(
            scan.objects
                .iter()
                .map(|&(id, off)| (id.num, (off, id.generation)))
                .collect(),
        );
        Ok(())
    }

    // ---------------------------------------------------------------- security

    fn setup_security(&mut self, password: Option<&[u8]>) {
        self.security = None;
        self.encrypt_ref = None;
        let enc = match self.trailer.get_key("Encrypt") {
            None | Some(Object::Null) => return,
            Some(Object::Ref(id)) => {
                self.encrypt_ref = Some(*id);
                match self.get(*id) {
                    Ok(o) => o.node.obj.clone(),
                    Err(e) => {
                        self.diags.push(Diagnostic::error(
                            DiagCode::EncryptionError,
                            format!("/Encrypt: {e}"),
                        ));
                        return;
                    }
                }
            }
            Some(o) => o.clone(),
        };
        let Object::Dict(mut enc) = enc else {
            self.diags.push(Diagnostic::error(
                DiagCode::EncryptionError,
                "/Encrypt is not a dictionary",
            ));
            return;
        };
        for v in enc.values_mut() {
            if matches!(v.obj, Object::Ref(_)) {
                v.obj = self.resolve(&v.obj).unwrap_or(Object::Null);
            }
        }
        let id0 = match self.trailer.get_key("ID") {
            Some(Object::Array(a)) => a
                .first()
                .and_then(|n| n.obj.as_string())
                .map(|s| s.bytes.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        match SecurityHandler::new(&enc, &id0) {
            Ok(mut h) => {
                let ok = match password {
                    Some(p) => h.authenticate(p).is_some() || h.authenticate(b"").is_some(),
                    None => h.authenticate(b"").is_some(),
                };
                if !ok {
                    self.diags.push(Diagnostic::error(
                        DiagCode::EncryptionError,
                        "password required",
                    ));
                }
                self.security = Some(h);
                // Objects loaded before the key was known (e.g. /Root check) must be reloaded.
                let cap = self.lock_cache().cap;
                *self.lock_cache() = Cache::new(cap);
                *self.objstms.lock().unwrap_or_else(|e| e.into_inner()) = Cache::new(64);
            }
            Err(e) => self
                .diags
                .push(Diagnostic::error(DiagCode::EncryptionError, e)),
        }
    }
}

#[derive(Default)]
struct ScanResult {
    objects: Vec<(ObjId, u64)>,
    trailers: Vec<Dict>,
    objstms: Vec<ObjId>,
    catalogs: Vec<ObjId>,
}

/// Given the offset of an `obj` keyword, find the start of `N G obj`.
fn header_start(sc: &mut ChunkScanner<'_>, obj_kw: u64) -> io::Result<Option<u64>> {
    let (base, b) = sc.around(obj_kw, 32, 4)?;
    let kw = (obj_kw - base) as usize;
    if b.get(kw + 3).is_some_and(|&c| crate::lexer::is_regular(c)) {
        return Ok(None);
    }
    let mut i = kw;
    let skip_ws = |i: &mut usize| {
        let s = *i;
        while *i > 0 && is_whitespace(b[*i - 1]) {
            *i -= 1;
        }
        *i < s
    };
    let digits = |i: &mut usize| {
        let s = *i;
        while *i > 0 && b[*i - 1].is_ascii_digit() {
            *i -= 1;
        }
        *i < s
    };
    if !(skip_ws(&mut i) && digits(&mut i) && skip_ws(&mut i) && digits(&mut i)) {
        return Ok(None);
    }
    if i > 0 && crate::lexer::is_regular(b[i - 1]) {
        return Ok(None);
    }
    Ok(Some(base + i as u64))
}

fn read_header(src: &dyn ByteSource, diags: &DiagSink) -> Result<Header> {
    let buf = read_vec(src, 0, 1024)?;
    match memchr::memmem::find(&buf, b"%PDF-") {
        Some(p) => {
            let v: Vec<u8> = buf[p + 5..]
                .iter()
                .take_while(|b| b.is_ascii_digit() || **b == b'.')
                .copied()
                .collect();
            if p > 0 {
                diags.push(Diagnostic::warning(
                    DiagCode::HeaderOffset,
                    format!("{p} bytes before %PDF- header"),
                ));
            }
            Ok(Header {
                version: Some(String::from_utf8_lossy(&v).into_owned()),
                offset: p as u64,
            })
        }
        None => {
            diags.push(Diagnostic::error(
                DiagCode::HeaderMissing,
                "%PDF- header not found",
            ));
            Ok(Header {
                version: None,
                offset: 0,
            })
        }
    }
}

fn merged_trailer(revs: &[Revision]) -> Dict {
    let mut t = revs.first().map(|r| r.trailer.clone()).unwrap_or_default();
    // Keys missing from the newest trailer are taken from older ones.
    for r in revs.iter().skip(1) {
        for key in ["Root", "Info", "ID", "Encrypt"] {
            if !t.contains_key(key.as_bytes()) {
                if let Some(v) = r.trailer.get(key.as_bytes()) {
                    t.insert(Name::from(key), v.clone());
                }
            }
        }
    }
    t
}

fn decrypt_strings(sec: &SecurityHandler, id: ObjId, node: &mut Node) {
    match &mut node.obj {
        Object::String(s) => {
            if let Some(d) = sec.decrypt_string(id, &s.bytes) {
                s.bytes = d;
            }
        }
        Object::Array(a) => a.iter_mut().for_each(|n| decrypt_strings(sec, id, n)),
        Object::Dict(d) => d.values_mut().for_each(|n| decrypt_strings(sec, id, n)),
        Object::Stream(s) => s
            .dict
            .values_mut()
            .for_each(|n| decrypt_strings(sec, id, n)),
        _ => {}
    }
}
