import type { DocId, Event, Request, ResponseBody, SaveMode } from "../protocol/generated";

/** A response body plus the binary parts it refers to (by `BytesInfo.part`). */
export interface Reply {
  body: ResponseBody;
  blobs: Uint8Array[];
}

export type ConnectionState = "connecting" | "open" | "closed";

/**
 * The only way the UI talks to a backend. Implemented by `RemoteBackend`
 * (WebSocket to `pdfi serve`) and `WorkerBackend` (wasm in a Worker).
 */
export interface Backend {
  readonly mode: "server" | "browser";
  request(req: Request, signal?: AbortSignal): Promise<Reply>;
  onEvent(cb: (ev: Event) => void): () => void;
  onState(cb: (s: ConnectionState) => void): () => void;
  /** Open a local file (browser mode; the server opens paths instead). Answers like `open`. */
  openFile?(file: File, password: string | null, recover: boolean): Promise<Reply>;
  /**
   * Write a document with its edits to a file the user picked, then reopen
   * it from there (browser mode; the server saves with a `save` job).
   * Answers `saved`.
   */
  saveFile?(doc: DocId, mode: SaveMode, handle: FileSystemFileHandle): Promise<Reply>;
}

export class ApiFailure extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}
