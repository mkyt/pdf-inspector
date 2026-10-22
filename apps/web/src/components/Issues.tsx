import { Badge, Button, Group, Stack, Text, UnstyledButton } from "@mantine/core";
import { useState } from "react";
import { objText } from "../format";
import { useNav } from "../nav";
import type { DiagnosticInfo, Severity } from "../protocol/generated";
import { severityColor } from "./DiagnosticsPanel";

const rank: Record<Severity, number> = { info: 0, warning: 1, error: 2 };

/** The most severe of `issues`, if any. */
export function worst(issues: DiagnosticInfo[]): Severity | null {
  let w: Severity | null = null;
  for (const i of issues) if (w === null || rank[i.severity] > rank[w]) w = i.severity;
  return w;
}

/** Where an issue is: `12 0 obj/Pages/Kids[3]`. */
export function issueWhere(d: DiagnosticInfo, withObj: boolean): string {
  const obj = withObj ? (d.obj ? objText(d.obj) : "trailer") : "";
  return `${obj}${d.path ?? ""}`;
}

/** Compact list of schema issues; clicking one shows its bytes. */
export function IssueList({ issues, max = 8, withObj = false }: { issues: DiagnosticInfo[]; max?: number; withObj?: boolean }) {
  const nav = useNav();
  const [all, setAll] = useState(false);
  const sorted = [...issues].sort((a, b) => rank[b.severity] - rank[a.severity]);
  const shown = all ? sorted : sorted.slice(0, max);
  return (
    <Stack gap={2}>
      {shown.map((d, i) => (
        <UnstyledButton key={i} onClick={() => d.span && nav.showBytes(d.span)} style={{ cursor: d.span ? "pointer" : "default" }}>
          <Group gap={6} wrap="nowrap" align="flex-start">
            <Badge size="xs" color={severityColor[d.severity]} variant="light" style={{ flexShrink: 0 }}>
              {d.code}
            </Badge>
            <Text size="xs" style={{ minWidth: 0 }}>
              {issueWhere(d, withObj) && (
                <Text span size="xs" ff="monospace" c="dimmed">
                  {issueWhere(d, withObj)}{" "}
                </Text>
              )}
              {d.message}
            </Text>
          </Group>
        </UnstyledButton>
      ))}
      {sorted.length > max && (
        <Button size="compact-xs" variant="subtle" onClick={() => setAll(!all)} style={{ alignSelf: "flex-start" }}>
          {all ? "Show fewer" : `Show all ${sorted.length}`}
        </Button>
      )}
    </Stack>
  );
}
