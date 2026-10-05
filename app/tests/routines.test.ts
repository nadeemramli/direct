// Run with `npm test` (Node's built-in runner; types are stripped by Node).
import assert from "node:assert/strict";
import { test } from "node:test";
import { dueLabel, latestOccurrences, openNotices, routineState, triggerLabel } from "../src/routines.ts";
import type { Routine, RoutineNotice, RoutineOccurrence } from "../src/api.ts";

test("triggers and due times read in the routine's timezone", () => {
  assert.equal(triggerLabel({ kind: "daily", time: "09:00", weekday: null, timezone: "Asia/Kuala_Lumpur" }), "Daily at 09:00 (Asia/Kuala_Lumpur)");
  assert.equal(triggerLabel({ kind: "weekly", time: "10:30", weekday: "wed", timezone: "Asia/Kuala_Lumpur" }), "Weekly on Wed at 10:30 (Asia/Kuala_Lumpur)");
  // 2026-10-05T01:00:00Z is 09:00 on Monday in Kuala Lumpur.
  assert.match(dueLabel(1791162000, "Asia/Kuala_Lumpur"), /Mon.*5 Oct 2026.*09:00/);
  assert.equal(dueLabel(null, "Asia/Kuala_Lumpur"), "Not scheduled");
});

test("state explains pauses, restores and holds", () => {
  const r = (over: Partial<Routine>) => ({ status: "active", activated_at: 100, held_reason: null, ...over }) as Routine;
  assert.equal(routineState(r({}), null), "Active");
  assert.equal(routineState(r({ status: "paused" }), null), "Paused");
  assert.equal(routineState(r({ activated_at: 50 }), 80), "Paused after restore — reactivate");
  assert.match(routineState(r({ held_reason: "over limit" }), null), /^Held/);
});

test("occurrences newest first and open notices only", () => {
  const occ = [
    { routine_id: "a", created_at: 1 },
    { routine_id: "a", created_at: 3 },
    { routine_id: "b", created_at: 2 },
  ] as RoutineOccurrence[];
  assert.deepEqual(latestOccurrences(occ, "a").map((o) => o.created_at), [3, 1]);
  const notices = [
    { routine_id: "a", acknowledged: false, at: 1 },
    { routine_id: "a", acknowledged: true, at: 2 },
  ] as RoutineNotice[];
  assert.equal(openNotices(notices, "a").length, 1);
});
