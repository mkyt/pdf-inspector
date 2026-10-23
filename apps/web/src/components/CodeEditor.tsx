import { Compartment, EditorState, Prec, type Extension } from "@codemirror/state";
import { EditorView, keymap, placeholder as placeholderExt } from "@codemirror/view";
import { useComputedColorScheme } from "@mantine/core";
import { useEffect, useRef } from "react";
import { contentExtensions } from "../content/cm";
import type { ResourceItem } from "../protocol/generated";

interface Props {
  /** The text it starts with (remount with a new `key` to load another). */
  initial: string;
  resources: ResourceItem[];
  /** Line numbers, fold and problem gutters. */
  gutters?: boolean;
  placeholder?: string;
  autoFocus?: boolean;
  onChange?: (text: string) => void;
  /** The cursor moved (or the selection changed). */
  onCursor?: (view: EditorView) => void;
  /** Ctrl/⌘+Enter, and Enter without `gutters` (Shift+Enter starts a line). */
  onSubmit?: (text: string) => void;
  onCancel?: () => void;
  onView?: (view: EditorView | null) => void;
  style?: React.CSSProperties;
}

/** A CodeMirror editor for content stream text. */
export function CodeEditor({ initial, resources, gutters = false, placeholder, autoFocus, onChange, onCursor, onSubmit, onCancel, onView, style }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const dark = useComputedColorScheme("light") === "dark";
  const themeSlot = useRef(new Compartment());
  // Callbacks change with every render; the editor reads the latest.
  const cb = useRef({ onChange, onCursor, onSubmit, onCancel, resources });
  cb.current = { onChange, onCursor, onSubmit, onCancel, resources };

  useEffect(() => {
    const submit = (v: EditorView) => {
      if (!cb.current.onSubmit) return false;
      cb.current.onSubmit(v.state.doc.toString());
      return true;
    };
    const keys = [
      { key: "Mod-Enter", run: submit },
      { key: "Escape", run: () => (cb.current.onCancel ? (cb.current.onCancel(), true) : false) },
      ...(gutters ? [] : [{ key: "Enter", run: submit }]),
    ];
    const extensions: Extension[] = [
      Prec.high(keymap.of(keys)),
      themeSlot.current.of(contentExtensions({ resources: () => cb.current.resources, gutters, dark })),
      EditorView.updateListener.of((u) => {
        if (u.docChanged) cb.current.onChange?.(u.state.doc.toString());
        if (u.selectionSet || u.docChanged) cb.current.onCursor?.(u.view);
      }),
    ];
    if (placeholder) extensions.push(placeholderExt(placeholder));
    if (!gutters) extensions.push(EditorView.lineWrapping);
    const v = new EditorView({ state: EditorState.create({ doc: initial, extensions }), parent: host.current! });
    view.current = v;
    onView?.(v);
    if (autoFocus) {
      v.focus();
      v.dispatch({ selection: { anchor: v.state.doc.length } });
    }
    return () => {
      onView?.(null);
      v.destroy();
      view.current = null;
    };
    // The editor is made once; a new text comes with a new key.
  }, []);

  useEffect(() => {
    view.current?.dispatch({
      effects: themeSlot.current.reconfigure(contentExtensions({ resources: () => cb.current.resources, gutters, dark })),
    });
  }, [dark, gutters]);

  return <div ref={host} style={{ minHeight: 0, ...style }} />;
}
