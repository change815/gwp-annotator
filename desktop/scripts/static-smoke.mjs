import { readdir, readFile } from "node:fs/promises";
import { extname, relative, resolve } from "node:path";

const distRoot = resolve("dist");
const html = await readFile(resolve(distRoot, "index.html"), "utf8");

if (!html.includes('id="app"')) {
  throw new Error("Frontend mount point is missing.");
}

const textExtensions = new Set([
  ".css",
  ".html",
  ".js",
  ".json",
  ".map",
  ".mjs",
  ".svg",
  ".txt",
  ".webmanifest",
  ".xml",
]);

for (const file of await listFiles(distRoot)) {
  if (!textExtensions.has(extname(file).toLowerCase())) {
    continue;
  }
  const contents = await readFile(file, "utf8");
  if (/https?:\/\//i.test(contents)) {
    throw new Error(`Built frontend contains an external network URL: ${relative(distRoot, file)}`);
  }
}

process.stdout.write("frontend static smoke (all text dist files): PASS\n");

async function listFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await listFiles(path)));
    } else if (entry.isFile()) {
      files.push(path);
    }
  }
  return files;
}
