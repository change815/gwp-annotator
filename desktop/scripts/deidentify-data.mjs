#!/usr/bin/env node

import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, readdir, rename, rm, stat, statfs, writeFile, copyFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

const IMAGE_EXTENSIONS = new Set([".jpg"]);
const VIDEO_EXTENSIONS = new Set([".mp4"]);

function usage() {
  return `用法：
  node scripts/deidentify-data.mjs --source <原数据目录> --target <脱敏目录> [--mapping <映射文件>] [--dry-run]
  node scripts/deidentify-data.mjs --self-test

默认映射文件：<脱敏目录>_映射.json
只复制 JPG 和 MP4；原目录始终不修改。`;
}

function parseArgs(argv) {
  const result = { dryRun: false, selfTest: false };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--dry-run") result.dryRun = true;
    else if (arg === "--self-test") result.selfTest = true;
    else if (arg === "--help" || arg === "-h") result.help = true;
    else if (arg === "--source" || arg === "--target" || arg === "--mapping") {
      const value = argv[index + 1];
      if (!value) throw new Error(`${arg} 缺少路径。`);
      result[arg.slice(2)] = value;
      index += 1;
    } else throw new Error(`未知参数：${arg}`);
  }
  return result;
}

function portable(relativePath) {
  return relativePath.split(path.sep).join("/");
}

function inside(parent, child) {
  const relative = path.relative(parent, child);
  return relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== "..");
}

function nextId(entries, key, prefix, width = 6) {
  let maximum = 0;
  for (const entry of entries) {
    const match = new RegExp(`^${prefix}-(\\d+)$`).exec(entry[key]);
    if (match) maximum = Math.max(maximum, Number(match[1]));
  }
  return `${prefix}-${String(maximum + 1).padStart(width, "0")}`;
}

async function directories(directory) {
  return (await readdir(directory, { withFileTypes: true }))
    .filter((entry) => entry.isDirectory() && !entry.name.startsWith("."))
    .sort((left, right) => left.name.localeCompare(right.name, "zh-CN", { numeric: true }));
}

async function mediaFiles(directory, relative = "") {
  const result = [];
  const entries = (await readdir(directory, { withFileTypes: true }))
    .sort((left, right) => left.name.localeCompare(right.name, "zh-CN", { numeric: true }));

  for (const entry of entries) {
    const relativePath = path.join(relative, entry.name);
    const absolutePath = path.join(directory, entry.name);
    if (entry.isSymbolicLink()) throw new Error("源数据中存在符号链接，已停止，未复制任何链接目标。");
    if (entry.isDirectory()) result.push(...await mediaFiles(absolutePath, relativePath));
    else if (entry.isFile()) {
      const extension = path.extname(entry.name).toLowerCase();
      if (IMAGE_EXTENSIONS.has(extension) || VIDEO_EXTENSIONS.has(extension)) {
        const details = await stat(absolutePath);
        result.push({ relativePath, extension, sizeBytes: details.size });
      }
    }
  }
  return result;
}

async function readMapping(mappingPath) {
  try {
    const parsed = JSON.parse(await readFile(mappingPath, "utf8"));
    if (!Array.isArray(parsed.sites) || !Array.isArray(parsed.cases) || !Array.isArray(parsed.assets)) {
      throw new Error("映射文件格式不正确。");
    }
    return parsed;
  } catch (error) {
    if (error?.code === "ENOENT") {
      return {
        warning: "包含患者原始相对路径，不得随脱敏数据分发。",
        sites: [],
        cases: [],
        assets: [],
      };
    }
    throw error;
  }
}

