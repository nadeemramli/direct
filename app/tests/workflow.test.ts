// Run with `npm test` (Node's built-in runner; types are stripped by Node).
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  WORKFLOW_CATEGORIES,
  WORKFLOW_STATES,
  breakdown,
  isStatusFilter,
  isWorkflowState,
  matchesStatusFilter,
  percentLabel,
  progressSegments,
  progressSummary,
  workflowState,
} from "../src/workflow.ts";
import type { Status } from "../src/api.ts";

const issue = (status: Status, needs_fix = false) => ({ status, needs_fix });

test("needs fix is its own state only while work is open", () => {
  assert.equal(workflowState(issue("doing", true)), "needs_fix");
  assert.equal(workflowState(issue("ready", true)), "needs_fix");
  assert.equal(workflowState(issue("doing")), "doing");
  for (const status of ["done", "legacy_completed", "canceled"] as Status[])
    assert.equal(workflowState(issue(status, true)), status);
});

test("every workflow state belongs to exactly one category", () => {
  const listed = WORKFLOW_CATEGORIES.flatMap((category) => category.states);
  assert.deepEqual([...listed].sort(), Object.keys(WORKFLOW_STATES).sort());
  assert.equal(new Set(listed).size, listed.length);
  for (const category of WORKFLOW_CATEGORIES)
    for (const state of category.states) assert.equal(WORKFLOW_STATES[state].category, category.id);
  assert.ok(isWorkflowState("needs_fix"));
  assert.ok(!isWorkflowState("all"));
  assert.ok(!isWorkflowState("toString"));
});

test("breakdown matches the service's completion formula", () => {
  const b = breakdown([
    issue("done"),
    issue("done"),
    issue("verify"),
    issue("doing", true),
    issue("doing"),
    issue("ready"),
    issue("backlog"),
    issue("legacy_completed"),
    issue("canceled"),
  ]);
  assert.equal(b.total, 9);
  assert.equal(b.eligible, 7);
  assert.equal(b.percent, 28); // floor(2 * 100 / 7)
  assert.equal(b.counts.needs_fix, 1);
  assert.equal(b.counts.doing, 1);
  const segments = progressSegments(b);
  assert.deepEqual(
    segments.map((s) => s.state),
    ["done", "verify", "needs_fix", "doing", "ready", "backlog"],
  );
  assert.equal(segments.reduce((sum, s) => sum + s.count, 0), b.eligible);
  assert.match(progressSummary(b), /^28% verified done · 2 done, 1 verify, 1 needs fix, 1 doing, 1 ready, 1 backlog · not counted: 1 legacy done, 1 canceled$/);
});

test("empty and fully excluded scopes have no segments", () => {
  assert.deepEqual(progressSegments(breakdown([])), []);
  const excluded = breakdown([issue("canceled"), issue("legacy_completed")]);
  assert.equal(excluded.eligible, 0);
  assert.equal(excluded.percent, 0);
  assert.deepEqual(progressSegments(excluded), []);
  assert.equal(progressSummary(breakdown([])), "No issues");
});

test("percent labels never round a nonzero share to zero", () => {
  assert.equal(percentLabel(1, 300), "<1%");
  assert.equal(percentLabel(0, 300), "0%");
  assert.equal(percentLabel(1, 3), "33%");
  assert.equal(percentLabel(1, 0), "0%");
});

test("status filter matches open work and single workflow states", () => {
  assert.ok(matchesStatusFilter(issue("doing", true), "needs_fix"));
  assert.ok(!matchesStatusFilter(issue("doing", true), "doing"));
  assert.ok(matchesStatusFilter(issue("doing"), "doing"));
  for (const status of ["backlog", "ready", "doing", "verify"] as Status[])
    assert.ok(matchesStatusFilter(issue(status), "open"), status);
  for (const status of ["done", "legacy_completed", "canceled"] as Status[]) {
    assert.ok(!matchesStatusFilter(issue(status), "open"), status);
    assert.ok(matchesStatusFilter(issue(status), "all"), status);
  }
  assert.ok(isStatusFilter("open") && isStatusFilter("all") && isStatusFilter("verify"));
  assert.ok(!isStatusFilter("needs") && !isStatusFilter("constructor"));
});

test("only open parent work can be canceled and active claims are surfaced", async () => {
  const { cancelEligibility } = await import("../src/workflow.ts");
  const base = { parent: null, claim: null };
  for (const status of ["backlog", "ready", "doing"] as Status[])
    assert.equal(cancelEligibility({ ...base, status }, 100).allowed, true);
  for (const status of ["verify", "done", "legacy_completed", "canceled"] as Status[])
    assert.equal(cancelEligibility({ ...base, status }, 100).allowed, false);
  assert.match(cancelEligibility({ ...base, status: "verify" }, 100).reason, /pending verification/);
  assert.equal(cancelEligibility({ ...base, status: "ready", parent: "id" }, 100).allowed, false);
  const held = { ...base, status: "doing" as Status, claim: { actor: "agent-1", expires_at: 200 } };
  assert.equal(cancelEligibility(held, 100).activeClaim, "agent-1");
  assert.equal(cancelEligibility(held, 300).activeClaim, null);
});
