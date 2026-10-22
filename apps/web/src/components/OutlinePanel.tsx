import { Anchor, Button, Loader, Text, UnstyledButton } from "@mantine/core";
import { useInfiniteQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api, useBackend } from "../api";
import { linkText } from "../format";
import { useNav } from "../nav";
import type { ObjRef, OutlineItem } from "../protocol/generated";
import classes from "./Tree.module.css";

const PAGE = 200;
const MAX_DEPTH = 32;

/** The outline (bookmarks), expanded lazily. */
export function OutlinePanel() {
  return (
    <div style={{ flex: 1, minHeight: 0, overflow: "auto", padding: "4px 0" }}>
      <Items parent={null} depth={0} />
    </div>
  );
}

function Items({ parent, depth }: { parent: ObjRef | null; depth: number }) {
  const backend = useBackend();
  const nav = useNav();
  const q = useInfiniteQuery({
    queryKey: ["doc", nav.doc, "outline", parent?.num ?? null],
    queryFn: ({ pageParam }) => api.outline(backend, nav.doc, parent, { offset: pageParam, limit: PAGE }),
    initialPageParam: 0,
    getNextPageParam: (last) =>
      last.offset + last.items.length < last.total ? last.offset + last.items.length : undefined,
  });
  if (q.error) {
    return (
      <Text size="xs" c="red" pl={depth * 16 + 8}>
        {String(q.error)}
      </Text>
    );
  }
  if (!q.data) return <Loader size={12} ml={depth * 16 + 8} />;
  const items = q.data.pages.flatMap((p) => p.items);
  if (items.length === 0 && depth === 0) {
    return (
      <Text size="sm" c="dimmed" p="sm">
        No outline.
      </Text>
    );
  }
  return (
    <>
      {items.map((it) => (
        <Item key={`${it.id.num}/${it.id.generation}`} item={it} depth={depth} />
      ))}
      {q.hasNextPage && (
        <Button
          size="compact-xs"
          variant="subtle"
          ml={depth * 16 + 20}
          loading={q.isFetchingNextPage}
          onClick={() => void q.fetchNextPage()}
        >
          Show more ({items.length.toLocaleString()} of {q.data.pages[0].total.toLocaleString()})
        </Button>
      )}
    </>
  );
}

function Item({ item, depth }: { item: OutlineItem; depth: number }) {
  const nav = useNav();
  // Positive /Count: shown open by viewers.
  const [open, setOpen] = useState((item.count ?? 0) > 0 && depth < 2);
  const expandable = item.has_children && depth < MAX_DEPTH;
  const page = item.link?.kind === "dest" ? item.link.dest.page_index : null;
  const color =
    item.color && item.color.some((c) => c > 0)
      ? `rgb(${item.color.map((c) => Math.round(c * 255)).join(",")})`
      : undefined;
  const active = nav.target.kind === "object" && nav.target.id.num === item.id.num;
  return (
    <>
      <div className={classes.row} data-selected={active || undefined} style={{ paddingLeft: depth * 16 }}>
        <UnstyledButton
          className={classes.toggle}
          onClick={() => setOpen(!open)}
          disabled={!expandable}
          aria-label={open ? "Collapse" : "Expand"}
        >
          {expandable ? (open ? "▾" : "▸") : ""}
        </UnstyledButton>
        <UnstyledButton
          className={classes.label}
          style={{ flex: 1, fontFamily: "var(--mantine-font-family)" }}
          onClick={() => nav.open(page !== null ? { kind: "page", index: page } : { kind: "object", id: item.id })}
          title={item.link ? linkText(item.link) : "no destination"}
        >
          <span
            style={{
              color,
              fontWeight: item.flags & 2 ? 700 : undefined,
              fontStyle: item.flags & 1 ? "italic" : undefined,
            }}
          >
            {item.title || <i>(untitled)</i>}
          </span>
          <span className={classes.schemaType}>
            {page !== null ? `p. ${page + 1}` : item.link ? linkText(item.link) : ""}
          </span>
        </UnstyledButton>
        <Anchor size="xs" c="dimmed" ff="monospace" pr={8} onClick={() => nav.open({ kind: "object", id: item.id })}>
          {item.id.num}
        </Anchor>
      </div>
      {open && expandable && <Items parent={item.id} depth={depth + 1} />}
    </>
  );
}
