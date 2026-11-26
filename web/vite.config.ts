import solid from "@solidjs/vite-plugin";
import { defineConfig } from "vite";
import { compression, defineAlgorithm } from "vite-plugin-compression2";
import { gzipEtags } from "./vite-plugin-etags.ts";

const runtime = "src/helpers/plugin-runtime.ts";
const runtimeName = "plugin-runtime-v1";

export default defineConfig({
  plugins: [
    solid(),
    compression({
      algorithms: [defineAlgorithm("gzip", { level: 9 })],
      include: /./,
      exclude: /\.(gz|etag)$/,
      skipIfLargerOrEqual: false,
    }),
    gzipEtags(),
    {
      name: "goodwin-plugin-runtime",
      apply: "serve",
      resolveId(id, importer, options) {
        if (id === `/${runtimeName}.js`) return this.resolve(`/${runtime}`, importer, options);
      },
    },
  ],
  build: {
    assetsInlineLimit: 64_000,
    reportCompressedSize: false,
    license: { fileName: "panel-licenses.md" },
    rolldownOptions: {
      input: { panel: "index.html", [runtimeName]: runtime },
      preserveEntrySignatures: "allow-extension",
      output: {
        entryFileNames: "[name].js",
        assetFileNames: "panel[extname]",
        codeSplitting: {
          // Keep Solid and the plugin exports in the same embedded module.
          groups: [{
            name: "solid",
            test: id => id.endsWith(`/${runtime}`) || /\/node_modules\/(?:solid-js|@solidjs\/(?:signals|web))\//.test(id),
          }],
        },
      },
    },
  },
  server: {
    proxy: {
      "/api": { target: "http://127.0.0.1:8787", changeOrigin: false },
      "/plugin-assets": "http://127.0.0.1:8787",
    },
  },
});