async function buildPlan(sourceRoot, targetRoot, mapping) {
  const sitesBySource = new Map(mapping.sites.map((entry) => [entry.sourceName, entry]));
  const casesBySource = new Map(mapping.cases.map((entry) => [entry.sourceRelative, entry]));
  const assetsBySource = new Map(mapping.assets.map((entry) => [entry.sourceRelative, entry]));
  const plan = [];

  for (const siteDirectory of await directories(sourceRoot)) {
    let site = sitesBySource.get(siteDirectory.name);
    if (!site) {
      site = { siteId: nextId(mapping.sites, "siteId", "SITE"), sourceName: siteDirectory.name };
      mapping.sites.push(site);
      sitesBySource.set(siteDirectory.name, site);
    }

    const sitePath = path.join(sourceRoot, siteDirectory.name);
    for (const caseDirectory of await directories(sitePath)) {
      const caseSource = portable(path.join(siteDirectory.name, caseDirectory.name));
      let caseEntry = casesBySource.get(caseSource);
      if (!caseEntry) {
        caseEntry = { caseId: nextId(mapping.cases, "caseId", "CASE"), siteId: site.siteId, sourceRelative: caseSource };
        mapping.cases.push(caseEntry);
        casesBySource.set(caseSource, caseEntry);
      }

      const casePath = path.join(sitePath, caseDirectory.name);
      let files;
      try {
        files = await mediaFiles(casePath);
      } catch (error) {
        if (error.message.startsWith("源数据中存在符号链接")) throw error;
        throw new Error(`读取病例 ${caseEntry.caseId} 失败（${error.code ?? "UNKNOWN"}）。`);
      }
      for (const file of files) {
        const sourceRelative = portable(path.join(caseSource, file.relativePath));
        let asset = assetsBySource.get(sourceRelative);
        if (!asset) {
          const kind = IMAGE_EXTENSIONS.has(file.extension) ? "IMG" : "VID";
          const sameCase = mapping.assets.filter((entry) => entry.caseId === caseEntry.caseId && entry.assetId.startsWith(`${kind}-`));
          const assetId = nextId(sameCase, "assetId", kind, 3);
          asset = {
            assetId,
            caseId: caseEntry.caseId,
            sourceRelative,
            targetRelative: portable(path.join(site.siteId, caseEntry.caseId, `${assetId}${file.extension}`)),
            sizeBytes: file.sizeBytes,
          };
          mapping.assets.push(asset);
          assetsBySource.set(sourceRelative, asset);
        }
        const target = path.resolve(targetRoot, ...asset.targetRelative.split("/"));
        if (!inside(targetRoot, target)) throw new Error("映射文件包含越过脱敏目录的目标路径。");
        plan.push({
          source: path.join(sourceRoot, ...sourceRelative.split("/")),
          target,
          targetRelative: asset.targetRelative,
          sizeBytes: file.sizeBytes,
        });
      }
    }
  }

  mapping.sites.sort((left, right) => left.siteId.localeCompare(right.siteId));
  mapping.cases.sort((left, right) => left.caseId.localeCompare(right.caseId));
  mapping.assets.sort((left, right) => left.targetRelative.localeCompare(right.targetRelative));
  return plan;
}

async function pendingFiles(plan) {
  const pending = [];
  let skippedBytes = 0;
  for (const item of plan) {
    try {
      const existing = await stat(item.target);
      if (!existing.isFile() || existing.size !== item.sizeBytes) {
        throw new Error(`已有脱敏文件大小不一致：${item.targetRelative}`);
      }
      skippedBytes += item.sizeBytes;
    } catch (error) {
      if (error?.code === "ENOENT") pending.push(item);
      else throw error;
    }
  }
  return { pending, skippedBytes };
}

async function writeMapping(mappingPath, mapping) {
  const temporary = `${mappingPath}.partial`;
  await writeFile(temporary, `${JSON.stringify(mapping, null, 2)}\n`, "utf8");
  await rename(temporary, mappingPath);
}

