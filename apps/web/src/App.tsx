import {
  ActionIcon,
  Alert,
  AppShell,
  Badge,
  Button,
  Center,
  Group,
  Select,
  Stack,
  Tabs,
  Text,
  Title,
  Tooltip,
} from "@mantine/core";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useState, type DragEvent } from "react";
import { api, useBackend } from "./api";
import type { ConnectionState } from "./backend/backend";
import { DiagnosticsPanel } from "./components/DiagnosticsPanel";
import { HexView } from "./components/HexView";
import { Inspector } from "./components/Inspector";
import { ObjectList } from "./components/ObjectList";
import { OpenDialog } from "./components/OpenDialog";
import { SchemaPanel } from "./components/SchemaPanel";
import { ValidatePanel } from "./components/ValidatePanel";
import { SummaryCard } from "./components/SummaryCard";
import { XrefPanel } from "./components/XrefPanel";
import { useJob } from "./jobs";
import { NavContext, sameTarget, type Nav, type Selection, type Target } from "./nav";
import type { DocId, Span } from "./protocol/generated";

export function App() {
  return <Shell />;
}

function useConnection(): ConnectionState {
  const backend = useBackend();
  const [state, setState] = useState<ConnectionState>("connecting");
  useEffect(() => backend.onState(setState), [backend]);
  return state;
}

function Shell() {
  const backend = useBackend();
  const qc = useQueryClient();
  const conn = useConnection();
  const [doc, setDoc] = useState<DocId | null>(null);
  const [openDialog, setOpenDialog] = useState(false);

  const docs = useQuery({ queryKey: ["docs"], queryFn: () => api.docs(backend), staleTime: 0 });
  const [dropError, setDropError] = useState<string | null>(null);

  // Browser mode: drop a PDF anywhere to open it.
  const onDragOver = (e: DragEvent) => {
    if (backend.openFile && e.dataTransfer.types.includes("Files")) e.preventDefault();
  };
  const onDrop = (e: DragEvent) => {
    const file = e.dataTransfer.files[0];
    if (!backend.openFile || !file) return;
    e.preventDefault();
    setDropError(null);
    api.openFile(backend, file, null, false).then(
      async (s) => {
        // Select it only once the list knows it, or the list effect resets the selection.
        await qc.invalidateQueries({ queryKey: ["docs"] });
        setDoc(s.info.doc);
      },
      (err) => setDropError(`${file.name}: ${err instanceof Error ? err.message : String(err)}`),
    );
  };

  useEffect(
    () =>
      backend.onEvent((ev) => {
        if (ev.type === "docOpened" || ev.type === "docClosed") {
          void qc.invalidateQueries({ queryKey: ["docs"] });
        }
        if (ev.type === "docClosed") {
          qc.removeQueries({ queryKey: ["doc", ev.doc] });
          setDoc((d) => (d === ev.doc ? null : d));
        }
      }),
    [backend, qc],
  );

  // Refresh after reconnecting: the server may have been restarted.
  useEffect(() => {
    if (conn === "open") void qc.invalidateQueries({ queryKey: ["docs"] });
  }, [conn, qc]);

  useEffect(() => {
    const list = docs.data;
    if (!list) return;
    if (doc === null || !list.some((d) => d.doc === doc)) setDoc(list[0]?.doc ?? null);
  }, [docs.data, doc]);

  return (
    <AppShell header={{ height: 48 }} padding={0} onDragOver={onDragOver} onDrop={onDrop}>
      <AppShell.Header>
        <Group h="100%" px="sm" gap="sm" wrap="nowrap">
          <Title order={5} style={{ whiteSpace: "nowrap" }}>
            pdf-inspector
          </Title>
          <Badge variant="light" color={conn === "open" ? "teal" : conn === "connecting" ? "yellow" : "red"}>
            {backend.mode} · {conn}
          </Badge>
          <Select
            size="xs"
            w={320}
            placeholder="No document"
            data={(docs.data ?? []).map((d) => ({ value: String(d.doc), label: d.name }))}
            value={doc === null ? null : String(doc)}
            onChange={(v) => setDoc(v === null ? null : Number(v))}
            allowDeselect={false}
          />
          <Button size="xs" variant="default" onClick={() => setOpenDialog(true)}>
            Open…
          </Button>
          {doc !== null && (
            <Button size="xs" variant="subtle" color="gray" onClick={() => void api.close(backend, doc)}>
              Close
            </Button>
          )}
        </Group>
      </AppShell.Header>
      <AppShell.Main h="100vh" style={{ display: "flex", flexDirection: "column" }}>
        {dropError && (
          <Alert color="red" m="xs" p={6} withCloseButton onClose={() => setDropError(null)}>
            <Text size="xs">{dropError}</Text>
          </Alert>
        )}
        {doc === null ? (
          <Center style={{ flex: 1 }}>
            <Stack align="center">
              <Text c="dimmed">No document open.</Text>
              <Button onClick={() => setOpenDialog(true)}>Open a PDF…</Button>
              {backend.mode === "browser" && (
                <Text size="xs" c="dimmed" maw={420} ta="center">
                  Or drop a file here. It is read in this tab only and never uploaded. For files of hundreds of MB,
                  use the local server (<code>pdfi serve</code>).
                </Text>
              )}
            </Stack>
          </Center>
        ) : (
          <DocView key={doc} doc={doc} />
        )}
      </AppShell.Main>
      <OpenDialog opened={openDialog} onClose={() => setOpenDialog(false)} onOpened={(d) => setDoc(d)} />
    </AppShell>
  );
}

