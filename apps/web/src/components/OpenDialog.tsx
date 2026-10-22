import { Alert, Button, Checkbox, Group, Loader, Modal, PasswordInput, ScrollArea, Stack, Text, TextInput } from "@mantine/core";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api, useBackend } from "../api";
import { bytesText } from "../format";
import type { DocId } from "../protocol/generated";
import { Row } from "./Row";

export function OpenDialog({
  opened,
  onClose,
  onOpened,
}: {
  opened: boolean;
  onClose: () => void;
  onOpened: (doc: DocId) => void;
}) {
  const backend = useBackend();
  const qc = useQueryClient();
  const [dir, setDir] = useState<string | null>(null);
  const [file, setFile] = useState("");
  const [password, setPassword] = useState("");
  const [recover, setRecover] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const listing = useQuery({
    queryKey: ["dir", dir],
    queryFn: () => api.listDir(backend, dir),
    enabled: opened,
    staleTime: 0,
  });

  const open = async (path: string) => {
    setBusy(true);
    setError(null);
    try {
      const s = await api.openPath(backend, path, password || null, recover);
      await qc.invalidateQueries({ queryKey: ["docs"] });
      onOpened(s.info.doc);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const l = listing.data;
  return (
    <Modal opened={opened} onClose={onClose} title="Open a PDF" size="lg">
      <Stack gap="xs">
        <Group gap="xs">
          <Button size="compact-xs" variant="default" onClick={() => setDir(null)}>
            Roots
          </Button>
          {l?.parent && (
            <Button size="compact-xs" variant="default" onClick={() => setDir(l.parent)}>
              Up
            </Button>
          )}
          <Text size="xs" c="dimmed" style={{ wordBreak: "break-all" }}>
            {l?.path ?? "Allowed locations"}
          </Text>
        </Group>
        <ScrollArea h={320} type="auto" style={{ border: "1px solid var(--mantine-color-default-border)" }}>
          {listing.isLoading && <Loader size="sm" m="sm" />}
          {listing.error && (
            <Text c="red" size="xs" p="xs">
              {String(listing.error)}
            </Text>
          )}
          {l?.entries.map((e) => (
            <div key={e.path} style={{ height: 26 }}>
              <Row
                active={e.path === file}
                onClick={() => (e.is_dir ? setDir(e.path) : setFile(e.path))}
              >
                <Text size="xs" w={16}>
                  {e.is_dir ? "▸" : ""}
                </Text>
                <Text size="xs" style={{ flex: 1 }} fw={e.name.toLowerCase().endsWith(".pdf") ? 600 : undefined}>
                  {e.name}
                </Text>
                {e.size !== null && (
                  <Text size="xs" c="dimmed">
                    {bytesText(e.size)}
                  </Text>
                )}
              </Row>
            </div>
          ))}
        </ScrollArea>
        <TextInput size="xs" label="File" value={file} onChange={(e) => setFile(e.currentTarget.value)} />
        <Group grow>
          <PasswordInput
            size="xs"
            label="Password (if encrypted)"
            value={password}
            onChange={(e) => setPassword(e.currentTarget.value)}
          />
          <Checkbox
            size="xs"
            mt="lg"
            label="Ignore xref, rebuild by scanning"
            checked={recover}
            onChange={(e) => setRecover(e.currentTarget.checked)}
          />
        </Group>
        {error && (
          <Alert color="red" p={6}>
            <Text size="xs">{error}</Text>
          </Alert>
        )}
        <Group justify="flex-end">
          <Button size="xs" disabled={!file} loading={busy} onClick={() => void open(file)}>
            Open
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}
