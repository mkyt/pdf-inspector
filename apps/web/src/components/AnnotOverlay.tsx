import { Anchor, Badge, Button, CloseButton, Group, Paper, Stack, Table, Text } from "@mantine/core";
import type { ReactNode } from "react";
import { rectText } from "../format";
import { useNav } from "../nav";
import type { AnnotInfo, AppearanceInfo, AppearancePick, Rect, ViewInfo } from "../protocol/generated";
import { ObjLink } from "./DocumentPanel";
import { LinkView } from "./LinkView";

/** Outline colors by subtype, for annotations without `/C`. */
const SUBTYPE_COLOR: Record<string, string> = {
  Link: "#1c7ed6",
  Widget: "#7048e8",
  Popup: "#868e96",
  Text: "#f08c00",
  FreeText: "#0c8599",
};
const DEFAULT_COLOR = "#e8590c";

/** `/C` as CSS (gray, RGB or naive CMYK), or null when absent or transparent. */
export function annotColor(c: number[] | null): string | null {
  if (!c || c.length === 0) return null;
  const v = (x: number) => Math.round(Math.min(1, Math.max(0, x)) * 255);
  if (c.length === 1) return `rgb(${v(c[0])} ${v(c[0])} ${v(c[0])})`;
  if (c.length === 3) return `rgb(${v(c[0])} ${v(c[1])} ${v(c[2])})`;
  if (c.length === 4) {
    const k = 1 - c[3];
    return `rgb(${v((1 - c[0]) * k)} ${v((1 - c[1]) * k)} ${v((1 - c[2]) * k)})`;
  }
  return null;
}

/** The outline color of an annotation. */
function stroke(a: AnnotInfo): string {
  if (!a.shown) return "#adb5bd";
  return annotColor(a.color) ?? SUBTYPE_COLOR[a.subtype ?? ""] ?? DEFAULT_COLOR;
}

/** Label text color over `stroke(a)`: black over light colors (a yellow highlight). */
function ink(a: AnnotInfo): string {
  const c = a.shown ? a.color : null;
  if (!c || (c.length !== 1 && c.length !== 3)) return "white";
  const [r, g, b] = c.length === 1 ? [c[0], c[0], c[0]] : c;
  return 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.6 ? "black" : "white";
}

/** The appearance painted for an annotation: the one picked, else the normal one. */
export function shownAppearance(a: AnnotInfo, pick: AppearancePick | null): AppearanceInfo | null {
  if (!pick) return a.appearance;
  return a.variants.find((v) => v.kind === pick.kind && v.state === pick.state) ?? null;
}

const variantText = (v: { kind: string; state: string | null }) => `/${v.kind}${v.state ? ` /${v.state}` : ""}`;

const matrixAttr = (m: number[]) => `matrix(${m.join(" ")})`;

/** Points of a quadrilateral in drawing order (the spec lists them 1 2 3 4 as two edges). */
function quadPoints(q: number[]) {
  return [0, 1, 3, 2].map((i) => `${q[2 * i]},${q[2 * i + 1]}`).join(" ");
}

function pairs(p: number[]) {
  const out: string[] = [];
  for (let i = 0; i + 1 < p.length; i += 2) out.push(`${p[i]},${p[i + 1]}`);
  return out.join(" ");
}

/** A rectangle of the content's space in view points. */
export function toView(r: Rect, v: ViewInfo) {
  const [a, b, c, d, e, f] = v.matrix;
  const xs: number[] = [];
  const ys: number[] = [];
  for (const [x, y] of [
    [r.x0, r.y0],
    [r.x1, r.y0],
    [r.x0, r.y1],
    [r.x1, r.y1],
  ]) {
    xs.push(a * x + c * y + e);
    ys.push(b * x + d * y + f);
  }
  return { x0: Math.min(...xs), y0: Math.min(...ys), x1: Math.max(...xs), y1: Math.max(...ys) };
}

/** Annotations whose `/Rect` holds a point of the page's default space, topmost (last painted) first. */
export function annotsAt(annots: AnnotInfo[], x: number, y: number): AnnotInfo[] {
  return annots.filter((a) => a.rect && x >= a.rect.x0 && x <= a.rect.x1 && y >= a.rect.y0 && y <= a.rect.y1).reverse();
}

