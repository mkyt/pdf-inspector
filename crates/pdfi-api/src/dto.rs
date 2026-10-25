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
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
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
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
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
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Range {
    pub offset: u64,
    pub limit: u32,
}

/// One slice of a list. `total` is the length of the whole list.
///
/// Messages carry it as one concrete type per item (`ObjectSummaryPage`,
/// ...: foreign bindings have no generics), the same in JSON and TS.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Page<T> {
    pub total: u64,
    pub offset: u64,
    pub items: Vec<T>,
}

macro_rules! pages {
    ($($name:ident($item:ident, $ts:literal)),* $(,)?) => {$(
        /// One slice of a list (see [`Page`]).
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
        #[cfg_attr(feature = "ffi", derive(uniffi::Record))]
        #[ts(rename = $ts)]
        pub struct $name {
            pub total: u64,
            pub offset: u64,
            pub items: Vec<$item>,
        }

        impl From<Page<$item>> for $name {
            fn from(p: Page<$item>) -> Self {
                Self {
                    total: p.total,
                    offset: p.offset,
                    items: p.items,
                }
            }
        }
    )*};
}

pages! {
    ObjectSummaryPage(ObjectSummary, "Page<ObjectSummary>"),
    NodeViewPage(NodeView, "Page<NodeView>"),
    XrefRowPage(XrefRow, "Page<XrefRow>"),
    DiagnosticInfoPage(DiagnosticInfo, "Page<DiagnosticInfo>"),
    PageSummaryPage(PageSummary, "Page<PageSummary>"),
    AnnotInfoPage(AnnotInfo, "Page<AnnotInfo>"),
    OutlineItemPage(OutlineItem, "Page<OutlineItem>"),
    FontInfoPage(FontInfo, "Page<FontInfo>"),
    ImageInfoPage(ImageInfo, "Page<ImageInfo>"),
    ReferrerPage(Referrer, "Page<Referrer>"),
    ObjRefPage(ObjRef, "Page<ObjRef>"),
    OpViewPage(OpView, "Page<OpView>"),
    ContentIssuePage(ContentIssue, "Page<ContentIssue>"),
    ElementViewPage(ElementView, "Page<ElementView>"),
    GlyphViewPage(GlyphView, "Page<GlyphView>"),
    ProgramGlyphPage(ProgramGlyph, "Page<ProgramGlyph>"),
    XmpNodePage(XmpNode, "Page<XmpNode>"),
    FontUsePage(FontUse, "Page<FontUse>"),
}

/// Operations `[opener, last]` of a block (see `OpView.end`); a pair in
/// JSON and TS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
#[serde(from = "[u32; 2]", into = "[u32; 2]")]
#[ts(as = "[u32; 2]", rename = "[number, number]")]
pub struct BlockSpan {
    pub opener: u32,
    pub last: u32,
}

impl From<[u32; 2]> for BlockSpan {
    fn from([opener, last]: [u32; 2]) -> Self {
        Self { opener, last }
    }
}

impl From<BlockSpan> for [u32; 2] {
    fn from(b: BlockSpan) -> Self {
        [b.opener, b.last]
    }
}

