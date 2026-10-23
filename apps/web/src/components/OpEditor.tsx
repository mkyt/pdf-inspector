import { ActionIcon, Alert, Badge, Button, Group, Kbd, Loader, Text, Tooltip } from "@mantine/core";
import { useQueries, useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useCallback, useEffect, useMemo, useRef, useState, type ClipboardEvent, type KeyboardEvent, type ReactNode } from "react";
import { api, useBackend } from "../api";
import { categoryColor, scan } from "../content/syntax";
import { useEdit } from "../edit";
import { contentKey, useNav } from "../nav";
import type { ContentIssue, ContentRef, ContentSummary, OpEdit, OpView } from "../protocol/generated";
import { CodeEditor } from "./CodeEditor";
import { severityColor } from "./DiagnosticsPanel";
import rowClasses from "./Row.module.css";

const ROW = 22;
const PAGE = 200;

const rank = { info: 0, warning: 1, error: 2 } as const;

function worstOf(issues: ContentIssue[]) {
  let w: ContentIssue["severity"] | null = null;
  for (const i of issues) if (w === null || rank[i.severity] > rank[w]) w = i.severity;
  return w;
}

/** Folded blocks: opener → last operation. */
export type Folds = Map<number, number>;

interface Hidden {
  /** Hidden operations `a..=b`. */
  a: number;
  b: number;
  /** Operations hidden before `a`. */
  before: number;
}

/** Rows of the list with some blocks folded. */
export class Layout {
  readonly hidden: Hidden[] = [];
  readonly rows: number;

  constructor(folds: Folds, total: number) {
    let count = 0;
    let last = -1;
    for (const [o, e] of [...folds].sort((x, y) => x[0] - y[0])) {
      const a = o + 1;
      const b = Math.min(e, total - 1);
      if (o <= last || b < a) continue;
      this.hidden.push({ a, b, before: count });
      count += b - a + 1;
      last = b;
    }
    this.rows = total - count;
  }

  /** The last interval starting at or before `x` by `key`. */
  private find(x: number, key: (h: Hidden) => number): Hidden | undefined {
    let lo = 0;
    let hi = this.hidden.length - 1;
    let best: Hidden | undefined;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (key(this.hidden[mid]) <= x) {
        best = this.hidden[mid];
        lo = mid + 1;
      } else hi = mid - 1;
    }
    return best;
  }

  op(row: number): number {
    const h = this.find(row, (h) => h.a - h.before);
    return h ? row + h.before + (h.b - h.a + 1) : row;
  }

  /** The row showing `op` (its folded opener's when hidden). */
  row(op: number): number {
    const h = this.find(op, (h) => h.a);
    if (!h) return op;
    if (op <= h.b) return h.a - 1 - h.before;
    return op - h.before - (h.b - h.a + 1);
  }
}

/** Shift folds for an edit that put `count` operations in place of `from..to`. */
function remapFolds(folds: Folds, from: number, to: number, count: number): Folds {
  const d = count - (to - from);
  const out: Folds = new Map();
  for (const [o, e] of folds) {
    if (e < from) out.set(o, e);
    else if (o >= to) out.set(o + d, e + d);
    else if (o < from && e >= to) out.set(o, e + d);
  }
  return out;
}

interface Bar {
  /** Replace `from..to` (insert when equal). */
  from: number;
  to: number;
  title: string;
  text: string;
}

const label = (from: number, to: number) => (to - from === 1 ? `#${from}` : `#${from}–#${to - 1}`);

/**
 * The operations of a content as an editable list: blocks fold, a range
 * of operations is selected with Shift, and the keyboard edits them
 * (Enter edits, I inserts, Delete removes, Alt+↑/↓ moves, copy and paste
 * as text). Edits are written as text with completion in an edit bar.
 */
