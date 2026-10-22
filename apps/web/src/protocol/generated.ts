// Generated from crates/pdfi-api by `UPDATE_TS=1 cargo test -p pdfi-api`. Do not edit.

export const PROTOCOL_VERSION = 5;

export type DocId = number;
export type JobId = number;
export type RequestId = number;

export type ObjRef = { num: number, generation: number, };

export type Span = { start: number, end: number, };

export type Range = { offset: number, limit: number, };

export type Page<T> = { total: number, offset: number, items: Array<T>, };

export type OpenSource = { "kind": "path", path: string, };

export type DirEntry = { name: string, path: string, is_dir: boolean, size: number | null, };

export type DirListing = { 
/**
 * `None` for the list of allowed roots.
 */
path: string | null, parent: string | null, entries: Array<DirEntry>, };

export type DocInfo = { doc: number, name: string, path: string | null, size: number, };

export type DocSummary = { info: DocInfo, version: string | null, header_offset: number, 
/**
 * Number of object numbers covered by the cross-reference data.
 */
object_slots: number, 
/**
 * Rows returned by `Objects` (entries present in the xref, free included).
 */
xref_entries: number, live_objects: number, recovered: boolean, revisions: Array<RevisionInfo>, root: ObjRef | null, info_dict: ObjRef | null, encryption: EncryptionInfo | null, diagnostics: number, 
/**
 * Effective version: the header's, or the catalog's `/Version` if later.
 */
pdf_version: string | null, 
/**
 * Number of pages, by the page tree's counts.
 */
pages: number, 
/**
 * What the inventory job found, once it has run.
 */
inventory: InventoryStats | null, };

export type EncryptionInfo = { filter: string, v: number, r: number, key_bits: number, stream_method: string, string_method: string, permissions: number, authenticated: boolean, };

export type XrefKind = "table" | "stream" | "hybrid" | "recovered";

export type RevisionInfo = { 
/**
 * 0 = newest.
 */
index: number, kind: XrefKind, offset: number, span: Span, entries: number, xref_stream: ObjRef | null, };

export type XrefEntry = { "kind": "free", next: number, generation: number, } | { "kind": "inUse", offset: number, generation: number, } | { "kind": "compressed", stream: number, index: number, };

export type XrefRow = { num: number, entry: XrefEntry, };

export type ObjectSummary = { num: number, entry: XrefEntry, 
/**
 * Object type (`dictionary`, `stream`, ...) when it loads.
 */
kind: string | null, 
/**
 * `/Type` and `/Subtype` of dictionaries and streams.
 */
type_name: string | null, subtype: string | null, 
/**
 * Schema type, learned from references or guessed from `/Type`.
 */
schema_type: string | null, error: string | null, };

export type NodeRoot = { "kind": "trailer" } | { "kind": "object", id: ObjRef, };

export type NodePath = { root: NodeRoot, path: Array<number>, };

export type StringKind = "literal" | "hex";

export type Value = { "type": "null" } | { "type": "bool", value: boolean, } | { "type": "integer", value: number, } | { "type": "real", value: number, } | { "type": "string", kind: StringKind, len: number, 
/**
 * Text decoded as UTF-16BE / UTF-8 (with BOM) or PDFDocEncoding,
 * truncated.
 */
text: string, 
/**
 * Raw bytes in hex, truncated.
 */
hex: string, truncated: boolean, } | { "type": "name", 
/**
 * `#xx`-escaped as in PDF syntax, without the leading `/`.
 */
name: string, } | { "type": "array", len: number, } | { "type": "dict", len: number, type_name: string | null, } | { "type": "stream", len: number, type_name: string | null, raw_len: number, } | { "type": "ref", id: ObjRef, };

