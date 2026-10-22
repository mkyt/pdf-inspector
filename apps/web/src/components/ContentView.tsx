import { ActionIcon, Alert, Anchor, Badge, Button, Collapse, Group, Loader, Stack, Text, Title, Tooltip } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { useCallback, useState, type KeyboardEvent } from "react";
import { api, useBackend } from "../api";
import { useDoc } from "../doc";
import { bytesText, objText, pageText } from "../format";
import { contentKey, useNav } from "../nav";
import type { ContentIssue, ContentRef, ContentSummary, OpView } from "../protocol/generated";
import { ContentDiagram } from "./ContentDiagram";
import { severityColor } from "./DiagnosticsPanel";
import { ObjLink } from "./DocumentPanel";
import { OpStatePanel } from "./OpStatePanel";
import { PagedList } from "./PagedList";
import { Row } from "./Row";
import rowClasses from "./Row.module.css";

const ROW = 22;
const ISSUE_PAGE = 500;

/** Color of each operator group, as a Mantine color name. */
export const categoryColor: Record<string, string> = {
  generalState: "grape",
  specialState: "red",
  pathConstruction: "blue",
  pathPainting: "indigo",
  clipping: "cyan",
  textObject: "orange",
  textState: "yellow",
  textPositioning: "lime",
  textShowing: "orange",
  type3: "pink",
  color: "teal",
  shading: "pink",
  inlineImage: "violet",
  xObject: "violet",
  markedContent: "gray",
  compatibility: "gray",
  unknown: "red",
};

export const categoryLabel: Record<string, string> = {
  generalState: "general graphics state",
  specialState: "q / Q / cm",
  pathConstruction: "path construction",
  pathPainting: "path painting",
  clipping: "clipping",
  textObject: "BT / ET",
  textState: "text state",
  textPositioning: "text positioning",
  textShowing: "text showing",
  type3: "Type 3",
  color: "color",
  shading: "shading",
  inlineImage: "inline image",
  xObject: "XObject",
  markedContent: "marked content",
  compatibility: "BX / EX",
  unknown: "unknown",
};

const rank = { info: 0, warning: 1, error: 2 } as const;

function worstOf(issues: ContentIssue[]) {
  let w: ContentIssue["severity"] | null = null;
  for (const i of issues) if (w === null || rank[i.severity] > rank[w]) w = i.severity;
  return w;
}

export function contentTitle(c: ContentRef, s?: ContentSummary, label?: string | null): string {
  if (c.kind === "page") return `Page ${pageText(c.index, label)} — content`;
  const kind = s ? { page: "page", form: "form XObject", pattern: "tiling pattern", other: "stream" }[s.kind] : "stream";
  return `${objText(c.id)} — ${kind} content`;
}

/**
 * The operations of a content (a page's /Contents or a stream) with their
 * problems, what each paints, and the graphics state around the selected one.
 */
