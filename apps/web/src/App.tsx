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
  useMantineColorScheme,
  type MantineColorScheme,
} from "@mantine/core";
import { useHotkeys } from "@mantine/hooks";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent } from "react";
import { api, useBackend } from "./api";
import type { ConnectionState } from "./backend/backend";
import { Breadcrumbs } from "./components/Breadcrumbs";
import { DiagnosticsPanel } from "./components/DiagnosticsPanel";
import { DocumentPanel } from "./components/DocumentPanel";
import { FontsPanel, ImagesPanel } from "./components/InventoryPanels";
import { OutlinePanel } from "./components/OutlinePanel";
import { PageList } from "./components/PageList";
import { PageView } from "./components/PageView";
import { ContentView } from "./components/ContentView";
import { FontView } from "./components/FontView";
import { HexView } from "./components/HexView";
import {
  AutoThemeIcon,
  BackIcon,
  BottomPaneIcon,
  ForwardIcon,
  LeftPaneIcon,
  MoonIcon,
  RedoIcon,
  RightPaneIcon,
  SunIcon,
  UndoIcon,
} from "./components/Icons";
import { Inspector } from "./components/Inspector";
import { Landmarks } from "./components/Landmarks";
import { ObjectList } from "./components/ObjectList";
import { OpenDialog } from "./components/OpenDialog";
import { SchemaPanel } from "./components/SchemaPanel";
import { ResizeHandle } from "./components/Split";
import { ValidatePanel } from "./components/ValidatePanel";
import { SummaryCard } from "./components/SummaryCard";
import { XrefPanel } from "./components/XrefPanel";
import { DocContext, type DocState } from "./doc";
import { EditProvider, fileHandles, useEdit } from "./edit";
import { SaveDialog } from "./components/SaveDialog";
import { useJob } from "./jobs";
import { useLayout, type Layout } from "./layout";
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

const THEMES: { value: MantineColorScheme; label: string; icon: () => React.JSX.Element }[] = [
  { value: "auto", label: "Theme: follow the system", icon: AutoThemeIcon },
  { value: "light", label: "Theme: light", icon: SunIcon },
  { value: "dark", label: "Theme: dark", icon: MoonIcon },
];

/** Cycles through system, light and dark (kept in this browser by Mantine). */
function ThemeToggle() {
  const { colorScheme, setColorScheme } = useMantineColorScheme();
  const i = Math.max(
    0,
    THEMES.findIndex((t) => t.value === colorScheme),
  );
  const { label, icon: Icon } = THEMES[i];
  return (
    <Tooltip label={label}>
      <ActionIcon
        variant="subtle"
        color="gray"
        aria-label={label}
        onClick={() => setColorScheme(THEMES[(i + 1) % THEMES.length].value)}
      >
        <Icon />
      </ActionIcon>
    </Tooltip>
  );
}

function PaneToggles({ layout }: { layout: Layout }) {
  const items = [
    { pane: layout.left, label: "Sidebar", keys: "Ctrl/⌘+B", Icon: LeftPaneIcon },
    { pane: layout.hex, label: "File bytes", keys: "Ctrl/⌘+J", Icon: BottomPaneIcon },
    { pane: layout.right, label: "Schema panel", keys: "Ctrl/⌘+Alt+B", Icon: RightPaneIcon },
  ];
  return (
    <Group gap={2} wrap="nowrap">
      {items.map(({ pane, label, keys, Icon }) => (
        <Tooltip key={label} label={`${pane.shown ? "Hide" : "Show"} ${label.toLowerCase()} (${keys})`}>
          <ActionIcon variant="subtle" color="gray" aria-label={label} aria-pressed={pane.shown} onClick={pane.toggle}>
            <Icon shown={pane.shown} />
          </ActionIcon>
        </Tooltip>
      ))}
    </Group>
  );
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
  const layout = useLayout();
  useHotkeys([
    ["mod+B", layout.left.toggle],
    ["mod+alt+B", layout.right.toggle],
    ["mod+J", layout.hex.toggle],
  ]);

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
    // Chromium hands out a handle to the dropped file: saving can write back to it.
    const item = e.dataTransfer.items[0] as DataTransferItem & {
      getAsFileSystemHandle?(): Promise<FileSystemHandle | null>;
    };
    const handle = item?.getAsFileSystemHandle?.();
    api.openFile(backend, file, null, false).then(
      async (s) => {
        const h = await handle?.catch(() => null);
        if (h?.kind === "file") fileHandles.set(s.info.doc, h as FileSystemFileHandle);
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
        if (ev.type === "docChanged") {
          // Everything read from it is stale (and saving may rename it).
          void qc.invalidateQueries({ queryKey: ["doc", ev.doc] });
          if (ev.info) void qc.invalidateQueries({ queryKey: ["docs"] });
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
          <Group gap={2} ml="auto" wrap="nowrap">
            {known && <PaneToggles layout={layout} />}
            <ThemeToggle />
          </Group>
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
          <DocView key={doc} doc={doc!} router={router} layout={layout} />
        )}
      </AppShell.Main>
      <OpenDialog opened={openDialog} onClose={() => setOpenDialog(false)} onOpened={showDoc} />
    </AppShell>
  );
}

