import { Badge, Code, Divider, Group, Loader, Stack, Table, Text, Title } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { api, useBackend } from "../api";
import { contentKey, useNav } from "../nav";
import type { ContentRef } from "../protocol/generated";
import { categoryColor, categoryLabel } from "./ContentView";

interface Operand {
  name: string;
  meaning: string;
  /** Names of the values it can take, by value. */
  values?: Record<string, string>;
}

interface OpDoc {
  /** Section of ISO 32000-2 defining it. */
  section: string;
  operands: Operand[];
  /** Takes as many operands as the color space has components (`pattern`: and then maybe a pattern name). */
  variadic?: "components" | "pattern";
  text: string;
}

const xy = (what: string): Operand[] => [
  { name: "x", meaning: `x of the ${what} (user space)` },
  { name: "y", meaning: `y of the ${what} (user space)` },
];
const matrix = (of: string): Operand[] =>
  ["a", "b", "c", "d", "e", "f"].map((name, i) => ({
    name,
    meaning: i < 4 ? `${of}: ${["x scale / rotation", "rotation / skew", "skew / rotation", "y scale / rotation"][i]}` : `${of}: ${i === 4 ? "x" : "y"} translation`,
  }));
const capStyles = { "0": "butt cap", "1": "round cap", "2": "projecting square cap" };
const joinStyles = { "0": "miter join", "1": "round join", "2": "bevel join" };
const renderModes = {
  "0": "fill",
  "1": "stroke",
  "2": "fill, then stroke",
  "3": "invisible",
  "4": "fill and add to clipping path",
  "5": "stroke and add to clipping path",
  "6": "fill, stroke and add to clipping path",
  "7": "add to clipping path",
};
const paint = (how: string, close = false): OpDoc => ({
  section: "8.5.3",
  operands: [],
  text: `${close ? "Closes the current subpath, then " : ""}${how} the current path with the current graphics state, then ends it (the path is discarded; a pending W / W* clips with it first).`,
});
const name = (meaning: string): Operand => ({ name: "name", meaning });
const tag: Operand = { name: "tag", meaning: "role or significance of the content (a name)" };
const props: Operand = { name: "properties", meaning: "property list: an inline dictionary, or the name of one in /Properties of the resources" };

