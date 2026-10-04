// Run with `npm test` (Node's built-in runner; types are stripped by Node).
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  relationCandidates,
  relationProblem,
  missingTargets,
  relationsForProduct,
} from "../src/relations.ts";

const issues = [
  { key: "DIR-1", title: "Parent epic", product_id: "dir" },
  { key: "DIR-2", title: "Blocking fix", product_id: "dir" },
  { key: "DIR-3", title: "Verify DIR-2", product_id: "dir", parent: "DIR-2" },
  { key: "ALT-1", title: "Parent elsewhere", product_id: "alt" },
];

test("candidates are same-product work issues matching key or title", () => {
  assert.deepEqual(
    relationCandidates(issues, "dir", "").map((issue) => issue.key),
    ["DIR-1", "DIR-2"],
  );
  assert.deepEqual(
    relationCandidates(issues, "dir", "parent").map((issue) => issue.key),
    ["DIR-1"],
  );
  assert.deepEqual(
    relationCandidates(issues, "dir", " dir-2 ").map((issue) => issue.key),
    ["DIR-2"],
  );
  assert.deepEqual(relationCandidates(issues, "alt", "DIR").length, 0);
});

test("duplicates and a second parent are caught before saving", () => {
  const pending = [{ target_key: "DIR-1", kind: "parent" as const }];
  assert.equal(relationProblem(pending, { target_key: "", kind: "related" }), "Choose an issue to relate.");
  assert.match(relationProblem(pending, { target_key: "DIR-1", kind: "parent" }), /already/);
  assert.match(relationProblem(pending, { target_key: "DIR-2", kind: "parent" }), /only one parent/);
  assert.equal(relationProblem(pending, { target_key: "DIR-1", kind: "related" }), "");
  assert.equal(relationProblem(pending, { target_key: "DIR-2", kind: "blocked_by" }), "");
});

test("changing product drops relations to the old product", () => {
  const pending = [
    { target_key: "DIR-1", kind: "parent" as const },
    { target_key: "ALT-1", kind: "related" as const },
  ];
  assert.deepEqual(relationsForProduct(pending, issues, "alt"), {
    kept: [{ target_key: "ALT-1", kind: "related" }],
    dropped: [{ target_key: "DIR-1", kind: "parent" }],
  });
});

test("a deleted target stays in the draft and is reported, not silently dropped", () => {
  const pending = [
    { target_key: "DIR-1", kind: "parent" as const },
    { target_key: "DIR-9", kind: "related" as const },
  ];
  assert.deepEqual(relationsForProduct(pending, issues, "dir"), { kept: pending, dropped: [] });
  assert.deepEqual(missingTargets(pending, issues), ["DIR-9"]);
  assert.deepEqual(missingTargets([{ target_key: "DIR-2", kind: "blocked_by" }], issues), []);
});
