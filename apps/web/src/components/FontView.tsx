import { Alert, Anchor, Badge, Button, Group, Loader, Stack, Table, Text, Title, Tooltip, UnstyledButton } from "@mantine/core";
import { useQueries, useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { api, useBackend } from "../api";
import { bytesText, objText } from "../format";
import { useNav } from "../nav";
import type { FontDetail, GlyphSourceKind, GlyphView, ObjRef } from "../protocol/generated";
import { ObjLink } from "./DocumentPanel";
import { IssueList } from "./Issues";
import classes from "./FontView.module.css";

/** Glyphs fetched per request. */
const PAGE = 128;
const CELL_W = 76;
const CELL_H = 92;

const sourceText: Record<GlyphSourceKind, string> = {
  embedded: "embedded",
  substitute: "substituted",
  procedures: "glyph procedures",
  none: "no glyphs",
};

const sourceColor: Record<GlyphSourceKind, string> = {
  embedded: "teal",
  substitute: "blue",
  procedures: "grape",
  none: "orange",
};

/** A font: its dictionaries, encoding, program, problems and the glyph table. */
export function FontView({ id }: { id: ObjRef }) {
  const backend = useBackend();
  const nav = useNav();
  const q = useQuery({
    queryKey: ["doc", nav.doc, "font", id.num, id.generation],
    queryFn: () => api.font(backend, nav.doc, id),
  });
  if (q.error) {
    return (
      <Alert m="sm" color="red" title={objText(id)}>
        {String(q.error)}
      </Alert>
    );
  }
  if (!q.data) return <Loader m="md" size="sm" />;
  const f = q.data;
  return (
    <Stack p="sm" gap="sm">
      <Group gap="sm">
        <Title order={5}>{f.base_font ?? "(no /BaseFont)"}</Title>
        <Badge variant="light" color="gray" tt="none">
          {f.subtype ?? "?"}
          {f.cid_subtype && ` · ${f.cid_subtype}`}
        </Badge>
        <Badge variant="light" color={sourceColor[f.glyph_source]} tt="none">
          {sourceText[f.glyph_source]}
          {f.substitute && `: ${f.substitute}`}
        </Badge>
        <Button size="compact-xs" variant="light" onClick={() => nav.open({ kind: "object", id: f.id })}>
          {objText(f.id)}
        </Button>
      </Group>
      {f.issues.length > 0 && (
        <Alert p={6} variant="light" color={f.issues.some((i) => i.severity === "error") ? "red" : f.issues.some((i) => i.severity === "warning") ? "orange" : "blue"}>
          <IssueList issues={f.issues} />
        </Alert>
      )}
      <Properties f={f} />
      <Glyphs f={f} />
    </Stack>
  );
}

function Properties({ f }: { f: FontDetail }) {
  const rows: [string, ReactNode][] = [];
  if (f.descendant) rows.push(["descendant", <ObjLink id={f.descendant} />]);
  rows.push(["encoding", `${f.encoding ?? "none (built-in)"}${f.vertical ? " · vertical" : ""}`]);
  if (f.cid_to_gid) rows.push(["CIDToGIDMap", f.cid_to_gid]);
  rows.push([
    "ToUnicode",
    f.to_unicode ? (
      <Group gap={6}>
        <ObjLink id={f.to_unicode} />
        <Text size="xs" c="dimmed">
          {f.to_unicode_entries.toLocaleString()} entries
        </Text>
      </Group>
    ) : (
      "none"
    ),
  ]);
  if (f.program) {
    const p = f.program;
    rows.push([
      "program",
      <Group gap={6}>
        <Text size="xs">
          /{p.key}
          {p.subtype && ` /${p.subtype}`}
        </Text>
        {p.stream && <ObjLink id={p.stream} />}
        <Text size="xs" c={p.format ? "dimmed" : "red"}>
          {p.format
            ? `${p.format} · ${bytesText(p.len)}${p.glyphs !== null ? ` · ${p.glyphs.toLocaleString()} glyphs` : ""}${p.units_per_em !== null ? ` · ${p.units_per_em} units/em` : ""}${p.cid_keyed ? " · CID-keyed" : ""}`
            : "unreadable"}
        </Text>
      </Group>,
    ]);
  } else if (f.kind !== "type3") {
    rows.push(["program", <Text size="xs" c="orange">not embedded</Text>]);
  }
  if (f.descriptor) rows.push(["descriptor", <ObjLink id={f.descriptor} />]);
  if (f.flags !== null) rows.push(["flags", `${f.flags}${f.flag_names.length ? ` (${f.flag_names.join(", ")})` : ""}`]);
  rows.push(["ascent / descent", `${round(f.ascent)} / ${round(f.descent)}`]);
  if (f.font_matrix) rows.push(["FontMatrix", `[${f.font_matrix.join(" ")}]`]);
  if (f.approximate) rows.push(["widths", <Text size="xs" c="orange">partly guessed</Text>]);
  return (
    <Table withRowBorders={false} verticalSpacing={2} fz="xs" style={{ width: "auto" }}>
      <Table.Tbody>
        {rows.map(([k, v]) => (
          <Table.Tr key={k}>
            <Table.Td c="dimmed" style={{ whiteSpace: "nowrap", verticalAlign: "top" }}>
              {k}
            </Table.Td>
            <Table.Td>{v}</Table.Td>
          </Table.Tr>
        ))}
      </Table.Tbody>
    </Table>
  );
}

const round = (v: number) => Math.round(v * 100) / 100;

/** The glyph in a box from the font's descent to its ascent, with the baseline and advance marked. */
function GlyphShape({ f, g, size }: { f: FontDetail; g: GlyphView; size: number }) {
  const top = Math.max(f.ascent || 800, g.bbox?.y1 ?? 0);
  const bottom = Math.min(f.descent || -200, g.bbox?.y0 ?? 0);
  const x0 = Math.min(0, g.bbox?.x0 ?? 0);
  const x1 = Math.max(g.width, g.bbox?.x1 ?? 0, 1);
  const pad = (top - bottom) * 0.06;
  const vb = `${x0 - pad} ${-top - pad} ${x1 - x0 + 2 * pad} ${top - bottom + 2 * pad}`;
  const thin = (top - bottom) / size;
  return (
    <svg viewBox={vb} width={size} height={size} preserveAspectRatio="xMidYMid meet" style={{ display: "block" }}>
      <line x1={x0 - pad} x2={x1 + pad} y1={0} y2={0} stroke="var(--mantine-color-blue-4)" strokeWidth={thin} />
      {g.width > 0 && (
        <line x1={g.width} x2={g.width} y1={-top} y2={-bottom} stroke="var(--mantine-color-gray-5)" strokeWidth={thin} strokeDasharray={`${4 * thin} ${3 * thin}`} />
      )}
      {g.outline && <path d={g.outline} transform="scale(1,-1)" fill="var(--mantine-color-text)" />}
    </svg>
  );
}

/** Code → glyph table: virtualized rows of cells, fetched by pages as they scroll into view. */
function Glyphs({ f }: { f: FontDetail }) {
  const backend = useBackend();
  const nav = useNav();
  const parent = useRef<HTMLDivElement>(null);
  const [cols, setCols] = useState(8);
  const [picked, setPicked] = useState<GlyphView | null>(null);
  useEffect(() => {
    const el = parent.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setCols(Math.max(1, Math.floor((el.clientWidth - 2) / CELL_W))));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const total = f.codes;
  const rows = Math.ceil(total / cols);
  const v = useVirtualizer({ count: rows, getScrollElement: () => parent.current, estimateSize: () => CELL_H, overscan: 4 });
  const items = v.getVirtualItems();
  const pages = [
    ...new Set(items.flatMap((r) => [Math.floor((r.index * cols) / PAGE), Math.floor(Math.min(total - 1, r.index * cols + cols - 1) / PAGE)])),
  ];
  const results = useQueries({
    queries: pages.map((p) => ({
      queryKey: ["doc", nav.doc, "glyphs", f.id.num, f.id.generation, PAGE, p],
      queryFn: ({ signal }: { signal: AbortSignal }) => api.glyphs(backend, nav.doc, f.id, { offset: p * PAGE, limit: PAGE }, signal),
    })),
  });
  const byPage = new Map(pages.map((p, k) => [p, results[k]?.data]));
  const glyphAt = (i: number) => {
    const page = byPage.get(Math.floor(i / PAGE));
    return page?.items[i - page.offset];
  };

  if (total === 0) {
    return (
      <Text size="xs" c="dimmed">
        No glyphs to list.
      </Text>
    );
  }
  return (
    <Stack gap={6}>
      <Group gap="sm">
        <Title order={6}>Glyphs</Title>
        <Text size="xs" c="dimmed">
          {total.toLocaleString()} codes · code → glyph → text
        </Text>
      </Group>
      {picked && <Picked f={f} g={picked} />}
      <div ref={parent} className={classes.grid} style={{ height: Math.min(rows * CELL_H + 2, 560) }}>
        <div style={{ height: v.getTotalSize(), position: "relative" }}>
          {items.map((r) => (
            <div key={r.key} style={{ position: "absolute", top: 0, left: 0, height: CELL_H, transform: `translateY(${r.start}px)`, display: "flex" }}>
              {Array.from({ length: Math.min(cols, total - r.index * cols) }, (_, c) => {
                const i = r.index * cols + c;
                const g = glyphAt(i);
                return (
                  <div key={c} style={{ width: CELL_W, height: CELL_H }}>
                    {g ? <Cell f={f} g={g} active={picked?.code === g.code && picked.bytes === g.bytes} onPick={() => setPicked(g)} /> : <Loader size={10} m={8} />}
                  </div>
                );
              })}
            </div>
          ))}
        </div>
      </div>
    </Stack>
  );
}

function Cell({ f, g, active, onPick }: { f: FontDetail; g: GlyphView; active: boolean; onPick: () => void }) {
  const empty = !g.outline && !g.procedure;
  return (
    <UnstyledButton className={classes.cell} data-active={active || undefined} data-empty={empty || undefined} onClick={onPick}>
      <Text size="10px" ff="monospace" c="dimmed" lh={1.2}>
        {g.bytes}
      </Text>
      <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center", minHeight: 0 }}>
        {g.outline ? <GlyphShape f={f} g={g} size={44} /> : g.procedure ? <Text size="10px" c="grape">proc</Text> : null}
      </div>
      <Text size="10px" lh={1.2} truncate="end" w="100%" ta="center" title={g.name ?? undefined}>
        {g.text ? JSON.stringify(g.text).slice(1, -1) : (g.name ?? "")}
      </Text>
    </UnstyledButton>
  );
}

/** The picked glyph, large, with everything known about it. */
function Picked({ f, g }: { f: FontDetail; g: GlyphView }) {
  const nav = useNav();
  const field = (k: string, v: ReactNode) => (
    <Group gap={4} wrap="nowrap">
      <Text size="xs" c="dimmed">
        {k}
      </Text>
      <Text size="xs" ff="monospace">
        {v}
      </Text>
    </Group>
  );
  const codepoints = g.text ? [...g.text].map((c) => `U+${c.codePointAt(0)!.toString(16).toUpperCase().padStart(4, "0")}`).join(" ") : null;
  return (
    <Group gap="md" align="flex-start" className={classes.picked}>
      <div className={classes.big}>{g.outline ? <GlyphShape f={f} g={g} size={120} /> : <Text size="xs" c="dimmed">no outline</Text>}</div>
      <Stack gap={2}>
        {field("code", `<${g.bytes}>`)}
        {g.cid !== null && field("CID", g.cid)}
        {g.gid !== null && field("glyph", g.gid)}
        {g.name !== null && field("name", `/${g.name}`)}
        {field("text", g.text ? `${JSON.stringify(g.text)} ${codepoints}` : "unknown")}
        {field("width", round(g.width))}
        {g.bbox && field("bbox", `[${round(g.bbox.x0)} ${round(g.bbox.y0)} ${round(g.bbox.x1)} ${round(g.bbox.y1)}]`)}
        {g.procedure && (
          <Tooltip label="The glyph procedure's operations">
            <Anchor size="xs" onClick={() => nav.open({ kind: "content", content: { kind: "stream", id: g.procedure! }, op: null })}>
              procedure {objText(g.procedure)} →
            </Anchor>
          </Tooltip>
        )}
      </Stack>
    </Group>
  );
}