/** What the specification says about each operator. */
const ops: Record<string, OpDoc> = {
  w: { section: "8.4.4", operands: [{ name: "lineWidth", meaning: "thickness of stroked lines, in user space units (0: the thinnest line the device can draw)" }], text: "Sets the line width used by stroking." },
  J: { section: "8.4.4", operands: [{ name: "lineCap", meaning: "shape at the ends of open subpaths", values: capStyles }], text: "Sets the line cap style used by stroking." },
  j: { section: "8.4.4", operands: [{ name: "lineJoin", meaning: "shape at the corners of paths", values: joinStyles }], text: "Sets the line join style used by stroking." },
  M: { section: "8.4.4", operands: [{ name: "miterLimit", meaning: "maximum ratio of miter length to line width before a miter join is beveled" }], text: "Sets the miter limit." },
  d: {
    section: "8.4.4",
    operands: [
      { name: "dashArray", meaning: "lengths of alternating dashes and gaps ([]: solid line)" },
      { name: "dashPhase", meaning: "distance into the pattern at which to start" },
    ],
    text: "Sets the line dash pattern used by stroking.",
  },
  ri: { section: "8.4.4", operands: [{ name: "intent", meaning: "rendering intent: /AbsoluteColorimetric, /RelativeColorimetric, /Saturation or /Perceptual" }], text: "Sets the color rendering intent." },
  i: { section: "8.4.4", operands: [{ name: "flatness", meaning: "flatness tolerance, 0 to 100 (0: the device's default)" }], text: "Sets the flatness tolerance: how precisely curves are approximated by straight lines." },
  gs: { section: "8.4.4", operands: [name("graphics state parameter dictionary in /ExtGState of the resources")], text: "Sets the parameters given in a graphics state parameter dictionary (line style, font, transparency, soft mask, overprint …)." },
  q: { section: "8.4.4", operands: [], text: "Saves the whole graphics state on the graphics state stack. Must be balanced by Q in the same content." },
  Q: { section: "8.4.4", operands: [], text: "Restores the graphics state saved by the matching q." },
  cm: { section: "8.4.4", operands: matrix("matrix"), text: "Modifies the current transformation matrix (CTM) by concatenating [a b c d e f] to it: what follows is drawn in the new coordinate system." },
  m: { section: "8.5.2", operands: xy("new current point"), text: "Begins a new subpath by moving the current point to (x, y), without drawing." },
  l: { section: "8.5.2", operands: xy("end point"), text: "Appends a straight line from the current point to (x, y)." },
  c: {
    section: "8.5.2",
    operands: [
      { name: "x1", meaning: "first control point x" },
      { name: "y1", meaning: "first control point y" },
      { name: "x2", meaning: "second control point x" },
      { name: "y2", meaning: "second control point y" },
      { name: "x3", meaning: "end point x" },
      { name: "y3", meaning: "end point y" },
    ],
    text: "Appends a cubic Bézier curve from the current point to (x3, y3), with control points (x1, y1) and (x2, y2).",
  },
  v: {
    section: "8.5.2",
    operands: [
      { name: "x2", meaning: "second control point x" },
      { name: "y2", meaning: "second control point y" },
      { name: "x3", meaning: "end point x" },
      { name: "y3", meaning: "end point y" },
    ],
    text: "Appends a cubic Bézier curve to (x3, y3) whose first control point is the current point.",
  },
  y: {
    section: "8.5.2",
    operands: [
      { name: "x1", meaning: "first control point x" },
      { name: "y1", meaning: "first control point y" },
      { name: "x3", meaning: "end point x" },
      { name: "y3", meaning: "end point y" },
    ],
    text: "Appends a cubic Bézier curve to (x3, y3) whose second control point is the end point.",
  },
  h: { section: "8.5.2", operands: [], text: "Closes the current subpath with a straight line back to its starting point." },
  re: {
    section: "8.5.2",
    operands: [
      { name: "x", meaning: "x of a corner (user space)" },
      { name: "y", meaning: "y of a corner (user space)" },
      { name: "width", meaning: "width (negative: extends left)" },
      { name: "height", meaning: "height (negative: extends down)" },
    ],
    text: "Appends a rectangle as a complete closed subpath.",
  },
  S: paint("Strokes"),
  s: paint("strokes", true),
  f: paint("Fills (nonzero winding rule)"),
  F: { ...paint("Fills (nonzero winding rule)"), text: "Same as f; kept for compatibility (PDF writers should use f)." },
  "f*": paint("Fills (even-odd rule)"),
  B: paint("Fills (nonzero winding rule) and then strokes"),
  "B*": paint("Fills (even-odd rule) and then strokes"),
  b: paint("fills (nonzero winding rule) and then strokes", true),
  "b*": paint("fills (even-odd rule) and then strokes", true),
  n: { section: "8.5.3", operands: [], text: "Ends the path without filling or stroking it. Used after W / W* to set a clipping path without painting." },
  W: { section: "8.5.4", operands: [], text: "Intersects the clipping path with the current path (nonzero winding rule). Takes effect after the next path-painting operator (usually n)." },
  "W*": { section: "8.5.4", operands: [], text: "Intersects the clipping path with the current path (even-odd rule). Takes effect after the next path-painting operator (usually n)." },
  BT: { section: "9.4.1", operands: [], text: "Begins a text object: resets the text matrix and text line matrix to the identity. Text objects do not nest." },
  ET: { section: "9.4.1", operands: [], text: "Ends a text object, discarding the text matrix. Glyphs drawn with a clipping render mode are added to the clipping path here." },
  Tc: { section: "9.3.2", operands: [{ name: "charSpace", meaning: "extra space added after each glyph, in unscaled text space units" }], text: "Sets the character spacing." },
  Tw: { section: "9.3.3", operands: [{ name: "wordSpace", meaning: "extra space added after each single-byte code 32 (space), in unscaled text space units" }], text: "Sets the word spacing." },
  Tz: { section: "9.3.4", operands: [{ name: "scale", meaning: "horizontal scaling of glyphs, in percent (100: normal)" }], text: "Sets the horizontal scaling." },
  TL: { section: "9.3.5", operands: [{ name: "leading", meaning: "vertical distance between the baselines of lines, in unscaled text space units" }], text: "Sets the text leading, used by T*, ' and \"." },
  Tf: {
    section: "9.3.1",
    operands: [
      { name: "font", meaning: "font dictionary in /Font of the resources" },
      { name: "size", meaning: "scale factor applied to the glyphs (the font size)" },
    ],
    text: "Sets the text font and size.",
  },
  Tr: { section: "9.3.6", operands: [{ name: "render", meaning: "text rendering mode", values: renderModes }], text: "Sets the text rendering mode: whether glyphs are filled, stroked, added to the clipping path or invisible." },
  Ts: { section: "9.3.7", operands: [{ name: "rise", meaning: "distance to move the baseline up (negative: down), in unscaled text space units" }], text: "Sets the text rise, for superscripts and subscripts." },
  Td: {
    section: "9.4.2",
    operands: [
      { name: "tx", meaning: "x offset, in unscaled text space units" },
      { name: "ty", meaning: "y offset, in unscaled text space units" },
    ],
    text: "Moves to the start of the next line, offset from the start of the current line by (tx, ty).",
  },
  TD: {
    section: "9.4.2",
    operands: [
      { name: "tx", meaning: "x offset, in unscaled text space units" },
      { name: "ty", meaning: "y offset, in unscaled text space units" },
    ],
    text: "Same as Td, and sets the leading to −ty (as -ty TL tx ty Td).",
  },
  Tm: { section: "9.4.2", operands: matrix("text matrix"), text: "Sets the text matrix and the text line matrix to [a b c d e f]. Does not concatenate: it replaces them." },
  "T*": { section: "9.4.2", operands: [], text: "Moves to the start of the next line (as 0 −leading Td)." },
  Tj: { section: "9.4.3", operands: [{ name: "string", meaning: "character codes, decoded through the current font's encoding" }], text: "Shows a text string: paints its glyphs and advances the text matrix past them." },
  "'": { section: "9.4.3", operands: [{ name: "string", meaning: "character codes to show" }], text: "Moves to the next line and shows a text string (as T* string Tj)." },
  '"': {
    section: "9.4.3",
    operands: [
      { name: "aw", meaning: "word spacing (as Tw)" },
      { name: "ac", meaning: "character spacing (as Tc)" },
      { name: "string", meaning: "character codes to show" },
    ],
    text: "Sets the word and character spacing, moves to the next line and shows a text string (as aw Tw ac Tc string ').",
  },
  TJ: {
    section: "9.4.3",
    operands: [{ name: "array", meaning: "strings to show, and numbers moving the next glyph back (left, in horizontal writing) by number/1000 of text space units" }],
    text: "Shows text strings, adjusting the position between them: kerning and justification.",
  },
  d0: {
    section: "9.6.5",
    operands: [
      { name: "wx", meaning: "horizontal displacement of the glyph, in glyph space" },
      { name: "wy", meaning: "vertical displacement (must be 0)" },
    ],
    text: "Sets the width of a Type 3 glyph and declares that its description sets its own colors. First operator of a glyph procedure.",
  },
  d1: {
    section: "9.6.5",
    operands: [
      { name: "wx", meaning: "horizontal displacement of the glyph, in glyph space" },
      { name: "wy", meaning: "vertical displacement (must be 0)" },
      { name: "llx", meaning: "glyph bounding box: lower-left x" },
      { name: "lly", meaning: "glyph bounding box: lower-left y" },
      { name: "urx", meaning: "glyph bounding box: upper-right x" },
      { name: "ury", meaning: "glyph bounding box: upper-right y" },
    ],
    text: "Sets the width and bounding box of a Type 3 glyph that is a shape painted in the current color (color operators are ignored in it). First operator of a glyph procedure.",
  },
  CS: { section: "8.6.8", operands: [name("color space: a device or Pattern family name, or a name in /ColorSpace of the resources")], text: "Sets the color space for stroking, and the stroking color to that space's initial color." },
  cs: { section: "8.6.8", operands: [name("color space: a device or Pattern family name, or a name in /ColorSpace of the resources")], text: "Sets the color space for filling (nonstroking), and the fill color to that space's initial color." },
  SC: { section: "8.6.8", operands: [], variadic: "components", text: "Sets the stroking color in the current stroking color space (device, CIE-based or Indexed spaces)." },
  SCN: { section: "8.6.8", operands: [], variadic: "pattern", text: "Sets the stroking color in the current stroking color space; also for Pattern, Separation, DeviceN and ICCBased spaces." },
  sc: { section: "8.6.8", operands: [], variadic: "components", text: "Sets the fill color in the current fill color space (device, CIE-based or Indexed spaces)." },
  scn: { section: "8.6.8", operands: [], variadic: "pattern", text: "Sets the fill color in the current fill color space; also for Pattern, Separation, DeviceN and ICCBased spaces." },
  G: { section: "8.6.8", operands: [{ name: "gray", meaning: "gray level, 0 (black) to 1 (white)" }], text: "Sets the stroking color space to DeviceGray and the stroking color." },
  g: { section: "8.6.8", operands: [{ name: "gray", meaning: "gray level, 0 (black) to 1 (white)" }], text: "Sets the fill color space to DeviceGray and the fill color." },
  RG: {
    section: "8.6.8",
    operands: ["red", "green", "blue"].map((c) => ({ name: c[0], meaning: `${c}, 0 to 1` })),
    text: "Sets the stroking color space to DeviceRGB and the stroking color.",
  },
  rg: {
    section: "8.6.8",
    operands: ["red", "green", "blue"].map((c) => ({ name: c[0], meaning: `${c}, 0 to 1` })),
    text: "Sets the fill color space to DeviceRGB and the fill color.",
  },
  K: {
    section: "8.6.8",
    operands: ["cyan", "magenta", "yellow", "black"].map((c) => ({ name: c === "black" ? "k" : c[0], meaning: `${c}, 0 to 1` })),
    text: "Sets the stroking color space to DeviceCMYK and the stroking color.",
  },
  k: {
    section: "8.6.8",
    operands: ["cyan", "magenta", "yellow", "black"].map((c) => ({ name: c === "black" ? "k" : c[0], meaning: `${c}, 0 to 1` })),
    text: "Sets the fill color space to DeviceCMYK and the fill color.",
  },
  sh: { section: "8.7.4.2", operands: [name("shading dictionary in /Shading of the resources")], text: "Paints the shading over the current clipping region (the whole of it, unless the shading has a /BBox), independently of the current path." },
  BI: { section: "8.9.7", operands: [], text: "Begins an inline image: its parameters follow as key–value pairs (abbreviated keys allowed), then ID, the image data and EI." },
  ID: { section: "8.9.7", operands: [], text: "Begins the data of an inline image." },
  EI: { section: "8.9.7", operands: [], text: "Ends an inline image and paints it in the unit square of user space." },
  Do: { section: "8.8", operands: [name("XObject in /XObject of the resources")], text: "Paints an XObject: an image is painted in the unit square of user space, a form XObject's content is run with its own /Matrix and resources." },
  MP: { section: "14.6", operands: [tag], text: "Marks a point in the content." },
  DP: { section: "14.6", operands: [tag, props], text: "Marks a point in the content, with a property list." },
  BMC: { section: "14.6", operands: [tag], text: "Begins a marked-content sequence, ended by the matching EMC." },
  BDC: { section: "14.6", operands: [tag, props], text: "Begins a marked-content sequence with a property list (e.g. /MCID for the structure tree, or an optional content group with /OC), ended by the matching EMC." },
  EMC: { section: "14.6", operands: [], text: "Ends the marked-content sequence begun by the matching BMC or BDC." },
  BX: { section: "7.8.2", operands: [], text: "Begins a compatibility section: unknown operators inside it are ignored instead of being errors." },
  EX: { section: "7.8.2", operands: [], text: "Ends a compatibility section." },
};

