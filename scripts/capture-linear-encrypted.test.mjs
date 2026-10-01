import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { encryptedCapture, captureDigest, CAPTURE_SHA256 } from "./capture-linear-encrypted.mjs";

const age = process.env.AGE_BIN;
const keygen = process.env.AGE_KEYGEN_BIN;
assert.ok(age && keygen, "Pinned age and age-keygen paths are required");

test("trusted capture file has the reviewed byte identity", async () => {
  const hash = captureDigest(await readFile(new URL("./capture-linear.mjs", import.meta.url)));
  assert.equal(hash, CAPTURE_SHA256);
});

for (const exitCode of [0, 2]) {
  test(`real encryption/decryption preserves capture and private diagnostics (exit ${exitCode})`, async () => {
    const root = await mkdtemp(join(tmpdir(), "direct-encryption-test-"));
    try {
      const identity = join(root, "identity");
      assert.equal(spawnSync(keygen, ["-o", identity], { stdio: "ignore" }).status, 0);
      const recipient = spawnSync(keygen, ["-y", identity], { encoding: "utf8" }).stdout.trim();
      const fixture = join(root, "fixture.mjs");
      const fixtureSource = `import {mkdirSync,writeFileSync} from 'node:fs';
        const out=process.argv[process.argv.indexOf('--out')+1];mkdirSync(out);
        writeFileSync(out+'/private-name.txt','private-source-sentinel');
        console.log('private-filename-sentinel');console.error(process.env.LINEAR_API_KEY);
        process.exitCode=${exitCode};`;
      await writeFile(fixture, fixtureSource);
      const outputFile = join(root, "output", "capture.age");
      const githubOutput = join(root, "github-output");
      const config = { root, recipient, age, script: fixture, scriptHash: createHash("sha256").update(fixtureSource).digest("hex"), key: "synthetic-key-sentinel", outputFile, githubOutput };
      const result = await encryptedCapture(config);
      assert.equal(result.code, exitCode);
      assert.equal(await readFile(githubOutput, "utf8"), "encrypted=true\n");
      const ciphertext = await readFile(outputFile);
      for (const marker of ["private-source-sentinel", "private-filename-sentinel", "synthetic-key-sentinel"]) assert.equal(ciphertext.includes(Buffer.from(marker)), false);
      assert.equal((await readdir(root)).some((name) => name.startsWith("direct-private-")), false);
      const plain = join(root, "decoded.tar.gz");
      assert.equal(spawnSync(age, ["-d", "-i", identity, "-o", plain, outputFile], { stdio: "ignore" }).status, 0);
      const extracted = join(root, "extracted"); await mkdir(extracted);
      assert.equal(spawnSync("tar", ["-xzf", plain, "-C", extracted], { stdio: "ignore" }).status, 0);
      assert.equal(await readFile(join(extracted, "source", "private-name.txt"), "utf8"), "private-source-sentinel");
      assert.match(await readFile(join(extracted, "capture.log"), "utf8"), /synthetic-key-sentinel/);
      assert.equal(JSON.parse(await readFile(join(extracted, "capture-result.json"))).capture_exit_code, exitCode);
      const tampered = Buffer.from(ciphertext); tampered[tampered.length - 1] ^= 1;
      await writeFile(join(root, "tampered.age"), tampered);
      assert.notEqual(spawnSync(age, ["-d", "-i", identity, join(root, "tampered.age")], { stdio: "ignore" }).status, 0);
      const before = await readFile(outputFile);
      await assert.rejects(encryptedCapture(config));
      assert.deepEqual(await readFile(outputFile), before, "retry must preserve the previous encrypted export");
      await assert.rejects(encryptedCapture({ ...config, outputFile: join(root, "bad.age"), scriptHash: "0".repeat(64) }));
      await assert.rejects(encryptedCapture({ ...config, outputFile: resolve(root, "..", "outside.age") }));
      await assert.rejects(encryptedCapture({ ...config, recipient: "--identity malicious" }));
      const failedOutput = join(root, "encryption-failure", "capture.age");
      await assert.rejects(encryptedCapture({ ...config, outputFile: failedOutput, age: join(root, "absent-age") }));
      await assert.rejects(readFile(failedOutput));
      assert.equal((await readdir(root)).some((name) => name.startsWith("direct-private-")), false);
    } finally { await rm(root, { recursive: true, force: true }); }
  });
}

test("public CLI failure is generic and does not echo environment or error details", () => {
  const result = spawnSync(process.execPath, ["scripts/capture-linear-encrypted.mjs"], {
    encoding: "utf8", env: { ...process.env, LINEAR_API_KEY: "synthetic-key-sentinel", RUNNER_TEMP: tmpdir(), CAPTURE_RECIPIENT: "invalid-private-sentinel", AGE_BIN: age },
  });
  assert.equal(result.status, 1);
  assert.equal(result.stdout, "");
  assert.equal(result.stderr.trim(), "Encrypted capture failed. No plaintext diagnostics are published.");
});
