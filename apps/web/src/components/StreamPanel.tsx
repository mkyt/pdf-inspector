import {
  Alert,
  Button,
  Code,
  FileInput,
  Group,
  Loader,
  Menu,
  Modal,
  SegmentedControl,
  Stack,
  Text,
  Textarea,
} from "@mantine/core";
import { useInfiniteQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { api, useBackend } from "../api";
import { base64, fromLatin1, latin1, useEdit } from "../edit";
import { bytesAsText, bytesText } from "../format";
import { useNav } from "../nav";
import type { ObjRef, StreamFilter, StreamInfo } from "../protocol/generated";
import { BytesDump } from "./BytesDump";

const CHUNK = 256 * 1024;

export function StreamPanel({ id, info }: { id: ObjRef; info: StreamInfo }) {
  const backend = useBackend();
  const nav = useNav();
  const { run } = useEdit();
  const [decoded, setDecoded] = useState(true);
  const [view, setView] = useState<"text" | "hex">("text");
  const [editing, setEditing] = useState(false);

  const q = useInfiniteQuery({
    queryKey: ["doc", nav.doc, "stream", id.num, id.generation, decoded],
    queryFn: ({ pageParam }) => api.streamData(backend, nav.doc, id, decoded, pageParam, CHUNK),
    initialPageParam: 0,
    getNextPageParam: (last) => (last.info.eof ? undefined : last.info.offset + last.info.len),
  });

  const pages = q.data?.pages ?? [];
  const data = concat(pages.map((p) => p.data));
  const lastInfo = pages[pages.length - 1]?.info;
  const errors = [...new Set(pages.flatMap((p) => p.info.errors))];

  return (
    <Stack gap="xs">
      <Group gap="sm">
        <SegmentedControl
          size="xs"
          value={decoded ? "decoded" : "raw"}
          onChange={(v) => setDecoded(v === "decoded")}
          data={[
            { value: "decoded", label: "Decoded" },
            { value: "raw", label: "Raw" },
          ]}
        />
        <SegmentedControl
          size="xs"
          value={view}
          onChange={(v) => setView(v as "text" | "hex")}
          data={[
            { value: "text", label: "Text" },
            { value: "hex", label: "Hex" },
          ]}
        />
        <Text size="xs" c="dimmed">
          filters: {info.filters.length ? info.filters.join(", ") : "none"}
          {info.encrypted && " · encrypted"} · stored {bytesText(info.raw_len)}
          {" · "}
          {lastInfo?.total != null ? `${bytesText(lastInfo.total)} total` : `${bytesText(data.length)} loaded`}
        </Text>
        {info.data_span && (
          <Button size="compact-xs" variant="subtle" onClick={() => nav.showBytes(info.data_span!)}>
            Show in file
          </Button>
        )}
        <Group gap={4} ml="auto">
          <Button size="compact-xs" variant="light" onClick={() => setEditing(true)}>
            Edit data…
          </Button>
          <Menu position="bottom-end" shadow="md">
            <Menu.Target>
              <Button size="compact-xs" variant="subtle">
                Encoding
              </Button>
            </Menu.Target>
            <Menu.Dropdown>
              <Menu.Item
                fz="xs"
                onClick={() => run("Compress the stream", [{ op: "setStreamFilter", id, filter: "flate" }])}
              >
                Compress (FlateDecode)
              </Menu.Item>
              <Menu.Item
                fz="xs"
                onClick={() => run("Decompress the stream", [{ op: "setStreamFilter", id, filter: "none" }])}
              >
                Store decoded (no filter)
              </Menu.Item>
              <Menu.Label>Image codecs (DCT, JPX, JBIG2, CCITT) stay.</Menu.Label>
            </Menu.Dropdown>
          </Menu>
        </Group>
      </Group>
      {lastInfo?.stopped_at && decoded && (
        <Alert p={6} color="blue">
          <Text size="xs">Data is still /{lastInfo.stopped_at} encoded (image codec or unsupported filter).</Text>
        </Alert>
      )}
      {errors.map((e) => (
        <Alert key={e} p={6} color="red">
          <Text size="xs">{e}</Text>
        </Alert>
      ))}
      {q.error && <Text c="red" size="xs">{String(q.error)}</Text>}
      {view === "text" ? (
        <Code block style={{ maxHeight: 480, overflow: "auto", whiteSpace: "pre-wrap", wordBreak: "break-all" }}>
          {bytesAsText(data)}
        </Code>
      ) : (
        <BytesDump data={data} height={420} />
      )}
      {q.hasNextPage && (
        <Button size="xs" variant="light" loading={q.isFetchingNextPage} onClick={() => void q.fetchNextPage()} w={200}>
          Load next {bytesText(CHUNK)}
        </Button>
      )}
      <Modal opened={editing} onClose={() => setEditing(false)} title={`Stream data of ${id.num} ${id.generation}`} size="xl">
        {editing && <DataEditor id={id} info={info} onClose={() => setEditing(false)} />}
      </Modal>
    </Stack>
  );
}

/** Largest decoded data edited as text. */
const MAX_TEXT = 4 << 20;

/** Control characters other than tab, line feed, form feed and carriage return: binary data. */
const BINARY = /[\u0000-\u0008\u000b\u000e-\u001f]/;

/** Replace the decoded data: as text (bytes as Latin-1 characters), or from a file. */
function DataEditor({ id, info, onClose }: { id: ObjRef; info: StreamInfo; onClose: () => void }) {
  const backend = useBackend();
  const nav = useNav();
  const { apply } = useEdit();
  const [text, setText] = useState<string | null>(null);
  const [binary, setBinary] = useState(false);
  const [stoppedAt, setStoppedAt] = useState<string | null>(null);
  const [file, setFile] = useState<File | null>(null);
  const [filter, setFilter] = useState<StreamFilter>(info.filters.includes("FlateDecode") ? "flate" : "none");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let live = true;
    api.streamData(backend, nav.doc, id, true, 0, MAX_TEXT).then(
      (b) => {
        if (!live) return;
        const t = latin1(b.data);
        setStoppedAt(b.info.stopped_at);
        setBinary(!b.info.eof || BINARY.test(t) || b.info.errors.length > 0);
        setText(t);
      },
      (e) => live && setError(String(e)),
    );
    return () => {
      live = false;
    };
  }, [backend, nav.doc, id]);

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const data = binary ? (file ? new Uint8Array(await file.arrayBuffer()) : null) : fromLatin1(text ?? "");
      if (!data) {
        setError(binary ? "Choose a file." : "Only characters up to U+00FF can be stored as bytes.");
        return;
      }
      await apply(`Edit the data of ${id.num} ${id.generation}`, [
        { op: "setStreamData", id, data: { kind: "base64", data: base64(data) }, filter },
      ]);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  if (text === null && !error) return <Loader size="sm" />;
  return (
    <Stack gap="xs">
      {stoppedAt && (
        <Alert p={6} color="blue">
          <Text size="xs">The data stays /{stoppedAt} encoded: what you give is that codec&apos;s input.</Text>
        </Alert>
      )}
      {binary ? (
        <>
          <Text size="xs" c="dimmed">
            The data is binary (or too large to edit as text): replace it with a file.
          </Text>
          <FileInput size="xs" label="New data" placeholder="Choose a file" value={file} onChange={setFile} clearable />
        </>
      ) : (
        <Textarea
          size="xs"
          description="Each character is one byte (Latin-1). Line breaks are stored as LF."
          styles={{ input: { fontFamily: "var(--mantine-font-family-monospace)", fontSize: 12 } }}
          autosize
          minRows={8}
          maxRows={28}
          value={text ?? ""}
          onChange={(e) => setText(e.currentTarget.value)}
        />
      )}
      <Group gap="xs">
        <Text size="xs">Store</Text>
        <SegmentedControl
          size="xs"
          value={filter}
          onChange={(v) => setFilter(v as StreamFilter)}
          data={[
            { value: "flate", label: "Compressed (Flate)" },
            { value: "none", label: "Uncompressed" },
          ]}
        />
        {!binary && (
          <Button size="compact-xs" variant="subtle" ml="auto" onClick={() => setBinary(true)}>
            Replace with a file instead
          </Button>
        )}
      </Group>
      {error && (
        <Alert color="red" p={6}>
          <Text size="xs">{error}</Text>
        </Alert>
      )}
      <Group justify="flex-end">
        <Button size="xs" variant="default" onClick={onClose}>
          Cancel
        </Button>
        <Button size="xs" loading={busy} onClick={() => void submit()}>
          Apply
        </Button>
      </Group>
    </Stack>
  );
}

function concat(parts: Uint8Array[]): Uint8Array {
  if (parts.length === 1) return parts[0];
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
}