function formatBytes(bytes) {
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

async function run({ source, target, mapping: requestedMapping, dryRun = false }) {
  if (!source || !target) throw new Error("必须同时提供 --source 和 --target。\n\n" + usage());
  const sourceRoot = path.resolve(source);
  const targetRoot = path.resolve(target);
  const mappingPath = path.resolve(requestedMapping ?? `${targetRoot}_映射.json`);
  const sourceDetails = await stat(sourceRoot);
  if (!sourceDetails.isDirectory()) throw new Error("源路径不是目录。");
  if (inside(sourceRoot, targetRoot) || inside(targetRoot, sourceRoot)) throw new Error("原数据目录与脱敏目录不能相同或互相包含。");
  if (inside(sourceRoot, mappingPath)) throw new Error("映射文件不能写入原数据目录。");
  if (inside(targetRoot, mappingPath)) throw new Error("映射文件必须位于脱敏目录之外。");

  const mapping = await readMapping(mappingPath);
  const plan = await buildPlan(sourceRoot, targetRoot, mapping);
  const { pending, skippedBytes } = await pendingFiles(plan);
  const pendingBytes = pending.reduce((sum, item) => sum + item.sizeBytes, 0);
  const space = await statfs(path.dirname(targetRoot), { bigint: true });
  const freeBytes = Number(space.bavail * space.bsize);
  if (pendingBytes > freeBytes) throw new Error(`空间不足：需要 ${formatBytes(pendingBytes)}，可用 ${formatBytes(freeBytes)}。`);

  console.log(`院区 ${mapping.sites.length} 个，病例 ${mapping.cases.length} 个，媒体 ${plan.length} 个。`);
  console.log(`待复制 ${pending.length} 个（${formatBytes(pendingBytes)}），已存在 ${plan.length - pending.length} 个（${formatBytes(skippedBytes)}）。`);
  console.log(`目标剩余空间 ${formatBytes(freeBytes)}。`);
  if (dryRun) {
    console.log("DRY-RUN 完成：未创建目录、映射或媒体副本。");
    return { mapping, plan, pending };
  }

  await writeMapping(mappingPath, mapping);
  let copiedBytes = 0;
  for (let index = 0; index < pending.length; index += 1) {
    const item = pending[index];
    await mkdir(path.dirname(item.target), { recursive: true });
    const temporary = `${item.target}.partial`;
    try {
      await copyFile(item.source, temporary);
      await rename(temporary, item.target);
    } catch (error) {
      throw new Error(`复制失败（脱敏目标 ${item.targetRelative}，错误代码 ${error.code ?? "UNKNOWN"}）。`);
    }
    copiedBytes += item.sizeBytes;
    if ((index + 1) % 50 === 0 || index + 1 === pending.length) {
      console.log(`已复制 ${index + 1}/${pending.length}（${formatBytes(copiedBytes)}）。`);
    }
  }
  console.log("脱敏副本完成；原数据未修改。映射文件不得随脱敏目录分发。");
  return { mapping, plan, pending };
}

async function selfTest() {
  const temporaryRoot = await mkdtemp(path.join(os.tmpdir(), "gwp-deidentify-"));
  try {
    const source = path.join(temporaryRoot, "source");
    const target = path.join(temporaryRoot, "target");
    const mapping = path.join(temporaryRoot, "mapping.json");
    await mkdir(path.join(source, "SITE-ALPHA", "CASE-ALPHA", "nested"), { recursive: true });
    await mkdir(path.join(source, "SITE-BETA", "CASE-BETA"), { recursive: true });
    await writeFile(path.join(source, "SITE-ALPHA", "CASE-ALPHA", "photo.jpg"), "image-a");
    await writeFile(path.join(source, "SITE-ALPHA", "CASE-ALPHA", "nested", "clip.mp4"), "video-a");
    await writeFile(path.join(source, "SITE-BETA", "CASE-BETA", "photo.jpg"), "image-b");

    const first = await run({ source, target, mapping });
    assert.equal(first.mapping.sites.length, 2);
    assert.equal(first.mapping.cases.length, 2);
    assert.equal(first.mapping.assets.length, 3);
    assert.equal(await readFile(path.join(target, "SITE-000001", "CASE-000001", "IMG-001.jpg"), "utf8"), "image-a");
    assert.equal(await readFile(path.join(target, "SITE-000001", "CASE-000001", "VID-001.mp4"), "utf8"), "video-a");
    assert.equal(await readFile(path.join(source, "SITE-ALPHA", "CASE-ALPHA", "photo.jpg"), "utf8"), "image-a");

    const second = await run({ source, target, mapping });
    assert.equal(second.pending.length, 0);

    await writeFile(path.join(source, "SITE-ALPHA", "CASE-ALPHA", "photo-2.jpg"), "image-a2");
    const third = await run({ source, target, mapping });
    assert.equal(third.pending.length, 1);
    assert.equal(third.mapping.cases.find((entry) => entry.sourceRelative === "SITE-ALPHA/CASE-ALPHA")?.caseId, "CASE-000001");
    assert.equal(await readFile(path.join(target, "SITE-000001", "CASE-000001", "IMG-002.jpg"), "utf8"), "image-a2");

    const damagedMapping = structuredClone(third.mapping);
    damagedMapping.assets[0].targetRelative = "../escape.jpg";
    const damagedMappingPath = path.join(temporaryRoot, "damaged-mapping.json");
    await writeFile(damagedMappingPath, JSON.stringify(damagedMapping));
    await assert.rejects(
      run({ source, target: path.join(temporaryRoot, "other-target"), mapping: damagedMappingPath, dryRun: true }),
      /越过脱敏目录/,
    );
    console.log("SELF-TEST PASS");
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
}

try {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) console.log(usage());
  else if (options.selfTest) await selfTest();
  else await run(options);
} catch (error) {
  console.error(`ERROR: ${error.message}`);
  process.exitCode = 1;
}
