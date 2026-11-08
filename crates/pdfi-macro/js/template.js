// @ts-check
/// <reference path="pdfi.d.ts" />
// Types: `pdfi macro types -o pdfi.d.ts` writes pdfi.d.ts next to this file.

export const meta = {
  api: 1,
  name: "NAME",
  title: "NAME",
  description: "What it does",
  // Whether it changes the document (then the CLI needs -o, --out-dir or --in-place).
  edits: false,
  params: {
    pages: { type: "pages", optional: true, description: "Pages to look at, like 1-3,8 (all when not given)" },
  },
};

/** @type {pdfi.Main<{ pages?: number[] }>} */
export default async function (doc, params, ctx) {
  const rows = [];
  for await (const page of doc.pages(params.pages)) {
    rows.push({ page: page.index + 1, label: page.label ?? "", operations: await page.content.count() });
  }
  ctx.log(`${doc.name}: ${rows.length} pages`);
  return rows;
}
