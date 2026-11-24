import { readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { gzipSync } from "node:zlib";

const dist = new URL("./dist/", import.meta.url);
const files = await readdir(dist, { recursive: true, withFileTypes: true });

for (const file of files) {
  if (!file.isFile() || file.name.endsWith(".gz")) continue;
  const path = join(file.parentPath, file.name);
  const content = await readFile(path);
  await writeFile(`${path}.gz`, gzipSync(content, { level: 9 }));
}
