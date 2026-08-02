import { readdir, readFile } from "node:fs/promises";
import { extname, relative, resolve } from "node:path";

const distRoot = resolve("dist");
const html = await readFile(resolve(distRoot, "index.html"), "utf8");
const tauriConfig = JSON.parse(await readFile(resolve("src-tauri/tauri.conf.json"), "utf8"));

if (!html.includes('id="app"')) {
  throw new Error("Frontend mount point is missing.");
}

const csp = tauriConfig.app?.security?.csp;
if (
  typeof csp !== "string" ||
  !csp.includes("connect-src 'none'") ||
  !csp.includes("img-src 'self' asset: data:") ||
  !csp.includes("media-src 'self' asset:")
) {
  throw new Error("Tauri CSP must allow data: only for on-demand image payloads and keep network disabled.");
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

process.stdout.write("frontend static smoke and CSP boundary: PASS\n");

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