/**
 * Where a page's annotations sit over its content: `/Rect` (dashed),
 * `/QuadPoints` (filled), ink / vertex / line paths, and for the selected
 * one its appearance's `/BBox` as put on the page (dotted, with the form's
 * origin and x axis), so a `/Matrix` that turns or scales it shows. Labels
 * (`[index] Subtype`, in painting order) select an annotation.
 */
export function AnnotOverlay({
  annots,
  view,
  zoom,
  selected,
  picks,
  onSelect,
}: {
  annots: AnnotInfo[];
  /** Appearances painted instead of the normal ones. */
  picks: Record<number, AppearancePick>;
  view: ViewInfo;
  /** CSS pixels per point. */
  zoom: number;
  selected: number | null;
  onSelect: (index: number) => void;
}) {
  const sel = annots.find((a) => a.index === selected) ?? null;
  const ap = sel ? shownAppearance(sel, picks[sel.index] ?? null) : null;
  return (
    <>
      <svg
        width={view.width * zoom}
        height={view.height * zoom}
        viewBox={`0 0 ${view.width} ${view.height}`}
        preserveAspectRatio="none"
        style={{ position: "absolute", left: 0, top: 0, pointerEvents: "none", overflow: "visible" }}
      >
        <g transform={matrixAttr(view.matrix)}>
          {annots.map((a) => {
            const c = stroke(a);
            const on = a.index === selected;
            const w = on ? 2 : 1;
            return (
              <g key={a.index} opacity={selected === null || on ? 1 : 0.45}>
                {a.quads.map((q, i) => (
                  <polygon key={i} points={quadPoints(q)} fill={c} fillOpacity={0.18} stroke={c} strokeWidth={0.5} vectorEffect="non-scaling-stroke" />
                ))}
                {a.paths.map((p, i) => {
                  const P = a.subtype === "Polygon" ? "polygon" : "polyline";
                  return <P key={i} points={pairs(p)} fill="none" stroke={c} strokeWidth={w} vectorEffect="non-scaling-stroke" />;
                })}
                {a.rect && (
                  <rect
                    x={a.rect.x0}
                    y={a.rect.y0}
                    width={a.rect.x1 - a.rect.x0}
                    height={a.rect.y1 - a.rect.y0}
                    fill={on ? c : "none"}
                    fillOpacity={0.08}
                    stroke={c}
                    strokeWidth={w}
                    strokeDasharray={a.shown ? "4 3" : "1 3"}
                    vectorEffect="non-scaling-stroke"
                  />
                )}
              </g>
            );
          })}
          {ap?.bbox && ap.to_page && (
            <g transform={matrixAttr(ap.to_page)}>
              <rect
                x={ap.bbox.x0}
                y={ap.bbox.y0}
                width={ap.bbox.x1 - ap.bbox.x0}
                height={ap.bbox.y1 - ap.bbox.y0}
                fill="none"
                stroke="var(--mantine-color-red-filled)"
                strokeWidth={1}
                strokeDasharray="1 2"
                vectorEffect="non-scaling-stroke"
              />
              {/* The form's origin corner and x axis (a third of the box's width). */}
              <line
                x1={ap.bbox.x0}
                y1={ap.bbox.y0}
                x2={ap.bbox.x0 + (ap.bbox.x1 - ap.bbox.x0) / 3}
                y2={ap.bbox.y0}
                stroke="var(--mantine-color-red-filled)"
                strokeWidth={3}
                vectorEffect="non-scaling-stroke"
              />
              <line
                x1={ap.bbox.x0}
                y1={ap.bbox.y0}
                x2={ap.bbox.x0}
                y2={ap.bbox.y0 + (ap.bbox.y1 - ap.bbox.y0) / 3}
                stroke="var(--mantine-color-red-filled)"
                strokeWidth={1.5}
                vectorEffect="non-scaling-stroke"
              />
            </g>
          )}
        </g>
      </svg>
      {annots.map((a) => {
        if (!a.rect) return null;
        const r = toView(a.rect, view);
        const on = a.index === selected;
        return (
          <button
            key={a.index}
            type="button"
            title={`[${a.index}] ${a.subtype ?? "?"}${a.shown ? "" : " (not shown)"}${a.contents ? `: ${a.contents}` : ""}`}
            onClick={(e) => {
              e.stopPropagation();
              onSelect(a.index);
            }}
            style={{
              position: "absolute",
              left: r.x0 * zoom,
              top: Math.max(0, r.y0 * zoom - 15),
              height: 15,
              padding: "0 4px",
              font: "10px/15px var(--mantine-font-family-monospace)",
              color: ink(a),
              background: stroke(a),
              opacity: selected === null || on ? 0.9 : 0.5,
              border: on ? "1px solid black" : "none",
              borderRadius: 2,
              cursor: "pointer",
              whiteSpace: "nowrap",
              zIndex: on ? 2 : 1,
            }}
          >
            {a.index} {a.subtype ?? "?"}
          </button>
        );
      })}
    </>
  );
}

