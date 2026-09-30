#!/usr/bin/env node
// Browser E2E for DIR-52: owner applies a prepared Linear migration in the real
// UI, reads retained document text, downloads retained file bytes, follows
// issue/source links, and confirms the result survives reload and a service
// restart. Synthetic data only.
//
// Setup (from the repository root):
//   cargo build -p direct && (cd app && npm run build)
//   DIRECT_BROWSER_FIXTURE=<dir> cargo test -p direct --test linear_cutover browser_fixture -- --ignored
// Run (Playwright must be resolvable; PLAYWRIGHT_MODULE may point at it):
//   node scripts/e2e/linear-sources.browser.mjs target/debug/direct <dir> app/dist <evidence-dir>

import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || "playwright");
const [binary, fixture, assets, evidence] = process.argv.slice(2).map((value) => resolve(value));
if (!evidence) {
  console.error("usage: linear-sources.browser.mjs <direct-binary> <fixture-dir> <assets-dir> <evidence-dir>");
  process.exit(2);
}
mkdirSync(evidence, { recursive: true });
const workspace = join(fixture, "workspace");
const artifact = join(fixture, "prepared", "linear-migration.direct-migration");
const uploadDir = join(fixture, "capture", "attachments");
const upload = readdirSync(uploadDir)[0];
const uploadSha = createHash("sha256").update(readFileSync(join(uploadDir, upload))).digest("hex");
const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok: Boolean(ok), detail: String(detail) });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? ` — ${detail}` : ""}`);
};
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

async function start() {
  const child = spawn(binary, ["--data-dir", workspace, "serve", "--assets", assets], { stdio: "ignore" });
  for (let attempt = 0; attempt < 100; attempt += 1) {
    try {
      return { child, url: execFileSync(binary, ["--data-dir", workspace, "open"]).toString().trim() };
    } catch {
      await sleep(100);
    }
  }
  child.kill();
  throw new Error("service did not start");
}
async function stop(child) {
  child.kill();
  await new Promise((done) => child.once("exit", done));
}

let service = await start();
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
const context = await browser.newContext({ acceptDownloads: true });
const page = await context.newPage();
const pageErrors = [];
page.on("pageerror", (error) => pageErrors.push(error.message));
const remote = [];
page.on("request", (request) => {
  if (!request.url().startsWith("http://127.0.0.1:")) remote.push(request.url());
});
try {
  await page.goto(service.url);
  await page.getByRole("button", { name: /Imported sources/ }).click();
  check("empty sources view before import", await page.getByText("No imported sources").isVisible());

  await page.getByLabel("Migration artifact").setInputFiles(artifact);
  await page.getByRole("button", { name: "Check artifact" }).click();
  await page.getByText("Ready to apply").waitFor();
  check("owner preview reports ready to apply", true, await page.getByLabel("Migration preview").innerText());
  page.once("dialog", (dialog) => dialog.accept());
  await page.getByRole("button", { name: "Apply migration" }).click();
  const status = page.getByRole("status");
  await status.waitFor();
  const applied = await status.innerText();
  check("owner apply succeeds with a pre-import backup", applied.includes("Applied. Pre-import backup"), applied);
  await page.getByLabel("Source bundle").first().waitFor();
  const card = await page.getByLabel("Source bundle").first().innerText();
  check("bundle card separates preservation, access, freshness and application",
    ["preservation: verified", "freshness: unverified", "live application: applied"].every((text) => card.includes(text)), card.replaceAll("\n", " | "));
  await page.screenshot({ path: join(evidence, "01-applied.png"), fullPage: true });

  await page.getByLabel("Search imported sources").fill("spec");
  await page.getByLabel("Search imported sources").press("Enter");
  await page.locator(".source-row", { hasText: "Spec <draft>" }).click();
  const documentText = page.locator(".source-text-block", { hasText: "<script>bad()</script>" });
  await documentText.waitFor();
  check("document text is readable as plain text", await documentText.isVisible());
  check("document text is not executed", (await page.locator("aside.source-detail script").count()) === 0 && pageErrors.length === 0, pageErrors.join("; "));
  await page.screenshot({ path: join(evidence, "02-document.png"), fullPage: true });

  await page.getByLabel("Search imported sources").fill("diagram");
  await page.getByLabel("Search imported sources").press("Enter");
  await page.locator(".source-row", { hasText: "uploaded file" }).first().click();
  const downloadButton = page.getByRole("button", { name: /Download diagram\.png/ });
  await downloadButton.waitFor();
  const [download] = await Promise.all([page.waitForEvent("download"), downloadButton.click()]);
  const saved = join(evidence, "downloaded-diagram.png");
  await download.saveAs(saved);
  const downloadedSha = createHash("sha256").update(readFileSync(saved)).digest("hex");
  check("downloaded upload bytes match the captured file", downloadedSha === uploadSha, `${downloadedSha} vs ${uploadSha}`);

  await page.locator(".trace-row", { hasText: "ENG-1" }).first().click();
  await page.getByRole("tab", { name: /Source/ }).click();
  const linked = page.getByLabel("Retained source records");
  await linked.waitFor();
  const linkedText = await linked.innerText();
  check("issue shows linked retained sources", linkedText.includes("comment") && linkedText.includes("link attachment"), linkedText.replaceAll("\n", " | "));
  await linked.locator(".trace-row").first().click();
  await page.getByLabel("Imported source detail").getByText("Read-only original data").waitFor();
  check("issue source link opens the retained record", true);
  await page.screenshot({ path: join(evidence, "03-issue-link.png"), fullPage: true });

  await page.reload();
  await page.getByRole("button", { name: /Imported sources/ }).click();
  await page.getByLabel("Source bundle").first().waitFor();
  check("retained sources persist after reload", true);

  await stop(service.child);
  service = await start();
  await page.goto(service.url);
  await page.getByRole("button", { name: /Imported sources/ }).click();
  await page.getByLabel("Search imported sources").fill("spec");
  await page.getByLabel("Search imported sources").press("Enter");
  await page.locator(".source-row", { hasText: "Spec <draft>" }).click();
  await page.locator(".source-text-block", { hasText: "<script>bad()</script>" }).waitFor();
  check("retained sources survive a service restart", true);
  check("no remote requests from the UI", remote.length === 0, remote.join(", "));
  await page.screenshot({ path: join(evidence, "04-after-restart.png"), fullPage: true });
} catch (error) {
  check("browser flow completed", false, error.stack || error);
  await page.screenshot({ path: join(evidence, "failure.png"), fullPage: true }).catch(() => {});
} finally {
  await browser.close();
  await stop(service.child);
}
writeFileSync(join(evidence, "results.json"), `${JSON.stringify(results, null, 2)}\n`);
process.exit(results.every((result) => result.ok) ? 0 : 1);
