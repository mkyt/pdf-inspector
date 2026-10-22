import { Badge, Group, NumberInput, Text } from "@mantine/core";
import { useState } from "react";
import { api, useBackend } from "../api";
import { useDoc } from "../doc";
import { pageText, sizeText } from "../format";
import { useNav } from "../nav";
import { PagedList } from "./PagedList";
import { Row } from "./Row";
import classes from "./Row.module.css";

/** Every page, fetched by range (the backend finds each one through the page tree's counts). */
export function PageList() {
  const backend = useBackend();
  const nav = useNav();
  const { summary } = useDoc();
  const [goto, setGoto] = useState<string | number>("");
  const current = nav.target.kind === "page" ? nav.target.index : null;
  return (
    <>
      <Group p={6} gap={6}>
        <NumberInput
          size="xs"
          placeholder="Go to page"
          min={1}
          max={summary.pages}
          hideControls
          value={goto}
          onChange={setGoto}
          onKeyDown={(e) => {
            if (e.key === "Enter" && typeof goto === "number" && goto >= 1 && goto <= summary.pages) {
              nav.open({ kind: "page", index: goto - 1 });
            }
          }}
          style={{ flex: 1 }}
        />
        <Text size="xs" c="dimmed">
          {summary.pages.toLocaleString()} pages
        </Text>
      </Group>
      <PagedList
        total={summary.pages}
        empty="No pages."
        queryKey={["doc", nav.doc, "pages"]}
        fetchPage={(r, signal) => api.pages(backend, nav.doc, r, signal)}
        renderRow={(p) => (
          <Row active={p.index === current} onClick={() => nav.open({ kind: "page", index: p.index })}>
            <Text size="xs" w={72} ta="right" fw={500} style={{ overflow: "hidden", textOverflow: "ellipsis" }}>
              {pageText(p.index, p.label)}
            </Text>
            {p.error ? (
              <Text size="xs" c="red" style={{ overflow: "hidden", textOverflow: "ellipsis" }} title={p.error}>
                {p.error}
              </Text>
            ) : (
              <>
                <Text size="xs" c="dimmed" className={classes.mono} w={64}>
                  {p.id && `${p.id.num} ${p.id.generation}`}
                </Text>
                <Text size="xs" style={{ flex: 1 }}>
                  {sizeText(p.size)}
                </Text>
                {p.rotate !== 0 && (
                  <Badge size="xs" variant="light" color="gray">
                    {p.rotate}°
                  </Badge>
                )}
                {p.annots > 0 && (
                  <Badge size="xs" variant="light" color="grape" title="annotations">
                    {p.annots}
                  </Badge>
                )}
              </>
            )}
          </Row>
        )}
      />
    </>
  );
}
