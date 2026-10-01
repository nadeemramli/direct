#!/usr/bin/env node
// Browser E2E for DIR-22 workspace intake templates. Synthetic data only: the
// script creates a fresh isolated workspace, seeds two synthetic products and
// labels through the real service, then drives the built UI: the owner
// creates issue and project templates (with a bounded product supplement),
// applies them while creating issues/projects in two products (including an
// explicit override), revises a template, retires one, and confirms exact
// provenance, outdated/retired visibility, no automatic readiness, and
// persistence across reload and a service restart.
//
// Setup (from the repository root):
//   cargo build -p direct && (cd app && npm run build)
// Run (Playwright must be resolvable; PLAYWRIGHT_MODULE may point at it):
//   node scripts/e2e/templates.browser.mjs target/debug/direct <new-workspace-dir> app/dist <evidence-dir>

import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || "playwright");
const [binary, root, assets, evidence] = process.argv.slice(2).map((value) => resolve(value));
if (!evidence) {
  console.error("usage: templates.browser.mjs <direct-binary> <new-workspace-dir> <assets-dir> <evidence-dir>");
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
const agentCli = (...args) =>
  JSON.parse(execFileSync(binary, ["--data-dir", workspace, "--actor", "e2e-agent", ...args]).toString());

let service = await start();
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
const page = await browser.newPage({ viewport: { width: 1440, height: 960 } });
const pageErrors = [];
page.on("pageerror", (error) => pageErrors.push(error.message));
const dialog = page.locator("dialog");
const sidebar = page.getByLabel("Sidebar");

async function openTemplates() {
  await sidebar.getByRole("button", { name: /Intake templates/ }).click();
  await page.getByRole("heading", { name: "Intake templates" }).waitFor();
}
async function allWork() {
  await sidebar.getByRole("button", { name: /All work/ }).click();
  await page.getByLabel("Filter by project").selectOption("all");
}
async function newIssue({ product, title, template, project, priority, mode }) {
  await page.getByRole("button", { name: /New issue/ }).first().click();
  await dialog.waitFor();
  await dialog.locator("select").first().selectOption(product);
  if (template) await dialog.getByLabel("Intake template").selectOption({ label: template });
  if (mode) await dialog.getByLabel("Execution mode").selectOption(mode);
  if (project)
    await dialog.locator("label.field", { hasText: "Choose the delivery scope" }).locator("select").selectOption({ label: project });
  if (priority) await dialog.getByLabel("Priority").selectOption(priority);
  await dialog.getByLabel("Issue title").fill(title);
}

try {
  await page.goto(service.url);
  await page.getByRole("button", { name: /New issue/ }).first().waitFor();
  // ------------------------------------------------------------ synthetic seed
  const alp = await owner(page, { op: "create_product", key: "ALP", name: "Synthetic Alpha" });
  const bet = await owner(page, { op: "create_product", key: "BET", name: "Synthetic Beta" });
  await owner(page, { op: "create_label", name: "Triage" });
  await owner(page, { op: "create_label", name: "Alpha only", products: [{ product_id: alp.id, default_for_new_issues: false }] });
  await owner(page, { op: "create_project", product: "ALP", name: "Alpha intake" });
  await owner(page, { op: "create_project", product: "BET", name: "Beta intake" });
  await page.reload();
  await page.getByRole("button", { name: /New issue/ }).first().waitFor();

  // ------------------------------------------------------------ no seeding
  await openTemplates();
  check("no templates are seeded automatically", await page.getByText("No templates yet").isVisible());
  await newIssue({ product: "ALP", title: "unused" });
  check("issue intake still offers free-form capture without templates",
    (await dialog.getByLabel("Intake template").locator("option").count()) === 1);
  await page.keyboard.press("Escape");

  // ------------------------------------------------------------ owner creates issue template
  await openTemplates();
  await page.getByRole("button", { name: "＋ New template" }).click();
  const detail = page.getByLabel("Template detail");
  await detail.getByLabel("Template name").fill("Delivery");
  await detail.getByLabel("Template shape").selectOption("delivery");
  await detail.getByLabel("Intent prompt").fill("Who gains what outcome?");
  await detail.getByLabel("Suggested execution mode").selectOption("agent");
  await detail.getByLabel("Suggested priority").selectOption("high");
  await detail.getByLabel("Suggested work route").selectOption("project");
  await detail.getByLabel("Boundaries prompt").fill("Out of scope: billing");
  await detail.getByLabel("Verification prompt").fill("Observed at the real entrypoint");
  await detail.getByLabel("Checklist", { exact: true }).fill("Acceptance per criterion\nFailure path");
  await detail.locator(".template-labels").getByLabel("Triage").check();
  const alpSupplement = detail.locator("details.template-supplement", { hasText: "Synthetic Alpha" });
  await alpSupplement.locator("summary").click();
  await alpSupplement.getByLabel("Supplement note for Synthetic Alpha").fill("Alpha ships behind its preview");
  await alpSupplement.getByLabel("Supplement checklist for Synthetic Alpha").fill("Preview link");
  await alpSupplement.getByLabel("Alpha only").check();
  await detail.getByLabel("Revision note").fill("First shared base");
  await detail.getByRole("button", { name: "Create template" }).click();
  await detail.getByRole("status").waitFor();
  check("owner creates an issue template (revision 1)",
    (await detail.getByRole("status").innerText()).includes("Saved revision 1"));
  check("revision 1 shows the explicit ALP supplement",
    (await detail.getByLabel("Revision 1").innerText()).includes("+ Synthetic Alpha"));

  // ------------------------------------------------------------ owner creates project template
  await page.getByRole("button", { name: "＋ New template" }).click();
  await detail.getByLabel("Template target").selectOption("project");
  await detail.getByLabel("Template name").fill("Release train");
  await detail.getByLabel("Template shape").selectOption("release");
  await detail.getByLabel("Intent prompt").fill("Release outcome and audience");
  await detail.getByLabel("Verification prompt").fill("Production evidence recorded");
  check("project templates cannot suggest an issue work route",
    (await detail.getByLabel("Suggested work route").count()) === 0);
  await detail.getByRole("button", { name: "Create template" }).click();
  await detail.getByRole("status").waitFor();
  await page.screenshot({ path: join(evidence, "01-templates.png"), fullPage: true });

  // ------------------------------------------------------------ apply in ALP (supplement)
  await newIssue({ product: "ALP", title: "Alpha templated brief", template: "Delivery · Delivery · v1", project: "Alpha intake" });
  const body = await dialog.getByLabel("Problem & expected outcome").inputValue();
  const acceptance = await dialog.getByLabel("Acceptance criteria").inputValue();
  check("template pre-fills concise intent, mode and boundaries",
    body.includes("Who gains what outcome?") && body.includes("Agent executes") && body.includes("Out of scope: billing")
      && body.includes("Alpha ships behind its preview"), body.replaceAll("\n", " | "));
  check("verification prompt and checklist (base + ALP supplement) are unchecked prompts",
    acceptance.includes("- [ ] Acceptance per criterion") && acceptance.includes("- [ ] Preview link"), acceptance.replaceAll("\n", " | "));
  check("suggested priority and route are pre-selected",
    (await dialog.getByLabel("Priority").inputValue()) === "high" && (await dialog.getByLabel(/Work route/).inputValue()) === "project");
  await page.screenshot({ path: join(evidence, "02-issue-intake.png"), fullPage: true });
  await dialog.getByRole("button", { name: /Create issue/ }).click();
  await dialog.waitFor({ state: "detached" });
  const provenance = page.getByLabel("Template provenance");
  await provenance.waitFor();
  check("issue detail shows exact provenance",
    (await provenance.innerText()).includes("Created from Delivery · revision 1"), await provenance.innerText());
  let snapshot = await owner(page, { op: "snapshot" });
  const alpIssue = snapshot.issues.find((issue) => issue.title === "Alpha templated brief");
  const triage = snapshot.labels.find((label) => label.name === "Triage");
  const alphaOnly = snapshot.labels.find((label) => label.name === "Alpha only");
  check("ALP record stores template, revision 1 and its supplement",
    alpIssue.template?.revision === 1 && alpIssue.template?.supplement_product_id === alp.id
      && JSON.stringify(alpIssue.template?.overrides) === "[]", JSON.stringify(alpIssue.template));
  check("kept suggested labels are attached", alpIssue.labels.includes(triage.id) && alpIssue.labels.includes(alphaOnly.id));
  check("templated issue stays in Backlog with no claim or run",
    alpIssue.status === "backlog" && !alpIssue.claim && !alpIssue.current_run);

  // ------------------------------------------------------------ apply in BET with overrides
  await newIssue({ product: "BET", title: "Beta templated brief", template: "Delivery · Delivery · v1", project: "Beta intake", mode: "prototype" });
  check("BET intake gets the shared base without the ALP supplement",
    !(await dialog.getByLabel("Acceptance criteria").inputValue()).includes("Preview link")
      && (await dialog.locator(".template-labels").getByLabel("Alpha only").count()) === 0);
  await dialog.getByLabel("Priority").selectOption("low");
  await dialog.getByRole("button", { name: /Create issue/ }).click();
  await dialog.waitFor({ state: "detached" });
  await provenance.waitFor();
  check("explicit overrides are shown in provenance",
    (await provenance.innerText()).includes("overridden: priority, execution_mode"), await provenance.innerText());
  snapshot = await owner(page, { op: "snapshot" });
  const betIssue = snapshot.issues.find((issue) => issue.title === "Beta templated brief");
  check("BET record: same shared template, no supplement, explicit overrides recorded",
    betIssue.template?.template_id === alpIssue.template?.template_id && betIssue.template?.supplement_product_id === null
      && betIssue.template?.execution_mode === "prototype"
      && JSON.stringify(betIssue.template?.overrides) === JSON.stringify(["priority", "execution_mode"]), JSON.stringify(betIssue.template));

  // ------------------------------------------------------------ project intake in two products
  for (const [productKey, name] of [["ALP", "Alpha release"], ["BET", "Beta release"]]) {
    await page.getByRole("button", { name: "＋ New project" }).click();
    await dialog.waitFor();
    await dialog.locator("select").first().selectOption(productKey);
    await dialog.getByLabel("Intake template").selectOption({ label: "Release train · Release · v1" });
    await dialog.getByLabel("Project name").fill(name);
    check(`project outcome pre-filled from template (${productKey})`,
      (await dialog.getByLabel("Project outcome").inputValue()).includes("Release outcome and audience"));
    await dialog.getByRole("button", { name: "Create project" }).click();
    await dialog.waitFor({ state: "detached" });
  }
  snapshot = await owner(page, { op: "snapshot" });
  const releases = snapshot.projects.filter((project) => project.name.endsWith(" release"));
  check("projects in two products record the same project template revision",
    releases.length === 2 && releases.every((project) => project.template?.revision === 1)
      && new Set(releases.map((project) => project.product_id)).size === 2, JSON.stringify(releases.map((p) => p.template)));
  check("project summary shows template provenance",
    (await page.locator(".project-summary .template-chip").innerText()).includes("Release train · v1"));

  // ------------------------------------------------------------ agent applies through the CLI
  const agentIssue = agentCli("create", "--product", "BET", "Agent applied", "--template-id", alpIssue.template.template_id, "--template-revision", "1");
  check("agent applies an allowed intake shape through the native CLI",
    agentIssue.template?.revision === 1 && agentIssue.status === "backlog");

  // ------------------------------------------------------------ owner revises
  await openTemplates();
  await page.getByRole("button", { name: "Template Delivery" }).click();
  await detail.getByRole("button", { name: "Revise" }).click();
  await detail.getByLabel("Intent prompt").fill("Outcome, user and evidence");
  await detail.getByLabel("Revision note").fill("Tighter intent");
  await detail.getByRole("button", { name: "Save new revision" }).click();
  await detail.getByRole("status").waitFor();
  const detailText = await detail.innerText();
  check("revision 2 saved and revision 1 retained in history",
    detailText.includes("REVISION 2 · CURRENT") && detailText.includes("REVISION 1") && detailText.includes("Who gains what outcome?"));
  check("outdated uses are visible on the template", detailText.includes("on an older revision"), detailText.match(/\d+ records? on an older revision/)?.[0]);
  await page.screenshot({ path: join(evidence, "03-revised.png"), fullPage: true });
  await allWork();
  await page.locator(".issue-row", { hasText: "Alpha templated brief" }).first().click();
  await provenance.waitFor();
  check("existing record keeps v1 and shows it is outdated",
    (await provenance.innerText()).includes("revision 1 · outdated (revision 2 is current)"), await provenance.innerText());
  snapshot = await owner(page, { op: "snapshot" });
  check("existing record was not rewritten", snapshot.issues.find((issue) => issue.key === alpIssue.key).template.revision === 1);
  await newIssue({ product: "ALP", title: "After revision", template: "Delivery · Delivery · v2", project: "Alpha intake" });
  await dialog.getByRole("button", { name: /Create issue/ }).click();
  await dialog.waitFor({ state: "detached" });
  snapshot = await owner(page, { op: "snapshot" });
  check("new work uses revision 2", snapshot.issues.find((issue) => issue.title === "After revision").template?.revision === 2);

  // ------------------------------------------------------------ owner retires
  await openTemplates();
  await page.getByRole("button", { name: "Template Release train" }).click();
  await detail.getByLabel("Retirement reason").fill("Superseded by per-product release flow");
  await detail.getByRole("button", { name: "Retire template" }).click();
  await detail.getByRole("status").waitFor();
  check("retired template remains readable",
    (await detail.innerText()).includes("Superseded by per-product release flow") && (await detail.getByLabel("Revision 1").count()) === 1);
  await allWork();
  await page.getByRole("button", { name: "＋ New project" }).click();
  await dialog.waitFor();
  const projectOptions = await dialog.getByLabel("Intake template").locator("option").allInnerTexts();
  check("retired template cannot be chosen for new work", !projectOptions.some((text) => text.includes("Release train")), projectOptions.join(" | "));
  await page.keyboard.press("Escape");
  snapshot = await owner(page, { op: "snapshot" });
  check("retiring left project provenance intact",
    snapshot.projects.filter((project) => project.template?.revision === 1).length === 2);

  // ------------------------------------------------------------ readiness is still an owner decision
  snapshot = await owner(page, { op: "snapshot" });
  check("no templated issue became Ready or entered verification",
    snapshot.issues.filter((issue) => issue.template).every((issue) => issue.status === "backlog")
      && (snapshot.review_ready_runs || []).length === 0);

  // ------------------------------------------------------------ reload and restart
  const before = await owner(page, { op: "export" });
  await page.reload();
  await openTemplates();
  check("templates persist after reload", (await page.locator(".template-row").count()) === 2);
  await stop(service.child);
  service = await start();
  await page.goto(service.url);
  await openTemplates();
  await page.getByRole("button", { name: "Template Delivery" }).click();
  check("revision history survives a service restart",
    (await detail.getByLabel("Revision 2").count()) === 1 && (await detail.getByLabel("Revision 1").count()) === 1);
  const after = await owner(page, { op: "export" });
  check("export after restart equals export before",
    JSON.stringify(before) === JSON.stringify(after) && after.format === 13);
  await allWork();
  await page.locator(".issue-row", { hasText: "Beta templated brief" }).first().click();
  await provenance.waitFor();
  check("provenance visible after restart", (await provenance.innerText()).includes("revision 1"));
  await page.screenshot({ path: join(evidence, "04-after-restart.png"), fullPage: true });
  check("no page errors", pageErrors.length === 0, pageErrors.join("; "));
} catch (error) {
  check("browser flow completed", false, error.stack || error);
  await page.screenshot({ path: join(evidence, "failure.png"), fullPage: true }).catch(() => {});
} finally {
  await browser.close();
  await stop(service.child);
}
writeFileSync(join(evidence, "results.json"), `${JSON.stringify(results, null, 2)}\n`);
process.exit(results.every((result) => result.ok) ? 0 : 1);
