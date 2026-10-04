#!/usr/bin/env node
// Browser E2E for the workflow view and segmented progress bars. Synthetic data
// only: the script creates a fresh isolated workspace, moves issues into each
// reachable workflow state through the real service (owner HTTP session for
// owner commands, agent CLI for claim/submit), and drives the built UI.
//
// Setup (from the repository root):
//   cargo build -p direct && (cd app && npm run build)
// Run (Playwright must be resolvable; PLAYWRIGHT_MODULE may point at it):
//   node scripts/e2e/workflow.browser.mjs target/debug/direct <new-workspace-dir> app/dist <evidence-dir>

import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || "playwright");
const [binary, root, assets, evidence] = process.argv.slice(2).map((value) => resolve(value));
if (!evidence) {
  console.error("usage: workflow.browser.mjs <direct-binary> <new-workspace-dir> <assets-dir> <evidence-dir>");
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

const AGENT = "e2e-workflow-agent";
let seq = 0;
const agent = (command) =>
  JSON.parse(
    execFileSync(binary, ["--data-dir", workspace, "call"], {
      input: JSON.stringify({ actor: AGENT, request_id: `e2e-workflow-${++seq}`, ...command }),
    }).toString(),
  );
async function owner(page, command) {
  return page.evaluate(async (command) => {
    const response = await fetch("/api/command", {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${sessionStorage.getItem("direct.session")}` },
      body: JSON.stringify({ actor: "owner", request_id: crypto.randomUUID(), ...command }),
    });
    const value = await response.json();
    if (!response.ok) throw new Error(`${command.op}: ${value.message}`);
    return value;
  }, command);
}

const BUILD = "commit:0123456789abcdef0123456789abcdef01234567";
async function create(page, project, title) {
  return owner(page, { op: "create_issue", product: "DIR", title, body: "Synthetic workflow fixture.", acceptance: "- Synthetic.", owner: "Owner", planning_scope: "project", project_id: project.id });
}
async function toReady(page, issue) {
  return owner(page, { op: "ready", key: issue.key, expected_version: issue.version });
}
const toDoing = (issue) => agent({ op: "claim", key: issue.key, expected_version: issue.version, lease_seconds: 3600 });
function toVerify(issue) {
  return agent({
    op: "submit",
    key: issue.key,
    expected_version: issue.version,
    build_ref: BUILD,
    delivery_ref: "synthetic/workflow",
    summary: "Synthetic submission",
    checks: "Synthetic checks",
    e2e: { build_ref: BUILD, delivered_build_ref: BUILD, environment: "Synthetic", entrypoint: "Synthetic", scenarios: "Synthetic", outcome: "passed", delivery_check: "Synthetic" },
    steps: [{ instruction: "Look", expected: "It works" }],
  });
}
async function review(page, issue, outcome) {
  const context = await owner(page, { op: "context", key: issue.key });
  return owner(page, {
    op: "review",
    key: issue.key,
    expected_version: context.issue.version,
    run_id: context.issue.current_run,
    outcome,
    results: [{ outcome, note: outcome === "passed" ? "" : "Synthetic failure" }],
    note: outcome === "passed" ? "" : "Synthetic failure",
  });
}

// Target distribution in project A: 3 done, 2 verify, 1 needs fix, 2 doing, 2 ready, 2 backlog (12 counted).
async function seed(page) {
  const a = await owner(page, { op: "create_project", product: "DIR", name: "Workflow alpha", description: "Synthetic", sort_order: 0 });
  const b = await owner(page, { op: "create_project", product: "DIR", name: "Workflow bravo", description: "Synthetic", sort_order: 1 });
  const issues = [];
  for (let n = 1; n <= 12; n += 1) issues.push(await create(page, a, `Alpha item ${n}`));
  const [d1, d2, d3, v1, v2, f1, g1, g2, r1, r2] = issues;
  for (const issue of [d1, d2, d3, v1, v2, f1, g1, g2, r1, r2]) Object.assign(issue, await toReady(page, issue));
  for (const issue of [d1, d2, d3, v1, v2, f1, g1, g2]) Object.assign(issue, toDoing(issue));
  for (const issue of [d1, d2, d3, v1, v2, f1]) Object.assign(issue, toVerify(issue));
  for (const issue of [d1, d2, d3]) await review(page, issue, "passed");
  await review(page, f1, "failed");
  // Project B: one done, one backlog → 50%.
  const b1 = await create(page, b, "Bravo item 1");
  await create(page, b, "Bravo item 2");
  Object.assign(b1, await toReady(page, b1));
  Object.assign(b1, toDoing(b1));
  Object.assign(b1, toVerify(b1));
  await review(page, b1, "passed");
  return { a, b, needsFixKey: f1.key };
}

const rowCount = (page, name) =>
  page.locator(".wf-state-row", { has: page.locator(".wf-state-name", { hasText: new RegExp(`^${name}`) }) }).locator(".wf-state-sub").innerText();

let service;
let browser;
const pageErrors = [];
try {
  service = await start();
  browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {});
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.goto(service.url);
  await page.waitForSelector(".issue-list, .empty");
  const fixture = await seed(page);
  await page.reload();
  await page.waitForSelector(".issue-row");

  // Navigation and state rows
  await page.locator('nav[aria-label="Praxis"] button[title="Workflow states and progress"]').click();
  await page.waitForSelector(".workflow-page");
  check("Workflow page opens from the sidebar", await page.locator(".workflow-page h1").innerText() === "Workflow");
  const categories = await page.locator(".wf-category").allInnerTexts();
  check("Categories are listed in workflow order", JSON.stringify(categories) === JSON.stringify(["Backlog", "Unstarted", "Started", "Completed", "Canceled"]), categories.join(", "));
  const names = await page.locator(".wf-state-name").allInnerTexts();
  check("Backlog is marked as the default state", names[0].replace(/\s+/g, " ").trim() === "Backlog · Default", names[0]);
  const expected = { Backlog: 2 + 1, Ready: 2, "Needs fix": 1, Doing: 2, Verify: 2, Done: 4 };
  for (const [name, count] of Object.entries(expected)) {
    const text = await rowCount(page, name);
    check(`${name} row counts ${count}`, text.startsWith(`${count} issue${count === 1 ? "" : "s"}`), text);
  }
  const verifyText = await rowCount(page, "Verify");
  check("Verify row splits owner-ready and agent-E2E work", verifyText.includes("2 ready for your review · 0 awaiting agent E2E"), verifyText);
  const headline = await page.locator(".wf-overview-head b").innerText();
  check("Overall percent uses the service formula (4 of 14 → 28%)", headline === "28%", headline);
  const segs = await page.locator(".wf-overview .wf-seg").evaluateAll((els) => els.map((el) => el.dataset.state));
  check("Overview bar segments are ordered done → backlog", JSON.stringify(segs) === JSON.stringify(["done", "verify", "needs_fix", "doing", "ready", "backlog"]), segs.join(","));
  const aria = await page.locator(".wf-overview .wf-bar").getAttribute("aria-label");
  check("Overview bar has a text summary for assistive tech", aria?.includes("28% verified done") && aria.includes("1 needs fix"), aria);
  await page.screenshot({ path: join(evidence, "workflow-page.png"), fullPage: true });

  // Hover tooltip
  await page.locator(".wf-overview .wf-seg.verify").hover();
  const tip = await page.locator(".wf-overview .wf-tooltip").innerText();
  check("Hovering a segment shows its state, count and share", tip.includes("Verify · 2 of 14 (14%)"), tip);
  await page.screenshot({ path: join(evidence, "workflow-tooltip.png"), clip: await page.locator(".wf-overview").boundingBox() });

  // Project progress rows
  const projectRows = await page.locator(".wf-project-row").allInnerTexts();
  check("Project progress lists both projects with percents", projectRows.some((t) => t.includes("Workflow alpha") && t.includes("25%")) && projectRows.some((t) => t.includes("Workflow bravo") && t.includes("50%")), projectRows.join(" | ").replace(/\n/g, " "));
  const serverAlpha = (await owner(page, { op: "snapshot" })).project_progress.find((p) => p.project_id === fixture.a.id);
  check("Client project percent equals the service's progress record", serverAlpha?.completion_percent === 25, JSON.stringify(serverAlpha));

  // Selecting a state filters the list by workflow state
  await page.locator(".wf-state-row", { has: page.locator(".wf-state-name", { hasText: /^Needs fix/ }) }).click();
  await page.waitForSelector(".issue-row");
  const fixRows = await page.locator(".issue-row").count();
  check("Needs fix opens a list of exactly the failed-verification issue", fixRows === 1 && (await page.locator(".issue-row").innerText()).includes(fixture.needsFixKey), fixRows);
  check("The list is titled with the state", (await page.locator(".page-heading h1").innerText()) === "Needs fix");
  await page.locator('nav[aria-label="Praxis"] button[title="Workflow states and progress"]').click();
  await page.locator(".wf-state-row", { has: page.locator(".wf-state-name", { hasText: /^Doing/ }) }).click();
  check("Doing excludes the needs-fix issue", (await page.locator(".issue-row").count()) === 2);
  await page.locator(".issue-row").first().click();
  const badge = await page.locator(".detail-heading .status-badge").innerText();
  check("Issue detail shows the workflow state badge", badge.trim() === "Doing", badge);
  await page.screenshot({ path: join(evidence, "workflow-doing-list.png") });

  // Project selection shows the segmented summary
  await page.locator('nav[aria-label="Praxis"] button[title="Workflow states and progress"]').click();
  await page.locator(".wf-project-row", { hasText: "Workflow alpha" }).click();
  await page.waitForSelector(".project-summary .wf-legend");
  const legend = await page.locator(".project-summary .wf-legend").innerText();
  check("Project summary legend names every counted state", ["Done", "Verify", "Needs fix", "Doing", "Ready", "Backlog"].every((n) => legend.includes(n)), legend.replace(/\n/g, " "));
  await page.screenshot({ path: join(evidence, "project-summary.png") });

  // By-project grouping shows a bar per group; All work shows a scope strip.
  await page.selectOption('select[aria-label="Filter by project"]', "all");
  await page.getByRole("button", { name: "By project" }).click();
  check("Each project group header has a progress bar", (await page.locator(".issue-group .group-progress .wf-bar").count()) === 2);
  check("Unfiltered list shows a scope progress strip", (await page.locator(".list-progress .wf-bar").count()) === 1);
  await page.screenshot({ path: join(evidence, "by-project.png") });

  // Reload keeps counts (persisted state, not client-only)
  await page.reload();
  await page.waitForSelector(".issue-row");
  await page.locator('nav[aria-label="Praxis"] button[title="Workflow states and progress"]').click();
  check("Counts survive a reload", (await rowCount(page, "Needs fix")).startsWith("1 issue"));

  // Narrow viewport: no horizontal page overflow
  await page.setViewportSize({ width: 900, height: 900 });
  await sleep(150);
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth);
  check("Workflow page has no horizontal overflow at 900px", overflow);
  await page.screenshot({ path: join(evidence, "workflow-narrow.png"), fullPage: true });

  check("No page errors", pageErrors.length === 0, pageErrors.join("; "));
} catch (error) {
  check("Scenario completed", false, error.stack || error.message);
} finally {
  await browser?.close();
  if (service) await stop(service.child);
  writeFileSync(join(evidence, "workflow-results.json"), JSON.stringify(results, null, 2));
}
const failed = results.filter((result) => !result.ok);
console.log(`${results.length - failed.length}/${results.length} passed`);
process.exit(failed.length ? 1 : 0);
