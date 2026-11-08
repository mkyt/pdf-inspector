import { Alert, Button, Checkbox, Group, Modal, Progress, Radio, Stack, Text, TextInput } from "@mantine/core";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { api, useBackend } from "../api";
import { useDoc } from "../doc";
import { fileHandles, useEdit } from "../edit";
import { bytesText } from "../format";
import { useJob } from "../jobs";
import type { DocId, SaveMode, SaveResult } from "../protocol/generated";

// The File System Access API (Chromium) is not in TypeScript's DOM library.
declare global {
  interface Window {
    showSaveFilePicker?(options?: {
      suggestedName?: string;
      types?: { description?: string; accept: Record<string, string[]> }[];
    }): Promise<FileSystemFileHandle>;
  }
}

const PDF_TYPES = [{ description: "PDF document", accept: { "application/pdf": [".pdf"] } }];

/** Save the document: an incremental update or a full rewrite, over the file or to another one. */
export function SaveDialog({
  doc,
  opened,
  mode,
  onClose,
}: {
  doc: DocId;
  opened: boolean;
  /** Chosen at first (a macro's `meta.save`). */
  mode?: SaveMode | null;
  onClose: () => void;
}) {
  return (
    <Modal opened={opened} onClose={onClose} title="Save" size="lg">
      {opened && <Form doc={doc} initial={mode ?? null} onClose={onClose} />}
    </Modal>
  );
}

