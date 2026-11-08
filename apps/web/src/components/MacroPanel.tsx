import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { javascript } from "@codemirror/lang-javascript";
import { bracketMatching, codeFolding, foldGutter, foldKeymap, indentOnInput, syntaxHighlighting } from "@codemirror/language";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import { EditorState } from "@codemirror/state";
import { EditorView, drawSelection, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from "@codemirror/view";
import {
  Alert,
  Badge,
  Button,
  Checkbox,
  FileButton,
  Group,
  Loader,
  Progress,
  ScrollArea,
  Select,
  Stack,
  Table,
  Tabs,
  Text,
  Textarea,
  TextInput,
  Title,
  Tooltip,
  UnstyledButton,
  useComputedColorScheme,
} from "@mantine/core";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import template from "../../../../crates/pdfi-macro/js/template.js?raw";
import { api, useBackend } from "../api";
import type { Backend } from "../backend/backend";
import { theme } from "../content/cm";
import { useDoc } from "../doc";
import { base64, useEdit } from "../edit";
import { bytesText } from "../format";
import type { MacroMeta, MacroParam } from "../macro/engine";
import { STEM, errorText, macroService, type MacroItem, type MacroService, type RunHandle, type RunResult } from "../macro/service";
import { useNav } from "../nav";
import { jsHighlight } from "./ScriptView";

const services = new WeakMap<Backend, MacroService>();

function useMacros(): MacroService {
  const b = useBackend();
  let s = services.get(b);
  if (!s) {
    s = macroService(b);
    services.set(b, s);
  }
  return s;
}

/** How a macro is named in the URL: user macros by file name, bundled ones prefixed. */
const keyOf = (m: MacroItem) => (m.bundled ? `bundled:${m.stem}` : m.stem);

function download(name: string, data: Uint8Array | string, mime = "application/octet-stream") {
  const url = URL.createObjectURL(new Blob([data as BlobPart], { type: mime }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}

/** The macros: a list, and the chosen one's form, results and code. */
export function MacroPanel({ stem }: { stem: string | null }) {
  const macros = useMacros();
  const nav = useNav();
  const qc = useQueryClient();
  const list = useQuery({ queryKey: ["macros", macros.where], queryFn: () => macros.list(), staleTime: 10_000 });
  const [error, setError] = useState<string | null>(null);
  const items = list.data?.items ?? [];
  const selected = items.find((m) => keyOf(m) === stem) ?? null;
  const refresh = () => qc.invalidateQueries({ queryKey: ["macros"] });

  const create = async (name: string, code: string) => {
    setError(null);
    try {
      const m = await macros.save(name, code);
      await refresh();
      nav.open({ kind: "macros", stem: keyOf(m) });
    } catch (e) {
      setError(errorText(e));
    }
  };

  const newMacro = () => {
    const name = prompt("Name of the new macro (letters, digits, '.', '_' and '-'):", "my-macro");
    if (!name) return;
    if (!STEM.test(name)) return setError(`${JSON.stringify(name)} is not a macro name`);
    if (items.some((m) => !m.bundled && m.stem === name)) return setError(`there is a macro ${name} already`);
    void create(name, template.replaceAll("NAME", name));
  };

  const importFile = async (f: File | null) => {
    if (!f) return;
    const name = f.name.replace(/\.js$/i, "");
    if (!STEM.test(name)) return setError(`${f.name}: the file name is not a macro name`);
    if (items.some((m) => !m.bundled && m.stem === name) && !confirm(`Replace the macro ${name}?`)) return;
    void create(name, await f.text());
  };

  const user = items.filter((m) => !m.bundled);
  const bundled = items.filter((m) => m.bundled);
  return (
    <div style={{ display: "flex", height: "100%", minHeight: 0 }}>
      <div
        style={{
          width: 240,
          flexShrink: 0,
          borderRight: "1px solid var(--mantine-color-default-border)",
          display: "flex",
          flexDirection: "column",
        }}
      >
        <Group gap={4} p="xs" wrap="nowrap">
          <Button size="compact-xs" variant="light" onClick={newMacro}>
            New
          </Button>
          <FileButton onChange={(f) => void importFile(f)} accept=".js,text/javascript">
            {(props) => (
              <Button size="compact-xs" variant="default" {...props}>
                Import .js…
              </Button>
            )}
          </FileButton>
        </Group>
        {error && (
          <Alert color="red" mx="xs" mb="xs" p={6} withCloseButton onClose={() => setError(null)}>
            <Text size="xs">{error}</Text>
          </Alert>
        )}
        <ScrollArea style={{ flex: 1 }}>
          {list.isPending && <Loader size="sm" m="sm" />}
          {list.error && (
            <Text size="xs" c="red" p="xs">
              {String(list.error)}
            </Text>
          )}
          <ListSection title={macros.where === "server" ? "Your macros" : "Your macros (this browser)"} items={user} current={stem} />
          {list.data && user.length === 0 && (
            <Text size="xs" c="dimmed" px="xs">
              None yet. {list.data.dir ? `They are kept in ${list.data.dir}.` : "They are kept in this browser."}
            </Text>
          )}
          <ListSection title="Bundled" items={bundled} current={stem} />
        </ScrollArea>
      </div>
      <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
        {selected ? (
          <MacroDetail key={keyOf(selected)} item={selected} refresh={refresh} onCopied={(m) => nav.open({ kind: "macros", stem: keyOf(m) })} />
        ) : (
          <Stack p="md" gap="xs" maw={640}>
            <Title order={5}>Macros</Title>
            <Text size="sm">
              JavaScript that reads or edits the document through the macro API: lists of fonts, metadata, outlines from JSON,
              page labels, removing printer&apos;s marks. A run is one step you can undo; nothing is saved until you save the
              document.
            </Text>
            <Text size="sm" c="dimmed">
              {macros.where === "server"
                ? "They run in the server, from the same directory as `pdfi macro run`."
                : "They run in this tab, in a Worker of their own: stopping one leaves the document and its edits alone."}{" "}
              The API is documented in docs/macros.
            </Text>
          </Stack>
        )}
      </div>
    </div>
  );
}

function ListSection({ title, items, current }: { title: string; items: MacroItem[]; current: string | null }) {
  const nav = useNav();
  if (items.length === 0) return null;
  return (
    <Stack gap={0} pb="xs">
      <Text size="xs" fw={600} c="dimmed" px="xs" py={4}>
        {title}
      </Text>
      {items.map((m) => (
        <UnstyledButton
          key={keyOf(m)}
          px="xs"
          py={4}
          onClick={() => nav.open({ kind: "macros", stem: keyOf(m) })}
          style={{
            background: keyOf(m) === current ? "var(--mantine-color-default-hover)" : undefined,
            opacity: m.shadowed ? 0.6 : 1,
          }}
        >
          <Group gap={4} wrap="nowrap">
            <Text size="xs" fw={500} truncate>
              {m.meta?.title ?? m.stem}
            </Text>
            {m.error && (
              <Badge size="xs" color="red" variant="light">
                error
              </Badge>
            )}
            {m.meta?.edits && (
              <Badge size="xs" color="yellow" variant="light">
                edits
              </Badge>
            )}
          </Group>
          <Text size="xs" c="dimmed" truncate>
            {m.name}
            {m.shadowed ? " (hidden by one of the same name)" : ""}
          </Text>
        </UnstyledButton>
      ))}
    </Stack>
  );
}

function MacroDetail({ item, refresh, onCopied }: { item: MacroItem; refresh: () => Promise<void>; onCopied: (m: MacroItem) => void }) {
  const [tab, setTab] = useState<string | null>(item.error ? "code" : "run");
  const [dirty, setDirty] = useState(false);
  const m = item.meta;
  return (
    <>
      <Stack gap={2} px="md" pt="sm" pb={6}>
        <Group gap={6}>
          <Title order={5}>{m?.title ?? item.stem}</Title>
          <Badge size="xs" variant="light" color="gray" tt="none">
            {item.name}
          </Badge>
          {m?.edits && (
            <Tooltip label={`Changes the document; suggested saving: ${typeof m.save === "string" ? m.save : "full rewrite"}`}>
              <Badge size="xs" variant="light" color="yellow">
                edits
              </Badge>
            </Tooltip>
          )}
        </Group>
        {m?.description && <Text size="xs">{m.description}</Text>}
        <Text size="xs" c="dimmed">
          {item.file}
        </Text>
      </Stack>
      <Tabs value={tab} onChange={setTab} style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0 }}>
        <Tabs.List px="md">
          <Tabs.Tab value="run">Run</Tabs.Tab>
          <Tabs.Tab value="code">
            Code{" "}
            {dirty && (
              <Badge size="xs" color="yellow" variant="light">
                unsaved
              </Badge>
            )}
          </Tabs.Tab>
        </Tabs.List>
        {/* Both stay mounted: switching tabs keeps the form, the results and the editor. */}
        <div style={{ flex: 1, minHeight: 0, overflow: "auto", display: tab === "run" ? "block" : "none" }}>
          <RunTab item={item} unsaved={dirty} />
        </div>
        <div style={{ flex: 1, minHeight: 0, display: tab === "code" ? "flex" : "none", flexDirection: "column" }}>
          <CodeTab item={item} refresh={refresh} onDirty={setDirty} onCopied={onCopied} />
        </div>
      </Tabs>
    </>
  );
}

// ------------------------------------------------------------------ run

/** What the form holds for a parameter: text as typed, or a picked value. */
type Field = { text: string } | { value: unknown; label: string } | { bool: boolean };

function rawParams(meta: MacroMeta, fields: Record<string, Field | undefined>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, p] of Object.entries(meta.params)) {
    const f = fields[k];
    if (!f) continue;
    if ("bool" in f) out[k] = f.bool;
    else if ("value" in f) out[k] = f.value;
    else if (f.text.trim() !== "" || p.type === "string" || p.type === "text") {
      if (f.text === "" && p.type !== "text") continue;
      if (p.type === "json") {
        try {
          out[k] = JSON.parse(f.text);
        } catch (e) {
          throw new Error(`parameter ${k}: not JSON: ${errorText(e)}`);
        }
      } else out[k] = f.text;
    }
  }
  return out;
}

