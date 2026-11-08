// Where the UI finds, stores and runs macros. The server runs them itself
// (`JobKind::Macro`, its macro directory); in browser-only mode they are
// kept in IndexedDB and run in a macro Worker (macro/worker.ts). The panel
// does not tell the two apart.
import type { Backend } from "../backend/backend";
import { ApiFailure } from "../backend/backend";
import type { DocId, Event, JobInfo, MacroInfo, Request, ResponseBody, SaveMode } from "../protocol/generated";
import { saveMode, type MacroMeta } from "./engine";
import type { MacroInbound, MacroOutbound } from "./worker";

export interface MacroItem {
  /** `meta.name` (the file name when it cannot be read). */
  name: string;
  /** The file name without `.js`: how it is saved and removed. */
  stem: string;
  /** Where it is, for people. */
  file: string;
  bundled: boolean;
  meta: MacroMeta | null;
  error: string | null;
  /** Hidden by a macro of the same name before it. */
  shadowed: boolean;
}

export interface RunFile {
  name: string;
  mime: string | null;
  size: number;
  read(): Promise<Uint8Array>;
}

export interface RunResult {
  value: unknown;
  columns: string[] | null;
  files: RunFile[];
  logTruncated: boolean;
  edits: boolean;
  save: SaveMode;
}

export interface RunCallbacks {
  log(level: "info" | "warn" | "error", text: string): void;
  progress(done: number, total: number, label: string | null): void;
}

export interface RunHandle {
  /** Rejects with `ApiFailure` (code `cancelled` when stopped). */
  result: Promise<RunResult>;
  cancel(): void;
}

export interface MacroService {
  readonly where: "server" | "browser";
  list(): Promise<{ items: MacroItem[]; dir: string | null }>;
  source(item: MacroItem): Promise<string>;
  /** Store `<stem>.js` (replacing it). */
  save(stem: string, code: string): Promise<MacroItem>;
  remove(stem: string): Promise<void>;
  /** Run the stored macro `item` on `doc` as one undoable step. */
  run(
    doc: DocId,
    item: MacroItem,
    params: Record<string, unknown>,
    input: { name: string; size: number },
    cb: RunCallbacks,
  ): RunHandle;
}

export const STEM = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

async function ask<T extends ResponseBody["type"]>(
  b: Backend,
  type: T,
  req: Request,
): Promise<Extract<ResponseBody, { type: T }>> {
  const r = await b.request(req);
  if (r.body.type !== type) throw new Error(`expected ${type}, got ${r.body.type}`);
  return r.body as Extract<ResponseBody, { type: T }>;
}

// ------------------------------------------------------------------ server

function fromInfo(m: MacroInfo): MacroItem {
  return {
    name: m.name,
    stem: m.stem,
    file: m.file,
    bundled: m.bundled,
    meta: m.meta === null ? null : (JSON.parse(m.meta) as MacroMeta),
    error: m.error,
    shadowed: m.shadowed,
  };
}

/** Bytes per `jobOutput` request (the protocol allows 4 MB). */
const CHUNK = 4 << 20;

class ServerMacros implements MacroService {
  readonly where = "server";
  constructor(private b: Backend) {}

  async list() {
    const r = await ask(this.b, "macros", { op: "macros" });
    return { items: r.items.map(fromInfo), dir: r.dir };
  }

  async source(item: MacroItem) {
    return (await ask(this.b, "macroSource", { op: "macroSource", name: item.bundled ? item.name : item.stem })).code;
  }

  async save(stem: string, code: string) {
    const { type: _, ...info } = await ask(this.b, "macro", { op: "saveMacro", name: stem, code });
    return fromInfo(info);
  }

  async remove(stem: string) {
    await ask(this.b, "empty", { op: "deleteMacro", name: stem });
  }

