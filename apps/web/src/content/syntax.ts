/**
 * Content stream syntax on the client: the operators (as in pdfi-content's
 * table), a scanner that splits text into operations, and the blocks
 * (q … Q, BT … ET, BMC / BDC … EMC, BX … EX) an editor folds.
 */

/** Color of each operator group, as a Mantine color name. */
export const categoryColor: Record<string, string> = {
  generalState: "grape",
  specialState: "red",
  pathConstruction: "blue",
  pathPainting: "indigo",
  clipping: "cyan",
  textObject: "orange",
  textState: "yellow",
  textPositioning: "lime",
  textShowing: "orange",
  type3: "pink",
  color: "teal",
  shading: "pink",
  inlineImage: "violet",
  xObject: "violet",
  markedContent: "gray",
  compatibility: "gray",
  unknown: "red",
};

export const categoryLabel: Record<string, string> = {
  generalState: "general graphics state",
  specialState: "q / Q / cm",
  pathConstruction: "path construction",
  pathPainting: "path painting",
  clipping: "clipping",
  textObject: "BT / ET",
  textState: "text state",
  textPositioning: "text positioning",
  textShowing: "text showing",
  type3: "Type 3",
  color: "color",
  shading: "shading",
  inlineImage: "inline image",
  xObject: "XObject",
  markedContent: "marked content",
  compatibility: "BX / EX",
  unknown: "unknown",
};

/** Kind of an operand: number, integer, name, string, array, dictionary or name. */
export type ArgKind = "num" | "int" | "name" | "str" | "array" | "props";

export interface OperatorInfo {
  name: string;
  category: string;
  /** `null`: as many as the color space has components (and maybe a pattern name). */
  args: ArgKind[] | null;
  description: string;
}

const table: [string, string, ArgKind[] | null, string][] = [
  ["w", "generalState", ["num"], "set line width"],
  ["J", "generalState", ["int"], "set line cap style"],
  ["j", "generalState", ["int"], "set line join style"],
  ["M", "generalState", ["num"], "set miter limit"],
  ["d", "generalState", ["array", "num"], "set line dash pattern"],
  ["ri", "generalState", ["name"], "set rendering intent"],
  ["i", "generalState", ["num"], "set flatness tolerance"],
  ["gs", "generalState", ["name"], "set parameters from a graphics state parameter dictionary"],
  ["q", "specialState", [], "save graphics state"],
  ["Q", "specialState", [], "restore graphics state"],
  ["cm", "specialState", ["num", "num", "num", "num", "num", "num"], "concatenate matrix to the current transformation matrix"],
  ["m", "pathConstruction", ["num", "num"], "begin new subpath"],
  ["l", "pathConstruction", ["num", "num"], "append straight line segment"],
  ["c", "pathConstruction", ["num", "num", "num", "num", "num", "num"], "append Bézier curve (two control points)"],
  ["v", "pathConstruction", ["num", "num", "num", "num"], "append Bézier curve (first control point = current point)"],
  ["y", "pathConstruction", ["num", "num", "num", "num"], "append Bézier curve (second control point = end point)"],
  ["h", "pathConstruction", [], "close subpath"],
  ["re", "pathConstruction", ["num", "num", "num", "num"], "append rectangle"],
  ["S", "pathPainting", [], "stroke path"],
  ["s", "pathPainting", [], "close and stroke path"],
  ["f", "pathPainting", [], "fill path (nonzero winding)"],
  ["F", "pathPainting", [], "fill path (nonzero winding; obsolete spelling)"],
  ["f*", "pathPainting", [], "fill path (even-odd)"],
  ["B", "pathPainting", [], "fill and stroke path (nonzero winding)"],
  ["B*", "pathPainting", [], "fill and stroke path (even-odd)"],
  ["b", "pathPainting", [], "close, fill and stroke path (nonzero winding)"],
  ["b*", "pathPainting", [], "close, fill and stroke path (even-odd)"],
  ["n", "pathPainting", [], "end path without filling or stroking"],
  ["W", "clipping", [], "set clipping path (nonzero winding)"],
  ["W*", "clipping", [], "set clipping path (even-odd)"],
  ["BT", "textObject", [], "begin text object"],
  ["ET", "textObject", [], "end text object"],
  ["Tc", "textState", ["num"], "set character spacing"],
  ["Tw", "textState", ["num"], "set word spacing"],
  ["Tz", "textState", ["num"], "set horizontal scaling (percent)"],
  ["TL", "textState", ["num"], "set text leading"],
  ["Tf", "textState", ["name", "num"], "set text font and size"],
  ["Tr", "textState", ["int"], "set text rendering mode"],
  ["Ts", "textState", ["num"], "set text rise"],
  ["Td", "textPositioning", ["num", "num"], "move text position"],
  ["TD", "textPositioning", ["num", "num"], "move text position and set leading"],
  ["Tm", "textPositioning", ["num", "num", "num", "num", "num", "num"], "set text matrix and text line matrix"],
  ["T*", "textPositioning", [], "move to start of next text line"],
  ["Tj", "textShowing", ["str"], "show text"],
  ["'", "textShowing", ["str"], "move to next line and show text"],
  ['"', "textShowing", ["num", "num", "str"], "set word and character spacing, move to next line and show text"],
  ["TJ", "textShowing", ["array"], "show text, allowing individual glyph positioning"],
  ["d0", "type3", ["num", "num"], "set glyph width (Type 3 font, colored glyph)"],
  ["d1", "type3", ["num", "num", "num", "num", "num", "num"], "set glyph width and bounding box (Type 3 font, uncolored glyph)"],
  ["CS", "color", ["name"], "set color space for stroking"],
  ["cs", "color", ["name"], "set color space for nonstroking"],
  ["SC", "color", null, "set color for stroking"],
  ["SCN", "color", null, "set color for stroking (also Pattern, Separation, DeviceN, ICCBased)"],
  ["sc", "color", null, "set color for nonstroking"],
  ["scn", "color", null, "set color for nonstroking (also Pattern, Separation, DeviceN, ICCBased)"],
  ["G", "color", ["num"], "set gray level for stroking"],
  ["g", "color", ["num"], "set gray level for nonstroking"],
  ["RG", "color", ["num", "num", "num"], "set RGB color for stroking"],
  ["rg", "color", ["num", "num", "num"], "set RGB color for nonstroking"],
  ["K", "color", ["num", "num", "num", "num"], "set CMYK color for stroking"],
  ["k", "color", ["num", "num", "num", "num"], "set CMYK color for nonstroking"],
  ["sh", "shading", ["name"], "paint area defined by shading pattern"],
  ["BI", "inlineImage", [], "inline image (BI … ID … EI)"],
  ["ID", "inlineImage", [], "begin inline image data"],
  ["EI", "inlineImage", [], "end inline image"],
  ["Do", "xObject", ["name"], "paint XObject"],
  ["MP", "markedContent", ["name"], "define marked-content point"],
  ["DP", "markedContent", ["name", "props"], "define marked-content point with property list"],
  ["BMC", "markedContent", ["name"], "begin marked-content sequence"],
  ["BDC", "markedContent", ["name", "props"], "begin marked-content sequence with property list"],
  ["EMC", "markedContent", [], "end marked-content sequence"],
  ["BX", "compatibility", [], "begin compatibility section"],
  ["EX", "compatibility", [], "end compatibility section"],
];

