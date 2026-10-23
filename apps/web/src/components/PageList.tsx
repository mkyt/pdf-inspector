import {
  ActionIcon,
  Alert,
  Badge,
  Button,
  Group,
  Menu,
  Modal,
  NumberInput,
  SegmentedControl,
  Select,
  Stack,
  Text,
  TextInput,
} from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api, useBackend } from "../api";
import { useDoc } from "../doc";
import { useEdit } from "../edit";
import { pageText, sizeText } from "../format";
import { useNav } from "../nav";
import type { EditCommand, PageSummary } from "../protocol/generated";
import { PagedList } from "./PagedList";
import { Row } from "./Row";
import classes from "./Row.module.css";

/** Every page, fetched by range (the backend finds each one through the page tree's counts). */
export function PageList() {
  const backend = useBackend();
  const nav = useNav();
  const { summary } = useDoc();
  const [goto, setGoto] = useState<string | number>("");
  const [adding, setAdding] = useState(false);
  const current = nav.target.kind === "page" ? nav.target.index : null;
  return (
    <>
      <Group p={6} gap={6}>
        <NumberInput
          size="xs"
          placeholder="Go to page"
          min={1}
          max={summary.pages}
          hideControls
          value={goto}
          onChange={setGoto}
          onKeyDown={(e) => {
            if (e.key === "Enter" && typeof goto === "number" && goto >= 1 && goto <= summary.pages) {
              nav.open({ kind: "page", index: goto - 1 });
            }
          }}
          style={{ flex: 1 }}
        />
        <Text size="xs" c="dimmed">
          {summary.pages.toLocaleString()} pages
        </Text>
        <Button size="compact-xs" variant="light" onClick={() => setAdding(true)}>
          Add…
        </Button>
      </Group>
      <PagedList
        total={summary.pages}
        empty="No pages."
        queryKey={["doc", nav.doc, "pages"]}
        fetchPage={(r, signal) => api.pages(backend, nav.doc, r, signal)}
        renderRow={(p) => (
          <div style={{ display: "flex", alignItems: "center", height: "100%" }}>
            <div style={{ flex: 1, minWidth: 0, height: "100%" }}>
              <Row active={p.index === current} onClick={() => nav.open({ kind: "page", index: p.index })}>
                <Text size="xs" w={72} ta="right" fw={500} style={{ overflow: "hidden", textOverflow: "ellipsis" }}>
                  {pageText(p.index, p.label)}
                </Text>
                {p.error ? (
                  <Text size="xs" c="red" style={{ overflow: "hidden", textOverflow: "ellipsis" }} title={p.error}>
                    {p.error}
                  </Text>
                ) : (
                  <>
                    <Text size="xs" c="dimmed" className={classes.mono} w={64}>
                      {p.id && `${p.id.num} ${p.id.generation}`}
                    </Text>
                    <Text size="xs" style={{ flex: 1 }}>
                      {sizeText(p.size)}
                    </Text>
                    {p.rotate !== 0 && (
                      <Badge size="xs" variant="light" color="gray">
                        {p.rotate}°
                      </Badge>
                    )}
                    {p.annots > 0 && (
                      <Badge size="xs" variant="light" color="grape" title="annotations">
                        {p.annots}
                      </Badge>
                    )}
                  </>
                )}
              </Row>
            </div>
            <PageMenu page={p} count={summary.pages} />
          </div>
        )}
      />
      <AddPages opened={adding} onClose={() => setAdding(false)} />
    </>
  );
}

