import { readdir, readFile, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { join } from "node:path";
import { gzipSync } from "node:zlib";

const dist = new URL("./dist/", import.meta.url);
const files = await readdir(dist, { recursive: true, withFileTypes: true });

for (const file of files) {
  if (!file.isFile() || file.name.endsWith(".gz") || file.name.endsWith(".etag")) continue;
  const path = join(file.parentPath, file.name);
  const content = await readFile(path);
  const compressed = gzipSync(content, { level: 9 });
  await writeFile(`${path}.gz`, compressed);
  if (file.name !== "index.html") {
    const hash = createHash("sha256").update(compressed).digest("hex");
    await writeFile(`${path}.etag`, `"${hash}"`);
  }
}