export function ContentView({ content, op }: { content: ContentRef; op: number | null }) {
  const backend = useBackend();
  const nav = useNav();
  const { summary: doc } = useDoc();
  const key = ["doc", nav.doc, "content", ...contentKey(content)];
  const q = useQuery({ queryKey: key, queryFn: () => api.content(backend, nav.doc, content) });
  const select = useCallback((i: number | null) => nav.update({ kind: "content", content, op: i }), [nav, content]);
  const [scrollTo, setScrollTo] = useState<number | null>(op);
  // Selecting from elsewhere (diagram, issues) also scrolls the list.
  const jump = (i: number) => {
    select(i);
    setScrollTo(i);
  };
  const s = q.data;

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (!s || s.ops === 0) return;
    const d = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : e.key === "PageDown" ? 20 : e.key === "PageUp" ? -20 : 0;
    if (!d) return;
    e.preventDefault();
    jump(Math.max(0, Math.min(s.ops - 1, (op ?? -1) + d)));
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <Stack px="sm" pt="sm" pb={6} gap={6} style={{ borderBottom: "1px solid var(--mantine-color-default-border)" }}>
        <Group gap="sm">
          {content.kind === "page" && (
            <Tooltip label="Previous page">
              <ActionIcon
                size="sm"
                variant="subtle"
                disabled={content.index === 0}
                onClick={() => nav.open({ kind: "content", content: { kind: "page", index: content.index - 1 }, op: null })}
                aria-label="Previous page"
              >
                ‹
              </ActionIcon>
            </Tooltip>
          )}
          <Title order={5}>{contentTitle(content, s)}</Title>
          {content.kind === "page" && (
            <Tooltip label="Next page">
              <ActionIcon
                size="sm"
                variant="subtle"
                disabled={content.index + 1 >= doc.pages}
                onClick={() => nav.open({ kind: "content", content: { kind: "page", index: content.index + 1 }, op: null })}
                aria-label="Next page"
              >
                ›
              </ActionIcon>
            </Tooltip>
          )}
          {content.kind === "page" ? (
            <Button size="compact-xs" variant="light" onClick={() => nav.open({ kind: "page", index: content.index })}>
              Page details
            </Button>
          ) : (
            <Button size="compact-xs" variant="light" onClick={() => nav.open({ kind: "object", id: content.id })}>
              {objText(content.id)}
            </Button>
          )}
        </Group>
        {q.error && (
          <Alert color="red" p={6}>
            <Text size="xs">{String(q.error)}</Text>
          </Alert>
        )}
        {!s && !q.error && <Loader size="sm" />}
        {s && <Overview s={s} />}
        {s && s.issues > 0 && <Issues content={content} s={s} onPick={jump} />}
      </Stack>
      {s && (
        <div style={{ flex: 1, minHeight: 0, display: "flex" }}>
          <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
            <PagedList<OpView>
              total={s.ops}
              rowHeight={ROW}
              empty="No operations: the content is empty."
              queryKey={[...key, "ops"]}
              fetchPage={(r, signal) => api.contentOps(backend, nav.doc, content, r, signal)}
              scrollTo={scrollTo}
              onKeyDown={onKeyDown}
              renderRow={(o) => <OpRow o={o} active={o.index === op} onClick={() => select(o.index)} />}
            />
          </div>
          <div
            style={{
              width: 420,
              flexShrink: 0,
              borderLeft: "1px solid var(--mantine-color-default-border)",
              overflow: "auto",
            }}
          >
            <ContentDiagram content={content} summary={s} op={op} onPick={jump} />
            {op !== null && op < s.ops ? (
              <OpStatePanel key={op} content={content} summary={s} index={op} />
            ) : (
              <Text size="xs" c="dimmed" p="sm">
                Select an operation to see the graphics state it runs in.
              </Text>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function Overview({ s }: { s: ContentSummary }) {
  return (
    <Stack gap={4}>
      <Group gap={6}>
        <Badge variant="light" color="gray" tt="none">
          {s.ops.toLocaleString()} operations
        </Badge>
        <Badge variant="light" color="blue" tt="none">
          {s.elements.toLocaleString()} painting
        </Badge>
        {s.errors > 0 && (
          <Badge variant="light" color="red" tt="none">
            {s.errors.toLocaleString()} errors
          </Badge>
        )}
        {s.warnings > 0 && (
          <Badge variant="light" color="orange" tt="none">
            {s.warnings.toLocaleString()} warnings
          </Badge>
        )}
        {s.truncated && (
          <Badge variant="light" color="red" tt="none">
            truncated
          </Badge>
        )}
        {s.approximate && (
          <Tooltip label="Some fonts lack widths for the codes shown (standard 14 fonts outside ASCII, predefined CJK encodings): their boxes are estimates." multiline maw={320}>
            <Badge variant="outline" color="gray" tt="none">
              approximate text boxes
            </Badge>
          </Tooltip>
        )}
        <Text size="xs" c="dimmed">
          Resources:{" "}
          {s.resources ? <ObjLink id={s.resources} /> : s.has_resources ? "direct" : <Text span size="xs" c="orange">none</Text>}
        </Text>
      </Group>
      <Group gap={10}>
        {s.sources.map((src, i) => (
          <Group key={i} gap={4}>
            {s.sources.length > 1 && (
              <Text size="xs" c="dimmed">
                [{i}]
              </Text>
            )}
            {src.id ? <ObjLink id={src.id} /> : <Text size="xs">direct</Text>}
            <Text size="xs" c={src.error ? "red" : "dimmed"}>
              {src.error ?? `${bytesText(src.len)} decoded`}
            </Text>
          </Group>
        ))}
      </Group>
      <Group gap={4}>
        {s.counts.map((c) => (
          <Badge key={c.category} size="xs" variant="dot" color={categoryColor[c.category]} tt="none">
            {categoryLabel[c.category] ?? c.category} {c.count.toLocaleString()}
          </Badge>
        ))}
      </Group>
    </Stack>
  );
}

function Issues({ content, s, onPick }: { content: ContentRef; s: ContentSummary; onPick: (op: number) => void }) {
  const backend = useBackend();
  const nav = useNav();
  const [open, setOpen] = useState(s.errors + s.warnings > 0);
  const q = useQuery({
    queryKey: ["doc", nav.doc, "content", ...contentKey(content), "issues"],
    queryFn: () => api.contentIssues(backend, nav.doc, content, { offset: 0, limit: ISSUE_PAGE }),
    enabled: open,
  });
  return (
    <Stack gap={2}>
      <Anchor size="xs" onClick={() => setOpen(!open)}>
        {open ? "▾" : "▸"} {s.issues.toLocaleString()} problem{s.issues === 1 ? "" : "s"}
      </Anchor>
      <Collapse expanded={open}>
        <div style={{ maxHeight: 160, overflow: "auto" }}>
          {q.data?.items.map((i, k) => (
            <Row key={k} onClick={i.op !== null ? () => onPick(i.op!) : undefined}>
              <Badge size="xs" color={severityColor[i.severity]} variant="light" style={{ flexShrink: 0 }}>
                {i.code}
              </Badge>
              <Text size="xs" ff="monospace" c="dimmed" w={56} style={{ flexShrink: 0 }}>
                {i.op === null ? "—" : `#${i.op}`}
              </Text>
              <Text size="xs" truncate>
                {i.message}
              </Text>
            </Row>
          ))}
          {q.data && q.data.total > q.data.items.length && (
            <Text size="xs" c="dimmed" px={10}>
              … and {(q.data.total - q.data.items.length).toLocaleString()} more (see the operations).
            </Text>
          )}
          {s.issues_dropped > 0 && (
            <Text size="xs" c="dimmed" px={10}>
              {s.issues_dropped.toLocaleString()} more were not kept.
            </Text>
          )}
        </div>
      </Collapse>
    </Stack>
  );
}

function OpRow({ o, active, onClick }: { o: OpView; active: boolean; onClick: () => void }) {
  const w = worstOf(o.issues);
  const color = categoryColor[o.category] ?? "gray";
  return (
    <Row active={active} onClick={onClick}>
      <Text span size="xs" c="dimmed" className={rowClasses.mono} w={56} ta="right" style={{ flexShrink: 0 }}>
        {o.index}
      </Text>
      <span style={{ width: Math.min(o.depth, 24) * 12, flexShrink: 0 }} />
      <Text
        span
        size="xs"
        fw={600}
        className={rowClasses.mono}
        style={{ color: `var(--mantine-color-${color}-text)`, minWidth: 28, flexShrink: 0 }}
      >
        {o.operator}
      </Text>
      <Text span size="xs" className={rowClasses.mono} truncate style={{ flex: "0 1 auto", minWidth: 0 }}>
        {o.operands.join(" ")}
      </Text>
      {o.text !== null && (
        <Text span size="xs" c="orange.8" truncate style={{ flex: "0 1 auto", minWidth: 40 }}>
          “{o.text}”
        </Text>
      )}
      <span style={{ marginLeft: "auto", display: "flex", gap: 4, flexShrink: 0 }}>
        {o.element && (
          <Badge size="xs" variant={o.element.hidden ? "outline" : "light"} color="gray" tt="none">
            {o.element.kind}
            {o.element.hidden && " (hidden)"}
          </Badge>
        )}
        {w && (
          <Tooltip label={o.issues.map((i) => `${i.code}: ${i.message}`).join("\n")} multiline maw={360} style={{ whiteSpace: "pre-line" }}>
            <Badge size="xs" color={severityColor[w]} variant="filled">
              {o.issues.length > 1 ? `${o.issues.length} ` : ""}
              {w}
            </Badge>
          </Tooltip>
        )}
      </span>
    </Row>
  );
}
