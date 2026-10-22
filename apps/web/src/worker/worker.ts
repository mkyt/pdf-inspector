// Dedicated Worker hosting the wasm backend (browser-only mode).
import init, { WasmSession } from "../wasm/pdfi_wasm.js";
import { Host, type Inbound, type Outbound } from "./host";

// The page is type-checked with the DOM library; declare the Worker globals used here.
declare const FileReaderSync: { new (): { readAsArrayBuffer(blob: Blob): ArrayBuffer } };
const scope = self as unknown as {
  onmessage: ((e: MessageEvent<Inbound>) => void) | null;
  postMessage(msg: Outbound, transfer: Transferable[]): void;
};

const early: Inbound[] = [];
scope.onmessage = (e) => early.push(e.data);

function post(out: Outbound) {
  scope.postMessage(out, "blobs" in out ? out.blobs.map((b) => b.buffer as ArrayBuffer) : []);
}

/** Schedules a task through a MessageChannel (setTimeout is clamped to 4 ms when nested). */
function channelScheduler(): (fn: () => void) => void {
  const ch = new MessageChannel();
  const fns: (() => void)[] = [];
  ch.port1.onmessage = () => fns.shift()?.();
  return (fn) => {
    fns.push(fn);
    ch.port2.postMessage(null);
  };
}

init()
  .then(() => {
    const reader = new FileReaderSync();
    const host = new Host({
      session: new WasmSession(),
      post,
      // The file stays a Blob; only the requested range is read.
      source: (file) => ({
        size: file.size,
        read: (offset, length) => new Uint8Array(reader.readAsArrayBuffer(file.slice(offset, offset + length))),
      }),
      schedule: channelScheduler(),
    });
    scope.onmessage = (e) => host.receive(e.data);
    for (const m of early.splice(0)) host.receive(m);
  })
  .catch((e: unknown) => post({ fatal: `cannot start the wasm backend: ${String(e)}` }));
