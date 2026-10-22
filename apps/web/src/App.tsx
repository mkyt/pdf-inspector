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
import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent } from "react";
import { api, useBackend } from "./api";
import type { ConnectionState } from "./backend/backend";
import { DiagnosticsPanel } from "./components/DiagnosticsPanel";
import { DocumentPanel } from "./components/DocumentPanel";
import { FontsPanel, ImagesPanel } from "./components/InventoryPanels";
import { OutlinePanel } from "./components/OutlinePanel";
import { PageList } from "./components/PageList";
import { PageView } from "./components/PageView";
import { ContentView } from "./components/ContentView";
import { HexView } from "./components/HexView";
import { Inspector } from "./components/Inspector";
import { ObjectList } from "./components/ObjectList";
import { OpenDialog } from "./components/OpenDialog";
import { SchemaPanel } from "./components/SchemaPanel";
import { ValidatePanel } from "./components/ValidatePanel";
import { SummaryCard } from "./components/SummaryCard";
import { XrefPanel } from "./components/XrefPanel";
import { DocContext, type DocState } from "./doc";
import { useJob } from "./jobs";
import { contentKey, NavContext, sameTarget, type Nav, type Selection, type Target } from "./nav";
import type { DocId, Span } from "./protocol/generated";
import { DEFAULT_TAB, useRouter, type Router } from "./route";

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
  const router = useRouter();
  const { route } = router;
  const doc = route.doc;
  const routeRef = useRef(route);
  routeRef.current = route;
  const showDoc = useCallback(
    (d: DocId) => router.push({ doc: d, target: { kind: "document" }, tab: routeRef.current.tab }),
    [router.push],
  );
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
        showDoc(s.info.doc);
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
          if (routeRef.current.doc === ev.doc) router.replace({ ...routeRef.current, doc: null });
        }
      }),
    [backend, qc, router.replace],
  );

  // Refresh after reconnecting: the server may have been restarted.
  useEffect(() => {
    if (conn === "open") void qc.invalidateQueries({ queryKey: ["docs"] });
  }, [conn, qc]);

  // Show the first document when none is chosen, or the URL names one that is not open (any more).
  const known = doc !== null && !!docs.data?.some((d) => d.doc === doc);
  useEffect(() => {
    const list = docs.data;
    if (!list || known) return;
    const first = list[0]?.doc ?? null;
    if (first !== doc) router.replace({ doc: first, target: { kind: "document" }, tab: DEFAULT_TAB });
  }, [docs.data, doc, known, router.replace]);

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
            onChange={(v) => v !== null && showDoc(Number(v))}
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
        {docs.isPending ? null : !known ? (
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
          <DocView key={doc} doc={doc!} router={router} />
        )}
      </AppShell.Main>
      <OpenDialog opened={openDialog} onClose={() => setOpenDialog(false)} onOpened={showDoc} />
    </AppShell>
  );
}

function DocView({ doc, router }: { doc: DocId; router: Router }) {
  const backend = useBackend();
  const summary = useQuery({ queryKey: ["doc", doc, "summary"], queryFn: () => api.summary(backend, doc) });
  const { route, push, replace } = router;
  const target = route.target;
  const tab = route.tab;
  const setTab = (t: string | null) => t && replace({ ...route, tab: t });
  const [hexSpan, setHexSpan] = useState<Span | null>(null);
  // A selection belongs to the target it was made in; going elsewhere (or back) drops it.
  const [picked, setPicked] = useState<{ target: Target; selection: Selection } | null>(null);
  const selection = picked && sameTarget(picked.target, target) ? picked.selection : null;
  const [validation, setValidation] = useJob(doc);
  const [inventoryJob, setInventoryJob] = useJob(doc);
  const [inventoryError, setInventoryError] = useState<string | null>(null);

  const open = useCallback(
    (t: Target) => {
      if (!sameTarget(target, t)) push({ ...route, target: t });
    },
    [route, target, push],
  );
  const nav: Nav = useMemo(
    () => ({
      doc,
      target,
      open,
      update: (t: Target) => replace({ ...route, target: t }),
      back: router.back,
      canBack: router.canBack,
      showBytes: setHexSpan,
      hexSpan,
      selection,
      select: (s) => setPicked(s && { target, selection: s }),
    }),
    [doc, target, open, replace, route, router.back, router.canBack, hexSpan, selection],
  );

  const docState: DocState | null = useMemo(
    () =>
      summary.data
        ? {
            summary: summary.data,
            inventory: {
              stats: summary.data.inventory,
              job: inventoryJob,
              error: inventoryError,
              start: () => {
                setInventoryError(null);
                api.startJob(backend, doc, { kind: "inventory" }).then(setInventoryJob, (e) => setInventoryError(String(e)));
              },
            },
          }
        : null,
    [summary.data, inventoryJob, inventoryError, backend, doc, setInventoryJob],
  );

  if (summary.error) {
    return (
      <Alert color="red" m="md" title="Cannot load document">
        {String(summary.error)}
      </Alert>
    );
  }
  if (!summary.data || !docState) return null;
  const s = summary.data;
  const t = nav.target;

  return (
    <DocContext.Provider value={docState}>
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
                <Tabs.Tab value="pages">Pages</Tabs.Tab>
                <Tabs.Tab value="outline">Outline</Tabs.Tab>
                <Tabs.Tab value="fonts">Fonts</Tabs.Tab>
                <Tabs.Tab value="images">Images</Tabs.Tab>
                <Tabs.Tab value="objects">Objects</Tabs.Tab>
                <Tabs.Tab value="xref">XRef</Tabs.Tab>
                <Tabs.Tab value="validate">Validate</Tabs.Tab>
                <Tabs.Tab value="diagnostics">
                  Diagnostics {s.diagnostics > 0 && <Badge size="xs" color="orange">{s.diagnostics}</Badge>}
                </Tabs.Tab>
              </Tabs.List>
              <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
                {tab === "pages" && <PageList />}
                {tab === "outline" && <OutlinePanel />}
                {tab === "fonts" && <FontsPanel />}
                {tab === "images" && <ImagesPanel />}
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
              <Button size="compact-xs" variant={t.kind === "document" ? "light" : "subtle"} onClick={() => open({ kind: "document" })}>
                Document
              </Button>
              <Button size="compact-xs" variant={t.kind === "trailer" ? "light" : "subtle"} onClick={() => open({ kind: "trailer" })}>
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
              {t.kind === "document" ? (
                <DocumentPanel />
              ) : t.kind === "page" ? (
                <PageView key={t.index} index={t.index} />
              ) : t.kind === "content" ? (
                <ContentView key={contentKey(t.content).join("/")} content={t.content} op={t.op} />
              ) : (
                <Inspector />
              )}
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
    </DocContext.Provider>
  );
}
