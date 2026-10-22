import type { Event, Request, ResponseBody } from "../protocol/generated";

/** A response body plus the binary parts it refers to (by `BytesInfo.part`). */
export interface Reply {
  body: ResponseBody;
  blobs: Uint8Array[];
}

export type ConnectionState = "connecting" | "open" | "closed";

/**
 * The only way the UI talks to a backend. Implemented by `RemoteBackend`
 * (WebSocket to `pdfi serve`) and, later, `WorkerBackend` (wasm in a Worker).
 */
export interface Backend {
  readonly mode: "server" | "browser";
  request(req: Request, signal?: AbortSignal): Promise<Reply>;
  onEvent(cb: (ev: Event) => void): () => void;
  onState(cb: (s: ConnectionState) => void): () => void;
}

export class ApiFailure extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}
