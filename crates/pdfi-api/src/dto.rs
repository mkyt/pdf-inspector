//! Data transfer objects: what the frontends see of a document.
//!
//! Every list is returned through [`Page`] with an explicit [`Range`]; nothing
//! returns "everything".

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub type DocId = u32;
pub type JobId = u32;
pub type RequestId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct ObjRef {
    pub num: u32,
    pub generation: u16,
}

impl From<pdfi_syntax::ObjId> for ObjRef {
    fn from(id: pdfi_syntax::ObjId) -> Self {
        Self {
            num: id.num,
            generation: id.generation,
        }
    }
}

impl From<ObjRef> for pdfi_syntax::ObjId {
    fn from(r: ObjRef) -> Self {
        Self::new(r.num, r.generation)
    }
}

/// Byte range `[start, end)` of the source file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Span {
    pub start: u64,
    pub end: u64,
}

impl From<pdfi_syntax::Span> for Span {
    fn from(s: pdfi_syntax::Span) -> Self {
        Self {
            start: s.start,
            end: s.end,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Range {
    pub offset: u64,
    pub limit: u32,
}

/// One slice of a list. `total` is the length of the whole list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Page<T> {
    pub total: u64,
    pub offset: u64,
    pub items: Vec<T>,
}

// ------------------------------------------------------------------ files

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OpenSource {
    /// A path on the machine running the backend (server mode). Subject to
    /// the backend's access policy.
    Path { path: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DirListing {
    /// `None` for the list of allowed roots.
    pub path: Option<String>,
    pub parent: Option<String>,
    pub entries: Vec<DirEntry>,
}

// ------------------------------------------------------------------ documents

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DocInfo {
    pub doc: DocId,
    pub name: String,
    pub path: Option<String>,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DocSummary {
    pub info: DocInfo,
    pub version: Option<String>,
    pub header_offset: u64,
    /// Number of object numbers covered by the cross-reference data.
    pub object_slots: u32,
    /// Rows returned by `Objects` (entries present in the xref, free included).
    pub xref_entries: u64,
    pub live_objects: u64,
    pub recovered: bool,
    pub revisions: Vec<RevisionInfo>,
    pub root: Option<ObjRef>,
    pub info_dict: Option<ObjRef>,
    pub encryption: Option<EncryptionInfo>,
    pub diagnostics: u64,
    /// Effective version: the header's, or the catalog's `/Version` if later.
    pub pdf_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct EncryptionInfo {
    pub filter: String,
    pub v: i64,
    pub r: i64,
    pub key_bits: u32,
    pub stream_method: String,
    pub string_method: String,
    pub permissions: i32,
    pub authenticated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum XrefKind {
    Table,
    Stream,
    Hybrid,
    Recovered,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct RevisionInfo {
    /// 0 = newest.
    pub index: u32,
    pub kind: XrefKind,
    pub offset: u64,
    pub span: Span,
    pub entries: u64,
    pub xref_stream: Option<ObjRef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum XrefEntry {
    Free { next: u32, generation: u16 },
    InUse { offset: u64, generation: u16 },
    Compressed { stream: u32, index: u32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct XrefRow {
    pub num: u32,
    pub entry: XrefEntry,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ObjectSummary {
    pub num: u32,
    pub entry: XrefEntry,
    /// Object type (`dictionary`, `stream`, ...) when it loads.
    pub kind: Option<String>,
    /// `/Type` and `/Subtype` of dictionaries and streams.
    pub type_name: Option<String>,
    pub subtype: Option<String>,
    /// Schema type, learned from references or guessed from `/Type`.
    pub schema_type: Option<String>,
    pub error: Option<String>,
}

// ------------------------------------------------------------------ objects

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NodeRoot {
    Trailer,
    Object { id: ObjRef },
}

/// A node inside an object: the root followed by child indices (array index,
/// or dictionary entry index in file order).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct NodePath {
    pub root: NodeRoot,
    pub path: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum StringKind {
    Literal,
    Hex,
}

/// One level of an object. Containers carry only their size; their items are
/// fetched with `Children`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Value {
    Null,
    Bool {
        value: bool,
    },
    Integer {
        value: i64,
    },
    Real {
        value: f64,
    },
    String {
        kind: StringKind,
        len: u64,
        /// Text decoded as UTF-16BE / UTF-8 (with BOM) or PDFDocEncoding,
        /// truncated.
        text: String,
        /// Raw bytes in hex, truncated.
        hex: String,
        truncated: bool,
    },
    Name {
        /// `#xx`-escaped as in PDF syntax, without the leading `/`.
        name: String,
    },
    Array {
        len: u64,
    },
    Dict {
        len: u64,
        type_name: Option<String>,
    },
    Stream {
        len: u64,
        type_name: Option<String>,
        raw_len: u64,
    },
    Ref {
        id: ObjRef,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct NodeView {
    pub index: u32,
    /// Dictionary key (`#xx`-escaped, without `/`); `None` for array items.
    pub key: Option<String>,
    pub value: Value,
    /// Location in the source file. `None` for objects inside object streams
    /// and for synthetic nodes.
    pub span: Option<Span>,
    /// What the schema says about this entry, when the container's type is
    /// known.
    pub schema: Option<NodeSchema>,
}

// ------------------------------------------------------------------ schema

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TypeOrigin {
    /// Reached by following typed references from the trailer.
    Context,
    /// Guessed from the object's `/Type` and other entries.
    Guess,
}

/// Schema type of an object (or the trailer) and its problems.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ObjectSchema {
    /// `None` when no type is known.
    pub type_name: Option<String>,
    pub origin: Option<TypeOrigin>,
    /// Where the specification defines it (e.g. `Table 29`).
    pub note: Option<String>,
    /// Version the document is checked against.
    pub pdf_version: Option<String>,
    /// Problems of the object and of the direct objects inside it.
    pub issues: Vec<DiagnosticInfo>,
    /// Some large container was checked only in part.
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Requirement {
    Required,
    Optional,
    /// Required when the (Arlington) condition holds.
    Conditional {
        condition: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SlotDoc {
    /// Arlington type: `dictionary`, `name`, `rectangle`, `string-text`, ...
    pub base: String,
    /// Types a dictionary, array or stream of this slot may have.
    pub links: Vec<String>,
}

/// Definition of a key (or array element) in the schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct KeyDoc {
    pub types: Vec<SlotDoc>,
    pub requirement: Requirement,
    pub since: Option<String>,
    pub deprecated: Option<String>,
    pub extension: Option<String>,
    pub inheritable: bool,
    pub default: Option<String>,
    /// Allowed values, when the schema lists them (truncated).
    pub allowed: Vec<String>,
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct NodeSchema {
    /// `None` for keys the schema does not define.
    pub key: Option<KeyDoc>,
    /// Type of the value: of a direct dictionary / array, or of the
    /// referenced object.
    pub value_type: Option<String>,
    /// Problems of this entry (and, for a direct container, of its own keys).
    pub issues: Vec<DiagnosticInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Location {
    File { span: Span },
    ObjStm { stream: ObjRef, index: u32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StreamInfo {
    /// Encoded data in the source file.
    pub data_span: Option<Span>,
    pub raw_len: u64,
    pub filters: Vec<String>,
    pub encrypted: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ObjectView {
    pub id: ObjRef,
    pub location: Location,
    pub value: Value,
    pub span: Option<Span>,
    pub stream: Option<StreamInfo>,
    pub diagnostics: Vec<DiagnosticInfo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DiagnosticInfo {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub obj: Option<ObjRef>,
    /// Location inside `obj` (or the trailer), like `/Pages/Kids[3]`.
    pub path: Option<String>,
    pub span: Option<Span>,
}

// ------------------------------------------------------------------ bytes

/// Metadata of a binary payload. The bytes travel separately (see
/// [`crate::protocol::BinaryFrame`]); `part` identifies them within the reply.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct BytesInfo {
    pub part: u32,
    pub offset: u64,
    pub len: u64,
    /// Length of the whole data, when known without decoding all of it.
    pub total: Option<u64>,
    /// The data ends within this chunk.
    pub eof: bool,
    /// Filter left undecoded (image codecs, unsupported filters).
    pub stopped_at: Option<String>,
    pub errors: Vec<String>,
}

// ------------------------------------------------------------------ jobs

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum JobKind {
    /// Load every object (optionally decoding every stream) and collect
    /// diagnostics.
    CheckObjects { decode_streams: bool },
    /// Validate against the schema, following typed references from the
    /// trailer, or from `root` (not following `/Parent` or other pages).
    /// Issues are read with `JobDiagnostics`.
    Validate { root: Option<ObjRef> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Running,
    Completed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct JobInfo {
    pub job: JobId,
    pub doc: DocId,
    pub kind: JobKind,
    pub state: JobState,
    pub done: u64,
    pub total: u64,
    pub result: Option<JobResult>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum JobResult {
    CheckObjects {
        ok: u64,
        failed: u64,
        missing: u64,
        streams: u64,
        decoded_bytes: u64,
        decode_errors: u64,
        diagnostics: u64,
    },
    Validate {
        objects: u64,
        errors: u64,
        warnings: u64,
        infos: u64,
        /// Issues kept for `JobDiagnostics` (the first ones, when there are
        /// very many).
        kept: u64,
    },
}
