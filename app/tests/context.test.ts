// Run with `npm test` (Node's built-in runner; types are stripped by Node).
import assert from "node:assert/strict";
import { test } from "node:test";
import type { ContextLink, Snapshot } from "../src/api.ts";
import { changedSinceLinked, linksFor, sourceDetail, sourceLabel, targetLabel } from "../src/context.ts";

function link(patch: Partial<ContextLink> = {}): ContextLink {
  return {
    id: "l1",
    target_kind: "project",
    target: "p1",
    source: { kind: "obsidian", product_id: "x", path: "Specs/Reporting.md" },
    title: "Reporting spec",
    note: "",
    pinned_fingerprint: "a".repeat(64),
    observation: { available: true, fingerprint: "a".repeat(64), checked_at: 1, reason: "" },
    version: 1,
    created_by: "owner",
    created_at: 1,
    updated_at: 1,
    ...patch,
  };
}

test("source labels and detail keep Theoria-free, provenance-bearing wording", () => {
  assert.equal(sourceLabel({ kind: "obsidian", product_id: "x", path: "a.md" }), "Obsidian");
  assert.equal(sourceLabel({ kind: "retained_record", record_id: "r" }), "Linear document");
  assert.equal(sourceLabel({ kind: "url", url: "https://e.com" }), "Link");
  assert.equal(sourceDetail(link()), "Specs/Reporting.md");
  assert.equal(
    sourceDetail(link({ source: { kind: "retained_record", record_id: "r" }, source_id: "lin-1", url: "https://linear.app/d" })),
    "Linear ID lin-1 · https://linear.app/d",
  );
  assert.equal(sourceDetail(link({ source: { kind: "retained_record", record_id: "r" } })), "Retained record");
});

test("a change is reported only when both fingerprints are known and differ", () => {
  assert.equal(changedSinceLinked(link()), false);
  assert.equal(changedSinceLinked(link({ observation: { available: true, fingerprint: "b".repeat(64), checked_at: 2, reason: "" } })), true);
  assert.equal(changedSinceLinked(link({ observation: { available: false, fingerprint: null, checked_at: 2, reason: "gone" } })), false);
  assert.equal(changedSinceLinked(link({ pinned_fingerprint: null })), false);
});

test("inherited links name where they come from", () => {
  const data = {
    projects: [{ id: "p1", name: "Reporting" }],
    goals: [{ id: "g1", name: "Self-serve" }],
    releases: [{ id: "r1", version_label: "1.0" }],
  } as unknown as Snapshot;
  assert.equal(targetLabel(link(), data), "project Reporting");
  assert.equal(targetLabel(link({ target_kind: "goal", target: "g1" }), data), "goal Self-serve");
  assert.equal(targetLabel(link({ target_kind: "release", target: "r1" }), data), "release 1.0");
  assert.equal(targetLabel(link({ target_kind: "issue", target: "DIR-4" }), data), "DIR-4");
  const all = [link(), link({ id: "l2", target_kind: "goal", target: "g1" })];
  assert.deepEqual(linksFor(all, "goal", "g1").map((l) => l.id), ["l2"]);
  assert.deepEqual(linksFor(undefined, "goal", "g1"), []);
});
