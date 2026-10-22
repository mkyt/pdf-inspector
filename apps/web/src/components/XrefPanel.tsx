import { Anchor, Group, Select, Stack, Text } from "@mantine/core";
import { useState } from "react";
import { api, useBackend } from "../api";
import { entryGeneration, entryText, objText, spanText } from "../format";
import { useNav } from "../nav";
import type { DocSummary } from "../protocol/generated";
import { PagedList } from "./PagedList";
import { Row } from "./Row";
import classes from "./Row.module.css";

export function XrefPanel({ summary }: { summary: DocSummary }) {
  const backend = useBackend();
  const nav = useNav();
  const [rev, setRev] = useState<string>("merged");
  const revision = rev === "merged" ? null : Number(rev);
  const r = revision === null ? null : summary.revisions[revision];
  const total = r ? r.entries : summary.xref_entries;

  return (
    <>
      <Stack p={6} gap={4}>
        <Select
          size="xs"
          value={rev}
          onChange={(v) => setRev(v ?? "merged")}
          allowDeselect={false}
          data={[
            { value: "merged", label: "Merged table" },
            ...summary.revisions.map((r) => ({
              value: String(r.index),
              label: `#${r.index} ${r.kind} @${r.offset.toLocaleString()}${r.index === 0 ? " (newest)" : ""}`,
            })),
          ]}
        />
        {r && (
          <Group gap="xs">
            <Anchor size="xs" onClick={() => nav.showBytes(r.span)}>
              {spanText(r.span)}
            </Anchor>
            {r.xref_stream && (
              <Anchor size="xs" onClick={() => nav.open({ kind: "object", id: r.xref_stream! })}>
                {objText(r.xref_stream)}
              </Anchor>
            )}
          </Group>
        )}
      </Stack>
      <PagedList
        key={rev}
        total={total}
        queryKey={["doc", nav.doc, "xref", rev]}
        fetchPage={(range, signal) => api.xref(backend, nav.doc, revision, range, signal)}
        renderRow={(x) => (
          <Row
            onClick={
              x.entry.kind === "free"
                ? undefined
                : () => nav.open({ kind: "object", id: { num: x.num, generation: entryGeneration(x.entry) } })
            }
          >
            <Text size="xs" className={classes.mono} w={64} ta="right">
              {x.num}
            </Text>
            <Text size="xs" className={classes.mono} w={48} c="dimmed">
              {entryGeneration(x.entry)}
            </Text>
            <Text size="xs" className={classes.mono}>
              {x.entry.kind === "inUse" ? "n" : x.entry.kind === "free" ? "f" : "c"} {entryText(x.entry)}
            </Text>
          </Row>
        )}
      />
    </>
  );
}
