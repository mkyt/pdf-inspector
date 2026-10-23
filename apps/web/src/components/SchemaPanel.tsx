import { Alert, Anchor, Badge, Code, Divider, Group, Loader, Stack, Table, Text, Title, Tooltip, UnstyledButton } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { Fragment, useState, type ReactNode } from "react";
import { api, useBackend } from "../api";
import { useDoc } from "../doc";
import { objText, valueText } from "../format";
import { useNav } from "../nav";
import type { KeyDoc, NodeRoot, TypeKey } from "../protocol/generated";
import { useSchema } from "./Inspector";
import { IssueList } from "./Issues";
import { OperatorDoc } from "./OperatorDoc";

/**
 * Right-hand panel: what the schema says about the selected entry (or, with
 * none selected, about the object shown: the catalog, a page, a font …), or
 * what the selected operation of a content does.
 */
export function SchemaPanel() {
  const { selection, target } = useNav();
  if (target.kind === "content") {
    const c = target.content;
    if (target.op !== null) return <OperatorDoc content={c} op={target.op} />;
    return c.kind === "page" ? <PageType index={c.index} /> : <TypeView root={{ kind: "object", id: c.id }} title={objText(c.id)} />;
  }
  if (!selection) {
    switch (target.kind) {
      case "object":
        return <TypeView root={{ kind: "object", id: target.id }} title={objText(target.id)} structure />;
      case "trailer":
        return <TypeView root={{ kind: "trailer" }} title="Trailer" structure />;
      case "document":
        return <CatalogType />;
      case "page":
        return <PageType index={target.index} />;
      case "font":
        return <TypeView root={{ kind: "object", id: target.id }} title={objText(target.id)} />;
    }
  }
  const { node, path } = selection;
  const s = node.schema;
  return (
    <Stack gap="xs" p="sm">
      <div>
        <Title order={6} ff="monospace" style={{ wordBreak: "break-all" }}>
          {path}
        </Title>
        <Text size="xs" ff="monospace" c="dimmed" style={{ wordBreak: "break-all" }}>
          {valueText(node.value)}
        </Text>
      </div>
      {s?.value_type && (
        <Group gap={6}>
          <Text size="xs" c="dimmed">
            Value type
          </Text>
          <Badge size="sm" variant="light" tt="none">
            {s.value_type}
          </Badge>
        </Group>
      )}
      {s && s.issues.length > 0 && (
        <>
          <Divider label="Issues" labelPosition="left" />
          <IssueList issues={s.issues} max={20} />
        </>
      )}
      <Divider label="Definition" labelPosition="left" />
      {!s ? (
        <Text size="xs" c="dimmed">
          The type of the containing object is not known.
        </Text>
      ) : !s.key ? (
        <Text size="xs" c="dimmed">
          Not defined by the schema for this type.
        </Text>
      ) : (
        <KeyDocView doc={s.key} />
      )}
    </Stack>
  );
}

function CatalogType() {
  const { summary } = useDoc();
  if (!summary.root) {
    return (
      <Text size="xs" c="red" p="sm">
        The trailer has no /Root: the document has no catalog to describe.
      </Text>
    );
  }
  return <TypeView root={{ kind: "object", id: summary.root }} title={objText(summary.root)} />;
}

function PageType({ index }: { index: number }) {
  const backend = useBackend();
  const nav = useNav();
  // Same query as the page view: already loaded.
  const q = useQuery({ queryKey: ["doc", nav.doc, "page", index], queryFn: () => api.page(backend, nav.doc, index) });
  if (q.error) {
    return (
      <Alert m="sm" color="red" p={6}>
        <Text size="xs">{String(q.error)}</Text>
      </Alert>
    );
  }
  if (!q.data) return <Loader m="sm" size="xs" />;
  return <TypeView root={{ kind: "object", id: q.data.id }} title={objText(q.data.id)} />;
}

/**
 * The schema type of an object (or the trailer) and the keys it defines.
 * `structure`: its structure is shown beside, to select entries from;
 * otherwise the title opens the object.
 */
