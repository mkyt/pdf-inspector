import { Anchor, Button, Loader, Text, UnstyledButton } from "@mantine/core";
import { useInfiniteQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api, useBackend } from "../api";
import { hasChildren, refText, valueText } from "../format";
import { useNav } from "../nav";
import type { NodePath, NodeView } from "../protocol/generated";
import classes from "./Tree.module.css";

const PAGE = 200;
const MAX_DEPTH = 32;

/** Children of `path`, fetched lazily and page by page. */
export function NodeChildren({ path, depth }: { path: NodePath; depth: number }) {
  const backend = useBackend();
  const nav = useNav();
  const q = useInfiniteQuery({
    queryKey: ["doc", nav.doc, "children", path],
    queryFn: ({ pageParam }) => api.children(backend, nav.doc, path, { offset: pageParam, limit: PAGE }),
    initialPageParam: 0,
    getNextPageParam: (last) =>
      last.offset + last.items.length < last.total ? last.offset + last.items.length : undefined,
  });
  if (q.error) {
    return (
      <Text size="xs" c="red" pl={depth * 16}>
        {String(q.error)}
      </Text>
    );
  }
  if (!q.data) return <Loader size={12} ml={depth * 16 + 8} />;
  const nodes = q.data.pages.flatMap((p) => p.items);
  const total = q.data.pages[0]?.total ?? 0;
  return (
    <>
      {nodes.map((n) => (
        <NodeRow key={n.index} parent={path} node={n} depth={depth} />
      ))}
      {q.hasNextPage && (
        <Button
          size="compact-xs"
          variant="subtle"
          ml={depth * 16 + 20}
          loading={q.isFetchingNextPage}
          onClick={() => void q.fetchNextPage()}
        >
          Show more ({nodes.length.toLocaleString()} of {total.toLocaleString()})
        </Button>
      )}
    </>
  );
}

function NodeRow({ parent, node, depth }: { parent: NodePath; node: NodeView; depth: number }) {
  const nav = useNav();
  const [open, setOpen] = useState(false);
  const v = node.value;
  const expandable = depth < MAX_DEPTH && (hasChildren(v) || v.type === "ref");
  const childPath: NodePath =
    v.type === "ref" ? { root: { kind: "object", id: v.id }, path: [] } : { root: parent.root, path: [...parent.path, node.index] };
  const selected =
    node.span && nav.hexSpan && node.span.start === nav.hexSpan.start && node.span.end === nav.hexSpan.end;

  return (
    <>
      <div className={classes.row} data-selected={selected || undefined} style={{ paddingLeft: depth * 16 }}>
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
          onClick={() => node.span && nav.showBytes(node.span)}
          title={node.span ? `bytes ${node.span.start}–${node.span.end}` : undefined}
        >
          <span className={classes.key}>{node.key !== null ? `/${node.key}` : `[${node.index}]`}</span>
          {v.type === "ref" ? (
            <Anchor
              component="span"
              size="xs"
              className={classes.value}
              onClick={(e) => {
                e.stopPropagation();
                nav.open({ kind: "object", id: v.id });
              }}
            >
              {refText(v.id)}
            </Anchor>
          ) : (
            <span className={classes.value} data-type={v.type}>
              {valueText(v)}
            </span>
          )}
        </UnstyledButton>
      </div>
      {open && expandable && <NodeChildren path={childPath} depth={depth + 1} />}
    </>
  );
}