export type NodeView = { index: number, 
/**
 * Dictionary key (`#xx`-escaped, without `/`); `None` for array items.
 */
key: string | null, value: Value, 
/**
 * Location in the source file. `None` for objects inside object streams
 * and for synthetic nodes.
 */
span: Span | null, 
/**
 * What the schema says about this entry, when the container's type is
 * known.
 */
schema: NodeSchema | null, };

export type Location = { "kind": "file", span: Span, } | { "kind": "objStm", stream: ObjRef, index: number, };

export type StreamInfo = { 
/**
 * Encoded data in the source file.
 */
data_span: Span | null, raw_len: number, filters: Array<string>, encrypted: boolean, };

export type ObjectView = { id: ObjRef, location: Location, value: Value, span: Span | null, stream: StreamInfo | null, diagnostics: Array<DiagnosticInfo>, };

export type Severity = "info" | "warning" | "error";

export type DiagnosticInfo = { severity: Severity, code: string, message: string, obj: ObjRef | null, 
/**
 * Location inside `obj` (or the trailer), like `/Pages/Kids[3]`.
 */
path: string | null, span: Span | null, };

export type TypeOrigin = "context" | "guess";

export type ObjectSchema = { 
/**
 * `None` when no type is known.
 */
type_name: string | null, origin: TypeOrigin | null, 
/**
 * Where the specification defines it (e.g. `Table 29`).
 */
note: string | null, 
/**
 * Version the document is checked against.
 */
pdf_version: string | null, 
/**
 * Problems of the object and of the direct objects inside it.
 */
issues: Array<DiagnosticInfo>, 
/**
 * Some large container was checked only in part.
 */
truncated: boolean, };

export type Requirement = { "kind": "required" } | { "kind": "optional" } | { "kind": "conditional", condition: string, };

export type SlotDoc = { 
/**
 * Arlington type: `dictionary`, `name`, `rectangle`, `string-text`, ...
 */
base: string, 
/**
 * Types a dictionary, array or stream of this slot may have.
 */
links: Array<string>, };

export type KeyDoc = { types: Array<SlotDoc>, requirement: Requirement, since: string | null, deprecated: string | null, extension: string | null, inheritable: boolean, default: string | null, 
/**
 * Allowed values, when the schema lists them (truncated).
 */
allowed: Array<string>, note: string | null, };

export type NodeSchema = { 
/**
 * `None` for keys the schema does not define.
 */
key: KeyDoc | null, 
/**
 * Type of the value: of a direct dictionary / array, or of the
 * referenced object.
 */
value_type: string | null, 
/**
 * Problems of this entry (and, for a direct container, of its own keys).
 */
issues: Array<DiagnosticInfo>, };

export type Rect = { x0: number, y0: number, x1: number, y1: number, };

export type AttrOrigin = { "kind": "page" } | { "kind": "inherited", from: ObjRef, } | { "kind": "default", of: string, } | { "kind": "missing" };

export type PageSummary = { 
/**
 * 0-based.
 */
index: number, id: ObjRef | null, label: string | null, 
/**
 * The visible area (crop box clipped to the media box).
 */
size: Rect | null, rotate: number, annots: number, error: string | null, };

export type PageBox = { name: string, rect: Rect | null, origin: AttrOrigin, };

export type ContentInfo = { id: ObjRef | null, raw_len: number | null, filters: Array<string>, error: string | null, };

export type ResourceItem = { category: string, name: string, id: ObjRef | null, 
/**
 * `Type1 Helvetica`, `Image 640×480 DeviceRGB`, ...
 */
detail: string, };

export type PageDetail = { index: number, id: ObjRef, label: string | null, 
/**
 * Page tree nodes from the root down to the page's parent.
 */
ancestors: Array<ObjRef>, 
/**
 * MediaBox, CropBox, BleedBox, TrimBox, ArtBox.
 */
boxes: Array<PageBox>, rotate: number, rotate_origin: AttrOrigin, user_unit: number, 
/**
 * The resource dictionary, when it is an indirect object.
 */
resources: ObjRef | null, resources_origin: AttrOrigin, resource_items: Array<ResourceItem>, resources_truncated: boolean, contents: Array<ContentInfo>, annots: number, issues: Array<DiagnosticInfo>, };

