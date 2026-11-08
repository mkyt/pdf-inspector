// @ts-check
/// <reference path="../js/pdfi.d.ts" />

export const meta = {
  api: 1,
  name: "outline-from-json",
  title: "Outline from JSON",
  description: "Replace the outline (bookmarks) with a tree written in JSON: [{ title, page, children }]",
  edits: true,
  save: "incremental",
  params: {
    outline: {
      type: "json",
      description:
        'Items [{ "title": "…", "page": 1, "children": [...] }]: page counts from 1 (or a page label as a string); also "uri", "bold", "italic", "open", "color"',
    },
    offset: { type: "integer", default: 0, description: "Added to every page number (printed page numbers → PDF pages)" },
    open: { type: "integer", default: 0, description: "Levels shown open (items without their own \"open\")" },
  },
};

/** @type {pdfi.Main<{ outline: any, offset: number, open: number }>} */
export default async function (doc, params, ctx) {
  const items = Array.isArray(params.outline) ? params.outline : params.outline?.items;
  if (!Array.isArray(items)) throw new TypeError("the outline is an array of items (or { items: [...] })");
  // Page labels → page numbers, when some item names a label.
  /** @type {Map<string, number> | null} */
  let labels = null;
  const pageOf = async (/** @type {any} */ p, /** @type {string} */ title) => {
    if (p === undefined || p === null) return null;
    let i;
    if (typeof p === "string" && !/^\d+$/.test(p)) {
      if (!labels) {
        labels = new Map();
        for await (const pg of doc.pages()) if (pg.label !== null && !labels.has(pg.label)) labels.set(pg.label, pg.index);
      }
      i = labels.get(p);
      if (i === undefined) throw new RangeError(`${JSON.stringify(title)}: no page labeled ${JSON.stringify(p)}`);
    } else i = Number(p) - 1 + params.offset;
    if (!Number.isInteger(i) || i < 0 || i >= doc.pageCount)
      throw new RangeError(`${JSON.stringify(title)}: page ${p} is not in the document (pages 1 to ${doc.pageCount})`);
    return i;
  };
  let count = 0;
  /** @returns {Promise<pdfi.OutlineInput[]>} */
  const convert = async (/** @type {any[]} */ list, /** @type {number} */ depth) => {
    const out = [];
    for (const it of list) {
      if (!it || typeof it.title !== "string") throw new TypeError(`an item needs a "title": ${JSON.stringify(it)}`);
      count++;
      out.push({
        title: it.title,
        page: await pageOf(it.page, it.title),
        view: it.view ?? null,
        dest: it.dest ?? null,
        uri: it.uri ?? null,
        open: it.open ?? depth < params.open,
        bold: !!it.bold,
        italic: !!it.italic,
        color: it.color ?? null,
        children: await convert(it.children ?? [], depth + 1),
      });
    }
    return out;
  };
  const tree = await convert(items, 0);
  await doc.setOutline(tree, meta.title);
  ctx.log(`${count} outline items`);
  return { items: count };
}
