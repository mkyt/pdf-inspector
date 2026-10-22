import { Badge, Button, Group, Progress, Stack, Text } from "@mantine/core";
import { useState } from "react";
import { api, useBackend } from "../api";
import { objText } from "../format";
import type { JobInfo } from "../protocol/generated";
import { useNav } from "../nav";
import { severityColor } from "./DiagnosticsPanel";
import { issueWhere } from "./Issues";
import { PagedList } from "./PagedList";
import { Row } from "./Row";

/**
 * Schema validation of the whole document or of the current object and what
 * it refers to. The job lives in `DocView` so switching tabs keeps it.
 */
export function ValidatePanel({ job, setJob }: { job: JobInfo | null; setJob: (j: JobInfo | null) => void }) {
  const backend = useBackend();
  const nav = useNav();
  const [error, setError] = useState<string | null>(null);
  const running = job?.state === "running";
  const target = nav.target.kind === "object" ? nav.target.id : null;
  const r = job?.result?.kind === "validate" ? job.result : null;

  const start = (root: typeof target) => {
    setError(null);
    api.startJob(backend, nav.doc, { kind: "validate", root }).then(setJob, (e) => setError(String(e)));
  };

  return (
    <>
      <Stack gap={6} p="xs" style={{ borderBottom: "1px solid var(--mantine-color-default-border)" }}>
        <Text size="xs" c="dimmed">
          Checks objects against the Arlington PDF Model, following typed references.
        </Text>
        <Group gap={6}>
          <Button size="compact-xs" variant="light" disabled={running} onClick={() => start(null)}>
            Whole document
          </Button>
          <Button size="compact-xs" variant="light" disabled={running || !target} onClick={() => start(target)}>
            {target ? `From ${objText(target)}` : "From object"}
          </Button>
          {running && (
            <Button size="compact-xs" variant="subtle" color="red" onClick={() => void api.cancelJob(backend, job.job)}>
              Cancel
            </Button>
          )}
        </Group>
        {job && (
          <>
            <Progress size="sm" value={job.total ? (100 * job.done) / job.total : 100} animated={running} />
            <Group gap={6}>
              <Text size="xs" c="dimmed">
                {job.kind.kind === "validate" && job.kind.root ? `From ${objText(job.kind.root)}: ` : "Document: "}
                {job.done.toLocaleString()} objects
                {running && ` · ${(job.total - job.done).toLocaleString()} queued`}
                {job.state === "cancelled" && " · cancelled"}
              </Text>
              {r && (
                <>
                  <Badge size="xs" color="red" variant="light">
                    {r.errors.toLocaleString()} errors
                  </Badge>
                  <Badge size="xs" color="orange" variant="light">
                    {r.warnings.toLocaleString()} warnings
                  </Badge>
                  <Badge size="xs" color="blue" variant="light">
                    {r.infos.toLocaleString()} info
                  </Badge>
                </>
              )}
            </Group>
            {r && r.kept < r.errors + r.warnings + r.infos && (
              <Text size="xs" c="dimmed">
                Showing the first {r.kept.toLocaleString()}.
              </Text>
            )}
          </>
        )}
        {error && (
          <Text size="xs" c="red">
            {error}
          </Text>
        )}
      </Stack>
      {r && job && (
        <PagedList
          total={r.kept}
          empty="No issues found."
          rowHeight={44}
          queryKey={["job", job.job, "diagnostics"]}
          fetchPage={(range, signal) => api.jobDiagnostics(backend, job.job, range, signal)}
          renderRow={(d) => (
            <Row
              onClick={() => {
                if (d.obj) nav.open({ kind: "object", id: d.obj });
                else nav.open({ kind: "trailer" });
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
                <Text size="xs" c="dimmed" ff="monospace" style={{ overflow: "hidden", textOverflow: "ellipsis" }}>
                  {issueWhere(d, true)}
                </Text>
              </div>
            </Row>
          )}
        />
      )}
    </>
  );
}