function Form({ doc, initial, onClose }: { doc: DocId; initial: SaveMode | null; onClose: () => void }) {
  const backend = useBackend();
  const qc = useQueryClient();
  const { summary } = useDoc();
  const { status, report } = useEdit();
  const browser = backend.saveFile !== undefined;
  const incremental = status.no_incremental === null;
  const asked = initial?.kind === "full" ? initial : null;
  const [full, setFull] = useState(!incremental || asked !== null);
  const [xrefStream, setXrefStream] = useState(asked?.xref_stream ?? true);
  const [objectStreams, setObjectStreams] = useState(asked?.object_streams ?? true);
  const [compress, setCompress] = useState(asked?.compress ?? true);
  const [removeUnused, setRemoveUnused] = useState(asked?.remove_unused ?? false);
  const [path, setPath] = useState(summary.info.path ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<SaveResult | null>(null);
  const [job, setJob] = useJob(doc);

  const mode: SaveMode = full
    ? {
        kind: "full",
        xref_stream: xrefStream || objectStreams,
        object_streams: objectStreams,
        compress,
        remove_unused: removeUnused,
      }
    : { kind: "incremental" };

  // Server: the job reports its end.
  useEffect(() => {
    if (!job || job.state === "running") return;
    setBusy(false);
    if (job.state === "completed" && job.result?.kind === "save") {
      const { kind: _, ...r } = job.result;
      setResult(r);
      void qc.invalidateQueries({ queryKey: ["docs"] });
    } else if (job.state === "failed") {
      setError(job.error ?? "saving failed");
    } else if (job.state === "cancelled") {
      setError("cancelled");
    }
  }, [job, qc]);

  const saveTo = async (handle: FileSystemFileHandle) => {
    setBusy(true);
    setError(null);
    try {
      const r = await api.saveFile(backend, doc, mode, handle);
      fileHandles.set(doc, handle);
      setResult(r.result);
      void qc.invalidateQueries({ queryKey: ["docs"] });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  // No File System Access API (Safari, Firefox): written in memory, then downloaded.
  const saveDownload = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await api.saveFile(backend, doc, mode, { download: summary.info.name });
      if (r.file) {
        const url = URL.createObjectURL(r.file);
        const a = document.createElement("a");
        a.href = url;
        a.download = r.file.name;
        a.click();
        setTimeout(() => URL.revokeObjectURL(url), 60_000);
      }
      setResult({ ...r.result, path: null });
      void qc.invalidateQueries({ queryKey: ["docs"] });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const saveAs = async () => {
    if (!window.showSaveFilePicker) return saveDownload();
    let handle: FileSystemFileHandle;
    try {
      handle = await window.showSaveFilePicker({ suggestedName: summary.info.name, types: PDF_TYPES });
    } catch {
      return; // Dismissed.
    }
    await saveTo(handle);
  };

  const saveServer = async (to: string | null) => {
    setBusy(true);
    setError(null);
    try {
      setJob(await api.startJob(backend, doc, { kind: "save", path: to, mode }));
    } catch (e) {
      setBusy(false);
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const known = fileHandles.get(doc);
  if (result) {
    return (
      <Stack gap="xs">
        <Alert color="teal" p={8}>
          <Text size="xs">
            Wrote {bytesText(result.bytes)} ({result.objects.toLocaleString()} objects)
            {result.path ? ` to ${result.path}` : " and downloaded it"}. The document was reopened from the saved file; the undo history starts
            over.
          </Text>
        </Alert>
        {result.warnings.map((w) => (
          <Alert key={w} color="orange" p={6}>
            <Text size="xs">{w}</Text>
          </Alert>
        ))}
        <Group justify="flex-end">
          <Button
            size="xs"
            onClick={() => {
              if (result.warnings.length > 0) report("orange", "Saved with warnings", result.warnings);
              onClose();
            }}
          >
            Done
          </Button>
        </Group>
      </Stack>
    );
  }
  return (
    <Stack gap="sm">
      <Text size="xs" c="dimmed">
        {status.dirty
          ? `${status.edited_objects.toLocaleString()} object(s) changed since the file was opened.`
          : "No changes: saving writes the file as it is (a full rewrite can still compact it)."}
      </Text>
      <Radio.Group value={full ? "full" : "incremental"} onChange={(v) => setFull(v === "full")}>
        <Stack gap={6}>
          <Radio
            size="xs"
            value="incremental"
            disabled={!incremental}
            label="Incremental update"
            description={
              status.no_incremental ??
              "The file's bytes stay as they are; the changed objects and a new cross-reference section are appended. Keeps signatures valid."
            }
          />
          <Radio
            size="xs"
            value="full"
            label="Full rewrite"
            description="Every object is written anew: decrypted, without the old revisions."
          />
        </Stack>
      </Radio.Group>
      {full && (
        <Stack gap={4} pl="lg">
          <Checkbox
            size="xs"
            label="Object streams (smaller; needs PDF 1.5)"
            checked={objectStreams}
            onChange={(e) => setObjectStreams(e.currentTarget.checked)}
          />
          <Checkbox
            size="xs"
            label="Cross-reference stream"
            checked={xrefStream || objectStreams}
            disabled={objectStreams}
            onChange={(e) => setXrefStream(e.currentTarget.checked)}
          />
          <Checkbox
            size="xs"
            label="Compress streams that have no filter"
            checked={compress}
            onChange={(e) => setCompress(e.currentTarget.checked)}
          />
          <Checkbox
            size="xs"
            label="Leave out objects nothing refers to"
            checked={removeUnused}
            onChange={(e) => setRemoveUnused(e.currentTarget.checked)}
          />
        </Stack>
      )}
      {status.signatures > 0 && (
        <Alert color={full ? "red" : "orange"} p={6}>
          <Text size="xs">
            {status.signatures} signed signature field(s).{" "}
            {full
              ? "A full rewrite invalidates the signatures."
              : "An update keeps the signed bytes, but viewers may flag the changes made after signing."}
          </Text>
        </Alert>
      )}
      {!browser && (
        <TextInput
          size="xs"
          label="File on the server"
          description="Within the locations the server allows. Written to a temporary file, then renamed over the target."
          value={path}
          onChange={(e) => setPath(e.currentTarget.value)}
        />
      )}
      {job?.state === "running" && <Progress size="sm" value={job.total ? (100 * job.done) / job.total : 0} />}
      {error && (
        <Alert color="red" p={6}>
          <Text size="xs" style={{ whiteSpace: "pre-wrap" }}>
            {error}
          </Text>
        </Alert>
      )}
      <Group justify="flex-end" gap="xs">
        <Button size="xs" variant="default" onClick={onClose}>
          Cancel
        </Button>
        {browser ? (
          <>
            {known && (
              <Button size="xs" variant="light" loading={busy} onClick={() => void saveTo(known)}>
                Save to {known.name}
              </Button>
            )}
            <Button size="xs" loading={busy} onClick={() => void saveAs()}>
              {window.showSaveFilePicker ? "Save as…" : "Download"}
            </Button>
          </>
        ) : (
          <Button size="xs" loading={busy} disabled={!path.trim()} onClick={() => void saveServer(path.trim())}>
            {path.trim() === summary.info.path ? "Save over the file" : "Save"}
          </Button>
        )}
      </Group>
    </Stack>
  );
}
