#!/usr/bin/env node
// The public Actions log must never contain capture output, including failures.
import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { appendFile, mkdir, mkdtemp, open, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

// Git canonical LF bytes of capture-linear.mjs at 4bc7fb40c2ae1999efa4b456982cbe7708b4d384.
export const CAPTURE_SHA256 = "66fcd3a802f71c366302dcc089768ac3cfbb79bd4fe35f60cef290ee24021569";
export function captureDigest(bytes) {
  return createHash("sha256").update(bytes.toString("utf8").replaceAll("\r\n", "\n")).digest("hex");
}

export async function encryptedCapture({ root, recipient, age, script, scriptHash, key, outputFile, githubOutput, timeout = 45 * 60 * 1000 }) {
  if (!isAbsolute(root) || !isAbsolute(age) || !/^age1[023456789acdefghjklmnpqrstuvwxyz]{58}$/.test(recipient)) throw new Error("invalid configuration");
  if (!key || !/^[0-9a-f]{64}$/.test(scriptHash)) throw new Error("missing configuration");
  if (captureDigest(await readFile(script)) !== scriptHash) throw new Error("untrusted capture script");
  const outputRelative = relative(root, outputFile);
  if (!outputRelative || isAbsolute(outputRelative) || outputRelative === ".." || outputRelative.startsWith(`..${sep}`)) throw new Error("output must be inside temporary root");
  await mkdir(root, { recursive: true, mode: 0o700 });
  const scratch = await mkdtemp(join(root, "direct-private-"));
  let log;
  let committed = false;
  let ownedOutput = false;
  try {
    const payload = join(scratch, "payload");
    await mkdir(payload, { mode: 0o700 });
    log = await open(join(payload, "capture.log"), "wx", 0o600);
    const cleanEnvironment = { ...process.env };
    delete cleanEnvironment.LINEAR_API_KEY;
    delete cleanEnvironment.LINEAR_MIGRATION_API_KEY;
    const result = await run(process.execPath, [script, "--out", join(payload, "source")], {
      env: { ...cleanEnvironment, LINEAR_API_KEY: key }, log: log.fd, timeout,
    });
    key = undefined;
    await writeFile(join(payload, "capture-result.json"), JSON.stringify({
      format: 1, capture_exit_code: result.code, capture_signal: result.signal,
      captured_at: new Date().toISOString(), source_script_sha256: scriptHash,
      workflow_commit: process.env.GITHUB_SHA ?? null,
      workflow_run: process.env.GITHUB_RUN_ID ?? null,
      // Capture success is not a completeness, freshness or migration verdict.
      requires_local_reconciliation: true,
    }, null, 2), { mode: 0o600, flag: "wx" });
    const archive = join(scratch, "payload.tar.gz");
    const packed = await run("tar", ["-czf", archive, "-C", payload, "."], { env: cleanEnvironment, log: log.fd, timeout: 120000 });
    if (packed.code !== 0) throw new Error("archive failed");
    await mkdir(dirname(outputFile), { recursive: true, mode: 0o700 });
    // Reserve a new file: never overwrite an earlier capture on retry.
    const encrypted = await open(outputFile, "wx", 0o600);
    ownedOutput = true;
    let sealed;
    try {
      sealed = await run(age, ["--encrypt", "--recipient", recipient, archive], {
        env: cleanEnvironment, log: log.fd, stdout: encrypted.fd, timeout: 120000,
      });
    } finally { await encrypted.close(); }
    if (sealed.code !== 0) throw new Error("encryption failed");
    committed = true;
    if (githubOutput) await appendFile(githubOutput, "encrypted=true\n");
    return { encrypted: true, code: result.code === 0 ? 0 : 2 };
  } finally {
    if (log) await log.close();
    if (ownedOutput && !committed) await rm(outputFile, { force: true }).catch(() => {});
    // Only the directory freshly created by mkdtemp is removed.
    await rm(scratch, { recursive: true, force: true });
  }
}

function run(command, args, { env, log, stdout = log, timeout }) {
  return new Promise((done) => {
    const child = spawn(command, args, { env, stdio: ["ignore", stdout, log], windowsHide: true });
    const timer = setTimeout(() => child.kill("SIGKILL"), timeout);
    timer.unref();
    const finish = (result) => { clearTimeout(timer); done(result); };
    child.once("error", () => finish({ code: -1, signal: "spawn_error" }));
    child.once("close", (code, signal) => finish({ code, signal }));
  });
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const key = process.env.LINEAR_API_KEY;
  delete process.env.LINEAR_API_KEY;
  try {
    const root = resolve(process.env.RUNNER_TEMP);
    const result = await encryptedCapture({
      root, recipient: process.env.CAPTURE_RECIPIENT, age: process.env.AGE_BIN,
      script: resolve("scripts/capture-linear.mjs"), scriptHash: CAPTURE_SHA256, key,
      outputFile: join(root, "direct-encrypted", "capture.age"), githubOutput: process.env.GITHUB_OUTPUT,
    });
    console.log("Encrypted capture prepared; local verification is required.");
    process.exitCode = result.code;
  } catch {
    // Do not print exception text, request errors, paths or a stack trace.
    console.error("Encrypted capture failed. No plaintext diagnostics are published.");
    process.exitCode = 1;
  }
}
