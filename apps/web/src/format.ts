import type { AttrOrigin, Destination, LinkTarget, ObjRef, Rect, Span, Value, XrefEntry } from "./protocol/generated";

export const refText = (r: ObjRef) => `${r.num} ${r.generation} R`;
export const objText = (r: ObjRef) => `${r.num} ${r.generation} obj`;

export function spanText(s: Span): string {
  return `${s.start.toLocaleString()}–${s.end.toLocaleString()} (${(s.end - s.start).toLocaleString()} B)`;
}

export function bytesText(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KiB", "MiB", "GiB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v < 10 ? 1 : 0)} ${units[i]}`;
}

/** Short inline rendering of a value, close to PDF syntax. */
export function valueText(v: Value): string {
  switch (v.type) {
    case "null":
      return "null";
    case "bool":
      return String(v.value);
    case "integer":
      return String(v.value);
    case "real":
      return String(v.value);
    case "string":
      return v.kind === "hex" && !isPrintable(v.text)
        ? `<${v.hex}${v.truncated ? "…" : ""}>`
        : `(${v.text}${v.truncated ? "…" : ""})`;
    case "name":
      return `/${v.name}`;
    case "array":
      return `[ ${v.len} items ]`;
    case "dict":
      return `<< ${v.len} keys >>${v.type_name ? ` /${v.type_name}` : ""}`;
    case "stream":
      return `stream << ${v.len} keys >>${v.type_name ? ` /${v.type_name}` : ""} · ${bytesText(v.raw_len)}`;
    case "ref":
      return refText(v.id);
  }
}

function isPrintable(s: string): boolean {
  return !/[\u0000-\u0008\u000e-\u001f�]/.test(s);
}

export function hasChildren(v: Value): boolean {
  return (v.type === "array" || v.type === "dict" || v.type === "stream") && v.len > 0;
}

export function entryText(e: XrefEntry): string {
  switch (e.kind) {
    case "free":
      return `free (next ${e.next})`;
    case "inUse":
      return `@${e.offset.toLocaleString()}`;
    case "compressed":
      return `in ${e.stream} [${e.index}]`;
    case "edited":
      return "edited (not saved)";
  }
}

export function entryGeneration(e: XrefEntry): number {
  return e.kind === "compressed" ? 0 : e.generation;
}

const HEX = Array.from({ length: 256 }, (_, i) => i.toString(16).padStart(2, "0"));

export const hexByte = (b: number) => HEX[b];

export function asciiByte(b: number): string {
  return b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : "·";
}

/** Bytes as text: Latin-1 with control characters (except whitespace) shown as `·`. */
export function bytesAsText(data: Uint8Array): string {
  let out = "";
  for (let i = 0; i < data.length; i += 0x8000) {
    const chunk = data.subarray(i, i + 0x8000);
    out += String.fromCharCode(
      ...Array.from(chunk, (b) => (b < 0x20 && b !== 9 && b !== 10 && b !== 13) || b === 0x7f ? 0xb7 : b),
    );
  }
  return out;
}

export const rectText = (r: Rect | null) => (r ? `[${r.x0} ${r.y0} ${r.x1} ${r.y1}]` : "—");

const num = (n: number) => (Number.isInteger(n) ? String(n) : n.toFixed(2).replace(/\.?0+$/, ""));

/** `612 × 792 pt`. */
export const sizeText = (r: Rect | null) => (r ? `${num(r.x1 - r.x0)} × ${num(r.y1 - r.y0)} pt` : "—");

/** Page number as shown to people (1-based), with the label when it differs. */
export function pageText(index: number, label?: string | null): string {
  const n = String(index + 1);
  return label && label !== n ? `${n} (${label})` : n;
}

export function originText(o: AttrOrigin): string {
  switch (o.kind) {
    case "page":
      return "on the page";
    case "inherited":
      return `inherited from ${objText(o.from)}`;
    case "default":
      return o.of === "0" ? "default" : `default (= ${o.of})`;
    case "missing":
      return "missing";
  }
}

export function destText(d: Destination): string {
  const where =
    d.page_index !== null
      ? `page ${d.page_index + 1}`
      : d.remote_page !== null
        ? `page ${d.remote_page + 1} of another file`
        : (d.error ?? "?");
  const fit = d.fit ? ` /${d.fit}${d.params.map((p) => ` ${p ?? "null"}`).join("")}` : "";
  return `${d.named !== null ? `“${d.named}” → ` : ""}${where}${fit}`;
}

export function linkText(l: LinkTarget): string {
  switch (l.kind) {
    case "dest":
      return destText(l.dest);
    case "uri":
      return l.uri;
    case "action":
      return l.detail ? `${l.action}: ${l.detail}` : l.action;
  }
}
