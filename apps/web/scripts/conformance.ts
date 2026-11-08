// Conformance runner for the wasm backend (docs/DESIGN.md §7): runs
// tests/conformance/scenario.json through the Worker host (src/worker/host.ts)
// and the wasm build, and compares with tests/conformance/expected.json,
// which the Rust API defines. Needs `npm run build:wasm` first.
//
//   node scripts/conformance.ts
import { readFileSync } from "node:fs";
import { isDeepStrictEqual } from "node:util";
import vm from "node:vm";
import { Engine, MacroFailure, type Link, type Realm } from "../src/macro/engine.ts";
import init, { WasmSession } from "../src/wasm/pdfi_wasm.js";
import { Host, type Inbound, type Outbound } from "../src/worker/host.ts";
import type { Event, Request, Response, ServerMessage } from "../src/protocol/generated.ts";

const root = new URL("../../../", import.meta.url);
const read = (p: string) => readFileSync(new URL(p, root));

type Step =
  | { request: Request }
  | { open: { file: string; password?: string | null; recover?: boolean } }
  | { job: Request }
  | { inGroup: { group: number; request: Request } };

await init({ module_or_path: read("apps/web/src/wasm/pdfi_wasm_bg.wasm") });

// Blobs cannot be read synchronously in Node; keep their bytes on the side.
const bytesOf = new Map<Blob, Uint8Array>();
const outbox: Outbound[] = [];
const host = new Host({
  session: new WasmSession(),
  post: (o) => outbox.push(o),
  source: (blob) => {
    const data = bytesOf.get(blob)!;
    return { size: data.length, read: (offset, length) => data.subarray(offset, offset + length) };
  },
  schedule: (fn) => setImmediate(fn),
});

const events: Event[] = [];
let nextId = 0;

const tick = () => new Promise<void>((r) => setImmediate(r));

/** Deliver `msg` and wait for the response to `id`; events go to `events`. */
async function call(id: number, msg: Inbound): Promise<{ response: Response; blobs: Uint8Array[] }> {
  host.receive(msg);
  for (;;) {
    const out = outbox.shift();
    if (!out) {
      await tick();
      continue;
    }
    if ("fatal" in out) throw new Error(out.fatal);
    const m = JSON.parse(out.text) as ServerMessage;
    if (m.kind === "event") events.push(m.event);
    else if (m.id === id) return { response: m.response, blobs: out.blobs };
    else throw new Error(`unexpected response to ${m.id}`);
  }
}

const request = (req: Request) => {
  const id = ++nextId;
  return call(id, { kind: "message", message: { id, request: req } });
};

// Mirrors `pdfi_api::conformance::entry`.
const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
function normalize(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(normalize);
  if (v === null || typeof v !== "object") return v;
  const o: Record<string, unknown> = {};
  for (const [k, x] of Object.entries(v)) o[k] = k === "path" && typeof x === "string" ? null : normalize(x);
  if (o.type === "hello") {
    o.backend = null;
    o.filesystem = null;
  }
  return o;
}
function entry(response: Response, blobs: Uint8Array[], evs: Event[]) {
  const e: Record<string, unknown> = { response, blobs: blobs.map(hex) };
  if (evs.length > 0) e.events = evs;
  return normalize(e);
}

const jobOf = (ev: Event) => (ev.type === "jobProgress" ? ev.job : ev.type === "jobDone" ? ev.info.job : null);

const steps = (JSON.parse(read("tests/conformance/scenario.json").toString()) as { steps: Step[] }).steps;
const expected = JSON.parse(read("tests/conformance/expected.json").toString()) as unknown[];
const actual: unknown[] = [];

for (const step of steps) {
  if ("request" in step) {
    const r = await request(step.request);
    actual.push(entry(r.response, r.blobs, []));
  } else if ("inGroup" in step) {
    const id = ++nextId;
    const r = await call(id, { kind: "message", message: { id, request: step.inGroup.request, group: step.inGroup.group } });
    actual.push(entry(r.response, r.blobs, []));
  } else if ("open" in step) {
    const { file, password = null, recover = false } = step.open;
    const data = new Uint8Array(read(`tests/fixtures/${file}`));
    const blob = new Blob([data]);
    bytesOf.set(blob, data);
    const id = ++nextId;
    const r = await call(id, { kind: "open", id, file: blob, name: file, password, recover });
    actual.push(entry(r.response, r.blobs, []));
  } else {
    events.length = 0;
    const r = await request(step.job);
    if (!("ok" in r.response) || r.response.ok.type !== "job") throw new Error("job not started");
    const job = r.response.ok.job;
    while (!events.some((e) => e.type === "jobDone" && e.info.job === job)) {
      await tick();
      for (let o = outbox.shift(); o; o = outbox.shift()) {
        if ("text" in o) {
          const m = JSON.parse(o.text) as ServerMessage;
          if (m.kind === "event") events.push(m.event);
        }
      }
    }
    actual.push(entry(r.response, r.blobs, events.filter((e) => jobOf(e) === job)));
  }
}

