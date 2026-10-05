// Run with `npm test` (Node's built-in runner; types are stripped by Node).
import assert from "node:assert/strict";
import { test } from "node:test";
import { runBlockers, runStateClass } from "../src/runs.ts";
import type { AgentRun, AssignmentContext, Issue } from "../src/api.ts";

const issue = (claim: Issue["claim"] = null) => ({ key: "DIR-1", claim }) as Issue;
const assignment = (over: Partial<AssignmentContext> = {}) =>
  ({
    assignment: { id: "a1" },
    member: { id: "m", name: "Claude", runtime: "claude-code", enabled: true },
    role: { id: "r", key: "planner", revision: 2, name: "Planner", status: "active" },
    requested_model: "claude-opus-5-5",
    actual_model: null,
    ...over,
  }) as AssignmentContext;
const run = (state: AgentRun["state"], assignment_id = "a1") => ({ state, assignment_id }) as AgentRun;

test("run now explains each service refusal", () => {
  assert.deepEqual(runBlockers(issue(), null, [], 0), ["Assign an agent first."]);
  assert.deepEqual(runBlockers(issue(), assignment(), [run("succeeded")], 0), []);
  assert.match(runBlockers(issue(), assignment(), [run("running")], 0).join(), /not finished/);
  assert.match(runBlockers(issue(), assignment(), [run("unknown")], 0).join(), /unknown outcome/);
  assert.deepEqual(runBlockers(issue(), assignment(), [run("unknown", "old")], 0), []);
  assert.match(runBlockers(issue({ actor: "w", expires_at: 10 }), assignment(), [], 5).join(), /w holds an active claim/);
  assert.match(
    runBlockers(issue(), assignment({ member: { id: "m", name: "Claude", runtime: "claude-code", enabled: false } }), [], 0).join(),
    /disabled/,
  );
});

test("states map to visible classes", () => {
  assert.equal(runStateClass("succeeded"), "cached");
  assert.equal(runStateClass("blocked"), "unavailable");
  assert.equal(runStateClass("running"), "stale");
});
