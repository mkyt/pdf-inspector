// Write the operator documentation of the web UI (OperatorDoc.tsx) as JSON
// for the macOS app, so both explain operators alike:
//
//   node apps/macos/scripts/operator-docs.mjs > apps/macos/PDFInspector/Resources/OperatorDocs.json
import { readFileSync } from "node:fs";

const src = readFileSync(new URL("../../web/src/components/OperatorDoc.tsx", import.meta.url), "utf8");
const start = src.indexOf("const xy");
const end = src.indexOf("/** Operand names");
if (start < 0 || end < 0) throw new Error("OperatorDoc.tsx changed shape");
// The table is plain data and small helpers; drop the type annotations.
const js = src
  .slice(start, end)
  .replace(/export const operatorDocs: Record<string, OpDoc> =/, "const operatorDocs =")
  .replace(/\b(\w+)\s*:\s*string\b/g, "$1")
  .replace(/\)\s*:\s*\w+(\[\])?\s*=>/g, ") =>")
  .replace(/const (\w+)\s*:\s*\w+\s*=/g, "const $1 =");
const docs = new Function(`${js}\nreturn operatorDocs;`)();
process.stdout.write(JSON.stringify(docs, null, 1) + "\n");
