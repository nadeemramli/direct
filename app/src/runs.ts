// Agent runs (DIR-76): presentation helpers. The service enforces every rule;
// these explain why "Run now" is unavailable before the owner presses it.
import type { AgentRun, AssignmentContext, Issue, RunState } from "./api";

export const RUN_STATE_LABEL: Record<RunState, string> = {
  intent: "Recorded · starting",
  launching: "Launching",
  running: "Running",
  succeeded: "Succeeded",
  failed: "Failed",
  blocked: "Blocked",
  unknown: "Unknown outcome",
  cancel_pending: "Canceling",
  canceled: "Canceled",
};

const TERMINAL: RunState[] = ["succeeded", "failed", "blocked", "unknown", "canceled"];

export function runStateClass(state: RunState): string {
  if (state === "succeeded") return "cached";
  if (["failed", "blocked", "unknown"].includes(state)) return "unavailable";
  return "stale";
}

/** Reasons the service will refuse a new run, in the order it checks. */
export function runBlockers(
  issue: Issue,
  assignment: AssignmentContext | null,
  runs: AgentRun[],
  now: number,
): string[] {
  const out: string[] = [];
  if (!assignment) return ["Assign an agent first."];
  if (assignment.member && !assignment.member.enabled) out.push(`${assignment.member.name} is disabled.`);
  if (assignment.role && assignment.role.status !== "active") out.push("The assigned role revision is not active.");
  const open = runs.find((r) => !r.queue && !TERMINAL.includes(r.state));
  if (open) out.push("A run for this issue has not finished.");
  if (runs.some((r) => r.state === "unknown" && r.assignment_id === assignment.assignment.id))
    out.push("A previous run has an unknown outcome; reassign to resume dispatch.");
  if (issue.claim && issue.claim.expires_at > now)
    out.push(`${issue.claim.actor} holds an active claim; reconcile with that writer first.`);
  return out;
}
