import { Badge, Group, NumberInput, Text } from "@mantine/core";
import { useState } from "react";
import { api, useBackend } from "../api";
import { entryGeneration, entryText } from "../format";
import { useNav } from "../nav";
import { PagedList } from "./PagedList";
import { Row } from "./Row";
import classes from "./Row.module.css";

export function ObjectList({ total }: { total: number }) {
  const backend = useBackend();
  const nav = useNav();
  const [goto, setGoto] = useState<string | number>("");
  const current = nav.target.kind === "object" ? nav.target.id.num : null;

  return (
    <>
      <Group p={6} gap={6}>
        <NumberInput
          size="xs"
          placeholder="Go to object #"
          min={0}
          hideControls
          value={goto}
          onChange={setGoto}
          onKeyDown={(e) => {
            if (e.key === "Enter" && typeof goto === "number") {
              nav.open({ kind: "object", id: { num: goto, generation: 0 } });
            }
          }}
          style={{ flex: 1 }}
        />
        <Text size="xs" c="dimmed">
          {total.toLocaleString()} entries
        </Text>
      </Group>
      <PagedList
        total={total}
        queryKey={["doc", nav.doc, "objects"]}
        fetchPage={(r, signal) => api.objects(backend, nav.doc, r, signal)}
        renderRow={(o) => {
          const free = o.entry.kind === "free";
          return (
            <Row
              active={o.num === current}
              onClick={
                free
                  ? undefined
                  : () => nav.open({ kind: "object", id: { num: o.num, generation: entryGeneration(o.entry) } })
              }
            >
              <Text size="xs" className={classes.mono} w={64} ta="right" c={free ? "dimmed" : undefined}>
                {o.num} {entryGeneration(o.entry)}
              </Text>
              {o.error ? (
                <Badge size="xs" color="red" variant="light">
                  error
                </Badge>
              ) : (
                o.kind && (
                  <Text size="xs" c="dimmed" w={70}>
                    {o.kind}
                  </Text>
                )
              )}
              <Text
                size="xs"
                fw={500}
                style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}
                title={[o.type_name && `/${o.type_name}`, o.subtype && `/${o.subtype}`].filter(Boolean).join(" ")}
              >
                {o.schema_type ?? (o.type_name && `/${o.type_name}`)}
                {!o.schema_type && o.subtype && ` /${o.subtype}`}
              </Text>
              <Text size="xs" c="dimmed" className={classes.mono}>
                {entryText(o.entry)}
              </Text>
            </Row>
          );
        }}
      />
    </>
  );
}
