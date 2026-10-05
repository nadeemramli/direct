// Run with `npm test` (Node's built-in runner; types are stripped by Node).
import assert from "node:assert/strict";
import { test } from "node:test";
import { compareBuilds, factLine, gapClass } from "../src/delivery.ts";
import type { DeliveryFact } from "../src/api.ts";

const a = "a".repeat(40);
const b = "b".repeat(40);

test("build identity is honest about mismatch, dirt and unknown provenance", () => {
  assert.equal(compareBuilds({ commit: a, dirty: "false" }, { commit: a, dirty: "false" }, { commit: a, dirty: "false" }).state, "match");
  assert.equal(compareBuilds({ commit: a, dirty: "false" }, { commit: b, dirty: "false" }, null).state, "mismatch");
  assert.equal(compareBuilds({ commit: a, dirty: "false" }, { commit: a, dirty: "false" }, { commit: b, dirty: "false" }).state, "mismatch");
  assert.equal(compareBuilds({ commit: a, dirty: "true" }, { commit: a, dirty: "false" }, null).state, "dirty");
  assert.equal(compareBuilds({ commit: "unknown", dirty: "unknown" }, { commit: a, dirty: "false" }, null).state, "unknown");
  assert.equal(compareBuilds(null, null, null).state, "unknown");
});

test("facts describe themselves without implying later stages", () => {
  const fact = (detail: object) => ({ detail }) as DeliveryFact;
  assert.equal(factLine(fact({ kind: "push", sha: a, remote: "origin", remote_ref: "refs/heads/x" })), `${a.slice(0, 12)} → origin refs/heads/x`);
  assert.match(factLine(fact({ kind: "integration", method: "squash", sources: [a], result: b, target: "main" })), /^squash: a+ → b+ on main$/);
  assert.equal(gapClass("delivered"), "cached");
  assert.equal(gapClass("drifted"), "unavailable");
  assert.equal(gapClass("unmerged"), "stale");
});
