import test from "node:test";
import assert from "node:assert/strict";
import {mkdtemp, mkdir, copyFile, writeFile, chmod, rm, readFile} from "node:fs/promises";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {spawn} from "node:child_process";

async function fixture(t, program) {
  const root = await mkdtemp(join(tmpdir(), "roundtable-npm-"));
  t.after(() => rm(root, {recursive: true, force: true}));
  await copyFile(new URL("../npm/bin/roundtable.mjs", import.meta.url), join(root, "roundtable.mjs"));
  const bin = join(root, "node_modules", "@roundtable", `cli-${process.platform}-${process.arch}`, "bin");
  await mkdir(bin, {recursive: true});
  await writeFile(join(bin, "roundtable"), "#!/usr/bin/env node\n" + program);
  await chmod(join(bin, "roundtable"), 0o755);
  return join(root, "roundtable.mjs");
}

test("launcher forwards literal arguments and native exit status", async t => {
  const launcher = await fixture(t, "console.log(JSON.stringify(process.argv.slice(2)));process.exit(7);");
  const args = ["hello world", "$(do-not-evaluate)", "--server", "https://roundtable.bot"];
  const child = spawn(process.execPath, [launcher, ...args]); let out = "";
  child.stdout.on("data", data => out += data);
  const [code] = await new Promise(resolve => child.once("exit", (...args) => resolve(args)));
  assert.equal(code, 7); assert.deepEqual(JSON.parse(out), args);
});

test("launcher forwards termination to the native child", async t => {
  const launcher = await fixture(t, "process.on('SIGTERM',()=>process.exit(23));console.log('ready');setInterval(()=>{},1000);");
  const child = spawn(process.execPath, [launcher]);
  await new Promise(resolve => child.stdout.once("data", resolve)); child.kill("SIGTERM");
  const code = await new Promise(resolve => child.once("exit", resolve)); assert.equal(code, 23);
});

test("native package versions match the launcher exactly", async () => {
  const manifest = JSON.parse(await readFile(new URL("../npm/package.json", import.meta.url)));
  assert.equal(Object.keys(manifest.optionalDependencies).length, 4);
  for (const version of Object.values(manifest.optionalDependencies)) assert.equal(version, manifest.version);
});
