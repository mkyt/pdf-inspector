import { Alert, Anchor, Badge, Button, Code, Group, Loader, Stack, Table, Text, Title } from "@mantine/core";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { api, useBackend } from "../api";
import { useDoc } from "../doc";
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

      <Section title="Info" right={d.info && <ObjLink id={d.info} />}>
        {d.info_entries.length === 0 ? (
          <Text size="xs" c="dimmed">
            {d.info ? "Empty." : "No /Info dictionary."}
          </Text>
        ) : (
          <KeyTable
            rows={d.info_entries.map((e) => [
              `/${e.key}`,
              <>
                <Text span size="xs" style={{ wordBreak: "break-word" }}>
                  {e.text}
                </Text>
                {e.date && (
                  <Text span size="xs" c="dimmed">
                    {" "}
                    = {e.date}
                  </Text>
                )}
              </>,
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

      <Section title="XMP metadata" right={d.metadata && <ObjLink id={d.metadata} />}>
        {d.metadata ? (
          <Xmp />
        ) : (
          <Text size="xs" c="dimmed">
            The catalog has no /Metadata stream.
          </Text>
        )}
      </Section>
    </Stack>
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
