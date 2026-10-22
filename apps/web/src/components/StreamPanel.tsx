import { Alert, Button, Code, Group, SegmentedControl, Stack, Text } from "@mantine/core";
import { useInfiniteQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api, useBackend } from "../api";
import { bytesAsText, bytesText } from "../format";
import { useNav } from "../nav";
import type { ObjRef, StreamInfo } from "../protocol/generated";
import { BytesDump } from "./BytesDump";

const CHUNK = 256 * 1024;

export function StreamPanel({ id, info }: { id: ObjRef; info: StreamInfo }) {
  const backend = useBackend();
  const nav = useNav();
  const [decoded, setDecoded] = useState(true);
  const [view, setView] = useState<"text" | "hex">("text");

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
