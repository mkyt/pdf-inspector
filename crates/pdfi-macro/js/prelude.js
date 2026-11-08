// The public API of macros (docs/DESIGN.md §4.11), the same for every host.
//
// Scripts depend on this file only, never on the protocol: `meta.api`
// names the version of this API they are written for, and changes of the
// protocol are absorbed here. The types and the documentation of what is
// defined here are in pdfi.d.ts; keep the two in step.
//
// The host evaluates this file as a script before the macro's module, with
// `globalThis.__host` set to its host functions (all may return a value or a
// Promise of it):
//
//   dispatch(requestJson)  → { body: responseJson, parts: Uint8Array[] }
//   waitJob(job)           → jobInfoJson, once the job is no longer running
//   log(level, text)       level: "info" | "warn" | "error"
//   progress(done, total, label)
//   emit(name, binary, mime)  binary: the bytes as a string, one char each
//   cancelled()            → boolean
//
// and then calls `__pdfi.meta(namespace)` and `__pdfi.run(...)` (below).

(() => {
  "use strict";

  const API = 1;
  const host = globalThis.__host;
  delete globalThis.__host;

  /** Items asked for per ranged request (the protocol allows 2000). */
  const PAGE = 1000;
  /** Bytes asked for per binary request (the protocol allows 4 MB). */
  const CHUNK = 4 << 20;

  // ------------------------------------------------------------- errors

  class PdfiError extends Error {
    constructor(code, message) {
      super(message);
      this.name = "PdfiError";
      this.code = code;
    }
  }

  const fail = (message) => {
    throw new PdfiError("badRequest", message);
  };

  // ------------------------------------------------------------- text

  if (typeof globalThis.TextEncoder === "undefined") {
    class TextEncoder {
      get encoding() {
        return "utf-8";
      }
      encode(s = "") {
        const out = [];
        for (const ch of String(s)) {
          let c = ch.codePointAt(0);
          if (c >= 0xd800 && c <= 0xdfff) c = 0xfffd;
          if (c < 0x80) out.push(c);
          else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
          else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
          else
            out.push(
              0xf0 | (c >> 18),
              0x80 | ((c >> 12) & 63),
              0x80 | ((c >> 6) & 63),
              0x80 | (c & 63),
            );
        }
        return new Uint8Array(out);
      }
    }
    globalThis.TextEncoder = TextEncoder;
  }

  if (typeof globalThis.TextDecoder === "undefined") {
    const LABELS = {
      "utf-8": "utf-8",
      utf8: "utf-8",
      "utf-16le": "utf-16le",
      "utf-16": "utf-16le",
      "utf-16be": "utf-16be",
      latin1: "latin1",
      "iso-8859-1": "latin1",
      "windows-1252": "latin1",
    };
    class TextDecoder {
      constructor(label = "utf-8", options = {}) {
        const enc = LABELS[String(label).toLowerCase()];
        if (!enc) throw new RangeError(`unsupported encoding ${label}`);
        this._enc = enc;
        this._fatal = !!options.fatal;
        this._bom = !options.ignoreBOM;
      }
      get encoding() {
        return this._enc === "latin1" ? "windows-1252" : this._enc;
      }
      get fatal() {
        return this._fatal;
      }
      get ignoreBOM() {
        return !this._bom;
      }
      decode(input) {
        const b = toBytes(input ?? new Uint8Array());
        const chunks = [];
        let s = "";
        const flush = () => {
          if (s.length > 8192) {
            chunks.push(s);
            s = "";
          }
        };
        const bad = () => {
          if (this._fatal) throw new TypeError("invalid data for " + this._enc);
          return "�";
        };
        if (this._enc === "latin1") {
          for (let i = 0; i < b.length; i++) {
            s += String.fromCharCode(b[i]);
            flush();
          }
        } else if (this._enc === "utf-8") {
          let i = this._bom && b[0] === 0xef && b[1] === 0xbb && b[2] === 0xbf ? 3 : 0;
          while (i < b.length) {
            const c = b[i];
            let n = 0;
            let cp = 0;
            if (c < 0x80) cp = c;
            else if (c >= 0xc2 && c < 0xe0) (n = 1), (cp = c & 31);
            else if (c >= 0xe0 && c < 0xf0) (n = 2), (cp = c & 15);
            else if (c >= 0xf0 && c < 0xf5) (n = 3), (cp = c & 7);
            else {
              s += bad();
              i++;
              continue;
            }
            let j = 1;
            for (; j <= n; j++) {
              const d = b[i + j];
              if (d === undefined || (d & 0xc0) !== 0x80) break;
              cp = (cp << 6) | (d & 63);
            }
            if (j <= n || cp > 0x10ffff || (cp >= 0xd800 && cp <= 0xdfff) || (n === 2 && cp < 0x800) || (n === 3 && cp < 0x10000)) {
              s += bad();
              i += Math.max(1, j);
              continue;
            }
            s += String.fromCodePoint(cp);
            i += n + 1;
            flush();
          }
        } else {
          const be = this._enc === "utf-16be";
          let i = 0;
          if (this._bom && b.length >= 2) {
            if (be && b[0] === 0xfe && b[1] === 0xff) i = 2;
            if (!be && b[0] === 0xff && b[1] === 0xfe) i = 2;
          }
          for (; i + 1 < b.length; i += 2) {
            s += String.fromCharCode(be ? (b[i] << 8) | b[i + 1] : b[i] | (b[i + 1] << 8));
            flush();
          }
          if (i < b.length) s += bad();
        }
        chunks.push(s);
        return chunks.join("");
      }
    }
    globalThis.TextDecoder = TextDecoder;
  }

  const utf8 = new TextEncoder();
  const utf8d = new TextDecoder();

  function toBytes(data) {
    if (data instanceof Uint8Array) return data;
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    if (typeof data === "string") return utf8.encode(data);
    throw new TypeError("expected a Uint8Array, an ArrayBuffer or a string");
  }

  const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

  function base64(bytes) {
    const parts = [];
    let s = "";
    for (let i = 0; i < bytes.length; i += 3) {
      const n = (bytes[i] << 16) | ((bytes[i + 1] ?? 0) << 8) | (bytes[i + 2] ?? 0);
      s += B64[(n >> 18) & 63] + B64[(n >> 12) & 63];
      s += i + 1 < bytes.length ? B64[(n >> 6) & 63] : "=";
      s += i + 2 < bytes.length ? B64[n & 63] : "=";
      if (s.length > 8192) {
        parts.push(s);
        s = "";
      }
    }
    parts.push(s);
    return parts.join("");
  }

  function unbase64(s) {
    const out = [];
    let acc = 0;
    let bits = 0;
    for (const c of s) {
      if (c === "=") break;
      const v = B64.indexOf(c);
      if (v < 0) continue;
      acc = ((acc << 6) | v) & 0xffffff;
      bits += 6;
      if (bits >= 8) {
        bits -= 8;
        out.push((acc >> bits) & 255);
      }
    }
    return new Uint8Array(out);
  }

  /** Bytes as a string of one character each. */
  const latin1 = (bytes) => {
    const parts = [];
    for (let i = 0; i < bytes.length; i += 8192) parts.push(String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + 8192))));
    return parts.join("");
  };

  const hexBytes = (hex) => {
    const out = new Uint8Array(hex.length >> 1);
    for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.substr(i * 2, 2), 16);
    return out;
  };

  // ------------------------------------------------------------- console

  function show(v) {
    if (typeof v === "string") return v;
    if (v instanceof Error) return v.stack ? `${v}\n${v.stack}` : String(v);
    if (v === undefined || typeof v === "function" || typeof v === "symbol") return String(v);
    try {
      return JSON.stringify(v, replacer);
    } catch {
      return String(v);
    }
  }

  const format = (args) => args.map(show).join(" ");

  globalThis.console = {
    log: (...a) => host.log("info", format(a)),
    info: (...a) => host.log("info", format(a)),
    debug: (...a) => host.log("info", format(a)),
    warn: (...a) => host.log("warn", format(a)),
    error: (...a) => host.log("error", format(a)),
  };

  // ------------------------------------------------------------- values

  /** `#xx`-escaped name (as the protocol writes it) → its text. */
  function unescapeName(s) {
    if (!s.includes("#")) return s;
    const bytes = [];
    for (let i = 0; i < s.length; i++) {
      if (s[i] === "#" && /^[0-9a-fA-F]{2}$/.test(s.substr(i + 1, 2))) {
        bytes.push(parseInt(s.substr(i + 1, 2), 16));
        i += 2;
      } else bytes.push(s.charCodeAt(i) & 255);
    }
    const b = Uint8Array.from(bytes);
    try {
      return new TextDecoder("utf-8", { fatal: true }).decode(b);
    } catch {
      return latin1(b);
    }
  }

  function escapeName(s) {
    let out = "";
    for (const b of utf8.encode(s)) {
      const c = String.fromCharCode(b);
      out += b < 0x21 || b > 0x7e || "()<>[]{}/%#".includes(c) ? "#" + b.toString(16).padStart(2, "0") : c;
    }
    return out;
  }

  /** A key as given (`"Type"`, `"/Type"` or a `Name`) → its text. */
  function keyText(k) {
    if (k instanceof Name) return k.name;
    if (typeof k !== "string" || k === "" || k === "/") throw new TypeError("a key is a non-empty string");
    return k.startsWith("/") ? k.slice(1) : k;
  }

  class Name {
    constructor(name) {
      this.name = String(name);
    }
    toString() {
      return "/" + this.name;
    }
    toJSON() {
      return this.name;
    }
  }

  class PdfString {
    constructor(text, bytes, truncated = false) {
      this.text = text;
      this.bytes = bytes;
      this.truncated = truncated;
    }
    toString() {
      return this.text;
    }
    toJSON() {
      return this.text;
    }
  }

  class Ref {
    constructor(num, generation = 0) {
      if (!Number.isInteger(num) || num <= 0) throw new TypeError(`bad object number ${num}`);
      this.num = num;
      this.generation = generation;
    }
    toString() {
      return `${this.num} ${this.generation} R`;
    }
    toJSON() {
      return this.toString();
    }
    equals(o) {
      return o instanceof Ref && o.num === this.num && o.generation === this.generation;
    }
  }

  class Raw {
    constructor(text) {
      this.text = String(text);
    }
  }

  const refOf = (r) => (r ? new Ref(r.num, r.generation) : null);
  const wire = (r) => ({ num: r.num, generation: r.generation });

  /** Protocol DTO → API value: camelCase keys, `ObjRef` as `Ref`. */
  function camel(v) {
    if (Array.isArray(v)) return v.map(camel);
    if (v === null || typeof v !== "object") return v;
    const keys = Object.keys(v);
    if (keys.length === 2 && "num" in v && "generation" in v) return new Ref(v.num, v.generation);
    const out = {};
    for (const k of keys) out[k.replace(/_([a-z])/g, (_, c) => c.toUpperCase())] = camel(v[k]);
    return out;
  }

  function number(n) {
    if (!Number.isFinite(n)) throw new TypeError(`${n} is not a PDF number`);
    if (Number.isInteger(n)) return String(n);
    let s = n.toFixed(6).replace(/0+$/, "").replace(/\.$/, "");
    return s === "-0" ? "0" : s;
  }

  function literal(s) {
    let out = "(";
    for (const ch of s) {
      const c = ch.charCodeAt(0);
      if (ch === "(" || ch === ")" || ch === "\\") out += "\\" + ch;
      else if (c < 0x20 || c === 0x7f) out += "\\" + c.toString(8).padStart(3, "0");
      else out += ch;
    }
    return out + ")";
  }

  const hex = (bytes) => "<" + Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("") + ">";

  /** A JS string as a PDF text string: ASCII as is, else UTF-16BE. */
  function textString(s) {
    if (/^[\x20-\x7e\t\n\r]*$/.test(s)) return literal(s);
    const b = [0xfe, 0xff];
    for (let i = 0; i < s.length; i++) {
      const c = s.charCodeAt(i);
      b.push(c >> 8, c & 255);
    }
    return hex(b);
  }

  /** A value written in PDF syntax. */
  function syntax(v) {
    if (v === null || v === undefined) return "null";
    if (typeof v === "boolean") return v ? "true" : "false";
    if (typeof v === "number") return number(v);
    if (typeof v === "string") return textString(v);
    if (v instanceof Name) return "/" + escapeName(v.name);
    if (v instanceof PdfString) {
      if (v.truncated) throw new TypeError("a string read only in part cannot be written back");
      // `pdf.str("text")` has no bytes: written as a text string.
      return v.bytes ? hex(v.bytes) : textString(v.text);
    }
    if (v instanceof Ref) return v.toString();
    if (v instanceof Raw) return v.text;
    if (v instanceof Node) {
      if (v.ref) return v.ref.toString();
      throw new TypeError("a direct dictionary cannot be written elsewhere: write a plain object");
    }
    if (v instanceof Uint8Array) return hex(v);
    if (v instanceof Date) return textString(pdfDate(v));
    if (Array.isArray(v)) return "[" + v.map(syntax).join(" ") + "]";
    if (typeof v === "object") {
      const parts = Object.entries(v)
        .filter(([, x]) => x !== undefined)
        .map(([k, x]) => `/${escapeName(keyText(k))} ${syntax(x)}`);
      return "<< " + parts.join(" ") + " >>";
    }
    throw new TypeError(`cannot write ${typeof v} as a PDF value`);
  }

  function pdfDate(d) {
    const p = (n, w = 2) => String(n).padStart(w, "0");
    return `D:${p(d.getUTCFullYear(), 4)}${p(d.getUTCMonth() + 1)}${p(d.getUTCDate())}${p(d.getUTCHours())}${p(d.getUTCMinutes())}${p(d.getUTCSeconds())}Z`;
  }

  function rectOf(r) {
    if (Array.isArray(r) && r.length === 4) r = { x0: r[0], y0: r[1], x1: r[2], y1: r[3] };
    if (!r || ![r.x0, r.y0, r.x1, r.y1].every(Number.isFinite)) throw new TypeError("expected a rectangle {x0, y0, x1, y1}");
    return {
      x0: Math.min(r.x0, r.x1),
      y0: Math.min(r.y0, r.y1),
      x1: Math.max(r.x0, r.x1),
      y1: Math.max(r.y0, r.y1),
    };
  }

  // ------------------------------------------------------------- calls

  async function call(request) {
    if (await host.cancelled()) throw new PdfiError("cancelled", "the macro was cancelled");
    const r = await host.dispatch(JSON.stringify(request));
    const res = JSON.parse(r.body);
    if (res.err) throw new PdfiError(res.err.code, res.err.message);
    return { body: res.ok, parts: r.parts };
  }

  const body = async (request) => (await call(request)).body;

  /** Every item of a ranged request, `make(range)` building the request. */
  async function all(make, pick = (b) => b) {
    const items = [];
    for (let offset = 0; ; ) {
      const p = pick(await body(make({ offset, limit: PAGE })));
      items.push(...p.items);
      offset += p.items.length;
      if (offset >= p.total || p.items.length === 0) return items;
    }
  }

  async function* each(make, from, to, pick = (b) => b) {
    for (let offset = from; offset < to; ) {
      const p = pick(await body(make({ offset, limit: Math.min(PAGE, to - offset) })));
      for (const it of p.items) yield it;
      offset += p.items.length;
      if (offset >= p.total || p.items.length === 0) return;
    }
  }

  // ------------------------------------------------------------- nodes

  const rootWire = (root) => (root === "trailer" ? { kind: "trailer" } : { kind: "object", id: wire(root) });

  class Node {
    constructor(doc, root, segs, kind) {
      this._doc = doc;
      this._root = root; // "trailer" or a Ref
      this._segs = segs; // keys (dictionaries) and indices (arrays)
      this.kind = kind; // "dict" | "stream"
      this._path = null; // [generation, NodePath]
      this._entries = null; // [generation, NodeView[]]
    }

    get ref() {
      return this._root instanceof Ref && this._segs.length === 0 ? this._root : null;
    }

    get isStream() {
      return this.kind === "stream";
    }

    /** The node's protocol path, valid until the document changes. */
    async _resolve() {
      const gen = this._doc._gen;
      if (this._path && this._path[0] === gen) return this._path[1];
      const path = { root: rootWire(this._root), path: [] };
      for (const s of this._segs) {
        if (typeof s === "number") {
          path.path.push(s);
          continue;
        }
        const items = await all((range) => ({ op: "children", doc: this._doc._id, path, range }));
        const k = escapeName(s);
        const it = items.find((i) => i.key === k);
        if (!it) throw new PdfiError("notFound", `no /${s} any more`);
        path.path.push(it.index);
      }
      this._path = [gen, path];
      return path;
    }

    async _items() {
      const gen = this._doc._gen;
      if (this._entries && this._entries[0] === gen) return this._entries[1];
      const path = await this._resolve();
      const items = await all((range) => ({ op: "children", doc: this._doc._id, path, range }));
      this._entries = [gen, items];
      return items;
    }

    async _find(key) {
      const k = escapeName(keyText(key));
      return (await this._items()).find((i) => i.key === k);
    }

    async keys() {
      return (await this._items()).map((i) => unescapeName(i.key));
    }

    async has(key) {
      return !!(await this._find(key));
    }

    async get(key) {
      const it = await this._find(key);
      if (!it) return undefined;
      return this._doc._value(it.value, this._root, [...this._segs, keyText(key)]);
    }

    async resolve(key) {
      const v = await this.get(key);
      return v instanceof Ref ? this._doc.object(v) : v;
    }

    async entries() {
      const out = [];
      for (const it of await this._items()) {
        const k = unescapeName(it.key);
        out.push([k, await this._doc._value(it.value, this._root, [...this._segs, k])]);
      }
      return out;
    }

    async dict(key, options = {}) {
      const k = keyText(key);
      let v = await this.get(k);
      if (v instanceof Ref) v = await this._doc.object(v);
      if (v instanceof Node) return v;
      if (v === undefined || v === null) {
        if (!options.create) return null;
        await this.set(k, {});
        return new Node(this._doc, this._root, [...this._segs, k], "dict");
      }
      throw new PdfiError("badRequest", `/${k} is not a dictionary`);
    }

    async set(key, value) {
      this._doc._writable();
      const node = await this._resolve();
      await this._doc.apply([{ op: "setEntry", node, key: escapeName(keyText(key)), value: syntax(value) }], `Set /${keyText(key)}`);
    }

    async remove(key) {
      this._doc._writable();
      if (!(await this._find(key))) return false;
      const node = await this._resolve();
      await this._doc.apply([{ op: "removeEntry", node, key: escapeName(keyText(key)) }], `Remove /${keyText(key)}`);
      return true;
    }

    async data(options = {}) {
      const id = this._streamRef();
      const decoded = options.decoded ?? true;
      const chunks = [];
      let size = 0;
      for (let offset = 0; ; ) {
        const { body: b, parts } = await call({ op: "streamData", doc: this._doc._id, id: wire(id), decoded, offset, len: CHUNK });
        const part = parts[b.part] ?? new Uint8Array();
        chunks.push(part);
        size += part.length;
        offset += part.length;
        if (b.eof || part.length === 0) break;
      }
      const out = new Uint8Array(size);
      let at = 0;
      for (const c of chunks) {
        out.set(c, at);
        at += c.length;
      }
      return out;
    }

    async setData(data, options = {}) {
      this._doc._writable();
      const id = this._streamRef();
      const filter = options.filter ?? "flate";
      if (filter !== "flate" && filter !== "none") throw new TypeError(`filter is "flate" or "none", not ${filter}`);
      const payload = { kind: "base64", data: base64(toBytes(data)) };
      await this._doc.apply([{ op: "setStreamData", id: wire(id), data: payload, filter }], "Set stream data");
    }

    _streamRef() {
      if (this.kind !== "stream" || !this.ref) throw new PdfiError("badRequest", "not a stream object");
      return this.ref;
    }

    toJSON() {
      return this.ref ? this.ref.toString() : `<<${this.kind}>>`;
    }
  }

  // ------------------------------------------------------------- pages

  const BOXES = { media: "MediaBox", crop: "CropBox", bleed: "BleedBox", trim: "TrimBox", art: "ArtBox" };

  function boxName(name) {
    const n = keyText(name);
    if (BOXES[n.toLowerCase()]) return BOXES[n.toLowerCase()];
    if (Object.values(BOXES).includes(n)) return n;
    throw new TypeError(`${name} is not a page box (media, crop, bleed, trim, art)`);
  }

  class Page {
    constructor(doc, s) {
      this._doc = doc;
      this.index = s.index;
      this.ref = refOf(s.id);
      this.label = s.label ?? null;
      this.rotate = s.rotate;
      this.size = s.size ?? null;
      this.annotationCount = s.annots;
      this.content = new Content(doc, { kind: "page", index: s.index });
    }

    get node() {
      if (!this.ref) throw new PdfiError("notFound", `page ${this.index} has no object`);
      return new Node(this._doc, this.ref, [], "dict");
    }

    async _detail() {
      return body({ op: "page", doc: this._doc._id, index: this.index });
    }

    async boxes() {
      const d = await this._detail();
      const out = { origins: {} };
      for (const [k, name] of Object.entries(BOXES)) {
        const b = d.boxes.find((x) => x.name === name);
        out[k] = b?.rect ?? null;
        out.origins[k] = b ? camel(b.origin) : { kind: "missing" };
      }
      return out;
    }

    async setBox(name, rect) {
      const r = rectOf(rect);
      await this.node.set(boxName(name), [r.x0, r.y0, r.x1, r.y1]);
    }

    async removeBox(name) {
      return this.node.remove(boxName(name));
    }

    async resources() {
      return camel((await this._detail()).resource_items);
    }

    async annotations() {
      return camel(await all((range) => ({ op: "annotations", doc: this._doc._id, page: this.index, range })));
    }
  }

  // ------------------------------------------------------------- content

  function opRange(range, count) {
    if (range === undefined) return [0, count];
    if (Array.isArray(range)) range = { from: range[0], to: range[1] };
    const from = Math.max(0, range.from ?? 0);
    const to = Math.min(count, range.to ?? count);
    return [from, Math.max(from, to)];
  }

  class Content {
    constructor(doc, ref) {
      this._doc = doc;
      this._ref = ref;
    }

    async summary() {
      return camel(await body({ op: "content", doc: this._doc._id, content: this._ref }));
    }

    async count() {
      return (await body({ op: "content", doc: this._doc._id, content: this._ref })).ops;
    }

    async *ops(range) {
      const [from, to] = opRange(range, await this.count());
      for await (const op of each((r) => ({ op: "contentOps", doc: this._doc._id, content: this._ref, range: r }), from, to))
        yield camel(op);
    }

    async *elements() {
      const n = await this.count();
      for await (const e of each((r) => ({ op: "contentElements", doc: this._doc._id, content: this._ref, range: r }), 0, n))
        yield camel(e);
    }

    async bounds() {
      let b = null;
      for await (const e of this.elements()) {
        if (e.hidden || e.kind === "clip") continue;
        const r = e.bbox;
        b = b ? { x0: Math.min(b.x0, r.x0), y0: Math.min(b.y0, r.y0), x1: Math.max(b.x1, r.x1), y1: Math.max(b.y1, r.y1) } : { ...r };
      }
      return b;
    }

    async text(range) {
      let [from, to] = opRange(range, await this.count());
      let s = "";
      while (from < to) {
        const t = await body({ op: "contentText", doc: this._doc._id, content: this._ref, range: { offset: from, limit: to - from } });
        s += t.text;
        if (t.to <= from) break;
        from = t.to;
      }
      return s;
    }

    async _edit(edits, label) {
      this._doc._writable();
      await this._doc.apply([{ op: "editOps", content: this._ref, edits }], label);
    }

    async replace(range, text) {
      const [from, to] = opRange(range, await this.count());
      await this._edit([{ kind: "replace", from, to, text: String(text) }], "Replace operations");
    }

    async insert(before, text) {
      await this._edit([{ kind: "replace", from: before, to: before, text: String(text) }], "Insert operations");
    }

    /** @param {any} [query] */
    async find(query = {}) {
      if (!query || Array.isArray(query) || typeof query !== "object") throw new TypeError("find takes a query object");
      const known = ["operators", "kinds", "markedContent", "ocg", "colorSpace", "bbox", "text", "hidden", "unit", "range", "where"];
      for (const k of Object.keys(query)) if (!known.includes(k)) throw new TypeError(`find: unknown criterion ${JSON.stringify(k)} (${known.join(", ")})`);
      const q = { unit: query.unit ?? "element" };
      if (!["op", "element", "block"].includes(q.unit)) throw new TypeError(`find: unit is "op", "element" or "block", not ${JSON.stringify(q.unit)}`);
      if (query.operators !== undefined) q.operators = strings("operators", query.operators);
      if (query.kinds !== undefined) q.kinds = strings("kinds", query.kinds);
      if (query.hidden !== undefined) q.hidden = !!query.hidden;
      const mc = query.markedContent;
      if (mc !== undefined) {
        q.marked = {
          tags: mc.tag === undefined ? [] : strings("markedContent.tag", mc.tag).map(keyText),
          properties: Object.entries(mc.properties ?? {}).map(([key, v]) => ({
            key: keyText(key),
            value: v instanceof Name ? v.name : v instanceof PdfString ? v.text : String(v),
          })),
        };
      }
      const cs = query.colorSpace;
      if (cs !== undefined) {
        q.color = {
          families: cs.family === undefined ? [] : strings("colorSpace.family", cs.family),
          separations: cs.separation === undefined ? [] : strings("colorSpace.separation", cs.separation),
        };
      }
      // Rectangles: the first relation is checked by the host, the others here.
      const boxes = [];
      if (query.bbox !== undefined) {
        for (const [rel, r] of Object.entries(query.bbox)) {
          if (!["inside", "outside", "intersects"].includes(rel)) throw new TypeError(`find: bbox takes inside, outside or intersects, not ${rel}`);
          if (r === null || r === undefined) continue;
          boxes.push([rel, rectOf(r)]);
        }
        if (boxes.length) q.bbox = { relation: boxes[0][0], rect: boxes[0][1] };
      }
      let textRe = null;
      if (query.text !== undefined) {
        if (query.text instanceof RegExp) {
          textRe = query.text;
          if (!q.kinds) q.kinds = ["text", "form"];
        } else q.text = String(query.text);
      }
      if (query.ocg !== undefined) q.ocgs = await this._doc._ocgRefs(query.ocg);
      let range = null;
      if (query.range !== undefined) {
        const [from, to] = opRange(query.range, Number.MAX_SAFE_INTEGER);
        range = { offset: from, limit: Math.min(to - from, 0xffffffff) };
      }
      const b = await body({ op: "contentFind", doc: this._doc._id, content: this._ref, query: q, range });
      if (b.truncated) host.log("warn", `find: only the first ${b.hits.length} hits`);
      const out = [];
      for (const h of b.hits) {
        const hit = {
          from: h.from,
          to: h.to,
          op: h.op,
          kind: h.kind ?? null,
          bbox: h.bbox ?? null,
          hidden: h.hidden,
          text: h.text ?? null,
          form: refOf(h.form),
        };
        if (boxes.slice(1).some(([rel, r]) => !hit.bbox || !relates(hit.bbox, rel, r))) continue;
        if (textRe) {
          textRe.lastIndex = 0;
          if (hit.text === null || !textRe.test(hit.text)) continue;
        }
        if (query.where && !(await query.where(hit))) continue;
        out.push(hit);
      }
      return out;
    }

    async remove(hits, label = "Remove operations") {
      if (!Array.isArray(hits)) hits = [hits];
      const count = await this.count();
      const ranges = hits
        .map((h) => opRange(h, count))
        .filter(([a, b]) => b > a)
        .sort((x, y) => x[0] - y[0]);
      const merged = [];
      for (const [a, b] of ranges) {
        const last = merged[merged.length - 1];
        if (last && a <= last[1]) last[1] = Math.max(last[1], b);
        else merged.push([a, b]);
      }
      if (!merged.length) return 0;
      // Each edit sees the result of those before: last first.
      const edits = merged.reverse().map(([from, to]) => ({ kind: "replace", from, to, text: "" }));
      await this._edit(edits, label);
      return merged.reduce((n, [a, b]) => n + b - a, 0);
    }
  }

  function strings(what, v) {
    const a = Array.isArray(v) ? v : [v];
    if (!a.every((x) => typeof x === "string" || x instanceof Name)) throw new TypeError(`find: ${what} takes strings`);
    return a.map((x) => (x instanceof Name ? x.name : x));
  }

  function relates(b, rel, r) {
    const ix = Math.max(b.x0, r.x0), iy = Math.max(b.y0, r.y0), ax = Math.min(b.x1, r.x1), ay = Math.min(b.y1, r.y1);
    const overlaps = ix <= ax && iy <= ay;
    switch (rel) {
      case "inside":
        return b.x0 >= r.x0 && b.y0 >= r.y0 && b.x1 <= r.x1 && b.y1 <= r.y1;
      case "intersects":
        return overlaps;
      default:
        return !overlaps || ix === ax || iy === ay;
    }
  }

  // ------------------------------------------------------------- documents

  const indices = (pages) => {
    if (typeof pages === "number") pages = [pages];
    if (!Array.isArray(pages) || !pages.every((i) => Number.isInteger(i) && i >= 0))
      throw new TypeError("pages are 0-based page numbers");
    return pages;
  };

  class Doc {
    constructor(id, summary, readOnly) {
      this._id = id;
      this._summary = summary;
      this._gen = 0;
      this.readOnly = readOnly;
      this.raw = {
        dispatch: async (request) => {
          const r = await call(request);
          if (request && (request.op === "apply" || request.op === "undo" || request.op === "redo")) await this._changed();
          return r;
        },
      };
    }

    get name() {
      return this._summary.info.name;
    }
    get pageCount() {
      return this._summary.pages;
    }
    get version() {
      return this._summary.pdf_version ?? null;
    }
    get encryption() {
      return camel(this._summary.encryption ?? null);
    }
    get edited() {
      return this._summary.edit.dirty;
    }
    get trailer() {
      return new Node(this, "trailer", [], "dict");
    }
    get catalog() {
      const r = refOf(this._summary.root);
      if (!r) throw new PdfiError("notFound", "the document has no catalog");
      return new Node(this, r, [], "dict");
    }

    _writable() {
      if (this.readOnly) throw new PdfiError("forbidden", `${this.name} is open read-only`);
    }

    async _changed() {
      this._gen++;
      this._summary = await body({ op: "summary", doc: this._id });
    }

    async _value(v, root, segs) {
      switch (v.type) {
        case "null":
          return null;
        case "bool":
        case "integer":
        case "real":
          return v.value;
        case "name":
          return new Name(unescapeName(v.name));
        case "string":
          return new PdfString(v.text, hexBytes(v.hex), v.truncated);
        case "ref":
          return new Ref(v.id.num, v.id.generation);
        case "dict":
          return new Node(this, root, segs, "dict");
        case "stream":
          return new Node(this, root, segs, "stream");
        case "array": {
          const node = new Node(this, root, segs, "array");
          const out = [];
          for (const it of await node._items()) out.push(await this._value(it.value, root, [...segs, it.index]));
          return out;
        }
        default:
          throw new PdfiError("internal", `unknown value type ${v.type}`);
      }
    }

    async object(ref) {
      if (!(ref instanceof Ref)) ref = new Ref(ref.num ?? ref, ref.generation ?? 0);
      const o = await body({ op: "object", doc: this._id, id: wire(ref) });
      return this._value(o.value, ref, []);
    }

    async page(index) {
      if (!Number.isInteger(index) || index < 0 || index >= this.pageCount)
        throw new RangeError(`no page ${index} (0-based; the document has ${this.pageCount})`);
      const p = await body({ op: "pages", doc: this._id, range: { offset: index, limit: 1 } });
      return new Page(this, p.items[0]);
    }

    async *pages(which) {
      const n = this.pageCount;
      let runs;
      if (which === undefined) runs = [[0, n]];
      else if (Array.isArray(which)) {
        runs = [];
        for (const i of indices(which)) {
          if (i >= n) throw new RangeError(`no page ${i} (0-based; the document has ${n})`);
          const last = runs[runs.length - 1];
          if (last && last[1] === i) last[1]++;
          else runs.push([i, i + 1]);
        }
      } else {
        const [from, to] = opRange(which, n);
        runs = [[from, to]];
      }
      for (const [from, to] of runs)
        for await (const s of each((range) => ({ op: "pages", doc: this._id, range }), from, to)) yield new Page(this, s);
    }

    async info() {
      const d = await body({ op: "document", doc: this._id });
      const out = {};
      for (const e of d.info_entries) out[e.key] = e.date ?? e.text;
      return out;
    }

    async setInfo(fields, options = {}) {
      const sync = options.syncXmp ?? true;
      const commands = Object.entries(fields).map(([key, v]) => ({
        op: "setInfo",
        key: keyText(key),
        value: v === null || v === undefined ? null : v instanceof Date ? v.toISOString() : String(v),
        sync_xmp: sync,
      }));
      if (commands.length) await this.apply(commands, "Set document information");
    }

    async xmp() {
      const d = await body({ op: "document", doc: this._id });
      if (!d.metadata) return null;
      const x = await body({ op: "xmp", doc: this._id, range: { offset: 0, limit: 0 } });
      const node = new Node(this, refOf(d.metadata), [], "stream");
      return {
        ref: refOf(d.metadata),
        properties: x.properties,
        xml: utf8d.decode(await node.data()),
        error: x.error ?? null,
      };
    }

    async outline() {
      const level = async (parent, depth) => {
        if (depth > 64) return [];
        const items = await all((range) => ({ op: "outline", doc: this._id, parent: parent && wire(parent), range }));
        const out = [];
        for (const it of items) {
          const link = camel(it.link ?? null);
          out.push({
            ref: refOf(it.id),
            title: it.title,
            page: link?.kind === "dest" ? link.dest.pageIndex ?? null : null,
            open: it.count === null || it.count === undefined ? null : it.count > 0,
            link,
            flags: it.flags,
            color: it.color ?? null,
            children: it.has_children ? await level(it.id, depth + 1) : [],
          });
        }
        return out;
      };
      return level(null, 0);
    }

    async ocgs() {
      const props = await this.catalog.resolve("OCProperties");
      if (!(props instanceof Node)) return [];
      const list = await props.resolve("OCGs");
      if (!Array.isArray(list)) return [];
      const out = [];
      for (const r of list) {
        if (!(r instanceof Ref)) continue;
        const g = await this.object(r);
        const n = g instanceof Node ? await g.get("Name") : undefined;
        out.push({ ref: r, name: n instanceof PdfString ? n.text : n === undefined ? "" : String(n) });
      }
      return out;
    }

    /** References of the groups an `ocg` criterion picks. */
    async _ocgRefs(sel) {
      const list = Array.isArray(sel) ? sel : [sel];
      if (list.length && list.every((x) => x instanceof Ref)) return list.map(wire);
      const groups = await this.ocgs();
      const picked = groups.filter((g) =>
        list.some((x) =>
          x instanceof Ref ? x.equals(g.ref) : x instanceof RegExp ? ((x.lastIndex = 0), x.test(g.name)) : typeof x === "function" ? x(g.name, g.ref) : g.name === String(x),
        ),
      );
      return picked.map((g) => wire(g.ref));
    }

    async setOutline(items, label = "Set the outline") {
      if (!Array.isArray(items)) throw new TypeError("setOutline takes an array of items");
      const entries = [];
      const walk = (list, level) => {
        for (const it of list) {
          if (!it || typeof it !== "object") throw new TypeError("an outline item is an object");
          if (level > 64) throw new RangeError("outline items nest 64 levels at most");
          const link = it.link ?? null;
          const dest = link && link.kind === "dest" ? link.dest : null;
          let target = null;
          const uri = it.uri ?? (link && link.kind === "uri" ? link.uri : undefined);
          const named = it.dest ?? (dest && dest.named ? dest.named : undefined);
          if (uri !== undefined && uri !== null) target = { kind: "uri", uri: String(uri) };
          else if (named !== undefined && named !== null) target = { kind: "named", name: String(named) };
          else if (it.page !== undefined && it.page !== null) {
            if (!Number.isInteger(it.page) || it.page < 0 || it.page >= this.pageCount)
              throw new RangeError(`outline item ${JSON.stringify(it.title)}: no page ${it.page} (0-based; the document has ${this.pageCount})`);
            const view = it.view ?? (dest ? { fit: dest.fit, params: dest.params } : null);
            const fit = typeof view === "string" ? view : view?.fit ?? "Fit";
            const params = typeof view === "string" ? [] : (view?.params ?? []).map((p) => (p === null || p === undefined ? null : Number(p)));
            target = { kind: "page", index: it.page, fit, params };
          }
          const flags = it.flags ?? ((it.bold ? 2 : 0) | (it.italic ? 1 : 0));
          const children = it.children ?? [];
          entries.push({
            level,
            title: String(it.title ?? ""),
            target,
            open: !!it.open,
            flags,
            color: it.color ?? null,
          });
          walk(children, level + 1);
        }
      };
      walk(items, 0);
      await this.apply([{ op: "setOutline", items: entries }], label);
    }

    async pageLabels() {
      return (await body({ op: "pageLabels", doc: this._id })).ranges.map((r) => ({
        start: r.start,
        style: r.style ?? null,
        prefix: r.prefix ?? "",
        first: r.first ?? 1,
      }));
    }

    async setPageLabels(ranges, label = "Set page labels") {
      if (!Array.isArray(ranges)) throw new TypeError("setPageLabels takes an array of ranges");
      const styles = { D: "decimal", R: "upperRoman", r: "lowerRoman", A: "upperLetters", a: "lowerLetters" };
      const out = ranges.map((r) => {
        if (!Number.isInteger(r.start) || r.start < 0 || r.start >= Math.max(1, this.pageCount))
          throw new RangeError(`label range: no page ${r.start} (0-based; the document has ${this.pageCount})`);
        let style = r.style ?? null;
        if (style !== null && styles[style]) style = styles[style];
        if (style !== null && !Object.values(styles).includes(style))
          throw new TypeError(`label style ${JSON.stringify(r.style)}: ${Object.values(styles).join(", ")} or null`);
        const first = r.first ?? 1;
        if (!Number.isInteger(first) || first < 1) throw new RangeError(`label range at ${r.start}: first is an integer, 1 or more`);
        return { start: r.start, style, prefix: r.prefix ? String(r.prefix) : null, first: first === 1 ? null : first };
      });
      await this.apply([{ op: "setPageLabels", ranges: out }], label);
    }

    async _inventory() {
      try {
        await body({ op: "fonts", doc: this._id, range: { offset: 0, limit: 0 } });
      } catch (e) {
        if (!(e instanceof PdfiError) || e.code !== "notReady") throw e;
        const job = await body({ op: "startJob", doc: this._id, job: { kind: "inventory" } });
        const info = JSON.parse(await host.waitJob(job.job));
        if (info.state !== "completed") throw new PdfiError(info.state === "cancelled" ? "cancelled" : "internal", info.error ?? `the inventory ${info.state}`);
      }
    }

    async fonts() {
      await this._inventory();
      return camel(await all((range) => ({ op: "fonts", doc: this._id, range })));
    }

    async images() {
      await this._inventory();
      return camel(await all((range) => ({ op: "images", doc: this._id, range })));
    }

    async apply(commands, label = "Macro") {
      this._writable();
      const r = await body({ op: "apply", doc: this._id, label, commands });
      await this._changed();
      return camel(r);
    }

    async deletePages(pages) {
      await this.apply([{ op: "deletePages", pages: indices(pages) }], "Delete pages");
    }

    async movePages(pages, to) {
      await this.apply([{ op: "movePages", pages: indices(pages), to }], "Move pages");
    }

    async rotatePages(pages, by) {
      if (!Number.isInteger(by) || by % 90 !== 0) throw new TypeError("rotation is a multiple of 90 degrees");
      await this.apply([{ op: "rotatePages", pages: indices(pages), by }], "Rotate pages");
    }

    async insertBlankPage(at = this.pageCount, size = {}) {
      const width = size.width ?? 595.276;
      const height = size.height ?? 841.89;
      await this.apply([{ op: "insertBlankPage", at, width, height }], "Insert a blank page");
    }

    async insertPages(from, options = {}) {
      if (!(from instanceof Doc)) throw new TypeError("insertPages copies from a Doc (a `pdf` parameter)");
      const pages = options.pages === undefined ? Array.from({ length: from.pageCount }, (_, i) => i) : indices(options.pages);
      const at = options.at ?? this.pageCount;
      await this.apply([{ op: "insertPages", at, from: from._id, pages }], "Insert pages");
    }
  }

  // ------------------------------------------------------------- meta and parameters

  const TYPES = ["string", "number", "integer", "boolean", "enum", "pages", "json", "text", "bytes", "pdf"];

  function checkMeta(m, fallbackName) {
    if (!m || typeof m !== "object") throw new TypeError("the module exports no `meta` object");
    if (m.api !== API) throw new TypeError(`meta.api is ${JSON.stringify(m.api)}: this host provides API ${API}`);
    const name = m.name ?? fallbackName;
    if (typeof name !== "string" || !/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(name))
      throw new TypeError(`meta.name ${JSON.stringify(name)}: letters, digits, '.', '_' and '-'`);
    let save = m.save ?? "incremental";
    if (save !== "incremental" && save !== "full") {
      if (typeof save !== "object") throw new TypeError(`meta.save: "incremental", "full" or full options`);
      save = {
        xrefStream: !!save.xrefStream,
        objectStreams: !!save.objectStreams,
        compress: !!save.compress,
        removeUnused: !!save.removeUnused,
      };
    }
    const params = {};
    for (const [k, p] of Object.entries(m.params ?? {})) {
      if (!/^[A-Za-z_][A-Za-z0-9_-]*$/.test(k)) throw new TypeError(`parameter name ${JSON.stringify(k)}`);
      if (!p || !TYPES.includes(p.type)) throw new TypeError(`parameter ${k}: type is one of ${TYPES.join(", ")}`);
      if (p.type === "enum" && (!Array.isArray(p.values) || p.values.length === 0 || !p.values.every((v) => typeof v === "string")))
        throw new TypeError(`parameter ${k}: an enum needs \`values\` (strings)`);
      const q = { type: p.type, optional: !!p.optional };
      if (p.values) q.values = p.values;
      if (p.description !== undefined) q.description = String(p.description);
      if (p.default !== undefined) {
        if (["text", "bytes", "pdf"].includes(p.type)) throw new TypeError(`parameter ${k}: a ${p.type} has no default`);
        param(k, q, p.default, null);
        q.default = p.default;
      }
      params[k] = q;
    }
    return {
      api: API,
      name,
      title: m.title === undefined ? name : String(m.title),
      description: m.description === undefined ? "" : String(m.description),
      edits: !!m.edits,
      save,
      params,
    };
  }

  const bad = (k, msg) => {
    throw new PdfiError("badRequest", `parameter ${k}: ${msg}`);
  };

  /** `"1-3,8"` (1-based, `5-` to the end) → 0-based page numbers. */
  /**
   * `count`: the document's pages (checked, and the end of open ranges);
   * `null`: no document, only the syntax is checked (and `[]` returned);
   * `undefined`: no document known, open ranges refused.
   */
  function parsePages(k, s, count) {
    if (typeof s === "number") s = String(s);
    if (Array.isArray(s)) s = s.join(",");
    if (typeof s !== "string") bad(k, "expected pages like 1-3,8");
    const out = [];
    for (const part of s.split(",").map((x) => x.trim()).filter(Boolean)) {
      const m = /^(\d*)\s*(-?)\s*(\d*)$/.exec(part);
      if (!m || (!m[1] && !m[3])) bad(k, `${JSON.stringify(part)} is not a page or a range`);
      const a = m[1] ? Number(m[1]) : 1;
      if (m[2] && !m[3] && count === undefined) bad(k, `${JSON.stringify(part)}: an open range needs the page count`);
      const b = m[2] ? (m[3] ? Number(m[3]) : (count ?? a)) : a;
      if (a < 1 || b < a) bad(k, `${JSON.stringify(part)}: pages count from 1`);
      if (count === null) continue; // only checked: no document yet
      if (count !== undefined && b > count) bad(k, `${JSON.stringify(part)}: the document has ${count} pages`);
      for (let i = a; i <= b; i++) out.push(i - 1);
    }
    return out;
  }

  /** One parameter as given (strings from a command line too) → its value. */
  function param(k, p, v, ctx) {
    switch (p.type) {
      case "string":
      case "text":
        if (typeof v !== "string") bad(k, "expected a string");
        return v;
      case "number":
      case "integer": {
        const n = typeof v === "string" && v.trim() !== "" ? Number(v) : v;
        if (typeof n !== "number" || !Number.isFinite(n)) bad(k, `expected a number, not ${JSON.stringify(v)}`);
        if (p.type === "integer" && !Number.isInteger(n)) bad(k, `expected an integer, not ${n}`);
        return n;
      }
      case "boolean":
        if (typeof v === "boolean") return v;
        if (["true", "yes", "1", "on"].includes(String(v).toLowerCase())) return true;
        if (["false", "no", "0", "off"].includes(String(v).toLowerCase())) return false;
        return bad(k, `expected true or false, not ${JSON.stringify(v)}`);
      case "enum":
        if (!p.values.includes(v)) bad(k, `${JSON.stringify(v)} is not one of ${p.values.join(", ")}`);
        return v;
      case "pages":
        return parsePages(k, v, ctx ? ctx.pageCount : null);
      case "json":
        return v;
      case "bytes":
        if (v instanceof Uint8Array) return v;
        if (v && typeof v.$bytes === "string") return unbase64(v.$bytes);
        if (typeof v === "string") return utf8.encode(v);
        return bad(k, "expected bytes");
      case "pdf":
        if (v instanceof Doc) return v;
        if (!ctx) return v; // only checked: documents are opened to run
        if (v && ctx && ctx.docs[v.$doc]) return ctx.docs[v.$doc];
        return bad(k, "expected a PDF document");
    }
  }

  function params(meta, raw, ctx) {
    const out = {};
    for (const k of Object.keys(raw)) if (!(k in meta.params)) bad(k, `unknown (the macro takes ${Object.keys(meta.params).join(", ") || "none"})`);
    for (const [k, p] of Object.entries(meta.params)) {
      if (raw[k] !== undefined && raw[k] !== null) out[k] = param(k, p, raw[k], ctx);
      else if (p.default !== undefined) out[k] = param(k, p, p.default, ctx);
      else if (!p.optional) bad(k, "missing");
    }
    return out;
  }

  // ------------------------------------------------------------- results

  function replacer(_k, v) {
    if (v instanceof Uint8Array) return Array.from(v);
    if (v instanceof Map) return Object.fromEntries(v);
    if (v instanceof Set) return Array.from(v);
    if (typeof v === "bigint") return Number(v);
    return v;
  }

  /** A stack without the prelude's frames: what the macro's author wrote. */
  function ownFrames(stack) {
    if (typeof stack !== "string") return null;
    const lines = stack.split("\n").filter((l) => l.trim() && !l.includes("prelude.js"));
    return lines.length ? lines.join("\n") : null;
  }

  // ------------------------------------------------------------- entry points

  const pdf = {
    API,
    name: (s) => new Name(s),
    str: (s) => (typeof s === "string" ? new PdfString(s, null, false) : new PdfString(latin1(toBytes(s)), toBytes(s), false)),
    ref: (num, generation = 0) => new Ref(num, generation),
    raw: (text) => new Raw(text),
    pages: (spec, count) => parsePages("pages", spec, count ?? undefined),
    Name,
    PdfString,
    Ref,
    Node,
    Doc,
    Page,
    Content,
    PdfiError,
  };
  Object.defineProperty(globalThis, "pdf", { value: Object.freeze(pdf), enumerable: false });

  Object.defineProperty(globalThis, "__pdfi", {
    enumerable: false,
    value: Object.freeze({
      /** The module's `meta`, checked, as JSON. */
      meta(ns, fallbackName) {
        return JSON.stringify(checkMeta(ns.meta, fallbackName));
      },

      /** Check `raw` parameters against `metaJson` without a document. */
      checkParams(metaJson, rawJson) {
        params(JSON.parse(metaJson), JSON.parse(rawJson), null);
      },

      /**
       * Run the default export. Resolves to JSON: `{ ok: true, value }` or
       * `{ ok: false, error: { code, message, stack } }`.
       */
      async run(ns, metaJson, docId, rawJson, inputJson) {
        try {
          const meta = JSON.parse(metaJson);
          if (typeof ns.default !== "function") throw new TypeError("the module has no default export function");
          const input = JSON.parse(inputJson);
          const summary = async (id) => body({ op: "summary", doc: id });
          const doc = new Doc(docId, await summary(docId), false);
          const raw = JSON.parse(rawJson);
          const docs = {};
          for (const [k, v] of Object.entries(raw)) {
            if (meta.params[k]?.type === "pdf" && v && typeof v.$doc === "number") docs[v.$doc] = new Doc(v.$doc, await summary(v.$doc), true);
          }
          const args = params(meta, raw, { pageCount: doc.pageCount, docs });
          const ctx = Object.freeze({
            log: (...a) => host.log("info", format(a)),
            warn: (...a) => host.log("warn", format(a)),
            progress: (done, total, label) => host.progress(Number(done) || 0, Number(total) || 0, label === undefined ? null : String(label)),
            get cancelled() {
              return !!host.cancelled();
            },
            emit: (name, data, mime) => {
              if (typeof name !== "string" || !name || /[\/\\]/.test(name) || name === "." || name === "..")
                throw new TypeError(`ctx.emit: ${JSON.stringify(name)} is not a file name`);
              const bytes = typeof data === "object" && !(data instanceof Uint8Array) && !ArrayBuffer.isView(data) && !(data instanceof ArrayBuffer)
                ? utf8.encode(JSON.stringify(data, replacer, 2))
                : toBytes(data);
              host.emit(name, latin1(bytes), mime ?? null);
            },
            input: Object.freeze({ ...input.input }),
            host: input.host,
            meta: Object.freeze(meta),
          });
          const value = await ns.default(doc, args, ctx);
          // Keys of rows in the order the macro wrote them (JSON objects
          // lose it on the host's side), for showing them as a table.
          let columns = null;
          if (Array.isArray(value) && value.length && value.every((r) => r && typeof r === "object" && !Array.isArray(r))) {
            columns = [];
            for (const r of value) for (const k of Object.keys(r)) if (!columns.includes(k)) columns.push(k);
          }
          return JSON.stringify({ ok: true, value: value === undefined ? null : value, columns }, replacer);
        } catch (e) {
          const error = e instanceof Error
            ? { code: e instanceof PdfiError ? e.code : e.name, message: e.message, stack: ownFrames(e.stack) }
            : { code: "Error", message: show(e), stack: null };
          return JSON.stringify({ ok: false, error });
        }
      },
    }),
  });
})();
