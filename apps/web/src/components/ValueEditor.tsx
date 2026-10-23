import { Alert, Button, Group, Loader, Modal, Stack, Text, Textarea, TextInput } from "@mantine/core";
import { useEffect, useState } from "react";
import { api, useBackend } from "../api";
import { useEdit } from "../edit";
import { useNav } from "../nav";
import type { EditCommand, NodePath } from "../protocol/generated";

/** What the editor does with the value typed. */
export type ValueEdit =
  /** Replace the node (an empty path: the whole object or the trailer). */
  | { kind: "set"; path: NodePath; title: string }
  /** Add or replace an entry of the dictionary at `path`. */
  | { kind: "entry"; path: NodePath; title: string; key?: string; value?: string }
  /** Insert into the array at `path`. */
  | { kind: "insert"; path: NodePath; index: number; title: string }
  /** A new object. */
  | { kind: "new"; title: string };

/** Edit a value written in PDF syntax: `<< /Type /Example >>`, `[0 0 612 792]`, `(text)`, `12 0 R`. */
export function ValueEditor({ edit, onClose }: { edit: ValueEdit | null; onClose: () => void }) {
  return (
    <Modal opened={edit !== null} onClose={onClose} title={edit?.title} size="xl">
      {edit && <Form key={JSON.stringify(edit)} edit={edit} onClose={onClose} />}
    </Modal>
  );
}

function Form({ edit, onClose }: { edit: ValueEdit; onClose: () => void }) {
  const backend = useBackend();
  const nav = useNav();
  const { apply } = useEdit();
  const [key, setKey] = useState(edit.kind === "entry" ? (edit.key ?? "") : "");
  const [text, setText] = useState(edit.kind === "entry" ? (edit.value ?? "") : edit.kind === "new" ? "<<\n  \n>>" : "");
  const [loading, setLoading] = useState(edit.kind === "set");
  const [tooLarge, setTooLarge] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (edit.kind !== "set") return;
    let live = true;
    api.nodeSyntax(backend, nav.doc, edit.path).then(
      (r) => {
        if (!live) return;
        setText(r.text);
        setTooLarge(r.truncated);
        setLoading(false);
      },
      (e) => {
        if (!live) return;
        setError(String(e));
        setLoading(false);
      },
    );
    return () => {
      live = false;
    };
  }, [backend, nav.doc, edit]);

  const command = (): EditCommand => {
    switch (edit.kind) {
      case "set":
        return { op: "setValue", node: edit.path, value: text };
      case "entry":
        return { op: "setEntry", node: edit.path, key, value: text };
      case "insert":
        return { op: "insert", node: edit.path, index: edit.index, value: text };
      case "new":
        return { op: "addObject", value: text, data: null };
    }
  };

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await apply(edit.title, [command()]);
      onClose();
      if (edit.kind === "new" && r.created[0]) nav.open({ kind: "object", id: r.created[0] });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  if (loading) return <Loader size="sm" />;
  return (
    <Stack gap="xs">
      {edit.kind === "entry" && (
        <TextInput
          size="xs"
          label="Key"
          placeholder="/Name"
          ff="monospace"
          value={key}
          onChange={(e) => setKey(e.currentTarget.value)}
          autoFocus
        />
      )}
      <Textarea
        size="xs"
        label="Value (PDF syntax)"
        description={
          edit.kind === "set" && edit.path.path.length === 0
            ? "A stream given a dictionary keeps its data."
            : "References are written 12 0 R; strings (text) or <hex>."
        }
        styles={{ input: { fontFamily: "var(--mantine-font-family-monospace)", fontSize: 12 } }}
        autosize
        minRows={3}
        maxRows={24}
        value={text}
        onChange={(e) => setText(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void submit();
        }}
        disabled={tooLarge}
        // Mounted once the text has loaded, after the modal placed its focus.
        autoFocus={edit.kind !== "entry"}
      />
      {tooLarge && (
        <Text size="xs" c="orange">
          Too large to edit as text here; edit its entries one by one.
        </Text>
      )}
      {error && (
        <Alert color="red" p={6}>
          <Text size="xs" style={{ whiteSpace: "pre-wrap" }}>
            {error}
          </Text>
        </Alert>
      )}
      <Group justify="flex-end" gap="xs">
        <Text size="xs" c="dimmed" mr="auto">
          Ctrl/⌘+Enter applies. Undo with Ctrl/⌘+Z.
        </Text>
        <Button size="xs" variant="default" onClick={onClose}>
          Cancel
        </Button>
        <Button
          size="xs"
          loading={busy}
          disabled={tooLarge || text.trim() === "" || (edit.kind === "entry" && key.trim() === "")}
          onClick={() => void submit()}
        >
          Apply
        </Button>
      </Group>
    </Stack>
  );
}
