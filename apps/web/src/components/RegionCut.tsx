import { ActionIcon, Button, Group, SegmentedControl, Switch, Text, Tooltip } from "@mantine/core";
import { useEffect, useState } from "react";
import { api, useBackend } from "../api";
import { useDoc } from "../doc";
import { useNav } from "../nav";
import type { ContentRef, ExtractInfo, Rect, RegionSelect, ViewInfo } from "../protocol/generated";
import { save } from "./ImagePreview";

/** A rectangle of the view in points (from the top left), as dragged: not normalized. */
export interface ViewRegion {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

export function normalized(r: ViewRegion): ViewRegion {
  return { x0: Math.min(r.x0, r.x1), y0: Math.min(r.y0, r.y1), x1: Math.max(r.x0, r.x1), y1: Math.max(r.y0, r.y1) };
}

/** A view rectangle in the content's space (the view matrix inverted). */
function toContent(r: ViewRegion, v: ViewInfo): Rect {
  const [a, b, c, d, e, f] = v.matrix;
  const det = a * d - b * c;
  const xs: number[] = [];
  const ys: number[] = [];
  for (const [x, y] of [
    [r.x0, r.y0],
    [r.x1, r.y0],
    [r.x0, r.y1],
    [r.x1, r.y1],
  ]) {
    const vx = x - e;
    const vy = y - f;
    xs.push((d * vx - c * vy) / det);
    ys.push((a * vy - b * vx) / det);
  }
  return { x0: Math.min(...xs), y0: Math.min(...ys), x1: Math.max(...xs), y1: Math.max(...ys) };
}

const num = (x: number) => (Math.round(x * 100) / 100).toString();

/**
 * The region dragged over a rendered content, and cutting it out: a
 * one-page PDF with only the operations painting there (kept as they are
 * in the file, vector and all).
 */
export function RegionCut({
  content,
  view,
  region,
  onClear,
}: {
  content: ContentRef;
  view: ViewInfo;
  region: ViewRegion;
  onClear: () => void;
}) {
  const backend = useBackend();
  const nav = useNav();
  const { summary } = useDoc();
  const [select, setSelect] = useState<RegionSelect>("inside");
  const [fit, setFit] = useState(true);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<ExtractInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const rect = toContent(normalized(region), view);
  const key = `${rect.x0},${rect.y0},${rect.x1},${rect.y1},${select},${fit}`;
  useEffect(() => {
    setDone(null);
    setError(null);
  }, [key]);

  const stem = summary.info.name.replace(/\.pdf$/i, "");
  const what = content.kind === "page" ? `p${content.index + 1}` : `${content.id.num}-${content.id.generation}`;
  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await api.extractRegion(backend, nav.doc, content, rect, select, fit);
      setDone(r.info);
      save(new Blob([r.data as BlobPart], { type: "application/pdf" }), `${stem}-${what}-region.pdf`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={{ marginTop: 4 }}>
      <Group gap="xs" wrap="wrap">
        <Text size="xs">
          Region [{num(rect.x0)} {num(rect.y0)} {num(rect.x1)} {num(rect.y1)}] ({num(rect.x1 - rect.x0)} ×{" "}
          {num(rect.y1 - rect.y0)} pt)
        </Text>
        <Tooltip label="Which painting operations to keep: those painting wholly inside the region, or also those only touching it">
          <SegmentedControl
            size="xs"
            value={select}
            onChange={(v) => setSelect(v as RegionSelect)}
            data={[
              { label: "Inside", value: "inside" },
              { label: "Touching", value: "overlaps" },
            ]}
          />
        </Tooltip>
        <Tooltip label="Shrink the page to what is kept (within the region)">
          <Switch size="xs" label="Crop to content" checked={fit} onChange={(e) => setFit(e.currentTarget.checked)} />
        </Tooltip>
        <Tooltip label="A one-page PDF with only the operations painting here; what they use is copied as it is in the file">
          <Button size="compact-xs" loading={busy} onClick={() => void run()}>
            Download PDF
          </Button>
        </Tooltip>
        <Tooltip label="Clear the region">
          <ActionIcon size="sm" variant="subtle" onClick={onClear} aria-label="Clear the region">
            ×
          </ActionIcon>
        </Tooltip>
      </Group>
      {done && (
        <Text size="xs" c="dimmed" mt={2}>
          Kept {done.kept.toLocaleString()} of {(done.kept + done.dropped).toLocaleString()} painting operations
          {done.forms > 0 ? `, ${done.forms} form XObject${done.forms > 1 ? "s" : ""} cut` : ""}; {done.objects} objects,{" "}
          {(done.bytes / 1024).toFixed(1)} KiB; /MediaBox [{num(done.media_box.x0)} {num(done.media_box.y0)}{" "}
          {num(done.media_box.x1)} {num(done.media_box.y1)}].
        </Text>
      )}
      {error && (
        <Text size="xs" c="red" mt={2}>
          {error}
        </Text>
      )}
    </div>
  );
}
