#!/usr/bin/env node
// Browser E2E for relating issues while creating them. Synthetic data only: the
// script creates a fresh isolated workspace, seeds two synthetic products and
// a few issues through the real service, then drives the built UI: the owner
// creates an issue with Parent, Blocked by and Related relations, sees
// pre-save validation, has a server-refused relation reject the whole create
// while the form keeps its input, finds the relations on the new issue and on
// each target, and confirms they persist across reload and a service restart.
// Changing product drops pending relations, and a plain create still works.
//
// Setup (from the repository root):
//   cargo build -p direct && (cd app && npm run build)
// Run (Playwright must be resolvable; PLAYWRIGHT_MODULE may point at it):
//   node scripts/e2e/relations.browser.mjs target/debug/direct <new-workspace-dir> app/dist <evidence-dir>

import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || "playwright");
const [binary, root, assets, evidence] = process.argv.slice(2).map((value) => resolve(value));
if (!evidence) {
  console.error("usage: relations.browser.mjs <direct-binary> <new-workspace-dir> <assets-dir> <evidence-dir>");
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

let service = await start();
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } });
const pageErrors = [];
page.on("pageerror", (error) => pageErrors.push(error.message));
const dialog = page.locator("dialog");
const sidebar = page.getByLabel("Sidebar");
const relations = dialog.getByRole("group", { name: "Relations", exact: true });

async function allWork() {
  await sidebar.getByRole("button", { name: /All work/ }).click();
  await page.getByLabel("Filter by project").selectOption("all");
}
async function newIssue(product, title) {
  await page.getByRole("button", { name: /New issue/ }).first().click();
  await dialog.waitFor();
  await dialog.locator("select").first().selectOption(product);
  await dialog.getByLabel("Work route").selectOption("inbox");
  await dialog.getByLabel("Issue title").fill(title);
}
async function relate(kind, search) {
  await relations.getByLabel("New relation kind").selectOption(kind);
  await relations.getByLabel("Find issue by key or title").fill(search);
  await relations.getByRole("button", { name: "Add relation" }).click();
}
const pendingCards = () => relations.locator(".relation-card").allInnerTexts();
async function openRelations(key) {
  await allWork();
  await page.locator(".issue-row", { hasText: key }).first().click();
  await page.getByRole("tab", { name: /Relations/ }).click();
  await page.locator(".relations-panel").first().waitFor();
  // Kind labels are upper-cased by CSS; compare case-insensitively.
  return (await page.locator(".relations-panel .relation-card").allInnerTexts()).map((text) =>
    text.replace(/\s+/g, " ").trim().toLowerCase(),
  );
}
async function scenario(name, body) {
  try {
    await body();
  } catch (error) {
    check(`${name} completed`, false, (error.stack || String(error)).split("\n")[0]);
    await page.screenshot({ path: join(evidence, `failure-${name.replaceAll(" ", "-")}.png`), fullPage: true }).catch(() => {});
    if (await dialog.count()) await page.keyboard.press("Escape");
    await dialog.waitFor({ state: "detached", timeout: 5000 }).catch(() => {});
  }
}

