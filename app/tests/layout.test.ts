// Run with `npm test` (Node's built-in runner; types are stripped by Node).
import assert from "node:assert/strict";
import { test } from "node:test";
import { defaultLayout, liveCollapsedSections, parseLayout, serializeLayout } from "../src/layout.ts";

test("collapsed product sections round-trip through stored layout", () => {
  const layout = defaultLayout();
  layout.collapsedProductSections = ["s1", "s2"];
  const parsed = parseLayout(serializeLayout(layout));
  assert.deepEqual(parsed.collapsedProductSections, ["s1", "s2"]);
});

test("older stored layouts and bad values fall back to nothing collapsed", () => {
  const older = JSON.stringify({ version: 1, left: {}, right: {}, sections: { products: true } });
  const parsed = parseLayout(older);
  assert.deepEqual(parsed.collapsedProductSections, []);
  assert.equal(parsed.collapsed.products, true);
  const bad = JSON.stringify({ version: 1, productSections: ["ok", 7, "", "x".repeat(65), "ok", null] });
  assert.deepEqual(parseLayout(bad).collapsedProductSections, ["ok"]);
  assert.deepEqual(parseLayout(JSON.stringify({ version: 1, productSections: "s1" })).collapsedProductSections, []);
  assert.deepEqual(parseLayout("not json").collapsedProductSections, []);
});

test("collapsed state forgets deleted sections and keeps renamed ones", () => {
  assert.deepEqual(liveCollapsedSections(["a", "gone", "b"], ["a", "b", "c"]), ["a", "b"]);
  assert.deepEqual(liveCollapsedSections([], ["a"]), []);
});
