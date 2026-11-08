import { javascript } from "@codemirror/lang-javascript";
import { HighlightStyle, bracketMatching, codeFolding, foldGutter, foldKeymap, syntaxHighlighting } from "@codemirror/language";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import { EditorState } from "@codemirror/state";
import { EditorView, drawSelection, highlightActiveLineGutter, highlightSpecialChars, keymap, lineNumbers } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";
import { Alert, Anchor, Group, Loader, Stack, Text, useComputedColorScheme } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import { api, useBackend } from "../api";
import { theme } from "../content/cm";
import { objText } from "../format";
import { useNav } from "../nav";
import type { NodePath } from "../protocol/generated";

export const jsHighlight = HighlightStyle.define([
  { tag: [t.keyword, t.controlKeyword, t.moduleKeyword, t.operatorKeyword], color: "var(--mantine-color-violet-text)", fontWeight: "600" },
  { tag: [t.definitionKeyword, t.modifier], color: "var(--mantine-color-violet-text)" },
  { tag: [t.string, t.special(t.string)], color: "var(--mantine-color-orange-text)" },
  { tag: t.regexp, color: "var(--mantine-color-red-text)" },
  { tag: [t.number, t.bool, t.null, t.atom], color: "var(--mantine-color-teal-text)" },
  { tag: [t.lineComment, t.blockComment], color: "var(--mantine-color-dimmed)", fontStyle: "italic" },
  { tag: [t.function(t.variableName), t.function(t.propertyName)], color: "var(--mantine-color-blue-text)" },
  { tag: t.definition(t.variableName), color: "var(--mantine-color-cyan-text)" },
  { tag: [t.self, t.special(t.variableName)], color: "var(--mantine-color-pink-text)" },
  { tag: t.propertyName, color: "var(--mantine-color-text)" },
  { tag: [t.bracket, t.punctuation, t.separator], color: "var(--mantine-color-dimmed)" },
  { tag: t.invalid, color: "var(--mantine-color-red-text)", textDecoration: "underline wavy" },
]);

/** Read-only, highlighted JavaScript. */
function ScriptCode({ text, height }: { text: string; height: number | string }) {
  const host = useRef<HTMLDivElement>(null);
  const dark = useComputedColorScheme("light") === "dark";
  useEffect(() => {
    const v = new EditorView({
      state: EditorState.create({
        doc: text,
        extensions: [
          EditorState.readOnly.of(true),
          // Shown as stored: CR stays visible rather than splitting lines.
          EditorState.lineSeparator.of("\n"),
          highlightSpecialChars({ addSpecialChars: /[\r]/ }),
          drawSelection(),
          lineNumbers(),
          highlightActiveLineGutter(),
          codeFolding(),
          foldGutter(),
          bracketMatching(),
          highlightSelectionMatches(),
          search({ top: true }),
          keymap.of([...searchKeymap, ...foldKeymap]),
          javascript(),
          syntaxHighlighting(jsHighlight),
          EditorView.lineWrapping,
          theme,
          EditorView.darkTheme.of(dark),
        ],
      }),
      parent: host.current!,
    });
    return () => v.destroy();
  }, [text, dark]);
  return (
    <div
      ref={host}
      style={{ height, border: "1px solid var(--mantine-color-default-border)", borderRadius: 4, overflow: "hidden" }}
    />
  );
}

/** The script at `path` (a JavaScript action, or its /JS string or stream). */
export function ScriptView({ path, height = 480 }: { path: NodePath; height?: number | string }) {
  const backend = useBackend();
  const nav = useNav();
  const q = useQuery({
    queryKey: ["doc", nav.doc, "script", path],
    queryFn: () => api.script(backend, nav.doc, path),
  });
  if (q.error) return <Text size="xs" c="red">{String(q.error)}</Text>;
  if (!q.data) return <Loader size="sm" />;
  const s = q.data;
  // Shown on the stream itself, a link to it would lead nowhere.
  const own = path.root.kind === "object" && path.path.length === 0 && s.stream !== null && objText(path.root.id) === objText(s.stream);
  return (
    <Stack gap="xs">
      <Group gap="sm">
        <Text size="xs" c="dimmed">
          {s.text.length.toLocaleString()} characters · {s.stream ? "in a stream" : "a string"}
        </Text>
        {s.stream && !own && (
          <Anchor size="xs" onClick={() => nav.open({ kind: "object", id: s.stream! })}>
            {objText(s.stream)} →
          </Anchor>
        )}
      </Group>
      {s.truncated && (
        <Alert p={6} color="blue">
          <Text size="xs">Only the beginning of the script is shown.</Text>
        </Alert>
      )}
      {s.errors.map((e) => (
        <Alert key={e} p={6} color="red">
          <Text size="xs">{e}</Text>
        </Alert>
      ))}
      <ScriptCode text={s.text} height={height} />
    </Stack>
  );
}
