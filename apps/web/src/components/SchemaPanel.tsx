import { Badge, Code, Divider, Group, Stack, Table, Text, Title } from "@mantine/core";
import type { ReactNode } from "react";
import { valueText } from "../format";
import { useNav } from "../nav";
import type { KeyDoc } from "../protocol/generated";
import { IssueList } from "./Issues";
import { OperatorDoc } from "./OperatorDoc";

/**
 * Right-hand panel: what the schema says about the selected entry, or what
 * the selected operation of a content does.
 */
export function SchemaPanel() {
  const { selection, target } = useNav();
  if (target.kind === "content") return <OperatorDoc content={target.content} op={target.op} />;
  if (!selection) {
    return (
      <Text size="xs" c="dimmed" p="sm">
        Select an entry in the structure to see its definition in the PDF specification (Arlington PDF Model).
      </Text>
    );
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
