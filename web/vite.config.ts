import solid from "@solidjs/vite-plugin";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [solid()],
  build: {
    assetsInlineLimit: 64_000,
    rolldownOptions: { output: { entryFileNames: "panel.js", assetFileNames: "panel[extname]" } },
  },
  server: { proxy: { "/api": "http://127.0.0.1:8787", "/plugin-assets": "http://127.0.0.1:8787" } },
});
