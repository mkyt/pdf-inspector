import { Group, Loader, Stack, Text } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { api, useBackend } from "../api";
import { useNav } from "../nav";
import type { NodeView, ObjRef, StreamInfo } from "../protocol/generated";

/** Decoded data we fetch for a preview (one request). */
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
  const supported =
    d &&
    (jpeg ||
      (info.filters.every(
        (f) =>
          f === "FlateDecode" ||
          f === "LZWDecode" ||
          f === "ASCII85Decode" ||
          f === "ASCIIHexDecode" ||
          f === "RunLengthDecode",
      ) &&
        (d.bits === 8 || (d.bits === 1 && (d.mask || d.space === "DeviceGray")))));
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

const frame = {
  border: "1px solid var(--mantine-color-default-border)",
  background: "repeating-conic-gradient(#ccc 0 25%, #fff 0 50%) 0 0 / 16px 16px",
  alignSelf: "flex-start",
} as const;

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
    const c = ref.current;
    const ctx = c?.getContext("2d");
    if (!c || !ctx) return;
    const { width: w, height: h } = d;
    const img = ctx.createImageData(w, h);
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
    } else {
      const n = bytes.length / (w * h);
      const comps =
        d.space === "DeviceGray" ? 1 : d.space === "DeviceRGB" ? 3 : d.space === "DeviceCMYK" ? 4 : Math.floor(n);
      if (![1, 3, 4].includes(comps) || bytes.length < w * h * comps) {
        setError(`cannot tell the samples apart (${bytes.length} bytes for ${w}×${h})`);
        return;
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
    }
    ctx.putImageData(img, 0, 0);
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
