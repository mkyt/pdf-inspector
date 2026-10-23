import { ActionIcon, Button, Group, Loader, Text, Tooltip } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import { api, useBackend } from "../api";
import { contentKey, useNav } from "../nav";
import type { AppearancePick, ContentRef, ContentSummary, PixelRect, RenderLayer, RenderNote } from "../protocol/generated";
import { AnnotCard, AnnotOverlay, annotsAt, toView } from "./AnnotOverlay";
import classes from "./RenderView.module.css";

/** Side of a rendered tile, in device pixels. */
const TILE = 512;
/** Click tolerance, in CSS pixels. */
const HIT_RADIUS = 4;
/** Largest highlight bitmap side; beyond it only the visible part is asked for. */
const MAX_HIGHLIGHT = 2048;
/** Annotations of a page fetched for the overlay. */
const MAX_ANNOTS = 2000;
const ZOOMS = [0.1, 0.17, 0.25, 0.33, 0.5, 0.67, 0.75, 1, 1.25, 1.5, 2, 3, 4, 6, 8, 12, 16];

/** Bitmap size in device pixels, as the backend computes it. */
const pixels = (v: number, scale: number) => Math.max(1, Math.ceil(v * scale));

function sameNotes(a: RenderNote[] | null, b: RenderNote[]) {
  return a !== null && a.length === b.length && a.every((n, i) => n.message === b[i].message && n.count === b[i].count);
}

function paint(canvas: HTMLCanvasElement | null, data: Uint8Array, w: number, h: number) {
  if (!canvas || data.length < w * h * 4) return;
  canvas.width = w;
  canvas.height = h;
  const px = new Uint8ClampedArray(data.buffer as ArrayBuffer, data.byteOffset, w * h * 4);
  canvas.getContext("2d")?.putImageData(new ImageData(px, w, h), 0, 0);
}

/**
 * A rendered content, drawn in tiles of the visible area. Clicking selects
 * the topmost operation painting there (again: the one below it); the
 * selected operation is highlighted; with `step`, only the operations up
 * to the selected one are drawn. A page's annotations can be shown over
 * it: their appearance streams (painted after the content, in `/Annots`
 * order) and outlines of where they are; a label selects one.
 */