export type Destination = { 
/**
 * The name it was looked up by (named destinations).
 */
named: string | null, page: ObjRef | null, page_index: number | null, 
/**
 * Page number in another file (`GoToR`).
 */
remote_page: number | null, 
/**
 * `XYZ`, `Fit`, `FitH`, ...
 */
fit: string | null, 
/**
 * Parameters after the fit type; `null` for null.
 */
params: Array<number | null>, error: string | null, };

export type LinkTarget = { "kind": "dest", dest: Destination, } | { "kind": "uri", uri: string, } | { "kind": "action", action: string, detail: string | null, };

export type OutlineItem = { id: ObjRef, title: string, 
/**
 * Open descendants when positive; closed when negative.
 */
count: number | null, has_children: boolean, link: LinkTarget | null, 
/**
 * 1 italic, 2 bold.
 */
flags: number, 
/**
 * RGB, 0..1.
 */
color: Array<number> | null, };

export type AnnotInfo = { 
/**
 * Position in the page's `/Annots`.
 */
index: number, id: ObjRef | null, subtype: string | null, rect: Rect | null, contents: string | null, 
/**
 * `/NM`.
 */
name: string | null, flags: number, 
/**
 * Has a normal appearance stream.
 */
appearance: boolean, link: LinkTarget | null, 
/**
 * `/Parent` of a pop-up, `/IRT` of a reply.
 */
parent: ObjRef | null, error: string | null, };

export type InfoEntry = { key: string, text: string, 
/**
 * ISO 8601, for values that are PDF dates.
 */
date: string | null, };

export type Feature = { key: string, text: string, obj: ObjRef | null, };

export type DocumentView = { catalog: ObjRef | null, info: ObjRef | null, info_entries: Array<InfoEntry>, 
/**
 * The catalog's XMP metadata stream.
 */
metadata: ObjRef | null, pages: number, features: Array<Feature>, 
/**
 * Problems of the page tree, outline and metadata (also Info / XMP
 * disagreements).
 */
issues: Array<DiagnosticInfo>, };

export type XmpNodeKind = "element" | "attribute";

export type XmpNode = { depth: number, kind: XmpNodeKind, name: string, namespace: string | null, value: string | null, };

export type XmpProperty = { name: string, namespace: string, value: string, };

export type XmpView = { id: ObjRef, 
/**
 * Length of the decoded packet.
 */
len: number, nodes: Page<XmpNode>, 
/**
 * The tree has more nodes than were kept.
 */
truncated: boolean, properties: Array<XmpProperty>, error: string | null, };

export type FontInfo = { id: ObjRef, base_font: string | null, subtype: string | null, encoding: string | null, 
/**
 * `FontFile`, `FontFile2`, `FontFile3/Type1C`, ...; `None`: not
 * embedded.
 */
embedded: string | null, font_file: ObjRef | null, to_unicode: boolean, descendant: ObjRef | null, subset: boolean, 
/**
 * One of the standard 14 fonts.
 */
standard: boolean, 
/**
 * Pages using it through their resources.
 */
page_count: number, 
/**
 * The first of those pages (0-based).
 */
pages: Array<number>, };

export type ImageInfo = { id: ObjRef, width: number | null, height: number | null, bits: number | null, color_space: string | null, filters: Array<string>, raw_len: number, image_mask: boolean, smask: ObjRef | null, mask: boolean, interpolate: boolean, page_count: number, pages: Array<number>, };

export type Referrer = { from: NodeRoot, 
/**
 * Schema type, or `/Type`.
 */
type_name: string | null, 
/**
 * Where in it: `/Resources/Font/F1`, ...
 */
paths: Array<string>, };

export type ContentRef = { "kind": "page", index: number, } | { "kind": "stream", id: ObjRef, };

