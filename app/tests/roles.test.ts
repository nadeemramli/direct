// Run with `npm test` (Node's built-in runner; types are stripped by Node).
import assert from "node:assert/strict";
import { test } from "node:test";
import { activationBlockers, harnessStatus, pinState, roleGroups } from "../src/roles.ts";
import type { AgentRole, RolePublication, SkillPackage, TheoriaDocument } from "../src/api.ts";

const doc = (id: string, fingerprint: string | null, availability = "available") =>
  ({ id, fingerprint, availability }) as TheoriaDocument;
const role = (over: Partial<AgentRole> = {}) =>
  ({
    id: "r1", key: "product-planner", revision: 1, status: "draft", owner_direction: "Weekly review",
    skills: ["s1"], runtime_compatibility: ["claude-code", "codex"],
    guidance: [{ document_id: "dos", recorded_fingerprint: "a", playbook_version: null, mandatory: true }],
    ...over,
  }) as AgentRole;
const skill = (over: Partial<SkillPackage> = {}) =>
  ({ id: "s1", name: "issue-health", revision: 1, retired: null, ...over }) as SkillPackage;

test("revisions group by key, newest first", () => {
  const groups = roleGroups([role(), role({ id: "r2", revision: 2 }), role({ id: "x", key: "auditor" })]);
  assert.deepEqual(groups.map((g) => g.key), ["auditor", "product-planner"]);
  assert.deepEqual(groups[1].revisions.map((r) => r.revision), [2, 1]);
});

test("pins report drift and unavailability without changing the recorded fingerprint", () => {
  const pin = role().guidance[0];
  assert.equal(pinState(pin, [doc("dos", "a")]), "cached");
  assert.equal(pinState(pin, [doc("dos", "b")]), "stale");
  assert.equal(pinState(pin, [doc("dos", "a", "unavailable")]), "unavailable");
  assert.equal(pinState(pin, []), "unavailable");
  assert.equal(pin.recorded_fingerprint, "a");
});

test("activation blockers mirror the service checks", () => {
  assert.deepEqual(activationBlockers(role(), [skill()], [doc("dos", "a")]), []);
  const blocked = activationBlockers(
    role({ owner_direction: " " }),
    [skill({ retired: { by: "owner", at: 1, reason: "old" } })],
    [doc("dos", "a", "unavailable")],
  );
  assert.equal(blocked.length, 3);
  assert.match(blocked.join(" "), /owner direction.*retired.*unavailable/s);
});

test("harnesses are unverified until a fresh session is recorded", () => {
  const publications = [
    { role_id: "r1", harness: "claude-code", evidence: [{}] },
    { role_id: "r1", harness: "codex", evidence: [] },
  ] as unknown as RolePublication[];
  assert.deepEqual(harnessStatus(role(), publications), [
    { harness: "claude-code", verified: true },
    { harness: "codex", verified: false },
  ]);
});
