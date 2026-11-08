// Running macros on the page's own JavaScript (docs/DESIGN.md §4.11): the
// prelude (crates/pdfi-macro/js/prelude.js, the same file QuickJS runs)
// over `__host` functions that send the protocol to a session elsewhere
// (the Worker holding the document) through `Link`.
//
// No DOM or Worker APIs here: the macro Worker (worker.ts) and the wasm
// conformance test (scripts/conformance.ts, in Node) both use it, each
// with its own `Realm`.
import type { DocId, Request, Response, SaveMode } from "../protocol/generated";

/** A parameter as `meta.params` declares it, defaults filled in. */
export interface MacroParam {
  type: "string" | "number" | "integer" | "boolean" | "enum" | "pages" | "json" | "text" | "bytes" | "pdf";
  optional: boolean;
  values?: string[];
  description?: string;
  default?: unknown;
}

/** A macro's `meta`, as the prelude checks it. */
export interface MacroMeta {
  api: number;
  name: string;
  title: string;
  description: string;
  edits: boolean;
  save: "incremental" | "full" | { xrefStream: boolean; objectStreams: boolean; compress: boolean; removeUnused: boolean };
  /** In declaration order. */
  params: Record<string, MacroParam>;
}

/** How `meta.save` asks to be saved. */
export function saveMode(m: MacroMeta): SaveMode {
  const s = m.save;
  if (s === "incremental") return { kind: "incremental" };
  const o = s === "full" ? { xrefStream: false, objectStreams: false, compress: false, removeUnused: false } : s;
  return {
    kind: "full",
    xref_stream: o.xrefStream,
    object_streams: o.objectStreams,
    compress: o.compress,
    remove_unused: o.removeUnused,
  };
}

/** The functions the prelude calls (its header lists them). */
export interface HostFunctions {
  dispatch(request: string): Promise<{ body: string; parts: Uint8Array[] }>;
  waitJob(job: number): Promise<string>;
  log(level: string, text: string): void;
  progress(done: number, total: number, label: string | null): void;
  emit(name: string, binary: string, mime: string | null): void;
  cancelled(): boolean;
}

/** What the prelude defines as `__pdfi`. */
interface Entry {
  meta(ns: object, fallback: string): string;
  checkParams(meta: string, raw: string): void;
  run(ns: object, meta: string, doc: DocId, raw: string, input: string): Promise<string>;
}

/** Where scripts are evaluated: a Worker's global scope, a Node `vm` context. */
export interface Realm {
  /** Evaluate the prelude with `globalThis.__host` set to `host`. */
  installPrelude(host: HostFunctions): Promise<void>;
  /** Evaluate a macro's module; its namespace. */
  importModule(code: string, file: string): Promise<object>;
  /** `globalThis.__pdfi` once the prelude ran. */
  entry(): unknown;
}

/** Sends one request to the session holding the documents (in an edit group). */
export type Link = (request: Request, group: number | null) => Promise<{ response: Response; blobs: Uint8Array[] }>;

export interface EmittedData {
  name: string;
  mime: string | null;
  data: Uint8Array;
}

export interface MacroOutcome {
  /** The default export's return value (JSON). */
  value: unknown;
  columns: string[] | null;
  emitted: EmittedData[];
}

/** The macro threw, its parameters are wrong, or the session refused. */
export class MacroFailure extends Error {
  readonly code: string;
  /** The macro's own frames of the stack. */
  readonly trace: string | null;
  constructor(code: string, message: string, trace: string | null = null) {
    super(message);
    this.code = code;
    this.trace = trace;
  }
}