export type ContentKind = "page" | "form" | "pattern" | "other";

export type ContentSourceInfo = { id: ObjRef | null, 
/**
 * Decoded bytes parsed.
 */
len: number, error: string | null, };

export type CategoryCount = { 
/**
 * Operator group: `pathConstruction`, `textShowing`, ...
 */
category: string, count: number, };

export type ContentSummary = { content: ContentRef, kind: ContentKind, 
/**
 * The page or stream object.
 */
id: ObjRef, 
/**
 * The streams parsed, in order (several for a `/Contents` array).
 */
sources: Array<ContentSourceInfo>, ops: number, 
/**
 * Stopped at the operation limit.
 */
truncated: boolean, 
/**
 * Operations that paint something (see `ContentElements`).
 */
elements: number, issues: number, errors: number, warnings: number, 
/**
 * Issues beyond the kept ones.
 */
issues_dropped: number, 
/**
 * The page's visible area, or the form's `/BBox`; the initial clip.
 */
bounds: Rect | null, 
/**
 * The resource dictionary, when it is an indirect object.
 */
resources: ObjRef | null, has_resources: boolean, 
/**
 * Some text bounding boxes rest on guessed glyph widths.
 */
approximate: boolean, counts: Array<CategoryCount>, 
/**
 * What `Render` shows.
 */
view: ViewInfo, };

export type ViewInfo = { 
/**
 * In points.
 */
width: number, height: number, 
/**
 * The content's space to the view, `[a b c d e f]`: origin at the top
 * left, y down.
 */
matrix: Array<number>, rotate: number, };

export type PixelRect = { x: number, y: number, width: number, height: number, };

export type RenderLayer = { "kind": "content" } | { "kind": "highlight", ops: Array<number>, };

export type RenderNote = { 
/**
 * What was approximated or left out.
 */
message: string, count: number, };

export type BitmapInfo = { part: number, 
/**
 * The region of the whole view's bitmap this is.
 */
rect: PixelRect, 
/**
 * Size of the whole view's bitmap at this scale.
 */
full_width: number, full_height: number, scale: number, 
/**
 * Operations run.
 */
ops: number, notes: Array<RenderNote>, };

export type ContentIssue = { op: number | null, severity: Severity, code: string, message: string, 
/**
 * For syntax problems: the stream and the bytes in its decoded data.
 */
source: number | null, span: Span | null, };

export type ElementView = { op: number, 
/**
 * `fill`, `stroke`, `fillStroke`, `clip`, `text`, `image`,
 * `inlineImage`, `form`, `shading`.
 */
kind: string, bbox: Rect, 
/**
 * Outside the clipping path, or invisible text.
 */
hidden: boolean, };

export type OpView = { index: number, operator: string, 
/**
 * Operator group (see [`CategoryCount`]); `unknown` for unknown ones.
 */
category: string, 
/**
 * In PDF syntax, shortened.
 */
operands: Array<string>, 
/**
 * Nesting of `q`, `BT` and marked content; closers at their opener's.
 */
depth: number, 
/**
 * The stream it was read from and its bytes in that stream's decoded
 * data.
 */
source: number | null, span: Span | null, 
/**
 * Decoded text of text-showing operators, when the font tells it.
 */
text: string | null, 
/**
 * The resource it names (`Tf`, `Do`, `gs`, `sh`, `cs`, `scn`, `BDC`),
 * when it is an indirect object.
 */
resource: ObjRef | null, element: ElementView | null, issues: Array<ContentIssue>, };

export type ColorView = { 
/**
 * As set: `DeviceRGB`, `Pattern`, or a `/ColorSpace` resource name.
 */
space: string, family: string, components: Array<number>, pattern: string | null, };

export type FontView = { 
/**
 * Resource name (`ExtGState` when set by `gs`).
 */
name: string, id: ObjRef | null, base_font: string | null, subtype: string | null, 
/**
 * Widths are partly guessed.
 */
approximate: boolean, };

