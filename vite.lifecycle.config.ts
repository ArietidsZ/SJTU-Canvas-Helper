import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const boundaries = fileURLToPath(new URL("./browser-lifecycle/boundaries.tsx", import.meta.url));

export default defineConfig({
  plugins: [react()],
  // Do not scan the production app/website with the fixture-only aliases.
  optimizeDeps: { entries: ["browser-lifecycle/index.html"] },
  server: { host: "127.0.0.1", port: 4179, strictPort: true, open: false },
  // Only these two production components are loaded. Native IPC/data and the
  // document viewer are explicit test boundaries; Blob and G6 are never aliased.
  resolve: {
    alias: [
      { find: "@tauri-apps/api/core", replacement: boundaries },
      { find: "@cyntler/react-doc-viewer", replacement: boundaries },
      { find: "../lib/hooks", replacement: boundaries },
      { find: "../lib/utils", replacement: boundaries },
      { find: "./renderers", replacement: boundaries },
    ],
  },
});
