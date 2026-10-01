#!/usr/bin/env node
// Browser E2E for DIR-53 (bounded, scrollable panes with many products and
// projects), DIR-37 (resizable/expandable detail sidebar) and DIR-40 (return
// from Theoria guidance to the originating issue). Synthetic data only: the
// script creates a fresh isolated workspace, seeds it through the real
// service (owner HTTP session + agent CLI), and drives the built UI.
//
// Setup (from the repository root):
//   cargo build -p direct && (cd app && npm run build)
// Run (Playwright must be resolvable; PLAYWRIGHT_MODULE may point at it):
//   node scripts/e2e/navigation.browser.mjs target/debug/direct <new-workspace-dir> app/dist <evidence-dir>

import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || "playwright");
const [binary, root, assets, evidence] = process.argv.slice(2).map((value) => resolve(value));
if (!evidence) {
  console.error("usage: navigation.browser.mjs <direct-binary> <new-workspace-dir> <assets-dir> <evidence-dir>");
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
const LAYOUT_KEY = "direct.layout.v1";

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

// ---------------------------------------------------------------- fixture
const sentence =
  "This deliberately long synthetic paragraph exercises comfortable reading of issue content in the detail sidebar, including unbroken-identifiers-like-this-one-that-should-wrap-without-pushing-the-layout-sideways and ordinary prose that keeps going for a while.";
const longBody = Array.from({ length: 36 }, (_, n) => `Paragraph ${n + 1}. ${sentence} ${sentence}`).join("\n\n");
const longAcceptance = Array.from({ length: 12 }, (_, n) => `- Criterion ${n + 1}: ${sentence}`).join("\n");
const theoriaRoot = join(root, "theoria-source");
const catalogPath = join(root, "theoria-catalog.json");
function writeTheoria() {
  mkdirSync(theoriaRoot, { recursive: true });
  const documents = [];
  for (let n = 1; n <= 6; n += 1) {
    const id = `synthetic-guide-${n}`;
    writeFileSync(
      join(theoriaRoot, `${id}.md`),
      `# Synthetic guide ${n}\n\n${Array.from({ length: 30 }, () => sentence).join("\n\n")}\n`,
    );
    documents.push({
      id,
      title: `Synthetic guide ${n} with a reasonably long descriptive title`,
      description: `Synthetic guidance ${n}. ${sentence}`,
      category: n % 2 ? "workflow" : "verification",
      path: `${id}.md`,
    });
  }
  // A catalog entry whose source file does not exist becomes "unavailable".
  documents.push({
    id: "synthetic-missing",
    title: "Synthetic guidance whose source is missing",
    description: "Exercises the unavailable/missing guidance path.",
    category: "playbook",
    path: "does-not-exist.md",
  });
  writeFileSync(catalogPath, JSON.stringify({ version: 1, documents }, null, 2));
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

const PRODUCT_KEYS = ["ALPHA", "BRAVO", "CHARLIE", "DELTA", "ECHO", "FOXTROT", "GOLF", "HOTEL", "INDIA", "JULIET", "KILO", "LIMA"];
async function seed(page) {
  for (const [n, key] of PRODUCT_KEYS.entries())
    await owner(page, {
      op: "create_product",
      key,
      name: `${key[0]}${key.slice(1).toLowerCase()} product ${n + 1} — customer lifecycle analytics and onboarding platform with a long name`,
    });
  const products = ["DIR", ...PRODUCT_KEYS]; // 13 products in total
  const projects = [];
  for (let n = 0; n < 32; n += 1) {
    const product = n < 16 ? "DIR" : products[1 + (n % PRODUCT_KEYS.length)];
    const project = await owner(page, {
      op: "create_project",
      product,
      name: `Project ${String(n + 1).padStart(2, "0")} — ${product} migration, reporting and navigation hardening with a very long name`,
      description: sentence,
      priority: ["urgent", "high", "medium", "low"][n % 4],
      sort_order: n,
    });
    projects.push({ ...project, product });
    await owner(page, {
      op: "create_issue",
      product,
      title: `Seed issue for project ${n + 1}: ${sentence.slice(0, 90)}`,
      body: sentence,
      planning_scope: "project",
      project_id: project.id,
    });
  }
  const target = projects[5];
  const long = await owner(page, {
    op: "create_issue",
    product: "DIR",
    title: `Long issue for reading: ${sentence.slice(0, 120)}`,
    body: longBody,
    acceptance: longAcceptance,
    planning_scope: "project",
    project_id: target.id,
  });
  execFileSync(binary, [
    "--data-dir", workspace, "--actor", "e2e-seed", "theoria-sync",
    "--root", theoriaRoot, "--catalog", catalogPath, "--product", "DIR", "--request-id", "e2e-theoria-sync-1",
  ]);
  let version = long.version;
  for (const id of ["synthetic-guide-1", "synthetic-guide-2", "synthetic-guide-3", "synthetic-guide-4", "synthetic-guide-5", "synthetic-guide-6", "synthetic-missing"]) {
    const updated = await owner(page, { op: "link_theoria", key: long.key, expected_version: version, document_id: id, playbook_version: null });
    version = updated.version;
  }
  const snapshot = await owner(page, { op: "snapshot" });
  const dirName = snapshot.products.find((product) => product.key === "DIR").name;
  return { products, projects, target, longKey: long.key, dirName };
}

// ---------------------------------------------------------------- probes
const pageOverflow = (page) =>
  page.evaluate(() => {
    const d = document.documentElement;
    return { sw: d.scrollWidth, cw: d.clientWidth, sh: d.scrollHeight, ch: d.clientHeight, sx: scrollX, sy: scrollY };
  });
const noPageOverflow = (o) => o.sw <= o.cw && o.sh <= o.ch && o.sx === 0 && o.sy === 0;
// Scrolls an element into view inside its own pane and reports whether it is
// then fully inside the viewport and actually hit-testable (not covered).
async function reachable(locator) {
  await locator.evaluate((el) => el.scrollIntoView({ block: "center", inline: "nearest" }));
  return locator.evaluate((el) => {
    const r = el.getBoundingClientRect();
    const inside = r.width > 0 && r.height > 0 && r.top >= 0 && r.left >= 0 && r.bottom <= innerHeight && r.right <= innerWidth;
    const hit = document.elementFromPoint(r.left + Math.min(r.width / 2, 20), r.top + r.height / 2);
    return inside && !!hit && (hit === el || el.contains(hit)) && scrollX === 0 && scrollY === 0;
  });
}
const width = (locator) => locator.evaluate((el) => Math.round(el.getBoundingClientRect().width));
async function drag(page, handle, dx) {
  const box = await handle.boundingBox();
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx / 2, y, { steps: 4 });
  await page.mouse.move(x + dx, y, { steps: 4 });
  await page.mouse.up();
}
// The guidance card that contains the detail body's top edge, and how far
// into it the top edge falls. Used to judge scroll restoration even when a
// resize reflows the text.
const detailAnchor = (page) =>
  page.locator("aside.detail-panel .detail-body").evaluate((body) => {
    const top = body.getBoundingClientRect().top + 1;
    const cards = [...body.querySelectorAll(".guidance-reference, .finding-card, .theoria-card")];
    const index = cards.findIndex((card) => {
      const r = card.getBoundingClientRect();
      return r.top <= top && r.bottom > top && card.classList.contains("guidance-reference");
    });
    const card = cards[index];
    const r = card?.getBoundingClientRect();
    return { index, fraction: card ? (top - r.top) / r.height : null, scrollTop: Math.round(body.scrollTop) };
  });

let service;
let browser;
const pageErrors = [];
try {
  writeTheoria();
  service = await start();
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const context = await browser.newContext({ viewport: { width: 1366, height: 768 } });
  const page = await context.newPage();
  page.setDefaultTimeout(8000);
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.goto(service.url);
  await page.getByText("Connected locally").waitFor();
  const fixture = await seed(page);
  await page.locator("aside.sidebar nav[aria-label='Products'] button", { hasText: "Lima product 12" }).waitFor({ timeout: 10000 });
  check("fixture seeded through the real service", true,
    `13 products, ${fixture.projects.length} projects, ${fixture.projects.length + 1} issues, 7 guidance documents (1 unavailable), long issue ${fixture.longKey}`);

  // ============================================================ DIR-53
  for (const viewport of [{ width: 1366, height: 768 }, { width: 1024, height: 640 }, { width: 1920, height: 1080 }]) {
    const label = `${viewport.width}x${viewport.height}`;
    try {
      await page.setViewportSize(viewport);
      await page.getByRole("button", { name: /All work/ }).click();
      await sleep(150);
      check(`[DIR-53 ${label}] no page overflow with 13 products in the sidebar`, noPageOverflow(await pageOverflow(page)), JSON.stringify(await pageOverflow(page)));
      const sidebarButtons = page.locator("aside.sidebar nav[aria-label='Products'] button");
      const count = await sidebarButtons.count();
      let allProducts = count === 13;
      for (let n = 0; n < count; n += 1) allProducts = (await reachable(sidebarButtons.nth(n))) && allProducts;
      check(`[DIR-53 ${label}] every product row is reachable inside the sidebar`, allProducts, `${count} rows`);
      check(`[DIR-53 ${label}] sidebar footer action stays reachable`, await reachable(page.getByRole("button", { name: /Export workspace/ })));
      await sidebarButtons.filter({ hasText: "Lima product 12" }).click();
      check(`[DIR-53 ${label}] a product far down the list opens`, (await page.locator("main .page-heading h1").innerText()).startsWith("Lima product 12"));
      await page.getByRole("button", { name: /All work/ }).click();
      await page.getByRole("button", { name: "By project" }).click();
      const groups = page.locator(".issue-group > header");
      const groupCount = await groups.count();
      let allGroups = groupCount === 32;
      for (let n = 0; n < groupCount; n += 1) allGroups = (await reachable(groups.nth(n))) && allGroups;
      check(`[DIR-53 ${label}] all 32 project groups are reachable in the list`, allGroups, `${groupCount} groups`);
      const lastRow = page.locator(".issue-row").last();
      check(`[DIR-53 ${label}] last issue row is reachable`, await reachable(lastRow));
      await page.getByRole("button", { name: "Flat" }).click();
      await page.getByLabel("Search issues").fill(fixture.longKey);
      await page.getByLabel("Search issues").press("Enter");
      await page.getByLabel("Issue detail").getByRole("tab", { name: "Brief" }).waitFor();
      check(`[DIR-53 ${label}] center actions stay usable with an issue open`, (await reachable(page.getByRole("button", { name: "＋ Issue" }))) && (await reachable(page.getByLabel("Search issues"))));
      check(`[DIR-53 ${label}] detail actions stay visible`, (await reachable(page.getByRole("button", { name: "Close issue" }))) && (await reachable(page.getByRole("tab", { name: "Activity" }))));
      const lastDetail = page.locator("aside.detail-panel .detail-body > *").last();
      check(`[DIR-53 ${label}] end of long issue content is reachable`, await reachable(lastDetail));
      check(`[DIR-53 ${label}] still no page overflow with long issue open`, noPageOverflow(await pageOverflow(page)), JSON.stringify(await pageOverflow(page)));
      await page.screenshot({ path: join(evidence, `dir53-${label}.png`) });
      await page.getByLabel("Search issues").fill("");
      await page.getByRole("button", { name: "Close issue" }).click();
    } catch (error) {
      check(`[DIR-53 ${label}] scenario completed`, false, error.message.split("\n")[0]);
      await page.screenshot({ path: join(evidence, `dir53-${label}-failure.png`) }).catch(() => {});
      await page.getByLabel("Search issues").fill("").catch(() => {});
    }
  }

  // ============================================== DIR-53 left sidebar layout
  try {
    await page.setViewportSize({ width: 1366, height: 768 });
    const sidebar = page.locator("aside.sidebar");
    const handle = page.getByRole("separator", { name: "Resize sidebar" });
    const before = await width(sidebar);
    await drag(page, handle, 80);
    const wider = await width(sidebar);
    check("[DIR-53 left] dragging the handle widens the sidebar", Math.abs(wider - before - 80) <= 3, `${before} → ${wider}`);
    await drag(page, handle, 900);
    const max = Number(await handle.getAttribute("aria-valuemax"));
    check("[DIR-53 left] width is clamped to the maximum", (await width(sidebar)) === max, `${await width(sidebar)} max ${max}`);
    await drag(page, handle, -900);
    const min = Number(await handle.getAttribute("aria-valuemin"));
    check("[DIR-53 left] width is clamped to the minimum", (await width(sidebar)) === min, `${await width(sidebar)} min ${min}`);
    await handle.focus();
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowRight");
    check("[DIR-53 left] keyboard arrows resize", (await width(sidebar)) === min + 32 && Number(await handle.getAttribute("aria-valuenow")) === min + 32, `${await width(sidebar)}`);
    await page.keyboard.press("End");
    check("[DIR-53 left] End jumps to maximum", (await width(sidebar)) === max);
    await page.keyboard.press("Home");
    await page.keyboard.press("ArrowRight");
    const chosen = await width(sidebar);
    await page.getByRole("button", { name: "Collapse sidebar" }).click();
    const collapsed = await width(sidebar);
    check("[DIR-53 left] collapse leaves a narrow rail", collapsed < 80, `${collapsed}`);
    await page.locator("aside.sidebar nav[aria-label='Products'] button", { hasText: "Charlie product 3" }).click();
    check("[DIR-53 left] navigation still works while collapsed", (await page.locator("main .page-heading h1").innerText()).startsWith("Charlie product 3"));
    check("[DIR-53 left] no page overflow while collapsed", noPageOverflow(await pageOverflow(page)));
    await page.screenshot({ path: join(evidence, "dir53-left-collapsed.png") });
    await page.reload();
    await page.getByText("Connected locally").waitFor({ state: "attached" });
    check("[DIR-53 left] collapsed state persists across reload", (await width(page.locator("aside.sidebar"))) === collapsed);
    await page.getByRole("button", { name: "Expand sidebar" }).click();
    check("[DIR-53 left] expand restores the previous width", (await width(page.locator("aside.sidebar"))) === chosen, `${await width(page.locator("aside.sidebar"))} vs ${chosen}`);
    const sectionToggle = page.getByRole("button", { name: /^PRODUCTS/ });
    await sectionToggle.click();
    check("[DIR-53 left] products section collapses", (await sectionToggle.getAttribute("aria-expanded")) === "false" && (await page.locator("aside.sidebar nav[aria-label='Products'] button").count()) === 0);
    await page.reload();
    await page.getByText("Connected locally").waitFor();
    check("[DIR-53 left] width and section state persist across reload",
      (await width(page.locator("aside.sidebar"))) === chosen && (await page.getByRole("button", { name: /^PRODUCTS/ }).getAttribute("aria-expanded")) === "false");
    await page.getByRole("button", { name: /^PRODUCTS/ }).click();
    await page.getByRole("separator", { name: "Resize sidebar" }).dblclick();
    check("[DIR-53 left] double-click resets to the default width", (await width(page.locator("aside.sidebar"))) === 224, `${await width(page.locator("aside.sidebar"))}`);
  } catch (error) {
    check("[DIR-53 left] sidebar layout scenario completed", false, error.message.split("\n")[0]);
    await page.screenshot({ path: join(evidence, "dir53-left-failure.png") }).catch(() => {});
  }

  // ======================================= DIR-53 corrupt/stale stored layout
  for (const [name, stored] of [
    ["corrupt JSON", "{not json"],
    ["stale out-of-range values", JSON.stringify({ version: 1, left: { width: 99999, collapsed: "yes" }, right: { width: -5, expanded: 3 }, sections: { products: "no" } })],
    ["unknown version", JSON.stringify({ version: 99, left: { width: 300 } })],
  ]) {
    try {
      await page.evaluate(([key, value]) => localStorage.setItem(key, value), [LAYOUT_KEY, stored]);
      await page.reload();
      await page.getByText("Connected locally").waitFor();
      const left = await width(page.locator("aside.sidebar"));
      const leftMax = Number(await page.getByRole("separator", { name: "Resize sidebar" }).getAttribute("aria-valuemax"));
      check(`[DIR-53 storage] ${name} recovers to a usable layout`,
        left >= 168 && left <= leftMax && noPageOverflow(await pageOverflow(page)) && (await page.locator("aside.sidebar nav[aria-label='Products'] button").count()) === 13,
        `left ${left}`);
    } catch (error) {
      check(`[DIR-53 storage] ${name} recovers to a usable layout`, false, error.message.split("\n")[0]);
    }
  }
  await page.evaluate((key) => localStorage.removeItem(key), LAYOUT_KEY);
  await page.reload();
  await page.getByText("Connected locally").waitFor();

  // ============================================================ DIR-37
  try {
    await page.setViewportSize({ width: 1366, height: 768 });
    await page.getByLabel("Search issues").fill(fixture.longKey);
    await page.getByLabel("Search issues").press("Enter");
    const detail = page.getByLabel("Issue detail");
    await detail.getByRole("tab", { name: "Brief" }).waitFor();
    const handle = page.getByRole("separator", { name: "Resize detail panel" });
    const initial = await width(detail);
    await drag(page, handle, -120);
    const resized = await width(detail);
    check("[DIR-37] dragging the detail handle widens the panel", Math.abs(resized - initial - 120) <= 3, `${initial} → ${resized}`);
    await handle.focus();
    await page.keyboard.press("ArrowLeft");
    check("[DIR-37] keyboard resizes the detail panel", (await width(detail)) === resized + 16, `${await width(detail)}`);
    await page.keyboard.press("ArrowRight");
    const previous = await width(detail);
    const expand = page.getByRole("button", { name: "Expand detail panel" });
    await expand.click();
    const expanded = await width(detail);
    const listWidth = await width(page.locator("section.list-panel"));
    check("[DIR-37] expand gives a wide reading pane", expanded >= 700 && listWidth >= 260, `detail ${expanded}, list ${listWidth}`);
    check("[DIR-37] actions stay visible while expanded",
      (await reachable(page.getByRole("button", { name: "Close issue" }))) &&
        (await reachable(page.getByRole("button", { name: "Restore detail panel width" }))) &&
        (await reachable(detail.getByRole("tab", { name: "Verification" }))) &&
        (await reachable(page.getByRole("button", { name: "＋ Issue" }))));
    const lineLength = await page.locator("aside.detail-panel .detail-body").evaluate((body) => {
      const paragraph = [...body.querySelectorAll("p, .prose, pre")].find((el) => el.textContent.includes("Paragraph 3."));
      return paragraph ? Math.round(paragraph.getBoundingClientRect().width) : 0;
    });
    check("[DIR-37] long paragraphs use the expanded width", lineLength >= 560, `${lineLength}px text column`);
    check("[DIR-37] no page overflow while expanded", noPageOverflow(await pageOverflow(page)));
    await page.screenshot({ path: join(evidence, "dir37-expanded.png") });
    await page.reload();
    await page.getByText("Connected locally").waitFor();
    await page.getByLabel("Search issues").fill(fixture.longKey);
    await page.getByLabel("Search issues").press("Enter");
    await detail.getByRole("tab", { name: "Brief" }).waitFor();
    check("[DIR-37] expanded state persists across reload", (await width(detail)) === expanded, `${await width(detail)} vs ${expanded}`);
    await page.getByRole("button", { name: "Restore detail panel width" }).click();
    check("[DIR-37] restore returns to the previous size", (await width(detail)) === previous, `${await width(detail)} vs ${previous}`);
    await drag(page, handle, 2000);
    const rmin = Number(await handle.getAttribute("aria-valuemin"));
    check("[DIR-37] detail width is clamped to the minimum", (await width(detail)) === rmin, `${await width(detail)} min ${rmin}`);
    await drag(page, handle, -3000);
    const rmax = Number(await handle.getAttribute("aria-valuemax"));
    check("[DIR-37] detail width is clamped to the maximum and the list stays usable",
      (await width(detail)) === rmax && (await width(page.locator("section.list-panel"))) >= 260, `${await width(detail)} max ${rmax}`);
    await page.setViewportSize({ width: 1024, height: 640 });
    await sleep(150);
    check("[DIR-37] shrinking the window re-clamps the panel without page overflow",
      noPageOverflow(await pageOverflow(page)) && (await width(page.locator("section.list-panel"))) >= 260 && (await reachable(page.getByRole("button", { name: "Close issue" }))),
      `detail ${await width(detail)}, list ${await width(page.locator("section.list-panel"))}`);
    await page.screenshot({ path: join(evidence, "dir37-narrow-clamped.png") });
    await page.setViewportSize({ width: 1366, height: 768 });
    await handle.dblclick();
    await page.getByRole("button", { name: "Close issue" }).click();
    await page.getByLabel("Search issues").fill("");
  } catch (error) {
    check("[DIR-37] detail sidebar scenario completed", false, error.message.split("\n")[0]);
    await page.screenshot({ path: join(evidence, "dir37-failure.png") }).catch(() => {});
    await page.getByLabel("Search issues").fill("").catch(() => {});
  }

  // ============================================================ DIR-40
  try {
    await page.setViewportSize({ width: 1366, height: 768 });
    await page.locator("aside.sidebar nav[aria-label='Products'] button", { hasText: fixture.dirName }).click();
    const productName = await page.locator("main .page-heading h1").innerText();
    await page.getByLabel("Filter by project").selectOption(fixture.target.id);
    const listPanel = page.locator("section.list-panel");
    const row = page.locator(".issue-row", { hasText: fixture.longKey });
    await row.click();
    const detail = page.getByLabel("Issue detail");
    await detail.getByRole("tab", { name: "Theoria" }).click();
    const body = page.locator("aside.detail-panel .detail-body");
    for (let round = 1; round <= 3; round += 1) {
      // Put card N's "Open guidance" button mid-pane so the click itself does
      // not move the scroll position being measured.
      const opener = page.locator(".guidance-reference").nth(round).getByRole("button", { name: "Open guidance →" });
      await opener.evaluate((button) => button.scrollIntoView({ block: "center" }));
      await sleep(100);
      const anchor = await detailAnchor(page);
      await opener.click();
      check(`[DIR-40 round ${round}] opened guidance from a scrolled position`, anchor.scrollTop > 0 && anchor.index >= 0, JSON.stringify(anchor));
      await page.getByLabel("Theoria guidance detail").waitFor();
      const back = page.getByRole("button", { name: new RegExp(`Back to ${fixture.longKey}`) });
      check(`[DIR-40 round ${round}] guidance opens with a Back control`, await back.isVisible());
      if (round === 3) {
        await page.setViewportSize({ width: 1100, height: 700 });
        await sleep(150);
      }
      await back.click();
      await detail.getByRole("tab", { name: "Theoria" }).waitFor();
      await sleep(250);
      const restored = await detailAnchor(page);
      check(`[DIR-40 round ${round}] same issue, product and project filter`,
        (await page.locator("aside.detail-panel .detail-top").innerText()).includes(fixture.longKey) &&
          (await page.locator("main .page-heading h1").innerText()).startsWith(productName) &&
          (await page.getByLabel("Filter by project").inputValue()) === fixture.target.id,
        await page.locator("main .page-heading h1").innerText());
      check(`[DIR-40 round ${round}] Theoria tab is restored`, (await detail.getByRole("tab", { name: "Theoria" }).getAttribute("aria-selected")) === "true");
      check(`[DIR-40 round ${round}${round === 3 ? ", after window resize" : ""}] detail scroll position is restored`,
        restored.index === anchor.index && (round === 3 ? Math.abs(restored.fraction - anchor.fraction) < 0.2 : Math.abs(restored.scrollTop - anchor.scrollTop) <= 2),
        `before ${JSON.stringify(anchor)} after ${JSON.stringify(restored)}`);
      check(`[DIR-40 round ${round}] selected row is visible in the list`, await row.evaluate((el) => {
        const r = el.getBoundingClientRect();
        const p = el.closest(".list-panel").getBoundingClientRect();
        return r.top >= p.top && r.bottom <= p.bottom;
      }));
      if (round === 3) await page.screenshot({ path: join(evidence, "dir40-restored-after-resize.png") });
    }
    await page.setViewportSize({ width: 1366, height: 768 });
    // List scroll restoration with many rows: All work, flat list, scrolled.
    await page.getByLabel("Filter by project").selectOption("all");
    await listPanel.evaluate((el) => (el.scrollTop = el.scrollHeight));
    await sleep(100);
    const listBefore = await listPanel.evaluate((el) => Math.round(el.scrollTop));
    await page.locator(".issue-row").last().click();
    await page.getByLabel("Search issues").fill("");
    await page.locator(".issue-row", { hasText: fixture.longKey }).click();
    await detail.getByRole("tab", { name: "Theoria" }).click();
    await listPanel.evaluate((el, top) => (el.scrollTop = top), listBefore);
    await sleep(100);
    const listAt = await listPanel.evaluate((el) => Math.round(el.scrollTop));
    await page.locator(".guidance-reference").first().getByRole("button", { name: "Open guidance →" }).click();
    await page.getByRole("button", { name: new RegExp(`Back to ${fixture.longKey}`) }).click();
    await detail.getByRole("tab", { name: "Theoria" }).waitFor();
    await sleep(250);
    check("[DIR-40] list scroll position is restored", Math.abs((await listPanel.evaluate((el) => Math.round(el.scrollTop))) - listAt) <= 2,
      `${listAt} → ${await listPanel.evaluate((el) => Math.round(el.scrollTop))}`);
    // Missing/unavailable guidance.
    const missing = page.locator(".guidance-reference", { hasText: "Synthetic guidance whose source is missing" });
    await missing.getByRole("button", { name: "Open guidance →" }).click();
    const theoriaDetail = page.getByLabel("Theoria guidance detail");
    await theoriaDetail.waitFor();
    check("[DIR-40 missing] unavailable guidance is labelled, not substituted",
      (await theoriaDetail.innerText()).includes("Synthetic guidance whose source is missing") && (await theoriaDetail.innerText()).includes("Authoritative source unavailable"));
    await page.screenshot({ path: join(evidence, "dir40-missing-guidance.png") });
    await page.getByRole("button", { name: new RegExp(`Back to ${fixture.longKey}`) }).click();
    await detail.getByRole("tab", { name: "Theoria" }).waitFor();
    check("[DIR-40 missing] Back returns to the issue's Theoria tab",
      (await page.locator("aside.detail-panel .detail-top").innerText()).includes(fixture.longKey) &&
        (await detail.getByRole("tab", { name: "Theoria" }).getAttribute("aria-selected")) === "true");
    // Sidebar navigation into Theoria starts fresh: no stale Back control.
    await page.getByRole("button", { name: /Guidance & findings/ }).click();
    check("[DIR-40] Theoria opened from the sidebar shows no stale Back control",
      (await page.getByRole("button", { name: /Back to / }).count()) === 0);
  } catch (error) {
    check("[DIR-40] return-to-issue scenario completed", false, error.message.split("\n")[0]);
    await page.screenshot({ path: join(evidence, "dir40-failure.png") }).catch(() => {});
  }

  // Persistence across a real service restart (fresh session).
  try {
    await stop(service.child);
    service = await start();
    await page.goto(service.url);
    await page.getByText("Connected locally").waitFor();
    check("[restart] workspace data and layout survive a service restart",
      (await page.locator("aside.sidebar nav[aria-label='Products'] button").count()) === 13 && noPageOverflow(await pageOverflow(page)));
  } catch (error) {
    check("[restart] service restart scenario completed", false, error.message.split("\n")[0]);
  }
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
