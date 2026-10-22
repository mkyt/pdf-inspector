// Conformance runner for the wasm backend (docs/DESIGN.md §7): runs
// tests/conformance/scenario.json through the Worker host (src/worker/host.ts)
// and the wasm build, and compares with tests/conformance/expected.json,
// which the Rust API defines. Needs `npm run build:wasm` first.
//
//   node scripts/conformance.ts
import { readFileSync } from "node:fs";
import { isDeepStrictEqual } from "node:util";
import init, { WasmSession } from "../src/wasm/pdfi_wasm.js";
import { Host, type Inbound, type Outbound } from "../src/worker/host.ts";
import type { Event, Request, Response, ServerMessage } from "../src/protocol/generated.ts";

const root = new URL("../../../", import.meta.url);
const read = (p: string) => readFileSync(new URL(p, root));

type Step =
  | { request: Request }
  | { open: { file: string; password?: string | null; recover?: boolean } }
  | { job: Request };

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

if (failed) process.exit(1);
console.log(`wasm: ${actual.length} steps match expected.json; cancel ok`);