export function OpEditor({
  content,
  s,
  op,
  onSelect,
  scrollTo,
  onText,
}: {
  content: ContentRef;
  s: ContentSummary;
  op: number | null;
  onSelect: (i: number) => void;
  scrollTo: number | null;
  /** Open the text editor on operations `from..to`. */
  onText: (from: number, to: number) => void;
}) {
  const backend = useBackend();
  const nav = useNav();
  const edit = useEdit();
  const key = useMemo(() => ["doc", nav.doc, "content", ...contentKey(content)], [nav.doc, content]);
  const parent = useRef<HTMLDivElement>(null);
  const [folds, setFolds] = useState<Folds>(new Map());
  const [anchor, setAnchor] = useState<number | null>(op);
  const [bar, setBar] = useState<Bar | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** An edit of ours is being applied (it remaps the folds itself). */
  const ours = useRef(false);
  const total = s.ops;

  // Something else changed the document (undo, another view): the folded
  // operations may be elsewhere now.
  useEffect(
    () =>
      backend.onEvent((ev) => {
        if (ev.type === "docChanged" && ev.doc === nav.doc && !(ev.reason === "edit" && ours.current)) setFolds(new Map());
      }),
    [backend, nav.doc],
  );
  useEffect(() => {
    setFolds(new Map());
    setBar(null);
  }, [content]);

  const layout = useMemo(() => new Layout(folds, total), [folds, total]);
  const v = useVirtualizer({
    count: layout.rows,
    getScrollElement: () => parent.current,
    estimateSize: () => ROW,
    overscan: 20,
  });
  useEffect(() => {
    if (scrollTo !== null && scrollTo < total) v.scrollToIndex(layout.row(scrollTo), { align: "auto" });
  }, [scrollTo, total, v, layout]);

  const items = v.getVirtualItems();
  const rowOps = items.map((it) => layout.op(it.index));
  const pages = [...new Set(rowOps.map((o) => Math.floor(o / PAGE)))];
  const results = useQueries({
    queries: pages.map((p) => ({
      queryKey: [...key, "ops", PAGE, p],
      queryFn: ({ signal }: { signal: AbortSignal }) => api.contentOps(backend, nav.doc, content, { offset: p * PAGE, limit: PAGE }, signal),
    })),
  });
  const byPage = new Map(pages.map((p, k) => [p, results[k]?.data]));
  const opView = (i: number): OpView | undefined => {
    const page = byPage.get(Math.floor(i / PAGE));
    return page?.items[i - page.offset];
  };

  // The selection: anchor..focus, a folded block counting as a whole.
  const focus = op !== null && op < total ? op : null;
  const lo = focus === null ? null : Math.min(anchor ?? focus, focus);
  const hiOp = focus === null ? null : Math.max(anchor ?? focus, focus);
  const hi = hiOp === null ? null : (folds.get(hiOp) ?? hiOp);
  const selected = (i: number) => lo !== null && hi !== null && i >= lo && i <= hi;

  const pick = useCallback(
    (i: number, extend = false) => {
      if (!extend) setAnchor(i);
      else setAnchor((a) => a ?? op ?? i);
      onSelect(i);
    },
    [onSelect, op],
  );
  useEffect(() => {
    // Selected from elsewhere (rendering, problems): a single operation.
    if (op !== null && (anchor === null || !selected(op))) setAnchor(op);
  }, [op]);

  const toggle = (i: number, end: number) =>
    setFolds((f) => {
      const n = new Map(f);
      if (n.has(i)) n.delete(i);
      else n.set(i, end);
      return n;
    });

  const run = async (title: string, edits: OpEdit[], after: { select: number; folds: Folds }) => {
    setBusy(true);
    setError(null);
    ours.current = true;
    try {
      await edit.apply(title, [{ op: "editOps", content, edits }]);
      setFolds(after.folds);
      setAnchor(after.select);
      onSelect(after.select);
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return false;
    } finally {
      ours.current = false;
      setBusy(false);
    }
  };

  const replace = async (title: string, from: number, to: number, text: string) => {
    const count = scan(text).ops.length;
    return run(title, [{ kind: "replace", from, to, text }], {
      select: Math.min(from, Math.max(0, total - (to - from) + count - 1)),
      folds: remapFolds(folds, from, to, count),
    });
  };

  const textOf = async (from: number, to: number) => {
    const t = await api.contentText(backend, nav.doc, content, { offset: from, limit: to - from });
    if (t.to < to) throw new Error(`the operations are too long to edit here (${label(from, to)}); use the text editor`);
    return t.text;
  };

  const openEdit = async () => {
    if (lo === null || hi === null) return;
    try {
      setError(null);
      setBar({ from: lo, to: hi + 1, title: `Edit ${label(lo, hi + 1)}`, text: await textOf(lo, hi + 1) });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  const openInsert = (before: boolean) => {
    const at = lo === null || hi === null ? total : before ? lo : hi + 1;
    setError(null);
    setBar({ from: at, to: at, title: at >= total ? "Add at the end" : `Insert before #${at}`, text: "" });
  };
  const remove = () => {
    if (lo === null || hi === null) return;
    void replace(`Delete ${label(lo, hi + 1)}`, lo, hi + 1, "");
  };
  const move = (down: boolean) => {
    if (lo === null || hi === null) return;
    if (down) {
      const nextRow = layout.row(hi) + 1;
      if (nextRow >= layout.rows) return;
      const next = layout.op(nextRow);
      const end = folds.get(next) ?? next;
      const by = end - hi;
      void run(`Move ${label(lo, hi + 1)} down`, [{ kind: "move", from: lo, to: hi + 1, before: end + 1 }], {
        select: focus! + by,
        folds: moveFolds(folds, lo, hi + 1, end + 1),
      }).then((ok) => ok && setAnchor(lo + by + ((anchor ?? lo) - lo)));
    } else {
      const prevRow = layout.row(lo) - 1;
      if (prevRow < 0) return;
      const prev = layout.op(prevRow);
      const by = lo - prev;
      void run(`Move ${label(lo, hi + 1)} up`, [{ kind: "move", from: lo, to: hi + 1, before: prev }], {
        select: focus! - by,
        folds: moveFolds(folds, lo, hi + 1, prev),
      }).then((ok) => ok && setAnchor((anchor ?? lo) - by));
    }
  };
  const wrap = () => {
    if (lo === null || hi === null) return;
    void run(
      `Wrap ${label(lo, hi + 1)} in q … Q`,
      [
        { kind: "replace", from: hi + 1, to: hi + 1, text: "Q" },
        { kind: "replace", from: lo, to: lo, text: "q" },
      ],
      { select: lo, folds: remapFolds(remapFolds(folds, hi + 1, hi + 1, 1), lo, lo, 1) },
    );
  };
  const duplicate = async () => {
    if (lo === null || hi === null) return;
    try {
      const text = await textOf(lo, hi + 1);
      await replace(`Duplicate ${label(lo, hi + 1)}`, hi + 1, hi + 1, `\n${text}`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  const copy = async (cut: boolean) => {
    if (lo === null || hi === null) return;
    try {
      const text = await textOf(lo, hi + 1);
      await navigator.clipboard.writeText(text);
      if (cut) await replace(`Cut ${label(lo, hi + 1)}`, lo, hi + 1, "");
      else edit.report("blue", `Copied ${label(lo, hi + 1)} as text`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  const onPaste = (e: ClipboardEvent<HTMLDivElement>) => {
    const text = e.clipboardData.getData("text/plain");
    if (!text || bar) return;
    e.preventDefault();
    const at = hi === null ? total : hi + 1;
    void replace(`Paste before #${at}`, at, at, text);
  };
  const foldAll = async (fold: boolean) => {
    if (!fold) return setFolds(new Map());
    const r = await api.contentBlocks(backend, nav.doc, content, 0);
    setFolds(new Map(r.blocks.filter(([a, b]) => b > a)));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (busy || total === 0) return;
    const mod = e.metaKey || e.ctrlKey;
    const step = (d: number) => {
      const row = focus === null ? 0 : layout.row(focus);
      const r = Math.max(0, Math.min(layout.rows - 1, focus === null ? 0 : row + d));
      pick(layout.op(r), e.shiftKey);
    };
    const k = e.key;
    let handled = true;
    if (e.altKey && (k === "ArrowUp" || k === "ArrowDown")) move(k === "ArrowDown");
    else if (k === "ArrowDown") step(1);
    else if (k === "ArrowUp") step(-1);
    else if (k === "PageDown") step(20);
    else if (k === "PageUp") step(-20);
    else if (k === "Home") pick(0, e.shiftKey);
    else if (k === "End") pick(layout.op(layout.rows - 1), e.shiftKey);
    else if ((k === "ArrowLeft" || k === "ArrowRight") && focus !== null) {
      const end = folds.get(focus) ?? opView(focus)?.end;
      if (end !== undefined && end !== null && end > focus && folds.has(focus) === (k === "ArrowRight")) toggle(focus, end);
    } else if (k === "Enter") void openEdit();
    else if ((k === "i" || k === "I" || k === "Insert") && !mod) openInsert(e.shiftKey);
    else if (k === "Delete" || k === "Backspace") remove();
    else if (mod && k === "c") void copy(false);
    else if (mod && k === "x") void copy(true);
    else if (mod && k === "d") void duplicate();
    else if (mod && k === "a") {
      setAnchor(0);
      onSelect(total - 1);
    } else if ((k === "t" || k === "T") && !mod && lo !== null && hi !== null) onText(lo, hi + 1);
    else handled = false;
    if (handled) e.preventDefault();
  };

  const selection = lo !== null && hi !== null ? `${(hi - lo + 1).toLocaleString()} selected` : "nothing selected";
  return (
    <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
      <Group gap={4} px={8} py={4} wrap="wrap" style={{ borderBottom: "1px solid var(--mantine-color-default-border)" }}>
        <Tool label="Insert after the selection" keys={["I"]} onClick={() => openInsert(false)} disabled={busy}>
          + Insert
        </Tool>
        <Tool label="Edit the selection as text" keys={["Enter"]} onClick={() => void openEdit()} disabled={busy || lo === null}>
          Edit
        </Tool>
        <Tool label="Delete the selection" keys={["Delete"]} onClick={remove} disabled={busy || lo === null}>
          Delete
        </Tool>
        <Tool label="Move up" keys={["Alt", "↑"]} onClick={() => move(false)} disabled={busy || lo === null || lo === 0}>
          ↑
        </Tool>
        <Tool label="Move down" keys={["Alt", "↓"]} onClick={() => move(true)} disabled={busy || hi === null || hi >= total - 1}>
          ↓
        </Tool>
        <Tool label="Wrap in q … Q (save and restore the graphics state around it)" onClick={wrap} disabled={busy || lo === null}>
          q … Q
        </Tool>
        <Tool label="Duplicate" keys={["Ctrl/⌘", "D"]} onClick={() => void duplicate()} disabled={busy || lo === null}>
          Duplicate
        </Tool>
        <Tool
          label="Open the selection (or everything) in the text editor"
          keys={["T"]}
          onClick={() => (lo !== null && hi !== null && hi > lo ? onText(lo, hi + 1) : onText(0, total))}
          disabled={busy}
        >
          Text…
        </Tool>
        <span style={{ flex: 1 }} />
        <Tool label="Fold every top-level block" onClick={() => void foldAll(true)} disabled={total === 0}>
          Fold all
        </Tool>
        <Tool label="Unfold every block" onClick={() => void foldAll(false)} disabled={folds.size === 0}>
          Unfold
        </Tool>
        <Text size="xs" c="dimmed">
          {selection}
        </Text>
        {busy && <Loader size={12} />}
      </Group>
      {error && (
        <Alert color="red" p={6} m={6} withCloseButton onClose={() => setError(null)}>
          <Text size="xs" style={{ whiteSpace: "pre-wrap" }}>
            {error}
          </Text>
        </Alert>
      )}
      {bar && (
        <EditBar
          key={`${bar.from}-${bar.to}-${bar.title}`}
          content={content}
          bar={bar}
          busy={busy}
          onCancel={() => {
            setBar(null);
            parent.current?.focus();
          }}
          onApply={async (text) => {
            if (await replace(bar.title, bar.from, bar.to, text)) {
              setBar(null);
              parent.current?.focus();
            }
          }}
        />
      )}
      {total === 0 ? (
        <Text c="dimmed" size="sm" p="sm">
          No operations: the content is empty. Insert some (I) to start it.
        </Text>
      ) : (
        <div
          ref={parent}
          style={{ flex: 1, minHeight: 0, overflow: "auto", outline: "none" }}
          tabIndex={0}
          onKeyDown={onKeyDown}
          onPaste={onPaste}
        >
          <div style={{ height: v.getTotalSize(), position: "relative" }}>
            {items.map((it, k) => {
              const i = rowOps[k];
              const o = opView(i);
              return (
                <div
                  key={it.key}
                  style={{ position: "absolute", top: 0, left: 0, right: 0, height: ROW, transform: `translateY(${it.start}px)` }}
                >
                  {o === undefined ? (
                    <Loader size={12} m={6} />
                  ) : (
                    <OpRow
                      o={o}
                      active={i === focus}
                      selected={selected(i)}
                      folded={folds.get(i)}
                      onFold={o.end !== null && o.end > i ? () => toggle(i, o.end!) : undefined}
                      onClick={(extend) => {
                        pick(i, extend);
                        parent.current?.focus();
                      }}
                      onDoubleClick={() => {
                        pick(i);
                        setTimeout(() => void openEdit(), 0);
                      }}
                    />
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

/** Folds after moving `from..to` before `before`. */
function moveFolds(folds: Folds, from: number, to: number, before: number): Folds {
  const n = to - from;
  const at = before > from ? before - n : before;
  const map = (i: number) => {
    if (i >= from && i < to) return at + (i - from);
    const rest = i >= to ? i - n : i;
    return rest >= at ? rest + n : rest;
  };
  const out: Folds = new Map();
  for (const [o, e] of folds) {
    const a = map(o);
    const b = map(e);
    // Keep a fold only when its block stays in one piece.
    if (b - a === e - o) out.set(a, b);
  }
  return out;
}

function Tool({
  label,
  keys,
  onClick,
  disabled,
  children,
}: {
  label: string;
  keys?: string[];
  onClick: () => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <Tooltip
      label={
        <span>
          {label}
          {keys && (
            <>
              {" "}
              {keys.map((k) => (
                <Kbd key={k} size="xs" mx={1}>
                  {k}
                </Kbd>
              ))}
            </>
          )}
        </span>
      }
    >
      <Button size="compact-xs" variant="subtle" onClick={onClick} disabled={disabled}>
        {children}
      </Button>
    </Tooltip>
  );
}

/** Where an edit is typed: content stream text with completion. */
function EditBar({
  content,
  bar,
  busy,
  onApply,
  onCancel,
}: {
  content: ContentRef;
  bar: Bar;
  busy: boolean;
  onApply: (text: string) => void;
  onCancel: () => void;
}) {
  const resources = useResources(content);
  const [text, setText] = useState(bar.text);
  return (
    <div style={{ padding: "6px 8px", borderBottom: "1px solid var(--mantine-color-default-border)", background: "var(--mantine-color-default-hover)" }}>
      <Group gap={6} mb={4}>
        <Text size="xs" fw={600}>
          {bar.title}
        </Text>
        <Text size="xs" c="dimmed">
          Content stream syntax; completion with <Kbd size="xs">Ctrl</Kbd>+<Kbd size="xs">Space</Kbd>. <Kbd size="xs">Enter</Kbd> applies,{" "}
          <Kbd size="xs">Shift</Kbd>+<Kbd size="xs">Enter</Kbd> starts a line, <Kbd size="xs">Esc</Kbd> cancels.
        </Text>
      </Group>
      <div style={{ display: "flex", gap: 6, alignItems: "flex-start" }}>
        <CodeEditor
          initial={bar.text}
          resources={resources}
          autoFocus
          placeholder="e.g. 1 0 0 rg 72 72 144 72 re f"
          onChange={setText}
          onSubmit={(t) => !busy && onApply(t)}
          onCancel={onCancel}
          style={{
            flex: 1,
            maxHeight: 200,
            overflow: "auto",
            border: "1px solid var(--mantine-color-default-border)",
            borderRadius: 4,
            background: "var(--mantine-color-body)",
          }}
        />
        <Button size="compact-xs" loading={busy} onClick={() => onApply(text)} disabled={bar.from === bar.to && text.trim() === ""}>
          Apply
        </Button>
        <ActionIcon size="sm" variant="subtle" onClick={onCancel} aria-label="Cancel">
          ✕
        </ActionIcon>
      </div>
    </div>
  );
}

/** The resource names a content can use (for completion). */
export function useResources(content: ContentRef) {
  const backend = useBackend();
  const nav = useNav();
  const q = useQuery({
    queryKey: ["doc", nav.doc, "content", ...contentKey(content), "resources"],
    queryFn: () => api.contentResources(backend, nav.doc, content),
    staleTime: Infinity,
  });
  return q.data?.items ?? [];
}

function OpRow({
  o,
  active,
  selected,
  folded,
  onFold,
  onClick,
  onDoubleClick,
}: {
  o: OpView;
  active: boolean;
  selected: boolean;
  /** Last operation of the folded block it opens. */
  folded: number | undefined;
  onFold?: () => void;
  onClick: (extend: boolean) => void;
  onDoubleClick: () => void;
}) {
  const w = worstOf(o.issues);
  const color = categoryColor[o.category] ?? "gray";
  return (
    <div
      className={rowClasses.row}
      data-active={active || selected || undefined}
      style={active ? { boxShadow: "inset 2px 0 0 var(--mantine-primary-color-filled)" } : undefined}
      onMouseDown={(e) => {
        if (e.shiftKey) e.preventDefault();
      }}
      onClick={(e) => onClick(e.shiftKey)}
      onDoubleClick={onDoubleClick}
    >
      <Text span size="xs" c="dimmed" className={rowClasses.mono} w={56} ta="right" style={{ flexShrink: 0 }}>
        {o.index}
      </Text>
      <span style={{ width: Math.min(o.depth, 24) * 12, flexShrink: 0 }} />
      <span
        style={{ width: 12, flexShrink: 0, cursor: onFold ? "pointer" : undefined, color: "var(--mantine-color-dimmed)" }}
        onClick={(e) => {
          if (!onFold) return;
          e.stopPropagation();
          onFold();
        }}
      >
        {onFold ? (folded !== undefined ? "▸" : "▾") : ""}
      </span>
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
      {folded !== undefined && (
        <Badge size="xs" variant="light" color="gray" tt="none" style={{ flexShrink: 0, cursor: "pointer" }} onClick={onFold}>
          … {(folded - o.index).toLocaleString()} operations to #{folded}
        </Badge>
      )}
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
    </div>
  );
}