function DocView({ doc, router, layout }: { doc: DocId; router: Router; layout: Layout }) {
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
  const [saving, setSaving] = useState(false);
  const openSave = useCallback(() => setSaving(true), []);
  // Jobs ran on the document before it changed: their results no longer apply.
  useEffect(
    () =>
      backend.onEvent((ev) => {
        if (ev.type === "docChanged" && ev.doc === doc) {
          setValidation(null);
          setInventoryJob(null);
        }
      }),
    [backend, doc, setValidation, setInventoryJob],
  );

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
      forward: router.forward,
      canForward: router.canForward,
      showBytes: setHexSpan,
      hexSpan,
      selection,
      select: (s) => setPicked(s && { target, selection: s }),
    }),
    [
      doc,
      target,
      open,
      replace,
      route,
      router.back,
      router.canBack,
      router.forward,
      router.canForward,
      hexSpan,
      selection,
    ],
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
  const { left, right, hex } = layout;

  return (
    <DocContext.Provider value={docState}>
      <NavContext.Provider value={nav}>
        <EditProvider doc={doc} status={s.edit} onSave={openSave}>
        <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
          {left.shown && (
            <div
              style={{
                width: left.size,
                flexShrink: 0,
                display: "flex",
                flexDirection: "column",
                minHeight: 0,
              }}
            >
              <SummaryCard summary={s} />
              <Landmarks />
              <Tabs
                value={tab}
                onChange={setTab}
                style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0 }}
              >
                <Tabs.List>
                  <Tabs.Tab value="pages">Pages</Tabs.Tab>
                  <Tabs.Tab value="outline">Outline</Tabs.Tab>
                  <Tabs.Tab value="fonts">Fonts</Tabs.Tab>
                  <Tabs.Tab value="images">Images</Tabs.Tab>
                  <Tabs.Tab value="objects">Objects</Tabs.Tab>
                  <Tabs.Tab value="xref">XRef</Tabs.Tab>
                  <Tabs.Tab value="validate">Validate</Tabs.Tab>
                  <Tabs.Tab value="diagnostics">
                    Diagnostics{" "}
                    {s.diagnostics > 0 && (
                      <Badge size="xs" color="orange">
                        {s.diagnostics}
                      </Badge>
                    )}
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
          )}
          {left.shown && <ResizeHandle pane={left} grow="right" label="sidebar" />}
          <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
            <Group
              px="sm"
              py={4}
              gap={4}
              wrap="nowrap"
              style={{ borderBottom: "1px solid var(--mantine-color-default-border)" }}
            >
              <Tooltip label="Back">
                <ActionIcon variant="subtle" disabled={!nav.canBack} onClick={nav.back} aria-label="Back">
                  <BackIcon />
                </ActionIcon>
              </Tooltip>
              <Tooltip label="Forward">
                <ActionIcon variant="subtle" disabled={!nav.canForward} onClick={nav.forward} aria-label="Forward">
                  <ForwardIcon />
                </ActionIcon>
              </Tooltip>
              <Breadcrumbs />
              <EditControls />
            </Group>
            <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
              {t.kind === "document" ? (
                <DocumentPanel />
              ) : t.kind === "page" ? (
                <PageView key={t.index} index={t.index} />
              ) : t.kind === "content" ? (
                <ContentView key={contentKey(t.content).join("/")} content={t.content} op={t.op} />
              ) : t.kind === "font" ? (
                <FontView key={`${t.id.num}/${t.id.generation}`} id={t.id} />
              ) : (
                <Inspector />
              )}
            </div>
            {hex.shown && (
              <>
                <ResizeHandle pane={hex} grow="up" label="file bytes" />
                <div style={{ height: hex.size, flexShrink: 0 }}>
                  <HexView size={s.info.size} />
                </div>
              </>
            )}
          </div>
          {right.shown && (
            <>
              <ResizeHandle pane={right} grow="left" label="schema panel" />
              <div style={{ width: right.size, flexShrink: 0, overflow: "auto" }}>
                <SchemaPanel />
              </div>
            </>
          )}
        </div>
        <SaveDialog doc={doc} opened={saving} onClose={() => setSaving(false)} />
        </EditProvider>
      </NavContext.Provider>
    </DocContext.Provider>
  );
}

/** Undo, redo and save, with what is unsaved. */
function EditControls() {
  const { status, undo, redo, openSave } = useEdit();
  return (
    <Group gap={4} ml="auto" wrap="nowrap">
      <Tooltip label={status.undo ? `Undo ${status.undo} (Ctrl/⌘+Z)` : "Nothing to undo"}>
        <ActionIcon variant="subtle" disabled={!status.undo} onClick={undo} aria-label="Undo">
          <UndoIcon />
        </ActionIcon>
      </Tooltip>
      <Tooltip label={status.redo ? `Redo ${status.redo} (Ctrl/⌘+Shift+Z)` : "Nothing to redo"}>
        <ActionIcon variant="subtle" disabled={!status.redo} onClick={redo} aria-label="Redo">
          <RedoIcon />
        </ActionIcon>
      </Tooltip>
      {status.dirty && (
        <Tooltip label="Objects added, changed or deleted and not saved">
          <Badge variant="light" color="yellow" style={{ flexShrink: 0 }}>
            {status.edited_objects.toLocaleString()} edited
          </Badge>
        </Tooltip>
      )}
      <Tooltip label="Save (Ctrl/⌘+S)">
        <Button size="compact-xs" variant={status.dirty ? "filled" : "default"} onClick={openSave}>
          Save…
        </Button>
      </Tooltip>
    </Group>
  );
}
