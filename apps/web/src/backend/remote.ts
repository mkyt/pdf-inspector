import {
  PROTOCOL_VERSION,
  type ClientMessage,
  type Event,
  type Request,
  type ServerMessage,
} from "../protocol/generated";
import { ApiFailure, type Backend, type ConnectionState, type Reply } from "./backend";

interface Pending {
  resolve: (r: Reply) => void;
  reject: (e: Error) => void;
  blobs: Uint8Array[];
}

const HEADER_LEN = 12;

/** Backend over the `pdfi serve` WebSocket. Reconnects automatically. */
export class RemoteBackend implements Backend {
  readonly mode = "server";
  private ws: WebSocket | null = null;
  private ready: Promise<void> | null = null;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private events = new Set<(ev: Event) => void>();
  private states = new Set<(s: ConnectionState) => void>();
  private retry = 0;

  constructor(
    private url: string,
    private token: string,
  ) {}

  onEvent(cb: (ev: Event) => void) {
    this.events.add(cb);
    return () => void this.events.delete(cb);
  }

  onState(cb: (s: ConnectionState) => void) {
    this.states.add(cb);
    return () => void this.states.delete(cb);
  }

  private setState(s: ConnectionState) {
    for (const cb of this.states) cb(s);
  }

  private connect(): Promise<void> {
    if (this.ready) return this.ready;
    this.setState("connecting");
    this.ready = new Promise((resolve, reject) => {
      const ws = new WebSocket(this.url, ["pdfi.v1", `pdfi.token.${this.token}`]);
      ws.binaryType = "arraybuffer";
      ws.onopen = () => {
        this.ws = ws;
        this.retry = 0;
        this.send({ op: "hello", protocol: PROTOCOL_VERSION })
          .then(() => {
            this.setState("open");
            resolve();
          })
          .catch((e) => {
            reject(e);
            ws.close();
          });
      };
      ws.onmessage = (m) => this.receive(m.data);
      ws.onclose = () => {
        const wasOpen = this.ws === ws;
        this.ws = null;
        this.ready = null;
        for (const p of this.pending.values()) p.reject(new ApiFailure("io", "connection closed"));
        this.pending.clear();
        this.setState("closed");
        if (!wasOpen) reject(new ApiFailure("io", "cannot connect to the server"));
        // Server-side sessions survive; reconnect with backoff.
        const delay = Math.min(10_000, 500 * 2 ** this.retry++);
        setTimeout(() => void this.connect().catch(() => {}), delay);
      };
    });
    return this.ready;
  }

  private receive(data: string | ArrayBuffer) {
    if (typeof data !== "string") {
      if (data.byteLength < HEADER_LEN) return;
      const view = new DataView(data);
      const id = Number(view.getBigUint64(0, true));
      const part = view.getUint32(8, true);
      const p = this.pending.get(id);
      if (p) p.blobs[part] = new Uint8Array(data, HEADER_LEN);
      return;
    }
    const msg = JSON.parse(data) as ServerMessage;
    if (msg.kind === "event") {
      for (const cb of this.events) cb(msg.event);
      return;
    }
    const p = this.pending.get(msg.id);
    if (!p) return;
    this.pending.delete(msg.id);
    if ("ok" in msg.response) p.resolve({ body: msg.response.ok, blobs: p.blobs });
    else p.reject(new ApiFailure(msg.response.err.code, msg.response.err.message));
  }

  private send(request: Request, signal?: AbortSignal): Promise<Reply> {
    const ws = this.ws;
    if (!ws) return Promise.reject(new ApiFailure("io", "not connected"));
    const id = this.nextId++;
    const msg: ClientMessage = { id, request };
    return new Promise<Reply>((resolve, reject) => {
      this.pending.set(id, { resolve, reject, blobs: [] });
      ws.send(JSON.stringify(msg));
      signal?.addEventListener(
        "abort",
        () => {
          if (!this.pending.has(id)) return;
          const cancel: ClientMessage = { id: this.nextId++, request: { op: "cancel", request: id } };
          this.ws?.send(JSON.stringify(cancel));
        },
        { once: true },
      );
    });
  }

  async request(req: Request, signal?: AbortSignal): Promise<Reply> {
    await this.connect();
    return this.send(req, signal);
  }
}

/**
 * Server URL and token from the page location. `pdfi serve` opens
 * `http://127.0.0.1:<port>/#token=...`; the Vite dev server needs
 * `#token=...&server=127.0.0.1:<port>` (and `pdfi serve --dev-origin`).
 */
export function remoteFromLocation(): RemoteBackend | null {
  const params = new URLSearchParams(location.hash.slice(1));
  let token = params.get("token");
  let host = params.get("server") ?? location.host;
  if (token) {
    // Keep the token out of the visible URL, but survive reloads of the tab.
    try {
      sessionStorage.setItem("pdfi.token", token);
      sessionStorage.setItem("pdfi.server", host);
    } catch {
      // Storage unavailable: the token stays in the URL.
      return new RemoteBackend(`ws://${host}/ws`, token);
    }
    history.replaceState(null, "", location.pathname + location.search);
  } else {
    try {
      token = sessionStorage.getItem("pdfi.token");
      host = sessionStorage.getItem("pdfi.server") ?? host;
    } catch {
      return null;
    }
  }
  return token ? new RemoteBackend(`ws://${host}/ws`, token) : null;
}