/** Page operations on one page. */
function PageMenu({ page: p, count }: { page: PageSummary; count: number }) {
  const nav = useNav();
  const { run } = useEdit();
  const i = p.index;
  const n = pageText(i, p.label);
  const pages = [i];
  const move = (to: number) => run(`Move page ${n}`, [{ op: "movePages", pages, to }]);
  const blank = (at: number) => {
    const w = p.size ? p.size.x1 - p.size.x0 : 612;
    const h = p.size ? p.size.y1 - p.size.y0 : 792;
    run("Insert a blank page", [{ op: "insertBlankPage", at, width: w, height: h }]);
  };
  return (
    <Menu position="bottom-end" withinPortal shadow="md">
      <Menu.Target>
        <ActionIcon size="xs" variant="subtle" color="gray" mr={4} aria-label={`Page ${n}`}>
          ⋯
        </ActionIcon>
      </Menu.Target>
      <Menu.Dropdown>
        <Menu.Label>Page {n}</Menu.Label>
        <Menu.Item fz="xs" onClick={() => run(`Rotate page ${n}`, [{ op: "rotatePages", pages, by: 90 }])}>
          Rotate clockwise
        </Menu.Item>
        <Menu.Item fz="xs" onClick={() => run(`Rotate page ${n}`, [{ op: "rotatePages", pages, by: -90 }])}>
          Rotate counterclockwise
        </Menu.Item>
        <Menu.Divider />
        <Menu.Item fz="xs" disabled={i === 0} onClick={() => move(0)}>
          Move to the start
        </Menu.Item>
        <Menu.Item fz="xs" disabled={i === 0} onClick={() => move(i - 1)}>
          Move up
        </Menu.Item>
        <Menu.Item fz="xs" disabled={i === count - 1} onClick={() => move(i + 1)}>
          Move down
        </Menu.Item>
        <Menu.Item fz="xs" disabled={i === count - 1} onClick={() => move(count - 1)}>
          Move to the end
        </Menu.Item>
        <Menu.Divider />
        <Menu.Item
          fz="xs"
          onClick={() => run(`Duplicate page ${n}`, [{ op: "insertPages", at: i + 1, from: nav.doc, pages }])}
        >
          Duplicate
        </Menu.Item>
        <Menu.Item fz="xs" onClick={() => blank(i)}>
          Insert a blank page before
        </Menu.Item>
        <Menu.Item fz="xs" onClick={() => blank(i + 1)}>
          Insert a blank page after
        </Menu.Item>
        <Menu.Divider />
        <Menu.Item
          fz="xs"
          color="red"
          onClick={() => {
            run(`Delete page ${n}`, [{ op: "deletePages", pages }]);
            if (nav.target.kind === "page" && nav.target.index === i) nav.open({ kind: "document" });
          }}
        >
          Delete
        </Menu.Item>
      </Menu.Dropdown>
    </Menu>
  );
}

const SIZES: Record<string, [number, number]> = {
  A4: [595.28, 841.89],
  A3: [841.89, 1190.55],
  A5: [419.53, 595.28],
  Letter: [612, 792],
  Legal: [612, 1008],
};

/** `1-3, 7` (1-based) as 0-based indices; `null` if malformed or out of 1..n. */
export function parsePages(s: string, n: number): number[] | null {
  const out: number[] = [];
  for (const part of s.split(",").map((p) => p.trim())) {
    if (!part) continue;
    const m = /^(\d+)\s*(?:-\s*(\d+))?$/.exec(part);
    if (!m) return null;
    const a = Number(m[1]);
    const b = m[2] ? Number(m[2]) : a;
    if (a < 1 || b < a || b > n) return null;
    for (let i = a; i <= b; i++) out.push(i - 1);
  }
  return out.length ? out : null;
}