  run(doc: DocId, item: MacroItem, params: Record<string, unknown>, _input: unknown, cb: RunCallbacks): RunHandle {
    const b = this.b;
    let job: number | null = null;
    // Events may come before the job's number does.
    const early: Event[] = [];
    let settle: ((info: JobInfo) => void) | null = null;
    const ended = new Promise<JobInfo>((r) => (settle = r));
    const onEvent = (ev: Event) => {
      if (job === null) {
        early.push(ev);
        return;
      }
      if (ev.type === "macroLog" && ev.job === job) cb.log(ev.level, ev.text);
      else if (ev.type === "jobProgress" && ev.job === job) cb.progress(ev.done, ev.total, ev.label ?? null);
      else if (ev.type === "jobDone" && ev.info.job === job) settle?.(ev.info);
    };
    const off = b.onEvent(onEvent);
    let cancelled = false;
    const result = (async () => {
      try {
        const started = await ask(b, "job", {
          op: "startJob",
          doc,
          job: { kind: "macro", name: item.bundled ? item.name : item.stem, source: null, params: JSON.stringify(params) },
        });
        job = started.job;
        for (const ev of early.splice(0)) onEvent(ev);
        if (cancelled) void b.request({ op: "cancelJob", job }).catch(() => {});
        // It may have ended before the answer came (and its event was dropped by a reconnect).
        void ask(b, "job", { op: "jobStatus", job }).then((j) => j.state !== "running" && settle?.(j), () => {});
        const info = await ended;
        if (info.state === "cancelled") throw new ApiFailure("cancelled", "cancelled");
        if (info.state === "failed") throw new ApiFailure("macro", info.error ?? "the macro failed");
        const r = info.result;
        if (r?.kind !== "macro") throw new ApiFailure("internal", "the job gave no macro result");
        const id = info.job;
        return {
          value: JSON.parse(r.value) as unknown,
          columns: r.columns,
          logTruncated: r.log_truncated,
          edits: r.edits,
          save: r.save,
          files: r.emitted.map((f, index) => ({
            name: f.name,
            mime: f.mime,
            size: f.size,
            read: async () => {
              const out = new Uint8Array(f.size);
              for (let offset = 0; offset < f.size; ) {
                const len = Math.min(CHUNK, f.size - offset);
                const rep = await b.request({ op: "jobOutput", job: id, index, offset, len });
                if (rep.body.type !== "bytes") throw new Error("expected bytes");
                const data = rep.blobs[rep.body.part] ?? new Uint8Array();
                if (data.length === 0) break;
                out.set(data, offset);
                offset += data.length;
              }
              return out;
            },
          })),
        };
      } finally {
        off();
      }
    })();
    return {
      result,
      cancel: () => {
        cancelled = true;
        if (job !== null) void b.request({ op: "cancelJob", job }).catch(() => {});
      },
    };
  }
}

// ------------------------------------------------------------------ browser

const BUNDLED: [string, string][] = Object.entries(
  import.meta.glob("../../../../crates/pdfi-macro/macros/*.js", { query: "?raw", import: "default", eager: true }) as Record<
    string,
    string
  >,
)
  .map(([path, code]): [string, string] => [path.split("/").pop()!.replace(/\.js$/, ""), code])
  .sort(([a], [b]) => a.localeCompare(b));

interface Stored {
  stem: string;
  code: string;
  updated: number;
}

const DB = "pdfi-macros";
const STORE = "macros";