let failed = false;
for (let i = 0; i < Math.max(actual.length, expected.length); i++) {
  if (!isDeepStrictEqual(actual[i], expected[i])) {
    console.error(`wasm: step ${i} differs from expected.json`);
    console.error(`  actual:   ${JSON.stringify(actual[i])}`);
    console.error(`  expected: ${JSON.stringify(expected[i])}`);
    failed = true;
    break;
  }
}

// Transport behavior (not part of the shared transcript): a request that has
// not started can be cancelled.
{
  const a = ++nextId;
  const b = ++nextId;
  const c = ++nextId;
  host.receive({ kind: "message", message: { id: a, request: { op: "docs" } } });
  host.receive({ kind: "message", message: { id: b, request: { op: "docs" } } });
  host.receive({ kind: "message", message: { id: c, request: { op: "cancel", request: b } } });
  const seen = new Map<number, Response>();
  while (!(seen.has(a) && seen.has(b) && seen.has(c))) {
    await tick();
    for (let o = outbox.shift(); o; o = outbox.shift()) {
      if (!("text" in o)) continue;
      const m = JSON.parse(o.text) as ServerMessage;
      if (m.kind === "response") seen.set(m.id, m.response);
    }
  }
  const ra = seen.get(a)!;
  const rb = seen.get(b)!;
  if (!("ok" in ra) || !("ok" in seen.get(c)!) || !("err" in rb && rb.err.code === "cancelled")) {
    console.error("wasm: cancelling a queued request did not work", [...seen]);
    failed = true;
  }
}

// Saving through the host (browser mode): written chunk by chunk to a file
// handle, then reopened from it. An in-memory handle stands in for the
// File System Access API.
{
  let written: Uint8Array[] = [];
  let file: Uint8Array | null = null;
  const handle = {
    createWritable: async () => ({
      write: async (d: Uint8Array) => void written.push(d.slice()),
      close: async () => {
        file = new Uint8Array(written.reduce((n, d) => n + d.length, 0));
        let o = 0;
        for (const d of written) {
          file.set(d, o);
          o += d.length;
        }
        written = [];
      },
      abort: async () => void (written = []),
    }),
    getFile: async () => {
      const blob = new File([file!], "saved.pdf");
      bytesOf.set(blob, file!);
      return blob;
    },
  };
  const original = new Uint8Array(read("tests/fixtures/model.pdf"));
  const blob = new Blob([original]);
  bytesOf.set(blob, original);
  const o = ++nextId;
  const opened = await call(o, { kind: "open", id: o, file: blob, name: "model.pdf", password: null, recover: false });
  const doc = "ok" in opened.response && opened.response.ok.type === "doc" ? opened.response.ok.info.doc : -1;
  await request({ op: "apply", doc, label: "delete", commands: [{ op: "deletePages", pages: [0] }] });
  const id = ++nextId;
  const r = await call(id, { kind: "save", id, doc, mode: { kind: "incremental" }, handle });
  const saved = file as Uint8Array | null;
  const ok =
    "ok" in r.response &&
    r.response.ok.type === "saved" &&
    r.response.ok.summary.pages === 4 &&
    r.response.ok.summary.revisions.length === 2 &&
    !r.response.ok.summary.edit.dirty &&
    saved !== null &&
    r.response.ok.result.bytes === saved.length &&
    original.every((b, i) => saved[i] === b);
  // An error comes back as an error response, and the host keeps going.
  const bad = ++nextId;
  const r2 = await call(bad, { kind: "save", id: bad, doc: 999, mode: { kind: "incremental" }, handle });
  const after = await request({ op: "summary", doc });
  if (!ok || !("err" in r2.response) || !("ok" in after.response)) {
    console.error("wasm: saving through the host did not work", JSON.stringify(r.response).slice(0, 400));
    failed = true;
  }
}

