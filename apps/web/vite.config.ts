import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  // The wasm glue is an ES module; Chromium runs module workers.
  worker: { format: "es" },
  // The macro prelude, template and bundled macros live in crates/pdfi-macro.
  server: { fs: { allow: ["../.."] } },
  build: { outDir: "dist", emptyOutDir: true, sourcemap: false, chunkSizeWarningLimit: 1024 },
});
