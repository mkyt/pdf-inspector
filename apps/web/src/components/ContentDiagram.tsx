import { Group, Loader, Slider, Switch, Text } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api, useBackend } from "../api";
import { contentKey, useNav } from "../nav";
import type { ContentRef, ContentSummary, ElementView, Rect } from "../protocol/generated";

/** Elements drawn at most (fetched in pages of the backend's maximum). */
const MAX_ELEMENTS = 10_000;
const PAGE = 2000;

const kindColor: Record<string, string> = {
  fill: "blue",
  stroke: "teal",
  fillStroke: "indigo",
  clip: "gray",
  text: "orange",
  image: "grape",
  inlineImage: "violet",
  form: "green",
  shading: "pink",
};

const area = (r: Rect) => (r.x1 - r.x0) * (r.y1 - r.y0);

function union(a: Rect | null, b: Rect): Rect {
  return a ? { x0: Math.min(a.x0, b.x0), y0: Math.min(a.y0, b.y0), x1: Math.max(a.x1, b.x1), y1: Math.max(a.y1, b.y1) } : b;
}

/**
 * What the operations paint, as bounding boxes in the content's space
 * (default user space, unrotated, for pages). With "step", only what the
 * operations up to the selected one paint is drawn.
 */
export function ContentDiagram({
  content,
  summary,
  op,
  onPick,
}: {
  content: ContentRef;
  summary: ContentSummary;
  op: number | null;
  onPick: (op: number) => void;
}) {
  const backend = useBackend();
  const nav = useNav();
  const total = Math.min(summary.elements, MAX_ELEMENTS);
  const q = useQuery({
    queryKey: ["doc", nav.doc, "content", ...contentKey(content), "elements", total],
    queryFn: async () => {
      const out: ElementView[] = [];
      for (let o = 0; o < total; o += PAGE) {
        const p = await api.contentElements(backend, nav.doc, content, { offset: o, limit: Math.min(PAGE, total - o) });
        out.push(...p.items);
      }
      return out;
    },
  });
  const [step, setStep] = useState(true);
  const [clips, setClips] = useState(false);
  // The slider moves freely; the selection follows when it is released.
  const [drag, setDrag] = useState<number | null>(null);
  useEffect(() => setDrag(null), [op]);
  const upto = drag ?? op;

  const elements = q.data;
  const sorted = useMemo(
    () => (elements ? [...elements].filter((e) => e.kind !== "clip" || clips).sort((a, b) => area(b.bbox) - area(a.bbox)) : []),
    [elements, clips],
  );
  const view = useMemo(() => {
    let r: Rect | null = summary.bounds;
    if (!r) for (const e of elements ?? []) r = union(r, e.bbox);
    return r ?? { x0: 0, y0: 0, x1: 100, y1: 100 };
  }, [summary.bounds, elements]);
  const w = Math.max(view.x1 - view.x0, 1);
  const h = Math.max(view.y1 - view.y0, 1);
  const pad = Math.max(w, h) * 0.03;
  const selected = elements?.find((e) => e.op === upto);

  return (
    <div style={{ padding: 8, borderBottom: "1px solid var(--mantine-color-default-border)" }}>
      <Group gap="sm" mb={4}>
        <Switch size="xs" label="Step" checked={step} onChange={(e) => setStep(e.currentTarget.checked)} />
        <Switch size="xs" label="Clips" checked={clips} onChange={(e) => setClips(e.currentTarget.checked)} />
        <Text size="xs" c="dimmed" style={{ marginLeft: "auto" }}>
          {summary.elements > MAX_ELEMENTS
            ? `first ${MAX_ELEMENTS.toLocaleString()} of ${summary.elements.toLocaleString()}`
            : `${summary.elements.toLocaleString()} boxes`}
        </Text>
      </Group>
      {!elements ? (
        <Loader size="xs" />
      ) : (
        <svg
          viewBox={`${view.x0 - pad} ${-view.y1 - pad} ${w + 2 * pad} ${h + 2 * pad}`}
          style={{ width: "100%", maxHeight: 520, display: "block", background: "var(--mantine-color-gray-light)" }}
        >
          <rect
            x={view.x0}
            y={-view.y1}
            width={w}
            height={h}
            fill="var(--mantine-color-body)"
            stroke="var(--mantine-color-default-border)"
            vectorEffect="non-scaling-stroke"
          />
          {sorted.map((e, k) => {
            const later = step && upto !== null && e.op > upto;
            const c = `var(--mantine-color-${kindColor[e.kind] ?? "gray"}-filled)`;
            return (
              <rect
                key={k}
                x={e.bbox.x0}
                y={-e.bbox.y1}
                width={Math.max(e.bbox.x1 - e.bbox.x0, 0)}
                height={Math.max(e.bbox.y1 - e.bbox.y0, 0)}
                fill={e.kind === "clip" ? "none" : c}
                fillOpacity={later ? 0.02 : 0.1}
                stroke={c}
                strokeOpacity={later ? 0.15 : 0.8}
                strokeWidth={1}
                strokeDasharray={e.hidden || e.kind === "clip" ? "3 2" : undefined}
                vectorEffect="non-scaling-stroke"
                style={{ cursor: "pointer" }}
                onClick={() => onPick(e.op)}
              >
                <title>{`#${e.op} ${e.kind}${e.hidden ? " (hidden)" : ""}`}</title>
              </rect>
            );
          })}
          {selected && (
            <rect
              x={selected.bbox.x0}
              y={-selected.bbox.y1}
              width={Math.max(selected.bbox.x1 - selected.bbox.x0, 0)}
              height={Math.max(selected.bbox.y1 - selected.bbox.y0, 0)}
              fill="var(--mantine-color-red-filled)"
              fillOpacity={0.15}
              stroke="var(--mantine-color-red-filled)"
              strokeWidth={2.5}
              vectorEffect="non-scaling-stroke"
              pointerEvents="none"
            />
          )}
        </svg>
      )}
      {summary.ops > 1 && (
        <Slider
          mt={8}
          size="xs"
          min={0}
          max={summary.ops - 1}
          value={upto ?? 0}
          onChange={setDrag}
          onChangeEnd={onPick}
          label={(v) => `#${v}`}
        />
      )}
      <Text size="xs" c="dimmed" mt={4}>
        Bounding boxes in {content.kind === "page" ? "default user space (unrotated)" : "the stream's space"}; click one to
        select its operation.
      </Text>
    </div>
  );
}