interface LogLine {
  level: "info" | "warn" | "error";
  text: string;
}

/** Log lines kept on the page. */
const MAX_LOG = 5000;

function RunTab({ item, unsaved }: { item: MacroItem; unsaved: boolean }) {
  const macros = useMacros();
  const nav = useNav();
  const { summary } = useDoc();
  const { status, openSave } = useEdit();
  const meta = item.meta;
  const [fields, setFields] = useState<Record<string, Field | undefined>>({});
  const [running, setRunning] = useState<RunHandle | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number; label: string | null } | null>(null);
  const [log, setLog] = useState<LogLine[]>([]);
  const [result, setResult] = useState<RunResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [took, setTook] = useState<number | null>(null);
  const handle = useRef<RunHandle | null>(null);
  // Leaving the panel stops the run (its results would have nowhere to go).
  useEffect(() => () => handle.current?.cancel(), []);

  if (!meta) {
    return (
      <Alert color="red" m="md" title="This macro cannot be read">
        <Text size="xs" style={{ whiteSpace: "pre-wrap" }}>
          {item.error}
        </Text>
      </Alert>
    );
  }

  const start = () => {
    let params: Record<string, unknown>;
    try {
      params = rawParams(meta, fields);
    } catch (e) {
      setError(errorText(e));
      return;
    }
    setError(null);
    setResult(null);
    setLog([]);
    setProgress(null);
    setTook(null);
    const t0 = performance.now();
    const lines: LogLine[] = [];
    let flush = 0;
    const h = macros.run(nav.doc, item, params, { name: summary.info.name, size: summary.info.size }, {
      log: (level, text) => {
        if (lines.length < MAX_LOG) lines.push({ level, text });
        // Many lines at once: show them in batches.
        if (!flush) flush = requestAnimationFrame(() => ((flush = 0), setLog(lines.slice())));
      },
      progress: (done, total, label) => setProgress({ done, total, label }),
    });
    handle.current = h;
    setRunning(h);
    h.result.then(
      (r) => setResult(r),
      (e) => setError(errorText(e)),
    ).finally(() => {
      setLog(lines.slice());
      setTook(performance.now() - t0);
      handle.current = null;
      setRunning(null);
    });
  };

  const params = Object.entries(meta.params);
  return (
    <Stack gap="sm" p="md" maw={960}>
      {params.length > 0 && (
        <Stack gap={6}>
          {params.map(([k, p]) => (
            <ParamField key={k} name={k} param={p} field={fields[k]} onChange={(f) => setFields((x) => ({ ...x, [k]: f }))} />
          ))}
        </Stack>
      )}
      <Group gap={6}>
        <Button size="xs" onClick={start} loading={running !== null} disabled={running !== null}>
          Run on {summary.info.name}
        </Button>
        {running && (
          <Button size="xs" variant="default" color="red" onClick={() => running.cancel()}>
            Stop
          </Button>
        )}
        {unsaved && (
          <Text size="xs" c="orange">
            The code has unsaved changes: Run uses the saved code.
          </Text>
        )}
        {took !== null && !running && (
          <Text size="xs" c="dimmed">
            {(took / 1000).toFixed(2)} s
          </Text>
        )}
      </Group>
      {running && progress && progress.total > 0 && (
        <Stack gap={2}>
          <Progress size="sm" value={(100 * progress.done) / progress.total} />
          <Text size="xs" c="dimmed">
            {progress.done.toLocaleString()} / {progress.total.toLocaleString()} {progress.label ?? ""}
          </Text>
        </Stack>
      )}
      {error && (
        <Alert color={error === "cancelled" ? "gray" : "red"} p={8} title={error === "cancelled" ? "Stopped" : "The macro failed"}>
          <Text size="xs" ff="monospace" style={{ whiteSpace: "pre-wrap" }}>
            {error === "cancelled" ? "Its edits were reverted." : `${error}\n\nIts edits were reverted.`}
          </Text>
        </Alert>
      )}
      {result && result.edits && status.dirty && (
        <Alert color="yellow" p={8}>
          <Group justify="space-between" wrap="nowrap">
            <Text size="xs">
              The document was changed{status.undo ? ` (undo: “${status.undo}”)` : ""}. It is not saved yet.
            </Text>
            <Button size="compact-xs" onClick={() => openSave(result.save)}>
              Save…
            </Button>
          </Group>
        </Alert>
      )}
      {result && <ResultView result={result} name={meta.name} />}
      {log.length > 0 && (
        <Stack gap={2}>
          <Text size="xs" fw={600}>
            Log
          </Text>
          <ScrollArea.Autosize mah={240} type="auto">
            <Text component="pre" size="xs" ff="monospace" m={0} style={{ whiteSpace: "pre-wrap" }}>
              {log.map((l, i) => (
                <span key={i} style={{ color: l.level === "info" ? undefined : `var(--mantine-color-${l.level === "warn" ? "orange" : "red"}-text)` }}>
                  {l.text}
                  {"\n"}
                </span>
              ))}
            </Text>
          </ScrollArea.Autosize>
          {(log.length >= MAX_LOG || result?.logTruncated) && (
            <Text size="xs" c="dimmed">
              Later lines were dropped.
            </Text>
          )}
        </Stack>
      )}
    </Stack>
  );
}

