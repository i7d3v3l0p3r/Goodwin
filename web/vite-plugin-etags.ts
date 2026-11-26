import { createHash } from "node:crypto";
import { glob, readFile, writeFile } from "node:fs/promises";
import { basename, resolve } from "node:path";
import type { Plugin } from "vite";

export function gzipEtags(): Plugin {
  return {
    name: "goodwin-gzip-etags",
    apply: "build",
    closeBundle: {
      order: "post",
      async handler() {
        const { root, build } = this.environment.config;
        const outDir = resolve(root, build.outDir);
        for await (const file of glob("**/*.gz", { cwd: outDir })) {
          if (basename(file) === "index.html.gz") continue;
          const path = resolve(outDir, file);
          const hash = createHash("sha256").update(await readFile(path)).digest("hex");
          await writeFile(path.replace(/\.gz$/, ".etag"), `"${hash}"`);
        }
      },
    },
  };
}
