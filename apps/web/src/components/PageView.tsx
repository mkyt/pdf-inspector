import {
  ActionIcon,
  Alert,
  Anchor,
  Badge,
  Button,
  Group,
  Loader,
  Stack,
  Table,
  Text,
  Title,
  Tooltip,
} from "@mantine/core";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useEffect, useState, type ReactNode } from "react";
import { api, useBackend } from "../api";
import { useDoc } from "../doc";
import { bytesText, originText, pageText, rectText, sizeText } from "../format";
import { contentKey, useNav } from "../nav";
import type { AttrOrigin, PageDetail } from "../protocol/generated";
import { ObjLink } from "./DocumentPanel";
import { IssueList } from "./Issues";
import { annotColor } from "./AnnotOverlay";
import { LinkView } from "./LinkView";
import { RenderView } from "./RenderView";

const ANNOT_PAGE = 200;

function Origin({ o }: { o: AttrOrigin }) {
  const nav = useNav();
  if (o.kind === "inherited") {
    return (
      <Text span size="xs" c="dimmed">
        inherited from{" "}
        <Anchor size="xs" ff="monospace" onClick={() => nav.open({ kind: "object", id: o.from })}>
          {o.from.num} {o.from.generation} obj
        </Anchor>
      </Text>
    );
  }
  return (
    <Text span size="xs" c={o.kind === "missing" ? "red" : "dimmed"}>
      {originText(o)}
    </Text>
  );
}

/** One page: where its attributes come from, its boxes, resources, contents and annotations. */
export function PageView({ index }: { index: number }) {
  const backend = useBackend();
  const nav = useNav();
  const { summary } = useDoc();
  const q = useQuery({
    queryKey: ["doc", nav.doc, "page", index],
    queryFn: () => api.page(backend, nav.doc, index),
  });
  const step = (d: number) => nav.open({ kind: "page", index: index + d });
  // The annotation selected on the rendering and in the table.
  const [annot, setAnnot] = useState<number | null>(null);
  useEffect(() => setAnnot(null), [index]);
  return (
    <Stack p="sm" gap="sm">
      <Group gap="sm">
        <Tooltip label="Previous page">
          <ActionIcon
            size="sm"
            variant="subtle"
            disabled={index === 0}
            onClick={() => step(-1)}
            aria-label="Previous page"
          >
            ‹
          </ActionIcon>
        </Tooltip>
        <Title order={5}>
          Page {pageText(index, q.data?.label)}{" "}
          <Text span c="dimmed" size="sm">
            of {summary.pages.toLocaleString()}
          </Text>
        </Title>
        <Tooltip label="Next page">
          <ActionIcon
            size="sm"
            variant="subtle"
            disabled={index + 1 >= summary.pages}
            onClick={() => step(1)}
            aria-label="Next page"
          >
            ›
          </ActionIcon>
        </Tooltip>
        {q.data && (
          <Button size="compact-xs" variant="light" onClick={() => nav.open({ kind: "object", id: q.data.id })}>
            {q.data.id.num} {q.data.id.generation} obj
          </Button>
        )}
        <Button
          size="compact-xs"
          variant="light"
          onClick={() => nav.open({ kind: "content", content: { kind: "page", index }, op: null })}
        >
          Operations
        </Button>
      </Group>
      {q.error && (
        <Alert color="red" p={6}>
          <Text size="xs">{String(q.error)}</Text>
        </Alert>
      )}
      {!q.data && !q.error && <Loader size="sm" />}
      {q.data && <Rendering index={index} annot={annot} onAnnot={setAnnot} />}
      {q.data && <Detail p={q.data} annot={annot} onAnnot={setAnnot} />}
    </Stack>
  );
}

/** The page rendered; clicking opens the operation painting there. */
function Rendering({
  index,
  annot,
  onAnnot,
}: {
  index: number;
  annot: number | null;
  onAnnot: (i: number | null) => void;
}) {
  const backend = useBackend();
  const nav = useNav();
  const content = { kind: "page", index } as const;
  const q = useQuery({
    queryKey: ["doc", nav.doc, "content", ...contentKey(content)],
    queryFn: () => api.content(backend, nav.doc, content),
  });
  if (q.error)
    return (
      <Text size="xs" c="red">
        {String(q.error)}
      </Text>
    );
  if (!q.data) return <Loader size="xs" />;
  return (
    <div style={{ maxWidth: 640 }}>
      <RenderView
        content={content}
        summary={q.data}
        op={null}
        step={false}
        onPick={(op) => nav.open({ kind: "content", content, op })}
        height="min(60vh, 720px)"
        annot={annot}
        onAnnot={onAnnot}
      />
    </div>
  );
}

