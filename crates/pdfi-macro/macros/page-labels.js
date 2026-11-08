// @ts-check
/// <reference path="../js/pdfi.d.ts" />

export const meta = {
  api: 1,
  name: "page-labels",
  title: "Set page labels",
  description: "Replace the page labels (the page numbers viewers show), like 1:r, 5:D, 120:D:A-",
  edits: true,
  save: "incremental",
  params: {
    labels: {
      type: "string",
      description:
        'Ranges PAGE:STYLE[:PREFIX[:FIRST]], comma-separated: PAGE counts from 1; STYLE is D (1, 2), R (I, II), r (i, ii), A (A, B), a (a, b) or - (prefix only); "none" removes the labels',
    },
  },
};

const STYLES = { D: "decimal", R: "upperRoman", r: "lowerRoman", A: "upperLetters", a: "lowerLetters" };

/** @type {pdfi.Main<{ labels: string }>} */
export default async function (doc, params) {
  const spec = params.labels.trim();
  /** @type {pdfi.PageLabelInput[]} */
  const ranges = [];
  if (spec !== "" && spec.toLowerCase() !== "none") {
    for (const part of spec.split(",").map((s) => s.trim()).filter(Boolean)) {
      const [page, style = "D", prefix = "", first = "1"] = part.split(":");
      const start = Number(page) - 1;
      if (!Number.isInteger(start) || start < 0 || start >= doc.pageCount)
        throw new RangeError(`${JSON.stringify(part)}: page ${page} is not in the document (pages 1 to ${doc.pageCount})`);
      if (style !== "-" && !(style in STYLES)) throw new TypeError(`${JSON.stringify(part)}: style is one of D, R, r, A, a or -`);
      const n = Number(first);
      if (!Number.isInteger(n) || n < 1) throw new RangeError(`${JSON.stringify(part)}: the first number is 1 or more`);
      ranges.push({ start, style: style === "-" ? null : /** @type {"D"} */ (style), prefix, first: n });
    }
    if (!ranges.some((r) => r.start === 0)) ranges.unshift({ start: 0, style: "decimal" });
  }
  await doc.setPageLabels(ranges, meta.title);
  const out = [];
  for (const r of await doc.pageLabels()) out.push({ page: r.start + 1, style: r.style, prefix: r.prefix, first: r.first });
  return out;
}
