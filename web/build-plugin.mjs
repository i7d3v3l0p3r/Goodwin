import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import solid from "@solidjs/vite-plugin";
import { build, parseAst } from "vite";

const entry = resolve(process.argv[2]);
const external = ["solid-js", "@solidjs/web"];
const runtime = "/plugin-runtime-v1.js";

const [result] = await build({
  configFile: false,
  root: fileURLToPath(new URL(".", import.meta.url)),
  plugins: [solid()],
  logLevel: "error",
  build: {
    write: false,
    lib: { entry, formats: ["es"], fileName: () => "index.mjs" },
    rolldownOptions: {
      external,
      output: { codeSplitting: false, paths: Object.fromEntries(external.map(name => [name, runtime])) },
    },
  },
});

const [chunk] = result.output;
if (result.output.length !== 1 || chunk?.type !== "chunk" || !chunk.exports.includes("default")) {
  throw new Error("The plugin web entry must export a default component and build to one module.");
}

const declarations = await readFile(new URL("./src/helpers/plugin-runtime.ts", import.meta.url), "utf8");
const available = new Set(parseAst(declarations).body.flatMap(node => node.type === "ExportNamedDeclaration"
  ? node.specifiers.map(specifier => specifier.exported.name)
  : []));
for (const statement of parseAst(chunk.code).body) {
  if (statement.type !== "ImportDeclaration" || statement.source.value !== runtime) continue;
  for (const binding of statement.specifiers) {
    if (binding.type !== "ImportSpecifier") {
      throw new Error("The Goodwin Solid runtime supports named imports.");
    }
    if (!available.has(binding.imported.name)) {
      throw new Error(`The Goodwin Solid runtime does not export ${binding.imported.name}.`);
    }
  }
}
process.stdout.write(chunk.code);