export const OPERATORS: Map<string, OperatorInfo> = new Map(
  table.map(([name, category, args, description]) => [name, { name, category, args, description }]),
);

/** Openers and their closers. */
export const CLOSERS: Record<string, string> = { q: "Q", BT: "ET", BMC: "EMC", BDC: "EMC", BX: "EX" };
const OPENER_OF: Record<string, string[]> = { Q: ["q"], ET: ["BT"], EMC: ["BMC", "BDC"], EX: ["BX"] };

/** The resource category an operator's name operand refers to. */
export const NAME_CATEGORY: Record<string, string> = {
  Tf: "Font",
  Do: "XObject",
  gs: "ExtGState",
  sh: "Shading",
  CS: "ColorSpace",
  cs: "ColorSpace",
  SCN: "Pattern",
  scn: "Pattern",
  BDC: "Properties",
  DP: "Properties",
};

export type TokKind = "num" | "name" | "str" | "array" | "dict" | "bool" | "null" | "op" | "image";

export interface Operand {
  kind: TokKind;
  from: number;
  to: number;
}

/** One operation in the text. */
export interface ScannedOp {
  operator: string;
  /** Start of the first operand (or of the operator). */
  from: number;
  /** The operator token. */
  opFrom: number;
  to: number;
  operands: Operand[];
}

export interface ScanProblem {
  from: number;
  to: number;
  severity: "error" | "warning";
  message: string;
}

export interface Scanned {
  ops: ScannedOp[];
  /** Operands after the last operator. */
  pending: Operand[];
  problems: ScanProblem[];
  /** Blocks by operation index: [opener, closer]; unclosed ones end at -1. */
  blocks: [number, number][];
}

