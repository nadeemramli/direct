#!/usr/bin/env node
// Browser E2E for DIR-59: the first connection or workspace load fails,
// stalls, or loses its response body, and the interface recovers by itself
// without a reload and without replaying any write. A used launch link and a
// session lost to a service restart are reported separately and never
// retried. Synthetic data only: the script creates a fresh isolated workspace
// and drives the built UI against the real `direct serve`.
//
// The browser reaches the service through a local pass-through proxy that
// presents the service's own Host/Origin (the service's local-origin check is
// unchanged) and can deliver a reply's headers and half its body, then stall:
// Playwright's route.fulfill always sends a complete body. Other faults are
// injected by intercepting the browser's requests, as in
// reliability.browser.mjs. One scenario uses no injection at all: a genuinely
// busy store (an outside SQLite write lock plus queued writes) stalls the real
// first snapshot. Persisted state is checked through the agent CLI.
//
// Setup (from the repository root):
//   cargo build -p direct && (cd app && npm run build)
// Run (Playwright must be resolvable; PLAYWRIGHT_MODULE may point at it):
//   node scripts/e2e/startup.browser.mjs target/debug/direct <new-workspace-dir> app/dist <evidence-dir>

import { execFile, execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { createServer, request as httpRequest } from "node:http";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";
import { DatabaseSync } from "node:sqlite";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || "playwright");
const [binary, root, assets, evidence] = process.argv.slice(2).map((value) => resolve(value));
if (!evidence) {
  console.error("usage: startup.browser.mjs <direct-binary> <new-workspace-dir> <assets-dir> <evidence-dir>");
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
let servicePort = 0;
async function start() {
  const child = spawn(binary, ["--data-dir", workspace, "serve", "--assets", assets], { stdio: "ignore", windowsHide: true });
  for (let attempt = 0; attempt < 100; attempt += 1) {
    try {
      const url = execFileSync(binary, ["--data-dir", workspace, "open"]).toString().trim();
      servicePort = Number(new URL(url).port);
      return { child };
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
const snapshot = () => cli("list");
const titled = (title) => snapshot().issues.filter((issue) => issue.title === title).length;
const agentCall = (command) =>
  JSON.parse(
    execFileSync(binary, ["--data-dir", workspace, "call"], {
      input: JSON.stringify({ actor: "e2e-other-actor", request_id: `e2e-${Math.random().toString(36).slice(2)}`, ...command }),
    }).toString(),
  );

// ------------------------------------------------------------------ proxy
// Forwards everything to the current service. `stallBody` makes the next
// snapshot reply send its headers and half its body, then hang until
// `releaseStalls()`.
let stallBody = 0;
const stalled = [];
const releaseStalls = () => stalled.splice(0).forEach((response) => response.destroy());
const proxy = createServer(async (incoming, outgoing) => {
  const chunks = [];
  for await (const chunk of incoming) chunks.push(chunk);
  const body = Buffer.concat(chunks);
  let op = "";
  try {
    op = JSON.parse(body.toString()).op || "";
  } catch {}
  const headers = { ...incoming.headers, host: `127.0.0.1:${servicePort}` };
  if (headers.origin) headers.origin = `http://127.0.0.1:${servicePort}`;
  const upstream = httpRequest({ host: "127.0.0.1", port: servicePort, method: incoming.method, path: incoming.url, headers }, (reply) => {
    const stall = incoming.url === "/api/command" && op === "snapshot" && stallBody > 0;
    if (!stall) {
      outgoing.writeHead(reply.statusCode, reply.headers);
      reply.pipe(outgoing);
      return;
    }
    stallBody -= 1;
    const parts = [];
    reply.on("data", (part) => parts.push(part));
    reply.on("end", () => {
      const whole = Buffer.concat(parts);
      const { "transfer-encoding": _chunked, ...head } = reply.headers;
      outgoing.writeHead(reply.statusCode, { ...head, "content-length": String(whole.length) });
      outgoing.write(whole.subarray(0, Math.floor(whole.length / 2)));
      stalled.push(outgoing);
    });
  });
  upstream.on("error", () => outgoing.destroy());
  upstream.end(body);
});
await new Promise((done) => proxy.listen(0, "127.0.0.1", done));
const proxyPort = proxy.address().port;
const launchUrl = () => {
  const url = new URL(execFileSync(binary, ["--data-dir", workspace, "open"]).toString().trim());
  url.port = String(proxyPort);
  return url.toString();
};

// ------------------------------------------------------- browser faults
// Per page: every request to the service is recorded; rules apply to the
// next matching request.
//   fail  — abort without forwarding (nothing reaches the service)
//   stale — fetch the real (then current) reply at once, deliver it only
//           after `open()`: a late answer from an abandoned read
function instrument(page) {
  const rules = [];
  const sent = [];
  async function handle(routeHandle, request) {
    const path = new URL(request.url()).pathname;
    let body = {};
    try {
      body = request.postDataJSON() || {};
    } catch {}
    sent.push({ path, op: body.op || "", request_id: body.request_id || "", grant: body.grant || "", title: body.title || "", at: Date.now() });
    const rule = rules.find((candidate) => candidate.times > 0 && candidate.path === path && candidate.match(body));
    if (!rule) return routeHandle.continue().catch(() => {});
    rule.times -= 1;
    rule.hits += 1;
    if (rule.action === "fail") return routeHandle.abort("failed").catch(() => {});
    const response = await routeHandle.fetch();
    await rule.gate;
    return routeHandle.fulfill({ response }).catch(() => {});
  }
  return {
    ready: Promise.all([page.route("**/api/command", handle), page.route("**/api/session", handle)]),
    sent,
    fault(action, path, match = () => true, { times = 1 } = {}) {
      const rule = { action, path, match, times, hits: 0, gate: null, open: null };
      rule.gate = new Promise((done) => (rule.open = done));
      if (action !== "stale") rule.open();
      rules.push(rule);
      return rule;
    },
    clear: () => rules.splice(0).forEach((rule) => rule.open?.()),
    writes: () => sent.filter((entry) => entry.path === "/api/command" && entry.request_id),
  };
}

const row = (page, title) => page.locator(".issue-row", { has: page.locator(".issue-title", { hasText: title }) });
const retrying = (page) => page.getByRole("status").filter({ hasText: "Can't load the workspace yet" });
const connected = (page) => page.getByText("Connected locally");
const shown = async (page, title) => (await row(page, title).count()) === 1;
const statusText = async (page) => (await page.getByRole("status").allInnerTexts().catch(() => [])).join(" | ").replace(/\s+/g, " ");
const alertText = async (page) => (await page.getByRole("alert").allInnerTexts().catch(() => [])).join(" | ").replace(/\s+/g, " ");

let service;
let browser;
const pageErrors = [];
const seededTitle = "Synthetic startup issue";
try {
  service = await start();
  agentCall({ op: "create_issue", product: "DIR", title: seededTitle, body: "Seeded before the browser opens", planning_scope: "inbox" });
  const baseline = snapshot().issues.length;
  check("fixture seeded through the real service", titled(seededTitle) === 1, `${baseline} issue(s)`);
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const context = await browser.newContext({ viewport: { width: 1366, height: 768 } });
  async function newPage() {
    const page = await context.newPage();
    page.setDefaultTimeout(8000);
    page.on("pageerror", (error) => pageErrors.push(error.message));
    const net = instrument(page);
    await net.ready;
    return { page, net };
  }

  // ===================================== dropped first snapshot, recovery
  try {
    const { page, net } = await newPage();
    net.fault("fail", "/api/command", (body) => body.op === "snapshot", { times: 2 });
    await page.goto(launchUrl());
    await retrying(page).waitFor();
    const banner = await statusText(page);
    check("[DIR-59 dropped] failed first load is shown with the reason and an automatic retry", /unavailable/i.test(banner) && /Retrying automatically/i.test(banner), banner);
    await page.screenshot({ path: join(evidence, "dir59-dropped-retrying.png") });
    const recovered = await until(async () => (await connected(page).isVisible()) && (await shown(page, seededTitle)), 10000);
    check("[DIR-59 dropped] recovers without a reload and loads the existing workspace", recovered);
    check("[DIR-59 dropped] the retry notice clears after recovery", await until(async () => !(await retrying(page).isVisible())));
    const snapshots = net.sent.filter((entry) => entry.op === "snapshot").length;
    check("[DIR-59 dropped] retried reads only; no write was sent", snapshots >= 3 && net.writes().length === 0, `${snapshots} snapshot reads, ${net.writes().length} writes`);

    // The recovered page stays responsive: one deliberate create persists once.
    const title = "Created after startup recovery";
    await page.getByRole("button", { name: "＋ Issue" }).first().click();
    const dialog = page.locator("dialog.modal");
    await dialog.waitFor();
    await dialog.getByLabel("Work route").selectOption("inbox");
    await dialog.getByLabel("Issue title").fill(title);
    await dialog.getByLabel("Problem & expected outcome").fill("Synthetic");
    await dialog.getByRole("button", { name: /Create issue/ }).click();
    await until(async () => (await shown(page, title)) && !(await dialog.isVisible()));
    const creates = net.writes().filter((entry) => entry.op === "create_issue");
    check("[DIR-59 dropped] the recovered UI saves a new issue exactly once", titled(title) === 1 && creates.length === 1, `${titled(title)} persisted, ${creates.length} sent`);

    await page.reload();
    await connected(page).waitFor();
    check("[DIR-59 reload] reload reuses the session and shows persisted work", (await shown(page, seededTitle)) && (await shown(page, title)));
    check("[DIR-59 reload] reload resends nothing", net.writes().length === 1 && titled(title) === 1);
    await page.close();
  } catch (error) {
    check("[DIR-59 dropped] scenario completed", false, error.message.split("\n")[0]);
  }

  // ================= hung first snapshot whose late, stale answer arrives
  try {
    const { page, net } = await newPage();
    const late = net.fault("stale", "/api/command", (body) => body.op === "snapshot");
    const started = Date.now();
    await page.goto(launchUrl());
    await retrying(page).waitFor({ timeout: 15000 });
    const waited = Date.now() - started;
    const banner = await statusText(page);
    check("[DIR-59 hung] a hung first snapshot is abandoned at its deadline", waited >= 9000 && waited < 14000 && /did not answer within 10 seconds/.test(banner), `${waited} ms: ${banner}`);
    await page.screenshot({ path: join(evidence, "dir59-hung-retrying.png") });
    check("[DIR-59 hung] recovers by itself after the abandoned read", await until(async () => (await connected(page).isVisible()) && (await shown(page, seededTitle)), 8000));
    // Newer state arrives through the poll, then the stale answer is released.
    const newer = "Created by an agent after recovery";
    agentCall({ op: "create_issue", product: "DIR", title: newer, body: "Newer than the stale read", planning_scope: "inbox" });
    check("[DIR-59 hung] the poll shows newer state", await until(async () => shown(page, newer)));
    late.open();
    await sleep(2000);
    check("[DIR-59 hung] the late stale answer never replaces newer state", (await shown(page, newer)) && (await connected(page).isVisible()));
    check("[DIR-59 hung] no write was sent", net.writes().length === 0, `${net.writes().length} writes`);
    await page.close();
  } catch (error) {
    check("[DIR-59 hung] scenario completed", false, error.message.split("\n")[0]);
  }

  // ======================== first snapshot body stalls after its headers
  try {
    const { page, net } = await newPage();
    stallBody = 1;
    const started = Date.now();
    await page.goto(launchUrl());
    await retrying(page).waitFor({ timeout: 15000 });
    const waited = Date.now() - started;
    const banner = await statusText(page);
    check("[DIR-59 body] a stalled response body is abandoned at the deadline", stallBody === 0 && waited >= 9000 && waited < 14000 && /did not answer within 10 seconds/.test(banner), `${waited} ms: ${banner}`);
    check("[DIR-59 body] recovers by itself while the stalled body is still open", await until(async () => (await connected(page).isVisible()) && (await shown(page, seededTitle)), 8000) && stalled.length === 1);
    releaseStalls();
    await sleep(1000);
    check("[DIR-59 body] the broken body changes nothing after recovery", (await connected(page).isVisible()) && net.writes().length === 0);
    await page.close();
  } catch (error) {
    check("[DIR-59 body] scenario completed", false, error.message.split("\n")[0]);
    releaseStalls();
  }

  // ========== a genuinely busy store stalls the real first snapshot (no injection)
  try {
    const db = new DatabaseSync(join(workspace, "direct.db"));
    db.exec("BEGIN IMMEDIATE");
    // Each queued write holds the store for its 5 s busy timeout.
    const writers = [0, 1, 2, 3, 4].map(
      (i) =>
        new Promise((done) => {
          setTimeout(() => {
            const input = JSON.stringify({ actor: "e2e-busy", request_id: `busy-${i}-${Date.now()}`, op: "create_issue", product: "DIR", title: `Queued busy write ${i}`, body: "Synthetic", planning_scope: "inbox" });
            const child = execFile(binary, ["--data-dir", workspace, "call"], { windowsHide: true }, () => done());
            child.stdin.end(input);
          }, i * 100);
        }),
    );
    await sleep(600);
    const { page, net } = await newPage();
    await page.goto(launchUrl());
    await retrying(page).waitFor({ timeout: 15000 });
    const banner = await statusText(page);
    check("[DIR-59 busy] a real busy store stalls the first snapshot past its deadline", /did not answer within 10 seconds/.test(banner), banner);
    db.exec("ROLLBACK");
    db.close();
    await Promise.all(writers);
    check("[DIR-59 busy] recovers by itself once the store frees up", await until(async () => (await connected(page).isVisible()) && (await shown(page, seededTitle)), 15000));
    check("[DIR-59 busy] the browser sent no write", net.writes().length === 0);
    await page.close();
  } catch (error) {
    check("[DIR-59 busy] scenario completed", false, error.message.split("\n")[0]);
  }

  // =================================================== retry now
  try {
    const { page, net } = await newPage();
    const down = net.fault("fail", "/api/command", (body) => body.op === "snapshot", { times: 1000 });
    await page.goto(launchUrl());
    await until(async () => /attempt 3/.test(await statusText(page)), 12000);
    down.times = 0;
    const clicked = Date.now();
    await retrying(page).getByRole("button", { name: "Retry now" }).click();
    await connected(page).waitFor();
    const took = Date.now() - clicked;
    check("[DIR-59 retry] Retry now loads at once instead of waiting for the next attempt", took < 2500, `${took} ms`);
    check("[DIR-59 retry] still no write", net.writes().length === 0);
    await page.close();
  } catch (error) {
    check("[DIR-59 retry] scenario completed", false, error.message.split("\n")[0]);
  }

  // ======================== a lost grant exchange is retried with that grant
  try {
    const { page, net } = await newPage();
    net.fault("fail", "/api/session");
    await page.goto(launchUrl());
    await connected(page).waitFor({ timeout: 10000 });
    const exchanges = net.sent.filter((entry) => entry.path === "/api/session");
    check("[DIR-59 grant] an exchange lost to an outage is retried with the same grant", exchanges.length === 2 && exchanges[0].grant === exchanges[1].grant, `${exchanges.length} exchanges`);
    await page.close();
  } catch (error) {
    check("[DIR-59 grant] scenario completed", false, error.message.split("\n")[0]);
  }

  // ============================== a used launch link is not retried
  try {
    const url = launchUrl();
    const first = await newPage();
    await first.page.goto(url);
    await connected(first.page).waitFor();
    const { page, net } = await newPage();
    await page.goto(url);
    await page.getByRole("alert").filter({ hasText: "already used" }).waitFor();
    const alert = await alertText(page);
    await sleep(3000);
    const exchanges = net.sent.filter((entry) => entry.path === "/api/session").length;
    const reads = net.sent.filter((entry) => entry.path === "/api/command").length;
    check("[DIR-59 session] a used launch link says so and asks for a fresh one", /already used/.test(alert) && /direct open/.test(alert), alert);
    check("[DIR-59 session] it is distinct from service reconnection", (await page.getByText("Browser session ended").isVisible()) && !(await retrying(page).isVisible()));
    check("[DIR-59 session] the grant is not retried and nothing is read", exchanges === 1 && reads === 0, `${exchanges} exchanges, ${reads} commands`);
    await page.screenshot({ path: join(evidence, "dir59-used-link.png") });
    await page.close();
    await first.page.close();
  } catch (error) {
    check("[DIR-59 session] scenario completed", false, error.message.split("\n")[0]);
  }

  // ============ a service restart ends the browser session: say so, stop
  try {
    const { page, net } = await newPage();
    await page.goto(launchUrl());
    await connected(page).waitFor();
    const before = snapshot().issues.length;
    await stop(service.child);
    service = await start();
    await page.getByRole("alert").filter({ hasText: "no longer valid" }).waitFor({ timeout: 10000 });
    const ended = Date.now();
    await sleep(3000);
    const after = net.sent.filter((entry) => entry.at > ended).length;
    check("[DIR-59 restart] an ended session is reported, not retried", after === 0 && (await page.getByText("Browser session ended").isVisible()), `${after} requests after the report`);
    const token = await page.evaluate(() => sessionStorage.getItem("direct.session"));
    check("[DIR-59 restart] the rejected session token is discarded", !token);
    await page.screenshot({ path: join(evidence, "dir59-session-ended.png") });
    const fresh = await newPage();
    await fresh.page.goto(launchUrl());
    await connected(fresh.page).waitFor();
    check("[DIR-59 restart] a fresh launch link loads the persisted workspace", (await shown(fresh.page, seededTitle)) && snapshot().issues.length === before);
    check("[DIR-59 restart] no browser write anywhere in the restart path", net.writes().length === 0 && fresh.net.writes().length === 0);
    await fresh.page.close();
    await page.close();
  } catch (error) {
    check("[DIR-59 restart] scenario completed", false, error.message.split("\n")[0]);
  }
  check("exactly one browser-created issue persisted", titled("Created after startup recovery") === 1);
  check("no uncaught page errors", pageErrors.length === 0, pageErrors.join(" | "));
} catch (error) {
  check("script completed", false, error.stack || error.message);
} finally {
  releaseStalls();
  await browser?.close().catch(() => {});
  if (service) await stop(service.child).catch(() => {});
  proxy.close();
  writeFileSync(join(evidence, "startup-results.json"), JSON.stringify(results, null, 2));
  const failed = results.filter((result) => !result.ok).length;
  console.log(`${results.length - failed}/${results.length} checks passed`);
  process.exit(failed ? 1 : 0);
}