/** Operand names and meanings for the operands written, variadic color operators included. */
function operandsOf(d: OpDoc, count: number): Operand[] {
  if (!d.variadic) return d.operands;
  const out: Operand[] = [];
  for (let i = 0; i < count; i++) {
    const last = d.variadic === "pattern" && i === count - 1;
    out.push(last ? { name: "name", meaning: "pattern in /Pattern of the resources (Pattern color space), when last and a name" } : { name: `c${i + 1}`, meaning: "color component" });
  }
  return out;
}

/** The selected operation of a content explained: what its operator does and what each operand means. */
export function OperatorDoc({ content, op }: { content: ContentRef; op: number | null }) {
  const backend = useBackend();
  const nav = useNav();
  const ov = useQuery({
    queryKey: ["doc", nav.doc, "content", ...contentKey(content), "op", op],
    queryFn: async () => (await api.contentOps(backend, nav.doc, content, { offset: op!, limit: 1 })).items[0] ?? null,
    enabled: op !== null,
  });
  if (op === null)
    return (
      <Text size="xs" c="dimmed" p="sm">
        Select an operation to see what its operator does (ISO 32000-2).
      </Text>
    );
  const o = ov.data;
  if (!o) return ov.isPending ? <Loader m="sm" size="xs" /> : null;
  const d = ops[o.operator];
  const color = categoryColor[o.category] ?? "gray";
  const operands = d ? operandsOf(d, o.operands.length) : [];
  const rows = Math.max(operands.length, o.operands.length);
  return (
    <Stack gap="xs" p="sm">
      <div>
        <Group gap={6}>
          <Title order={6} ff="monospace" style={{ color: `var(--mantine-color-${color}-text)` }}>
            {o.operator}
          </Title>
          <Badge size="xs" variant="dot" color={color} tt="none">
            {categoryLabel[o.category] ?? o.category}
          </Badge>
        </Group>
        <Text size="xs" c="dimmed">
          #{o.index}
          {d && ` · ISO 32000-2 §${d.section}`}
        </Text>
      </div>
      {d ? (
        <Text size="xs">{d.text}</Text>
      ) : (
        <Text size="xs" c="dimmed">
          Not an operator of the specification: a reader ignores it inside BX … EX, and reports an error elsewhere.
        </Text>
      )}
      {rows > 0 && (
        <>
          <Divider label="Operands" labelPosition="left" />
          <Table withRowBorders={false} verticalSpacing={2} horizontalSpacing={4} fz="xs">
            <Table.Tbody>
              {Array.from({ length: rows }, (_, i) => {
                const spec = operands[i];
                const value = o.operands[i];
                const named = spec?.values && value !== undefined ? spec.values[value] : undefined;
                return (
                  <Table.Tr key={i}>
                    <Table.Td w={72} style={{ verticalAlign: "top" }}>
                      <Text size="xs" ff="monospace" c={spec ? undefined : "red"}>
                        {spec?.name ?? "(extra)"}
                      </Text>
                    </Table.Td>
                    <Table.Td style={{ wordBreak: "break-word" }}>
                      {value === undefined ? (
                        <Text size="xs" c="red">
                          missing
                        </Text>
                      ) : (
                        <Code>{value}</Code>
                      )}
                      {named && <Text span size="xs"> {named}</Text>}
                      {spec && (
                        <Text size="xs" c="dimmed">
                          {spec.meaning}
                        </Text>
                      )}
                      {spec?.values && (
                        <Text size="xs" c="dimmed">
                          {Object.entries(spec.values)
                            .map(([v, n]) => `${v}: ${n}`)
                            .join(" · ")}
                        </Text>
                      )}
                    </Table.Td>
                  </Table.Tr>
                );
              })}
            </Table.Tbody>
          </Table>
        </>
      )}
    </Stack>
  );
}
