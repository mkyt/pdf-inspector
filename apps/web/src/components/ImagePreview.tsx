import { Button, Group, Loader, Stack, Text } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { api, useBackend } from "../api";
import type { Backend } from "../backend/backend";
import { useDoc } from "../doc";
import { useNav } from "../nav";
import type { DocId, NodeView, ObjRef, StreamInfo } from "../protocol/generated";

/** Decoded data we fetch for a preview (one request; also the protocol's chunk limit). */
const MAX_DATA = 4 << 20;
const MAX_SIDE = 320;

interface ImageDict {
  width: number;
  height: number;
  bits: number;
  /** Device color space name, if it is one. */
  space: string | null;
  mask: boolean;
}

function readDict(nodes: NodeView[]): ImageDict | null {
  const get = (k: string) => nodes.find((n) => n.key === k)?.value;
  const int = (k: string) => {
    const v = get(k);
    return v?.type === "integer" ? v.value : null;
  };
  const sub = get("Subtype");
  if (sub?.type !== "name" || sub.name !== "Image") return null;
  const width = int("Width");
  const height = int("Height");
  if (!width || !height) return null;
  const cs = get("ColorSpace");
  const mask = get("ImageMask");
  return {
    width,
    height,
    bits: int("BitsPerComponent") ?? 1,
    space: cs?.type === "name" ? cs.name : null,
    mask: mask?.type === "bool" && mask.value,
  };
}

/**
 * A preview of an image XObject, drawn in the browser: JPEG (DCTDecode) as
 * is, uncompressed or Flate data with 8-bit gray / RGB / CMYK samples (and
 * 1-bit masks). Everything else waits for the renderer.
 */
export function ImagePreview({ id, info }: { id: ObjRef; info: StreamInfo }) {
  const backend = useBackend();
  const nav = useNav();
  const dict = useQuery({
    queryKey: ["doc", nav.doc, "imageDict", id.num, id.generation],
    queryFn: async () =>
      readDict(
        (await api.children(backend, nav.doc, { root: { kind: "object", id }, path: [] }, { offset: 0, limit: 200 }))
          .items,
      ),
  });
  const d = dict.data;
  const jpeg = info.filters[info.filters.length - 1] === "DCTDecode";
  const supported = d && (jpeg || drawable(d, info));
  const data = useQuery({
    queryKey: ["doc", nav.doc, "imageData", id.num, id.generation],
    queryFn: () => api.streamData(backend, nav.doc, id, true, 0, MAX_DATA),
    enabled: !!supported,
  });
  if (!d) return null;
  // Large images shrink to fit; small ones grow (up to 32×) so that they can be seen.
  const scale = Math.min(32, MAX_SIDE / Math.max(d.width, d.height));
  return (
    <Stack gap={4}>
      <Group gap={6}>
        <Text size="xs" fw={600}>
          Image
        </Text>
        <Text size="xs" c="dimmed">
          {d.width} × {d.height} · {d.mask ? "ImageMask" : (d.space ?? "other color space")} · {d.bits} bpc
        </Text>
        <Download id={id} d={d} kind={downloadKind(d, info)} />
        {drawable(d, info) && <Download id={id} d={d} kind={samplesKind(info)} />}
      </Group>
      {!supported ? (
        <Text size="xs" c="dimmed">
          No preview for this encoding yet (comes with the renderer).
        </Text>
      ) : !data.data ? (
        <Loader size="xs" />
      ) : !data.data.info.eof && !jpeg ? (
        <Text size="xs" c="dimmed">
          Too large to preview here.
        </Text>
      ) : jpeg ? (
        <Jpeg bytes={data.data.data} width={d.width * scale} />
      ) : (
        <Raster d={d} bytes={data.data.data} scale={scale} />
      )}
    </Stack>
  );
}

const PLAIN_FILTERS = ["FlateDecode", "LZWDecode", "ASCII85Decode", "ASCIIHexDecode", "RunLengthDecode"];

/** Samples we can turn into pixels ourselves. */
function drawable(d: ImageDict, info: StreamInfo): boolean {
  return (
    info.filters.every((f) => PLAIN_FILTERS.includes(f)) &&
    (d.bits === 8 || (d.bits === 1 && (d.mask || d.space === "DeviceGray")))
  );
}

interface DownloadKind {
  label: string;
  ext: string;
}

/**
 * What a download saves: JPEG and JPEG 2000 data as stored (they are image
 * files already), samples we can draw as PNG, anything else as the stream
 * data decoded as far as we can.
 */
function downloadKind(d: ImageDict, info: StreamInfo): DownloadKind {
  const last = info.filters[info.filters.length - 1];
  if (last === "DCTDecode") return { label: "JPEG", ext: "jpg" };
  if (last === "JPXDecode") return { label: "JPEG 2000", ext: "jp2" };
  if (drawable(d, info)) return { label: "PNG", ext: "png" };
  return samplesKind(info);
}

/** The stream data decoded as far as we can, without conversion. */
function samplesKind(info: StreamInfo): DownloadKind {
  const last = info.filters[info.filters.length - 1];
  return { label: last && !PLAIN_FILTERS.includes(last) ? `data (/${last})` : "samples", ext: "bin" };
}

