import { ActionIcon, Anchor, Badge, Group, Loader, Text, Tooltip } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { Fragment, useLayoutEffect, useRef, type ReactNode } from "react";
import { api, useBackend } from "../api";
import { useDoc } from "../doc";
import { pageText } from "../format";
import { useNav, type Target } from "../nav";
import type { ObjRef, PageDetail } from "../protocol/generated";

/** The page a target is about, if any. */
function targetPage(t: Target): number | null {
  if (t.kind === "page") return t.index;
  if (t.kind === "content" && t.content.kind === "page") return t.content.index;
  return null;
}

/** The indirect object a target shows (for pages, the page object), once known. */
export function useTargetObject(): { id: ObjRef | null; page: PageDetail | undefined } {
  const backend = useBackend();
  const nav = useNav();
  const t = nav.target;
  const index = targetPage(t);
  const page = useQuery({
    queryKey: ["doc", nav.doc, "page", index],
    queryFn: () => api.page(backend, nav.doc, index!),
    enabled: index !== null,
  }).data;
  const id =
    t.kind === "object" || t.kind === "font"
      ? t.id
      : t.kind === "content" && t.content.kind === "stream"
        ? t.content.id
        : index !== null
          ? (page?.id ?? null)
          : null;
  return { id, page };
}

interface Crumb {
  /** Entry of the previous crumb that leads here: `/Pages`, `/Kids[3]`. */
  key?: string;
  label: ReactNode;
  title?: string;
  /** Where clicking goes (none: the current view). */
  to?: Target;
}

const objLabel = (id: ObjRef) => `${id.num} ${id.generation}`;
/** Arlington names are long; `/Type` guesses come with a slash. */
const typeLabel = (t: string | null | undefined) => t?.replace(/^\//, "");

/**
 * Where the current view sits: the chain of references from the trailer to
 * the object it shows (shortest, preferring forward links), then the view.
 */
export function Breadcrumbs() {
  const backend = useBackend();
  const nav = useNav();
  const { summary, inventory } = useDoc();
  const t = nav.target;
  const { id, page } = useTargetObject();
  const schema = useQuery({
    queryKey: ["doc", nav.doc, "schema", id?.num, id?.generation],
    queryFn: () => api.schema(backend, nav.doc, { kind: "object", id: id! }),
    enabled: id !== null,
  }).data;
  const path = useQuery({
    queryKey: ["doc", nav.doc, "refPath", id?.num, id?.generation],
    queryFn: () => api.refPath(backend, nav.doc, id!),
    enabled: id !== null && !!inventory.stats,
  });
  const scroller = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el) el.scrollLeft = el.scrollWidth;
  });

  const crumbs: Crumb[] = [];
  let gap: ReactNode = null;
  if (t.kind === "document") {
    crumbs.push({ label: "Document" });
  } else if (t.kind === "trailer") {
    crumbs.push({ label: "Trailer" });
  } else if (id) {
    const steps = path.data?.steps;
    if (steps) {
      steps.forEach((s, i) =>
        crumbs.push(
          s.from.kind === "trailer"
            ? { label: "Trailer", to: { kind: "trailer" } }
            : {
                key: steps[i - 1]?.key,
                label: typeLabel(s.type_name) ?? objLabel(s.from.id),
                title: `${objLabel(s.from.id)} obj`,
                to: { kind: "object", id: s.from.id },
              },
        ),
      );
    } else if (page) {
      // Known without the inventory: the page tree.
      crumbs.push({ label: "Trailer", to: { kind: "trailer" } });
      if (summary.root) crumbs.push({ key: "/Root", label: "Catalog", to: { kind: "object", id: summary.root } });
      page.ancestors.forEach((a, i) =>
        crumbs.push({
          key: i === 0 ? "/Pages" : "/Kids",
          label: "Pages",
          title: `${objLabel(a)} obj`,
          to: { kind: "object", id: a },
        }),
      );
    } else {
      crumbs.push({ label: "Trailer", to: { kind: "trailer" } });
      gap =
        path.data && !path.data.reachable ? (
          <Tooltip label="No chain of references from the trailer leads to this object.">
            <Badge size="xs" color="orange" variant="light" tt="none">
              unreachable
            </Badge>
          </Tooltip>
        ) : path.isFetching ? (
          <Loader size={12} />
        ) : inventory.stats ? null : (
          <Tooltip label="Build the inventory (one pass over the file) to show how the trailer reaches this object">
            <ActionIcon
              size="xs"
              variant="subtle"
              color="gray"
              loading={inventory.job?.state === "running"}
              onClick={inventory.start}
              aria-label="Trace from the trailer"
            >
              …
            </ActionIcon>
          </Tooltip>
        );
    }
    const steps2 = path.data?.steps;
    const lastKey = steps2?.length
      ? steps2[steps2.length - 1].key
      : page && page.ancestors.length
        ? "/Kids"
        : undefined;
    const isObject = t.kind === "object";
    crumbs.push({
      key: lastKey,
      label: typeLabel(schema?.type_name) ?? objLabel(id),
      title: `${objLabel(id)} obj`,
      to: isObject ? undefined : { kind: "object", id },
    });
    if (t.kind === "page") crumbs.push({ label: `Page ${pageText(t.index, page?.label)}` });
    if (t.kind === "content") crumbs.push({ label: "Operations" });
    if (t.kind === "font") crumbs.push({ label: "Font" });
  } else if (targetPage(t) !== null) {
    crumbs.push({ label: `Page ${targetPage(t)! + 1}` });
  }

  return (
    <div ref={scroller} style={{ flex: 1, minWidth: 0, overflowX: "auto", scrollbarWidth: "none" }}>
      <Group gap={4} wrap="nowrap" component="nav" aria-label="Breadcrumbs">
        {crumbs.map((c, i) => (
          <Fragment key={i}>
            {i > 0 && (
              <Text size="xs" c="dimmed" aria-hidden>
                ›
              </Text>
            )}
            {i === 1 && gap && (
              <>
                {gap}
                <Text size="xs" c="dimmed" aria-hidden>
                  ›
                </Text>
              </>
            )}
            {c.key && (
              <Text size="xs" c="dimmed" ff="monospace" style={{ whiteSpace: "nowrap" }}>
                {c.key}
              </Text>
            )}
            {c.to ? (
              <Anchor
                component="button"
                type="button"
                size="xs"
                title={c.title}
                onClick={() => nav.open(c.to!)}
                style={{ whiteSpace: "nowrap" }}
              >
                {c.label}
              </Anchor>
            ) : (
              <Text size="xs" fw={600} title={c.title} style={{ whiteSpace: "nowrap" }} aria-current="page">
                {c.label}
              </Text>
            )}
          </Fragment>
        ))}
      </Group>
    </div>
  );
}
