import { Alert, CloseButton, Stack, Text } from "@mantine/core";
import { useHotkeys } from "@mantine/hooks";
import { createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode } from "react";
import { api, useBackend } from "./api";
import type { ApplyResult, DocId, EditCommand, EditStatus } from "./protocol/generated";

/** Browser mode: the file each document was opened from or saved to, for "Save" without asking. */
export const fileHandles = new Map<DocId, FileSystemFileHandle>();

interface Message {
  id: number;
  color: "red" | "orange" | "blue";
  title: string;
  lines: string[];
}

export interface Edit {
  status: EditStatus;
  /** Apply commands as one undoable step. Rejects with the backend's error; reports notes and schema issues. */
  apply(label: string, commands: EditCommand[]): Promise<ApplyResult>;
  /** Like `apply`, but errors are reported instead of thrown. */
  run(label: string, commands: EditCommand[]): void;
  undo(): void;
  redo(): void;
  /** Show something about an edit or a save. */
  report(color: Message["color"], title: string, lines?: string[]): void;
  openSave(): void;
}

const EditContext = createContext<Edit | null>(null);

export function useEdit(): Edit {
  const e = useContext(EditContext);
  if (!e) throw new Error("no document");
  return e;
}

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** Shown messages fade after this long, unless they are errors. */
const MESSAGE_MS = 8000;
/** Schema issues listed in a message. */
const ISSUE_LINES = 6;

/**
 * Editing for one document. Changes reach the views through the backend's
 * `docChanged` event, which drops the document's cached answers.
 */
export function EditProvider({
  doc,
  status,
  onSave,
  children,
}: {
  doc: DocId;
  status: EditStatus;
  onSave: () => void;
  children: ReactNode;
}) {
  const backend = useBackend();
  const [messages, setMessages] = useState<Message[]>([]);
  const next = useRef(1);

  const report = useCallback((color: Message["color"], title: string, lines: string[] = []) => {
    const id = next.current++;
    setMessages((m) => [...m.slice(-3), { id, color, title, lines }]);
    if (color !== "red") setTimeout(() => setMessages((m) => m.filter((x) => x.id !== id)), MESSAGE_MS);
  }, []);

  const apply = useCallback(
    async (label: string, commands: EditCommand[]) => {
      const r = await api.apply(backend, doc, label, commands);
      if (r.notes.length > 0) report("blue", label, r.notes);
      if (r.issues.length > 0) {
        const lines = r.issues
          .slice(0, ISSUE_LINES)
          .map((i) => `${i.obj ? `${i.obj.num} ${i.obj.generation}: ` : ""}${i.path ? `${i.path}: ` : ""}${i.message}`);
        if (r.issues.length > ISSUE_LINES) lines.push(`… ${r.issues.length - ISSUE_LINES} more`);
        report("orange", `${label}: the changed objects have schema problems`, lines);
      }
      return r;
    },
    [backend, doc, report],
  );

  const run = useCallback(
    (label: string, commands: EditCommand[]) => {
      apply(label, commands).catch((e) => report("red", `${label} failed`, [errorText(e)]));
    },
    [apply, report],
  );

  const undo = useCallback(() => {
    if (!status.undo) return;
    api.undo(backend, doc).catch((e) => report("red", "Undo failed", [errorText(e)]));
  }, [backend, doc, status.undo, report]);
  const redo = useCallback(() => {
    if (!status.redo) return;
    api.redo(backend, doc).catch((e) => report("red", "Redo failed", [errorText(e)]));
  }, [backend, doc, status.redo, report]);

  useHotkeys([
    ["mod+Z", undo],
    ["mod+shift+Z", redo],
    ["mod+Y", redo],
    ["mod+S", onSave],
  ]);

  const value = useMemo(
    () => ({ status, apply, run, undo, redo, report, openSave: onSave }),
    [status, apply, run, undo, redo, report, onSave],
  );
  return (
    <EditContext.Provider value={value}>
      {children}
      {messages.length > 0 && (
        <Stack gap={6} style={{ position: "fixed", left: 12, bottom: 12, width: 420, zIndex: 300 }}>
          {messages.map((m) => (
            <Alert key={m.id} color={m.color} p={8} variant="filled" style={{ boxShadow: "var(--mantine-shadow-md)" }}>
              <div style={{ display: "flex", gap: 8 }}>
                <Stack gap={2} style={{ flex: 1, minWidth: 0 }}>
                  <Text size="xs" fw={600}>
                    {m.title}
                  </Text>
                  {m.lines.map((l, i) => (
                    <Text key={i} size="xs" style={{ wordBreak: "break-word" }}>
                      {l}
                    </Text>
                  ))}
                </Stack>
                <CloseButton
                  size="xs"
                  variant="transparent"
                  c="white"
                  aria-label="Dismiss"
                  onClick={() => setMessages((x) => x.filter((y) => y.id !== m.id))}
                />
              </div>
            </Alert>
          ))}
        </Stack>
      )}
    </EditContext.Provider>
  );
}

/** Each byte as the character with that code (Latin-1), so any data round-trips through text. */
export function latin1(data: Uint8Array): string {
  let s = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < data.length; i += CHUNK) s += String.fromCharCode(...data.subarray(i, i + CHUNK));
  return s;
}

/** Bytes as base64 (for stream data in JSON). */
export const base64 = (data: Uint8Array) => btoa(latin1(data));

/** The inverse of `latin1`; `null` if a character is beyond U+00FF. */
export function fromLatin1(s: string): Uint8Array | null {
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c > 0xff) return null;
    out[i] = c;
  }
  return out;
}