const num = (x: number) => String(Math.round(x * 1000) / 1000);
const matrixText = (m: number[]) => `[${m.map(num).join(" ")}]`;

function Row({ k, children }: { k: string; children: ReactNode }) {
  return (
    <Table.Tr>
      <Table.Td c="dimmed" w={110} style={{ verticalAlign: "top" }}>
        {k}
      </Table.Td>
      <Table.Td style={{ wordBreak: "break-word" }}>{children}</Table.Td>
    </Table.Tr>
  );
}

/** The selected annotation: what it is, and how its appearance is put over the content. */
export function AnnotCard({
  a,
  total,
  contentOps,
  pick,
  onPick,
  onClose,
}: {
  a: AnnotInfo;
  /** Annotations on the page. */
  total: number;
  /** Operations of the page's content (painted before every annotation). */
  contentOps: number;
  /** The appearance painted instead of the normal one. */
  pick: AppearancePick | null;
  onPick: (p: AppearancePick | null) => void;
  onClose: () => void;
}) {
  const nav = useNav();
  const ap = shownAppearance(a, pick);
  const normal = a.appearance;
  const isNormal = (v: AppearanceInfo) => !!normal && !normal.error && v.kind === normal.kind && v.state === normal.state;
  const color = annotColor(a.color);
  const painted = a.shown && ap && !ap.error && ap.id && ap.to_page;
  return (
    <Paper withBorder p={6} mt={6}>
      <Stack gap={4}>
        <Group gap={6}>
          <Badge size="sm" variant="light" tt="none">
            [{a.index}] {a.subtype ?? "?"}
          </Badge>
          {a.id ? <ObjLink id={a.id} /> : <Text size="xs" c="dimmed">direct</Text>}
          {a.flag_names.map((f) => (
            <Badge key={f} size="xs" variant="outline" color="gray" tt="none">
              {f}
            </Badge>
          ))}
          <CloseButton size="xs" ml="auto" onClick={onClose} aria-label="Close" />
        </Group>
        {a.variants.length > 1 || (a.variants.length === 1 && !!normal?.error) ? (
          <Group gap={4}>
            <Text size="xs" c="dimmed">
              Appearance:
            </Text>
            {a.variants.map((v) => {
              const on = pick ? v.kind === pick.kind && v.state === pick.state : isNormal(v);
              return (
                <Button
                  key={variantText(v)}
                  size="compact-xs"
                  variant={on ? "filled" : "light"}
                  color={v.error ? "orange" : undefined}
                  title={v.error ?? (isNormal(v) ? "What a viewer shows (chosen by /AS)" : undefined)}
                  onClick={() => onPick(isNormal(v) ? null : { index: a.index, kind: v.kind, state: v.state })}
                >
                  {variantText(v)}
                  {isNormal(v) ? " ·AS" : ""}
                </Button>
              );
            })}
            {pick && (
              <Text size="xs" c="orange.8">
                (not what a viewer shows at rest)
              </Text>
            )}
          </Group>
        ) : null}
        <Text size="xs" c="dimmed">
          {painted
            ? `Painted over all ${contentOps.toLocaleString()} content operations${a.index > 0 ? ` and annotations [0]–[${a.index - 1}]` : ""}; ${total - a.index - 1} annotation${total - a.index - 1 === 1 ? "" : "s"} above it.`
            : !a.shown
              ? "Not shown on screen (Hidden / NoView, or Invisible without an appearance)."
              : ap?.error
                ? `Not painted: ${ap.error}.`
                : "Not painted: no normal appearance stream (/AP /N); a viewer may make one up."}
        </Text>
        <Table withRowBorders={false} verticalSpacing={1} horizontalSpacing={6} fz="xs">
          <Table.Tbody>
            {a.error && (
              <Row k="Error">
                <Text span size="xs" c="red">
                  {a.error}
                </Text>
              </Row>
            )}
            <Row k="/Rect">
              <Text span size="xs" ff="monospace">
                {rectText(a.rect)}
              </Text>
            </Row>
            {a.contents && <Row k="/Contents">{a.contents}</Row>}
            {(a.author || a.subject) && (
              <Row k="/T · /Subj">
                {a.author ?? "—"} · {a.subject ?? "—"}
              </Row>
            )}
            {a.modified && <Row k="/M">{a.modified}</Row>}
            {(a.color || a.opacity !== null || a.border !== null) && (
              <Row k="/C · /CA · width">
                <Group gap={6}>
                  {color && <span style={{ display: "inline-block", width: 12, height: 12, background: color, border: "1px solid #0003" }} />}
                  <Text span size="xs" ff="monospace">
                    {a.color ? `[${a.color.map(num).join(" ")}]` : "—"} · {a.opacity ?? "—"} · {a.border ?? "—"}
                  </Text>
                </Group>
              </Row>
            )}
            {a.quads.length > 0 && (
              <Row k="/QuadPoints">
                {a.quads.length} quadrilateral{a.quads.length === 1 ? "" : "s"}
              </Row>
            )}
            {a.paths.length > 0 && (
              <Row k="Paths">
                {a.paths.length} path{a.paths.length === 1 ? "" : "s"}, {a.paths.reduce((n, p) => n + p.length / 2, 0)} points
              </Row>
            )}
            {a.link && (
              <Row k="Link">
                <LinkView link={a.link} />
              </Row>
            )}
            {(a.parent || a.popup) && (
              <Row k={a.parent ? "For" : "/Popup"}>
                <ObjLink id={(a.parent ?? a.popup)!} />
              </Row>
            )}
            <Row k="/AP">{a.appearances.length > 0 ? a.appearances.map((k) => `/${k}`).join(" ") : "none"}</Row>
            {ap && (
              <>
                <Row k={ap.kind === "N" ? "Normal" : ap.kind === "R" ? "Rollover" : "Down"}>
                  {ap.id ? (
                    <Anchor
                      size="xs"
                      ff="monospace"
                      onClick={() => nav.open({ kind: "content", content: { kind: "stream", id: ap.id! }, op: null })}
                    >
                      {ap.id.num} {ap.id.generation} obj — open its operations
                    </Anchor>
                  ) : (
                    "—"
                  )}
                  {ap.states.length > 0 && (
                    <Text span size="xs" c="dimmed">
                      {" "}
                      · {pick ? "state" : "/AS"} {ap.state ? `/${ap.state}` : "missing"} of {ap.states.map((s) => `/${s}`).join(" ")}
                    </Text>
                  )}
                </Row>
                <Row k="/BBox · /Matrix">
                  <Text span size="xs" ff="monospace">
                    {rectText(ap.bbox)} · {matrixText(ap.matrix)}
                  </Text>
                </Row>
                {ap.to_page && (
                  <Row k="Form → page">
                    <Text span size="xs" ff="monospace">
                      {matrixText(ap.to_page)}
                    </Text>
                    <Text span size="xs" c="dimmed">
                      {" "}
                      (/Matrix, then /BBox fit to /Rect; dotted on the page, thick edge = form x axis)
                    </Text>
                  </Row>
                )}
                {ap.error && (
                  <Row k="Problem">
                    <Text span size="xs" c="red">
                      {ap.error}
                    </Text>
                  </Row>
                )}
              </>
            )}
          </Table.Tbody>
        </Table>
      </Stack>
    </Paper>
  );
}