try {
  await page.goto(service.url);
  await page.getByRole("button", { name: /New issue/ }).first().waitFor();
  // ------------------------------------------------------------ synthetic seed
  await owner(page, { op: "create_product", key: "ALP", name: "Synthetic Alpha" });
  await owner(page, { op: "create_product", key: "BET", name: "Synthetic Beta" });
  const seed = (product, title) => owner(page, { op: "create_issue", product, title, planning_scope: "inbox" });
  const parent = await seed("ALP", "Alpha parent epic");
  const blocker = await seed("ALP", "Alpha blocker");
  const related = await seed("ALP", "Alpha related note");
  const foreign = await seed("BET", "Beta foreign work");
  await page.reload();
  await page.getByRole("button", { name: /New issue/ }).first().waitFor();

  // ------------------------------------------------------------ plain create still works
  await scenario("plain create", async () => {
    await newIssue("ALP", "Plain capture without relations");
    check("new issue form offers relations", await relations.isVisible());
    let sent = null;
    const watch = (request) => {
      const body = request.postDataJSON?.();
      if (body?.op === "create_issue") sent = body;
    };
    page.on("request", watch);
    await dialog.getByRole("button", { name: /Create issue/ }).click();
    await dialog.waitFor({ state: "detached" });
    page.off("request", watch);
    check("a create without relations omits links from the request", sent && !("links" in sent), JSON.stringify(sent));
    const snapshot = await owner(page, { op: "snapshot" });
    const plain = snapshot.issues.find((issue) => issue.title === "Plain capture without relations");
    check("plain issue created with no links", plain && snapshot.issue_links.length === 0, plain?.key);
  });

  // ------------------------------------------------------------ changing product drops pending relations
  await scenario("product change", async () => {
    await newIssue("ALP", "Switching product");
    await relate("related", "ALP-2");
    check("pending relation listed", (await pendingCards()).length === 1);
    await dialog.locator("select").first().selectOption("BET");
    await relations.getByRole("status").waitFor();
    const status = await relations.getByRole("status").innerText();
    check("changing product drops relations to the old product and says so",
      (await pendingCards()).length === 0 && status.includes("ALP-2"), status);
    await relations.getByLabel("Find issue by key or title").fill("ALP");
    check("search only offers the selected product's issues",
      (await relations.getByLabel("Issue to relate").locator("option").allInnerTexts()).join("|").includes("No matching issue"));
    // Escape in a search box clears it first; close from the title field.
    await dialog.getByLabel("Issue title").focus();
    await page.keyboard.press("Escape");
    await dialog.waitFor({ state: "detached" });
  });

  // ------------------------------------------------------------ create with three relations
  const title = "Child work with relations";
  let created;
  await scenario("create with relations", async () => {
    await newIssue("ALP", title);
    await relate("parent", "epic");
    await relate("blocked_by", blocker.key);
    // Enter in the search box adds the single match instead of submitting the form.
    await relations.getByLabel("New relation kind").selectOption("related");
    await relations.getByLabel("Find issue by key or title").fill("related note");
    await relations.getByLabel("Find issue by key or title").press("Enter");
    check("dialog still open after Enter in relation search", await dialog.isVisible());
    let cards = await pendingCards();
    check("three pending relations listed",
      cards.length === 3 && cards[0].includes(parent.key) && cards[1].includes(blocker.key) && cards[2].includes(related.key),
      cards.join(" / "));

    // Pre-save validation.
    await relate("related", related.key);
    check("duplicate relation flagged before saving",
      (await relations.getByRole("alert").innerText()).includes("already"));
    await relate("parent", blocker.key);
    check("second parent flagged before saving",
      (await relations.getByRole("alert").innerText()).includes("only one parent"));
    await relate("related", blocker.key);
    check("a different kind to the same issue is allowed", (await pendingCards()).length === 4);
    await relations.getByRole("button", { name: `Remove Related ${blocker.key}` }).click();
    check("a pending relation can be removed", (await pendingCards()).length === 3);
    await page.screenshot({ path: join(evidence, "01-pending-relations.png"), fullPage: true });

    // A relation the service refuses rejects the whole create. The outgoing
    // request is rewritten to target another product's issue, which the form
    // cannot offer, to exercise the server rule and the form's failure path.
    const before = await owner(page, { op: "snapshot" });
    await page.route("**/api/command", async (route) => {
      const body = route.request().postDataJSON();
      if (body?.op !== "create_issue" || !body.links) return route.continue();
      body.links[2].target_key = foreign.key;
      await route.continue({ postData: JSON.stringify(body) });
    });
    await dialog.getByRole("button", { name: /Create issue/ }).click();
    const error = dialog.locator(".error[role=alert]");
    await error.waitFor();
    const message = await error.innerText();
    check("refused relation explains which relation and why",
      message.includes(`Relation to ${foreign.key}`) && message.includes("same product"), message);
    check("the refusal is also shown beside the relations",
      (await relations.getByRole("alert").innerText()) === message);
    check("form keeps its input after the refusal",
      (await dialog.getByLabel("Issue title").inputValue()) === title && (await pendingCards()).length === 3);
    const after = await owner(page, { op: "snapshot" });
    check("refused create wrote no issue and no link",
      after.issues.length === before.issues.length && after.issue_links.length === before.issue_links.length);
    await page.screenshot({ path: join(evidence, "02-refused-create.png"), fullPage: true });
    await page.unroute("**/api/command");

    await dialog.getByRole("button", { name: /Create issue/ }).click();
    await dialog.waitFor({ state: "detached" });
    const snapshot = await owner(page, { op: "snapshot" });
    created = snapshot.issues.find((issue) => issue.title === title);
    const links = snapshot.issue_links.filter((link) => link.source_key === created?.key);
    check("issue and its three relations created together",
      created && created.version === 1 && links.length === 3
        && ["parent", "blocked_by", "related"].every((kind) => links.some((link) => link.kind === kind)),
      JSON.stringify(links.map((link) => [link.kind, link.target_key])));
    cards = await openRelations(created.key);
    check("new issue shows its relations",
      cards.length === 3
        && cards.some((text) => text.startsWith("parent") && text.includes(parent.key.toLowerCase()))
        && cards.some((text) => text.startsWith("blocked by") && text.includes(blocker.key.toLowerCase()))
        && cards.some((text) => text.startsWith("related") && text.includes(related.key.toLowerCase())),
      cards.join(" / "));
    await page.screenshot({ path: join(evidence, "03-new-issue-relations.png"), fullPage: true });
    for (const [target, label] of [[parent, "Child"], [blocker, "Blocks"], [related, "Related"]]) {
      const incoming = await openRelations(target.key);
      check(`${target.key} shows the new issue as ${label}`,
        incoming.some((text) => text.startsWith(label.toLowerCase()) && text.includes(created.key.toLowerCase())),
        incoming.join(" / "));
    }
  });

  // ------------------------------------------------------------ reload and restart
  await scenario("persistence", async () => {
    await page.reload();
    await page.getByRole("button", { name: /New issue/ }).first().waitFor();
    check("relations persist after reload", (await openRelations(created.key)).length === 3);
    const before = await owner(page, { op: "export" });
    await stop(service.child);
    service = await start();
    await page.goto(service.url);
    await page.getByRole("button", { name: /New issue/ }).first().waitFor();
    check("relations persist after a service restart", (await openRelations(created.key)).length === 3);
    const after = await owner(page, { op: "export" });
    check("export after restart equals export before", JSON.stringify(before) === JSON.stringify(after));
    await page.screenshot({ path: join(evidence, "04-after-restart.png"), fullPage: true });
  });
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
