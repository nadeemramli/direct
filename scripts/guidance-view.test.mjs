// DIR-39: playbook-version provenance shown for issue guidance and method
// findings. Run: node --test scripts/guidance-view.test.mjs (Node 24 strips
// the TypeScript types of the imported module natively).

import assert from "node:assert/strict";
import test from "node:test";
import {
  STATE_NOTE,
  UNKNOWN_VERSION,
  guidanceState,
  guidanceUses,
  playbookContext,
  playbookContextLabel,
  recordedVersion,
  shortFingerprint,
  versionLabel,
} from "../app/src/guidance.ts";

const FP_OLD = "041451a94f24c476e69c4b6ad470df1e42f71def9c8320a0b69f9b4dfe309d6a";
const FP_NEW = "86183dd65901cb14d0dcc3c8a01803c114945e574307cd68f62b00be4ed6b228";
const KNOWN = "v1.1 Direct baseline + owner-adopted E2E protocol (2026-10-01)";

const doc = (id, overrides = {}) => ({
  id,
  product_id: "p",
  title: `${id} title`,
  description: "",
  category: "playbook",
  source_root: "root",
  relative_path: `${id}.md`,
  source_updated: "2026-10-01",
  source_modified_at: null,
  fingerprint: FP_OLD,
  // Content that a careless implementation might mine for a version.
  content: "| v2 Agent Verification | Recommended next pilot |\n| v5 | Owner-directed pilot |",
  availability: "available",
  unavailable_reason: null,
  catalog_version: 7,
  checked_at: 1,
  cached_at: 1,
  ...overrides,
});
const ref = (document_id, playbook_version, recorded_fingerprint = FP_OLD) => ({
  document_id,
  recorded_fingerprint,
  playbook_version,
  linked_by: "agent-a",
  linked_at: 1791027586,
});
const issue = (key, theoria_refs) => ({ key, title: `${key} title`, theoria_refs });

test("a recorded version is shown exactly", () => {
  assert.equal(versionLabel(ref("a", KNOWN)), KNOWN);
  assert.equal(recordedVersion(ref("a", `  ${KNOWN} `)), KNOWN);
});

test("a missing version is explicitly Unknown and never inferred", () => {
  for (const missing of [null, "", "   ", undefined]) {
    assert.equal(recordedVersion(ref("a", missing)), null);
    assert.equal(versionLabel(ref("a", missing)), UNKNOWN_VERSION);
  }
  // Document content, dates, title and catalog revision are not consulted.
  const label = versionLabel(ref("dos-version-guide", null));
  assert.ok(!/v2|v5|Recommended|2026|7/.test(label), label);
});

test("stale and unavailable guidance are distinguished from the pinned version", () => {
  assert.equal(guidanceState(doc("a"), FP_OLD), "cached");
  assert.equal(guidanceState(doc("a", { fingerprint: FP_NEW }), FP_OLD), "stale");
  assert.equal(guidanceState(doc("a", { availability: "unavailable" }), FP_OLD), "unavailable");
  assert.equal(guidanceState(undefined, FP_OLD), "unavailable");
  // A stale cache keeps the historical pin and its version.
  const pinned = ref("a", KNOWN, FP_OLD);
  assert.equal(versionLabel(pinned), KNOWN);
  assert.match(STATE_NOTE.stale, /historical pin is preserved/);
  assert.equal(shortFingerprint(pinned.recorded_fingerprint), FP_OLD.slice(0, 12));
  assert.equal(shortFingerprint(null), "unavailable");
});

test("guidance detail lists every pinning issue with its own recorded version", () => {
  const issues = [
    issue("DIR-1", [ref("a", KNOWN), ref("b", null)]),
    issue("DIR-2", [ref("a", null, FP_NEW)]),
    issue("DIR-3", [ref("b", "v1-original")]),
  ];
  const uses = guidanceUses(issues, "a");
  assert.deepEqual(uses.map((use) => [use.issue.key, versionLabel(use.reference)]), [
    ["DIR-1", KNOWN],
    ["DIR-2", UNKNOWN_VERSION],
  ]);
  assert.deepEqual(guidanceUses(issues, "missing"), []);
});

test("method-finding playbook context comes only from the issue's pinned guidance", () => {
  assert.equal(playbookContextLabel(playbookContext(undefined)), "Unknown — no guidance linked to this issue");
  assert.equal(playbookContextLabel(playbookContext(issue("X", []))), "Unknown — no guidance linked to this issue");
  const allKnown = playbookContext(issue("X", [ref("a", KNOWN), ref("b", KNOWN), ref("c", KNOWN)]));
  assert.deepEqual(allKnown, { versions: [KNOWN], unknown: 0, linked: 3 });
  assert.equal(playbookContextLabel(allKnown), KNOWN);
  const allUnknown = playbookContext(issue("X", [ref("a", null), ref("b", "")]));
  assert.equal(playbookContextLabel(allUnknown), UNKNOWN_VERSION);
  const mixed = playbookContext(issue("X", [ref("a", "v1-original"), ref("b", null), ref("c", KNOWN)]));
  assert.equal(playbookContextLabel(mixed), `v1-original · ${KNOWN} · Unknown for 1 of 3 links`);
});
