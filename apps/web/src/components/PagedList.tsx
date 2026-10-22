import { Loader, Text } from "@mantine/core";
import { useQueries } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useRef, type ReactNode } from "react";
import type { Page, Range } from "../protocol/generated";

const PAGE = 200;

interface Props<T> {
  total: number;
  queryKey: readonly unknown[];
  fetchPage: (range: Range, signal: AbortSignal) => Promise<Page<T>>;
  rowHeight?: number;
  renderRow: (item: T, index: number) => ReactNode;
  empty?: string;
}

/** Virtualized list whose rows are fetched page by page as they scroll into view. */
export function PagedList<T>({ total, queryKey, fetchPage, rowHeight = 26, renderRow, empty }: Props<T>) {
  const parent = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({
    count: total,
    getScrollElement: () => parent.current,
    estimateSize: () => rowHeight,
    overscan: 20,
  });
  const items = v.getVirtualItems();
  const pages = [...new Set(items.map((i) => Math.floor(i.index / PAGE)))];
  const results = useQueries({
    queries: pages.map((p) => ({
      queryKey: [...queryKey, PAGE, p],
      queryFn: ({ signal }: { signal: AbortSignal }) => fetchPage({ offset: p * PAGE, limit: PAGE }, signal),
    })),
  });
  const byPage = new Map(pages.map((p, k) => [p, results[k]?.data]));

  if (total === 0) {
    return (
      <Text c="dimmed" size="sm" p="sm">
        {empty ?? "Nothing here."}
      </Text>
    );
  }
  return (
    <div ref={parent} style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
      <div style={{ height: v.getTotalSize(), position: "relative" }}>
        {items.map((it) => {
          const page = byPage.get(Math.floor(it.index / PAGE));
          const item = page?.items[it.index - page.offset];
          return (
            <div
              key={it.key}
              style={{ position: "absolute", top: 0, left: 0, right: 0, height: rowHeight, transform: `translateY(${it.start}px)` }}
            >
              {item === undefined ? <Loader size={12} m={6} /> : renderRow(item, it.index)}
            </div>
          );
        })}
      </div>
    </div>
  );
}
