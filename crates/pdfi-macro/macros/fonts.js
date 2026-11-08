// @ts-check
/// <reference path="../js/pdfi.d.ts" />

export const meta = {
  api: 1,
  name: "fonts",
  title: "Fonts",
  description: "The fonts of the document: name, type, encoding, embedding, subset, ToUnicode and how many pages use them",
  edits: false,
};

/** @type {pdfi.Main} */
export default async function (doc) {
  const fonts = await doc.fonts();
  return fonts.map((f) => ({
    font: f.baseFont ?? "",
    type: f.subtype ?? "",
    encoding: f.encoding ?? "",
    embedded: f.embedded ?? "no",
    subset: f.subset,
    toUnicode: f.toUnicode,
    pages: f.pageCount,
    object: f.id.num,
  }));
}
