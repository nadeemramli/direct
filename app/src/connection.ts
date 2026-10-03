/**
 * Loading the workspace and keeping it current (DIR-59). The first load is
 * retried automatically with a deadline per attempt; afterwards the steady
 * poll runs. Only reads happen here, so nothing is ever replayed as a write.
 */

/**
 * This browser has no valid local session: the launch link was used or
 * expired, or the service no longer knows the session (it restarted or the
 * session ended). Retrying cannot fix it; a fresh launch link can. This is
 * distinct from the service being unreachable, which recovers by itself.
 */
export class SessionExpired extends Error {}

/** A read that has not finished by then is abandoned, including its body. */
export const LOAD_DEADLINE_MS = 10_000;
const POLL_MS = 750;

/** Wait before the next attempt to load: 1 s, 2 s, 4 s, then every 5 s. */
export function retryDelay(attempt: number) {
  return Math.min(1000 * 2 ** Math.max(0, attempt - 1), 5000);
}

export type LoadState =
  | { kind: "loading" }
  | { kind: "retrying"; attempt: number; reason: string; retryInMs: number }
  | { kind: "loaded" }
  | { kind: "session_expired"; message: string };

function reason(error: unknown, deadlineMs: number) {
  if (
    error instanceof DOMException &&
    (error.name === "TimeoutError" || error.name === "AbortError")
  )
    return `Direct did not answer within ${Math.round(deadlineMs / 1000)} seconds.`;
  return String(error instanceof Error ? error.message : error).replace(
    /^Error: /,
    "",
  );
}

/** Settles with `work`, or rejects as soon as `signal` fires. */
function bounded(work: Promise<void>, signal: AbortSignal) {
  return new Promise<void>((resolve, reject) => {
    const abandon = () => reject(signal.reason);
    if (signal.aborted) return abandon();
    signal.addEventListener("abort", abandon, { once: true });
    work
      .then(resolve, reject)
      .finally(() => signal.removeEventListener("abort", abandon));
  });
}

export interface ConnectionHooks {
  /** Connect and load the workspace; must give up when `signal` fires. */
  load(signal: AbortSignal): Promise<void>;
  /** One steady-state check; handles its own failures except SessionExpired. */
  poll(): Promise<void>;
  state(state: LoadState): void;
  deadlineMs?: number;
  pollMs?: number;
  delay?: (attempt: number) => number;
}

export function startConnection(hooks: ConnectionHooks) {
  const deadline = hooks.deadlineMs ?? LOAD_DEADLINE_MS;
  const pollMs = hooks.pollMs ?? POLL_MS;
  const delay = hooks.delay ?? retryDelay;
  let stopped = false;
  let running = false;
  let loaded = false;
  let attempt = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  function schedule(ms: number) {
    if (!stopped) timer = setTimeout(step, ms);
  }
  function expire(error: SessionExpired) {
    stopped = true;
    hooks.state({ kind: "session_expired", message: error.message });
  }
  async function step() {
    if (stopped || running) return;
    clearTimeout(timer);
    running = true;
    try {
      if (loaded) {
        await hooks.poll();
        schedule(pollMs);
        return;
      }
      if (attempt === 0) hooks.state({ kind: "loading" });
      // A late answer from an abandoned attempt is never applied: the hook
      // checks the signal, and this attempt has already moved on.
      const signal = AbortSignal.timeout(deadline);
      await bounded(hooks.load(signal), signal);
      if (stopped) return;
      loaded = true;
      attempt = 0;
      hooks.state({ kind: "loaded" });
      schedule(pollMs);
    } catch (error) {
      if (stopped) return;
      if (error instanceof SessionExpired) return expire(error);
      if (loaded) {
        schedule(pollMs);
        return;
      }
      attempt += 1;
      const wait = delay(attempt);
      hooks.state({
        kind: "retrying",
        attempt,
        reason: reason(error, deadline),
        retryInMs: wait,
      });
      schedule(wait);
    } finally {
      running = false;
    }
  }
  step();
  return {
    /** Try again now instead of waiting for the next scheduled attempt. */
    retryNow() {
      if (!running) step();
    },
    stop() {
      stopped = true;
      clearTimeout(timer);
    },
  };
}