/** All of a stream's decoded data, in protocol-sized chunks. */
async function streamAll(b: Backend, doc: DocId, id: ObjRef): Promise<Uint8Array> {
  const parts: Uint8Array[] = [];
  let offset = 0;
  for (;;) {
    const r = await api.streamData(b, doc, id, true, offset, MAX_DATA);
    if (r.info.errors.length) throw new Error(r.info.errors.join("; "));
    parts.push(r.data);
    offset += r.data.length;
    if (r.info.eof || r.data.length === 0) break;
  }
  const out = new Uint8Array(offset);
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
}

async function imageBlob(d: ImageDict, ext: string, bytes: Uint8Array): Promise<Blob> {
  if (ext !== "png") {
    const type = ext === "jpg" ? "image/jpeg" : ext === "jp2" ? "image/jp2" : "application/octet-stream";
    return new Blob([bytes as BlobPart], { type });
  }
  const img = rasterize(d, bytes);
  if (typeof img === "string") throw new Error(img);
  const c = new OffscreenCanvas(d.width, d.height);
  c.getContext("2d")!.putImageData(img, 0, 0);
  return c.convertToBlob({ type: "image/png" });
}

function save(blob: Blob, name: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

function Download({ id, d, kind }: { id: ObjRef; d: ImageDict; kind: DownloadKind }) {
  const backend = useBackend();
  const nav = useNav();
  const { summary } = useDoc();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { label, ext } = kind;
  const stem = summary.info.name.replace(/\.pdf$/i, "");
  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const blob = await imageBlob(d, ext, await streamAll(backend, nav.doc, id));
      save(blob, `${stem}-${id.num}-${id.generation}.${ext}`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <Button size="compact-xs" variant="subtle" loading={busy} onClick={() => void run()}>
        Download {label}
      </Button>
      {error && (
        <Text size="xs" c="red">
          {error}
        </Text>
      )}
    </>
  );
}

const frame = {
  border: "1px solid var(--mantine-color-default-border)",
  background: "repeating-conic-gradient(#ccc 0 25%, #fff 0 50%) 0 0 / 16px 16px",
  alignSelf: "flex-start",
} as const;

/** Samples as RGBA pixels, or why they cannot be. */
function rasterize(d: ImageDict, bytes: Uint8Array): ImageData | string {
  const { width: w, height: h } = d;
  const img = new ImageData(w, h);
  const px = img.data;
  if (d.bits === 1) {
    const row = Math.ceil(w / 8);
    for (let y = 0; y < h; y++)
      for (let x = 0; x < w; x++) {
        const bit = (bytes[y * row + (x >> 3)] >> (7 - (x & 7))) & 1;
        const o = 4 * (y * w + x);
        // Image masks paint where the sample is 0; gray 1 is white.
        const v = d.mask ? (bit ? 255 : 0) : bit * 255;
        px[o] = px[o + 1] = px[o + 2] = d.mask ? 0 : v;
        px[o + 3] = d.mask ? 255 - v : 255;
      }
    return img;
  }
  const n = bytes.length / (w * h);
  const comps =
    d.space === "DeviceGray" ? 1 : d.space === "DeviceRGB" ? 3 : d.space === "DeviceCMYK" ? 4 : Math.floor(n);
  if (![1, 3, 4].includes(comps) || bytes.length < w * h * comps) {
    return `cannot tell the samples apart (${bytes.length} bytes for ${w}×${h})`;
  }
  for (let i = 0; i < w * h; i++) {
    const s = i * comps;
    const o = 4 * i;
    if (comps === 1) px[o] = px[o + 1] = px[o + 2] = bytes[s];
    else if (comps === 3) {
      px[o] = bytes[s];
      px[o + 1] = bytes[s + 1];
      px[o + 2] = bytes[s + 2];
    } else {
      const k = bytes[s + 3];
      px[o] = 255 - Math.min(255, bytes[s] + k);
      px[o + 1] = 255 - Math.min(255, bytes[s + 1] + k);
      px[o + 2] = 255 - Math.min(255, bytes[s + 2] + k);
    }
    px[o + 3] = 255;
  }
  return img;
}

function Jpeg({ bytes, width }: { bytes: Uint8Array; width: number }) {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    const u = URL.createObjectURL(new Blob([bytes as BlobPart], { type: "image/jpeg" }));
    setUrl(u);
    return () => URL.revokeObjectURL(u);
  }, [bytes]);
  return url ? <img src={url} width={width} style={frame} alt="" /> : null;
}

function Raster({ d, bytes, scale }: { d: ImageDict; bytes: Uint8Array; scale: number }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const ctx = ref.current?.getContext("2d");
    if (!ctx) return;
    const img = rasterize(d, bytes);
    if (typeof img === "string") setError(img);
    else ctx.putImageData(img, 0, 0);
  }, [d, bytes]);
  if (error)
    return (
      <Text size="xs" c="dimmed">
        {error}
      </Text>
    );
  return (
    <canvas
      ref={ref}
      width={d.width}
      height={d.height}
      style={{
        ...frame,
        width: d.width * scale,
        imageRendering: scale > 1 || d.width < 64 ? "pixelated" : "auto",
      }}
    />
  );
}
