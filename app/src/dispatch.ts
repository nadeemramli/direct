// Implementation dispatch display helpers (DIR-79). The service decides
// eligibility, writers and caps; these only describe what it recorded.
import type { DispatchObjective, RoutineRevision } from "./api";

export const LANE_LABEL: Record<string, string> = {
  dispatched: "Dispatched",
  ready: "Ready to dispatch",
  running: "Writer active",
  blocked: "Blocked",
  exhausted: "No Ready work",
};

export function laneClass(lane: string | null | undefined): string {
  if (lane === "dispatched" || lane === "ready") return "cached";
  if (lane === "blocked") return "unavailable";
  return "stale";
}

export function policyLabel(rev: Pick<RoutineRevision, "policy">): string {
  if (rev.policy === "dispatch_ready") return "Dispatch Ready work";
  return rev.policy === "refine_backlog" ? "Refine Backlog" : "Inspect only";
}

/** One line per objective, never implying more than was observed. */
export function objectiveLine(o: DispatchObjective): string {
  const repair = o.review_repair ? " · review repair" : "";
  switch (o.state) {
    case "pr_opened": {
      const push = o.verified_push === true ? "push verified" : o.verified_push === false ? "push NOT verified" : "push not checked";
      const owner = o.taken_over_by ? ` · taken over by ${o.taken_over_by}` : " · awaiting coordinator";
      return `${o.key}: PR ${o.pr_url} (${push})${owner}${repair}`;
    }
    case "pending":
      return `${o.key}: not started${repair}`;
    default:
      return `${o.key}: ${o.state}${o.detail ? ` — ${o.detail}` : ""}${o.taken_over_by ? ` · taken over by ${o.taken_over_by}` : ""}${repair}`;
  }
}

/** Parse the owner's comma-separated extra command prefixes. */
export function commandList(text: string): string[] {
  return text
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}
