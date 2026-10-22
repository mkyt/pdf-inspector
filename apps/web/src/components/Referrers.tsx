import { Anchor, Button, Group, Loader, Stack, Text } from "@mantine/core";
import { useInfiniteQuery } from "@tanstack/react-query";
import { api, useBackend } from "../api";
import { useDoc } from "../doc";
import { objText } from "../format";
import { useNav } from "../nav";
import type { ObjRef } from "../protocol/generated";

const PAGE = 100;

/** Objects referring to `id`, from the inventory's reference graph. */
export function Referrers({ id }: { id: ObjRef }) {
  const { inventory } = useDoc();
  if (!inventory.stats) {
    const running = inventory.job?.state === "running";
    return (
      <Group gap={6}>
        <Text size="xs" c="dimmed">
          Referenced by: needs the inventory.
        </Text>
        <Button size="compact-xs" variant="subtle" disabled={running} onClick={inventory.start}>
          {running
            ? `Building… ${inventory.job?.total ? Math.round((100 * inventory.job.done) / inventory.job.total) : 0}%`
            : "Build"}
        </Button>
      </Group>
    );
  }
  return <List id={id} />;
}

function List({ id }: { id: ObjRef }) {
  const backend = useBackend();
  const nav = useNav();
  const q = useInfiniteQuery({
    queryKey: ["doc", nav.doc, "referrers", id.num, id.generation],
    queryFn: ({ pageParam }) => api.referrers(backend, nav.doc, id, { offset: pageParam, limit: PAGE }),
    initialPageParam: 0,
    getNextPageParam: (last) =>
      last.offset + last.items.length < last.total ? last.offset + last.items.length : undefined,
  });
  if (q.error)
    return (
      <Text size="xs" c="red">
        {String(q.error)}
      </Text>
    );
  if (!q.data) return <Loader size="xs" />;
  const items = q.data.pages.flatMap((p) => p.items);
  const total = q.data.pages[0].total;
  return (
    <Stack gap={2}>
      <Text size="xs" c="dimmed">
        {total === 0 ? "Referenced by nothing." : `Referenced by ${total.toLocaleString()}:`}
      </Text>
      {items.map((r, i) => (
        <Group key={i} gap={6} wrap="nowrap">
          <Anchor
            size="xs"
            ff="monospace"
            onClick={() =>
              nav.open(r.from.kind === "trailer" ? { kind: "trailer" } : { kind: "object", id: r.from.id })
            }
          >
            {r.from.kind === "trailer" ? "trailer" : objText(r.from.id)}
          </Anchor>
          {r.type_name && (
            <Text size="xs" c="teal">
              {r.type_name}
            </Text>
          )}
          <Text
            size="xs"
            ff="monospace"
            c="dimmed"
            style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
          >
            {r.paths.join("  ")}
          </Text>
        </Group>
      ))}
      {q.hasNextPage && (
        <Button
          size="compact-xs"
          variant="subtle"
          loading={q.isFetchingNextPage}
          onClick={() => void q.fetchNextPage()}
          style={{ alignSelf: "flex-start" }}
        >
          Show more ({items.length.toLocaleString()} of {total.toLocaleString()})
        </Button>
      )}
    </Stack>
  );
}