function db(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open(DB, 1);
    r.onupgradeneeded = () => r.result.createObjectStore(STORE, { keyPath: "stem" });
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

async function tx<T>(mode: IDBTransactionMode, f: (s: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  const d = await db();
  try {
    return await new Promise<T>((resolve, reject) => {
      const t = d.transaction(STORE, mode);
      const r = f(t.objectStore(STORE));
      t.oncomplete = () => resolve(r.result);
      t.onerror = () => reject(t.error);
      t.onabort = () => reject(t.error);
    });
  } finally {
    d.close();
  }
}

const newWorker = () => new Worker(new URL("./worker.ts", import.meta.url), { type: "module", name: "pdfi-macro" });

/** One request to a short-lived macro Worker (no document). */
function oneShot(m: MacroInbound): Promise<MacroOutbound> {
  const w = newWorker();
  return new Promise<MacroOutbound>((resolve, reject) => {
    w.onmessage = (e: MessageEvent<MacroOutbound>) => resolve(e.data);
    w.onerror = (e) => reject(new Error(e.message || "the macro Worker failed"));
    w.postMessage(m);
  }).finally(() => w.terminate());
}

class BrowserMacros implements MacroService {
  readonly where = "browser";
  /** `meta` (or why there is none) by code: evaluating every macro on each listing is wasteful. */
  private metas = new Map<string, { meta: MacroMeta } | { error: string }>();

  constructor(private b: Backend) {}

  private async readMetas(items: { stem: string; code: string; file: string }[]) {
    const missing = items.filter((i) => !this.metas.has(i.code));
    if (missing.length > 0) {
      const r = await oneShot({
        kind: "metas",
        items: missing.map((i) => ({ code: i.code, file: i.file, fallback: i.stem })),
      });
      if (r.kind !== "metas") throw new Error("unexpected answer from the macro Worker");
      missing.forEach((i, k) => this.metas.set(i.code, r.results[k]));
    }
    return items.map((i) => this.metas.get(i.code)!);
  }

  private async all(): Promise<{ stem: string; code: string; file: string; bundled: boolean }[]> {
    const stored = (await tx("readonly", (s) => s.getAll() as IDBRequest<Stored[]>)).sort((a, b) =>
      a.stem.localeCompare(b.stem),
    );
    return [
      ...stored.map((s) => ({ stem: s.stem, code: s.code, file: `${s.stem}.js`, bundled: false })),
      ...BUNDLED.map(([stem, code]) => ({ stem, code, file: `bundled:${stem}.js`, bundled: true })),
    ];
  }

  async list() {
    const all = await this.all();
    const metas = await this.readMetas(all);
    const items: MacroItem[] = [];
    all.forEach((m, i) => {
      const r = metas[i];
      const meta = "meta" in r ? r.meta : null;
      const name = meta?.name ?? m.stem;
      items.push({
        name,
        stem: m.stem,
        file: m.file,
        bundled: m.bundled,
        meta,
        error: "error" in r ? r.error : null,
        shadowed: items.some((o) => o.name === name),
      });
    });
    return { items, dir: null };
  }

  async source(item: MacroItem) {
    if (item.bundled) return BUNDLED.find(([s]) => s === item.stem)?.[1] ?? "";
    const s = await tx("readonly", (st) => st.get(item.stem) as IDBRequest<Stored | undefined>);
    if (!s) throw new ApiFailure("notFound", `no macro ${item.stem}`);
    return s.code;
  }

  async save(stem: string, code: string) {
    if (!STEM.test(stem)) throw new ApiFailure("badRequest", `${JSON.stringify(stem)} is not a macro name`);
    await tx("readwrite", (s) => s.put({ stem, code, updated: Date.now() } satisfies Stored));
    const { items } = await this.list();
    return items.find((i) => !i.bundled && i.stem === stem)!;
  }

  async remove(stem: string) {
    await tx("readwrite", (s) => s.delete(stem));
  }

  run(
    doc: DocId,
    item: MacroItem,
    params: Record<string, unknown>,
    input: { name: string; size: number },
    cb: RunCallbacks,
  ): RunHandle {
    const b = this.b;
    const w = newWorker();
    let group: number | null = null;
    let finish: ((m: MacroOutbound | { kind: "cancelled" } | { kind: "crashed"; message: string }) => void) | null = null;
    const ended = new Promise<MacroOutbound | { kind: "cancelled" } | { kind: "crashed"; message: string }>(
      (r) => (finish = r),
    );
    w.onmessage = (e: MessageEvent<MacroOutbound>) => {
      const m = e.data;
      if (m.kind === "log") cb.log(m.level, m.text);
      else if (m.kind === "progress") cb.progress(m.done, m.total, m.label);
      else if (m.kind === "group") group = m.group;
      else finish?.(m);
    };
    w.onerror = (e) => finish?.({ kind: "crashed", message: e.message || "the macro Worker failed" });
    const stop = async () => {
      w.terminate();
      // Its edits are reverted (it cannot do that itself any more).
      if (group !== null) {
        await b
          .request({ op: "editGroup", doc, action: { type: "abort", group } })
          .catch(() => {});
        group = null;
      }
    };
    const result = (async (): Promise<RunResult> => {
      const meta = item.meta;
      if (!meta) throw new ApiFailure("badRequest", item.error ?? "the macro cannot be read");
      if (!b.connectPort) throw new ApiFailure("unsupported", "this backend cannot run macros in the page");
      const code = await this.source(item);
      const ch = new MessageChannel();
      b.connectPort(ch.port1);
      w.postMessage(
        { kind: "run", port: ch.port2, doc, code, file: item.file, meta, params, input } satisfies MacroInbound,
        [ch.port2],
      );
      const m = await ended;
      if (m.kind === "cancelled" || m.kind === "crashed") {
        await stop();
        if (m.kind === "crashed") throw new ApiFailure("internal", m.message);
        throw new ApiFailure("cancelled", "cancelled");
      }
      w.terminate();
      if (m.kind === "failed") {
        throw new ApiFailure(m.code, m.stack && !m.message.includes(m.stack) ? `${m.message}\n${m.stack}` : m.message);
      }
      if (m.kind !== "done") throw new ApiFailure("internal", `unexpected ${m.kind} from the macro Worker`);
      return {
        value: m.value,
        columns: m.columns,
        logTruncated: false,
        edits: meta.edits,
        save: saveMode(meta),
        files: m.emitted.map((f) => ({ name: f.name, mime: f.mime, size: f.data.length, read: async () => f.data })),
      };
    })();
    return { result, cancel: () => finish?.({ kind: "cancelled" }) };
  }
}

export function macroService(b: Backend): MacroService {
  return b.mode === "server" ? new ServerMacros(b) : new BrowserMacros(b);
}

export { message as errorText };