function TypeView({ root, title, structure = false }: { root: NodeRoot; title: string; structure?: boolean }) {
  const nav = useNav();
  const q = useSchema(root);
  const [open, setOpen] = useState<string | null>(null);
  const [extensions, setExtensions] = useState(false);
  if (q.error) {
    return (
      <Alert m="sm" color="red" p={6}>
        <Text size="xs">{String(q.error)}</Text>
      </Alert>
    );
  }
  if (!q.data) return <Loader m="sm" size="xs" />;
  const s = q.data;
  const guess = s.origin === "guess";
  const standard = s.keys.filter((k) => !k.doc.extension);
  const extra = s.keys.length - standard.length;
  const shown = extensions ? s.keys : standard;
  return (
    <Stack gap="xs" p="sm">
      <div>
        <Title order={6} ff="monospace">
          {structure || root.kind !== "object" ? (
            title
          ) : (
            <Anchor inherit onClick={() => nav.open({ kind: "object", id: root.id })}>
              {title}
            </Anchor>
          )}
        </Title>
        {s.type_name && (
          <Group gap={6}>
            <Badge size="sm" variant={guess ? "outline" : "light"} color="teal" tt="none">
              {s.type_name}
              {guess && "?"}
            </Badge>
            <Text size="xs" c="dimmed">
              {guess ? "guessed from /Type" : "typed by the references reaching it"}
            </Text>
          </Group>
        )}
      </div>
      {!s.type_name ? (
        <Text size="xs" c="dimmed">
          The schema type of this object is not known. Open it through a reference, or validate the document, to type it from
          where it is used.
        </Text>
      ) : (
        <>
          {(s.note || s.pdf_version) && (
            <Text size="xs" c="dimmed">
              {s.note && <>Defined in {s.note}. </>}
              {s.pdf_version && <>Checked against PDF {s.pdf_version}.</>}
            </Text>
          )}
          {shown.length > 0 && (
            <>
              <Divider
                label={`Keys: ${standard.filter((k) => k.present || k.inherited).length} of ${standard.length} present`}
                labelPosition="left"
              />
              <Table withRowBorders={false} verticalSpacing={1} horizontalSpacing={4} fz="xs">
                <Table.Tbody>
                  {shown.map((k) => (
                    <Fragment key={k.key}>
                      <TypeKeyRow k={k} open={open === k.key} onToggle={() => setOpen(open === k.key ? null : k.key)} />
                      {open === k.key && (
                        <Table.Tr>
                          <Table.Td colSpan={3} pl={12} pb={6}>
                            <KeyDocView doc={k.doc} />
                          </Table.Td>
                        </Table.Tr>
                      )}
                    </Fragment>
                  ))}
                </Table.Tbody>
              </Table>
              {extra > 0 && (
                <Anchor size="xs" onClick={() => setExtensions(!extensions)}>
                  {extensions ? "Hide" : "Show"} {extra} key{extra === 1 ? "" : "s"} from extensions
                </Anchor>
              )}
            </>
          )}
        </>
      )}
      {structure && (
        <Text size="xs" c="dimmed">
          Select an entry in the structure for the definition of its value.
        </Text>
      )}
    </Stack>
  );
}

function TypeKeyRow({ k, open, onToggle }: { k: TypeKey; open: boolean; onToggle: () => void }) {
  const r = k.doc.requirement;
  const old = k.doc.deprecated !== null;
  return (
    <Table.Tr style={{ whiteSpace: "nowrap" }}>
      <Table.Td>
        <UnstyledButton onClick={onToggle} fz="xs" ff="monospace" aria-expanded={open}>
          <Text span size="xs" c="dimmed">
            {open ? "▾ " : "▸ "}
          </Text>
          <Text
            span
            size="xs"
            ff="monospace"
            c={k.doc.extension || old ? "dimmed" : "indigo"}
            td={old ? "line-through" : undefined}
          >
            {k.key === "*" ? "* (any)" : /^\d/.test(k.key) ? `[${k.key}]` : `/${k.key}`}
          </Text>
        </UnstyledButton>
      </Table.Td>
      <Table.Td>
        {r.kind === "required" ? (
          <Text span size="xs" c="red">
            required
          </Text>
        ) : r.kind === "conditional" ? (
          <Tooltip label={r.condition} multiline maw={360}>
            <Text span size="xs" c="orange">
              conditional
            </Text>
          </Tooltip>
        ) : null}
      </Table.Td>
      <Table.Td ta="right">
        {k.present === true ? (
          <Text span size="xs" c="teal">
            present
          </Text>
        ) : k.inherited ? (
          <Tooltip label="Not in this dictionary: given by an ancestor through /Parent">
            <Text span size="xs" c="teal">
              inherited
            </Text>
          </Tooltip>
        ) : k.present === false ? (
          <Text span size="xs" c={r.kind === "required" ? "red" : "dimmed"}>
            {r.kind === "required" ? "missing" : "—"}
          </Text>
        ) : null}
      </Table.Td>
    </Table.Tr>
  );
}

function KeyDocView({ doc: k }: { doc: KeyDoc }) {
  const rows: [string, ReactNode][] = [
    [
      "Type",
      <Stack gap={2}>
        {k.types.map((t, i) => (
          <Text key={i} size="xs">
            <b>{t.base}</b>
            {t.links.length > 0 && (
              <Text span size="xs" c="dimmed">
                {" "}
                → {t.links.join(", ")}
              </Text>
            )}
          </Text>
        ))}
      </Stack>,
    ],
    [
      "Required",
      k.requirement.kind === "conditional" ? (
        <Text size="xs">
          when <Code>{k.requirement.condition}</Code>
        </Text>
      ) : (
        <Badge size="xs" variant="light" color={k.requirement.kind === "required" ? "red" : "gray"}>
          {k.requirement.kind}
        </Badge>
      ),
    ],
  ];
  if (k.since) rows.push(["Since", `PDF ${k.since}${k.extension ? ` (extension ${k.extension})` : ""}`]);
  else if (k.extension) rows.push(["Extension", k.extension]);
  if (k.deprecated) rows.push(["Deprecated", `PDF ${k.deprecated}`]);
  if (k.inheritable) rows.push(["Inheritable", "yes"]);
  if (k.default) rows.push(["Default", <Code>{k.default}</Code>]);
  if (k.allowed.length > 0) rows.push(["Values", <Text size="xs" ff="monospace">{k.allowed.join(" ")}</Text>]);
  if (k.note) rows.push(["Note", k.note]);
  return (
    <Table withRowBorders={false} verticalSpacing={2} horizontalSpacing={4} fz="xs">
      <Table.Tbody>
        {rows.map(([label, v]) => (
          <Table.Tr key={label}>
            <Table.Td c="dimmed" w={80} style={{ verticalAlign: "top" }}>
              {label}
            </Table.Td>
            <Table.Td style={{ wordBreak: "break-word" }}>{v}</Table.Td>
          </Table.Tr>
        ))}
      </Table.Tbody>
    </Table>
  );
}
