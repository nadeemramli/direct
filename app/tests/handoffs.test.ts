// Run with `npm test` (Node's built-in runner; types are stripped by Node).
import assert from "node:assert/strict";
import { test } from "node:test";
import { checksByEnvironment, packetMarkdown, stateClass } from "../src/handoffs.ts";
import type { CloudCheck, CloudHandoff } from "../src/api.ts";

const handoff = {
  id: "h1",
  status: "prepared",
  reconciliation: null,
  packet_sha256: "f".repeat(64),
  packet: {
    format: 1, issue_key: "INS-3", product_key: "INS", title: "Task", body: "Brief", acceptance: "Done when",
    issue_version: 5, claim_actor: "coord", claim_expires_at: 99, repository: "acme/widgets", base_ref: "main",
    required_model: "claude-opus-5-5", evidence_plan: "Plan",
    guidance: [{ document_id: "dos", title: "Workflow", relative_path: "8.md", recorded_fingerprint: null, playbook_version: null }],
    constraints: ["Work only on INS-3"],
  },
} as unknown as CloudHandoff;

test("packet markdown carries the frozen packet and nothing else", () => {
  const md = packetMarkdown(handoff);
  assert.match(md, /^# Cloud handoff: INS-3 — Task\n/);
  assert.match(md, /Required model: claude-opus-5-5/);
  assert.match(md, /fingerprint `unavailable` · playbook Unknown/);
  assert.match(md, /- Work only on INS-3\n$/);
  assert.doesNotMatch(md, /grant=|127\.0\.0\.1|localhost/);
});

test("checks split by environment and failed cloud results are flagged", () => {
  const checks = [
    { name: "a", outcome: "passed", environment: "cloud", detail: "" },
    { name: "b", outcome: "failed", environment: "local", detail: "" },
  ] as CloudCheck[];
  const groups = checksByEnvironment(checks);
  assert.deepEqual(groups.cloud.map((c) => c.name), ["a"]);
  assert.deepEqual(groups.local.map((c) => c.name), ["b"]);
  assert.equal(stateClass(handoff), "cached");
  const blocked = { ...handoff, status: "reconciled", reconciliation: { cloud_verdict: "blocked" } } as unknown as CloudHandoff;
  assert.equal(stateClass(blocked), "stale");
  assert.equal(stateClass({ ...handoff, status: "withdrawn" }), "unavailable");
});
