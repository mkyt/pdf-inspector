// Runs a wasm session inside a Worker: queues requests (so `cancel` can drop
// ones that have not started) and advances jobs between messages.
//
// Only type imports: the wasm conformance test runs this file directly in
// Node (scripts/conformance.ts).
import type { ClientMessage, DocId, RequestId, SaveMode, SaveResult, ServerMessage } from "../protocol/generated";
import type { JsByteSource, WasmReply, WasmSession } from "../wasm/pdfi_wasm";

/** Main thread → Worker. */
export type Inbound =
  | { kind: "message"; message: ClientMessage }
  /** Answered like `Request::Open`; files cannot travel inside JSON. */
  | { kind: "open"; id: RequestId; file: Blob; name: string; password: string | null; recover: boolean }
  /** Write `doc` to the file, then reopen it from there; answered `saved`. */
  | { kind: "save"; id: RequestId; doc: DocId; mode: SaveMode; handle: SaveHandle | { download: string } }
  /**
   * A port of a macro Worker: it sends `{ kind: "message" }` (with
   * `ClientMessage.group` for its edits) and gets the responses; events
   * still go to the page.
   */
  | { kind: "port"; port: MessagePort };

/** What saving needs of a `FileSystemFileHandle`. */
export interface SaveHandle {
  createWritable(): Promise<{
    write(data: Uint8Array): Promise<void>;
    close(): Promise<void>;
    abort(): Promise<void>;
  }>;
  getFile(): Promise<File>;
}

/**
 * Worker → main thread: a JSON `ServerMessage` plus the binary parts of a
 * response; `file` is a save written in memory, for the page to download.
 */
export type Outbound = { text: string; blobs: Uint8Array[]; file?: File } | { fatal: string };

/** Without the File System Access API: collect the bytes into a `File`. */
function memoryHandle(name: string): SaveHandle {
  let file: File | null = null;
  return {
    createWritable: async () => {
      const parts: Uint8Array[] = [];
      return {
        write: async (data) => void parts.push(data.slice()),
        close: async () => void (file = new File(parts as BlobPart[], name, { type: "application/pdf" })),
        abort: async () => void (parts.length = 0),
      };
    },
    getFile: async () => {
      if (!file) throw new Error("nothing was written");
      return file;
    },
  };
}

export interface HostOptions {
  session: Pick<
    WasmSession,
    "dispatch" | "open" | "runJobStep" | "takeEvents" | "saveBegin" | "saveChunk" | "saveEnd" | "reload"
  >;
  post: (out: Outbound) => void;
  /** Synchronous random access to a file (Worker: `FileReaderSync`). */
  source: (file: Blob) => JsByteSource;
  /** Run `fn` as a new task, after pending messages. */
  schedule: (fn: () => void) => void;
  /** Time spent on jobs per turn. */
  jobSliceMs?: number;
}

function idOf(m: Inbound): RequestId | null {
  return m.kind === "message" ? m.message.id : m.kind === "port" ? null : m.id;
}

/** A queued request and where its response goes (`null`: the page). */
interface Queued {
  m: Inbound;
  reply: ((out: Outbound) => void) | null;
}

export class Host {
  private queue: Queued[] = [];
  private scheduled = false;
  private jobs = false;
  /** A save is writing (awaiting the file): nothing else runs meanwhile. */
  private saving = false;
  private opts: HostOptions;

  constructor(opts: HostOptions) {
    this.opts = opts;
  }

  /** A message from the page, or (`reply`) from a macro Worker's port. */
  receive(m: Inbound, reply: ((out: Outbound) => void) | null = null) {
    if (m.kind === "port") {
      const port = m.port;
      const post = (out: Outbound) => port.postMessage(out, "blobs" in out ? out.blobs.map((b) => b.buffer as ArrayBuffer) : []);
      port.onmessage = (e: MessageEvent<Inbound>) => {
        // Only requests: files and saves come from the page.
        if (e.data?.kind === "message") this.receive(e.data, post);
      };
      return;
    }
    if (m.kind === "message" && m.message.request.op === "cancel" && !reply) {
      const target = m.message.request.request;
      const i = this.queue.findIndex((q) => !q.reply && idOf(q.m) === target);
      if (i >= 0) {
        this.queue.splice(i, 1);
        this.send({ kind: "response", id: target, response: { err: { code: "cancelled", message: "cancelled" } } });
      }
      this.send({ kind: "response", id: m.message.id, response: { ok: { type: "empty" } } });
      return;
    }
    this.queue.push({ m, reply });
    this.kick();
  }

  private send(msg: ServerMessage) {
    this.opts.post({ text: JSON.stringify(msg), blobs: [] });
  }

  private kick() {
    if (this.scheduled || this.saving) return;
    this.scheduled = true;
    this.opts.schedule(() => this.turn());
  }

  /** One request, then a slice of job work, then yield to new messages. */
  private turn() {
    this.scheduled = false;
    const { session } = this.opts;
    const q = this.queue.shift();
    const m = q?.m;
    if (m?.kind === "save") {
      this.saving = true;
      void this.save(m).finally(() => {
        this.saving = false;
        this.jobs = true;
        this.kick();
      });
      return;
    }
    if (m && m.kind !== "port") {
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
      (q?.reply ?? this.opts.post)(out);
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

  /** Stream the document into the file chunk by chunk, then reopen it from the file. */
  private async save(m: Extract<Inbound, { kind: "save" }>) {
    const { session } = this.opts;
    const download = "download" in m.handle;
    const handle = "download" in m.handle ? memoryHandle(m.handle.download) : m.handle;
    try {
      const out = await handle.createWritable();
      let result: SaveResult;
      let id: number | null = null;
      try {
        id = session.saveBegin(m.doc, JSON.stringify(m.mode));
        for (;;) {
          const chunk = session.saveChunk(id);
          if (!chunk) break;
          await out.write(chunk);
        }
        result = JSON.parse(session.saveEnd(id)) as SaveResult;
        id = null;
        await out.close();
      } catch (e) {
        if (id !== null) session.saveEnd(id);
        await out.abort().catch(() => {});
        throw e;
      }
      const file = await handle.getFile();
      const reply = session.reload(m.id, m.doc, this.opts.source(file), file.name);
      const msg = JSON.parse(reply.message) as ServerMessage;
      reply.free();
      if (msg.kind === "response" && "ok" in msg.response && msg.response.ok.type === "doc") {
        const { type: _, ...summary } = msg.response.ok;
        msg.response = { ok: { type: "saved", summary, result: { ...result, path: file.name } } };
      }
      this.flushEvents();
      this.opts.post({ text: JSON.stringify(msg), blobs: [], file: download ? file : undefined });
    } catch (e) {
      this.flushEvents();
      const message = e instanceof Error ? e.message : String(e);
      this.send({ kind: "response", id: m.id, response: { err: { code: "io", message } } });
    }
  }

  private flushEvents() {
    for (const text of this.opts.session.takeEvents()) this.opts.post({ text, blobs: [] });
  }
}
