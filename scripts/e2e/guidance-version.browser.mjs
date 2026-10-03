#!/usr/bin/env node
// Browser E2E for DIR-39: the playbook version recorded on an issue's
// Theoria guidance is visible with source/fingerprint provenance in the issue
// Theoria tab, its method-finding context, the guidance detail trace and the
// Theoria proposal list. An unrecorded version stays explicitly Unknown and
// is never inferred from guidance text ("Recommended next pilot"), dates or
// the catalog revision. Stale and unavailable guidance keep the historical
// pin. Checked across navigation, reload, a service restart and the agent CLI.
// Synthetic data only, in a fresh isolated workspace.
//
// Setup (from the repository root):
//   cargo build -p direct && (cd app && npm run build)
// Run (Playwright must be resolvable; PLAYWRIGHT_MODULE may point at it):
//   node scripts/e2e/guidance-version.browser.mjs target/debug/direct <new-workspace-dir> app/dist <evidence-dir>

import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || "playwright");
const [binary, root, assets, evidence] = process.argv.slice(2).map((value) => resolve(value));
if (!evidence) {
  console.error("usage: guidance-version.browser.mjs <direct-binary> <new-workspace-dir> <assets-dir> <evidence-dir>");
  process.exit(2);
}
if (existsSync(join(root, "workspace"))) {
  console.error(`${root}/workspace already exists; use a fresh directory`);
  process.exit(2);
}
mkdirSync(evidence, { recursive: true });
const workspace = join(root, "workspace");
const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok: Boolean(ok), detail: String(detail) });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? ` — ${detail}` : ""}`);
};
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

async function start() {
  const child = spawn(binary, ["--data-dir", workspace, "serve", "--assets", assets], { stdio: "ignore", windowsHide: true });
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
const cli = (...args) => JSON.parse(execFileSync(binary, ["--data-dir", workspace, "--actor", "e2e-probe", ...args]).toString());

// ---------------------------------------------------------------- fixture
const KNOWN = "v1.1 Direct baseline (synthetic E2E pin)";
const HISTORICAL = "v1-original (synthetic historical pin)";
const UNKNOWN = "Unknown — not recorded when linked";
const NO_GUIDANCE = "Unknown — no guidance linked to this issue";
const theoriaRoot = join(root, "theoria-source");
const catalogPath = join(root, "theoria-catalog.json");
let syncs = 0;
function writeTheoria(revision) {
  mkdirSync(theoriaRoot, { recursive: true });
  writeFileSync(join(theoriaRoot, "workflow.md"), `# Synthetic workflow\n\nRevision ${revision}.\n`);
  // Text a careless implementation might mine for an "active" version.
  writeFileSync(
    join(theoriaRoot, "versions.md"),
    "# Synthetic version guide\n\nupdated: 2026-10-01\n\n| v2 Agent Verification | Recommended next pilot |\n| v5 Prototype | Owner-directed pilot |\n",
  );
  const documents = [
    { id: "synthetic-workflow", title: "Synthetic workflow guidance", description: "Known and stale pins.", category: "workflow", path: "workflow.md" },
    { id: "synthetic-versions", title: "Synthetic version guide", description: "Recommends v2; never an assignment.", category: "playbook", path: "versions.md" },
    { id: "synthetic-missing", title: "Synthetic guidance whose source is missing", description: "Unavailable path.", category: "playbook", path: "does-not-exist.md" },
  ];
  writeFileSync(catalogPath, JSON.stringify({ version: 1, documents }, null, 2));
  syncs += 1;
  execFileSync(binary, [
    "--data-dir", workspace, "--actor", "e2e-seed", "theoria-sync",
    "--root", theoriaRoot, "--catalog", catalogPath, "--product", "DIR", "--request-id", `e2e-dir39-sync-${syncs}`,
  ]);
}

