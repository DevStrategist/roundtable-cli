#!/usr/bin/env node
// Build publishable platform packages from CI-produced native binaries, never credentials.
import {readFile, mkdir, copyFile, writeFile, chmod} from "node:fs/promises";
import {resolve, dirname} from "node:path";
import {fileURLToPath} from "node:url";
const cli = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifest = JSON.parse(await readFile(resolve(cli, "npm/package.json")));
const cargo = await readFile(resolve(cli, "Cargo.toml"), "utf8");
if (!cargo.includes(`version = "${manifest.version}"`)) throw new Error("Cargo and npm versions must match");
const artifacts = resolve(process.argv[2] || resolve(cli, "artifacts"));
const output = resolve(process.argv[3] || resolve(cli, "dist"));
const targets = [
  ["darwin", "arm64", "aarch64-apple-darwin"], ["darwin", "x64", "x86_64-apple-darwin"],
  ["linux", "arm64", "aarch64-unknown-linux-gnu"], ["linux", "x64", "x86_64-unknown-linux-gnu"],
];
const selected = process.argv[4] ? targets.filter(([, , target]) => target === process.argv[4]) : targets;
if (!selected.length) throw new Error("Unknown native target");
for (const [os, cpu, target] of selected) {
  const dir = resolve(output, `${os}-${cpu}`);
  await mkdir(resolve(dir, "bin"), {recursive: true});
  await copyFile(resolve(artifacts, target, "roundtable"), resolve(dir, "bin/roundtable"));
  await chmod(resolve(dir, "bin/roundtable"), 0o755);
  await writeFile(resolve(dir, "package.json"), JSON.stringify({
    name: `@roundtable/cli-${os}-${cpu}`, version: manifest.version, description: `Roundtable native binary for ${os}/${cpu}`,
    license: manifest.license, repository: manifest.repository, os: [os], cpu: [cpu],
    ...(os === "linux" ? {libc: ["glibc"]} : {}), files: ["bin/roundtable"], publishConfig: {access: "public"},
  }, null, 2) + "\n");
  await copyFile(resolve(cli, "npm/README.md"), resolve(dir, "README.md"));
}
console.log(`Prepared ${selected.length} native packages at ${output}`);
