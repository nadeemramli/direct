// Run with `npm test` (Node's built-in runner; types are stripped by Node).
import assert from "node:assert/strict";
import { test } from "node:test";
import { orderedFindings, outcomeCounts, queueRuns, reviewable } from "../src/planner.ts";
import type { AgentRun, Issue, PlannerFinding } from "../src/api.ts";

const f = (id: string, updated_at: number, escalation: unknown = null, confirmation: unknown = null) =>
  ({ id, updated_at, escalation, confirmation }) as PlannerFinding;

test("unconfirmed escalations come first", () => {
  const ordered = orderedFindings([f("a", 3), f("b", 1, { criterion: "x" }), f("c", 2, { criterion: "x" }, { confirmed: true })]);
  assert.deepEqual(ordered.map((x) => x.id), ["b", "a", "c"]);
});

test("reviewable issues are the product's parents, open work first", () => {
  const issues = [
    { key: "DIR-3", product_id: "p", parent: null, status: "done" },
    { key: "DIR-10", product_id: "p", parent: null, status: "backlog" },
    { key: "DIR-2", product_id: "p", parent: "DIR-10", status: "ready" },
    { key: "ALT-1", product_id: "q", parent: null, status: "backlog" },
    { key: "DIR-9", product_id: "p", parent: null, status: "backlog" },
  ] as Issue[];
  assert.deepEqual(reviewable(issues, "p").map((i) => i.key), ["DIR-9", "DIR-10", "DIR-3"]);
});

test("queue runs and outcome counts", () => {
  const runs = [
    { id: "1", created_at: 1, queue: { product_id: "p" }, actions: [{ outcome: "applied" }, { outcome: "denied" }, { outcome: "retained" }] },
    { id: "2", created_at: 2, queue: null, actions: [] },
  ] as unknown as AgentRun[];
  assert.deepEqual(queueRuns(runs, "p").map((r) => r.id), ["1"]);
  assert.deepEqual(outcomeCounts(runs[0]), { applied: 1, retained: 1, denied: 1 });
});
