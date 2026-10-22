import { Alert, Anchor, Badge, Code, Group, Loader, Stack, Tabs, Text, Title, Tooltip } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { api, useBackend } from "../api";
import { bytesAsText, objText, spanText, valueText } from "../format";
import { useNav } from "../nav";
import type { NodeRoot, ObjRef, ObjectView, Span } from "../protocol/generated";
import { severityColor } from "./DiagnosticsPanel";
import { ImagePreview } from "./ImagePreview";
import { IssueList } from "./Issues";
import { Referrers } from "./Referrers";
import { StreamPanel } from "./StreamPanel";
import { NodeChildren } from "./Tree";

/** Source bytes shown in the "Source" tab. */
const MAX_SOURCE = 1 << 20;

/** The trailer or an object: structure, schema type, stream data, source. */
export function Inspector() {
  const nav = useNav();
  const t = nav.target;
  if (t.kind !== "object") {
    return (
      <Stack p="sm" gap="xs">
        <Group gap="sm">
          <Title order={5}>Trailer</Title>
          <SchemaBadge root={{ kind: "trailer" }} />
        </Group>
        <Text size="xs" c="dimmed">
          Merged trailer of all revisions.
        </Text>
        <SchemaIssues root={{ kind: "trailer" }} />
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
        <SchemaBadge root={{ kind: "object", id: o.id }} />
        <Badge variant="light" color="gray">
          {valueText(o.value)}
        </Badge>
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
      <SchemaIssues root={{ kind: "object", id: o.id }} />
      {o.value.type === "dict" && o.value.type_name === "Page" && <PageLink id={o.id} />}
      <Referrers id={o.id} />
      {o.stream && <ImagePreview id={o.id} info={o.stream} />}
      {o.stream && <ContentLink id={o.id} />}
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

/** Streams that are not content of any kind (by schema type). */
const NOT_CONTENT = /Image|XRef|ObjStm|ObjectStream|Metadata|FontFile|ICC|EmbeddedFile|CMap|Function|Shading|Thumbnail|Halftone|JavaScript|SoftMask$/;

/** For streams that may hold content (forms, patterns, glyph procedures, appearances): their operations. */
function ContentLink({ id }: { id: ObjRef }) {
  const nav = useNav();
  const s = useSchema({ kind: "object", id }).data;
  if (!s || (s.type_name && NOT_CONTENT.test(s.type_name))) return null;
  return (
    <Anchor size="xs" onClick={() => nav.open({ kind: "content", content: { kind: "stream", id }, op: null })}>
      Read as a content stream →
    </Anchor>
  );
}

/** For page objects: open the page view. */
function PageLink({ id }: { id: ObjRef }) {
  const backend = useBackend();
  const nav = useNav();
  const q = useQuery({
    queryKey: ["doc", nav.doc, "pageIndex", id.num, id.generation],
    queryFn: () => api.pageIndex(backend, nav.doc, id),
  });
  if (q.data === undefined) return null;
  if (q.data === null) {
    return (
      <Text size="xs" c="orange">
        A /Page that the page tree does not reach.
      </Text>
    );
  }
  const index = q.data;
  return (
    <Anchor size="xs" onClick={() => nav.open({ kind: "page", index })}>
      Page {index + 1} of the document →
    </Anchor>
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

function useSchema(root: NodeRoot) {
  const backend = useBackend();
  const nav = useNav();
  const key = root.kind === "trailer" ? ["trailer"] : [root.id.num, root.id.generation];
  return useQuery({
    queryKey: ["doc", nav.doc, "schema", ...key],
    queryFn: () => api.schema(backend, nav.doc, root),
  });
}

/** Schema type of the object; a guess (from `/Type`) is marked as such. */
function SchemaBadge({ root }: { root: NodeRoot }) {
  const s = useSchema(root).data;
  if (!s?.type_name) return null;
  const guess = s.origin === "guess";
  const tip = [
    guess
      ? "Guessed from /Type; open it through a reference or validate the document to type it from context."
      : "Typed by following references from the trailer.",
    s.note,
    s.pdf_version && `Checked against PDF ${s.pdf_version}.`,
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <Tooltip label={tip} multiline maw={320}>
      <Badge variant={guess ? "outline" : "filled"} color="teal" tt="none">
        {s.type_name}
        {guess && "?"}
      </Badge>
    </Tooltip>
  );
}

function SchemaIssues({ root }: { root: NodeRoot }) {
  const s = useSchema(root).data;
  if (!s || s.issues.length === 0) return null;
  return (
    <Alert p={6} color={s.issues.some((i) => i.severity === "error") ? "red" : "orange"} variant="light">
      <IssueList issues={s.issues} />
      {s.truncated && (
        <Text size="xs" c="dimmed" mt={4}>
          Partly checked; validate from this object for everything.
        </Text>
      )}
    </Alert>
  );
}
