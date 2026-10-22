import { Alert, Anchor, Badge, Code, Group, Loader, Stack, Table, Text } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { api, useBackend } from "../api";
import { bytesAsText } from "../format";
import { contentKey, useNav } from "../nav";
import type { ColorView, ContentRef, ContentSummary, GraphicsState, ObjRef, Rect } from "../protocol/generated";
import { categoryColor } from "./ContentView";
import { severityColor } from "./DiagnosticsPanel";
import { ObjLink } from "./DocumentPanel";

/** Bytes of an operation's source shown. */
const MAX_SOURCE = 4096;

const n = (v: number) => (Number.isInteger(v) ? String(v) : String(Number(v.toFixed(4))));
const list = (v: number[]) => `[${v.map(n).join(" ")}]`;
const rect = (r: Rect | null) => (r ? `[${n(r.x0)} ${n(r.y0)} ${n(r.x1)} ${n(r.y1)}]` : "none");

/** An sRGB approximation of a color, when its space tells how. */
function swatch(c: ColorView): string | null {
  const v = c.components.map((x) => Math.round(Math.max(0, Math.min(1, x)) * 255));
  const by = c.family === "ICCBased" || c.family.startsWith("Device") || c.family.startsWith("Cal") ? v.length : 0;
  if (by === 1) return `rgb(${v[0]},${v[0]},${v[0]})`;
  if (by === 3 && c.family !== "Lab") return `rgb(${v[0]},${v[1]},${v[2]})`;
  if (by === 4) {
    const [cc, m, y, k] = c.components;
    const f = (x: number) => Math.round(255 * (1 - Math.min(1, x)) * (1 - Math.min(1, k)));
    return `rgb(${f(cc)},${f(m)},${f(y)})`;
  }
  return null;
}

function colorText(c: ColorView): string {
  const space = c.space === c.family ? c.space : `/${c.space} (${c.family})`;
  return `${space} ${list(c.components)}${c.pattern ? ` /${c.pattern}` : ""}`;
}

function Color({ c }: { c: ColorView }) {
  const s = swatch(c);
  return (
    <Group gap={4} wrap="nowrap">
      {s && (
        <span
          style={{
            width: 10,
            height: 10,
            flexShrink: 0,
            background: s,
            border: "1px solid var(--mantine-color-default-border)",
          }}
        />
      )}
      <span>{colorText(c)}</span>
    </Group>
  );
}

interface Field {
  label: string;
  text: (g: GraphicsState) => string;
  show?: (g: GraphicsState) => ReactNode;
  /** Only shown when set in either state. */
  optional?: (g: GraphicsState) => boolean;
}

const fields: Field[] = [
  { label: "CTM", text: (g) => list(g.ctm) },
  { label: "clip", text: (g) => rect(g.clip) },
  { label: "saved (q)", text: (g) => String(g.saved) },
  { label: "marked", text: (g) => g.marked.map((t) => `/${t}`).join(" › ") || "—" },
  { label: "fill", text: (g) => colorText(g.fill), show: (g) => <Color c={g.fill} /> },
  { label: "stroke", text: (g) => colorText(g.stroke), show: (g) => <Color c={g.stroke} /> },
  { label: "alpha", text: (g) => `fill ${n(g.fill_alpha)} · stroke ${n(g.stroke_alpha)}` },
  { label: "blend", text: (g) => g.blend + (g.soft_mask ? ` · soft mask: ${g.soft_mask}` : "") },
  { label: "gs", text: (g) => g.ext_gstates.map((x) => `/${x}`).join(" ") || "—", optional: (g) => g.ext_gstates.length > 0 },
  {
    label: "line",
    text: (g) =>
      `width ${n(g.line_width)} · cap ${g.line_cap} · join ${g.line_join} · miter ${n(g.miter_limit)}` +
      (g.dash.length ? ` · dash ${list(g.dash)} ${n(g.dash_phase)}` : ""),
  },
  { label: "intent", text: (g) => `${g.intent} · flatness ${n(g.flatness)}` },
  {
    label: "font",
    text: (g) => (g.font ? `/${g.font.name} ${n(g.font_size)}${g.font.base_font ? ` (${g.font.base_font})` : ""}` : "none"),
    show: (g) =>
      g.font ? (
        <Group gap={4} wrap="nowrap">
          <span>
            /{g.font.name} {n(g.font_size)}
            {g.font.base_font && ` (${g.font.base_font}${g.font.subtype ? `, ${g.font.subtype}` : ""})`}
          </span>
          {g.font.id && <ObjLink id={g.font.id} />}
          {g.font.id && <FontAnchor id={g.font.id} />}
        </Group>
      ) : (
        "none"
      ),
  },
  {
    label: "text state",
    text: (g) =>
      `Tc ${n(g.char_spacing)} · Tw ${n(g.word_spacing)} · Tz ${n(g.horizontal_scale)} · TL ${n(g.leading)} · Tr ${g.render_mode} · Ts ${n(g.rise)}`,
  },
  { label: "Tm", text: (g) => (g.text_matrix ? list(g.text_matrix) : "—"), optional: (g) => g.in_text },
  { label: "Tlm", text: (g) => (g.line_matrix ? list(g.line_matrix) : "—"), optional: (g) => g.in_text },
  { label: "path", text: (g) => rect(g.path), optional: (g) => g.path !== null },
  {
    label: "current point",
    text: (g) => (g.current_point ? list(g.current_point) : "—"),
    optional: (g) => g.current_point !== null,
  },
];