function ParamField({
  name,
  param: p,
  field,
  onChange,
}: {
  name: string;
  param: MacroParam;
  field: Field | undefined;
  onChange: (f: Field | undefined) => void;
}) {
  const backend = useBackend();
  const docs = useQuery({ queryKey: ["docs"], queryFn: () => api.docs(backend), enabled: p.type === "pdf" });
  const label = (
    <Group gap={4} wrap="nowrap" component="span">
      <Text size="xs" fw={600} component="span">
        {name}
      </Text>
      <Text size="xs" c="dimmed" component="span">
        {p.type}
        {p.optional ? ", optional" : ""}
      </Text>
    </Group>
  );
  const def = p.default === undefined ? undefined : typeof p.default === "string" ? p.default : JSON.stringify(p.default);
  const text = field && "text" in field ? field.text : "";
  const common = { label, description: p.description, size: "xs" as const };
  switch (p.type) {
    case "boolean":
      return (
        <Checkbox
          {...common}
          checked={field && "bool" in field ? field.bool : p.default === true}
          onChange={(e) => onChange({ bool: e.currentTarget.checked })}
        />
      );
    case "enum":
      return (
        <Select
          {...common}
          data={p.values ?? []}
          value={field && "value" in field ? String(field.value) : (def ?? null)}
          onChange={(v) => onChange(v === null ? undefined : { value: v, label: v })}
          clearable={p.optional}
          w={320}
        />
      );
    case "pdf":
      return (
        <Select
          {...common}
          placeholder="An open document"
          data={(docs.data ?? []).map((d) => ({ value: String(d.doc), label: d.name }))}
          value={field && "value" in field ? String((field.value as { $doc: number }).$doc) : null}
          onChange={(v) => onChange(v === null ? undefined : { value: { $doc: Number(v) }, label: v })}
          clearable
          w={320}
        />
      );
    case "bytes":
    case "text":
      return (
        <Stack gap={2}>
          {p.type === "text" ? (
            <Textarea {...common} autosize minRows={2} maxRows={10} value={text} onChange={(e) => onChange({ text: e.currentTarget.value })} />
          ) : (
            <Text size="xs" component="div">
              {label}
              {p.description && (
                <Text size="xs" c="dimmed">
                  {p.description}
                </Text>
              )}
            </Text>
          )}
          <Group gap={6}>
            <FileButton
              onChange={async (f) => {
                if (!f) return;
                if (p.type === "text") onChange({ text: await f.text() });
                else onChange({ value: { $bytes: base64(new Uint8Array(await f.arrayBuffer())) }, label: `${f.name} (${bytesText(f.size)})` });
              }}
            >
              {(props) => (
                <Button size="compact-xs" variant="default" {...props}>
                  Choose a file…
                </Button>
              )}
            </FileButton>
            {field && "label" in field && <Text size="xs">{field.label}</Text>}
          </Group>
        </Stack>
      );
    case "json":
      return (
        <Stack gap={2}>
          <Textarea
            {...common}
            autosize
            minRows={2}
            maxRows={14}
            ff="monospace"
            placeholder={def}
            value={text}
            onChange={(e) => onChange({ text: e.currentTarget.value })}
          />
          <FileButton onChange={async (f) => f && onChange({ text: await f.text() })} accept=".json,application/json">
            {(props) => (
              <Button size="compact-xs" variant="default" w="fit-content" {...props}>
                Load a JSON file…
              </Button>
            )}
          </FileButton>
        </Stack>
      );
    default:
      return (
        <TextInput
          {...common}
          placeholder={def ?? (p.type === "pages" ? "like 1-3,8 (all when empty)" : undefined)}
          value={text}
          onChange={(e) => onChange({ text: e.currentTarget.value })}
          w={p.type === "string" ? 480 : 240}
        />
      );
  }
}

