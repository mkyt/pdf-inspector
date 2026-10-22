import { Alert, Badge, Button, Group, Progress, Stack, Text } from "@mantine/core";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { api, useBackend } from "../api";
import { bytesText } from "../format";
import type { DocSummary, JobInfo } from "../protocol/generated";

export function SummaryCard({ summary: s }: { summary: DocSummary }) {
  const backend = useBackend();
  const qc = useQueryClient();
  const [job, setJob] = useState<JobInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const doc = s.info.doc;

  useEffect(
    () =>
      backend.onEvent((ev) => {
        if (ev.type === "jobProgress" && ev.doc === doc) {
          setJob((j) => (j && j.job === ev.job ? { ...j, done: ev.done, total: ev.total } : j));
        } else if (ev.type === "jobDone" && ev.info.doc === doc) {
          setJob((j) => (j && j.job === ev.info.job ? ev.info : j));
          // Loading objects records new diagnostics.
          void qc.invalidateQueries({ queryKey: ["doc", doc, "summary"] });
          void qc.invalidateQueries({ queryKey: ["doc", doc, "objects"] });
        }
      }),
    [backend, qc, doc],
  );

  const start = (decode: boolean) => {
    setError(null);
    api.startJob(backend, doc, { kind: "checkObjects", decode_streams: decode }).then(setJob, (e) => setError(String(e)));
  };
  const running = job?.state === "running";
  const r = job?.result;

  return (
    <Stack gap={4} p="sm" style={{ borderBottom: "1px solid var(--mantine-color-default-border)" }}>
      <Text fw={600} size="sm" style={{ wordBreak: "break-all" }} title={s.info.path ?? undefined}>
        {s.info.name}
      </Text>
      <Group gap={6}>
        <Badge size="sm" variant="default">
          PDF {s.version ?? "?"}
        </Badge>
        <Badge size="sm" variant="default">
          {bytesText(s.info.size)}
        </Badge>
        <Badge size="sm" variant="default">
          {s.live_objects.toLocaleString()} objects
        </Badge>
        <Badge size="sm" variant="default">
          {s.revisions.length} rev
        </Badge>
        {s.recovered && (
          <Badge size="sm" color="orange">
            recovered
          </Badge>
        )}
        {s.encryption && (
          <Badge size="sm" color={s.encryption.authenticated ? "grape" : "red"}>
            {s.encryption.stream_method} {s.encryption.key_bits}-bit{s.encryption.authenticated ? "" : " · locked"}
          </Badge>
        )}
      </Group>
      {s.encryption && !s.encryption.authenticated && (
        <Alert color="red" p={6}>
          <Text size="xs">Encrypted; reopen with the password to see strings and streams.</Text>
        </Alert>
      )}
      <Group gap={6} mt={4}>
        <Button size="compact-xs" variant="light" disabled={running} onClick={() => start(false)}>
          Check objects
        </Button>
        <Button size="compact-xs" variant="light" disabled={running} onClick={() => start(true)}>
          + decode streams
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
          <Text size="xs" c="dimmed">
            {job.state === "running" && `${job.done.toLocaleString()} / ${job.total.toLocaleString()}`}
            {running && backend.mode === "browser" && " · runs in this tab; pdfi serve is faster for large files"}
            {job.state === "cancelled" && "Cancelled"}
            {r &&
              `ok ${r.ok.toLocaleString()} · failed ${r.failed} · missing ${r.missing}` +
                (r.streams ? ` · ${r.streams} streams, ${bytesText(r.decoded_bytes)}, ${r.decode_errors} errors` : "") +
                ` · +${r.diagnostics} diagnostics`}
          </Text>
        </>
      )}
      {error && (
        <Text size="xs" c="red">
          {error}
        </Text>
      )}
    </Stack>
  );
}