/** One operation: what it is, its source bytes, and the state before and after it. */
export function OpStatePanel({ content, summary, index }: { content: ContentRef; summary: ContentSummary; index: number }) {
  const backend = useBackend();
  const nav = useNav();
  const key = ["doc", nav.doc, "content", ...contentKey(content)];
  const st = useQuery({ queryKey: [...key, "state", index], queryFn: () => api.opState(backend, nav.doc, content, index) });
  const ov = useQuery({
    queryKey: [...key, "op", index],
    queryFn: async () => (await api.contentOps(backend, nav.doc, content, { offset: index, limit: 1 })).items[0] ?? null,
  });
  const o = ov.data;
  const source = o?.source !== null && o?.source !== undefined ? summary.sources[o.source] : undefined;
  const span = o?.span;
  const src = useQuery({
    queryKey: [...key, "source", index],
    queryFn: () => api.streamData(backend, nav.doc, source!.id!, true, span!.start, Math.min(span!.end - span!.start, MAX_SOURCE)),
    enabled: !!source?.id && !!span,
  });

  if (st.error) {
    return (
      <Alert m="sm" color="red" p={6}>
        <Text size="xs">{String(st.error)}</Text>
      </Alert>
    );
  }
  if (!st.data || ov.isPending) return <Loader m="sm" size="xs" />;
  const s = st.data;
  const shownFields = fields.filter((f) => !f.optional || f.optional(s.before) || f.optional(s.after));
  return (
    <Stack p="sm" gap={6}>
      <Group gap={6}>
        <Text size="sm" fw={600} ff="monospace" style={{ color: `var(--mantine-color-${categoryColor[o?.category ?? "unknown"]}-text)` }}>
          {s.operator}
        </Text>
        <Text size="xs" c="dimmed">
          #{index} · {s.description}
        </Text>
      </Group>
      {o && o.issues.length > 0 && (
        <Stack gap={2}>
          {o.issues.map((i, k) => (
            <Group key={k} gap={6} wrap="nowrap" align="flex-start">
              <Badge size="xs" color={severityColor[i.severity]} variant="light" style={{ flexShrink: 0 }}>
                {i.code}
              </Badge>
              <Text size="xs">{i.message}</Text>
            </Group>
          ))}
        </Stack>
      )}
      {o?.text !== null && o?.text !== undefined && (
        <Text size="xs">
          Text: <Text span c="orange.8">“{o.text}”</Text>
        </Text>
      )}
      {o?.resource && (
        <Group gap={6}>
          <Text size="xs">Resource:</Text>
          <ObjLink id={o.resource} />
          {o.operator === "Do" && o.element?.kind === "form" && (
            <Anchor size="xs" onClick={() => nav.open({ kind: "content", content: { kind: "stream", id: o.resource! }, op: null })}>
              its operations →
            </Anchor>
          )}
        </Group>
      )}
      {o?.element && (
        <Text size="xs">
          Paints: {o.element.kind} {rect(o.element.bbox)}
          {o.element.hidden && (
            <Text span size="xs" c="dimmed">
              {" "}
              (hidden: outside the clip, or invisible text)
            </Text>
          )}
        </Text>
      )}
      {src.data && (
        <Code block style={{ whiteSpace: "pre-wrap", wordBreak: "break-all", maxHeight: 120, overflow: "auto", fontSize: 11 }}>
          {bytesAsText(src.data.data)}
          {span && span.end - span.start > MAX_SOURCE && " …"}
        </Code>
      )}
      {source && span && (
        <Text size="xs" c="dimmed">
          {summary.sources.length > 1 ? `Stream [${o!.source}] ` : "Stream "}
          {source.id && <ObjLink id={source.id} />} bytes {span.start.toLocaleString()}–{span.end.toLocaleString()} (decoded)
        </Text>
      )}
      <Table withRowBorders={false} verticalSpacing={1} horizontalSpacing={4} fz="xs" layout="fixed">
        <Table.Thead>
          <Table.Tr>
            <Table.Th w={80} />
            <Table.Th>Before</Table.Th>
            <Table.Th>After</Table.Th>
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {shownFields.map((f) => {
            const changed = f.text(s.before) !== f.text(s.after);
            return (
              <Table.Tr key={f.label} style={changed ? { background: "var(--mantine-color-yellow-light)" } : undefined}>
                <Table.Td c="dimmed" style={{ verticalAlign: "top" }}>
                  {f.label}
                </Table.Td>
                <Table.Td ff="monospace" style={{ wordBreak: "break-word", verticalAlign: "top" }}>
                  {f.show ? f.show(s.before) : f.text(s.before)}
                </Table.Td>
                <Table.Td ff="monospace" style={{ wordBreak: "break-word", verticalAlign: "top" }} c={changed ? undefined : "dimmed"}>
                  {changed ? (f.show ? f.show(s.after) : f.text(s.after)) : "="}
                </Table.Td>
              </Table.Tr>
            );
          })}
        </Table.Tbody>
      </Table>
    </Stack>
  );
}

/** Open the font view (program, encoding, glyphs). */
function FontAnchor({ id }: { id: ObjRef }) {
  const nav = useNav();
  return (
    <Anchor size="xs" onClick={() => nav.open({ kind: "font", id })}>
      glyphs
    </Anchor>
  );
}