// ------------------------------------------------------------------ files

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OpenSource {
    /// A path on the machine running the backend (server mode). Subject to
    /// the backend's access policy.
    Path { path: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct DirListing {
    /// `None` for the list of allowed roots.
    pub path: Option<String>,
    pub parent: Option<String>,
    pub entries: Vec<DirEntry>,
}

// ------------------------------------------------------------------ documents

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct DocInfo {
    pub doc: DocId,
    pub name: String,
    pub path: Option<String>,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
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
    /// Number of pages, by the page tree's counts.
    pub pages: u32,
    /// What the inventory job found, once it has run.
    pub inventory: Option<InventoryStats>,
    pub edit: EditStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
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
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum XrefKind {
    Table,
    Stream,
    Hybrid,
    Recovered,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
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
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum XrefEntry {
    Free {
        next: u32,
        generation: u16,
    },
    InUse {
        offset: u64,
        generation: u16,
    },
    Compressed {
        stream: u32,
        index: u32,
    },
    /// Changed or added by an edit that is not saved yet.
    Edited {
        generation: u16,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct XrefRow {
    pub num: u32,
    pub entry: XrefEntry,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
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
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NodeRoot {
    Trailer,
    Object { id: ObjRef },
}

/// A node inside an object: the root followed by child indices (array index,
/// or dictionary entry index in file order).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NodePath {
    pub root: NodeRoot,
    pub path: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum StringKind {
    Literal,
    Hex,
}

/// One level of an object. Containers carry only their size; their items are
/// fetched with `Children`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
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
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
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
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum TypeOrigin {
    /// Reached by following typed references from the trailer.
    Context,
    /// Named by the object's `/Type`, which only this type takes.
    TypeKey,
    /// Guessed from the object's `/Type` and other entries, among several
    /// types taking that `/Type`.
    Guess,
}

/// Schema type of an object (or the trailer) and its problems.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
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
    /// Keys (or array elements) the type defines, in the model's order.
    pub keys: Vec<TypeKey>,
}

/// A key (or array element) that a type defines, and whether the object
/// has it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct TypeKey {
    /// Dictionary key, array index (`0`), wildcard (`*`) or repeating array
    /// element (`1*`).
    pub key: String,
    /// `None` for wildcards and repeating elements.
    pub present: Option<bool>,
    /// Absent here but inheritable and given by an ancestor (`/Parent`).
    pub inherited: bool,
    pub doc: KeyDoc,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
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
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SlotDoc {
    /// Arlington type: `dictionary`, `name`, `rectangle`, `string-text`, ...
    pub base: String,
    /// Types a dictionary, array or stream of this slot may have.
    pub links: Vec<String>,
}

/// A value the schema lists for a key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct AllowedValue {
    /// In PDF syntax: `/Name`, `3`, `(text)`, `true`.
    pub value: String,
    pub since: Option<String>,
    pub deprecated: Option<String>,
}

/// Definition of a key (or array element) in the schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct KeyDoc {
    pub types: Vec<SlotDoc>,
    pub requirement: Requirement,
    pub since: Option<String>,
    pub deprecated: Option<String>,
    pub extension: Option<String>,
    pub inheritable: bool,
    pub default: Option<String>,
    /// Allowed values, when the schema lists them (truncated).
    pub allowed: Vec<AllowedValue>,
    /// Conditions a value meets instead (Arlington expressions, e.g.
    /// `@P>=0`).
    pub constraints: Vec<String>,
    /// Values besides `allowed` and `constraints` are possible (wildcards,
    /// conditions the model does not spell out).
    pub allowed_open: bool,
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
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
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Location {
    File {
        span: Span,
    },
    ObjStm {
        stream: ObjRef,
        index: u32,
    },
    /// Changed or added by an edit that is not saved yet.
    Edited,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct StreamInfo {
    /// Encoded data in the source file.
    pub data_span: Option<Span>,
    pub raw_len: u64,
    pub filters: Vec<String>,
    pub encrypted: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ObjectView {
    pub id: ObjRef,
    pub location: Location,
    pub value: Value,
    pub span: Option<Span>,
    pub stream: Option<StreamInfo>,
    pub diagnostics: Vec<DiagnosticInfo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
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
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
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
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum JobKind {
    /// Load every object (optionally decoding every stream) and collect
    /// diagnostics.
    CheckObjects { decode_streams: bool },
    /// Validate against the schema, following typed references from the
    /// trailer, or from `root` (not following `/Parent` or other pages).
    /// Issues are read with `JobDiagnostics`.
    Validate { root: Option<ObjRef> },
    /// Scan every object and page for fonts, images and references; the
    /// results are read with `Fonts`, `Images`, `Referrers` and
    /// `Unreferenced`.
    Inventory,
    /// Go through the pages using a font (all pages without the
    /// inventory) for the text it shows; read with `FontUses`.
    FontUses { font: ObjRef },
    /// Write the document with its edits to `path` (`None`: over the file
    /// it was opened from), then reopen it from there. Needs a backend with
    /// a filesystem; the browser saves through the Worker instead.
    Save {
        path: Option<String>,
        mode: SaveMode,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Running,
    Completed,
    Cancelled,
    /// Stopped by an error (`JobInfo.error`).
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct JobInfo {
    pub job: JobId,
    pub doc: DocId,
    pub kind: JobKind,
    pub state: JobState,
    pub done: u64,
    pub total: u64,
    pub result: Option<JobResult>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
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
    Inventory(InventoryStats),
    FontUses {
        /// Pages gone through, and those the font shows text on.
        scanned: u64,
        pages: u64,
        /// Text-showing operations.
        uses: u64,
    },
    Save(SaveResult),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct InventoryStats {
    pub objects: u64,
    /// Objects that failed to load.
    pub failed: u64,
    pub pages: u64,
    pub fonts: u64,
    pub images: u64,
    pub forms: u64,
    /// Edges of the reference graph.
    pub references: u64,
    pub unreferenced: u64,
}

impl InventoryStats {
    pub fn of(inv: &pdfi_model::inventory::Inventory) -> Self {
        Self {
            objects: inv.objects,
            failed: inv.failed,
            pages: inv.pages,
            fonts: inv.fonts.len() as u64,
            images: inv.images.len() as u64,
            forms: inv.forms,
            references: inv.refs.edges() as u64,
            unreferenced: inv.unreferenced.len() as u64,
        }
    }
}

// ------------------------------------------------------------------ editing

/// One edit. Values are written in PDF syntax: `<< /A [1 2 0 R] >>`,
/// `(text)`, `/Name`. Nodes are addressed like `Children` does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum EditCommand {
    /// Replace a node. An empty path replaces the whole object (a stream
    /// given a dictionary keeps its data) or the trailer.
    SetValue {
        node: NodePath,
        value: String,
    },
    /// Add or replace an entry of the dictionary or stream at `node`.
    SetEntry {
        node: NodePath,
        key: String,
        value: String,
    },
    RemoveEntry {
        node: NodePath,
        key: String,
    },
    /// Insert into the array at `node` (`index` may be its length).
    Insert {
        node: NodePath,
        index: u32,
        value: String,
    },
    /// Remove the child at `index` of the array or dictionary at `node`.
    RemoveChild {
        node: NodePath,
        index: u32,
    },
    /// A new object; a stream when `data` is given (stored as is, so the
    /// dictionary's `/Filter` must match it).
    AddObject {
        value: String,
        data: Option<StreamPayload>,
    },
    DeleteObject {
        id: ObjRef,
    },
    /// Replace the decoded data of a stream. Image codecs at the end of its
    /// filters (DCT, JPX, JBIG2, CCITT) stay: the data is their input.
    SetStreamData {
        id: ObjRef,
        data: StreamPayload,
        filter: StreamFilter,
    },
    /// Decode a stream's data and encode it again.
    SetStreamFilter {
        id: ObjRef,
        filter: StreamFilter,
    },
    DeletePages {
        pages: Vec<u32>,
    },
    /// Move pages so that they start at index `to` of the result.
    MovePages {
        pages: Vec<u32>,
        to: u32,
    },
    /// Add a multiple of 90 degrees to each page's rotation.
    RotatePages {
        pages: Vec<u32>,
        by: i32,
    },
    /// A page with an empty content, before page `at` (or at the end).
    InsertBlankPage {
        at: u32,
        width: f64,
        height: f64,
    },
    /// Copy pages of an open document (this one too) before page `at`.
    InsertPages {
        at: u32,
        from: DocId,
        pages: Vec<u32>,
    },
    /// Set (`None`: remove) an Info entry. Dates may be ISO 8601 or PDF
    /// dates. `sync_xmp` updates the matching XMP property, if any.
    SetInfo {
        key: String,
        value: Option<String>,
        sync_xmp: bool,
    },
    /// Replace or add the catalog's XMP metadata.
    SetXmp {
        xml: String,
    },
    /// Edit a content as operations (in order, each on the result of the
    /// ones before). Operations left as they were keep their bytes; the
    /// streams that change are written again (compressed if they were).
    EditOps {
        content: ContentRef,
        edits: Vec<OpEdit>,
    },
}

/// An edit of a content's operations. Text is content stream syntax, one
/// character per byte (Latin-1, as `ContentText` gives it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OpEdit {
    /// Replace operations `from..to` with those in `text`: insert when
    /// `from == to`, remove with an empty text. Those equal to the ones
    /// replaced stay as they were.
    Replace { from: u32, to: u32, text: String },
    /// Move operations `from..to` before operation `before` (or to the end
    /// with the count).
    Move { from: u32, to: u32, before: u32 },
}

/// Operations of a content as written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct OpText {
    pub from: u32,
    /// Less than asked when the text grew too long.
    pub to: u32,
    /// One character per byte (Latin-1).
    pub text: String,
}

/// Stream data in a JSON message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StreamPayload {
    /// UTF-8 text.
    Text {
        text: String,
    },
    Base64 {
        data: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum StreamFilter {
    None,
    Flate,
}

/// The undo / redo state of a document and what saving can do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct EditStatus {
    /// Edits are applied that are not saved.
    pub dirty: bool,
    /// Objects added, changed or deleted.
    pub edited_objects: u32,
    /// Labels of what `Undo` and `Redo` would do.
    pub undo: Option<String>,
    pub redo: Option<String>,
    /// Signed signature fields: a full rewrite invalidates them.
    pub signatures: u32,
    /// Why an incremental update is not possible (encrypted or recovered).
    pub no_incremental: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ApplyResult {
    pub status: EditStatus,
    /// Objects added, in order.
    pub created: Vec<ObjRef>,
    /// Object numbers changed, added or deleted.
    pub changed: Vec<u32>,
    pub trailer_changed: bool,
    pub notes: Vec<String>,
    /// Schema problems of the changed objects (the first ones).
    pub issues: Vec<DiagnosticInfo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SaveMode {
    /// Append the edits to the unchanged file; keeps signatures valid.
    Incremental,
    /// Write every object anew: decrypted, without old revisions.
    Full {
        /// A cross-reference stream instead of a table.
        xref_stream: bool,
        /// Objects other than streams in object streams (implies a
        /// cross-reference stream).
        object_streams: bool,
        /// Flate-compress streams that have no filter.
        compress: bool,
        /// Leave out objects the trailer does not reach.
        remove_unused: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SaveResult {
    /// Where it went (`None` in the browser).
    pub path: Option<String>,
    pub bytes: u64,
    pub objects: u64,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum ChangeReason {
    Edit,
    Undo,
    Redo,
    /// Saved and reopened from the saved file.
    Saved,
}

// ------------------------------------------------------------------ model

/// A rectangle in default user space, normalized (`x0 <= x1`, `y0 <= y1`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// Where a page attribute comes from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AttrOrigin {
    Page,
    /// From this ancestor in the page tree.
    Inherited {
        from: ObjRef,
    },
    /// Not given: the default value, or that of the named box.
    Default {
        of: String,
    },
    Missing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PageSummary {
    /// 0-based.
    pub index: u32,
    pub id: Option<ObjRef>,
    pub label: Option<String>,
    /// The visible area (crop box clipped to the media box).
    pub size: Option<Rect>,
    pub rotate: i64,
    pub annots: u32,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PageBox {
    pub name: String,
    pub rect: Option<Rect>,
    pub origin: AttrOrigin,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ContentInfo {
    pub id: Option<ObjRef>,
    pub raw_len: Option<u64>,
    pub filters: Vec<String>,
    pub error: Option<String>,
}

/// One named resource: `/Font /F1`, `/XObject /Im3`, ...
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ResourceItem {
    pub category: String,
    pub name: String,
    pub id: Option<ObjRef>,
    /// `Type1 Helvetica`, `Image 640×480 DeviceRGB`, ...
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PageDetail {
    pub index: u32,
    pub id: ObjRef,
    pub label: Option<String>,
    /// Page tree nodes from the root down to the page's parent.
    pub ancestors: Vec<ObjRef>,
    /// MediaBox, CropBox, BleedBox, TrimBox, ArtBox.
    pub boxes: Vec<PageBox>,
    pub rotate: i64,
    pub rotate_origin: AttrOrigin,
    pub user_unit: f64,
    /// The resource dictionary, when it is an indirect object.
    pub resources: Option<ObjRef>,
    pub resources_origin: AttrOrigin,
    pub resource_items: Vec<ResourceItem>,
    pub resources_truncated: bool,
    pub contents: Vec<ContentInfo>,
    pub annots: u32,
    pub issues: Vec<DiagnosticInfo>,
}

/// A destination, resolved to a page of this document where possible.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Destination {
    /// The name it was looked up by (named destinations).
    pub named: Option<String>,
    pub page: Option<ObjRef>,
    pub page_index: Option<u32>,
    /// Page number in another file (`GoToR`).
    pub remote_page: Option<i64>,
    /// `XYZ`, `Fit`, `FitH`, ...
    pub fit: Option<String>,
    /// Parameters after the fit type; `null` for null.
    pub params: Vec<Option<f64>>,
    pub error: Option<String>,
}

/// Where an outline item, link or open action goes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LinkTarget {
    Dest {
        dest: Destination,
    },
    Uri {
        uri: String,
    },
    /// Another action, by its `/S` type.
    Action {
        action: String,
        detail: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct OutlineItem {
    pub id: ObjRef,
    pub title: String,
    /// Open descendants when positive; closed when negative.
    pub count: Option<i64>,
    pub has_children: bool,
    pub link: Option<LinkTarget>,
    /// 1 italic, 2 bold.
    pub flags: i64,
    /// RGB, 0..1.
    pub color: Option<Vec<f64>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct AnnotInfo {
    /// Position in the page's `/Annots` (also the order they are painted
    /// in, over the content).
    pub index: u32,
    pub id: Option<ObjRef>,
    pub subtype: Option<String>,
    /// In the page's default space.
    pub rect: Option<Rect>,
    pub contents: Option<String>,
    /// `/NM`.
    pub name: Option<String>,
    pub flags: i64,
    /// Names of the `/F` bits set.
    pub flag_names: Vec<String>,
    /// Shown on screen (not Hidden or NoView; Invisible without an
    /// appearance).
    pub shown: bool,
    /// `/T`.
    pub author: Option<String>,
    /// `/Subj`.
    pub subject: Option<String>,
    /// `/M`, ISO 8601 when it is a PDF date.
    pub modified: Option<String>,
    /// `/C` as written: 0, 1 (gray), 3 (RGB) or 4 (CMYK) components.
    pub color: Option<Vec<f64>>,
    /// `/IC`.
    pub interior: Option<Vec<f64>>,
    /// `/CA`.
    pub opacity: Option<f64>,
    pub border: Option<f64>,
    /// `/QuadPoints`, 8 numbers each, in the page's default space.
    pub quads: Vec<Vec<f64>>,
    /// Ink paths, polygon vertices or a line: `[x0, y0, x1, y1, ...]`.
    pub paths: Vec<Vec<f64>>,
    /// The `/AP` entries present (`N`, `R`, `D`).
    pub appearances: Vec<String>,
    /// The normal appearance (by `/AS` when `/N` has states).
    pub appearance: Option<AppearanceInfo>,
    /// Every appearance stream of `/AP`: `N`, `R`, `D`, each state.
    pub variants: Vec<AppearanceInfo>,
    /// `/AS`.
    pub as_state: Option<String>,
    pub link: Option<LinkTarget>,
    /// `/Parent` of a pop-up, `/IRT` of a reply.
    pub parent: Option<ObjRef>,
    /// `/Popup`.
    pub popup: Option<ObjRef>,
    pub error: Option<String>,
}

/// An annotation's normal appearance stream and how it is put on the page
/// (ISO 32000-2 12.5.5).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct AppearanceInfo {
    /// The `/AP` entry: `N` (normal), `R` (rollover) or `D` (down).
    pub kind: String,
    /// The form XObject (open it as a `stream` content).
    pub id: Option<ObjRef>,
    /// The state, when the entry has states (for the normal appearance,
    /// the one `/AS` chooses).
    pub state: Option<String>,
    pub states: Vec<String>,
    /// The form's `/BBox`, in form space.
    pub bbox: Option<Rect>,
    /// The form's `/Matrix`.
    pub matrix: Vec<f64>,
    /// Form space to the page's default space: `/Matrix`, then the fit
    /// of the transformed `/BBox` to `/Rect`.
    pub to_page: Option<Vec<f64>>,
    /// Why it is not painted.
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct InfoEntry {
    pub key: String,
    pub text: String,
    /// ISO 8601, for values that are PDF dates.
    pub date: Option<String>,
}

/// An entry of the catalog, summarized (`AcroForm`: `3 top-level fields`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Feature {
    pub key: String,
    pub text: String,
    pub obj: Option<ObjRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct DocumentView {
    pub catalog: Option<ObjRef>,
    pub info: Option<ObjRef>,
    pub info_entries: Vec<InfoEntry>,
    /// The catalog's XMP metadata stream.
    pub metadata: Option<ObjRef>,
    pub pages: u32,
    pub features: Vec<Feature>,
    /// Problems of the page tree, outline and metadata (also Info / XMP
    /// disagreements).
    pub issues: Vec<DiagnosticInfo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum XmpNodeKind {
    Element,
    Attribute,
}

/// A node of the XMP tree; children follow their parent with a greater
/// `depth`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct XmpNode {
    pub depth: u32,
    pub kind: XmpNodeKind,
    pub name: String,
    pub namespace: Option<String>,
    pub value: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct XmpProperty {
    pub name: String,
    pub namespace: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct XmpView {
    pub id: ObjRef,
    /// Length of the decoded packet.
    pub len: u64,
    pub nodes: XmpNodePage,
    /// The tree has more nodes than were kept.
    pub truncated: bool,
    pub properties: Vec<XmpProperty>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct FontInfo {
    pub id: ObjRef,
    pub base_font: Option<String>,
    pub subtype: Option<String>,
    pub encoding: Option<String>,
    /// `FontFile`, `FontFile2`, `FontFile3/Type1C`, ...; `None`: not
    /// embedded.
    pub embedded: Option<String>,
    pub font_file: Option<ObjRef>,
    pub to_unicode: bool,
    pub descendant: Option<ObjRef>,
    pub subset: bool,
    /// One of the standard 14 fonts.
    pub standard: bool,
    /// Pages using it through their resources.
    pub page_count: u32,
    /// The first of those pages (0-based).
    pub pages: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum FontKind {
    /// Type 1, TrueType, MMType1.
    Simple,
    Type0,
    Type3,
}

/// What draws a font's glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum GlyphSourceKind {
    /// The embedded program.
    Embedded,
    /// A compiled-in font (simple fonts not embedded, or whose program
    /// cannot be read).
    Substitute,
    /// Type 3 glyph procedures.
    Procedures,
    /// Nothing: glyphs are drawn as boxes.
    None,
}

/// An embedded font program.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct FontProgramInfo {
    pub stream: Option<ObjRef>,
    /// `FontFile`, `FontFile2` or `FontFile3`.
    pub key: String,
    /// `/Subtype` of a `FontFile3`.
    pub subtype: Option<String>,
    /// As read: `TrueType`, `OpenType (CFF)`, `CFF`, `Type 1` (`None`:
    /// unreadable).
    pub format: Option<String>,
    /// Decoded bytes.
    pub len: u64,
    pub glyphs: Option<u32>,
    pub units_per_em: Option<u32>,
    pub cid_keyed: bool,
}

/// A font: dictionaries, encoding, program and problems.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct FontDetail {
    pub id: ObjRef,
    pub kind: FontKind,
    pub subtype: Option<String>,
    pub base_font: Option<String>,
    pub descendant: Option<ObjRef>,
    /// `CIDFontType0` / `CIDFontType2`.
    pub cid_subtype: Option<String>,
    pub descriptor: Option<ObjRef>,
    /// `WinAnsiEncoding + 3 differences`, `Identity-H`, `embedded CMap`, ...
    pub encoding: Option<String>,
    pub vertical: bool,
    pub to_unicode: Option<ObjRef>,
    pub to_unicode_entries: u32,
    /// `Identity`, or the number of CIDs a `/CIDToGIDMap` stream maps.
    pub cid_to_gid: Option<String>,
    pub program: Option<FontProgramInfo>,
    pub glyph_source: GlyphSourceKind,
    /// Name of the substitute font.
    pub substitute: Option<String>,
    /// `/Flags` of the descriptor, and their names.
    pub flags: Option<i64>,
    pub flag_names: Vec<String>,
    pub symbolic: bool,
    /// Thousandths of an em (glyph space for Type 3).
    pub ascent: f64,
    pub descent: f64,
    /// Type 3 `/FontMatrix`.
    pub font_matrix: Option<Vec<f64>>,
    /// Widths are partly guessed.
    pub approximate: bool,
    /// Number of entries `Glyphs` lists.
    pub codes: u32,
    pub issues: Vec<DiagnosticInfo>,
}

/// One code of a font and the glyph it selects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct GlyphView {
    pub code: u32,
    /// The code's bytes in hex (`41`, `0041`).
    pub bytes: String,
    pub cid: Option<u32>,
    pub gid: Option<u32>,
    pub name: Option<String>,
    /// From `/ToUnicode`, or the glyph name.
    pub text: Option<String>,
    /// Advance width in glyph space (thousandths of an em but for Type 3).
    pub width: f64,
    /// SVG path data of the outline, 1000 units per em, y up.
    pub outline: Option<String>,
    /// Bounding box of the outline, same units.
    pub bbox: Option<Rect>,
    /// Type 3: the glyph procedure.
    pub procedure: Option<ObjRef>,
}

/// One glyph of an embedded font program, by index, and the codes that
/// select it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ProgramGlyph {
    pub gid: u32,
    pub name: Option<String>,
    /// Bytes in hex of the codes `Glyphs` lists that select it (none: the
    /// font cannot show it).
    pub codes: Vec<String>,
    /// The text of the first of them.
    pub text: Option<String>,
    /// Advance width in the program, thousandths of an em.
    pub width: f64,
    /// SVG path data of the outline, 1000 units per em, y up.
    pub outline: Option<String>,
    pub bbox: Option<Rect>,
}

/// Text a font shows: one text-showing operation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct FontUse {
    /// 0-based page index.
    pub page: u32,
    /// The page's operation (for text inside a form XObject, the `Do`
    /// that paints the form).
    pub op: u32,
    /// Inside a form XObject: the innermost form and the operation in it.
    pub form: Option<ObjRef>,
    pub form_op: Option<u32>,
    /// Decoded text (U+FFFD for codes without text), shortened.
    pub text: String,
    /// Codes shown.
    pub codes: u32,
    /// On the page, in default user space.
    pub bbox: Option<Rect>,
    /// Text render mode 3 or 7.
    pub hidden: bool,
}

/// How often a code is shown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CodeCount {
    /// The code's bytes in hex, as `GlyphView.bytes`.
    pub bytes: String,
    pub count: u64,
}

/// The text a font shows, once a `FontUses` job has gone through the
/// document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct FontUsesView {
    /// A `FontUses` job has finished for the font (otherwise the rest is
    /// empty).
    pub scanned: bool,
    /// Pages gone through, and those the font shows text on.
    pub pages_scanned: u32,
    pub pages: u32,
    /// Every code shown, ascending.
    pub codes: Vec<CodeCount>,
    pub uses: FontUsePage,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ImageInfo {
    pub id: ObjRef,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub bits: Option<i64>,
    pub color_space: Option<String>,
    pub filters: Vec<String>,
    pub raw_len: u64,
    pub image_mask: bool,
    pub smask: Option<ObjRef>,
    pub mask: bool,
    pub interpolate: bool,
    pub page_count: u32,
    pub pages: Vec<u32>,
}

/// Live objects of one kind (`dictionary`, `stream`, ...; `unloadable` for
/// those that failed to load) with the same `/Type`, `/Subtype` and schema
/// type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ObjectClass {
    pub kind: String,
    pub type_name: Option<String>,
    pub subtype: Option<String>,
    /// Typed from the trailer when the inventory was built; for objects it
    /// does not reach, guessed from `/Type`.
    pub schema_type: Option<String>,
    pub count: u64,
}

/// Which objects `Objects` lists (needs the inventory).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ObjectFilter {
    /// Objects in any of these classes (`count` ignored); `None`: any.
    pub classes: Option<Vec<ObjectClass>>,
    /// Only objects nothing refers to.
    pub unreferenced: bool,
}

/// How the trailer reaches an object: each step refers to the next one (the
/// last to the object) through `key`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct RefPath {
    /// Whether some chain of references from the trailer reaches it.
    pub reachable: bool,
    pub steps: Vec<PathStep>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PathStep {
    pub from: NodeRoot,
    /// Schema type, or `/Type`.
    pub type_name: Option<String>,
    /// Where in it: `/Pages`, `/Kids[3]`, ...
    pub key: String,
}

/// An object (or the trailer) that refers to another one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Referrer {
    pub from: NodeRoot,
    /// Schema type, or `/Type`.
    pub type_name: Option<String>,
    /// Where in it: `/Resources/Font/F1`, ...
    pub paths: Vec<String>,
}

// ------------------------------------------------------------------ content

/// A content: a page's `/Contents` (with its resources) or a stream (form
/// XObject, tiling pattern, glyph procedure, appearance stream, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ContentRef {
    Page { index: u32 },
    Stream { id: ObjRef },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum ContentKind {
    Page,
    Form,
    /// Tiling pattern.
    Pattern,
    /// A stream of unknown use.
    Other,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ContentSourceInfo {
    pub id: Option<ObjRef>,
    /// Decoded bytes parsed.
    pub len: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CategoryCount {
    /// Operator group: `pathConstruction`, `textShowing`, ...
    pub category: String,
    pub count: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ContentSummary {
    pub content: ContentRef,
    pub kind: ContentKind,
    /// The page or stream object.
    pub id: ObjRef,
    /// The streams parsed, in order (several for a `/Contents` array).
    pub sources: Vec<ContentSourceInfo>,
    pub ops: u64,
    /// Stopped at the operation limit.
    pub truncated: bool,
    /// Operations that paint something (see `ContentElements`).
    pub elements: u64,
    pub issues: u64,
    pub errors: u64,
    pub warnings: u64,
    /// Issues beyond the kept ones.
    pub issues_dropped: u64,
    /// The page's visible area, or the form's `/BBox`; the initial clip.
    pub bounds: Option<Rect>,
    /// The resource dictionary, when it is an indirect object.
    pub resources: Option<ObjRef>,
    pub has_resources: bool,
    /// Some text bounding boxes rest on guessed glyph widths.
    pub approximate: bool,
    pub counts: Vec<CategoryCount>,
    /// What `Render` shows.
    pub view: ViewInfo,
}

/// What is shown of a content: a page's visible area turned by its
/// `/Rotate`, a form's `/BBox`, or what a stream paints. Rendered bitmaps
/// and hit tests use this space (scaled to pixels for bitmaps).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ViewInfo {
    /// In points.
    pub width: f64,
    pub height: f64,
    /// The content's space to the view, `[a b c d e f]`: origin at the top
    /// left, y down.
    pub matrix: Vec<f64>,
    pub rotate: u32,
}

/// A region of a rendered view, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RenderLayer {
    /// The content (pages on white, streams on transparent).
    Content,
    /// Only these operations, in one opaque color on transparent.
    Highlight { ops: Vec<u32> },
    /// A page's annotation appearances (those shown, or only these
    /// `/Annots` indices) on transparent, instead of the content: they are
    /// painted over it in this order.
    Annots {
        only: Option<Vec<u32>>,
        /// Appearances to paint instead of the normal one.
        #[serde(default)]
        pick: Vec<AppearancePick>,
    },
}

/// Another appearance of an annotation to paint: `/AP` entry `kind`
/// (`N`, `R`, `D`) and its state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct AppearancePick {
    /// Position in the page's `/Annots`.
    pub index: u32,
    pub kind: String,
    pub state: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct RenderNote {
    /// What was approximated or left out.
    pub message: String,
    pub count: u32,
}

/// A rendered bitmap: RGBA (not premultiplied), row by row, in binary part
/// `part`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct BitmapInfo {
    pub part: u32,
    /// The region of the whole view's bitmap this is.
    pub rect: PixelRect,
    /// Size of the whole view's bitmap at this scale.
    pub full_width: u32,
    pub full_height: u32,
    pub scale: f64,
    /// Operations run.
    pub ops: u64,
    pub notes: Vec<RenderNote>,
}

/// An image file a stream's data is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum ImageFile {
    Jpeg,
    Jpeg2000,
}

/// An image XObject decoded as it paints (`/Decode`, color space and tint
/// transforms, `/SMask` or `/Mask`): RGBA (not premultiplied), row by row,
/// in binary part `part`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct DecodedImage {
    /// `None` when the image cannot be decoded (`error` says why).
    pub part: Option<u32>,
    pub width: u32,
    pub height: u32,
    /// Colors converted approximately (tint transforms).
    pub approximate: bool,
    pub error: Option<String>,
    /// The stream's data is this image file and viewers show it as the
    /// image paints (not, for example, ink amounts of a separation that
    /// they would show inverted).
    pub file: Option<ImageFile>,
}

/// Which painting operations a region of a content takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum RegionSelect {
    /// What they paint lies inside it.
    Inside,
    /// What they paint touches it.
    Overlaps,
}

/// A region cut out of a content: a one-page PDF in binary part `part`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ExtractInfo {
    pub part: u32,
    pub bytes: u64,
    /// The page's `/MediaBox`, in the content's space.
    pub media_box: Rect,
    /// Painting operations kept and dropped (also inside forms cut).
    pub kept: u64,
    pub dropped: u64,
    /// Form XObjects cut because they were partly inside.
    pub forms: u64,
    pub objects: u64,
}

/// How an SVG export writes text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum SvgText {
    /// `<text>` with the font's name, each character where it paints
    /// (glyphs without Unicode text as outlines).
    Text,
    /// Glyph outlines.
    Paths,
}

/// A content (or a region of it) as SVG, in binary part `part`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SvgInfo {
    pub part: u32,
    pub bytes: u64,
    /// The view box, in points of the view (from its top left).
    pub view_box: Rect,
    /// Elements written (paths, texts, images, shadings), and images.
    pub elements: u64,
    pub images: u64,
    /// What was approximated or left out.
    pub notes: Vec<RenderNote>,
}

/// A problem of a content: of one operation, or (`op: None`) of the whole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ContentIssue {
    pub op: Option<u32>,
    pub severity: Severity,
    pub code: String,
    pub message: String,
    /// For syntax problems: the stream and the bytes in its decoded data.
    pub source: Option<u32>,
    pub span: Option<Span>,
}

/// What an operation paints, in the content's space (default user space for
/// pages).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ElementView {
    pub op: u32,
    /// `fill`, `stroke`, `fillStroke`, `clip`, `text`, `image`,
    /// `inlineImage`, `form`, `shading`.
    pub kind: String,
    pub bbox: Rect,
    /// Outside the clipping path, or invisible text.
    pub hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct OpView {
    pub index: u32,
    pub operator: String,
    /// Operator group (see [`CategoryCount`]); `unknown` for unknown ones.
    pub category: String,
    /// In PDF syntax, shortened.
    pub operands: Vec<String>,
    /// Nesting of `q`, `BT` and marked content; closers at their opener's.
    pub depth: u32,
    /// For `q`, `BT`, `BMC`, `BDC` and `BX`: the last operation of the
    /// block it opens (its closer when there is one).
    pub end: Option<u32>,
    /// The stream it was read from and its bytes in that stream's decoded
    /// data.
    pub source: Option<u32>,
    pub span: Option<Span>,
    /// Decoded text of text-showing operators, when the font tells it.
    pub text: Option<String>,
    /// The resource it names (`Tf`, `Do`, `gs`, `sh`, `cs`, `scn`, `BDC`),
    /// when it is an indirect object.
    pub resource: Option<ObjRef>,
    pub element: Option<ElementView>,
    pub issues: Vec<ContentIssue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ColorView {
    /// As set: `DeviceRGB`, `Pattern`, or a `/ColorSpace` resource name.
    pub space: String,
    pub family: String,
    pub components: Vec<f64>,
    pub pattern: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct FontView {
    /// Resource name (`ExtGState` when set by `gs`).
    pub name: String,
    pub id: Option<ObjRef>,
    pub base_font: Option<String>,
    pub subtype: Option<String>,
    /// Widths are partly guessed.
    pub approximate: bool,
}

/// The graphics and text state around one operation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct GraphicsState {
    /// `[a b c d e f]`.
    pub ctm: Vec<f64>,
    /// Bounding box of the clipping path (`None`: not clipped).
    pub clip: Option<Rect>,
    pub line_width: f64,
    pub line_cap: i64,
    pub line_join: i64,
    pub miter_limit: f64,
    pub dash: Vec<f64>,
    pub dash_phase: f64,
    pub intent: String,
    pub flatness: f64,
    pub stroke: ColorView,
    pub fill: ColorView,
    pub stroke_alpha: f64,
    pub fill_alpha: f64,
    pub blend: String,
    pub soft_mask: Option<String>,
    pub ext_gstates: Vec<String>,
    pub font: Option<FontView>,
    pub font_size: f64,
    pub char_spacing: f64,
    pub word_spacing: f64,
    /// Percent.
    pub horizontal_scale: f64,
    pub leading: f64,
    pub render_mode: i64,
    pub rise: f64,
    pub in_text: bool,
    /// Text matrix and text line matrix (inside a text object).
    pub text_matrix: Option<Vec<f64>>,
    pub line_matrix: Option<Vec<f64>>,
    /// Saved states (`q` depth).
    pub saved: u32,
    /// Open marked-content tags, outermost first.
    pub marked: Vec<String>,
    /// Bounding box of the path being built, and its current point (user
    /// space).
    pub path: Option<Rect>,
    pub current_point: Option<Vec<f64>>,
}

/// The state an operation runs in and the state it leaves.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct OpState {
    pub index: u32,
    pub operator: String,
    pub description: String,
    pub before: GraphicsState,
    pub after: GraphicsState,
}