const WS = new Set([0, 9, 10, 12, 13, 32]);
const DELIM = new Set([...`()<>[]{}/%`].map((c) => c.charCodeAt(0)));
const isWs = (c: number) => WS.has(c);
const isRegular = (c: number) => !WS.has(c) && !DELIM.has(c) && !Number.isNaN(c);
const NUMBER = /^[+-]?(\d+\.?\d*|\.\d+)$/;

/** End of a literal string starting at `i` (the `(`), or -1 if it is not closed. */
function stringEnd(t: string, i: number): number {
  let depth = 0;
  for (let k = i; k < t.length; k++) {
    const c = t[k];
    if (c === "\\") k++;
    else if (c === "(") depth++;
    else if (c === ")" && --depth === 0) return k + 1;
  }
  return -1;
}

/** End of a composite object (array or dictionary) starting at `i`, or -1. */
function compositeEnd(t: string, i: number): number {
  const stack: string[] = [];
  let k = i;
  while (k < t.length) {
    const c = t[k];
    if (c === "%") {
      while (k < t.length && t[k] !== "\n" && t[k] !== "\r") k++;
    } else if (c === "(") {
      const e = stringEnd(t, k);
      if (e < 0) return -1;
      k = e;
      continue;
    } else if (c === "<" && t[k + 1] === "<") {
      stack.push(">>");
      k += 2;
      continue;
    } else if (c === "<") {
      const e = t.indexOf(">", k);
      if (e < 0) return -1;
      k = e + 1;
      continue;
    } else if (c === "[") stack.push("]");
    else if (c === ">" && t[k + 1] === ">") {
      if (stack.pop() !== ">>") return -1;
      k += 2;
      if (stack.length === 0) return k;
      continue;
    } else if (c === "]") {
      if (stack.pop() !== "]") return -1;
      if (stack.length === 0) return k + 1;
    }
    k++;
  }
  return -1;
}