async function owner(page, command) {
  return page.evaluate(async (command) => {
    const response = await fetch("/api/command", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${sessionStorage.getItem("direct.session")}`,
      },
      body: JSON.stringify({ actor: "owner", request_id: crypto.randomUUID(), ...command }),
    });
    const value = await response.json();
    if (!response.ok) throw new Error(`${command.op}: ${value.message}`);
    return value;
  }, command);
}
const versionOf = (key) => cli("context", key).issue.version;
async function link(page, key, document_id, playbook_version) {
  await owner(page, { op: "link_theoria", key, expected_version: versionOf(key), document_id, playbook_version });
}
async function finding(page, key, proposal) {
  await owner(page, {
    op: "create_method_finding", key, expected_version: versionOf(key), classification: "method_friction",
    observation: `Synthetic observation for ${key}`, hypothesis: "", proposal,
    evidence: [{ kind: "check", reference: "e2e:dir39", summary: "synthetic" }],
  });
}
async function seed(page) {
  writeTheoria(1);
  const create = (title) => owner(page, { op: "create_issue", product: "DIR", title, body: "Synthetic DIR-39 fixture" });
  const mixed = await create("DIR-39 fixture mixed guidance");
  const unknownOnly = await create("DIR-39 fixture unknown only");
  const none = await create("DIR-39 fixture no guidance");
  await link(page, mixed.key, "synthetic-workflow", KNOWN);
  await link(page, mixed.key, "synthetic-versions", null);
  await link(page, mixed.key, "synthetic-missing", HISTORICAL);
  await link(page, unknownOnly.key, "synthetic-workflow", "   ");
  await finding(page, mixed.key, "Mixed proposal");
  await finding(page, none.key, "Unlinked proposal");
  return { mixed: mixed.key, unknownOnly: unknownOnly.key, none: none.key };
}

// ---------------------------------------------------------------- probes
const detail = (page) => page.getByLabel("Issue detail");
async function openIssueTheoria(page, key) {
  await page.getByRole("button", { name: /All work/ }).click();
  await page.getByLabel("Search issues").fill(key);
  await page.getByLabel("Search issues").press("Enter");
  await detail(page).getByRole("tab", { name: "Theoria" }).click();
  await page.locator(".theoria-card").first().waitFor();
}
const reference = (page, title) => page.locator(".guidance-reference", { hasText: title });
const metaValue = (card, term) =>
  card.locator("dl.reference-meta").evaluate((dl, term) => {
    const dt = [...dl.querySelectorAll("dt")].find((node) => node.textContent.trim() === term);
    return dt?.nextElementSibling?.textContent.trim() ?? null;
  }, term);
const findingsContext = (page) =>
  page.locator(".theoria-card dl.playbook-context dd").innerText();

async function issueTabChecks(page, fixture, label, { stale }) {
  await openIssueTheoria(page, fixture.mixed);
  const known = reference(page, "Synthetic workflow guidance");
  check(`[${label}] known version shown on the issue Theoria tab`, (await metaValue(known, "Playbook version")) === KNOWN, await metaValue(known, "Playbook version"));
  check(`[${label}] known pin shows recorded fingerprint and link provenance`,
    /^[0-9a-f]{64}$/.test(await metaValue(known, "Recorded fingerprint")) && (await metaValue(known, "Linked")).startsWith("owner"),
    `${await metaValue(known, "Recorded fingerprint")} · ${await metaValue(known, "Linked")}`);
  check(`[${label}] known pin state is ${stale ? "stale" : "cached"}`,
    (await known.locator(".source-state").innerText()).trim().toLowerCase() === (stale ? "stale" : "cached"));
  if (stale)
    check(`[${label}] stale pin keeps its recorded version and differs from the current fingerprint`,
      (await metaValue(known, "Recorded fingerprint")) !== (await metaValue(known, "Current fingerprint")) && (await known.innerText()).includes("historical reference is preserved"));
  const recommended = reference(page, "Synthetic version guide");
  const recommendedVersion = await metaValue(recommended, "Playbook version");
  check(`[${label}] unrecorded version is explicit Unknown, not inferred from "Recommended" guidance`,
    recommendedVersion === UNKNOWN && !/v2|v5|2026/.test(recommendedVersion), recommendedVersion);
  const missing = reference(page, "Synthetic guidance whose source is missing");
  check(`[${label}] unavailable guidance keeps its historical version pin`,
    (await metaValue(missing, "Playbook version")) === HISTORICAL && (await missing.locator(".source-state").innerText()).trim().toLowerCase() === "unavailable");
  check(`[${label}] method findings show the issue's playbook context`,
    (await findingsContext(page)) === `${KNOWN} · ${HISTORICAL} · Unknown for 1 of 3 links`, await findingsContext(page));

  await openIssueTheoria(page, fixture.unknownOnly);
  check(`[${label}] whitespace-only version stays Unknown`, (await metaValue(reference(page, "Synthetic workflow guidance"), "Playbook version")) === UNKNOWN);
  await openIssueTheoria(page, fixture.none);
  check(`[${label}] findings without linked guidance are explicit Unknown`, (await findingsContext(page)) === NO_GUIDANCE, await findingsContext(page));
}

async function guidanceDetailChecks(page, fixture, label) {
  await openIssueTheoria(page, fixture.mixed);
  await reference(page, "Synthetic workflow guidance").getByRole("button", { name: "Open guidance →" }).click();
  const panel = page.getByLabel("Theoria guidance detail");
  await panel.waitFor();
  const text = await panel.innerText();
  check(`[${label}] guidance header labels the catalog revision, not a playbook version`,
    text.includes("catalog revision") && !/catalog v\d/.test(text));
  const traceVersion = async (key) =>
    (await panel.locator(".trace-row", { hasText: key }).locator(".trace-version").innerText()).trim();
  check(`[${label}] guidance trace shows each issue's recorded version`,
    (await traceVersion(fixture.mixed)) === `Playbook version: ${KNOWN}` && (await traceVersion(fixture.unknownOnly)) === `Playbook version: ${UNKNOWN}`,
    `${await traceVersion(fixture.mixed)} | ${await traceVersion(fixture.unknownOnly)}`);
  const provenance = await panel.locator(".trace-row", { hasText: fixture.mixed }).innerText();
  check(`[${label}] guidance trace shows fingerprint and linking provenance`, /Pinned [0-9a-f]{12} by owner/.test(provenance), provenance.replace(/\s+/g, " "));
  await page.screenshot({ path: join(evidence, `dir39-${label}-guidance-trace.png`) });
  await page.getByRole("button", { name: new RegExp(`Back to ${fixture.mixed}`) }).click();
  await detail(page).getByRole("tab", { name: "Theoria", selected: true }).waitFor();
  check(`[${label}] Back returns to the issue Theoria tab with the version visible`,
    (await detail(page).getByRole("tab", { name: "Theoria" }).getAttribute("aria-selected")) === "true" &&
      (await metaValue(reference(page, "Synthetic workflow guidance"), "Playbook version")) === KNOWN);
  await page.screenshot({ path: join(evidence, `dir39-${label}-issue-theoria.png`) });
  await page.getByRole("button", { name: /Guidance & findings/ }).click();
  const proposal = (name) => page.locator(".proposal-row", { hasText: name }).locator(".playbook-context").innerText();
  check(`[${label}] Theoria proposal list shows each finding's issue playbook context`,
    (await proposal("Mixed proposal")) === `Issue playbook: ${KNOWN} · ${HISTORICAL} · Unknown for 1 of 3 links` &&
      (await proposal("Unlinked proposal")) === `Issue playbook: ${NO_GUIDANCE}`,
    `${await proposal("Mixed proposal")} | ${await proposal("Unlinked proposal")}`);
}

let service;
let browser;
const pageErrors = [];
try {
  service = await start();
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const page = await (await browser.newContext({ viewport: { width: 1366, height: 768 } })).newPage();
  page.setDefaultTimeout(8000);
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.goto(service.url);
  await page.getByText("Connected locally").waitFor();
  const fixture = await seed(page);
  await page.reload();
  await page.getByText("Connected locally").waitFor();
  check("fixture seeded through the real service", true, JSON.stringify(fixture));

  for (const [label, run] of [
    ["fresh", () => issueTabChecks(page, fixture, "fresh", { stale: false })],
    ["fresh", () => guidanceDetailChecks(page, fixture, "fresh")],
  ])
    await run().catch(async (error) => {
      check(`[${label}] scenario completed`, false, error.message.split("\n")[0]);
      await page.screenshot({ path: join(evidence, `dir39-${label}-failure.png`) }).catch(() => {});
    });

  // Newer guidance: the cache changes, the issue's pinned version does not.
  writeTheoria(2);
  await page.reload();
  await page.getByText("Connected locally").waitFor();
  await issueTabChecks(page, fixture, "stale+reload", { stale: true }).catch((error) =>
    check("[stale+reload] scenario completed", false, error.message.split("\n")[0]));

  await stop(service.child);
  service = await start();
  await page.goto(service.url);
  await page.getByText("Connected locally").waitFor();
  for (const run of [
    () => issueTabChecks(page, fixture, "restart", { stale: true }),
    () => guidanceDetailChecks(page, fixture, "restart"),
  ])
    await run().catch(async (error) => {
      check("[restart] scenario completed", false, error.message.split("\n")[0]);
      await page.screenshot({ path: join(evidence, "dir39-restart-failure.png") }).catch(() => {});
    });

  const refs = cli("context", fixture.mixed).issue.theoria_refs;
  check("[persistence] agent context keeps the exact recorded versions",
    JSON.stringify(refs.map((ref) => ref.playbook_version)) === JSON.stringify([KNOWN, null, HISTORICAL]),
    JSON.stringify(refs.map((ref) => ref.playbook_version)));
  check("[persistence] unknown-only issue still has no recorded version",
    cli("context", fixture.unknownOnly).issue.theoria_refs[0].playbook_version === null);
  check("no page errors", pageErrors.length === 0, pageErrors.join("; "));
} catch (error) {
  check("browser flow completed", false, error.stack || error);
} finally {
  await browser?.close();
  if (service) await stop(service.child);
}
writeFileSync(join(evidence, "results.json"), `${JSON.stringify(results, null, 2)}\n`);
const failed = results.filter((result) => !result.ok).length;
console.log(`${results.length - failed}/${results.length} checks passed`);
process.exit(failed ? 1 : 0);
