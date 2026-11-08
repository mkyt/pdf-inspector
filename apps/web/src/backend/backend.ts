import type { DocId, Event, Request, ResponseBody, SaveMode } from "../protocol/generated";

/** A response body plus the binary parts it refers to (by `BytesInfo.part`). */
export interface Reply {
  body: ResponseBody;
  blobs: Uint8Array[];
  /** A save without a file handle: the written file, to download. */
  file?: File;
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
   * Answers `saved`. With `{ download }` instead of a handle (no File System
   * Access API) the file is written in memory and comes back as `Reply.file`.
   */
  saveFile?(doc: DocId, mode: SaveMode, handle: FileSystemFileHandle | { download: string }): Promise<Reply>;
  /**
   * Give a macro Worker its own line to the session (browser mode, where
   * macros run in the page): requests sent on `port` are answered there.
   */
  connectPort?(port: MessagePort): void;
}

export class ApiFailure extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}
