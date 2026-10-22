import { createContext, useContext } from "react";
import type { Backend, Reply } from "./backend/backend";
import type {
  BytesInfo,
  DirListing,
  DocId,
  DocInfo,
  DocSummary,
  DiagnosticInfo,
  JobInfo,
  JobKind,
  NodePath,
  NodeRoot,
  NodeView,
  ObjectSchema,
  ObjRef,
  ObjectSummary,
  ObjectView,
  Page,
  Range,
  Request,
  ResponseBody,
  XrefRow,
} from "./protocol/generated";

export const BackendContext = createContext<Backend | null>(null);

export function useBackend(): Backend {
  const b = useContext(BackendContext);
  if (!b) throw new Error("no backend");
  return b;
}

type Body<T extends ResponseBody["type"]> = Extract<ResponseBody, { type: T }>;

async function call<T extends ResponseBody["type"]>(
  b: Backend,
  type: T,
  req: Request,
  signal?: AbortSignal,
): Promise<{ body: Body<T>; blobs: Reply["blobs"] }> {
  const r = await b.request(req, signal);
  if (r.body.type !== type) throw new Error(`expected ${type}, got ${r.body.type}`);
  return { body: r.body as Body<T>, blobs: r.blobs };
}

export interface Bytes {
  info: BytesInfo;
  data: Uint8Array;
}

function bytes(r: { body: Body<"bytes">; blobs: Uint8Array[] }): Bytes {
  return { info: r.body, data: r.blobs[r.body.part] ?? new Uint8Array() };
}

/** Typed wrappers over the protocol. */
export const api = {
  listDir: async (b: Backend, path: string | null): Promise<DirListing> =>
    (await call(b, "dir", { op: "listDir", path })).body,
  openPath: async (
    b: Backend,
    path: string,
    password: string | null,
    recover: boolean,
  ): Promise<DocSummary> =>
    (await call(b, "doc", { op: "open", source: { kind: "path", path }, password, recover })).body,
  openFile: async (b: Backend, file: File, password: string | null, recover: boolean): Promise<DocSummary> => {
    if (!b.openFile) throw new Error("this backend opens paths, not files");
    const r = await b.openFile(file, password, recover);
    if (r.body.type !== "doc") throw new Error(`expected doc, got ${r.body.type}`);
    return r.body;
  },
  docs: async (b: Backend): Promise<DocInfo[]> => (await call(b, "docs", { op: "docs" })).body.docs,
  close: async (b: Backend, doc: DocId) => void (await b.request({ op: "close", doc })),
  summary: async (b: Backend, doc: DocId): Promise<DocSummary> =>
    (await call(b, "doc", { op: "summary", doc })).body,
  objects: async (b: Backend, doc: DocId, range: Range, signal?: AbortSignal): Promise<Page<ObjectSummary>> =>
    (await call(b, "objects", { op: "objects", doc, range }, signal)).body,
  object: async (b: Backend, doc: DocId, id: ObjRef): Promise<ObjectView> =>
    (await call(b, "object", { op: "object", doc, id })).body,
  children: async (b: Backend, doc: DocId, path: NodePath, range: Range): Promise<Page<NodeView>> =>
    (await call(b, "children", { op: "children", doc, path, range })).body,
  xref: async (b: Backend, doc: DocId, revision: number | null, range: Range, signal?: AbortSignal): Promise<Page<XrefRow>> =>
    (await call(b, "xref", { op: "xref", doc, revision, range }, signal)).body,
  diagnostics: async (b: Backend, doc: DocId, range: Range, signal?: AbortSignal): Promise<Page<DiagnosticInfo>> =>
    (await call(b, "diagnostics", { op: "diagnostics", doc, range }, signal)).body,
  schema: async (b: Backend, doc: DocId, root: NodeRoot): Promise<ObjectSchema> =>
    (await call(b, "schema", { op: "schema", doc, root })).body,
  jobDiagnostics: async (b: Backend, job: number, range: Range, signal?: AbortSignal): Promise<Page<DiagnosticInfo>> =>
    (await call(b, "diagnostics", { op: "jobDiagnostics", job, range }, signal)).body,
  rawBytes: async (b: Backend, doc: DocId, offset: number, len: number, signal?: AbortSignal): Promise<Bytes> =>
    bytes(await call(b, "bytes", { op: "rawBytes", doc, offset, len }, signal)),
  streamData: async (
    b: Backend,
    doc: DocId,
    id: ObjRef,
    decoded: boolean,
    offset: number,
    len: number,
  ): Promise<Bytes> => bytes(await call(b, "bytes", { op: "streamData", doc, id, decoded, offset, len })),
  startJob: async (b: Backend, doc: DocId, job: JobKind): Promise<JobInfo> =>
    (await call(b, "job", { op: "startJob", doc, job })).body,
  jobStatus: async (b: Backend, job: number): Promise<JobInfo> =>
    (await call(b, "job", { op: "jobStatus", job })).body,
  cancelJob: async (b: Backend, job: number): Promise<JobInfo> =>
    (await call(b, "job", { op: "cancelJob", job })).body,
};
