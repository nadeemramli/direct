// Run with `npm test` (Node's built-in runner; types are stripped by Node).
import assert from "node:assert/strict";
import { test } from "node:test";
import { capabilityLabel, memberChoices, modelChoices, roleChoices } from "../src/members.ts";
import type { AgentMember, AgentRole } from "../src/api.ts";

const member = (over: Partial<AgentMember> = {}) =>
  ({
    id: "m1", name: "Claude", runtime: "claude-code", enabled: true, product_ids: ["dir"],
    default_model: "claude-opus-5-5", capability: null, ...over,
  }) as AgentMember;
const role = (over: Partial<AgentRole> = {}) =>
  ({ id: "r1", key: "product-planner", revision: 1, status: "active", runtime_compatibility: ["claude-code"], ...over }) as AgentRole;

test("members outside scope or disabled are explained, not hidden", () => {
  const choices = memberChoices([member(), member({ id: "m2", enabled: false }), member({ id: "m3", product_ids: ["alt"] })], "dir");
  assert.deepEqual(choices.map((c) => c.blocked), ["", "disabled", "not permitted in this product"]);
});

test("only active, runtime-compatible role revisions are selectable", () => {
  const choices = roleChoices(
    [role(), role({ id: "r2", status: "draft" }), role({ id: "r3", runtime_compatibility: ["codex"] }), role({ id: "r4", status: "retired" })],
    member(),
  );
  assert.deepEqual(choices.map((c) => [c.item.id, c.blocked]), [
    ["r1", ""], ["r2", "not active"], ["r3", "not written for claude-code"],
  ]);
});

test("models come from capability checks; an unchecked default stays unverified", () => {
  assert.deepEqual(modelChoices(member()), [{ item: "claude-opus-5-5", blocked: "unverified — run a capability check" }]);
  const checked = member({ capability: { harness_version: "2.1", verified_models: ["claude-opus-5-5"], evidence: "e", checked_by: "a", checked_at: 1 } });
  assert.deepEqual(modelChoices(checked), [{ item: "claude-opus-5-5", blocked: "" }]);
  assert.match(capabilityLabel(member()), /Unverified/);
  assert.match(capabilityLabel(checked), /Verified claude-opus-5-5/);
});
