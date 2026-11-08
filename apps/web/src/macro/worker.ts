// A Worker running macros on the browser's JavaScript (browser-only mode).
// It holds no document: requests go through a MessagePort to the Worker
// holding the session (worker/host.ts). Stopping a macro terminates this
// Worker, which leaves that one and its unsaved edits alone; the page then
// aborts the macro's edit group.
import preludeUrl from "../../../../crates/pdfi-macro/js/prelude.js?url";
import type { DocId, Request, Response, ServerMessage } from "../protocol/generated";
import type { Inbound, Outbound } from "../worker/host";
import { Engine, MacroFailure, type EmittedData, type Link, type MacroMeta, type Realm } from "./engine";

/** Page → this Worker. */
export type MacroInbound =
  /** Read the `meta` of each macro (no document). */
  | { kind: "metas"; items: { code: string; file: string; fallback: string }[] }
  /** Check parameters against `meta` (no document). */
  | { kind: "check"; meta: MacroMeta; params: Record<string, unknown> }
  | {
      kind: "run";
      /** To the Worker holding the session. */
      port: MessagePort;
      doc: DocId;
      code: string;
      file: string;
      meta: MacroMeta;
      params: Record<string, unknown>;
      input: { name?: string; size?: number };
    };

/** This Worker → page. */
export type MacroOutbound =
  | { kind: "metas"; results: ({ meta: MacroMeta } | { error: string })[] }
  | { kind: "checked"; error: string | null }
  | { kind: "log"; level: "info" | "warn" | "error"; text: string }
  | { kind: "progress"; done: number; total: number; label: string | null }
  | { kind: "group"; group: number }
  | { kind: "done"; value: unknown; columns: string[] | null; emitted: EmittedData[] }
  | { kind: "failed"; code: string; message: string; stack: string | null };

const scope = self as unknown as {
  onmessage: ((e: MessageEvent<MacroInbound>) => void) | null;
  postMessage(msg: MacroOutbound, transfer?: Transferable[]): void;
};

/** Blob URLs of evaluated macros → their file names (for stack traces). */
const files = new Map<string, string>();

const realm: Realm = {
  async installPrelude(host) {
    (globalThis as { __host?: unknown }).__host = host;
    await import(/* @vite-ignore */ preludeUrl);
  },
  async importModule(code, file) {
    const url = URL.createObjectURL(new Blob([code], { type: "text/javascript" }));
    files.set(url, file);
    try {
      return (await import(/* @vite-ignore */ url)) as object;
    } finally {
      URL.revokeObjectURL(url);
    }
  },
  entry: () => (globalThis as { __pdfi?: unknown }).__pdfi,
};

const engine = new Engine(realm);

/** What the macro's author wrote: blob URLs named, the prelude's frames left out. */
function cleanStack(stack: string | null): string | null {
  if (!stack) return null;
  let s = stack;
  for (const [url, file] of files) s = s.split(url).join(file);
  const lines = s.split("\n").filter((l) => l.trim() && !l.includes(preludeUrl) && !/prelude[^/]*\.js/.test(l));
  return lines.length ? lines.join("\n") : null;
}

/** Requests over the port to the session, answered in any order. */
function portLink(port: MessagePort): Link {
  let next = 1;
  const pending = new Map<number, (r: { response: Response; blobs: Uint8Array[] }) => void>();
  port.onmessage = (e: MessageEvent<Outbound>) => {
    const out = e.data;
    if ("fatal" in out) {
      for (const p of pending.values()) p({ response: { err: { code: "internal", message: out.fatal } }, blobs: [] });
      pending.clear();
      return;
    }
    const msg = JSON.parse(out.text) as ServerMessage;
    if (msg.kind !== "response") return;
    const p = pending.get(msg.id);
    pending.delete(msg.id);
    p?.({ response: msg.response, blobs: out.blobs });
  };
  return (request: Request, group: number | null) =>
    new Promise((resolve) => {
      const id = next++;
      pending.set(id, resolve);
      const m: Inbound = { kind: "message", message: group === null ? { id, request } : { id, request, group } };
      port.postMessage(m);
    });
}

const post = (m: MacroOutbound, transfer?: Transferable[]) => scope.postMessage(m, transfer);
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

scope.onmessage = async (e) => {
  const m = e.data;
  if (m.kind === "metas") {
    const results: ({ meta: MacroMeta } | { error: string })[] = [];
    for (const it of m.items) {
      try {
        results.push({ meta: await engine.meta(it.code, it.file, it.fallback) });
      } catch (x) {
        results.push({ error: message(x) });
      }
    }
    post({ kind: "metas", results });
  } else if (m.kind === "check") {
    try {
      await engine.checkParams(m.meta, m.params);
      post({ kind: "checked", error: null });
    } catch (x) {
      post({ kind: "checked", error: message(x) });
    }
  } else {
    try {
      const out = await engine.run(portLink(m.port), {
        doc: m.doc,
        code: m.code,
        file: m.file,
        meta: m.meta,
        params: m.params,
        host: "web",
        input: m.input,
        log: (level, text) => post({ kind: "log", level, text }),
        progress: (done, total, label) => post({ kind: "progress", done, total, label }),
        onGroup: (group) => post({ kind: "group", group }),
      });
      post(
        { kind: "done", value: out.value, columns: out.columns, emitted: out.emitted },
        out.emitted.map((f) => f.data.buffer as ArrayBuffer),
      );
    } catch (x) {
      if (x instanceof MacroFailure) post({ kind: "failed", code: x.code, message: x.message, stack: cleanStack(x.trace) });
      else post({ kind: "failed", code: "internal", message: message(x), stack: null });
    }
  }
};
