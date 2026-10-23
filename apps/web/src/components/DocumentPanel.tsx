import {
  ActionIcon,
  Alert,
  Anchor,
  Autocomplete,
  Badge,
  Button,
  Checkbox,
  Code,
  Group,
  Loader,
  Modal,
  Stack,
  Table,
  Text,
  Textarea,
  Title,
  Tooltip,
} from "@mantine/core";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useEffect, useState, type ReactNode } from "react";
import { api, useBackend } from "../api";
import { useDoc } from "../doc";
import { useEdit } from "../edit";
import { bytesText, objText } from "../format";
import { useNav } from "../nav";
import type { ObjRef } from "../protocol/generated";
import { IssueList } from "./Issues";

const XMP_PAGE = 500;

export function ObjLink({ id }: { id: ObjRef }) {
  const nav = useNav();
  return (
    <Anchor size="xs" ff="monospace" onClick={() => nav.open({ kind: "object", id })}>
      {objText(id)}
    </Anchor>
  );
}

/** The document at a glance: Info, what the catalog declares, XMP metadata. */
export function DocumentPanel() {
  const backend = useBackend();
  const nav = useNav();
  const { summary: s } = useDoc();
  const { run } = useEdit();
  const [syncXmp, setSyncXmp] = useState(true);
  const [info, setInfo] = useState<{ key: string; value: string; fixed: boolean } | null>(null);
  const [xmp, setXmp] = useState(false);
  const q = useQuery({ queryKey: ["doc", nav.doc, "document"], queryFn: () => api.document(backend, nav.doc) });
  if (q.error) {
    return (
      <Alert m="sm" color="red" title="Document">
        {String(q.error)}
      </Alert>
    );
  }
  if (!q.data) return <Loader m="md" size="sm" />;
  const d = q.data;
  return (
    <Stack p="sm" gap="sm">
      <Group gap="sm">
        <Title order={5}>Document</Title>
        <Badge variant="light" color="gray">
          {d.pages.toLocaleString()} pages
        </Badge>
        <Badge variant="light" color="gray">
          PDF {s.pdf_version ?? s.version ?? "?"}
        </Badge>
        {d.catalog && <ObjLink id={d.catalog} />}
      </Group>
      {d.issues.length > 0 && (
        <Alert p={6} color={d.issues.some((i) => i.severity === "error") ? "red" : "orange"} variant="light">
          <IssueList issues={d.issues} withObj />
        </Alert>
      )}

      <Section
        title="Info"
        right={
          <>
            {d.info && <ObjLink id={d.info} />}
            <Button size="compact-xs" variant="light" onClick={() => setInfo({ key: "", value: "", fixed: false })}>
              Add entry…
            </Button>
            <Tooltip label="Changing an Info entry changes the matching XMP property (dc:title, xmp:ModifyDate, …)">
              <Checkbox
                size="xs"
                label="Update XMP too"
                checked={syncXmp}
                onChange={(e) => setSyncXmp(e.currentTarget.checked)}
              />
            </Tooltip>
          </>
        }
      >
        {d.info_entries.length === 0 ? (
          <Text size="xs" c="dimmed">
            {d.info ? "Empty." : "No /Info dictionary."}
          </Text>
        ) : (
          <KeyTable
            rows={d.info_entries.map((e) => [
              `/${e.key}`,
              <Group gap={6} wrap="nowrap" align="flex-start">
                <div style={{ flex: 1, minWidth: 0 }}>
                  <Text span size="xs" style={{ wordBreak: "break-word" }}>
                    {e.text}
                  </Text>
                  {e.date && (
                    <Text span size="xs" c="dimmed">
                      {" "}
                      = {e.date}
                    </Text>
                  )}
                </div>
                <ActionIcon
                  size="xs"
                  variant="subtle"
                  color="gray"
                  aria-label={`Edit /${e.key}`}
                  onClick={() => setInfo({ key: e.key, value: e.date ?? e.text.replace(/^\//, ""), fixed: true })}
                >
                  ✎
                </ActionIcon>
                <ActionIcon
                  size="xs"
                  variant="subtle"
                  color="red"
                  aria-label={`Remove /${e.key}`}
                  onClick={() =>
                    run(`Remove /Info /${e.key}`, [{ op: "setInfo", key: e.key, value: null, sync_xmp: syncXmp }])
                  }
                >
                  ✕
                </ActionIcon>
              </Group>,
            ])}
          />
        )}
      </Section>

      <Section title="Catalog">
        {d.features.length === 0 ? (
          <Text size="xs" c="dimmed">
            Nothing beyond the page tree.
          </Text>
        ) : (
          <KeyTable
            rows={d.features.map((f) => [
              `/${f.key}`,
              <Group gap={6}>
                <Text span size="xs">
                  {f.text}
                </Text>
                {f.obj && <ObjLink id={f.obj} />}
              </Group>,
            ])}
          />
        )}
      </Section>

      <Section
        title="XMP metadata"
        right={
          <>
            {d.metadata && <ObjLink id={d.metadata} />}
            <Button size="compact-xs" variant="light" onClick={() => setXmp(true)}>
              {d.metadata ? "Edit XML…" : "Add XMP…"}
            </Button>
          </>
        }
      >
        {d.metadata ? (
          <Xmp />
        ) : (
          <Text size="xs" c="dimmed">
            The catalog has no /Metadata stream.
          </Text>
        )}
      </Section>
      <InfoEditor entry={info} syncXmp={syncXmp} onClose={() => setInfo(null)} />
      <XmpEditor opened={xmp} metadata={d.metadata} onClose={() => setXmp(false)} />
    </Stack>
  );
}

const INFO_KEYS = ["Title", "Author", "Subject", "Keywords", "Creator", "Producer", "CreationDate", "ModDate", "Trapped"];

function InfoEditor({
  entry,
  syncXmp,
  onClose,
}: {
  entry: { key: string; value: string; fixed: boolean } | null;
  syncXmp: boolean;
  onClose: () => void;
}) {
  const { apply } = useEdit();
  const [key, setKey] = useState("");
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    setKey(entry?.key ?? "");
    setValue(entry?.value ?? "");
    setError(null);
  }, [entry]);
  const k = key.trim().replace(/^\//, "");
  const date = k === "CreationDate" || k === "ModDate";
  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      await apply(`Set /Info /${k}`, [{ op: "setInfo", key: k, value, sync_xmp: syncXmp }]);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal opened={entry !== null} onClose={onClose} title={entry?.fixed ? `Edit /Info /${entry.key}` : "Add an Info entry"}>
      <Stack gap="xs">
        <Autocomplete
          size="xs"
          label="Key"
          data={INFO_KEYS}
          value={key}
          onChange={setKey}
          disabled={entry?.fixed}
          data-autofocus={!entry?.fixed || undefined}
        />
        <Textarea
          size="xs"
          label="Value"
          description={
            date
              ? "A date: ISO 8601 (2026-10-24T09:30:00+09:00) or a PDF date (D:20261024093000+09'00')."
              : k === "Trapped"
                ? "True, False or Unknown (stored as a name)."
                : "Text; stored as PDFDocEncoding when plain ASCII, UTF-16 otherwise."
          }
          autosize
          minRows={1}
          maxRows={8}
          value={value}
          onChange={(e) => setValue(e.currentTarget.value)}
          data-autofocus={entry?.fixed || undefined}
        />
        {date && (
          <Button size="compact-xs" variant="subtle" style={{ alignSelf: "flex-start" }} onClick={() => setValue(nowIso())}>
            Now
          </Button>
        )}
        {error && (
          <Alert color="red" p={6}>
            <Text size="xs">{error}</Text>
          </Alert>
        )}
        <Group justify="flex-end">
          <Button size="xs" variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button size="xs" loading={busy} disabled={!k} onClick={() => void submit()}>
            Apply
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}

/** The current time in ISO 8601 with the local offset. */
function nowIso(): string {
  const d = new Date();
  const p = (n: number) => String(Math.abs(n)).padStart(2, "0");
  const off = -d.getTimezoneOffset();
  return (
    `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}` +
    `${off >= 0 ? "+" : "-"}${p(Math.trunc(off / 60))}:${p(off % 60)}`
  );
}

const XMP_TEMPLATE = `<?xpacket begin="\uFEFF" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description rdf:about=""
        xmlns:dc="http://purl.org/dc/elements/1.1/"
        xmlns:xmp="http://ns.adobe.com/xap/1.0/"
        xmlns:pdf="http://ns.adobe.com/pdf/1.3/">
    </rdf:Description>
  </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>`;

/** Largest metadata loaded for editing as text. */
const MAX_XMP = 4 << 20;

function XmpEditor({ opened, metadata, onClose }: { opened: boolean; metadata: ObjRef | null; onClose: () => void }) {
  const backend = useBackend();
  const nav = useNav();
  const { apply } = useEdit();
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!opened) return;
    setError(null);
    if (!metadata) {
      setText(XMP_TEMPLATE);
      return;
    }
    setText(null);
    let live = true;
    api.streamData(backend, nav.doc, metadata, true, 0, MAX_XMP).then(
      (b) => {
        if (!live) return;
        if (!b.info.eof) setError("The metadata is too large to edit here.");
        setText(new TextDecoder().decode(b.data));
      },
      (e) => live && setError(String(e)),
    );
    return () => {
      live = false;
    };
  }, [opened, metadata, backend, nav.doc]);
  const submit = async () => {
    if (text === null) return;
    setBusy(true);
    setError(null);
    try {
      await apply(metadata ? "Edit the XMP metadata" : "Add XMP metadata", [{ op: "setXmp", xml: text }]);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal opened={opened} onClose={onClose} title="XMP metadata" size="xl">
      <Stack gap="xs">
        {text === null && !error ? (
          <Loader size="sm" />
        ) : (
          <Textarea
            size="xs"
            styles={{ input: { fontFamily: "var(--mantine-font-family-monospace)", fontSize: 12 } }}
            autosize
            minRows={8}
            maxRows={28}
            value={text ?? ""}
            onChange={(e) => setText(e.currentTarget.value)}
          />
        )}
        <Text size="xs" c="dimmed">
          Stored unfiltered (UTF-8), as the catalog&apos;s /Metadata stream. It must be well-formed XML.
        </Text>
        {error && (
          <Alert color="red" p={6}>
            <Text size="xs">{error}</Text>
          </Alert>
        )}
        <Group justify="flex-end">
          <Button size="xs" variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button size="xs" loading={busy} disabled={text === null} onClick={() => void submit()}>
            Apply
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}

function Section({ title, right, children }: { title: string; right?: ReactNode; children: ReactNode }) {
  return (
    <Stack gap={4}>
      <Group gap="xs">
        <Text size="sm" fw={600}>
          {title}
        </Text>
        {right}
      </Group>
      {children}
    </Stack>
  );
}

export function KeyTable({ rows, keyWidth = 140 }: { rows: [ReactNode, ReactNode][]; keyWidth?: number }) {
  return (
    <Table withRowBorders={false} verticalSpacing={2} horizontalSpacing={6} fz="xs">
      <Table.Tbody>
        {rows.map(([k, v], i) => (
          <Table.Tr key={i}>
            <Table.Td c="dimmed" w={keyWidth} ff="monospace" style={{ verticalAlign: "top", whiteSpace: "nowrap" }}>
              {k}
            </Table.Td>
            <Table.Td style={{ wordBreak: "break-word" }}>{v}</Table.Td>
          </Table.Tr>
        ))}
      </Table.Tbody>
    </Table>
  );
}

function Xmp() {
  const backend = useBackend();
  const nav = useNav();
  const q = useInfiniteQuery({
    queryKey: ["doc", nav.doc, "xmp"],
    queryFn: ({ pageParam }) => api.xmp(backend, nav.doc, { offset: pageParam, limit: XMP_PAGE }),
    initialPageParam: 0,
    getNextPageParam: (last) => {
      const n = last.nodes;
      return n.offset + n.items.length < n.total ? n.offset + n.items.length : undefined;
    },
  });
  if (q.error)
    return (
      <Text size="xs" c="red">
        {String(q.error)}
      </Text>
    );
  if (!q.data) return <Loader size="xs" />;
  const first = q.data.pages[0];
  const nodes = q.data.pages.flatMap((p) => p.nodes.items);
  return (
    <Stack gap={6}>
      <Text size="xs" c="dimmed">
        {bytesText(first.len)} · {first.nodes.total.toLocaleString()} nodes
        {first.truncated && " (tree truncated)"}
      </Text>
      {first.error && (
        <Alert p={6} color="orange">
          <Text size="xs">{first.error}</Text>
        </Alert>
      )}
      {first.properties.length > 0 && (
        <KeyTable
          rows={first.properties.map((p) => [<span title={p.namespace}>{p.name}</span>, p.value])}
          keyWidth={180}
        />
      )}
      <Code block style={{ maxHeight: 420, overflow: "auto", fontSize: 11 }}>
        {nodes.map((n, i) => (
          <div
            key={i}
            style={{ paddingLeft: n.depth * 14, whiteSpace: "pre-wrap", wordBreak: "break-all" }}
            title={n.namespace ?? undefined}
          >
            <span
              style={{ color: n.kind === "attribute" ? "var(--mantine-color-grape-6)" : "var(--mantine-color-blue-6)" }}
            >
              {n.kind === "attribute" ? `@${n.name}` : `<${n.name}>`}
            </span>
            {n.value !== null && ` ${n.value}`}
          </div>
        ))}
      </Code>
      {q.hasNextPage && (
        <Button
          size="compact-xs"
          variant="subtle"
          loading={q.isFetchingNextPage}
          onClick={() => void q.fetchNextPage()}
          style={{ alignSelf: "flex-start" }}
        >
          Show more ({nodes.length.toLocaleString()} of {first.nodes.total.toLocaleString()})
        </Button>
      )}
    </Stack>
  );
}
