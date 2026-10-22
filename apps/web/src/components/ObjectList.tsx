import { Badge, Button, Checkbox, Group, NumberInput, Select, Text } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api, useBackend } from "../api";
import { useDoc } from "../doc";
import { entryGeneration, entryText } from "../format";
import { useNav } from "../nav";
import type { DocId, ObjectClass, ObjectFilter } from "../protocol/generated";
import { PagedList } from "./PagedList";
import { Row } from "./Row";
import classes from "./Row.module.css";

/** Select value standing for "no such entry" (a name can be empty, so not ""). */
const NONE = "\u0000";

interface Picks {
  type: string | null;
  subtype: string | null;
  kind: string | null;
  unreferenced: boolean;
}

const NO_PICKS: Picks = { type: null, subtype: null, kind: null, unreferenced: false };
/** Kept per document while it is open, so switching tabs does not lose them. */
const remembered = new Map<DocId, Picks>();

const nameKey = (n: string | null) => n ?? NONE;

function matches(c: ObjectClass, p: Picks, skip?: keyof Picks): boolean {
  return (
    (skip === "type" || p.type === null || nameKey(c.type_name) === p.type) &&
    (skip === "subtype" || p.subtype === null || nameKey(c.subtype) === p.subtype) &&
    (skip === "kind" || p.kind === null || c.kind === p.kind)
  );
}

/** Options of one select: the values of the classes the other picks leave, with counts. */
function options(all: ObjectClass[], p: Picks, field: "type" | "subtype" | "kind") {
  const counts = new Map<string, number>();
  for (const c of all) {
    if (!matches(c, p, field)) continue;
    const v = field === "kind" ? c.kind : nameKey(field === "type" ? c.type_name : c.subtype);
    counts.set(v, (counts.get(v) ?? 0) + c.count);
  }
  return [...counts]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([v, n]) => ({
      value: v,
      label: `${v === NONE ? "(none)" : field === "kind" ? v : `/${v}`}  ·  ${n.toLocaleString()}`,
    }));
}

export function ObjectList({ total }: { total: number }) {
  const backend = useBackend();
  const nav = useNav();
  const { inventory } = useDoc();
  const [goto, setGoto] = useState<string | number>("");
  const [picks, setPicksState] = useState<Picks>(() => remembered.get(nav.doc) ?? NO_PICKS);
  const setPicks = (p: Picks) => {
    remembered.set(nav.doc, p);
    setPicksState(p);
  };
  const current = nav.target.kind === "object" ? nav.target.id.num : null;

  const classList = useQuery({
    queryKey: ["doc", nav.doc, "objectClasses"],
    queryFn: () => api.objectClasses(backend, nav.doc),
    enabled: !!inventory.stats,
  }).data;
  const byClass = picks.type !== null || picks.subtype !== null || picks.kind !== null;
  const filter: ObjectFilter | null =
    inventory.stats && classList && (byClass || picks.unreferenced)
      ? { classes: byClass ? classList.filter((c) => matches(c, picks)) : null, unreferenced: picks.unreferenced }
      : null;
  const filterKey = JSON.stringify(filter);
  const counted = useQuery({
    queryKey: ["doc", nav.doc, "objects", filterKey, "total"],
    queryFn: () => api.objects(backend, nav.doc, { offset: 0, limit: 0 }, filter),
    enabled: filter !== null,
  });
  const shown = filter ? counted.data?.total : total;

  return (
    <>
      <Group p={6} gap={6} wrap="nowrap">
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
        <Text size="xs" c="dimmed" style={{ whiteSpace: "nowrap" }}>
          {filter
            ? `${shown === undefined ? "…" : shown.toLocaleString()} of ${total.toLocaleString()}`
            : `${total.toLocaleString()} entries`}
        </Text>
      </Group>
      {inventory.stats && classList ? (
        <Group px={6} pb={6} gap={6}>
          <Select
            size="xs"
            placeholder="/Type"
            aria-label="/Type"
            searchable
            clearable
            data={options(classList, picks, "type")}
            value={picks.type}
            onChange={(v) => setPicks({ ...picks, type: v })}
            style={{ flex: "1 1 120px" }}
            comboboxProps={{ width: 260, position: "bottom-start" }}
          />
          <Select
            size="xs"
            placeholder="/Subtype"
            aria-label="/Subtype"
            searchable
            clearable
            data={options(classList, picks, "subtype")}
            value={picks.subtype}
            onChange={(v) => setPicks({ ...picks, subtype: v })}
            style={{ flex: "1 1 120px" }}
            comboboxProps={{ width: 260, position: "bottom-start" }}
          />
          <Select
            size="xs"
            placeholder="Kind"
            aria-label="Kind"
            clearable
            data={options(classList, picks, "kind")}
            value={picks.kind}
            onChange={(v) => setPicks({ ...picks, kind: v })}
            style={{ flex: "1 1 100px" }}
            comboboxProps={{ width: 200, position: "bottom-start" }}
          />
          <Checkbox
            size="xs"
            label={`Unreferenced (${inventory.stats.unreferenced.toLocaleString()})`}
            checked={picks.unreferenced}
            onChange={(e) => setPicks({ ...picks, unreferenced: e.currentTarget.checked })}
          />
          {(byClass || picks.unreferenced) && (
            <Button size="compact-xs" variant="subtle" color="gray" onClick={() => setPicks(NO_PICKS)}>
              Clear
            </Button>
          )}
        </Group>
      ) : (
        !inventory.stats && (
          <Group px={6} pb={6} gap={6}>
            <Text size="xs" c="dimmed">
              Filter by /Type, /Subtype, kind or references:
            </Text>
            <Button
              size="compact-xs"
              variant="subtle"
              loading={inventory.job?.state === "running"}
              onClick={inventory.start}
            >
              Build inventory
            </Button>
          </Group>
        )
      )}
      {shown !== undefined && (
        <PagedList
          key={filterKey}
          total={shown}
          empty={picks.unreferenced && !byClass ? "Every object is referred to." : "No object matches."}
          queryKey={["doc", nav.doc, "objects", filterKey]}
          fetchPage={(r, signal) => api.objects(backend, nav.doc, r, filter, signal)}
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
      )}
    </>
  );
}