/** Table cells: short text for any JSON value. */
function cell(v: unknown): string {
  if (v === null || v === undefined) return "";
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}

function csv(rows: Record<string, unknown>[], columns: string[]): string {
  const q = (s: string) => (/[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s);
  return [columns.map(q).join(","), ...rows.map((r) => columns.map((c) => q(cell(r[c]))).join(","))].join("\r\n") + "\r\n";
}

/** Rows shown in the table (the downloads have all of them). */
const SHOWN_ROWS = 1000;

function ResultView({ result, name }: { result: RunResult; name: string }) {
  const [copied, setCopied] = useState(false);
  const json = useMemo(() => JSON.stringify(result.value, null, 2), [result.value]);
  const rows = result.columns && Array.isArray(result.value) ? (result.value as Record<string, unknown>[]) : null;
  return (
    <Stack gap={6}>
      <Group gap={6}>
        <Text size="xs" fw={600}>
          Result
        </Text>
        {rows && (
          <Text size="xs" c="dimmed">
            {rows.length.toLocaleString()} rows
          </Text>
        )}
        <Button
          size="compact-xs"
          variant="default"
          ml="auto"
          onClick={() => void navigator.clipboard.writeText(json).then(() => setCopied(true))}
        >
          {copied ? "Copied" : "Copy JSON"}
        </Button>
        <Button size="compact-xs" variant="default" onClick={() => download(`${name}.json`, json + "\n", "application/json")}>
          JSON
        </Button>
        {rows && result.columns && (
          <Button size="compact-xs" variant="default" onClick={() => download(`${name}.csv`, csv(rows, result.columns!), "text/csv")}>
            CSV
          </Button>
        )}
      </Group>
      {rows && result.columns ? (
        <ScrollArea.Autosize mah={420} type="auto">
          <Table striped withTableBorder withColumnBorders fz="xs" verticalSpacing={2} horizontalSpacing={6}>
            <Table.Thead>
              <Table.Tr>
                {result.columns.map((c) => (
                  <Table.Th key={c}>{c}</Table.Th>
                ))}
              </Table.Tr>
            </Table.Thead>
            <Table.Tbody>
              {rows.slice(0, SHOWN_ROWS).map((r, i) => (
                <Table.Tr key={i}>
                  {result.columns!.map((c) => (
                    <Table.Td key={c} style={{ fontFamily: typeof r[c] === "number" ? "var(--mantine-font-family-monospace)" : undefined }}>
                      {cell(r[c])}
                    </Table.Td>
                  ))}
                </Table.Tr>
              ))}
            </Table.Tbody>
          </Table>
          {rows.length > SHOWN_ROWS && (
            <Text size="xs" c="dimmed">
              The first {SHOWN_ROWS.toLocaleString()} rows are shown; the downloads have all of them.
            </Text>
          )}
        </ScrollArea.Autosize>
      ) : (
        <ScrollArea.Autosize mah={420} type="auto">
          <Text component="pre" size="xs" ff="monospace" m={0}>
            {json}
          </Text>
        </ScrollArea.Autosize>
      )}
      {result.files.length > 0 && (
        <Stack gap={2}>
          <Text size="xs" fw={600}>
            Files
          </Text>
          {result.files.map((f, i) => (
            <Group key={i} gap={6}>
              <Button
                size="compact-xs"
                variant="default"
                onClick={() => void f.read().then((d) => download(f.name, d, f.mime ?? "application/octet-stream"))}
              >
                Download
              </Button>
              <Text size="xs">{f.name}</Text>
              <Text size="xs" c="dimmed">
                {bytesText(f.size)}
                {f.mime ? ` · ${f.mime}` : ""}
              </Text>
            </Group>
          ))}
        </Stack>
      )}
    </Stack>
  );
}

// ------------------------------------------------------------------ code

function CodeTab({
  item,
  refresh,
  onDirty,
  onCopied,
}: {
  item: MacroItem;
  refresh: () => Promise<void>;
  onDirty: (d: boolean) => void;
  onCopied: (m: MacroItem) => void;
}) {
  const macros = useMacros();
  const nav = useNav();
  const source = useQuery({ queryKey: ["macros", macros.where, "source", keyOf(item)], queryFn: () => macros.source(item), staleTime: Infinity });
  const [text, setText] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const qc = useQueryClient();
  const saved = source.data ?? null;
  const current = text ?? saved;
  useEffect(() => onDirty(text !== null && text !== saved), [text, saved, onDirty]);

  const save = async () => {
    if (current === null || item.bundled) return;
    setBusy(true);
    setError(null);
    try {
      const m = await macros.save(item.stem, current);
      qc.setQueryData(["macros", macros.where, "source", keyOf(item)], current);
      setText(null);
      await refresh();
      if (m.error) setError(`Saved, but it cannot be run: ${m.error}`);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const copy = async () => {
    if (current === null) return;
    const name = prompt("Name of your copy:", item.stem);
    if (!name) return;
    if (!STEM.test(name)) return setError(`${JSON.stringify(name)} is not a macro name`);
    try {
      const m = await macros.save(name, current);
      await refresh();
      onCopied(m);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const remove = async () => {
    if (!confirm(`Delete the macro ${item.stem}?`)) return;
    try {
      await macros.remove(item.stem);
      await refresh();
      nav.open({ kind: "macros", stem: null });
    } catch (e) {
      setError(errorText(e));
    }
  };

  if (source.error) return <Text size="xs" c="red" p="md">{String(source.error)}</Text>;
  if (current === null) return <Loader size="sm" m="md" />;
  return (
    <>
      <Group gap={6} px="md" py={6} wrap="nowrap">
        {item.bundled ? (
          <>
            <Text size="xs" c="dimmed">
              Bundled macros cannot be changed.
            </Text>
            <Button size="compact-xs" variant="light" onClick={() => void copy()}>
              Copy to my macros…
            </Button>
          </>
        ) : (
          <>
            <Button size="compact-xs" onClick={() => void save()} loading={busy} disabled={text === null || text === saved}>
              Save (Ctrl/⌘+S)
            </Button>
            <Button size="compact-xs" variant="default" disabled={text === null || text === saved} onClick={() => setText(null)}>
              Revert
            </Button>
            <Button size="compact-xs" variant="subtle" color="red" onClick={() => void remove()}>
              Delete
            </Button>
          </>
        )}
        <Button size="compact-xs" variant="default" ml="auto" onClick={() => download(`${item.stem}.js`, current, "text/javascript")}>
          Export .js
        </Button>
      </Group>
      {error && (
        <Alert color="red" mx="md" mb={6} p={6} withCloseButton onClose={() => setError(null)}>
          <Text size="xs" style={{ whiteSpace: "pre-wrap" }}>
            {error}
          </Text>
        </Alert>
      )}
      <div style={{ flex: 1, minHeight: 0, margin: "0 16px 16px" }}>
        <JsEditor key={saved ?? ""} initial={current} readOnly={item.bundled} onChange={setText} onSave={() => void save()} />
      </div>
    </>
  );
}

function JsEditor({
  initial,
  readOnly,
  onChange,
  onSave,
}: {
  initial: string;
  readOnly: boolean;
  onChange: (text: string) => void;
  onSave: () => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const dark = useComputedColorScheme("light") === "dark";
  const cbs = useRef({ onChange, onSave });
  cbs.current = { onChange, onSave };
  useEffect(() => {
    const v = new EditorView({
      state: EditorState.create({
        doc: initial,
        extensions: [
          EditorState.readOnly.of(readOnly),
          lineNumbers(),
          highlightActiveLineGutter(),
          highlightActiveLine(),
          drawSelection(),
          history(),
          indentOnInput(),
          codeFolding(),
          foldGutter(),
          bracketMatching(),
          highlightSelectionMatches(),
          search({ top: true }),
          keymap.of([
            { key: "Mod-s", preventDefault: true, run: () => (cbs.current.onSave(), true) },
            ...defaultKeymap,
            ...historyKeymap,
            ...searchKeymap,
            ...foldKeymap,
            indentWithTab,
          ]),
          javascript(),
          syntaxHighlighting(jsHighlight),
          theme,
          EditorView.darkTheme.of(dark),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) cbs.current.onChange(u.state.doc.toString());
          }),
        ],
      }),
      parent: host.current!,
    });
    return () => v.destroy();
    // The text is the editor's own once it is made.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [readOnly, dark]);
  return (
    <div
      ref={host}
      style={{ height: "100%", minHeight: 320, border: "1px solid var(--mantine-color-default-border)", borderRadius: 4, overflow: "hidden" }}
    />
  );
}
