// Run with `npm test` (Node's built-in runner; types are stripped by Node).
import assert from "node:assert/strict";
import { test } from "node:test";
import type { Product, ProductSection } from "../src/api.ts";
import {
  applyArrangement,
  arrangement,
  moveProduct,
  moveSection,
  nudgeProduct,
  nudgeSection,
  sidebarGroups,
} from "../src/sidebar.ts";

function product(id: string, sort_order = 0, section_id: string | null = null): Product {
  return {
    id,
    key: id.toUpperCase(),
    name: id,
    repo_windows: "",
    repo_wsl: "",
    vault_windows: "",
    vault_wsl: "",
    sort_order,
    section_id,
  };
}
function section(id: string, sort_order: number): ProductSection {
  return { id, name: id, sort_order, version: 1, created_at: 0, updated_at: 0 };
}
const keys = (groups: ReturnType<typeof sidebarGroups>) =>
  groups.map((g) => `${g.section?.id ?? "-"}:${g.products.map((p) => p.id).join(",")}`);

test("legacy products without order keep the service's ID order", () => {
  const groups = sidebarGroups([product("c"), product("a"), product("b")]);
  assert.deepEqual(keys(groups), ["-:a,b,c"]);
});

test("groups list ungrouped first, then sections by order; unknown sections fall back", () => {
  const groups = sidebarGroups(
    [product("a", 2, "s2"), product("b", 1, "s1"), product("c", 0), product("d", 3, "gone")],
    [section("s1", 1), section("s2", 0)],
  );
  assert.deepEqual(keys(groups), ["-:c,d", "s2:a", "s1:b"]);
});

test("moving a product into, within and out of a section", () => {
  const start = sidebarGroups([product("a", 0), product("b", 1), product("c", 2)], [section("ter", 0)]);
  const into = moveProduct(start, "b", "ter", null)!;
  assert.deepEqual(keys(into), ["-:a,c", "ter:b"]);
  const before = moveProduct(into, "c", "ter", "b")!;
  assert.deepEqual(keys(before), ["-:a", "ter:c,b"]);
  const out = moveProduct(before, "b", null, "a")!;
  assert.deepEqual(keys(out), ["-:b,a", "ter:c"]);
  assert.deepEqual(arrangement(out), {
    sections: ["ter"],
    products: [
      { product_id: "b", section_id: null },
      { product_id: "a", section_id: null },
      { product_id: "c", section_id: "ter" },
    ],
  });
  // A no-op or unknown reference sends nothing.
  assert.equal(moveProduct(out, "b", null, "a"), null);
  assert.equal(moveProduct(out, "b", null, "b"), null);
  assert.equal(moveProduct(out, "b", "missing", null), null);
  assert.equal(moveProduct(out, "zzz", null, null), null);
});

test("keyboard nudges cross group edges so every placement is reachable", () => {
  let groups = sidebarGroups([product("a", 0), product("b", 1, "s"), product("c", 2, "t")], [
    section("s", 0),
    section("t", 1),
  ]);
  groups = nudgeProduct(groups, "a", 1)!;
  assert.deepEqual(keys(groups), ["-:", "s:a,b", "t:c"]);
  groups = nudgeProduct(groups, "a", 1)!;
  assert.deepEqual(keys(groups), ["-:", "s:b,a", "t:c"]);
  groups = nudgeProduct(groups, "a", 1)!;
  assert.deepEqual(keys(groups), ["-:", "s:b", "t:a,c"]);
  groups = nudgeProduct(groups, "c", 1) ?? groups; // already last
  assert.equal(nudgeProduct(groups, "c", 1), null);
  groups = nudgeProduct(groups, "a", -1)!;
  assert.deepEqual(keys(groups), ["-:", "s:b,a", "t:c"]);
  groups = nudgeProduct(groups, "b", -1)!;
  assert.deepEqual(keys(groups), ["-:b", "s:a", "t:c"]);
  assert.equal(nudgeProduct(groups, "b", -1), null);
});

test("sections reorder but the ungrouped list stays first", () => {
  const groups = sidebarGroups([product("a", 0, "s"), product("b", 1, "t")], [section("s", 0), section("t", 1)]);
  assert.deepEqual(keys(moveSection(groups, "t", "s")!), ["-:", "t:b", "s:a"]);
  assert.deepEqual(keys(nudgeSection(groups, "s", 1)!), ["-:", "t:b", "s:a"]);
  assert.equal(nudgeSection(groups, "s", -1), null);
  assert.equal(nudgeSection(groups, "t", 1), null);
  assert.equal(moveSection(groups, "s", "s"), null);
});

test("an accepted arrangement updates local records for immediate feedback", () => {
  const products = [product("a", 0), product("b", 1)];
  const sections = [section("s", 3)];
  const next = arrangement(moveProduct(sidebarGroups(products, sections), "a", "s", null)!);
  const local = applyArrangement(products, sections, next);
  assert.deepEqual(
    local.products.map((p) => [p.id, p.sort_order, p.section_id]),
    [
      ["a", 1, "s"],
      ["b", 0, null],
    ],
  );
  assert.equal(local.sections[0].sort_order, 0);
  assert.deepEqual(keys(sidebarGroups(local.products, local.sections)), ["-:b", "s:a"]);
});
