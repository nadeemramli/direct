#!/usr/bin/env node
// Browser E2E for DIR-54 (comment drafts stay bound to their originating
// issue) and DIR-55 (committed writes vs failed refreshes, retry identity and
// version-conflict recovery). Synthetic data only: the script creates a fresh
// isolated workspace, drives the built UI against the real `direct serve`,
// and injects transport faults by intercepting the browser's /api/command
// traffic. A "dropped reply" is forwarded to the real service (which commits)
// before the browser's connection is aborted; nothing fakes the core.
// Persisted state is checked through the agent CLI, after reload and after a
// service restart.
//
// Setup (from the repository root):
//   cargo build -p direct && (cd app && npm run build)
// Run (Playwright must be resolvable; PLAYWRIGHT_MODULE may point at it):
//   node scripts/e2e/reliability.browser.mjs target/debug/direct <new-workspace-dir> app/dist <evidence-dir>

import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || "playwright");
const [binary, root, assets, evidence] = process.argv.slice(2).map((value) => resolve(value));
if (!evidence) {
  console.error("usage: reliability.browser.mjs <direct-binary> <new-workspace-dir> <assets-dir> <evidence-dir>");
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
async function until(probe, timeout = 6000) {
  const end = Date.now() + timeout;
  for (;;) {
    const value = await probe().catch(() => false);
    if (value || Date.now() > end) return value;
    await sleep(100);
  }
}

// ---------------------------------------------------------------- service
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
// Agent-side reads and writes through the real service (independent of the
// browser's transport and its injected faults).
const cli = (...args) => JSON.parse(execFileSync(binary, ["--data-dir", workspace, "--actor", "e2e-probe", ...args]).toString());
const issueContext = (key) => cli("context", key);
const snapshot = () => cli("list");
const agentCall = (command) =>
  JSON.parse(
    execFileSync(binary, ["--data-dir", workspace, "call"], {
      input: JSON.stringify({ actor: "e2e-other-actor", request_id: `e2e-${Math.random().toString(36).slice(2)}`, ...command }),
    }).toString(),
  );
const commentsOn = (key, text) => issueContext(key).comments.filter((comment) => comment.body === text).length;
const titled = (title) => snapshot().issues.filter((issue) => issue.title === title).length;

// ------------------------------------------------------- transport faults
// Each rule applies to the next matching /api/command request:
//   drop  — forward to the real service, then abort the browser's connection
//   fail  — abort without forwarding (nothing reaches the service)
//   hold  — forward only after `open()`; the reply is delivered normally
//   holddrop — forward only after `open()`, then abort the browser's
//          connection: the write commits while the caller sees no reply
const rules = [];
const sent = [];
function fault(action, match, { times = 1 } = {}) {
  const rule = { action, match, times, gate: null, open: null, hits: 0 };
  if (action === "hold" || action === "holddrop") rule.gate = new Promise((done) => (rule.open = done));
  rules.push(rule);
  return rule;
}
const clearFaults = () => rules.splice(0, rules.length).forEach((rule) => rule.open?.());
async function route(routeHandle, request) {
  let body = {};
  try {
    body = request.postDataJSON() || {};
  } catch {}
  if (body.request_id)
    sent.push({ op: body.op, request_id: body.request_id, title: body.title, name: body.name, body: body.body, key: body.key, payload: JSON.stringify({ ...body, request_id: undefined }) });
  const rule = rules.find((candidate) => candidate.times > 0 && candidate.match(body));
  if (!rule) return routeHandle.continue();
  rule.times -= 1;
  rule.hits += 1;
  if (rule.action === "fail") return routeHandle.abort("failed");
  if (rule.action === "hold" || rule.action === "holddrop") await rule.gate;
  const response = await routeHandle.fetch();
  if (rule.action === "drop" || rule.action === "holddrop") {
    await response.body().catch(() => {}); // the service has replied (committed)
    return routeHandle.abort("connectionreset");
  }
  return routeHandle.fulfill({ response });
}

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

// ---------------------------------------------------------------- UI helpers
const row = (page, title) => page.locator(".issue-row", { has: page.locator(".issue-title", { hasText: title }) });
// The list re-sorts as records change, so confirm the detail pane really
// shows `key` (retrying a click that landed on a row that moved).
async function open(page, title, key) {
  const shown = async () => (await page.locator("aside.detail-panel .detail-top > span").first().innerText()) === key;
  for (let attempt = 0; attempt < 3; attempt += 1) {
    await row(page, title).first().click();
    if (await until(shown, 3000)) return;
  }
  throw new Error(`could not open ${key}`);
}
const tabButton = (page, name) => page.getByLabel("Issue detail").getByRole("tab", { name });
const commentBox = (page) => page.getByLabel("Add a comment");
const commentButton = (page) => page.getByRole("button", { name: /^(Add comment|Retry posting)/ });
async function activity(page) {
  await tabButton(page, "Activity").click();
  await commentBox(page).waitFor();
}
const dialog = (page) => page.locator("dialog.modal");
const templatesNamed = (name) => snapshot().templates.filter((template) => template.name === name).length;
// Changes a control's value the way an edit would, even when the control is
// disabled for user input: proves a retry never re-reads the form.
const outOfBandEdit = (locator, value) =>
  locator.evaluate((element, value) => {
    element.value = value;
    element.dispatchEvent(new Event("input", { bubbles: true }));
  }, value);
// A user edits while the save is pending: typing when the control accepts
// input, otherwise an out-of-band change. Returns whether it was locked.
async function editWhilePending(locator, value) {
  const locked = await locator.isDisabled();
  if (locked) await outOfBandEdit(locator, value);
  else await locator.fill(value);
  return locked;
}

let service;
let browser;
const pageErrors = [];
const theoriaRoot = join(root, "theoria-source");
const catalogPath = join(root, "theoria-catalog.json");
try {
  mkdirSync(theoriaRoot, { recursive: true });
  writeFileSync(join(theoriaRoot, "guide.md"), "# Synthetic guide\n\nSynthetic guidance used to test the return path.\n");
  writeFileSync(
    catalogPath,
    JSON.stringify({ version: 1, documents: [{ id: "synthetic-guide", title: "Synthetic guide", description: "Synthetic", category: "workflow", path: "guide.md" }] }),
  );
  service = await start();
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const context = await browser.newContext({ viewport: { width: 1366, height: 768 } });
  const page = await context.newPage();
  page.setDefaultTimeout(8000);
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.route("**/api/command", route);
  await page.goto(service.url);
  await page.getByText("Connected locally").waitFor();

  // ------------------------------------------------------------ fixture
  const create = (title, extra = {}) =>
    owner(page, { op: "create_issue", product: "DIR", title, body: `Body of ${title}`, acceptance: "Synthetic acceptance", owner: "Synthetic owner", planning_scope: "inbox", ...extra });
  const A = await create("Alpha drafts issue");
  const B = await create("Bravo drafts issue");
  const E = await create("Echo conflict issue");
  let C = await create("Charlie handoff issue");
  C = await owner(page, { op: "ready", key: C.key, expected_version: C.version });
  execFileSync(binary, ["--data-dir", workspace, "--actor", "e2e-seed", "theoria-sync", "--root", theoriaRoot, "--catalog", catalogPath, "--product", "DIR", "--request-id", "e2e-theoria-sync-1"]);
  await owner(page, { op: "link_theoria", key: A.key, expected_version: A.version, document_id: "synthetic-guide", playbook_version: null });
  await row(page, "Charlie handoff issue").waitFor();
  check("fixture seeded through the real service", true, `${A.key} ${B.key} ${E.key} ${C.key}`);

  const draftA = "Draft for ALPHA only — must never reach Bravo.";
  const draftB = "Draft for BRAVO only.";

  // ============================================================ DIR-54
  try {
    await open(page, "Alpha drafts issue", A.key);
    await activity(page);
    await commentBox(page).fill(draftA);
    await open(page, "Bravo drafts issue", B.key);
    await activity(page);
    const shownOnB = await commentBox(page).inputValue();
    check("[DIR-54] switching to B does not show A's draft", shownOnB === "", JSON.stringify(shownOnB));
    await page.screenshot({ path: join(evidence, "dir54-switch-to-b.png") });

    // A user who presses "Add comment" on B must never post A's text to B.
    const button = commentButton(page);
    if (await button.isEnabled()) {
      await button.click();
      await sleep(800);
    }
    check("[DIR-54] A's draft is never posted to B", commentsOn(B.key, draftA) === 0, `${commentsOn(B.key, draftA)} copies on ${B.key}`);
    check("[DIR-54] Add comment is disabled for B's empty draft", !(await commentButton(page).isEnabled()));

    await commentBox(page).fill(draftB);
    await open(page, "Alpha drafts issue", A.key);
    await activity(page);
    check("[DIR-54] returning to A restores A's draft", (await commentBox(page).inputValue()) === draftA, await commentBox(page).inputValue());
    await open(page, "Bravo drafts issue", B.key);
    await activity(page);
    check("[DIR-54] B keeps its own draft", (await commentBox(page).inputValue()) === draftB, await commentBox(page).inputValue());

    // Guidance trip and back.
    await open(page, "Alpha drafts issue", A.key);
    await tabButton(page, "Theoria").click();
    await page.getByRole("button", { name: "Open guidance →" }).first().click();
    await page.getByRole("button", { name: new RegExp(`Back to ${A.key}`) }).click();
    await until(async () => (await page.locator("aside.detail-panel .detail-top > span").first().innerText()) === A.key);
    await activity(page);
    check("[DIR-54] A's draft survives a guidance trip and back", (await commentBox(page).inputValue()) === draftA);

    // In-flight post on A while the user switches to B and keeps typing.
    const held = fault("hold", (body) => body.op === "comment" && body.key === A.key);
    await commentButton(page).click();
    await until(async () => held.hits === 1);
    await open(page, "Bravo drafts issue", B.key);
    await activity(page);
    await commentBox(page).fill(`${draftB} (edited while A posts)`);
    held.open();
    await until(async () => commentsOn(A.key, draftA) === 1);
    await sleep(600);
    check("[DIR-54] in-flight post lands on A exactly once", commentsOn(A.key, draftA) === 1);
    check("[DIR-54] in-flight completion keeps B's draft", (await commentBox(page).inputValue()) === `${draftB} (edited while A posts)`, await commentBox(page).inputValue());
    check("[DIR-54] nothing was posted to B", issueContext(B.key).comments.length === 0, `${issueContext(B.key).comments.length} comments on ${B.key}`);
    await open(page, "Alpha drafts issue", A.key);
    await activity(page);
    check("[DIR-54] A's draft is cleared after its own successful post", (await commentBox(page).inputValue()) === "");
    check("[DIR-54] A's discussion shows the posted text once", (await page.locator("article.comment", { hasText: draftA }).count()) === 1);

    // Failure state: the post never reaches the service; the draft is kept and a retry posts once.
    const failText = "Posted after a transport failure.";
    await commentBox(page).fill(failText);
    fault("fail", (body) => body.op === "comment" && body.key === A.key);
    await commentButton(page).click();
    await page.getByRole("alert").first().waitFor();
    check("[DIR-54] failed post keeps the draft and reports it", (await commentBox(page).inputValue()) === failText && commentsOn(A.key, failText) === 0);
    await page.screenshot({ path: join(evidence, "dir54-post-failed.png") });
    await commentButton(page).click();
    await until(async () => commentsOn(A.key, failText) === 1);
    await sleep(400);
    check("[DIR-54] retry after failure posts exactly once", commentsOn(A.key, failText) === 1);

    // Keyboard flow: focus, type, Tab to the button, Enter.
    const keyboardText = "Posted with the keyboard.";
    await commentBox(page).focus();
    await page.keyboard.type(keyboardText);
    await page.keyboard.press("Tab");
    const focused = await page.evaluate(() => document.activeElement?.textContent?.trim() || "");
    await page.keyboard.press("Enter");
    await until(async () => commentsOn(A.key, keyboardText) === 1);
    check("[DIR-54] keyboard Tab+Enter posts once", commentsOn(A.key, keyboardText) === 1, `focused "${focused}"`);
    check("[DIR-54] the posted draft clears in place", await until(async () => (await commentBox(page).inputValue()) === ""));
    const shortcutText = "Posted with Ctrl+Enter.";
    await commentBox(page).focus();
    await page.keyboard.type(shortcutText);
    await until(async () => commentButton(page).isEnabled()); // the previous post has settled
    await page.keyboard.press("Control+Enter");
    await until(async () => commentsOn(A.key, shortcutText) === 1, 3000);
    check("[DIR-54] Ctrl+Enter in the draft posts once", commentsOn(A.key, shortcutText) === 1);

    // Reload and restart.
    await page.reload();
    await page.getByText("Connected locally").waitFor();
    await open(page, "Alpha drafts issue", A.key);
    await activity(page);
    check("[DIR-54] after reload A shows its text once", (await page.locator("article.comment", { hasText: draftA }).count()) === 1);
    await stop(service.child);
    service = await start();
    await page.goto(service.url);
    await page.getByText("Connected locally").waitFor();
    check("[DIR-54] after service restart A has the exact text once and B none",
      commentsOn(A.key, draftA) === 1 && issueContext(B.key).comments.length === 0);
  } catch (error) {
    check("[DIR-54] scenario completed", false, error.message.split("\n")[0]);
    await page.screenshot({ path: join(evidence, "dir54-failure.png") }).catch(() => {});
  }
  clearFaults();

  // ============================================================ DIR-55
  async function newIssueForm(title) {
    await page.getByRole("button", { name: "＋ Issue" }).first().click();
    await dialog(page).waitFor();
    await dialog(page).getByLabel("Work route").selectOption("inbox");
    await dialog(page).getByLabel("Issue title").fill(title);
    await dialog(page).getByLabel("Problem & expected outcome").fill(`Body of ${title}`);
  }
  // Committed create whose follow-up refresh fails.
  try {
    const title = "Committed while refresh fails";
    await newIssueForm(title);
    const refreshFault = fault("fail", (body) => body.op === "snapshot", { times: 1000 });
    await dialog(page).getByRole("button", { name: /Create issue/ }).click();
    await until(async () => titled(title) >= 1);
    await sleep(800);
    await page.screenshot({ path: join(evidence, "dir55-refresh-failed.png") });
    const stillOpen = await dialog(page).isVisible();
    const status = await page.getByRole("status").allInnerTexts().catch(() => []);
    check("[DIR-55] committed create closes the form instead of reporting failure", !stillOpen, stillOpen ? "form still open" : "");
    check("[DIR-55] UI reports saved + reconnecting", status.some((text) => /saved/i.test(text) && /reconnect|refresh/i.test(text)), JSON.stringify(status));
    // A user facing an apparently failed save presses it again.
    if (stillOpen) {
      await dialog(page).getByRole("button", { name: /Create issue/ }).click().catch(() => {});
      await sleep(800);
    }
    refreshFault.times = 0; // restore the transport
    await until(async () => (await row(page, title).count()) >= 1);
    await sleep(600);
    check("[DIR-55] exactly one issue persisted after refresh failure", titled(title) === 1, `${titled(title)} persisted`);
    check("[DIR-55] list shows the issue once after reconnecting", (await row(page, title).count()) === 1, `${await row(page, title).count()} rows`);
    check("[DIR-55] reconnect notice clears after recovery", !(await page.getByRole("status").allInnerTexts()).some((text) => /reconnect/i.test(text)));
    if (await dialog(page).isVisible()) await dialog(page).getByRole("button", { name: "Close dialog" }).click();
  } catch (error) {
    check("[DIR-55] refresh-failure scenario completed", false, error.message.split("\n")[0]);
    await page.screenshot({ path: join(evidence, "dir55-refresh-failure.png") }).catch(() => {});
  }
  clearFaults();

  // Dropped mutation reply: retry must reuse the exact payload and request ID.
  try {
    const title = "Reply dropped after commit";
    await newIssueForm(title);
    fault("drop", (body) => body.op === "create_issue" && body.title === title);
    await dialog(page).getByRole("button", { name: /Create issue/ }).click();
    await until(async () => titled(title) === 1);
    await page.getByRole("alert").first().waitFor();
    await page.screenshot({ path: join(evidence, "dir55-reply-dropped.png") });
    const alerts = (await page.getByRole("alert").allInnerTexts()).join(" | ");
    check("[DIR-55] dropped reply is reported as an unconfirmed outcome", /confirm/i.test(alerts) && /twice|same request|once/i.test(alerts), alerts);
    check("[DIR-55] typed content is kept after the dropped reply", (await dialog(page).getByLabel("Issue title").inputValue()) === title);
    const retry = dialog(page).getByRole("button", { name: /Retry the same save/ });
    const retryShown = await retry.isVisible().catch(() => false);
    check("[DIR-55] explicit retry action is offered", retryShown);
    await (retryShown ? retry : dialog(page).getByRole("button", { name: /Create issue/ })).click();
    await until(async () => !(await dialog(page).isVisible()));
    const ids = sent.filter((entry) => entry.op === "create_issue" && entry.title === title).map((entry) => entry.request_id);
    check("[DIR-55] retry reused the original request ID", ids.length === 2 && ids[0] === ids[1], JSON.stringify(ids));
    check("[DIR-55] one persisted issue after the retry", titled(title) === 1, `${titled(title)} persisted`);

    // A distinct user operation with identical content is a distinct record.
    await newIssueForm(title);
    await dialog(page).getByRole("button", { name: /Create issue/ }).click();
    await until(async () => titled(title) === 2);
    const all = sent.filter((entry) => entry.op === "create_issue" && entry.title === title).map((entry) => entry.request_id);
    check("[DIR-55] a new form creates a distinct record with a new request ID", titled(title) === 2 && new Set(all).size === 2, JSON.stringify(all));
  } catch (error) {
    check("[DIR-55] dropped-reply scenario completed", false, error.message.split("\n")[0]);
    await page.screenshot({ path: join(evidence, "dir55-dropped-failure.png") }).catch(() => {});
    if (await dialog(page).isVisible()) await dialog(page).getByRole("button", { name: "Close dialog" }).click();
  }
  clearFaults();

  // QC on a9e6b65: an edit made while the create is pending must never ride
  // the retry of the original request.
  try {
    const title = "Held create original";
    const edited = "Held create edited while pending";
    await newIssueForm(title);
    const held = fault("holddrop", (body) => body.op === "create_issue" && body.title === title);
    await dialog(page).getByRole("button", { name: /Create issue/ }).click();
    await until(async () => held.hits === 1);
    const locked = await editWhilePending(dialog(page).getByLabel("Issue title"), edited);
    check("[DIR-55 QC] form fields are locked while the save is pending", locked);
    held.open();
    await until(async () => titled(title) === 1);
    await page.getByRole("alert").first().waitFor();
    const retry = dialog(page).getByRole("button", { name: /Retry the same save/ });
    await ((await retry.isVisible().catch(() => false)) ? retry : dialog(page).getByRole("button", { name: /Create issue/ })).click();
    await sleep(1200);
    const attempts = sent.filter((entry) => entry.op === "create_issue" && (entry.title === title || entry.title === edited));
    check("[DIR-55 QC] retry sends the exact original command and request ID",
      attempts.length === 2 && attempts[0].request_id === attempts[1].request_id && attempts[0].payload === attempts[1].payload,
      JSON.stringify(attempts.map((entry) => [entry.title, entry.request_id])));
    check("[DIR-55 QC] exactly one record; the pending-time edit created nothing", titled(title) + titled(edited) === 1 && titled(title) === 1,
      JSON.stringify({ original: titled(title), edited: titled(edited) }));
    const kept = (await dialog(page).isVisible()) ? await dialog(page).getByLabel("Issue title").inputValue() : "";
    check("[DIR-55 QC] the later edit is kept, unsaved, for an explicit decision", kept === edited, JSON.stringify(kept));
    const status = (await page.getByRole("status").allInnerTexts()).join(" | ").replace(/\s+/g, " ");
    check("[DIR-55 QC] the form says the later changes are not saved", /not saved/i.test(status), status);
    await page.screenshot({ path: join(evidence, "dir55-qc-later-edit.png") });
    if (kept === edited) {
      // Editing requires a complete brief; the original create left acceptance empty.
      await dialog(page).getByLabel("Acceptance criteria").fill("Synthetic acceptance");
      await dialog(page).getByRole("button", { name: /Save brief/ }).click();
      await until(async () => titled(edited) === 1);
      await until(async () => !(await dialog(page).isVisible()));
      const key = snapshot().issues.find((issue) => issue.title === edited)?.key;
      check("[DIR-55 QC] saving the kept edit updates that same record", titled(edited) === 1 && titled(title) === 0,
        `${key} ${JSON.stringify({ original: titled(title), edited: titled(edited) })}`);
    }
    if (await dialog(page).isVisible()) await dialog(page).getByRole("button", { name: "Close dialog" }).click();
  } catch (error) {
    check("[DIR-55 QC] held-reply edit scenario completed", false, error.message.split("\n")[0]);
    await page.screenshot({ path: join(evidence, "dir55-qc-failure.png") }).catch(() => {});
    if (await dialog(page).isVisible()) await dialog(page).getByRole("button", { name: "Close dialog" }).click();
  }
  clearFaults();

  // Unknown outcome followed by a reload: nothing is resubmitted silently.
  try {
    const committed = "Unknown then reload (committed)";
    await newIssueForm(committed);
    fault("drop", (body) => body.op === "create_issue" && body.title === committed);
    await dialog(page).getByRole("button", { name: /Create issue/ }).click();
    await page.getByRole("alert").first().waitFor();
    await page.reload();
    await page.getByText("Connected locally").waitFor();
    await sleep(1500);
    check("[DIR-55 QC] committed unknown write is listed once after reload, never resent",
      titled(committed) === 1 && (await row(page, committed).count()) === 1 &&
        sent.filter((entry) => entry.op === "create_issue" && entry.title === committed).length === 1);
    const lost = "Unknown then reload (never sent)";
    await newIssueForm(lost);
    fault("fail", (body) => body.op === "create_issue" && body.title === lost);
    await dialog(page).getByRole("button", { name: /Create issue/ }).click();
    await page.getByRole("alert").first().waitFor();
    await page.reload();
    await page.getByText("Connected locally").waitFor();
    await sleep(1500);
    check("[DIR-55 QC] a write that never arrived is not created by reload", titled(lost) === 0);
    // The owner deliberately creates it again: a distinct, explicit operation.
    await newIssueForm(committed);
    await dialog(page).getByRole("button", { name: /Create issue/ }).click();
    await until(async () => titled(committed) === 2);
    const ids = sent.filter((entry) => entry.op === "create_issue" && entry.title === committed).map((entry) => entry.request_id);
    check("[DIR-55 QC] an intentional repeat after reload is a distinct record", titled(committed) === 2 && new Set(ids).size === 2, JSON.stringify(ids));
  } catch (error) {
    check("[DIR-55 QC] unknown+reload scenario completed", false, error.message.split("\n")[0]);
    if (await dialog(page).isVisible()) await dialog(page).getByRole("button", { name: "Close dialog" }).click();
  }
  clearFaults();

  // The inline template editor: same guarantees.
  try {
    const name = "Held template original";
    const edited = "Held template edited while pending";
    await page.getByRole("button", { name: /Intake templates/ }).first().click();
    await page.getByRole("heading", { name: "Intake templates" }).waitFor();
    const detail = page.getByLabel("Template detail");
    await page.getByRole("button", { name: "＋ New template" }).click();
    await detail.getByLabel("Template name").fill(name);
    await detail.getByLabel("Intent prompt").fill("Who gains what outcome?");
    const held = fault("holddrop", (body) => body.op === "create_template" && body.name === name);
    await detail.getByRole("button", { name: "Create template" }).click();
    await until(async () => held.hits === 1);
    const locked = await editWhilePending(detail.getByLabel("Template name"), edited);
    check("[DIR-55 QC templates] editor fields are locked while the save is pending", locked);
    held.open();
    await until(async () => templatesNamed(name) === 1);
    await detail.getByRole("alert").first().waitFor();
    const alert = (await detail.getByRole("alert").allInnerTexts()).join(" | ");
    check("[DIR-55 QC templates] unconfirmed save is reported", /confirm/i.test(alert), alert);
    const retry = detail.getByRole("button", { name: /Retry the same save/ });
    const offered = await retry.isVisible().catch(() => false);
    check("[DIR-55 QC templates] explicit exact retry is offered", offered);
    await (offered ? retry : detail.getByRole("button", { name: /Create template|Save new revision/ })).click();
    await sleep(1200);
    const attempts = sent.filter((entry) => entry.op === "create_template" && (entry.name === name || entry.name === edited));
    check("[DIR-55 QC templates] retry sends the exact original command and request ID",
      attempts.length === 2 && attempts[0].request_id === attempts[1].request_id && attempts[0].payload === attempts[1].payload,
      JSON.stringify(attempts.map((entry) => [entry.name, entry.request_id])));
    check("[DIR-55 QC templates] exactly one template; the pending-time edit created nothing",
      templatesNamed(name) === 1 && templatesNamed(edited) === 0, JSON.stringify({ original: templatesNamed(name), edited: templatesNamed(edited) }));
    const kept = await detail.getByLabel("Template name").inputValue().catch(() => "");
    const heading = await detail.locator(".detail-top > span").innerText();
    check("[DIR-55 QC templates] the later edit is kept as an unsaved revision", kept === edited && heading === "Revise template", `${heading}: ${kept}`);
    await page.screenshot({ path: join(evidence, "dir55-qc-template-later-edit.png") });
    if (kept === edited) {
      await detail.getByRole("button", { name: "Save new revision" }).click();
      await until(async () => templatesNamed(edited) === 1);
      check("[DIR-55 QC templates] saving the kept edit revises that same template",
        templatesNamed(edited) === 1 && templatesNamed(name) === 0 && snapshot().templates.find((t) => t.name === edited)?.current_revision === 2);
    }
    // A distinct, intentional template with identical content is its own record.
    await page.getByRole("button", { name: "＋ New template" }).click();
    await detail.getByLabel("Template name").fill(edited);
    await detail.getByLabel("Intent prompt").fill("Who gains what outcome?");
    await detail.getByRole("button", { name: "Create template" }).click();
    await until(async () => templatesNamed(edited) === 2);
    check("[DIR-55 QC templates] a new editor is a distinct operation", templatesNamed(edited) === 2);
    await page.getByRole("button", { name: /All work/ }).first().click();
  } catch (error) {
    check("[DIR-55 QC templates] scenario completed", false, error.message.split("\n")[0]);
    await page.screenshot({ path: join(evidence, "dir55-qc-template-failure.png") }).catch(() => {});
    await page.getByRole("button", { name: /All work/ }).first().click().catch(() => {});
  }
  clearFaults();

  // Version conflict while editing: keep typed content, reconcile explicitly, never overwrite.
  try {
    await open(page, "Echo conflict issue", E.key);
    await page.getByRole("button", { name: "Edit brief" }).click();
    await dialog(page).waitFor();
    const myBody = "Owner's revised body typed before the conflict.";
    await dialog(page).getByLabel("Problem & expected outcome").fill(myBody);
    const before = issueContext(E.key).issue;
    agentCall({ op: "update_issue", key: E.key, expected_version: before.version, title: "Echo retitled by another actor", body: before.body, acceptance: "Acceptance changed by another actor", owner: before.owner, priority: before.priority });
    await dialog(page).getByRole("button", { name: /Save brief/ }).click();
    await page.getByRole("alert").first().waitFor();
    await page.screenshot({ path: join(evidence, "dir55-edit-conflict.png") });
    check("[DIR-55] edit conflict keeps the typed body", (await dialog(page).getByLabel("Problem & expected outcome").inputValue()) === myBody);
    const reconcile = dialog(page).getByRole("button", { name: /Load latest and merge/ });
    const offered = await reconcile.isVisible().catch(() => false);
    check("[DIR-55] edit conflict offers an explicit reconcile action", offered);
    const afterConflict = issueContext(E.key).issue;
    check("[DIR-55] the conflicting save changed nothing", afterConflict.title === "Echo retitled by another actor" && afterConflict.body === before.body);
    if (offered) {
      await reconcile.click();
      await until(async () => (await dialog(page).getByLabel("Issue title").inputValue()) === "Echo retitled by another actor");
      check("[DIR-55] reconcile adopts the other actor's untouched-field changes",
        (await dialog(page).getByLabel("Issue title").inputValue()) === "Echo retitled by another actor" &&
          (await dialog(page).getByLabel("Acceptance criteria").inputValue()) === "Acceptance changed by another actor");
      check("[DIR-55] reconcile keeps the owner's own change", (await dialog(page).getByLabel("Problem & expected outcome").inputValue()) === myBody);
      await dialog(page).getByRole("button", { name: /Save brief/ }).click();
      await until(async () => !(await dialog(page).isVisible()));
    }
    const saved = issueContext(E.key).issue;
    check("[DIR-55] saved brief combines both actors' work without overwriting",
      saved.title === "Echo retitled by another actor" && saved.acceptance === "Acceptance changed by another actor" && saved.body === myBody,
      JSON.stringify({ title: saved.title, acceptance: saved.acceptance, body: saved.body }));
    const updates = sent.filter((entry) => entry.op === "update_issue" && entry.key === E.key).map((entry) => entry.request_id);
    check("[DIR-55] revised save used a fresh request ID", updates.length >= 2 && new Set(updates).size === updates.length, JSON.stringify(updates));
  } catch (error) {
    check("[DIR-55] edit-conflict scenario completed", false, error.message.split("\n")[0]);
    await page.screenshot({ path: join(evidence, "dir55-edit-failure.png") }).catch(() => {});
    if (await dialog(page).isVisible()) await dialog(page).getByRole("button", { name: "Close dialog" }).click();
  }
  clearFaults();

  // Version conflict while submitting a handoff.
  try {
    await open(page, "Charlie handoff issue", C.key);
    await page.getByRole("button", { name: /Claim this work/ }).click();
    await page.getByRole("button", { name: /Submit for verification/ }).waitFor();
    await page.getByRole("button", { name: /Submit for verification/ }).click();
    await dialog(page).waitFor();
    const build = "commit:0123456789abcdef0123456789abcdef01234567";
    const fill = async (label, value) => dialog(page).getByLabel(label, { exact: true }).fill(value);
    await fill("Test environment", "isolated synthetic workspace");
    await fill("Tested entrypoint", "built UI over direct serve");
    await fill("Acceptance scenarios and observed results", "Synthetic scenario: expected and observed match");
    await fill("Delivered build", build);
    await fill("Delivery check", "fixture service launched from this build");
    await fill("What changed", "Synthetic handoff text that must survive a conflict");
    await fill("Tested build / commit", build);
    await fill("Delivery reference", "claude/synthetic");
    await fill("Checks run and results", "synthetic checks passed");
    await fill("Step 1", "Open the issue");
    await fill("Expected result", "It renders");
    const latest = issueContext(C.key).issue;
    await owner(page, { op: "comment", key: C.key, expected_version: latest.version, body: "Concurrent change from another session" });
    await dialog(page).getByRole("button", { name: /Submit for verification/ }).click();
    await page.getByRole("alert").first().waitFor();
    check("[DIR-55] submit conflict keeps the handoff text", (await dialog(page).getByLabel("What changed").inputValue()) === "Synthetic handoff text that must survive a conflict");
    check("[DIR-55] submit conflict left the issue in Doing", issueContext(C.key).issue.status === "doing");
    const refresh = dialog(page).getByRole("button", { name: /Use the current version/ });
    const offered = await refresh.isVisible().catch(() => false);
    check("[DIR-55] submit conflict offers an explicit refresh action", offered);
    await page.screenshot({ path: join(evidence, "dir55-submit-conflict.png") });
    if (offered) {
      await refresh.click();
      await dialog(page).getByRole("button", { name: /Submit for verification/ }).click();
      await until(async () => issueContext(C.key).issue.status === "verify");
    }
    const submitted = issueContext(C.key);
    check("[DIR-55] resubmission with the fresh version lands once", submitted.issue.status === "verify" && submitted.verifications.length === 1 &&
      submitted.verifications[0].summary === "Synthetic handoff text that must survive a conflict");
  } catch (error) {
    check("[DIR-55] submit-conflict scenario completed", false, error.message.split("\n")[0]);
    await page.screenshot({ path: join(evidence, "dir55-submit-failure.png") }).catch(() => {});
    if (await dialog(page).isVisible()) await dialog(page).getByRole("button", { name: "Close dialog" }).click();
  }
  clearFaults();

  // Reload and restart: persisted counts are unchanged.
  try {
    await page.reload();
    await page.getByText("Connected locally").waitFor();
    const counts = () => ({
      refresh: titled("Committed while refresh fails"),
      dropped: titled("Reply dropped after commit"),
    });
    const beforeRestart = counts();
    await stop(service.child);
    service = await start();
    await page.goto(service.url);
    await page.getByText("Connected locally").waitFor();
    const afterRestart = counts();
    check("[DIR-55] reload/restart preserve exactly the intended records",
      beforeRestart.refresh === 1 && beforeRestart.dropped === 2 && JSON.stringify(beforeRestart) === JSON.stringify(afterRestart),
      JSON.stringify({ beforeRestart, afterRestart }));
    check("[DIR-55] UI lists the recovered issue once after restart", (await row(page, "Committed while refresh fails").count()) === 1);
  } catch (error) {
    check("[DIR-55] reload/restart scenario completed", false, error.message.split("\n")[0]);
  }
  check("no uncaught page errors", pageErrors.length === 0, pageErrors.join(" | "));
} catch (error) {
  check("script completed", false, error.stack || error.message);
} finally {
  await browser?.close().catch(() => {});
  if (service) await stop(service.child).catch(() => {});
  writeFileSync(join(evidence, "reliability-results.json"), JSON.stringify(results, null, 2));
  const failed = results.filter((result) => !result.ok).length;
  console.log(`${results.length - failed}/${results.length} checks passed`);
  process.exit(failed ? 1 : 0);
}