export type GraphicsState = { 
/**
 * `[a b c d e f]`.
 */
ctm: Array<number>, 
/**
 * Bounding box of the clipping path (`None`: not clipped).
 */
clip: Rect | null, line_width: number, line_cap: number, line_join: number, miter_limit: number, dash: Array<number>, dash_phase: number, intent: string, flatness: number, stroke: ColorView, fill: ColorView, stroke_alpha: number, fill_alpha: number, blend: string, soft_mask: string | null, ext_gstates: Array<string>, font: FontView | null, font_size: number, char_spacing: number, word_spacing: number, 
/**
 * Percent.
 */
horizontal_scale: number, leading: number, render_mode: number, rise: number, in_text: boolean, 
/**
 * Text matrix and text line matrix (inside a text object).
 */
text_matrix: Array<number> | null, line_matrix: Array<number> | null, 
/**
 * Saved states (`q` depth).
 */
saved: number, 
/**
 * Open marked-content tags, outermost first.
 */
marked: Array<string>, 
/**
 * Bounding box of the path being built, and its current point (user
 * space).
 */
path: Rect | null, current_point: Array<number> | null, };

export type OpState = { index: number, operator: string, description: string, before: GraphicsState, after: GraphicsState, };

export type BytesInfo = { part: number, offset: number, len: number, 
/**
 * Length of the whole data, when known without decoding all of it.
 */
total: number | null, 
/**
 * The data ends within this chunk.
 */
eof: boolean, 
/**
 * Filter left undecoded (image codecs, unsupported filters).
 */
stopped_at: string | null, errors: Array<string>, };

export type JobKind = { "kind": "checkObjects", decode_streams: boolean, } | { "kind": "validate", root: ObjRef | null, } | { "kind": "inventory" };

export type JobState = "running" | "completed" | "cancelled";

export type JobInfo = { job: number, doc: number, kind: JobKind, state: JobState, done: number, total: number, result: JobResult | null, };

export type JobResult = { "kind": "checkObjects", ok: number, failed: number, missing: number, streams: number, decoded_bytes: number, decode_errors: number, diagnostics: number, } | { "kind": "validate", objects: number, errors: number, warnings: number, infos: number, 
/**
 * Issues kept for `JobDiagnostics` (the first ones, when there are
 * very many).
 */
kept: number, } | { "kind": "inventory" } & InventoryStats;

export type InventoryStats = { objects: number, 
/**
 * Objects that failed to load.
 */
failed: number, pages: number, fonts: number, images: number, forms: number, 
/**
 * Edges of the reference graph.
 */
references: number, unreferenced: number, };

export type ClientMessage = { id: number, request: Request, };

