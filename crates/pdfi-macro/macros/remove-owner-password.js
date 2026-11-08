// @ts-check
/// <reference path="../js/pdfi.d.ts" />

export const meta = {
  api: 1,
  name: "remove-owner-password",
  title: "Remove the owner password",
  description:
    "Write the document decrypted, without its permissions. It opens with the empty user password (give --password when it has another one); the full rewrite does the rest",
  edits: true,
  save: "full",
};

/** @type {pdfi.Main} */
export default async function (doc, _params, ctx) {
  const e = doc.encryption;
  if (!e) {
    ctx.warn("the document is not encrypted");
    return { encrypted: false };
  }
  if (!e.authenticated) throw new Error("the password is not right: give the user password with --password");
  return { encrypted: true, filter: e.filter, revision: e.r, permissions: e.permissions };
}
