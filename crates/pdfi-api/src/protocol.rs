//! The message protocol shared by every transport (WebSocket, Worker).
//!
//! Control messages are JSON: the client sends [`ClientMessage`], the backend
//! answers with [`ServerMessage`]. Binary payloads are sent as separate
//! [`BinaryFrame`]s *before* the response that refers to them.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::dto::*;

/// Bumped on every incompatible change; checked by `Hello`.
pub const PROTOCOL_VERSION: u32 = 16;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ClientMessage {
    pub id: RequestId,
    pub request: Request,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Request {
    Hello {
        protocol: u32,
    },
    ListDir {
        path: Option<String>,
    },
    Open {
        source: OpenSource,
        password: Option<String>,
        /// Ignore the cross-reference data and rebuild it by scanning.
        recover: bool,
    },
    Docs,
    Close {
        doc: DocId,
    },
    Summary {
        doc: DocId,
    },
    /// Cross-reference rows in object-number order (merged table), or the
    /// live objects a filter picks (needs the inventory).
    Objects {
        doc: DocId,
        range: Range,
        #[serde(default)]
        filter: Option<ObjectFilter>,
    },
    /// Live objects grouped by kind, `/Type`, `/Subtype` and schema type
    /// (needs the inventory).
    ObjectClasses {
        doc: DocId,
    },
    Object {
        doc: DocId,
        id: ObjRef,
    },
    Children {
        doc: DocId,
        path: NodePath,
        range: Range,
    },
    /// Entries of one revision (0 = newest), or of the merged table.
    Xref {
        doc: DocId,
        revision: Option<u32>,
        range: Range,
    },
    Diagnostics {
        doc: DocId,
        range: Range,
    },
    /// Schema type of the trailer or an object, and its problems.
    Schema {
        doc: DocId,
        root: NodeRoot,
    },
    /// Info, XMP, page count and what the catalog declares.
    Document {
        doc: DocId,
    },
    /// The catalog's XMP metadata: properties and a page of the tree.
    Xmp {
        doc: DocId,
        range: Range,
    },
    Pages {
        doc: DocId,
        range: Range,
    },
    /// One page (0-based) with its inherited attributes resolved.
    Page {
        doc: DocId,
        index: u32,
    },
    /// Index of a page object (`None` if it is not a page of the tree).
    PageIndex {
        doc: DocId,
        id: ObjRef,
    },
    Annotations {
        doc: DocId,
        page: u32,
        range: Range,
    },
    /// Outline items under `parent` (`None`: the top level).
    Outline {
        doc: DocId,
        parent: Option<ObjRef>,
        range: Range,
    },
    /// Fonts found by the `Inventory` job (`NotReady` before it ran).
    Fonts {
        doc: DocId,
        range: Range,
    },
    Images {
        doc: DocId,
        range: Range,
    },
    /// Objects referring to `id` (needs the inventory).
    Referrers {
        doc: DocId,
        id: ObjRef,
        range: Range,
    },
    /// A shortest chain of references from the trailer to `id`, preferring
    /// forward links (`/Kids`) to back links (`/Parent`) (needs the
    /// inventory).
    RefPath {
        doc: DocId,
        id: ObjRef,
    },
    /// Live objects nothing refers to (needs the inventory).
    Unreferenced {
        doc: DocId,
        range: Range,
    },
    /// A content: its streams, size, problems and bounds.
    Content {
        doc: DocId,
        content: ContentRef,
    },
    /// Operations of a content, with their problems and what they paint.
    ContentOps {
        doc: DocId,
        content: ContentRef,
        range: Range,
    },
    /// Problems of a content: content-wide ones first, then by operation.
    ContentIssues {
        doc: DocId,
        content: ContentRef,
        range: Range,
    },
    /// Operations `range` of a content as written, for editing (about
    /// 4 MB at most: `OpText.to` tells where it stopped).
    ContentText {
        doc: DocId,
        content: ContentRef,
        range: Range,
    },
    /// Blocks of a content (see `OpView.end`) whose opener is at `depth`,
    /// as `[opener, last]`, for folding them all.
    ContentBlocks {
        doc: DocId,
        content: ContentRef,
        depth: u32,
    },
    /// The resources a content can name, by category.
    ContentResources {
        doc: DocId,
        content: ContentRef,
    },
    /// What the operations of a content paint, in operation order.
    ContentElements {
        doc: DocId,
        content: ContentRef,
        range: Range,
    },
    /// The graphics state an operation runs in and the one it leaves.
    OpState {
        doc: DocId,
        content: ContentRef,
        index: u32,
    },
    /// A font: its dictionaries, encoding, program and problems.
    Font {
        doc: DocId,
        id: ObjRef,
    },
    /// The glyphs of a font by code (`FontDetail.codes` of them): CID,
    /// glyph, name, text, width and outline.
    Glyphs {
        doc: DocId,
        id: ObjRef,
        range: Range,
    },
    /// The glyphs of a font's embedded program by index
    /// (`FontProgramInfo.glyphs` of them), with the codes that select each.
    ProgramGlyphs {
        doc: DocId,
        id: ObjRef,
        range: Range,
    },
    /// The text a font shows, as found by its last `FontUses` job.
    FontUses {
        doc: DocId,
        id: ObjRef,
        range: Range,
    },
    /// Render a content, or a tile of it, at `scale` pixels per point.
    /// The bitmap is a binary part (`BitmapInfo`). Can be cancelled while
    /// it runs (server).
    Render {
        doc: DocId,
        content: ContentRef,
        scale: f64,
        /// `None`: the whole view.
        tile: Option<PixelRect>,
        /// Run operations up to and including this one.
        until: Option<u32>,
        layer: RenderLayer,
    },
    /// Operations painting within `radius` of a view point (points from
    /// the top left of `ViewInfo`), topmost first.
    HitTest {
        doc: DocId,
        content: ContentRef,
        x: f64,
        y: f64,
        radius: f64,
        until: Option<u32>,
    },
    /// Cut a region out of a content (`rect` in the content's space) into
    /// a one-page PDF with only the operations painting there: a binary
    /// part (`ExtractInfo`).
    ExtractRegion {
        doc: DocId,
        content: ContentRef,
        rect: Rect,
        select: RegionSelect,
        /// Shrink the page to what is kept.
        fit: bool,
    },
    /// A content as SVG to edit in a vector graphics program: all of it,
    /// or (`rect` in the content's space) what paints in a region, picked
    /// as `ExtractRegion` picks it. A binary part (`SvgInfo`).
    ExportSvg {
        doc: DocId,
        content: ContentRef,
        rect: Option<Rect>,
        select: RegionSelect,
        /// Shrink the view to what is kept (with `rect`).
        fit: bool,
        text: SvgText,
    },
    RawBytes {
        doc: DocId,
        offset: u64,
        len: u32,
    },
    StreamData {
        doc: DocId,
        id: ObjRef,
        /// Decrypt and apply filters; otherwise the bytes as stored.
        decoded: bool,
        offset: u64,
        len: u32,
    },
    /// An image XObject as it paints, reduced by about `reduce` in both
    /// directions (1: full size); a binary part (`DecodedImage`).
    DecodeImage {
        doc: DocId,
        id: ObjRef,
        reduce: u32,
    },
    StartJob {
        doc: DocId,
        job: JobKind,
    },
    JobStatus {
        job: JobId,
    },
    CancelJob {
        job: JobId,
    },
    /// Issues found by a `Validate` job (also while it runs).
    JobDiagnostics {
        job: JobId,
        range: Range,
    },
    /// A node written in PDF syntax (multi-line), to edit it as text. A
    /// stream is written as its dictionary.
    NodeSyntax {
        doc: DocId,
        path: NodePath,
    },
    /// Apply edits as one undoable step: all of them or none.
    Apply {
        doc: DocId,
        /// What `Undo` will say it reverts.
        label: String,
        commands: Vec<EditCommand>,
    },
    Undo {
        doc: DocId,
    },
    Redo {
        doc: DocId,
    },
    /// Drop a request that has not started yet. Handled by the transport.
    Cancel {
        request: RequestId,
    },
    Batch {
        requests: Vec<Request>,
    },
}

// Responses are short-lived and moved once; boxing would only add noise.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum Response {
    Ok(ResponseBody),
    Err(ApiError),
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ResponseBody {
    Hello(HelloInfo),
    Dir(DirListing),
    Doc(DocSummary),
    Docs {
        docs: Vec<DocInfo>,
    },
    Objects(ObjectSummaryPage),
    ObjectClasses {
        classes: Vec<ObjectClass>,
    },
    Object(ObjectView),
    Children(NodeViewPage),
    Xref(XrefRowPage),
    Diagnostics(DiagnosticInfoPage),
    Schema(ObjectSchema),
    Document(DocumentView),
    Xmp(XmpView),
    Pages(PageSummaryPage),
    PageDetail(PageDetail),
    PageIndex {
        index: Option<u32>,
    },
    Annotations(AnnotInfoPage),
    Outline(OutlineItemPage),
    Fonts(FontInfoPage),
    Images(ImageInfoPage),
    Referrers(ReferrerPage),
    RefPath(RefPath),
    ObjRefs(ObjRefPage),
    Content(ContentSummary),
    Ops(OpViewPage),
    ContentIssues(ContentIssuePage),
    OpText(OpText),
    Blocks {
        blocks: Vec<BlockSpan>,
        /// More blocks than returned.
        truncated: bool,
    },
    Resources {
        items: Vec<ResourceItem>,
        truncated: bool,
    },
    Elements(ElementViewPage),
    OpState(Box<OpState>),
    Font(Box<FontDetail>),
    Glyphs(GlyphViewPage),
    ProgramGlyphs(ProgramGlyphPage),
    FontUses(FontUsesView),
    Bitmap(BitmapInfo),
    DecodedImage(DecodedImage),
    Hits {
        ops: Vec<u32>,
    },
    Extract(ExtractInfo),
    Svg(SvgInfo),
    Bytes(BytesInfo),
    Job(JobInfo),
    Syntax {
        text: String,
        /// Longer than what is returned: not editable as text.
        truncated: bool,
    },
    Applied(ApplyResult),
    /// The browser's answer to saving through the Worker: the document as
    /// reopened from the file, and what was written.
    Saved {
        summary: DocSummary,
        result: SaveResult,
    },
    Edit(EditStatus),
    Batch {
        responses: Vec<Response>,
    },
    Empty,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct HelloInfo {
    pub protocol: u32,
    pub backend: String,
    pub version: String,
    /// Backend can open paths (`OpenSource::Path`) and list directories.
    pub filesystem: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    BadRequest,
    VersionMismatch,
    NoSuchDoc,
    NoSuchJob,
    NotFound,
    Forbidden,
    Unsupported,
    TooLarge,
    /// Needs a job to run first (the inventory).
    NotReady,
    Io,
    Parse,
    Cancelled,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl From<pdfi_syntax::Error> for ApiError {
    fn from(e: pdfi_syntax::Error) -> Self {
        let code = match &e {
            pdfi_syntax::Error::Io(_) => ErrorCode::Io,
            pdfi_syntax::Error::Missing(_) => ErrorCode::NotFound,
            pdfi_syntax::Error::Object(..)
            | pdfi_syntax::Error::Parse(_)
            | pdfi_syntax::Error::Invalid(_) => ErrorCode::Parse,
        };
        Self::new(code, e.to_string())
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        Self::new(ErrorCode::Io, e.to_string())
    }
}

// Like `Response`: built once and moved straight to the transport.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ServerMessage {
    Response { id: RequestId, response: Response },
    Event { event: Event },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    DocOpened {
        info: DocInfo,
    },
    DocClosed {
        doc: DocId,
    },
    JobProgress {
        job: JobId,
        doc: DocId,
        done: u64,
        total: u64,
    },
    JobDone {
        info: JobInfo,
    },
    /// The document changed: everything read from it before is stale.
    DocChanged {
        doc: DocId,
        reason: ChangeReason,
        /// The document as reopened (`Saved`).
        info: Option<DocInfo>,
    },
}

/// Binary payload of a response: `id` (u64 LE) + `part` (u32 LE) + bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryFrame {
    pub id: RequestId,
    pub part: u32,
    pub data: Vec<u8>,
}

impl BinaryFrame {
    pub const HEADER_LEN: usize = 12;

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::HEADER_LEN + self.data.len());
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.part.to_le_bytes());
        out.extend_from_slice(&self.data);
        out
    }

    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::HEADER_LEN {
            return None;
        }
        let id = u64::from_le_bytes(buf[..8].try_into().ok()?);
        let part = u32::from_le_bytes(buf[8..12].try_into().ok()?);
        Some(Self {
            id,
            part,
            data: buf[12..].to_vec(),
        })
    }
}

/// What `dispatch` produces: the response plus the binary parts it refers to
/// (indexed by [`BytesInfo::part`]).
#[derive(Debug)]
pub struct Reply {
    pub response: Response,
    pub blobs: Vec<Vec<u8>>,
}

impl Reply {
    /// Frames to send for request `id`, binary parts first.
    pub fn into_frames(self, id: RequestId) -> (Vec<BinaryFrame>, ServerMessage) {
        let frames = self
            .blobs
            .into_iter()
            .enumerate()
            .map(|(i, data)| BinaryFrame {
                id,
                part: i as u32,
                data,
            })
            .collect();
        (
            frames,
            ServerMessage::Response {
                id,
                response: self.response,
            },
        )
    }
}
