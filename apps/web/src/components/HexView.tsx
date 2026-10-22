import { Group, Text, TextInput } from "@mantine/core";
import { useQueries, useQuery } from "@tanstack/react-query";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { api, useBackend } from "../api";
import { asciiByte, hexByte, spanText } from "../format";
import { useNav } from "../nav";
import type { Span } from "../protocol/generated";
import { useTargetObject } from "./Breadcrumbs";
import classes from "./Hex.module.css";

const ROW = 16;
const LINE_H = 18;
const CHUNK = 64 * 1024;
/** Browsers cap element heights (~33M px in Chromium); scale the scrollbar beyond this. */
const MAX_SCROLL = 8_000_000;

/**
 * Bytes of the object the current view shows ("N G obj … endobj"); for an
 * object inside an object stream, those of the stream.
 */
function useTargetSpan(): { key: string; span: Span } | null {
  const backend = useBackend();
  const nav = useNav();
  const { id } = useTargetObject();
  const obj = useQuery({
    queryKey: ["doc", nav.doc, "object", id?.num, id?.generation],
    queryFn: () => api.object(backend, nav.doc, id!),
    enabled: id !== null,
  }).data;
  const stream = obj?.location.kind === "objStm" ? obj.location.stream : null;
  const container = useQuery({
    queryKey: ["doc", nav.doc, "object", stream?.num, stream?.generation],
    queryFn: () => api.object(backend, nav.doc, stream!),
    enabled: stream !== null,
  }).data;
  const loc = stream ? container?.location : obj?.location;
  if (!id || loc?.kind !== "file") return null;
  return { key: `${id.num}/${id.generation}`, span: loc.span };
}

/**
 * Hex view over the whole file. Only the visible rows are fetched (64 KiB
 * chunks), and the scrollbar is scaled so files of any size fit.
 */
export function HexView({ size }: { size: number }) {
  const backend = useBackend();
  const nav = useNav();
  const scroller = useRef<HTMLDivElement>(null);
  const [viewH, setViewH] = useState(200);
  const [top, setTop] = useState(0);
  const [goto, setGoto] = useState("");

  const totalRows = Math.max(1, Math.ceil(size / ROW));
  const visible = Math.max(1, Math.floor((viewH - 4) / LINE_H));
  const maxTop = Math.max(0, totalRows - visible);
  const spacer = Math.min(totalRows * LINE_H, MAX_SCROLL);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setViewH(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const onScroll = () => {
    const el = scroller.current!;
    const range = Math.max(1, spacer - el.clientHeight);
    setTop(Math.round((el.scrollTop / range) * maxTop));
  };

  const scrollToRow = useCallback(
    (row: number) => {
      const el = scroller.current;
      if (!el) return;
      const t = Math.max(0, Math.min(maxTop, row));
      el.scrollTop = maxTop ? (t / maxTop) * Math.max(0, spacer - el.clientHeight) : 0;
      setTop(t);
    },
    [maxTop, spacer],
  );

  useEffect(() => {
    if (nav.hexSpan) scrollToRow(Math.floor(nav.hexSpan.start / ROW) - 2);
  }, [nav.hexSpan, scrollToRow]);

  // Select the object shown, once per object (later picks in the view win).
  const target = useTargetSpan();
  const followed = useRef<string | null>(null);
  const { showBytes } = nav;
  useEffect(() => {
    if (!target || followed.current === target.key) return;
    followed.current = target.key;
    showBytes(target.span);
  }, [target?.key, target?.span.start, target?.span.end, showBytes]);

  const first = top * ROW;
  const last = Math.min(size, (top + visible) * ROW);
  const chunks: number[] = [];
  for (let c = Math.floor(first / CHUNK); c * CHUNK < Math.max(last, first + 1); c++) chunks.push(c);
  const results = useQueries({
    queries: chunks.map((c) => ({
      queryKey: ["doc", nav.doc, "raw", c],
      queryFn: ({ signal }: { signal: AbortSignal }) =>
        api.rawBytes(backend, nav.doc, c * CHUNK, CHUNK, signal).then((b) => b.data),
    })),
  });
  const byte = (off: number): number | undefined => results[Math.floor(off / CHUNK) - chunks[0]]?.data?.[off % CHUNK];
  const hl = nav.hexSpan;

  const rows = [];
  for (let r = top; r < Math.min(totalRows, top + visible); r++) {
    const base = r * ROW;
    const hex = [];
    let ascii = "";
    for (let i = 0; i < ROW && base + i < size; i++) {
      const b = byte(base + i);
      const on = hl && base + i >= hl.start && base + i < hl.end;
      hex.push(
        <span key={i} className={classes.byte} data-hl={on || undefined}>
          {b === undefined ? "··" : hexByte(b)}
          {i < ROW - 1 ? " " : ""}
        </span>,
      );
      ascii += b === undefined ? " " : asciiByte(b);
    }
    rows.push(
      <div key={r} className={classes.line} style={{ position: "relative" }}>
        <span className={classes.offset}>{base.toString(16).padStart(10, "0")}</span>
        <span className={classes.hex}>{hex}</span>
        <span className={classes.ascii}>{ascii}</span>
      </div>,
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <Group px="sm" py={2} gap="sm" style={{ borderBottom: "1px solid var(--mantine-color-default-border)" }}>
        <Text size="xs" fw={600}>
          File bytes
        </Text>
        <TextInput
          size="xs"
          w={180}
          placeholder="Go to offset (dec or 0x…)"
          value={goto}
          onChange={(e) => setGoto(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key !== "Enter") return;
            const n = Number(goto.trim());
            if (Number.isFinite(n) && n >= 0) nav.showBytes({ start: n, end: n + 1 });
          }}
        />
        {hl && (
          <Text size="xs" c="dimmed">
            selection {spanText(hl)}
          </Text>
        )}
      </Group>
      <div ref={scroller} onScroll={onScroll} style={{ flex: 1, minHeight: 0, overflowY: "scroll", position: "relative" }}>
        <div style={{ height: spacer }}>
          <div className={classes.dump} style={{ position: "sticky", top: 0, height: viewH, overflow: "hidden" }}>
            {rows}
          </div>
        </div>
      </div>
    </div>
  );
}
