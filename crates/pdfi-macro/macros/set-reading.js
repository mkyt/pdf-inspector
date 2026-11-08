// @ts-check
/// <reference path="../js/pdfi.d.ts" />

export const meta = {
  api: 1,
  name: "set-reading",
  title: "Set page layout and reading direction",
  description: "Set the catalog's /PageLayout and /ViewerPreferences /Direction (spreads, binding side)",
  edits: true,
  save: "incremental",
  params: {
    layout: {
      type: "enum",
      values: ["SinglePage", "OneColumn", "TwoColumnLeft", "TwoColumnRight", "TwoPageLeft", "TwoPageRight"],
      default: "TwoPageRight",
      description: "How pages are shown: TwoPage* shows spreads, *Right puts odd pages on the right",
    },
    direction: {
      type: "enum",
      values: ["L2R", "R2L"],
      default: "R2L",
      description: "Reading direction: R2L for right-to-left (bound on the right)",
    },
  },
};

/** @type {pdfi.Main<{ layout: string, direction: string }>} */
export default async function (doc, params) {
  await doc.catalog.set("PageLayout", pdf.name(params.layout));
  const prefs = await doc.catalog.dict("ViewerPreferences", { create: true });
  await prefs?.set("Direction", pdf.name(params.direction));
  return { layout: params.layout, direction: params.direction };
}
