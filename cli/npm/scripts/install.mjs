#!/usr/bin/env node
import {createHash, randomUUID} from "node:crypto";
import {mkdir, writeFile, rename, rm} from "node:fs/promises";
import {dirname} from "node:path";
import {fileURLToPath} from "node:url";
import {gunzipSync} from "node:zlib";

const targets = {"darwin-arm64":"aarch64-apple-darwin", "darwin-x64":"x86_64-apple-darwin",
  "linux-arm64":"aarch64-unknown-linux-gnu", "linux-x64":"x86_64-unknown-linux-gnu"};
async function download(url, limit) {
  const response = await fetch(url, {signal: AbortSignal.timeout(120000)});
  if (!response.ok) throw new Error(`Download failed (${response.status}): ${url}`);
  let size = 0; const chunks = [];
  for await (const chunk of response.body) {
    size += chunk.length;
    if (size > limit) throw new Error("Release asset exceeds the size limit");
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}
export async function install({version, baseUrl, destination, platform = process.platform, arch = process.arch}) {
  const target = targets[`${platform}-${arch}`];
  if (!target) throw new Error(`Unsupported platform ${platform}/${arch}`);
  const base = baseUrl || `https://github.com/DevStrategist/roundtable-cli/releases/download/v${version}/`;
  const asset = `roundtable-${target}.gz`;
  const sums = (await download(new URL("SHA256SUMS", base), 16384)).toString("utf8");
  const expected = sums.split(/\r?\n/).map(line => line.trim().split(/\s+/)).find(parts => parts[1] === asset)?.[0];
  if (!expected || !/^[a-f0-9]{64}$/.test(expected)) throw new Error("Release checksum is missing");
  const compressed = await download(new URL(asset, base), 32 * 1024 * 1024);
  if (createHash("sha256").update(compressed).digest("hex") !== expected) throw new Error("Release checksum mismatch");
  const binary = gunzipSync(compressed, {maxOutputLength: 64 * 1024 * 1024});
  await mkdir(dirname(destination), {recursive: true});
  const staged = `${destination}.${randomUUID()}.tmp`;
  try {
    await writeFile(staged, binary, {mode: 0o755, flag: "wx"});
    await rename(staged, destination);
  } finally { await rm(staged, {force: true}); }
}
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const {readFile} = await import("node:fs/promises");
  const manifest = JSON.parse(await readFile(new URL("../package.json", import.meta.url)));
  install({version: manifest.version, destination: fileURLToPath(new URL("../bin/native/roundtable", import.meta.url))})
    .catch(error => { console.error(`Roundtable installation failed: ${error.message}`); process.exitCode = 1; });
}