function Detail({
  p,
  annot,
  onAnnot,
}: {
  p: PageDetail;
  annot: number | null;
  onAnnot: (i: number | null) => void;
}) {
  const nav = useNav();
  return (
    <>
      <Group gap={4}>
        <Text size="xs" c="dimmed">
          Page tree:
        </Text>
        {p.ancestors.map((a, i) => (
          <Group key={i} gap={4}>
            {i > 0 && (
              <Text size="xs" c="dimmed">
                →
              </Text>
            )}
            <ObjLink id={a} />
          </Group>
        ))}
      </Group>
      {p.issues.length > 0 && (
        <Alert p={6} color={p.issues.some((i) => i.severity === "error") ? "red" : "orange"} variant="light">
          <IssueList issues={p.issues} />
        </Alert>
      )}

      <Section title="Boxes">
        <Table withRowBorders={false} verticalSpacing={2} horizontalSpacing={6} fz="xs">
          <Table.Tbody>
            {p.boxes.map((b) => (
              <Table.Tr key={b.name}>
                <Table.Td ff="monospace" c="dimmed" w={90}>
                  /{b.name}
                </Table.Td>
                <Table.Td ff="monospace">{rectText(b.rect)}</Table.Td>
                <Table.Td>{sizeText(b.rect)}</Table.Td>
                <Table.Td>
                  <Origin o={b.origin} />
                </Table.Td>
              </Table.Tr>
            ))}
            <Table.Tr>
              <Table.Td ff="monospace" c="dimmed">
                /Rotate
              </Table.Td>
              <Table.Td ff="monospace" colSpan={2}>
                {p.rotate}
              </Table.Td>
              <Table.Td>
                <Origin o={p.rotate_origin} />
              </Table.Td>
            </Table.Tr>
            {p.user_unit !== 1 && (
              <Table.Tr>
                <Table.Td ff="monospace" c="dimmed">
                  /UserUnit
                </Table.Td>
                <Table.Td ff="monospace" colSpan={3}>
                  {p.user_unit} ({p.user_unit / 72} in per unit)
                </Table.Td>
              </Table.Tr>
            )}
          </Table.Tbody>
        </Table>
      </Section>

      <Section
        title="Resources"
        right={
          <Group gap={6}>
            {p.resources && <ObjLink id={p.resources} />}
            <Origin o={p.resources_origin} />
          </Group>
        }
      >
        {p.resource_items.length === 0 ? (
          <Text size="xs" c="dimmed">
            None.
          </Text>
        ) : (
          <Table withRowBorders={false} verticalSpacing={2} horizontalSpacing={6} fz="xs">
            <Table.Tbody>
              {p.resource_items.map((r, i) => (
                <Table.Tr key={i}>
                  <Table.Td ff="monospace" c="dimmed" w={90}>
                    /{r.category}
                  </Table.Td>
                  <Table.Td ff="monospace" w={80}>
                    /{r.name}
                  </Table.Td>
                  <Table.Td w={90}>
                    {r.id ? (
                      <ObjLink id={r.id} />
                    ) : (
                      <Text span size="xs" c="dimmed">
                        direct
                      </Text>
                    )}
                  </Table.Td>
                  <Table.Td>{r.detail}</Table.Td>
                </Table.Tr>
              ))}
            </Table.Tbody>
          </Table>
        )}
        {p.resources_truncated && (
          <Text size="xs" c="dimmed">
            Only the first {p.resource_items.length} are listed.
          </Text>
        )}
      </Section>

      <Section
        title={`Contents (${p.contents.length})`}
        right={
          p.contents.length > 0 && (
            <Anchor size="xs" onClick={() => nav.open({ kind: "content", content: { kind: "page", index: p.index }, op: null })}>
              operations →
            </Anchor>
          )
        }
      >
        {p.contents.length === 0 ? (
          <Text size="xs" c="dimmed">
            No /Contents: the page is blank.
          </Text>
        ) : (
          <Stack gap={2}>
            {p.contents.map((c, i) => (
              <Group key={i} gap={8}>
                <Text size="xs" c="dimmed" w={24}>
                  [{i}]
                </Text>
                {c.id ? <ObjLink id={c.id} /> : <Text size="xs">direct</Text>}
                {c.error ? (
                  <Text size="xs" c="red">
                    {c.error}
                  </Text>
                ) : (
                  <Text size="xs" c="dimmed">
                    {bytesText(c.raw_len ?? 0)} stored{c.filters.length > 0 && ` · ${c.filters.join(", ")}`}
                  </Text>
                )}
              </Group>
            ))}
          </Stack>
        )}
      </Section>

      <Section title={`Annotations (${p.annots.toLocaleString()})`}>
        {p.annots > 0 ? (
          <Annotations page={p.index} annot={annot} onAnnot={onAnnot} />
        ) : (
          <Text size="xs" c="dimmed">
            None.
          </Text>
        )}
      </Section>
    </>
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

function Annotations({
  page,
  annot,
  onAnnot,
}: {
  page: number;
  annot: number | null;
  onAnnot: (i: number | null) => void;
}) {
  const backend = useBackend();
  const nav = useNav();
  const q = useInfiniteQuery({
    queryKey: ["doc", nav.doc, "annots", page],
    queryFn: ({ pageParam }) => api.annotations(backend, nav.doc, page, { offset: pageParam, limit: ANNOT_PAGE }),
    initialPageParam: 0,
    getNextPageParam: (last) =>
      last.offset + last.items.length < last.total ? last.offset + last.items.length : undefined,
  });
  if (q.error)
    return (
      <Text size="xs" c="red">
        {String(q.error)}
      </Text>
    );
  if (!q.data) return <Loader size="xs" />;
  const items = q.data.pages.flatMap((p) => p.items);
  return (
    <>
      <Table withRowBorders verticalSpacing={2} horizontalSpacing={6} fz="xs">
        <Table.Tbody>
          {items.map((a) => (
            <Table.Tr key={a.index} bg={a.index === annot ? "var(--mantine-color-blue-light)" : undefined}>
              <Table.Td w={36}>
                <Tooltip label="Show it on the page">
                  <Anchor size="xs" c="dimmed" onClick={() => onAnnot(a.index === annot ? null : a.index)}>
                    [{a.index}]
                  </Anchor>
                </Tooltip>
              </Table.Td>
              <Table.Td w={90}>
                {a.id ? (
                  <ObjLink id={a.id} />
                ) : (
                  <Text span size="xs" c="dimmed">
                    direct
                  </Text>
                )}
              </Table.Td>
              <Table.Td w={90}>
                {a.error ? (
                  <Text span size="xs" c="red">
                    {a.error}
                  </Text>
                ) : (
                  <Group gap={4} wrap="nowrap">
                    {annotColor(a.color) && (
                      <span
                        style={{ display: "inline-block", width: 8, height: 8, background: annotColor(a.color)!, border: "1px solid #0003" }}
                      />
                    )}
                    <Badge size="xs" variant="light" tt="none" color={a.shown ? undefined : "gray"}>
                      {a.subtype ?? "?"}
                    </Badge>
                  </Group>
                )}
              </Table.Td>
              <Table.Td ff="monospace" c="dimmed">
                {a.rect && rectText(a.rect)}
              </Table.Td>
              <Table.Td style={{ wordBreak: "break-word" }}>
                {a.link && <LinkView link={a.link} />}
                {a.author && (
                  <Text span size="xs" fw={600}>
                    {a.author}:{" "}
                  </Text>
                )}
                {a.contents && (
                  <Text span size="xs">
                    {a.link && " · "}
                    {a.contents}
                  </Text>
                )}
                {!a.shown && (
                  <Text span size="xs" c="dimmed">
                    {" "}
                    · not shown ({a.flag_names.filter((f) => ["Invisible", "Hidden", "NoView"].includes(f)).join(", ")})
                  </Text>
                )}
                {a.appearance?.error && (
                  <Text span size="xs" c="orange.8">
                    {" "}
                    · {a.appearance.error}
                  </Text>
                )}
                {a.parent && (
                  <Text span size="xs" c="dimmed">
                    {" "}
                    · for <ObjLink id={a.parent} />
                  </Text>
                )}
                {!a.appearance && a.subtype !== "Popup" && a.subtype !== "Link" && (
                  <Text span size="xs" c="dimmed">
                    {" "}
                    · no appearance
                  </Text>
                )}
              </Table.Td>
            </Table.Tr>
          ))}
        </Table.Tbody>
      </Table>
      {q.hasNextPage && (
        <Button
          size="compact-xs"
          variant="subtle"
          loading={q.isFetchingNextPage}
          onClick={() => void q.fetchNextPage()}
          style={{ alignSelf: "flex-start" }}
        >
          Show more ({items.length.toLocaleString()} of {q.data.pages[0].total.toLocaleString()})
        </Button>
      )}
    </>
  );
}