// The bundled macros on this engine (the browser's JavaScript, here Node's
// `vm`), through the Worker host's port path, against what QuickJS gives
// (tests/conformance/macros-expected.json, from pdfi-macro's tests).
let macroCases = 0;
{
  const prelude = read("crates/pdfi-macro/js/prelude.js").toString();
  const nodeRealm = (): Realm => {
    const context = vm.createContext({}) as Record<string, unknown>;
    return {
      async installPrelude(h) {
        context.__host = h;
        new vm.Script(prelude, { filename: "prelude.js" }).runInContext(context);
      },
      async importModule(code, file) {
        // Needs --experimental-vm-modules.
        const VmModule = (vm as unknown as { SourceTextModule: new (code: string, o: object) => any }).SourceTextModule;
        const m = new VmModule(code, { context, identifier: file });
        await m.link(() => {
          throw new Error("macros cannot import");
        });
        await m.evaluate();
        return m.namespace as object;
      },
      entry: () => context.__pdfi,
    };
  };
  const fnv = (data: Uint8Array) => {
    let h = 0x811c9dc5;
    for (const b of data) h = Math.imul(h ^ b, 0x01000193) >>> 0;
    return h.toString(16).padStart(8, "0");
  };
  type Case = { macro: string; file: string; params: Record<string, unknown> };
  const spec = JSON.parse(read("tests/conformance/macros.json").toString()) as { cases: Case[]; probes: Request[] };
  const expectedMacros = JSON.parse(read("tests/conformance/macros-expected.json").toString()) as unknown[];
  const macroCode = (name: string) => read(`crates/pdfi-macro/macros/${name}.js`).toString();
  for (const [i, c] of spec.cases.entries()) {
    const out: Outbound[] = [];
    const h = new Host({
      session: new WasmSession(),
      post: (o) => out.push(o),
      source: (blob) => {
        const data = bytesOf.get(blob)!;
        return { size: data.length, read: (offset, length) => data.subarray(offset, offset + length) };
      },
      schedule: (fn) => setImmediate(fn),
    });
    let n = 0;
    // As the macro Worker's port does: answers come back to it, events go to the page.
    const link: Link = (request, group) =>
      new Promise((resolve, reject) => {
        const id = ++n;
        const message = group === null ? { id, request } : { id, request, group };
        h.receive({ kind: "message", message }, (o) => {
          if ("fatal" in o) return reject(new Error(o.fatal));
          const m = JSON.parse(o.text) as ServerMessage;
          if (m.kind === "response" && m.id === id) resolve({ response: m.response, blobs: o.blobs });
        });
      });
    const data = new Uint8Array(read(`tests/fixtures/${c.file}`));
    const blob = new Blob([data]);
    bytesOf.set(blob, data);
    h.receive({ kind: "open", id: 0, file: blob, name: c.file, password: null, recover: false });
    while (!out.some((o) => "text" in o && (JSON.parse(o.text) as ServerMessage).kind === "response")) await tick();
    const engine = new Engine(nodeRealm());
    const code = macroCode(c.macro);
    const file = `bundled:${c.macro}.js`;
    const meta = await engine.meta(code, file, c.macro);
    const r: Record<string, unknown> = { macro: c.macro, file: c.file };
    try {
      const o = await engine.run(link, {
        doc: 1,
        code,
        file,
        meta,
        params: c.params,
        host: "test",
        input: { name: c.file, size: data.length },
      });
      r.ok = true;
      r.value = o.value;
      r.columns = o.columns;
      r.emitted = o.emitted.map((e) => ({ name: e.name, mime: e.mime, size: e.data.length, fnv: fnv(e.data) }));
    } catch (e) {
      r.ok = false;
      r.error = e instanceof MacroFailure ? e.message : String(e);
    }
    const sum = await link({ op: "summary", doc: 1 }, null);
    r.edit = "ok" in sum.response && sum.response.ok.type === "doc" ? sum.response.ok.edit : null;
    const probes = [];
    for (const p of spec.probes) {
      const x = await link(p, null);
      probes.push(entry(x.response, x.blobs, []));
    }
    r.probes = probes;
    macroCases++;
    if (!isDeepStrictEqual(JSON.parse(JSON.stringify(r)), expectedMacros[i])) {
      console.error(`wasm: macro ${c.macro} on ${c.file} differs from macros-expected.json`);
      console.error(`  actual:   ${JSON.stringify(r).slice(0, 2000)}`);
      console.error(`  expected: ${JSON.stringify(expectedMacros[i]).slice(0, 2000)}`);
      failed = true;
      break;
    }
  }
  if (!failed && macroCases !== expectedMacros.length) {
    console.error(`wasm: ${macroCases} macro cases, macros-expected.json has ${expectedMacros.length}`);
    failed = true;
  }
}

if (failed) process.exit(1);
console.log(
  `wasm: ${actual.length} steps match expected.json; cancel ok; save ok; ${macroCases} macro runs match macros-expected.json`,
);
