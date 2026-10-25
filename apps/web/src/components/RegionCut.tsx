import { ActionIcon, Button, Group, SegmentedControl, Switch, Text, Tooltip } from "@mantine/core";
import { useEffect, useState } from "react";
import { api, useBackend } from "../api";
import { useDoc } from "../doc";
import { useNav } from "../nav";
import type { ContentRef, ExtractInfo, Rect, RegionSelect, SvgInfo, SvgText, ViewInfo } from "../protocol/generated";
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

type Format = "pdf" | "svg";

/**
 * The region dragged over a rendered content, and cutting it out: a
 * one-page PDF with only the operations painting there (kept as they are
 * in the file, vector and all), or SVG to edit in a drawing program.
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
  const [format, setFormat] = useState<Format>("pdf");
  const [text, setText] = useState<SvgText>("text");
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<ExtractInfo | null>(null);
  const [svgDone, setSvgDone] = useState<SvgInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const rect = toContent(normalized(region), view);
  const key = `${rect.x0},${rect.y0},${rect.x1},${rect.y1},${select},${fit},${format},${text}`;
  useEffect(() => {
    setDone(null);
    setSvgDone(null);
    setError(null);
  }, [key]);

  const stem = summary.info.name.replace(/\.pdf$/i, "");
  const what = content.kind === "page" ? `p${content.index + 1}` : `${content.id.num}-${content.id.generation}`;
  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      if (format === "svg") {
        const r = await api.exportSvg(backend, nav.doc, content, rect, select, fit, text);
        setSvgDone(r.info);
        save(new Blob([r.data as BlobPart], { type: "image/svg+xml" }), `${stem}-${what}-region.svg`);
      } else {
        const r = await api.extractRegion(backend, nav.doc, content, rect, select, fit);
        setDone(r.info);
        save(new Blob([r.data as BlobPart], { type: "application/pdf" }), `${stem}-${what}-region.pdf`);
      }
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
        <Tooltip label="PDF: a one-page PDF with only the operations painting here, what they use copied as it is in the file. SVG: to edit in a drawing program, groups, clips and layers kept">
          <SegmentedControl
            size="xs"
            value={format}
            onChange={(v) => setFormat(v as Format)}
            data={[
              { label: "PDF", value: "pdf" },
              { label: "SVG", value: "svg" },
            ]}
          />
        </Tooltip>
        {format === "svg" && (
          <Tooltip label="Text as <text> elements to edit (in the fonts the file names, each character where it paints), or as glyph outlines that look exactly the same">
            <SegmentedControl
              size="xs"
              value={text}
              onChange={(v) => setText(v as SvgText)}
              data={[
                { label: "Text", value: "text" },
                { label: "Outlines", value: "paths" },
              ]}
            />
          </Tooltip>
        )}
        <Button size="compact-xs" loading={busy} onClick={() => void run()}>
          Download {format === "svg" ? "SVG" : "PDF"}
        </Button>
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
      {svgDone && (
        <Text size="xs" c="dimmed" mt={2}>
          {svgDone.elements.toLocaleString()} element{svgDone.elements === 1 ? "" : "s"}
          {svgDone.images > 0 ? ` (${svgDone.images} image${svgDone.images > 1 ? "s" : ""})` : ""},{" "}
          {(svgDone.bytes / 1024).toFixed(1)} KiB; {num(svgDone.view_box.x1 - svgDone.view_box.x0)} ×{" "}
          {num(svgDone.view_box.y1 - svgDone.view_box.y0)} pt.
          {svgDone.notes.map((n) => ` ${n.message}${n.count > 1 ? ` (×${n.count})` : ""}.`).join("")}
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
