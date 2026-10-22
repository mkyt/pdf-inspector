import { Button, Progress, Stack, Text } from "@mantine/core";
import type { ReactNode } from "react";
import { useBackend } from "../api";
import { useDoc } from "../doc";

/**
 * Shows `children` once the inventory is built; until then a button that
 * starts the job (one scan of every object and page) and its progress.
 */
export function InventoryGate({ what, children }: { what: string; children: ReactNode }) {
  const backend = useBackend();
  const { inventory: inv } = useDoc();
  if (inv.stats) return <>{children}</>;
  const running = inv.job?.state === "running";
  return (
    <Stack gap={6} p="xs">
      <Text size="xs" c="dimmed">
        {what} needs the inventory: one pass over every object and page.
      </Text>
      <Button
        size="compact-xs"
        variant="light"
        disabled={running}
        onClick={inv.start}
        style={{ alignSelf: "flex-start" }}
      >
        Build inventory
      </Button>
      {inv.job && (
        <>
          <Progress size="sm" value={inv.job.total ? (100 * inv.job.done) / inv.job.total : 100} animated={running} />
          <Text size="xs" c="dimmed">
            {inv.job.done.toLocaleString()} / {inv.job.total.toLocaleString()}
            {inv.job.state === "cancelled" && " · cancelled"}
            {running && backend.mode === "browser" && " · runs in this tab; pdfi serve is faster for large files"}
          </Text>
        </>
      )}
      {inv.error && (
        <Text size="xs" c="red">
          {inv.error}
        </Text>
      )}
    </Stack>
  );
}
