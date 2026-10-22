import { Badge, Text } from "@mantine/core";
import { api, useBackend } from "../api";
import { useNav } from "../nav";
import { PagedList } from "./PagedList";
import { Row } from "./Row";

export const severityColor = { error: "red", warning: "orange", info: "blue" } as const;

export function DiagnosticsPanel({ total }: { total: number }) {
  const backend = useBackend();
  const nav = useNav();
  return (
    <PagedList
      total={total}
      empty="No diagnostics."
      rowHeight={44}
      queryKey={["doc", nav.doc, "diagnostics", total]}
      fetchPage={(r, signal) => api.diagnostics(backend, nav.doc, r, signal)}
      renderRow={(d) => (
        <Row
          onClick={() => {
            if (d.obj) nav.open({ kind: "object", id: d.obj });
            if (d.span) nav.showBytes(d.span);
          }}
        >
          <Badge size="xs" color={severityColor[d.severity]} variant="light" style={{ flexShrink: 0 }}>
            {d.code}
          </Badge>
          <div style={{ minWidth: 0 }}>
            <Text size="xs" style={{ overflow: "hidden", textOverflow: "ellipsis" }} title={d.message}>
              {d.message}
            </Text>
            <Text size="xs" c="dimmed">
              {d.obj && `${d.obj.num} ${d.obj.generation} obj`}
              {d.span && ` @${d.span.start.toLocaleString()}`}
            </Text>
          </div>
        </Row>
      )}
    />
  );
}
