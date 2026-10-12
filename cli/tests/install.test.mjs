import test from "node:test";
import assert from "node:assert/strict";
import {createServer} from "node:http";
import {createHash} from "node:crypto";
import {gzipSync} from "node:zlib";
import {mkdtemp, readFile, stat, rm} from "node:fs/promises";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {install} from "../npm/scripts/install.mjs";

test("the default installer downloads checksum-verified assets from the organization release", async t => {
  const binary = Buffer.from("organization release fixture"), asset = gzipSync(binary);
  const hash = createHash("sha256").update(asset).digest("hex");
  const base = "https://github.com/roundtable-bot/roundtable-cli/releases/download/v0.2.8/";
  const responses = new Map([
    [base + "SHA256SUMS", `${hash}  roundtable-aarch64-apple-darwin.gz\n`],
    [base + "roundtable-aarch64-apple-darwin.gz", asset],
  ]);
  t.mock.method(globalThis, "fetch", async url => {
    const body = responses.get(String(url));
    return new Response(body ?? "Not found", {status: body === undefined ? 404 : 200});
  });
  const root = await mkdtemp(join(tmpdir(), "roundtable-org-install-"));
  t.after(() => rm(root, {recursive:true, force:true}));
  const destination = join(root, "roundtable");
  await install({version:"0.2.8", platform:"darwin", arch:"arm64", destination});
  assert.deepEqual(await readFile(destination), binary);
});

test("GitHub installer verifies the checksum and writes an executable atomically", async t => {
  const binary = Buffer.from("native fixture"), asset = gzipSync(binary);
  let hash = createHash("sha256").update(asset).digest("hex");
  const server = createServer((req,res) => res.end(req.url === "/SHA256SUMS" ? `${hash}  roundtable-x86_64-unknown-linux-gnu.gz\n` : asset));
  await new Promise(resolve => server.listen(0,"127.0.0.1",resolve));
  t.after(() => new Promise(resolve => server.close(resolve)));
  const root = await mkdtemp(join(tmpdir(),"roundtable-install-"));
  t.after(() => rm(root,{recursive:true,force:true}));
  const opts = {platform:"linux",arch:"x64",baseUrl:`http://127.0.0.1:${server.address().port}/`,destination:join(root,"bin/roundtable")};
  await install(opts);
  assert.deepEqual(await readFile(opts.destination),binary);
  assert.equal((await stat(opts.destination)).mode & 0o777,0o755);
  hash="0".repeat(64);
  await assert.rejects(install(opts),/checksum mismatch/);
  assert.deepEqual(await readFile(opts.destination),binary);
});
