// Runs a wasm session inside a Worker: queues requests (so `cancel` can drop
// ones that have not started) and advances jobs between messages.
//
// Only type imports: the wasm conformance test runs this file directly in
// Node (scripts/conformance.ts).
import type { ClientMessage, RequestId, ServerMessage } from "../protocol/generated";
import type { JsByteSource, WasmReply, WasmSession } from "../wasm/pdfi_wasm";

/** Main thread → Worker. */
export type Inbound =
  | { kind: "message"; message: ClientMessage }
  /** Answered like `Request::Open`; files cannot travel inside JSON. */
  | { kind: "open"; id: RequestId; file: Blob; name: string; password: string | null; recover: boolean };

/** Worker → main thread: a JSON `ServerMessage` plus the binary parts of a response. */
export type Outbound = { text: string; blobs: Uint8Array[] } | { fatal: string };

export interface HostOptions {
  session: Pick<WasmSession, "dispatch" | "open" | "runJobStep" | "takeEvents">;
  post: (out: Outbound) => void;
  /** Synchronous random access to a file (Worker: `FileReaderSync`). */
  source: (file: Blob) => JsByteSource;
  /** Run `fn` as a new task, after pending messages. */
  schedule: (fn: () => void) => void;
  /** Time spent on jobs per turn. */
  jobSliceMs?: number;
}

function idOf(m: Inbound): RequestId {
  return m.kind === "open" ? m.id : m.message.id;
}

export class Host {
  private queue: Inbound[] = [];
  private scheduled = false;
  private jobs = false;
  private opts: HostOptions;

  constructor(opts: HostOptions) {
    this.opts = opts;
  }

  receive(m: Inbound) {
    if (m.kind === "message" && m.message.request.op === "cancel") {
      const target = m.message.request.request;
      const i = this.queue.findIndex((q) => idOf(q) === target);
      if (i >= 0) {
        this.queue.splice(i, 1);
        this.send({ kind: "response", id: target, response: { err: { code: "cancelled", message: "cancelled" } } });
      }
      this.send({ kind: "response", id: m.message.id, response: { ok: { type: "empty" } } });
      return;
    }
    this.queue.push(m);
    this.kick();
  }

  private send(msg: ServerMessage) {
    this.opts.post({ text: JSON.stringify(msg), blobs: [] });
  }

  private kick() {
    if (this.scheduled) return;
    this.scheduled = true;
    this.opts.schedule(() => this.turn());
  }

  /** One request, then a slice of job work, then yield to new messages. */
  private turn() {
    this.scheduled = false;
    const { session } = this.opts;
    const m = this.queue.shift();
    if (m) {
      let reply: WasmReply;
      if (m.kind === "open") {
        reply = session.open(m.id, this.opts.source(m.file), m.name, m.password, m.recover);
      } else {
        reply = session.dispatch(JSON.stringify(m.message));
      }
      const out = { text: reply.message, blobs: reply.blobs };
      reply.free();
      // Events raised by the request (e.g. docOpened) precede its response,
      // as on the WebSocket.
      this.flushEvents();
      this.opts.post(out);
      this.jobs = true;
    }
    if (this.jobs) {
      const end = performance.now() + (this.opts.jobSliceMs ?? 16);
      do {
        this.jobs = session.runJobStep();
      } while (this.jobs && performance.now() < end);
      this.flushEvents();
    }
    if (this.queue.length > 0 || this.jobs) this.kick();
  }

  private flushEvents() {
    for (const text of this.opts.session.takeEvents()) this.opts.post({ text, blobs: [] });
  }
}