export type Request = { "op": "hello", protocol: number, } | { "op": "listDir", path: string | null, } | { "op": "open", source: OpenSource, password: string | null, 
/**
 * Ignore the cross-reference data and rebuild it by scanning.
 */
recover: boolean, } | { "op": "docs" } | { "op": "close", doc: number, } | { "op": "summary", doc: number, } | { "op": "objects", doc: number, range: Range, } | { "op": "object", doc: number, id: ObjRef, } | { "op": "children", doc: number, path: NodePath, range: Range, } | { "op": "xref", doc: number, revision: number | null, range: Range, } | { "op": "diagnostics", doc: number, range: Range, } | { "op": "schema", doc: number, root: NodeRoot, } | { "op": "document", doc: number, } | { "op": "xmp", doc: number, range: Range, } | { "op": "pages", doc: number, range: Range, } | { "op": "page", doc: number, index: number, } | { "op": "pageIndex", doc: number, id: ObjRef, } | { "op": "annotations", doc: number, page: number, range: Range, } | { "op": "outline", doc: number, parent: ObjRef | null, range: Range, } | { "op": "fonts", doc: number, range: Range, } | { "op": "images", doc: number, range: Range, } | { "op": "referrers", doc: number, id: ObjRef, range: Range, } | { "op": "unreferenced", doc: number, range: Range, } | { "op": "content", doc: number, content: ContentRef, } | { "op": "contentOps", doc: number, content: ContentRef, range: Range, } | { "op": "contentIssues", doc: number, content: ContentRef, range: Range, } | { "op": "contentElements", doc: number, content: ContentRef, range: Range, } | { "op": "opState", doc: number, content: ContentRef, index: number, } | { "op": "render", doc: number, content: ContentRef, scale: number, 
/**
 * `None`: the whole view.
 */
tile: PixelRect | null, 
/**
 * Run operations up to and including this one.
 */
until: number | null, layer: RenderLayer, } | { "op": "hitTest", doc: number, content: ContentRef, x: number, y: number, radius: number, until: number | null, } | { "op": "rawBytes", doc: number, offset: number, len: number, } | { "op": "streamData", doc: number, id: ObjRef, 
/**
 * Decrypt and apply filters; otherwise the bytes as stored.
 */
decoded: boolean, offset: number, len: number, } | { "op": "startJob", doc: number, job: JobKind, } | { "op": "jobStatus", job: number, } | { "op": "cancelJob", job: number, } | { "op": "jobDiagnostics", job: number, range: Range, } | { "op": "cancel", request: number, } | { "op": "batch", requests: Array<Request>, };

export type Response = { "ok": ResponseBody } | { "err": ApiError };

export type ResponseBody = { "type": "hello" } & HelloInfo | { "type": "dir" } & DirListing | { "type": "doc" } & DocSummary | { "type": "docs", docs: Array<DocInfo>, } | { "type": "objects" } & Page<ObjectSummary> | { "type": "object" } & ObjectView | { "type": "children" } & Page<NodeView> | { "type": "xref" } & Page<XrefRow> | { "type": "diagnostics" } & Page<DiagnosticInfo> | { "type": "schema" } & ObjectSchema | { "type": "document" } & DocumentView | { "type": "xmp" } & XmpView | { "type": "pages" } & Page<PageSummary> | { "type": "pageDetail" } & PageDetail | { "type": "pageIndex", index: number | null, } | { "type": "annotations" } & Page<AnnotInfo> | { "type": "outline" } & Page<OutlineItem> | { "type": "fonts" } & Page<FontInfo> | { "type": "images" } & Page<ImageInfo> | { "type": "referrers" } & Page<Referrer> | { "type": "objRefs" } & Page<ObjRef> | { "type": "content" } & ContentSummary | { "type": "ops" } & Page<OpView> | { "type": "contentIssues" } & Page<ContentIssue> | { "type": "elements" } & Page<ElementView> | { "type": "opState" } & OpState | { "type": "bitmap" } & BitmapInfo | { "type": "hits", ops: Array<number>, } | { "type": "bytes" } & BytesInfo | { "type": "job" } & JobInfo | { "type": "batch", responses: Array<Response>, } | { "type": "empty" };

export type HelloInfo = { protocol: number, backend: string, version: string, 
/**
 * Backend can open paths (`OpenSource::Path`) and list directories.
 */
filesystem: boolean, };

export type ErrorCode = "badRequest" | "versionMismatch" | "noSuchDoc" | "noSuchJob" | "notFound" | "forbidden" | "unsupported" | "tooLarge" | "notReady" | "io" | "parse" | "cancelled" | "internal";

export type ApiError = { code: ErrorCode, message: string, };

export type ServerMessage = { "kind": "response", id: number, response: Response, } | { "kind": "event", event: Event, };

export type Event = { "type": "docOpened", info: DocInfo, } | { "type": "docClosed", doc: number, } | { "type": "jobProgress", job: number, doc: number, done: number, total: number, } | { "type": "jobDone", info: JobInfo, };
