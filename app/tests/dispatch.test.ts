// Run with `npm test` (Node's built-in runner; types are stripped by Node).
import assert from "node:assert/strict";
import { test } from "node:test";
import { commandList, laneClass, objectiveLine, policyLabel } from "../src/dispatch.ts";
import type { DispatchObjective } from "../src/api.ts";

const objective = (patch: Partial<DispatchObjective>): DispatchObjective => ({
  key: "DIR-7",
  version: 3,
  review_repair: false,
  state: "pending",
  guidance: [],
  claim_actor: null,
  branch: null,
  base_sha: null,
  head_sha: null,
  pr_url: null,
  evidence: "",
  verified_push: null,
  detail: "",
  taken_over_by: null,
  ...patch,
});

test("objective lines never imply more than was observed", () => {
  assert.equal(objectiveLine(objective({})), "DIR-7: not started");
  const pr = objective({ state: "pr_opened", pr_url: "https://github.com/o/r/pull/3" });
  assert.match(objectiveLine(pr), /\(push not checked\) · awaiting coordinator$/);
  assert.match(objectiveLine({ ...pr, verified_push: false }), /push NOT verified/);
  assert.match(objectiveLine({ ...pr, verified_push: true, taken_over_by: "claude-coordinator" }), /push verified\) · taken over by claude-coordinator/);
  assert.equal(objectiveLine(objective({ state: "skipped", detail: "Stale: DIR-7 changed", review_repair: true })), "DIR-7: skipped — Stale: DIR-7 changed · review repair");
});

test("lanes and policies are labelled honestly", () => {
  assert.equal(laneClass("dispatched"), "cached");
  assert.equal(laneClass("blocked"), "unavailable");
  assert.equal(laneClass("running"), "stale");
  assert.equal(policyLabel({ policy: "dispatch_ready" }), "Dispatch Ready work");
  assert.equal(policyLabel({ policy: "inspect_only" }), "Inspect only");
  assert.deepEqual(commandList(" cargo test, ,npm run build "), ["cargo test", "npm run build"]);
});