export interface RunOptions {
  doc: DocId;
  code: string;
  file: string;
  meta: MacroMeta;
  /** Raw parameters: JSON values, strings to convert, `{ $bytes }`, `{ $doc }`. */
  params: Record<string, unknown>;
  host: string;
  input: { name?: string; size?: number };
  log?: (level: "info" | "warn" | "error", text: string) => void;
  progress?: (done: number, total: number, label: string | null) => void;
  /** The edit group is open (to abort it if the run is stopped from outside). */
  onGroup?: (group: number) => void;
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** Bytes as a string of one character each, and back. */
function fromBinary(s: string): Uint8Array {
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

const errorMessage = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** One realm with the prelude; runs one macro at a time. */
export class Engine {
  private link: Link | null = null;
  private group: number | null = null;
  private opts: RunOptions | null = null;
  private emitted: EmittedData[] = [];
  private ready: Promise<void>;

  private realm: Realm;

  constructor(realm: Realm) {
    this.realm = realm;
    this.ready = realm.installPrelude(this.hostFunctions());
  }

  private hostFunctions(): HostFunctions {
    const failed = (message: string) => ({ body: JSON.stringify({ err: { code: "unsupported", message } }), parts: [] });
    return {
      dispatch: async (json) => {
        const link = this.link;
        if (!link) return failed("there is no document while the macro's meta is read");
        let req: Request;
        try {
          req = JSON.parse(json) as Request;
        } catch (e) {
          return { body: JSON.stringify({ err: { code: "badRequest", message: `malformed request: ${errorMessage(e)}` } }), parts: [] };
        }
        const r = await link(req, this.group);
        return { body: JSON.stringify(r.response), parts: r.blobs };
      },
      waitJob: async (job) => {
        const link = this.link;
        if (!link) return JSON.stringify({ state: "failed", error: "no document" });
        for (;;) {
          const r = await link({ op: "jobStatus", job }, this.group);
          if ("err" in r.response) return JSON.stringify({ state: "failed", error: r.response.err.message });
          const body = r.response.ok;
          if (body.type !== "job") return "{}";
          if (body.state !== "running") {
            const { type: _, ...info } = body;
            return JSON.stringify(info);
          }
          await sleep(4);
        }
      },
      log: (level, text) => this.opts?.log?.(level === "warn" || level === "error" ? level : "info", text),
      progress: (done, total, label) => this.opts?.progress?.(done, total, label),
      emit: (name, binary, mime) => void this.emitted.push({ name, mime, data: fromBinary(binary) }),
      // Stopping a macro terminates its Worker: nothing to poll.
      cancelled: () => false,
    };
  }

  private async entry(): Promise<Entry> {
    await this.ready;
    return this.realm.entry() as Entry;
  }

  /** Evaluate a macro without a document and read its `meta`. */
  async meta(code: string, file: string, fallback: string): Promise<MacroMeta> {
    const e = await this.entry();
    let ns: object;
    try {
      ns = await this.realm.importModule(code, file);
    } catch (x) {
      throw new MacroFailure("Error", errorMessage(x));
    }
    try {
      return JSON.parse(e.meta(ns, fallback)) as MacroMeta;
    } catch (x) {
      throw new MacroFailure("invalid", errorMessage(x).split("\n")[0]);
    }
  }

  /** Check parameters against `meta` without a document. */
  async checkParams(meta: MacroMeta, params: Record<string, unknown>): Promise<void> {
    const e = await this.entry();
    try {
      e.checkParams(JSON.stringify(meta), JSON.stringify(params));
    } catch (x) {
      throw new MacroFailure("invalid", errorMessage(x).split("\n")[0]);
    }
  }

  /**
   * Run a macro on `opts.doc` as one undoable step: its edits are kept if
   * it succeeds, reverted otherwise. Throws `MacroFailure`.
   */
  async run(link: Link, opts: RunOptions): Promise<MacroOutcome> {
    const e = await this.entry();
    if (this.opts) throw new Error("a macro is running");
    this.link = link;
    this.opts = opts;
    this.emitted = [];
    const ask = async (request: Request) => {
      const r = await link(request, null);
      if ("err" in r.response) throw new MacroFailure(r.response.err.code, r.response.err.message);
      return r.response.ok;
    };
    try {
      const begun = await ask({ op: "editGroup", doc: opts.doc, action: { type: "begin", label: opts.meta.title } });
      if (begun.type !== "group") throw new MacroFailure("internal", `expected group, got ${begun.type}`);
      const group = begun.group;
      this.group = group;
      opts.onGroup?.(group);
      let result: { ok: boolean; value?: unknown; columns?: string[] | null; error?: { code: string; message: string; stack: string | null } };
      try {
        const ns = await this.realm.importModule(opts.code, opts.file);
        const input = JSON.stringify({ host: opts.host, input: opts.input });
        result = JSON.parse(await e.run(ns, JSON.stringify(opts.meta), opts.doc, JSON.stringify(opts.params), input));
      } catch (x) {
        result = { ok: false, error: { code: "Error", message: errorMessage(x), stack: null } };
      }
      this.group = null;
      await ask({ op: "editGroup", doc: opts.doc, action: { type: result.ok ? "commit" : "abort", group } });
      if (!result.ok) {
        const err = result.error ?? { code: "Error", message: "no error given", stack: null };
        throw new MacroFailure(err.code, err.message, err.stack);
      }
      return { value: result.value ?? null, columns: result.columns ?? null, emitted: this.emitted };
    } finally {
      this.link = null;
      this.opts = null;
      this.group = null;
    }
  }
}
