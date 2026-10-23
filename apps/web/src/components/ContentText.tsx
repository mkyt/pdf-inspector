import { foldAll, unfoldAll } from "@codemirror/language";
import { openSearchPanel } from "@codemirror/search";
import { EditorSelection } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import { Alert, Button, Group, Kbd, Loader, Text } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { api, useBackend } from "../api";
import { opIndexAt, opRange } from "../content/cm";
import { scan } from "../content/syntax";
import { useEdit } from "../edit";
import { contentKey, useNav } from "../nav";
import type { ContentRef, ContentSummary } from "../protocol/generated";
import { CodeEditor } from "./CodeEditor";
import { useResources } from "./OpEditor";

/**
 * Operations `from..to` of a content as text, edited with completion,
 * folding and problems shown as you type. Applying replaces them: the
 * backend compares the new operations with the old ones, so those left
 * alone keep their bytes. The cursor and the selected operation follow
 * each other.
 */
export function ContentText({
  content,
  s,
  from,
  to,
  op,
  onSelect,
  onClose,
}: {
  content: ContentRef;
  s: ContentSummary;
  from: number;
  to: number;
  op: number | null;
  onSelect: (i: number) => void;
  onClose: (select: number | null) => void;
}) {
  const backend = useBackend();
  const nav = useNav();
  const edit = useEdit();
  const resources = useResources(content);
  const [range, setRange] = useState({ from, to: Math.min(to, s.ops) });
  const q = useQuery({
    queryKey: ["doc", nav.doc, "content", ...contentKey(content), "text", range.from, range.to],
    queryFn: () => api.contentText(backend, nav.doc, content, { offset: range.from, limit: range.to - range.from }),
    staleTime: Infinity,
    gcTime: 0,
  });
  const [text, setText] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [version, setVersion] = useState(0);
  const view = useRef<EditorView | null>(null);
  /** The last operation the cursor selected (not to move the cursor back to it). */
  const fromCursor = useRef<number | null>(null);
  const t = q.data;
  const dirty = t !== undefined && text !== null && text !== t.text;
  const end = t?.to ?? range.to;

  // Selected elsewhere (the rendering, problems): put the cursor on it.
  useEffect(() => {
    const v = view.current;
    if (!v || op === null || op === fromCursor.current || op < range.from || op >= end) return;
    const r = opRange(v.state, op - range.from);
    if (r) v.dispatch({ selection: EditorSelection.cursor(r.from), scrollIntoView: true });
  }, [op, range.from, end, t]);

  const apply = async (value: string) => {
    if (!t) return;
    if (value === t.text) return;
    setBusy(true);
    setError(null);
    try {
      const count = scan(value).ops.length;
      await edit.apply(`Edit ${range.from === 0 && end === s.ops ? "the content" : `operations #${range.from}–#${end - 1}`} as text`, [
        { op: "editOps", content, edits: [{ kind: "replace", from: range.from, to: end, text: value }] },
      ]);
      // Go on editing the operations that replaced them.
      setText(null);
      setRange({ from: range.from, to: range.from + count });
      setVersion((x) => x + 1);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const close = () => {
    if (dirty && !window.confirm("Discard the changes to the text?")) return;
    onClose(fromCursor.current);
  };

  return (
    <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
      <Group gap={6} px={8} py={4} style={{ borderBottom: "1px solid var(--mantine-color-default-border)" }}>
        <Text size="xs" fw={600}>
          {range.from === 0 && end >= s.ops ? "All operations" : `Operations #${range.from}–#${end - 1}`}
        </Text>
        {t && t.to < range.to && (
          <Text size="xs" c="orange">
            only up to #{t.to - 1} (the text is limited to 4 MB)
          </Text>
        )}
        {dirty && (
          <Text size="xs" c="dimmed">
            · edited
          </Text>
        )}
        <span style={{ flex: 1 }} />
        <Button size="compact-xs" variant="subtle" onClick={() => view.current && foldAll(view.current)}>
          Fold all
        </Button>
        <Button size="compact-xs" variant="subtle" onClick={() => view.current && unfoldAll(view.current)}>
          Unfold
        </Button>
        <Button size="compact-xs" variant="subtle" onClick={() => view.current && openSearchPanel(view.current)}>
          Find
        </Button>
        <Button
          size="compact-xs"
          variant="subtle"
          disabled={!dirty || busy}
          onClick={() => {
            setText(null);
            setVersion((x) => x + 1);
          }}
        >
          Revert
        </Button>
        <Button size="compact-xs" loading={busy} disabled={!dirty} onClick={() => text !== null && void apply(text)}>
          Apply <Kbd size="xs" ml={4}>Ctrl/⌘+Enter</Kbd>
        </Button>
        <Button size="compact-xs" variant="default" onClick={close}>
          Close
        </Button>
      </Group>
      {error && (
        <Alert color="red" p={6} m={6} withCloseButton onClose={() => setError(null)}>
          <Text size="xs" style={{ whiteSpace: "pre-wrap" }}>
            {error}
          </Text>
        </Alert>
      )}
      {q.error && (
        <Alert color="red" p={6} m={6}>
          <Text size="xs">{String(q.error)}</Text>
        </Alert>
      )}
      {!t && !q.error && <Loader size="sm" m="sm" />}
      {t && (
        <CodeEditor
          key={`${version}-${range.from}-${range.to}-${t.text.length}`}
          initial={t.text}
          resources={resources}
          gutters
          autoFocus={version === 0}
          onView={(v) => (view.current = v)}
          onChange={setText}
          onSubmit={(value) => void apply(value)}
          onCursor={(v) => {
            if (v.state.selection.main.head === 0 && !v.hasFocus) return;
            const k = opIndexAt(v.state, v.state.selection.main.head);
            if (k < 0) return;
            const i = range.from + k;
            if (i === fromCursor.current || i >= s.ops) return;
            fromCursor.current = i;
            onSelect(i);
          }}
          style={{ flex: 1, overflow: "hidden" }}
        />
      )}
      <Text size="xs" c="dimmed" px={8} py={2} style={{ borderTop: "1px solid var(--mantine-color-default-border)" }}>
        Completion: <Kbd size="xs">Ctrl</Kbd>+<Kbd size="xs">Space</Kbd> (operators ranked by the operands before them, resource names after /, snippets) · fold
        q … Q, BT … ET, BDC … EMC in the gutter or with <Kbd size="xs">Ctrl</Kbd>+<Kbd size="xs">Shift</Kbd>+<Kbd size="xs">[</Kbd> · the operation at the cursor
        is selected in the rendering
      </Text>
    </div>
  );
}
