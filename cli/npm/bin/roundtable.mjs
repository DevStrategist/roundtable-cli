#!/usr/bin/env node
import {createRequire} from "node:module";
import {spawn} from "node:child_process";
import {constants} from "node:os";
import {existsSync} from "node:fs";
import {fileURLToPath} from "node:url";

const require = createRequire(import.meta.url);
const platform = process.platform, arch = process.arch;
const name = `@roundtable/cli-${platform}-${arch}`;
if (!["darwin", "linux"].includes(platform) || !["arm64", "x64"].includes(arch)) {
  console.error(`Roundtable does not support ${platform}/${arch}. Supported: macOS and Linux on arm64 or x64.`);
  process.exit(1);
}
if (platform === "linux") {
  const glibc = process.report?.getReport().header.glibcVersionRuntime;
  const minimum = arch === "arm64" ? 39 : 35;
  const [major, minor] = (glibc || "0.0").split(".").map(Number);
  if (major < 2 || (major === 2 && minor < minimum)) {
    console.error(`This Roundtable build requires glibc 2.${minimum}+ (${arch}). Build from cli/ on other Linux distributions; musl/Alpine binaries are not included.`);
    process.exit(1);
  }
}
let binary;
try { binary = require.resolve(`${name}/bin/roundtable`); }
catch {
  binary = fileURLToPath(new URL("./native/roundtable", import.meta.url));
  if (!existsSync(binary)) {
    console.error("Missing native binary. Reinstall using the npm command at https://roundtable.bot/docs/cli, or download a native GitHub Release.");
    process.exit(1);
  }
}
const child = spawn(binary, process.argv.slice(2), {stdio: "inherit"});
const handlers = new Map();
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  const forward = () => child.kill(signal);
  handlers.set(signal, forward); process.on(signal, forward);
}
child.once("error", error => { console.error(`Could not start Roundtable: ${error.message}`); process.exitCode = 1; });
child.once("exit", (code, signal) => {
  for (const [name, handler] of handlers) process.removeListener(name, handler);
  if (signal) { process.exitCode = 128 + (constants.signals[signal] || 0); process.kill(process.pid, signal); }
  else process.exitCode = code ?? 1;
});