function DocView({ doc }: { doc: DocId }) {
  const backend = useBackend();
  const summary = useQuery({ queryKey: ["doc", doc, "summary"], queryFn: () => api.summary(backend, doc) });
  const [history, setHistory] = useState<Target[]>([{ kind: "trailer" }]);
  const [hexSpan, setHexSpan] = useState<Span | null>(null);
  const [tab, setTab] = useState<string | null>("objects");
  const [selection, setSelection] = useState<Selection | null>(null);
  const [validation, setValidation] = useJob(doc);

  const open = useCallback((t: Target) => {
    setHistory((h) => (sameTarget(h[h.length - 1], t) ? h : [...h.slice(-99), t]));
    setSelection(null);
  }, []);
  const nav: Nav = useMemo(
    () => ({
      doc,
      target: history[history.length - 1],
      open,
      back: () => {
        setHistory((h) => (h.length > 1 ? h.slice(0, -1) : h));
        setSelection(null);
      },
      canBack: history.length > 1,
      showBytes: setHexSpan,
      hexSpan,
      selection,
      select: setSelection,
    }),
    [doc, history, open, hexSpan, selection],
  );

  if (summary.error) {
    return (
      <Alert color="red" m="md" title="Cannot load document">
        {String(summary.error)}
      </Alert>
    );
  }
  if (!summary.data) return null;
  const s = summary.data;

  return (
    <NavContext.Provider value={nav}>
      <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
        <div
          style={{
            width: 380,
            borderRight: "1px solid var(--mantine-color-default-border)",
            display: "flex",
            flexDirection: "column",
            minHeight: 0,
          }}
        >
          <SummaryCard summary={s} />
          <Tabs value={tab} onChange={setTab} style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0 }}>
            <Tabs.List>
              <Tabs.Tab value="objects">Objects</Tabs.Tab>
              <Tabs.Tab value="xref">XRef</Tabs.Tab>
              <Tabs.Tab value="validate">Validate</Tabs.Tab>
              <Tabs.Tab value="diagnostics">
                Diagnostics {s.diagnostics > 0 && <Badge size="xs" color="orange">{s.diagnostics}</Badge>}
              </Tabs.Tab>
            </Tabs.List>
            <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
              {tab === "objects" && <ObjectList total={s.xref_entries} />}
              {tab === "xref" && <XrefPanel summary={s} />}
              {tab === "validate" && <ValidatePanel job={validation} setJob={setValidation} />}
              {tab === "diagnostics" && <DiagnosticsPanel total={s.diagnostics} />}
            </div>
          </Tabs>
        </div>
        <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
          <Group px="sm" py={4} gap="xs" style={{ borderBottom: "1px solid var(--mantine-color-default-border)" }}>
            <Tooltip label="Back">
              <ActionIcon variant="subtle" disabled={!nav.canBack} onClick={nav.back} aria-label="Back">
                ←
              </ActionIcon>
            </Tooltip>
            <Button size="compact-xs" variant="subtle" onClick={() => open({ kind: "trailer" })}>
              Trailer
            </Button>
            {s.root && (
              <Button size="compact-xs" variant="subtle" onClick={() => open({ kind: "object", id: s.root! })}>
                Catalog
              </Button>
            )}
            {s.info_dict && (
              <Button size="compact-xs" variant="subtle" onClick={() => open({ kind: "object", id: s.info_dict! })}>
                Info
              </Button>
            )}
          </Group>
          <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
            <Inspector />
          </div>
          <div style={{ height: 280, borderTop: "1px solid var(--mantine-color-default-border)" }}>
            <HexView size={s.info.size} />
          </div>
        </div>
        <div
          style={{
            width: 300,
            flexShrink: 0,
            borderLeft: "1px solid var(--mantine-color-default-border)",
            overflow: "auto",
          }}
        >
          <SchemaPanel />
        </div>
      </div>
    </NavContext.Provider>
  );
}
