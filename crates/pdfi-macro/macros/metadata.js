// @ts-check
/// <reference path="../js/pdfi.d.ts" />

export const meta = {
  api: 1,
  name: "metadata",
  title: "Metadata",
  description: "The Info dictionary, the XMP properties and what the catalog declares (layout, mode, language, viewer preferences)",
  edits: false,
};

const DECLARED = ["Version", "PageLayout", "PageMode", "Lang"];

/** @type {pdfi.Main} */
export default async function (doc) {
  /** @type {Record<string, unknown>} */
  const catalog = {};
  for (const key of DECLARED) {
    const v = await doc.catalog.get(key);
    if (v !== undefined) catalog[key] = v;
  }
  const prefs = await doc.catalog.dict("ViewerPreferences");
  if (prefs) catalog.ViewerPreferences = Object.fromEntries(await prefs.entries());
  const xmp = await doc.xmp();
  const e = doc.encryption;
  return {
    version: doc.version,
    pages: doc.pageCount,
    encryption: e && { filter: e.filter, revision: e.r, keyBits: e.keyBits, permissions: e.permissions },
    info: await doc.info(),
    xmp: xmp && Object.fromEntries(xmp.properties.map((p) => [p.name, p.value])),
    catalog,
  };
}
