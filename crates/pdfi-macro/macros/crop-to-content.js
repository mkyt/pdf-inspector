// @ts-check
/// <reference path="../js/pdfi.d.ts" />

export const meta = {
  api: 1,
  name: "crop-to-content",
  title: "Crop to content",
  description: "Set each page's CropBox to what its content paints, with a margin, within the MediaBox",
  edits: true,
  save: "incremental",
  params: {
    margin: { type: "number", default: 0, description: "Points added on every side" },
    pages: { type: "pages", optional: true, description: "Pages to crop, like 1-3,8 (all when not given)" },
  },
};

/** @type {pdfi.Main<{ margin: number, pages?: number[] }>} */
export default async function (doc, params, ctx) {
  const total = params.pages?.length ?? doc.pageCount;
  const cropped = [];
  let done = 0;
  for await (const page of doc.pages(params.pages)) {
    ctx.progress(done++, total, `page ${page.index + 1}`);
    const painted = await page.content.bounds();
    const { media } = await page.boxes();
    if (!painted || !media) {
      ctx.warn(`page ${page.index + 1}: nothing painted, left as it is`);
      continue;
    }
    const m = params.margin;
    const crop = {
      x0: Math.max(media.x0, painted.x0 - m),
      y0: Math.max(media.y0, painted.y0 - m),
      x1: Math.min(media.x1, painted.x1 + m),
      y1: Math.min(media.y1, painted.y1 + m),
    };
    if (crop.x1 <= crop.x0 || crop.y1 <= crop.y0) {
      ctx.warn(`page ${page.index + 1}: paints outside its MediaBox, left as it is`);
      continue;
    }
    await page.setBox("crop", crop);
    const r = (n) => Math.round(n * 100) / 100;
    cropped.push({ page: page.index + 1, x0: r(crop.x0), y0: r(crop.y0), x1: r(crop.x1), y1: r(crop.y1) });
  }
  ctx.progress(total, total);
  return cropped;
}