/** Blank pages, or pages copied from an open document (this one too). */
function AddPages({ opened, onClose }: { opened: boolean; onClose: () => void }) {
  const backend = useBackend();
  const nav = useNav();
  const { summary } = useDoc();
  const { apply } = useEdit();
  const [source, setSource] = useState<"blank" | "copy">("blank");
  const [size, setSize] = useState("A4");
  const [width, setWidth] = useState<number | string>(SIZES.A4[0]);
  const [height, setHeight] = useState<number | string>(SIZES.A4[1]);
  const [from, setFrom] = useState<string | null>(null);
  const [range, setRange] = useState("");
  const [at, setAt] = useState<number | string>(summary.pages + 1);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const docs = useQuery({ queryKey: ["docs"], queryFn: () => api.docs(backend), enabled: opened, staleTime: 0 });
  const fromDoc = from === null ? null : Number(from);
  const fromSummary = useQuery({
    queryKey: ["doc", fromDoc, "summary"],
    queryFn: () => api.summary(backend, fromDoc!),
    enabled: opened && fromDoc !== null,
  });
  const fromPages = fromSummary.data?.pages ?? 0;
  const position = typeof at === "number" ? Math.min(Math.max(at, 1), summary.pages + 1) - 1 : summary.pages;

  const submit = async () => {
    let cmd: EditCommand;
    if (source === "blank") {
      cmd = { op: "insertBlankPage", at: position, width: Number(width), height: Number(height) };
    } else {
      const pages = fromDoc !== null ? parsePages(range || `1-${fromPages}`, fromPages) : null;
      if (fromDoc === null || !pages) {
        setError(`Pages are numbers from 1 to ${fromPages}, like 1-3, 7.`);
        return;
      }
      cmd = { op: "insertPages", at: position, from: fromDoc, pages };
    }
    setBusy(true);
    setError(null);
    try {
      await apply(source === "blank" ? "Insert a blank page" : "Insert pages", [cmd]);
      onClose();
      nav.open({ kind: "page", index: position });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal opened={opened} onClose={onClose} title="Add pages">
      <Stack gap="xs">
        <SegmentedControl
          size="xs"
          value={source}
          onChange={(v) => setSource(v as "blank" | "copy")}
          data={[
            { value: "blank", label: "Blank page" },
            { value: "copy", label: "From a document" },
          ]}
        />
        {source === "blank" ? (
          <Group gap="xs" align="flex-end">
            <Select
              size="xs"
              label="Size"
              w={110}
              data={[...Object.keys(SIZES), "Custom"]}
              value={size}
              onChange={(v) => {
                if (!v) return;
                setSize(v);
                if (SIZES[v]) {
                  setWidth(SIZES[v][0]);
                  setHeight(SIZES[v][1]);
                }
              }}
              allowDeselect={false}
            />
            <NumberInput
              size="xs"
              label="Width (pt)"
              w={100}
              min={1}
              max={14400}
              value={width}
              onChange={(v) => {
                setWidth(v);
                setSize("Custom");
              }}
            />
            <NumberInput
              size="xs"
              label="Height (pt)"
              w={100}
              min={1}
              max={14400}
              value={height}
              onChange={(v) => {
                setHeight(v);
                setSize("Custom");
              }}
            />
          </Group>
        ) : (
          <Group gap="xs" align="flex-end" grow>
            <Select
              size="xs"
              label="Document"
              placeholder="An open document"
              data={(docs.data ?? []).map((d) => ({
                value: String(d.doc),
                label: d.doc === nav.doc ? `${d.name} (this one)` : d.name,
              }))}
              value={from}
              onChange={setFrom}
            />
            <TextInput
              size="xs"
              label="Pages"
              placeholder={fromPages ? `1-${fromPages}` : "all"}
              value={range}
              onChange={(e) => setRange(e.currentTarget.value)}
            />
          </Group>
        )}
        <NumberInput
          size="xs"
          label="Before page"
          description={`${summary.pages + 1} adds them at the end.`}
          min={1}
          max={summary.pages + 1}
          value={at}
          onChange={setAt}
          w={160}
        />
        {source === "copy" && (
          <Text size="xs" c="dimmed">
            Copies each page with everything it uses (from this document, the resources are shared). Links to pages
            not copied become null.
          </Text>
        )}
        {error && (
          <Alert color="red" p={6}>
            <Text size="xs">{error}</Text>
          </Alert>
        )}
        <Group justify="flex-end">
          <Button size="xs" variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button
            size="xs"
            loading={busy}
            disabled={source === "copy" && fromDoc === null}
            onClick={() => void submit()}
          >
            Add
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}
