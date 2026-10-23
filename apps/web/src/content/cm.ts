/**
 * CodeMirror support for content stream text: highlighting by operator
 * group, folding of q … Q / BT … ET / BMC … EMC / BX … EX, completion of
 * operators (ranked by the operands written before them), snippets and
 * resource names, problems as you type, and operator help on hover.
 */

import {
  autocompletion,
  closeBrackets,
  completionKeymap,
  snippetCompletion,
  type Completion,
  type CompletionContext,
  type CompletionResult,
  type CompletionSection,
} from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import {
  HighlightStyle,
  StreamLanguage,
  bracketMatching,
  codeFolding,
  foldGutter,
  foldKeymap,
  foldService,
  syntaxHighlighting,
  syntaxTree,
  type StringStream,
} from "@codemirror/language";
import { linter, lintGutter, type Diagnostic } from "@codemirror/lint";
import { highlightSelectionMatches, searchKeymap } from "@codemirror/search";
import { EditorState, type Extension } from "@codemirror/state";
import {
  EditorView,
  drawSelection,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  hoverTooltip,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { Tag, tags as t } from "@lezer/highlight";
import { operandsOf, operatorDocs } from "../components/OperatorDoc";
import type { ResourceItem } from "../protocol/generated";
import {
  CLOSERS,
  NAME_CATEGORY,
  OPERATORS,
  categoryColor,
  categoryLabel,
  nonBytes,
  opAt,
  scanCached,
  type Operand,
  type Scanned,
} from "./syntax";

const scanned = (state: EditorState): Scanned => scanCached(state.doc, () => state.doc.toString());

// ------------------------------------------------------------ highlighting

const opTags: Record<string, Tag> = Object.fromEntries(Object.keys(categoryColor).map((c) => [c, Tag.define()]));
const pdfName = Tag.define(t.atom);
const imageData = Tag.define(t.string);

interface TokState {
  /** Depth inside a literal string (0: outside). */
  str: number;
  hex: boolean;
  /** Inside inline image data (after ID). */
  image: boolean;
}

const WORD = /^[^\s()<>[\]{}/%]+/;
const NUMBER = /^[+-]?(\d+\.?\d*|\.\d+)$/;
const EI = /(^|\s)EI(?=[\s/[<(%]|$)/;

function literal(stream: StringStream, st: TokState) {
  while (!stream.eol()) {
    const c = stream.next();
    if (c === "\\") stream.next();
    else if (c === "(") st.str++;
    else if (c === ")" && --st.str === 0) return;
  }
}

const language = StreamLanguage.define<TokState>({
  name: "pdf-content",
  startState: () => ({ str: 0, hex: false, image: false }),
  copyState: (s) => ({ ...s }),
  token(stream, st) {
    if (st.image) {
      const rest = stream.string.slice(stream.pos);
      const m = EI.exec(rest);
      if (m) {
        const at = m.index + m[1].length;
        st.image = false;
        if (at > 0) {
          stream.pos += at;
          return "imageData";
        }
      } else {
        stream.skipToEnd();
        return "imageData";
      }
    }
    if (st.str > 0) {
      literal(stream, st);
      return "string";
    }
    if (st.hex) {
      if (stream.skipTo(">")) {
        stream.next();
        st.hex = false;
      } else stream.skipToEnd();
      return "string";
    }
    if (stream.eatSpace()) return null;
    const ch = stream.peek()!;
    if (ch === "%") {
      stream.skipToEnd();
      return "comment";
    }
    if (ch === "(") {
      stream.next();
      st.str = 1;
      literal(stream, st);
      return "string";
    }
    if (stream.match("<<") || stream.match(">>")) return "bracket";
    if (ch === "<") {
      stream.next();
      st.hex = true;
      if (stream.skipTo(">")) {
        stream.next();
        st.hex = false;
      } else stream.skipToEnd();
      return "string";
    }
    if ("[]{}".includes(ch)) {
      stream.next();
      return "bracket";
    }
    if (ch === "/") {
      stream.next();
      stream.match(WORD);
      return "pdfName";
    }
    if (ch === ")" || ch === ">") {
      stream.next();
      return "invalid";
    }
    const m = stream.match(WORD) as RegExpMatchArray | null;
    if (!m) {
      stream.next();
      return "invalid";
    }
    const w = m[0];
    if (NUMBER.test(w)) return "number";
    if (w === "true" || w === "false" || w === "null") return "atom";
    if (w === "ID") {
      // One white-space character, then the data.
      st.image = true;
      if (stream.eol()) return "op-inlineImage";
      stream.next();
      return "op-inlineImage";
    }
    return `op-${OPERATORS.get(w)?.category ?? "unknown"}`;
  },
  tokenTable: {
    ...Object.fromEntries(Object.entries(opTags).map(([c, tag]) => [`op-${c}`, tag])),
    pdfName,
    imageData,
  },
  languageData: { commentTokens: { line: "%" } },
});

const highlight = HighlightStyle.define([
  ...Object.entries(opTags).map(([c, tag]) => ({
    tag,
    color: `var(--mantine-color-${categoryColor[c]}-text)`,
    fontWeight: "600",
  })),
  { tag: pdfName, color: "var(--mantine-color-blue-text)" },
  { tag: t.number, color: "var(--mantine-color-text)" },
  { tag: t.string, color: "var(--mantine-color-orange-text)" },
  { tag: imageData, color: "var(--mantine-color-dimmed)", fontStyle: "italic" },
  { tag: t.comment, color: "var(--mantine-color-dimmed)", fontStyle: "italic" },
  { tag: t.atom, color: "var(--mantine-color-violet-text)" },
  { tag: t.bracket, color: "var(--mantine-color-dimmed)" },
  { tag: t.invalid, color: "var(--mantine-color-red-text)", textDecoration: "underline wavy" },
]);

// ----------------------------------------------------------------- folding

function blockAt(s: Scanned, k: number): [number, number] | undefined {
  let lo = 0;
  let hi = s.blocks.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const b = s.blocks[mid];
    if (b[0] === k) return b;
    if (b[0] < k) lo = mid + 1;
    else hi = mid - 1;
  }
  return undefined;
}

const folding = foldService.of((state, lineStart, lineEnd) => {
  const s = scanned(state);
  let k = Math.max(0, opAt(s.ops, lineStart));
  for (; k < s.ops.length && s.ops[k].opFrom <= lineEnd; k++) {
    const op = s.ops[k];
    if (op.opFrom < lineStart || !(op.operator in CLOSERS)) continue;
    const b = blockAt(s, k);
    if (!b) continue;
    const close = b[1] >= 0 ? s.ops[b[1]] : null;
    const to = close ? close.opFrom : state.doc.length;
    if (to > lineEnd) return { from: op.to, to };
  }
  return null;
});

const foldPlaceholder = codeFolding({
  preparePlaceholder(state, range) {
    const s = scanned(state);
    const a = opAt(s.ops, range.from);
    const b = opAt(s.ops, range.to - 1);
    return Math.max(0, b - a);
  },
  placeholderDOM(_view, onclick, n: number) {
    const el = document.createElement("span");
    el.className = "cm-foldPlaceholder";
    el.textContent = ` … ${n.toLocaleString()} operation${n === 1 ? "" : "s"} … `;
    el.title = "Unfold";
    el.onclick = onclick;
    return el;
  },
});

// ---------------------------------------------------------------- problems

const problems = linter(
  (view) => {
    const state = view.state;
    const s = scanned(state);
    const out: Diagnostic[] = s.problems.map((p) => ({
      from: p.from,
      to: Math.max(p.to, p.from),
      severity: p.severity,
      message: p.message,
    }));
    for (const at of nonBytes(state.doc.toString()).slice(0, 100))
      out.push({
        from: at,
        to: at + 1,
        severity: "error",
        message: "not a byte: content streams hold bytes; write the text as a <hex> string in the font's encoding",
      });
    return out;
  },
  { delay: 300 },
);

// -------------------------------------------------------------- completion

const signature = (name: string) => {
  const d = operatorDocs[name];
  const info = OPERATORS.get(name);
  if (d) return d.variadic ? "c1 … cn" + (d.variadic === "pattern" ? " [name]" : "") : d.operands.map((o) => o.name).join(" ");
  return info?.args?.join(" ") ?? "";
};

function operatorInfo(name: string): HTMLElement {
  const el = document.createElement("div");
  el.style.maxWidth = "360px";
  el.style.fontSize = "12px";
  const d = operatorDocs[name];
  const info = OPERATORS.get(name);
  const head = document.createElement("div");
  head.style.fontWeight = "600";
  head.textContent = `${signature(name)} ${name}`.trim();
  el.append(head);
  const p = document.createElement("div");
  p.textContent = d?.text ?? info?.description ?? "Not an operator of the specification.";
  el.append(p);
  if (d) {
    for (const o of d.operands) {
      const line = document.createElement("div");
      line.style.opacity = "0.75";
      line.textContent = `${o.name}: ${o.meaning}${o.values ? ` (${Object.entries(o.values).map(([v, n]) => `${v} ${n}`).join(", ")})` : ""}`;
      el.append(line);
    }
    const ref = document.createElement("div");
    ref.style.opacity = "0.6";
    ref.textContent = `ISO 32000-2 §${d.section}`;
    el.append(ref);
  }
  return el;
}

const sections: Record<string, CompletionSection> = {};
const section = (name: string, rank: number): CompletionSection => (sections[name] ??= { name, rank });

/** How well an operator fits the operands written before it. */
function fit(name: string, operands: Operand[]): number {
  const args = OPERATORS.get(name)?.args;
  if (args === undefined) return -1;
  if (args === null) return operands.length > 0 && operands.every((o) => o.kind === "num" || o.kind === "name") ? 2 : -1;
  if (args.length !== operands.length) return args.length === 0 && operands.length > 0 ? -2 : -1;
  const ok = args.every((a, k) => {
    const g = operands[k].kind;
    return a === "num" || a === "int" ? g === "num" : a === "name" ? g === "name" : a === "str" ? g === "str" : a === "array" ? g === "array" : g === "dict" || g === "name";
  });
  return ok ? 3 : 0;
}

const operatorOptions: Completion[] = [...OPERATORS.values()]
  .filter((o) => o.name !== "ID" && o.name !== "EI")
  .map((o) => ({
    label: o.name,
    type: "keyword",
    detail: signature(o.name),
    info: () => operatorInfo(o.name),
    section: section(categoryLabel[o.category] ?? o.category, 10),
  }));

const snippets: Completion[] = [
  snippetCompletion("q\n${}\nQ", { label: "q … Q", detail: "save and restore the state", type: "text", section: section("Blocks", 0) }),
  snippetCompletion("BT\n/${F1} ${12} Tf\n${72} ${720} Td\n(${text}) Tj\nET", {
    label: "BT … ET",
    detail: "a text object",
    type: "text",
    section: section("Blocks", 0),
  }),
  snippetCompletion("/${Span} <</MCID ${0}>> BDC\n${}\nEMC", { label: "BDC … EMC", detail: "marked content", type: "text", section: section("Blocks", 0) }),
  snippetCompletion("/${Artifact} BMC\n${}\nEMC", { label: "BMC … EMC", detail: "marked content", type: "text", section: section("Blocks", 0) }),
  snippetCompletion("${x} ${y} ${width} ${height} re\nf", { label: "re f", detail: "fill a rectangle", type: "text", section: section("Blocks", 0) }),
  snippetCompletion("${x} ${y} ${width} ${height} re\nS", { label: "re S", detail: "stroke a rectangle", type: "text", section: section("Blocks", 0) }),
  snippetCompletion("1 0 0 1 ${tx} ${ty} cm", { label: "cm translate", detail: "move the origin", type: "text", section: section("Blocks", 0) }),
  snippetCompletion("${sx} 0 0 ${sy} 0 0 cm", { label: "cm scale", detail: "scale", type: "text", section: section("Blocks", 0) }),
  snippetCompletion("q\n${width} 0 0 ${height} ${x} ${y} cm\n/${Im0} Do\nQ", { label: "Do placed", detail: "paint an XObject in a box", type: "text", section: section("Blocks", 0) }),
];

const standardNames: [string, string, string][] = [
  ["DeviceGray", "color space", "cs / CS"],
  ["DeviceRGB", "color space", "cs / CS"],
  ["DeviceCMYK", "color space", "cs / CS"],
  ["Pattern", "color space", "cs / CS"],
  ["Artifact", "marked content tag", "BMC / BDC"],
  ["Span", "marked content tag", "BDC"],
  ["P", "marked content tag", "BDC"],
  ["Figure", "marked content tag", "BDC"],
  ["OC", "optional content tag", "BDC with a /Properties name"],
  ["Tx", "marked content tag", "form field text"],
  ["Perceptual", "rendering intent", "ri"],
  ["RelativeColorimetric", "rendering intent", "ri"],
  ["AbsoluteColorimetric", "rendering intent", "ri"],
  ["Saturation", "rendering intent", "ri"],
];

/** What follows a resource name when it ends the line. */
const nameTail: Record<string, string> = {
  Font: " ${12} Tf",
  XObject: " Do",
  ExtGState: " gs",
  Shading: " sh",
  ColorSpace: " cs",
  Pattern: " scn",
};

/** The operands written right before `pos`. */
function operandsBefore(s: Scanned, pos: number): Operand[] {
  const k = opAt(s.ops, pos);
  if (k >= 0 && s.ops[k].opFrom >= pos) return s.ops[k].operands.filter((o) => o.to <= pos);
  if (k >= 0 && s.ops[k].opFrom < pos && s.ops[k].to >= pos) return s.ops[k].operands;
  const after = s.ops[k + 1];
  const start = k >= 0 ? s.ops[k].to : 0;
  const list = after && after.from < pos ? after.operands : s.pending;
  return list.filter((o) => o.from >= start && o.to <= pos);
}

function completer(resources: () => ResourceItem[]) {
  return (ctx: CompletionContext): CompletionResult | null => {
    const node = syntaxTree(ctx.state).resolveInner(ctx.pos, -1);
    if (/string|comment|imageData/.test(node.type.name) && node.from < ctx.pos) return null;
    const name = ctx.matchBefore(/\/[^\s()<>[\]{}/%]*/);
    if (name) {
      const line = ctx.state.doc.lineAt(ctx.pos);
      const lineEnds = line.text.slice(ctx.pos - line.from).trim() === "";
      const options: Completion[] = resources().map((r) => {
        const tail = lineEnds ? nameTail[r.category] : undefined;
        const base = {
          label: `/${r.name}`,
          detail: r.category,
          info: r.detail || undefined,
          type: "variable",
          section: section(r.category, 1),
        };
        return tail ? snippetCompletion(`/${r.name.replace(/[$\\{}]/g, "\\$&")}${tail}`, base) : base;
      });
      for (const [n, kind, used] of standardNames)
        options.push({ label: `/${n}`, detail: kind, info: `Used with ${used}.`, type: "constant", section: section("Standard names", 5) });
      return { from: name.from, options, validFor: /^\/[^\s()<>[\]{}/%]*$/ };
    }
    const word = ctx.matchBefore(/[A-Za-z0-9*'"]+/);
    if (!word && !ctx.explicit) return null;
    if (word && /^[+-]?[\d.]+$/.test(word.text)) return null;
    const from = word?.from ?? ctx.pos;
    // Rank by the operands written before the word.
    const operands = operandsBefore(scanned(ctx.state), from);
    const options: Completion[] = operatorOptions.map((o) => ({ ...o, boost: fit(o.label, operands) * 10 }));
    if (operands.length === 0) options.push(...snippets);
    return { from, options, validFor: /^[A-Za-z0-9*'"]*$/ };
  };
}

// ------------------------------------------------------------------- hover

function hover(resources: () => ResourceItem[]) {
  return hoverTooltip((view, pos) => {
    const s = scanned(view.state);
    const k = opAt(s.ops, pos);
    const op = s.ops[k];
    if (!op || pos > op.to) return null;
    if (pos >= op.opFrom) {
      return {
        pos: op.opFrom,
        end: op.to,
        above: true,
        create: () => ({ dom: operatorInfo(op.operator) }),
      };
    }
    const operand = op.operands.find((o) => o.from <= pos && pos < o.to);
    if (!operand) return null;
    const index = op.operands.indexOf(operand);
    const d = operatorDocs[op.operator];
    const specs = d ? operandsOf(d, op.operands.length) : [];
    const spec = specs[index - (op.operands.length - specs.length)];
    const text = view.state.doc.sliceString(operand.from, operand.to);
    const res =
      operand.kind === "name"
        ? resources().find((r) => `/${r.name}` === text && (!NAME_CATEGORY[op.operator] || NAME_CATEGORY[op.operator] === r.category))
        : undefined;
    if (!spec && !res) return null;
    return {
      pos: operand.from,
      end: operand.to,
      above: true,
      create: () => {
        const dom = document.createElement("div");
        dom.style.maxWidth = "360px";
        dom.style.fontSize = "12px";
        if (spec) {
          const b = document.createElement("div");
          b.style.fontWeight = "600";
          b.textContent = `${spec.name} of ${op.operator}`;
          const m = document.createElement("div");
          m.textContent = spec.meaning + (spec.values?.[text] ? ` — ${text}: ${spec.values[text]}` : "");
          dom.append(b, m);
        }
        if (res) {
          const r = document.createElement("div");
          r.style.opacity = "0.8";
          r.textContent = `${res.category} /${res.name}${res.id ? ` (${res.id.num} ${res.id.generation} R)` : ""}${res.detail ? `: ${res.detail}` : ""}`;
          dom.append(r);
        } else if (operand.kind === "name" && NAME_CATEGORY[op.operator] && resources().length > 0) {
          const r = document.createElement("div");
          r.style.color = "var(--mantine-color-red-text)";
          r.textContent = `No ${NAME_CATEGORY[op.operator]} resource ${text}`;
          dom.append(r);
        }
        return { dom };
      },
    };
  });
}

// ------------------------------------------------------------------- setup

const theme = EditorView.theme({
  "&": { fontSize: "12px", height: "100%", backgroundColor: "var(--mantine-color-body)", color: "var(--mantine-color-text)" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--mantine-font-family-monospace)", lineHeight: "1.5" },
  ".cm-gutters": { backgroundColor: "var(--mantine-color-body)", color: "var(--mantine-color-dimmed)", border: "none" },
  ".cm-activeLine, .cm-activeLineGutter": { backgroundColor: "var(--mantine-color-default-hover)" },
  ".cm-foldPlaceholder": {
    backgroundColor: "var(--mantine-color-default-hover)",
    border: "1px solid var(--mantine-color-default-border)",
    color: "var(--mantine-color-dimmed)",
    borderRadius: "4px",
    padding: "0 4px",
    cursor: "pointer",
  },
  ".cm-tooltip": {
    backgroundColor: "var(--mantine-color-body)",
    border: "1px solid var(--mantine-color-default-border)",
    color: "var(--mantine-color-text)",
  },
  ".cm-tooltip-hover": { padding: "4px 8px" },
  ".cm-tooltip.cm-tooltip-autocomplete > ul > li[aria-selected]": {
    backgroundColor: "var(--mantine-primary-color-light)",
    color: "var(--mantine-color-text)",
  },
  ".cm-completionInfo": { padding: "4px 8px" },
  ".cm-completionDetail": { opacity: 0.6, marginLeft: "8px" },
  ".cm-completionSection": { opacity: 0.7, fontSize: "11px", padding: "2px 6px" },
  ".cm-specialChar": { color: "var(--mantine-color-red-text)" },
});

export interface ContentEditorOptions {
  /** Resource names to complete and explain. */
  resources: () => ResourceItem[];
  /** Line numbers, fold and problem gutters (the multi-line editor). */
  gutters: boolean;
  dark: boolean;
}

/** Extensions for editing content stream text. */
export function contentExtensions(o: ContentEditorOptions): Extension[] {
  const ext: Extension[] = [
    // Only LF splits lines: CR stays a character, so bytes round-trip.
    EditorState.lineSeparator.of("\n"),
    highlightSpecialChars({ addSpecialChars: /[\r\u0080- ]/ }),
    history(),
    drawSelection(),
    language,
    syntaxHighlighting(highlight),
    bracketMatching(),
    closeBrackets(),
    highlightSelectionMatches(),
    folding,
    foldPlaceholder,
    problems,
    autocompletion({ override: [completer(o.resources)], activateOnTyping: true, icons: false, maxRenderedOptions: 120 }),
    hover(o.resources),
    keymap.of([...completionKeymap, ...foldKeymap, ...searchKeymap, ...historyKeymap, indentWithTab, ...defaultKeymap]),
    theme,
    EditorView.darkTheme.of(o.dark),
  ];
  if (o.gutters) ext.push(lineNumbers(), foldGutter(), lintGutter(), highlightActiveLine(), highlightActiveLineGutter());
  return ext;
}

/** Index (in the text) of the operation at a position, or -1. */
export function opIndexAt(state: EditorState, pos: number): number {
  const s = scanned(state);
  const k = opAt(s.ops, pos);
  if (k < 0) return s.ops.length > 0 ? 0 : -1;
  // Between operations, the next one.
  if (pos > s.ops[k].to && k + 1 < s.ops.length) {
    const gap = state.doc.sliceString(s.ops[k].to, pos);
    if (gap.includes("\n")) return k + 1;
  }
  return k;
}

/** The text range of operation `k`. */
export function opRange(state: EditorState, k: number): { from: number; to: number } | null {
  const op = scanned(state).ops[k];
  return op ? { from: op.from, to: op.to } : null;
}
