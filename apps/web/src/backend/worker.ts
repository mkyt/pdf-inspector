import { PROTOCOL_VERSION, type ClientMessage, type Event, type Request, type ServerMessage } from "../protocol/generated";
import type { Inbound, Outbound } from "../worker/host";
import { ApiFailure, type Backend, type ConnectionState, type Reply } from "./backend";

interface Pending {
  resolve: (r: Reply) => void;
  reject: (e: Error) => void;
}

/** Backend running the wasm build in a dedicated Worker (no server needed). */
export class WorkerBackend implements Backend {
  readonly mode = "browser";
  private worker: Worker;
  private ready: Promise<unknown>;
  private state: ConnectionState = "connecting";
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private events = new Set<(ev: Event) => void>();
  private states = new Set<(s: ConnectionState) => void>();

  constructor() {
    this.worker = new Worker(new URL("../worker/worker.ts", import.meta.url), { type: "module", name: "pdfi" });
    this.worker.onmessage = (e: MessageEvent<Outbound>) => this.receive(e.data);
    this.worker.onerror = (e) => this.fail(e.message || "worker error");
    this.ready = this.send({ op: "hello", protocol: PROTOCOL_VERSION }).then(
      () => this.setState("open"),
      (e) => {
        this.fail(String(e));
        throw e;
      },
    );
  }

  onEvent(cb: (ev: Event) => void) {
    this.events.add(cb);
    return () => void this.events.delete(cb);
  }

  onState(cb: (s: ConnectionState) => void) {
    this.states.add(cb);
    cb(this.state);
    return () => void this.states.delete(cb);
  }

  private setState(s: ConnectionState) {
    this.state = s;
    for (const cb of this.states) cb(s);
  }

  /** The worker cannot recover; reject everything. */
  private fail(message: string) {
    for (const p of this.pending.values()) p.reject(new ApiFailure("internal", message));
    this.pending.clear();
    this.setState("closed");
  }

  private receive(out: Outbound) {
    if ("fatal" in out) {
      this.fail(out.fatal);
      return;
    }
    const msg = JSON.parse(out.text) as ServerMessage;
    if (msg.kind === "event") {
      for (const cb of this.events) cb(msg.event);
      return;
    }
    const p = this.pending.get(msg.id);
    if (!p) return;
    this.pending.delete(msg.id);
    if ("ok" in msg.response) p.resolve({ body: msg.response.ok, blobs: out.blobs });
    else p.reject(new ApiFailure(msg.response.err.code, msg.response.err.message));
  }

  private post(make: (id: number) => Inbound, signal?: AbortSignal): Promise<Reply> {
    if (this.state === "closed") return Promise.reject(new ApiFailure("internal", "the wasm backend stopped"));
    const id = this.nextId++;
    return new Promise<Reply>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.worker.postMessage(make(id));
      signal?.addEventListener(
        "abort",
        () => {
          if (!this.pending.has(id)) return;
          const cancel: ClientMessage = { id: this.nextId++, request: { op: "cancel", request: id } };
          this.worker.postMessage({ kind: "message", message: cancel } satisfies Inbound);
        },
        { once: true },
      );
    });
  }

  private send(request: Request, signal?: AbortSignal): Promise<Reply> {
    return this.post((id) => ({ kind: "message", message: { id, request } }), signal);
  }

  async request(req: Request, signal?: AbortSignal): Promise<Reply> {
    await this.ready;
    return this.send(req, signal);
  }

  async openFile(file: File, password: string | null, recover: boolean): Promise<Reply> {
    await this.ready;
    return this.post((id) => ({ kind: "open", id, file, name: file.name, password, recover }));
  }
}
