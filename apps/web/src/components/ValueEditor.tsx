import {
  Alert,
  Autocomplete,
  Button,
  Group,
  Loader,
  Modal,
  Stack,
  Text,
  Textarea,
  type ComboboxItem,
  type ComboboxParsedItem,
  type OptionsFilter,
} from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useState, type KeyboardEvent } from "react";
import { api, useBackend } from "../api";
import { useEdit } from "../edit";
import { useNav } from "../nav";
import type { AllowedValue, EditCommand, KeyDoc, NodePath, TypeKey } from "../protocol/generated";
import { same } from "./SchemaPanel";

/** What the editor does with the value typed. */
export type ValueEdit =
  /**
   * Replace the node (an empty path: the whole object or the trailer).
   * `doc`: what the schema says about the entry, to complete its value.
   */
  | { kind: "set"; path: NodePath; title: string; doc?: KeyDoc | null }
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
  // Keys of the dictionary an entry is added to, to complete the key and its value.
  const entryPath = edit.kind === "entry" ? edit.path : null;
  const keys = useQuery({
    queryKey: ["doc", nav.doc, "keys", entryPath],
    queryFn: () => api.keys(backend, nav.doc, entryPath!),
    enabled: entryPath !== null,
  });
  const keyList = (keys.data?.keys ?? []).filter((k) => !k.key.includes("*"));
  const doc = edit.kind === "set" ? edit.doc : keyDoc(keys.data?.keys ?? [], key);
  const values = doc ? candidates(doc) : [];
  // A value written on several lines is not one of them: edited as text.
  const complete = values.length > 0 && !text.trim().includes("\n");

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

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void submit();
  };

  if (loading) return <Loader size="sm" />;
  return (
    <Stack gap="xs">
      {edit.kind === "entry" && (
        <Autocomplete
          size="xs"
          label="Key"
          placeholder="/Name"
          description={keys.data?.type_name ? `Keys of ${keys.data.type_name}.` : undefined}
          styles={{ input: { fontFamily: "var(--mantine-font-family-monospace)" } }}
          data={keyOptions(keyList)}
          filter={filter}
          limit={500}
          maxDropdownHeight={280}
          renderOption={({ option }) => <KeyOption k={keyList.find((k) => `/${k.key}` === option.value)} />}
          value={key}
          onChange={setKey}
          onKeyDown={onKeyDown}
          autoFocus
        />
      )}
      {complete && doc ? (
        <Autocomplete
          size="xs"
          label="Value (PDF syntax)"
          description={
            doc.allowed_open || doc.constraints.length > 0
              ? "Values the schema lists; others are possible too."
              : "Values the schema lists."
          }
          styles={{ input: { fontFamily: "var(--mantine-font-family-monospace)", fontSize: 12 } }}
          data={values.map((v) => v.value)}
          filter={filter}
          limit={500}
          maxDropdownHeight={280}
          renderOption={({ option }) => (
            <ValueOption v={values.find((v) => v.value === option.value)} keySince={doc.since} />
          )}
          value={text}
          onChange={setText}
          onKeyDown={onKeyDown}
          disabled={tooLarge}
          autoFocus={edit.kind !== "entry"}
        />
      ) : (
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
          onKeyDown={onKeyDown}
          disabled={tooLarge}
          // Mounted once the text has loaded, after the modal placed its focus.
          autoFocus={edit.kind !== "entry"}
        />
      )}
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

/** Definition of `key` (with or without `/`) among `keys`, else of their wildcard. */
function keyDoc(keys: TypeKey[], key: string): KeyDoc | null {
  const k = key.trim().replace(/^\//, "");
  if (k === "") return null;
  return (keys.find((x) => x.key === k) ?? keys.find((x) => x.key === "*"))?.doc ?? null;
}

/** Values to offer: those the schema lists, else `true` and `false` for a boolean. */
function candidates(doc: KeyDoc): AllowedValue[] {
  if (doc.allowed.length > 0) return doc.allowed;
  if (doc.types.length === 1 && doc.types[0]?.base === "boolean") {
    return ["true", "false"].map((value) => ({ value, since: null, deprecated: null }));
  }
  return [];
}

/** Keys the dictionary lacks first (required ones before the others), then those it has. */
function keyOptions(keys: TypeKey[]) {
  const rank = (k: TypeKey) => (k.doc.requirement.kind === "required" ? 0 : 1);
  const names = (present: boolean) =>
    keys
      .filter((k) => (k.present ?? false) === present)
      .sort((a, b) => rank(a) - rank(b))
      .map((k) => `/${k.key}`);
  return [
    { group: "Not set", items: names(false) },
    { group: "Set (replaced)", items: names(true) },
  ].filter((g) => g.items.length > 0);
}

/** All options while the text is empty or one of them; else those containing it, ignoring case and `/`. */
const filter: OptionsFilter = ({ options, search }) => {
  const q = search.trim().replace(/^\//, "").toLowerCase();
  const all = options.flatMap((o) => ("group" in o ? o.items : [o]));
  if (q === "" || all.some((o) => same(o.value, search.trim()))) return options;
  const hit = (o: ComboboxItem) => o.value.replace(/^\//, "").toLowerCase().includes(q);
  return options.flatMap((o): ComboboxParsedItem[] => {
    if (!("group" in o)) return hit(o) ? [o] : [];
    const items = o.items.filter(hit);
    return items.length > 0 ? [{ ...o, items }] : [];
  });
};

function KeyOption({ k }: { k: TypeKey | undefined }) {
  if (!k) return null;
  const req = k.doc.requirement.kind;
  const notes = [
    req !== "optional" && req,
    k.doc.since && `PDF ${k.doc.since}`,
    k.doc.deprecated && `deprecated in ${k.doc.deprecated}`,
    k.inherited && "inherited",
  ].filter(Boolean);
  return (
    <Group gap={6} wrap="nowrap" w="100%">
      <Text size="xs" ff="monospace" td={k.doc.deprecated ? "line-through" : undefined}>
        /{k.key}
      </Text>
      <Text size="xs" c="dimmed" ml="auto">
        {notes.join(" · ")}
      </Text>
    </Group>
  );
}

function ValueOption({ v, keySince }: { v: AllowedValue | undefined; keySince: string | null }) {
  if (!v) return null;
  const since = v.since !== keySince ? v.since : null;
  return (
    <Group gap={6} wrap="nowrap" w="100%">
      <Text
        size="xs"
        ff="monospace"
        td={v.deprecated ? "line-through" : undefined}
        c={v.deprecated ? "dimmed" : undefined}
      >
        {v.value}
      </Text>
      <Text size="xs" c="dimmed" ml="auto">
        {[since && `PDF ${since}`, v.deprecated && `deprecated in ${v.deprecated}`].filter(Boolean).join(" · ")}
      </Text>
    </Group>
  );
}
