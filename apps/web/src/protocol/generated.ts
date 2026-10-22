// Generated from crates/pdfi-api by `UPDATE_TS=1 cargo test -p pdfi-api`. Do not edit.

export const PROTOCOL_VERSION = 2;

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
pdf_version: string | null, };

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

export type JobKind = { "kind": "checkObjects", decode_streams: boolean, } | { "kind": "validate", root: ObjRef | null, };

export type JobState = "running" | "completed" | "cancelled";

export type JobInfo = { job: number, doc: number, kind: JobKind, state: JobState, done: number, total: number, result: JobResult | null, };

export type JobResult = { "kind": "checkObjects", ok: number, failed: number, missing: number, streams: number, decoded_bytes: number, decode_errors: number, diagnostics: number, } | { "kind": "validate", objects: number, errors: number, warnings: number, infos: number, 
/**
 * Issues kept for `JobDiagnostics` (the first ones, when there are
 * very many).
 */
kept: number, };

export type ClientMessage = { id: number, request: Request, };

export type Request = { "op": "hello", protocol: number, } | { "op": "listDir", path: string | null, } | { "op": "open", source: OpenSource, password: string | null, 
/**
 * Ignore the cross-reference data and rebuild it by scanning.
 */
recover: boolean, } | { "op": "docs" } | { "op": "close", doc: number, } | { "op": "summary", doc: number, } | { "op": "objects", doc: number, range: Range, } | { "op": "object", doc: number, id: ObjRef, } | { "op": "children", doc: number, path: NodePath, range: Range, } | { "op": "xref", doc: number, revision: number | null, range: Range, } | { "op": "diagnostics", doc: number, range: Range, } | { "op": "schema", doc: number, root: NodeRoot, } | { "op": "rawBytes", doc: number, offset: number, len: number, } | { "op": "streamData", doc: number, id: ObjRef, 
/**
 * Decrypt and apply filters; otherwise the bytes as stored.
 */
decoded: boolean, offset: number, len: number, } | { "op": "startJob", doc: number, job: JobKind, } | { "op": "jobStatus", job: number, } | { "op": "cancelJob", job: number, } | { "op": "jobDiagnostics", job: number, range: Range, } | { "op": "cancel", request: number, } | { "op": "batch", requests: Array<Request>, };

export type Response = { "ok": ResponseBody } | { "err": ApiError };

export type ResponseBody = { "type": "hello" } & HelloInfo | { "type": "dir" } & DirListing | { "type": "doc" } & DocSummary | { "type": "docs", docs: Array<DocInfo>, } | { "type": "objects" } & Page<ObjectSummary> | { "type": "object" } & ObjectView | { "type": "children" } & Page<NodeView> | { "type": "xref" } & Page<XrefRow> | { "type": "diagnostics" } & Page<DiagnosticInfo> | { "type": "schema" } & ObjectSchema | { "type": "bytes" } & BytesInfo | { "type": "job" } & JobInfo | { "type": "batch", responses: Array<Response>, } | { "type": "empty" };

export type HelloInfo = { protocol: number, backend: string, version: string, 
/**
 * Backend can open paths (`OpenSource::Path`) and list directories.
 */
filesystem: boolean, };

export type ErrorCode = "badRequest" | "versionMismatch" | "noSuchDoc" | "noSuchJob" | "notFound" | "forbidden" | "unsupported" | "tooLarge" | "io" | "parse" | "cancelled" | "internal";

export type ApiError = { code: ErrorCode, message: string, };

export type ServerMessage = { "kind": "response", id: number, response: Response, } | { "kind": "event", event: Event, };

export type Event = { "type": "docOpened", info: DocInfo, } | { "type": "docClosed", doc: number, } | { "type": "jobProgress", job: number, doc: number, done: number, total: number, } | { "type": "jobDone", info: JobInfo, };
