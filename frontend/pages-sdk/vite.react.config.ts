import path from "node:path";

import { defineConfig } from "vite";

// Builds this app's own React + ReactDOM/client to `dist/pages-sdk/react.mjs`
// — the module a served page's import map resolves both "react" and
// "react-dom/client" to (docs/spec/runtime/pages.md §5; see react-entry.ts
// for why one file answers both specifiers). Its own config, separate from
// `vite.config.ts`: that build marks React external so it does not end up
// bundled twice, and this one exists specifically to bundle it.
export default defineConfig({
  build: {
    outDir: path.resolve(__dirname, "../dist/pages-sdk"),
    emptyOutDir: false,
    // Library mode defaults `minify` to `false` — explicit here so React
    // itself doesn't ship to a sandboxed page as an unminified dev bundle.
    minify: "esbuild",
    lib: {
      entry: path.resolve(__dirname, "react-entry.ts"),
      formats: ["es"],
      fileName: () => "react.mjs",
    },
  },
});
