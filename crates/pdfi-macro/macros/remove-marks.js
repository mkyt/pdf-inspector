// @ts-check
/// <reference path="../js/pdfi.d.ts" />

export const meta = {
  api: 1,
  name: "remove-marks",
  title: "Remove printer's marks and watermarks",
  description:
    "Remove crop marks and other printer's marks (outside the TrimBox, or in the registration color), watermarks, layers or text from page contents",
  edits: true,
  save: "incremental",
  params: {
    outsideTrim: {
      type: "boolean",
      default: true,
      description: "What paints wholly outside the TrimBox (pages without a TrimBox are left)",
    },
    registration: {
      type: "boolean",
      default: true,
      description: "What paints in the registration color (Separation /All)",
    },
    watermarks: {
      type: "boolean",
      default: true,
      description: "Marked content /Artifact with /Subtype /Watermark, and layers named like 'watermark'",
    },
    layers: {
      type: "string",
      optional: true,
      description: "Content of the layers (optional content groups) whose names match this regular expression",
    },
    text: {
      type: "string",
      optional: true,
      description: "Text containing this (a whole text-showing operation, or the form painting it)",
    },
    pages: { type: "pages", optional: true, description: "Pages, like 1-3,8 (all when not given)" },
  },
};

/**
 * @type {pdfi.Main<{ outsideTrim: boolean, registration: boolean, watermarks: boolean,
 *   layers?: string, text?: string, pages?: number[] }>}
 */
export default async function (doc, params, ctx) {
  const layers = params.layers === undefined ? null : new RegExp(params.layers, "i");
  const total = params.pages?.length ?? doc.pageCount;
  const rows = [];
  let done = 0;
  for await (const page of doc.pages(params.pages)) {
    ctx.progress(done++, total, `page ${page.index + 1}`);
    const c = page.content;
    /** @type {pdfi.Hit[]} */
    const hits = [];
    const counts = { outsideTrim: 0, registration: 0, watermarks: 0, layers: 0, text: 0 };
    /** @param {keyof typeof counts} why @param {pdfi.Hit[]} found */
    const add = (why, found) => {
      counts[why] += found.length;
      hits.push(...found);
    };
    if (params.outsideTrim) {
      const boxes = await page.boxes();
      if (boxes.trim && boxes.origins.trim.kind !== "default") add("outsideTrim", await c.find({ bbox: { outside: boxes.trim } }));
    }
    if (params.registration) add("registration", await c.find({ colorSpace: { separation: "All" } }));
    if (params.watermarks) {
      add("watermarks", await c.find({ markedContent: { tag: "Artifact", properties: { Subtype: "Watermark" } }, unit: "block" }));
      add("watermarks", await c.find({ ocg: /watermark/i, unit: "block" }));
    }
    if (layers) add("layers", await c.find({ ocg: layers, unit: "block" }));
    if (params.text !== undefined) add("text", await c.find({ text: params.text }));
    const removed = hits.length ? await c.remove(hits, meta.title) : 0;
    if (removed) rows.push({ page: page.index + 1, ...counts, operations: removed });
  }
  ctx.progress(total, total);
  if (!rows.length) ctx.log("nothing found to remove");
  return rows;
}
