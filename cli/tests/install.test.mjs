import test from "node:test";
import assert from "node:assert/strict";
import {createServer} from "node:http";
import {createHash} from "node:crypto";
import {gzipSync} from "node:zlib";
import {mkdtemp, readFile, stat, rm} from "node:fs/promises";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {install} from "../npm/scripts/install.mjs";

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
