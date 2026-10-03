// Run with `npm test` (Node's built-in runner; types are stripped by Node).
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  retryDelay,
  SessionExpired,
  startConnection,
  type ConnectionHooks,
  type LoadState,
} from "../src/connection.ts";

async function until(check: () => boolean, ms = 2000) {
  const end = Date.now() + ms;
  while (!check()) {
    if (Date.now() > end) throw new Error("condition not reached");
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}
function harness(load: ConnectionHooks["load"], poll: ConnectionHooks["poll"] = async () => {}) {
  const states: LoadState[] = [];
  const connection = startConnection({
    load,
    poll,
    state: (state) => states.push(state),
    deadlineMs: 60,
    pollMs: 10,
    delay: () => 10,
  });
  return { states, connection, last: () => states[states.length - 1] };
}

test("a failed first load is retried automatically, then polling starts", async () => {
  let loads = 0;
  let polls = 0;
  const h = harness(
    async () => {
      loads += 1;
      if (loads < 3) throw new Error("Direct is unavailable right now.");
    },
    async () => {
      polls += 1;
    },
  );
  await until(() => polls >= 2);
  h.connection.stop();
  assert.equal(loads, 3);
  assert.deepEqual(
    h.states.map((s) => s.kind),
    ["loading", "retrying", "retrying", "loaded"],
  );
  const retry = h.states[1] as Extract<LoadState, { kind: "retrying" }>;
  assert.equal(retry.attempt, 1);
  assert.match(retry.reason, /unavailable/);
});

test("a hung first load is abandoned at its deadline and its late answer is ignored", async () => {
  let loads = 0;
  let abandoned: AbortSignal | undefined;
  let finishLate = () => {};
  const h = harness(async (signal) => {
    loads += 1;
    if (loads === 1) {
      abandoned = signal;
      // Ignores the signal entirely, like a stuck desktop IPC call.
      return new Promise<void>((resolve) => (finishLate = resolve));
    }
  });
  await until(() => h.last()?.kind === "loaded");
  assert.equal(abandoned?.aborted, true, "the stalled read and its body were aborted");
  const retry = h.states.find((s) => s.kind === "retrying") as Extract<LoadState, { kind: "retrying" }>;
  assert.match(retry.reason, /did not answer within/);
  const seen = h.states.length;
  finishLate();
  await new Promise((resolve) => setTimeout(resolve, 30));
  h.connection.stop();
  assert.equal(h.states.length, seen, "the abandoned attempt changed nothing");
  assert.equal(loads, 2);
});

test("an expired browser session is reported once and never retried", async () => {
  let loads = 0;
  const h = harness(async () => {
    loads += 1;
    throw new SessionExpired("Launch link expired or already used.");
  });
  await until(() => h.last()?.kind === "session_expired");
  await new Promise((resolve) => setTimeout(resolve, 50));
  assert.equal(loads, 1);
  assert.deepEqual(h.last(), {
    kind: "session_expired",
    message: "Launch link expired or already used.",
  });
  h.connection.retryNow();
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(loads, 1, "retry cannot revive a dead session");
});

test("a session that ends while polling stops the poll and says so", async () => {
  let polls = 0;
  const h = harness(
    async () => {},
    async () => {
      polls += 1;
      if (polls === 3) throw new SessionExpired("This browser session is no longer valid.");
    },
  );
  await until(() => h.last()?.kind === "session_expired");
  await new Promise((resolve) => setTimeout(resolve, 50));
  assert.equal(polls, 3);
});

test("retry now starts one attempt immediately and never overlaps another", async () => {
  let loads = 0;
  let active = 0;
  let overlap = false;
  const states: LoadState[] = [];
  const connection = startConnection({
    async load() {
      loads += 1;
      active += 1;
      overlap ||= active > 1;
      await new Promise((resolve) => setTimeout(resolve, 5));
      active -= 1;
      if (loads === 1) throw new Error("offline");
    },
    poll: async () => {},
    state: (state) => states.push(state),
    deadlineMs: 1000,
    pollMs: 1000,
    delay: () => 60_000,
  });
  await until(() => states.some((s) => s.kind === "retrying"));
  connection.retryNow();
  connection.retryNow();
  await until(() => states.some((s) => s.kind === "loaded"), 500);
  connection.stop();
  assert.equal(loads, 2);
  assert.equal(overlap, false);
});

test("retry waits grow to a steady five seconds", () => {
  assert.deepEqual([1, 2, 3, 4, 5, 20].map(retryDelay), [1000, 2000, 4000, 5000, 5000, 5000]);
});
