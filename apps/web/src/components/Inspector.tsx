import { Alert, Anchor, Badge, Code, Group, Loader, Stack, Tabs, Text, Title } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { api, useBackend } from "../api";
import { bytesAsText, objText, spanText, valueText } from "../format";
import { useNav } from "../nav";
import type { ObjRef, ObjectView, Span } from "../protocol/generated";
import { severityColor } from "./DiagnosticsPanel";
import { StreamPanel } from "./StreamPanel";
import { NodeChildren } from "./Tree";

/** Source bytes shown in the "Source" tab. */
const MAX_SOURCE = 1 << 20;

export function Inspector() {
  const nav = useNav();
  const t = nav.target;
  if (t.kind === "trailer") {
    return (
      <Stack p="sm" gap="xs">
        <Title order={5}>Trailer</Title>
        <Text size="xs" c="dimmed">
          Merged trailer of all revisions.
        </Text>
        <div>
          <NodeChildren path={{ root: { kind: "trailer" }, path: [] }} depth={0} />
        </div>
      </Stack>
    );
  }
  return <ObjectInspector key={`${t.id.num}/${t.id.generation}`} id={t.id} />;
}

function ObjectInspector({ id }: { id: ObjRef }) {
  const backend = useBackend();
  const nav = useNav();
  const q = useQuery({
    queryKey: ["doc", nav.doc, "object", id.num, id.generation],
    queryFn: () => api.object(backend, nav.doc, id),
  });
  if (q.error) {
    return (
      <Alert m="sm" color="red" title={objText(id)}>
        {String(q.error)}
      </Alert>
    );
  }
  if (!q.data) return <Loader m="md" size="sm" />;
  const o = q.data;
  return (
    <Stack p="sm" gap="xs">
      <Group gap="sm">
        <Title order={5}>{objText(o.id)}</Title>
        <Badge variant="light">{valueText(o.value)}</Badge>
        {o.location.kind === "file" ? (
          <Anchor size="xs" onClick={() => nav.showBytes((o.location as { span: Span }).span)}>
            file {spanText(o.location.span)}
          </Anchor>
        ) : (
          <Anchor size="xs" onClick={() => o.location.kind === "objStm" && nav.open({ kind: "object", id: o.location.stream })}>
            in object stream {o.location.stream.num} {o.location.stream.generation} [index {o.location.index}]
          </Anchor>
        )}
      </Group>
      {o.diagnostics.map((d, i) => (
        <Alert key={i} p={6} color={severityColor[d.severity]}>
          <Text size="xs">
            <b>{d.code}</b> {d.message}
          </Text>
        </Alert>
      ))}
      <Tabs defaultValue="structure" keepMounted={false}>
        <Tabs.List>
          <Tabs.Tab value="structure">Structure</Tabs.Tab>
          {o.stream && <Tabs.Tab value="stream">Stream data</Tabs.Tab>}
          <Tabs.Tab value="source">Source</Tabs.Tab>
        </Tabs.List>
        <Tabs.Panel value="structure" pt="xs">
          <Structure o={o} />
        </Tabs.Panel>
        {o.stream && (
          <Tabs.Panel value="stream" pt="xs">
            <StreamPanel id={o.id} info={o.stream} />
          </Tabs.Panel>
        )}
        <Tabs.Panel value="source" pt="xs">
          <Source o={o} />
        </Tabs.Panel>
      </Tabs>
    </Stack>
  );
}

function Structure({ o }: { o: ObjectView }) {
  const v = o.value;
  if (v.type === "dict" || v.type === "array" || v.type === "stream") {
    return <NodeChildren path={{ root: { kind: "object", id: o.id }, path: [] }} depth={0} />;
  }
  return <Code>{valueText(v)}</Code>;
}

function Source({ o }: { o: ObjectView }) {
  const backend = useBackend();
  const nav = useNav();
  const span = o.location.kind === "file" ? o.location.span : null;
  // Stream data is shown in its own tab; here only the object up to `stream`.
  const end = span && o.stream?.data_span ? o.stream.data_span.start : span?.end;
  const len = span && end !== undefined ? Math.min(end - span.start, MAX_SOURCE) : 0;
  const q = useQuery({
    queryKey: ["doc", nav.doc, "source", span?.start, len],
    queryFn: () => api.rawBytes(backend, nav.doc, span!.start, len),
    enabled: span !== null,
  });
  if (!span) {
    return (
      <Text size="xs" c="dimmed">
        Stored in an object stream; open the stream to see its decoded data.
      </Text>
    );
  }
  if (!q.data) return <Loader size="sm" />;
  return (
    <Code block style={{ whiteSpace: "pre-wrap", wordBreak: "break-all", maxHeight: 520, overflow: "auto" }}>
      {bytesAsText(q.data.data)}
      {o.stream && "\n… stream data …\nendstream\nendobj"}
      {len < (end ?? 0) - span.start && "\n… truncated"}
    </Code>
  );
}
