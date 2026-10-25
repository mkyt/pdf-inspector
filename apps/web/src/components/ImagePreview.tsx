import { Button, Group, Loader, Stack, Text } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { api, useBackend } from "../api";
import type { Backend } from "../backend/backend";
import { useDoc } from "../doc";
import { useNav } from "../nav";
import type { DecodedImage, DocId, ImageFile, NodeView, ObjRef, StreamInfo } from "../protocol/generated";

/** The protocol's chunk limit for stream data. */
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
 * A preview of an image XObject as the renderer decodes it: `/Decode`,
 * color spaces and tint transforms, soft masks.
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
  // Decoded at about twice the preview's size.
  const reduce = d ? Math.max(1, Math.floor(Math.max(d.width, d.height) / (2 * MAX_SIDE))) : 1;
  const image = useQuery({
    queryKey: ["doc", nav.doc, "decodedImage", id.num, id.generation, reduce],
    queryFn: () => api.decodeImage(backend, nav.doc, id, reduce),
    enabled: !!d,
  });
  if (!d) return null;
  // Large images shrink to fit; small ones grow (up to 32×) so that they can be seen.
  const scale = Math.min(32, MAX_SIDE / Math.max(d.width, d.height));
  const decoded = image.data?.info;
  return (
    <Stack gap={4}>
      <Group gap={6}>
        <Text size="xs" fw={600}>
          Image
        </Text>
        <Text size="xs" c="dimmed">
          {d.width} × {d.height} · {d.mask ? "ImageMask" : (d.space ?? "other color space")} · {d.bits} bpc
        </Text>
        {decoded?.file && <Download id={id} kind={storedKind(decoded.file)} />}
        {decoded && decoded.part !== null && <Download id={id} kind={{ label: "PNG", ext: "png" }} />}
        <Download id={id} kind={samplesKind(info)} />
      </Group>
      {image.error ? (
        <Text size="xs" c="dimmed">
          {image.error.message}
        </Text>
      ) : !image.data ? (
        <Loader size="xs" />
      ) : image.data.info.part === null ? (
        <Text size="xs" c="dimmed">
          {image.data.info.error ?? "No preview for this image."}
        </Text>
      ) : (
        <Raster img={image.data.info} data={image.data.data} width={d.width * scale} height={d.height * scale} />
      )}
    </Stack>
  );
}

const PLAIN_FILTERS = ["FlateDecode", "LZWDecode", "ASCII85Decode", "ASCIIHexDecode", "RunLengthDecode"];

interface DownloadKind {
  label: string;
  ext: string;
}

/** A JPEG or JPEG 2000 file as stored. */
function storedKind(file: ImageFile): DownloadKind {
  return file === "jpeg" ? { label: "JPEG", ext: "jpg" } : { label: "JPEG 2000", ext: "jp2" };
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

/** What a download saves: the image as it paints (PNG), or the stream data as stored. */
async function imageBlob(b: Backend, doc: DocId, id: ObjRef, ext: string): Promise<Blob> {
  if (ext === "png") {
    const { info, data } = await api.decodeImage(b, doc, id, 1);
    if (info.part === null) throw new Error(info.error ?? "cannot decode the image");
    const c = new OffscreenCanvas(info.width, info.height);
    c.getContext("2d")!.putImageData(imageData(info, data), 0, 0);
    return c.convertToBlob({ type: "image/png" });
  }
  const type = ext === "jpg" ? "image/jpeg" : ext === "jp2" ? "image/jp2" : "application/octet-stream";
  return new Blob([(await streamAll(b, doc, id)) as BlobPart], { type });
}

function imageData(info: DecodedImage, data: Uint8Array): ImageData {
  return new ImageData(new Uint8ClampedArray(data.buffer as ArrayBuffer, data.byteOffset, data.byteLength), info.width, info.height);
}

export function save(blob: Blob, name: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

function Download({ id, kind }: { id: ObjRef; kind: DownloadKind }) {
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
      save(await imageBlob(backend, nav.doc, id, ext), `${stem}-${id.num}-${id.generation}.${ext}`);
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

function Raster({ img, data, width, height }: { img: DecodedImage; data: Uint8Array; width: number; height: number }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    ref.current?.getContext("2d")?.putImageData(imageData(img, data), 0, 0);
  }, [img, data]);
  return (
    <canvas
      ref={ref}
      width={img.width}
      height={img.height}
      style={{
        ...frame,
        width,
        height,
        imageRendering: width > img.width || img.width < 64 ? "pixelated" : "auto",
      }}
    />
  );
}