/** Where the data of an inline image ends: before whitespace + `EI` + a delimiter. */
function imageEnd(t: string, i: number): number {
  const re = /\sEI(?=[\s/[<(%]|$)/g;
  re.lastIndex = i;
  const m = re.exec(t);
  return m ? m.index + 1 : -1;
}

const cache = new WeakMap<object, Scanned>();

/** Scan `text` (cached by `key`, e.g. a CodeMirror document). */
export function scanCached(key: object, text: () => string): Scanned {
  let s = cache.get(key);
  if (!s) {
    s = scan(text());
    cache.set(key, s);
  }
  return s;
}

/** Split content stream text into operations, noting what a parser would refuse. */
export function scan(t: string): Scanned {
  const ops: ScannedOp[] = [];
  const problems: ScanProblem[] = [];
  let pending: Operand[] = [];
  let i = 0;
  const n = t.length;
  const operand = (kind: TokKind, from: number, to: number) => pending.push({ kind, from, to });
  while (i < n) {
    const c = t.charCodeAt(i);
    if (isWs(c)) {
      i++;
      continue;
    }
    const ch = t[i];
    if (ch === "%") {
      while (i < n && t[i] !== "\n" && t[i] !== "\r") i++;
      continue;
    }
    if (ch === "(") {
      const e = stringEnd(t, i);
      if (e < 0) {
        problems.push({ from: i, to: n, severity: "error", message: "string not closed" });
        break;
      }
      operand("str", i, e);
      i = e;
      continue;
    }
    if (ch === "<" && t[i + 1] !== "<") {
      const e = t.indexOf(">", i);
      if (e < 0) {
        problems.push({ from: i, to: n, severity: "error", message: "hex string not closed" });
        break;
      }
      if (/[^0-9a-fA-F\s]/.test(t.slice(i + 1, e)))
        problems.push({ from: i, to: e + 1, severity: "warning", message: "hex strings hold only 0-9, a-f and white space" });
      operand("str", i, e + 1);
      i = e + 1;
      continue;
    }
    if (ch === "[" || (ch === "<" && t[i + 1] === "<")) {
      const e = compositeEnd(t, i);
      if (e < 0) {
        problems.push({ from: i, to: n, severity: "error", message: ch === "[" ? "array not closed" : "dictionary not closed" });
        break;
      }
      operand(ch === "[" ? "array" : "dict", i, e);
      i = e;
      continue;
    }
    if (ch === "]" || ch === ">" || ch === ")" || ch === "{" || ch === "}") {
      problems.push({ from: i, to: i + 1, severity: "error", message: `unexpected '${ch}'` });
      i++;
      continue;
    }
    if (ch === "/") {
      let e = i + 1;
      while (e < n && isRegular(t.charCodeAt(e))) e++;
      operand("name", i, e);
      i = e;
      continue;
    }
    let e = i;
    while (e < n && isRegular(t.charCodeAt(e))) e++;
    const word = t.slice(i, e);
    if (NUMBER.test(word)) operand("num", i, e);
    else if (word === "true" || word === "false") operand("bool", i, e);
    else if (word === "null") operand("null", i, e);
    else if (word === "BI") {
      // BI key value … ID data EI is one operation.
      const id = /\sID[\s]/g;
      id.lastIndex = e;
      const m = id.exec(t);
      const end = m ? imageEnd(t, m.index + 4) : -1;
      if (end < 0) {
        problems.push({ from: i, to: n, severity: "error", message: m ? "no EI after the inline image data" : "BI without ID" });
        break;
      }
      if (pending.length) problems.push({ from: pending[0].from, to: i, severity: "warning", message: "operands before BI are ignored" });
      ops.push({ operator: "BI", from: i, opFrom: i, to: end + 2, operands: [] });
      pending = [];
      i = end + 2;
      continue;
    } else {
      if (!OPERATORS.has(word)) problems.push({ from: i, to: e, severity: "warning", message: `unknown operator ${word}` });
      ops.push({ operator: word, from: pending.length ? pending[0].from : i, opFrom: i, to: e, operands: pending });
      pending = [];
    }
    i = e;
  }
  if (pending.length)
    problems.push({
      from: pending[0].from,
      to: pending[pending.length - 1].to,
      severity: "error",
      message: `${pending.length} operand${pending.length === 1 ? "" : "s"} without an operator`,
    });
  for (const op of ops) checkOperands(op, problems);
  return { ops, pending, problems, blocks: blocksOf(ops, problems) };
}

const kindOk = (want: ArgKind, got: TokKind) =>
  want === "num" || want === "int"
    ? got === "num"
    : want === "name"
      ? got === "name"
      : want === "str"
        ? got === "str"
        : want === "array"
          ? got === "array"
          : got === "dict" || got === "name";

function checkOperands(op: ScannedOp, problems: ScanProblem[]) {
  const info = OPERATORS.get(op.operator);
  if (!info || info.args === null) return;
  const want = info.args;
  const got = op.operands;
  if (got.length < want.length) {
    problems.push({
      from: op.opFrom,
      to: op.to,
      severity: "error",
      message: `${op.operator} takes ${want.length} operand${want.length === 1 ? "" : "s"} (${want.join(", ")}), ${got.length} given`,
    });
    return;
  }
  if (got.length > want.length)
    problems.push({
      from: got[0].from,
      to: got[got.length - want.length - 1].to,
      severity: "warning",
      message: `${op.operator} takes ${want.length} operand${want.length === 1 ? "" : "s"}: the extra ones are ignored`,
    });
  const used = got.slice(got.length - want.length);
  want.forEach((w, k) => {
    const o = used[k];
    if (!kindOk(w, o.kind)) problems.push({ from: o.from, to: o.to, severity: "warning", message: `${op.operator}: operand ${k + 1} should be a ${w}` });
  });
}

function blocksOf(ops: ScannedOp[], problems: ScanProblem[]): [number, number][] {
  const open: number[] = [];
  const blocks: [number, number][] = [];
  ops.forEach((op, k) => {
    if (op.operator in CLOSERS) {
      open.push(k);
      return;
    }
    const openers = OPENER_OF[op.operator];
    if (!openers) return;
    let j = open.length - 1;
    while (j >= 0 && !openers.includes(ops[open[j]].operator)) j--;
    if (j < 0) {
      problems.push({ from: op.opFrom, to: op.to, severity: "warning", message: `${op.operator} without ${openers.join(" or ")} before it in this text` });
      return;
    }
    for (const s of open.splice(j + 1)) {
      problems.push({ from: ops[s].opFrom, to: ops[s].to, severity: "warning", message: `${ops[s].operator} not closed before ${op.operator}` });
      blocks.push([s, k - 1]);
    }
    blocks.push([open.pop()!, k]);
  });
  for (const s of open) {
    problems.push({ from: ops[s].opFrom, to: ops[s].to, severity: "warning", message: `${ops[s].operator} not closed in this text` });
    blocks.push([s, -1]);
  }
  return blocks.sort((a, b) => a[0] - b[0]);
}

/** The operation at or before `pos` (index into `ops`), or -1. */
export function opAt(ops: ScannedOp[], pos: number): number {
  let lo = 0;
  let hi = ops.length - 1;
  let best = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (ops[mid].from <= pos) {
      best = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return best;
}

/** Characters beyond one byte (content streams hold bytes): their positions. */
export function nonBytes(t: string): number[] {
  const out: number[] = [];
  for (let i = 0; i < t.length; i++) if (t.charCodeAt(i) > 0xff) out.push(i);
  return out;
}
