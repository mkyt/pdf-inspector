import { Badge, Group, Text } from "@mantine/core";
import { api, useBackend } from "../api";
import { useDoc } from "../doc";
import { bytesText } from "../format";
import { useNav } from "../nav";
import type { ObjRef } from "../protocol/generated";
import { InventoryGate } from "./InventoryGate";
import { PagedList } from "./PagedList";
import { Row } from "./Row";
import classes from "./Row.module.css";

/** `5 pages: 1, 2, 3, …` (the backend lists the first ones). */
function usage(count: number, pages: number[]): string {
  if (count === 0) return "unused";
  const shown = pages.slice(0, 6).map((p) => p + 1);
  return `${count === 1 ? "page" : `${count.toLocaleString()} pages:`} ${shown.join(", ")}${count > shown.length ? ", …" : ""}`;
}

function Num({ id }: { id: ObjRef }) {
  return (
    <Text size="xs" className={classes.mono} w={56} ta="right" c="dimmed" style={{ flexShrink: 0 }}>
      {id.num} {id.generation}
    </Text>
  );
}

export function FontsPanel() {
  return (
    <InventoryGate what="The font list">
      <Fonts />
    </InventoryGate>
  );
}

export function ImagesPanel() {
  return (
    <InventoryGate what="The image list">
      <Images />
    </InventoryGate>
  );
}

function Fonts() {
  const backend = useBackend();
  const nav = useNav();
  const { inventory } = useDoc();
  const current = nav.target.kind === "object" ? nav.target.id.num : null;
  return (
    <PagedList
      total={inventory.stats?.fonts ?? 0}
      empty="No fonts."
      rowHeight={44}
      queryKey={["doc", nav.doc, "fonts"]}
      fetchPage={(r, signal) => api.fonts(backend, nav.doc, r, signal)}
      renderRow={(f) => (
        <Row active={f.id.num === current} onClick={() => nav.open({ kind: "object", id: f.id })}>
          <Num id={f.id} />
          <div style={{ minWidth: 0, flex: 1 }}>
            <Group gap={6} wrap="nowrap">
              <Text
                size="xs"
                fw={500}
                style={{ overflow: "hidden", textOverflow: "ellipsis" }}
                title={f.base_font ?? undefined}
              >
                {f.base_font ?? "(no /BaseFont)"}
              </Text>
              <Badge size="xs" variant="light" color="gray" tt="none" style={{ flexShrink: 0 }}>
                {f.subtype ?? "?"}
              </Badge>
            </Group>
            <Group gap={6} wrap="nowrap">
              {f.embedded ? (
                <Badge size="xs" variant="light" color="teal" tt="none">
                  {f.embedded}
                </Badge>
              ) : (
                <Badge size="xs" variant="light" color={f.standard ? "blue" : "orange"} tt="none">
                  {f.standard ? "standard 14" : "not embedded"}
                </Badge>
              )}
              {f.subset && (
                <Badge size="xs" variant="outline" color="gray" tt="none">
                  subset
                </Badge>
              )}
              {f.to_unicode && (
                <Badge size="xs" variant="outline" color="gray" tt="none">
                  ToUnicode
                </Badge>
              )}
              <Text size="xs" c="dimmed" style={{ overflow: "hidden", textOverflow: "ellipsis" }}>
                {f.descendant ? `→ ${f.descendant.num} · ` : ""}
                {usage(f.page_count, f.pages)}
              </Text>
            </Group>
          </div>
        </Row>
      )}
    />
  );
}

function Images() {
  const backend = useBackend();
  const nav = useNav();
  const { inventory } = useDoc();
  const current = nav.target.kind === "object" ? nav.target.id.num : null;
  return (
    <PagedList
      total={inventory.stats?.images ?? 0}
      empty="No images."
      rowHeight={44}
      queryKey={["doc", nav.doc, "images"]}
      fetchPage={(r, signal) => api.images(backend, nav.doc, r, signal)}
      renderRow={(i) => (
        <Row active={i.id.num === current} onClick={() => nav.open({ kind: "object", id: i.id })}>
          <Num id={i.id} />
          <div style={{ minWidth: 0, flex: 1 }}>
            <Group gap={6} wrap="nowrap">
              <Text size="xs" fw={500}>
                {i.width ?? "?"} × {i.height ?? "?"}
              </Text>
              <Text size="xs" c="dimmed" style={{ overflow: "hidden", textOverflow: "ellipsis" }}>
                {i.image_mask ? "ImageMask" : (i.color_space ?? "?")} · {i.bits ?? "?"} bpc
                {i.filters.length > 0 && ` · ${i.filters.join(", ")}`} · {bytesText(i.raw_len)}
              </Text>
            </Group>
            <Group gap={6} wrap="nowrap">
              {i.smask && (
                <Badge size="xs" variant="outline" color="gray" tt="none">
                  SMask {i.smask.num}
                </Badge>
              )}
              {i.mask && (
                <Badge size="xs" variant="outline" color="gray" tt="none">
                  Mask
                </Badge>
              )}
              <Text size="xs" c="dimmed" style={{ overflow: "hidden", textOverflow: "ellipsis" }}>
                {usage(i.page_count, i.pages)}
              </Text>
            </Group>
          </div>
        </Row>
      )}
    />
  );
}