export function RenderView({
  content,
  summary,
  op,
  step,
  onPick,
  height,
  annot: annotProp,
  onAnnot,
}: {
  content: ContentRef;
  summary: ContentSummary;
  op: number | null;
  step: boolean;
  onPick: (op: number) => void;
  /** CSS height of the scrolling area. */
  height: number | string;
  /** The selected annotation (by `/Annots` index), when the parent keeps it. */
  annot?: number | null;
  onAnnot?: (index: number | null) => void;
}) {
  const backend = useBackend();
  const nav = useNav();
  const v = summary.view;
  const box = useRef<HTMLDivElement>(null);
  const [port, setPort] = useState({ left: 0, top: 0, width: 0, height: 0 });
  // CSS pixels per point; null: fit the whole view in the box.
  const [zoom, setZoom] = useState<number | null>(null);
  const [notes, setNotes] = useState<RenderNote[] | null>(null);
  const [hint, setHint] = useState<string | null>(null);
  const last = useRef<{ x: number; y: number; ops: number[] } | null>(null);
  const dpr = window.devicePixelRatio || 1;
  const [ownAnnot, setOwnAnnot] = useState<number | null>(null);
  const annot = annotProp !== undefined ? annotProp : ownAnnot;
  const setAnnot = onAnnot ?? setOwnAnnot;
  const [showAp, setShowAp] = useState(true);
  // Appearances shown instead of the normal ones (rollover, down, other states), by annotation.
  const [picks, setPicks] = useState<Record<number, AppearancePick>>({});
  const annotsLayer = useMemo<RenderLayer>(() => ({ kind: "annots", only: null, pick: Object.values(picks) }), [picks]);
  const [showOutlines, setShowOutlines] = useState(true);
  const page = content.kind === "page" ? content.index : null;
  useEffect(() => setPicks({}), [page]);
  const annotsQ = useQuery({
    queryKey: ["doc", nav.doc, "annots", page, "overlay"],
    queryFn: () => api.annotations(backend, nav.doc, page!, { offset: 0, limit: MAX_ANNOTS }),
    enabled: page !== null,
  });
  const annots = annotsQ.data?.items ?? [];
  const selAnnot = annots.find((a) => a.index === annot) ?? null;

  const measure = useCallback(() => {
    const el = box.current;
    if (el) setPort({ left: el.scrollLeft, top: el.scrollTop, width: el.clientWidth, height: el.clientHeight });
  }, []);
  useLayoutEffect(() => {
    measure();
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [measure]);

  const fit =
    port.width > 0 && port.height > 0
      ? Math.max(0.05, Math.min((port.width - 2) / v.width, (port.height - 2) / v.height))
      : 1;
  const z = zoom ?? fit;
  const scale = z * dpr;
  const fw = pixels(v.width, scale);
  const fh = pixels(v.height, scale);
  const until = step && op !== null ? op : null;

  // Tiles overlapping the visible area (device pixels).
  const tiles = useMemo(() => {
    const out: PixelRect[] = [];
    const x0 = Math.floor((port.left * dpr) / TILE);
    const y0 = Math.floor((port.top * dpr) / TILE);
    const x1 = Math.min(Math.ceil(((port.left + port.width) * dpr) / TILE), Math.ceil(fw / TILE));
    const y1 = Math.min(Math.ceil(((port.top + port.height) * dpr) / TILE), Math.ceil(fh / TILE));
    for (let ty = y0; ty < y1; ty++)
      for (let tx = x0; tx < x1; tx++) {
        const x = tx * TILE;
        const y = ty * TILE;
        out.push({ x, y, width: Math.min(TILE, fw - x), height: Math.min(TILE, fh - y) });
      }
    return out;
  }, [port, dpr, fw, fh]);

  const onNotes = useCallback((n: RenderNote[]) => setNotes((p) => (sameNotes(p, n) ? p : n)), []);

  // The selected operation: what it paints, for the highlight.
  const sel = useQuery({
    queryKey: ["doc", nav.doc, "content", ...contentKey(content), "op", op],
    queryFn: async () => (await api.contentOps(backend, nav.doc, content, { offset: op!, limit: 1 })).items[0] ?? null,
    enabled: op !== null,
  });
  const el = op !== null && sel.data?.index === op ? sel.data.element : null;
  const hl = useMemo(() => {
    if (!el) return null;
    const r = toView(el.bbox, v);
    // Device pixels, grown a little for strokes drawn at least 3 px wide.
    let x0 = Math.max(0, Math.floor(r.x0 * scale) - 4);
    let y0 = Math.max(0, Math.floor(r.y0 * scale) - 4);
    let x1 = Math.min(fw, Math.ceil(r.x1 * scale) + 4);
    let y1 = Math.min(fh, Math.ceil(r.y1 * scale) + 4);
    if (x1 - x0 > MAX_HIGHLIGHT || y1 - y0 > MAX_HIGHLIGHT) {
      x0 = Math.max(x0, Math.floor(port.left * dpr));
      y0 = Math.max(y0, Math.floor(port.top * dpr));
      x1 = Math.min(x1, Math.ceil((port.left + port.width) * dpr));
      y1 = Math.min(y1, Math.ceil((port.top + port.height) * dpr));
    }
    if (x1 <= x0 || y1 <= y0) return { box: r, tile: null };
    return { box: r, tile: { x: x0, y: y0, width: x1 - x0, height: y1 - y0 } };
  }, [el, v, scale, fw, fh, port, dpr]);

  const onClick = async (e: MouseEvent<HTMLDivElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const cx = e.clientX - r.left;
    const cy = e.clientY - r.top;
    const ops = await api.hitTest(backend, nav.doc, content, cx / z, cy / z, HIT_RADIUS / z, until);
    // The click in the page's default space, for the annotations there.
    const [a, b, c, d, e2, f] = v.matrix;
    const det = a * d - b * c;
    const vx = cx / z - e2;
    const vy = cy / z - f;
    const over = showOutlines || showAp ? annotsAt(annots, (d * vx - c * vy) / det, (a * vy - b * vx) / det) : [];
    const overText =
      over.length > 0
        ? ` Over annotation${over.length > 1 ? "s" : ""} ${over.map((x) => `[${x.index}] ${x.subtype ?? "?"}`).join(", ")} (select one by its label).`
        : "";
    const prev = last.current;
    const again =
      prev && Math.abs(prev.x - cx) <= 3 && Math.abs(prev.y - cy) <= 3 && prev.ops.join() === ops.join();
    last.current = { x: cx, y: cy, ops };
    if (ops.length === 0) {
      setHint(`Nothing in the content is painted here.${overText}`);
      return;
    }
    const k = again && op !== null && ops.includes(op) ? (ops.indexOf(op) + 1) % ops.length : 0;
    onPick(ops[k]);
    const more =
      ops.length > 1
        ? `${ops.length} operations paint here (${ops.map((o) => `#${o}`).join(", ")}, topmost first); click again for the next.`
        : "";
    setHint((more + overText).trim() || null);
  };

  const step2 = (d: number) => {
    const i = ZOOMS.findIndex((x) => x >= z - 1e-6);
    const next = d > 0 ? ZOOMS.find((x) => x > z + 1e-6) : [...ZOOMS].reverse().find((x) => x < z - 1e-6);
    setZoom(next ?? ZOOMS[i] ?? z);
  };

  return (
    <div>
      <Group gap={4} mb={4}>
        <Tooltip label="Zoom out">
          <ActionIcon size="sm" variant="subtle" onClick={() => step2(-1)} aria-label="Zoom out">
            −
          </ActionIcon>
        </Tooltip>
        <Text size="xs" w={44} ta="center">
          {Math.round(z * 100)}%
        </Text>
        <Tooltip label="Zoom in">
          <ActionIcon size="sm" variant="subtle" onClick={() => step2(1)} aria-label="Zoom in">
            +
          </ActionIcon>
        </Tooltip>
        <Tooltip label="Fit">
          <ActionIcon size="sm" variant={zoom === null ? "light" : "subtle"} onClick={() => setZoom(null)} aria-label="Fit">
            ↔
          </ActionIcon>
        </Tooltip>
        <Tooltip label="Actual size (100%)">
          <ActionIcon size="sm" variant={zoom === 1 ? "light" : "subtle"} onClick={() => setZoom(1)} aria-label="Actual size">
            1:1
          </ActionIcon>
        </Tooltip>
        {annots.length > 0 && (
          <>
            <Tooltip label="Paint the annotations' appearance streams over the content">
              <Button size="compact-xs" variant={showAp ? "light" : "subtle"} ml="xs" onClick={() => setShowAp(!showAp)}>
                Appearances
              </Button>
            </Tooltip>
            <Tooltip label="Outline each annotation's /Rect, /QuadPoints and paths">
              <Button size="compact-xs" variant={showOutlines ? "light" : "subtle"} onClick={() => setShowOutlines(!showOutlines)}>
                Outlines ({annots.length.toLocaleString()})
              </Button>
            </Tooltip>
          </>
        )}
        <Text size="xs" c="dimmed" style={{ marginLeft: "auto" }}>
          {Math.round(v.width)} × {Math.round(v.height)} pt{v.rotate ? ` · /Rotate ${v.rotate}` : ""}
        </Text>
      </Group>
      <div
        ref={box}
        onScroll={measure}
        style={{
          height,
          overflow: "auto",
          background: "var(--mantine-color-gray-light)",
          border: "1px solid var(--mantine-color-default-border)",
        }}
      >
        <div
          onClick={onClick}
          style={{
            position: "relative",
            width: fw / dpr,
            height: fh / dpr,
            // Centered when narrower than the box (then nothing scrolls sideways).
            margin: "0 auto",
            cursor: "crosshair",
            // Streams are transparent: show it.
            background:
              summary.kind === "page"
                ? "white"
                : "repeating-conic-gradient(#ddd 0% 25%, #fff 0% 50%) 0 0 / 16px 16px",
            boxShadow: "0 0 3px rgba(0,0,0,0.3)",
          }}
        >
          {tiles.map((t) => (
            <Tile
              key={`${t.x},${t.y}`}
              content={content}
              scale={scale}
              until={until}
              tile={t}
              dpr={dpr}
              onNotes={onNotes}
            />
          ))}
          {showAp &&
            annots.some((a) => a.variants.some((x) => x.to_page)) &&
            tiles.map((t) => (
              <Tile
                key={`a${t.x},${t.y}`}
                content={content}
                scale={scale}
                until={null}
                tile={t}
                dpr={dpr}
                layer={annotsLayer}
                onNotes={onNotes}
              />
            ))}
          {hl?.tile && op !== null && <Highlight content={content} scale={scale} until={until} op={op} tile={hl.tile} dpr={dpr} />}
          {hl && (
            <div
              style={{
                position: "absolute",
                left: hl.box.x0 * z - 1,
                top: hl.box.y0 * z - 1,
                width: (hl.box.x1 - hl.box.x0) * z + 2,
                height: (hl.box.y1 - hl.box.y0) * z + 2,
                border: "1px dashed var(--mantine-color-red-filled)",
                pointerEvents: "none",
              }}
            />
          )}
          {(showOutlines || selAnnot) && (
            <AnnotOverlay
              annots={showOutlines ? annots : selAnnot ? [selAnnot] : []}
              view={v}
              zoom={z}
              selected={annot}
              picks={picks}
              onSelect={(i) => setAnnot(i === annot ? null : i)}
            />
          )}
        </div>
      </div>
      {selAnnot && (
        <AnnotCard
          a={selAnnot}
          total={annotsQ.data?.total ?? annots.length}
          contentOps={summary.ops}
          pick={picks[selAnnot.index] ?? null}
          onPick={(p) =>
            setPicks((old) => {
              const next = { ...old };
              if (p) next[selAnnot.index] = p;
              else delete next[selAnnot.index];
              return next;
            })
          }
          onClose={() => setAnnot(null)}
        />
      )}
      {hint && (
        <Text size="xs" c="dimmed" mt={4}>
          {hint}
        </Text>
      )}
      {notes && notes.length > 0 && (
        <Text size="xs" c="orange.8" mt={4}>
          Not exact yet: {notes.map((n) => n.message).join("; ")}.
        </Text>
      )}
    </div>
  );
}

const CONTENT_LAYER: RenderLayer = { kind: "content" };

function Tile({
  content,
  scale,
  until,
  tile,
  dpr,
  layer = CONTENT_LAYER,
  onNotes,
}: {
  content: ContentRef;
  scale: number;
  until: number | null;
  tile: PixelRect;
  dpr: number;
  layer?: RenderLayer;
  onNotes: (n: RenderNote[]) => void;
}) {
  const backend = useBackend();
  const nav = useNav();
  const canvas = useRef<HTMLCanvasElement>(null);
  const q = useQuery({
    queryKey: ["doc", nav.doc, "content", ...contentKey(content), "render", JSON.stringify(layer), scale, until, tile.x, tile.y, tile.width, tile.height],
    queryFn: ({ signal }) => api.render(backend, nav.doc, content, scale, tile, until, layer, signal),
    // Keep showing the previous bitmap while a new one (step, zoom) renders.
    placeholderData: (prev) => prev,
    gcTime: 60_000,
  });
  const b = q.data;
  useEffect(() => {
    if (!b) return;
    paint(canvas.current, b.data, b.info.rect.width, b.info.rect.height);
    onNotes(b.info.notes);
  }, [b, onNotes]);
  // A placeholder from another scale or tile is stretched over this one.
  return (
    <>
      <canvas
        ref={canvas}
        style={{
          position: "absolute",
          left: tile.x / dpr,
          top: tile.y / dpr,
          width: tile.width / dpr,
          height: tile.height / dpr,
          pointerEvents: "none",
        }}
      />
      {!b && q.isFetching && (
        <Loader size="xs" style={{ position: "absolute", left: tile.x / dpr + 8, top: tile.y / dpr + 8 }} />
      )}
      {q.error && (
        <Text size="xs" c="red" style={{ position: "absolute", left: tile.x / dpr + 8, top: tile.y / dpr + 8 }}>
          {String(q.error)}
        </Text>
      )}
    </>
  );
}

function Highlight({
  content,
  scale,
  until,
  op,
  tile,
  dpr,
}: {
  content: ContentRef;
  scale: number;
  until: number | null;
  op: number;
  tile: PixelRect;
  dpr: number;
}) {
  const backend = useBackend();
  const nav = useNav();
  const canvas = useRef<HTMLCanvasElement>(null);
  const q = useQuery({
    queryKey: ["doc", nav.doc, "content", ...contentKey(content), "highlight", op, scale, until, tile.x, tile.y, tile.width, tile.height],
    queryFn: ({ signal }) =>
      api.render(backend, nav.doc, content, scale, tile, until, { kind: "highlight", ops: [op] }, signal),
    gcTime: 10_000,
  });
  const b = q.data;
  useEffect(() => {
    if (b) paint(canvas.current, b.data, b.info.rect.width, b.info.rect.height);
  }, [b]);
  if (!b) return null;
  return (
    <canvas
      ref={canvas}
      className={classes.highlight}
      style={{
        position: "absolute",
        left: b.info.rect.x / dpr,
        top: b.info.rect.y / dpr,
        width: b.info.rect.width / dpr,
        height: b.info.rect.height / dpr,
      }}
    />
  );
}
