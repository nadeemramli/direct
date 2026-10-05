// Product Planner reviews (DIR-77): presentation helpers. The service judges
// every proposal; these only arrange findings and explain the choices.
import type { AgentRun, Issue, PlannerFinding } from "./api";

export const KIND_LABEL: Record<string, string> = {
  unclear_outcome: "Unclear outcome",
  oversized: "Oversized",
  missing_criteria: "Missing criteria",
  duplicate: "Possible duplicate",
  dependency: "Dependency problem",
  route_mismatch: "Route mismatch",
};

export const ROUTE_LABEL: Record<string, string> = {
  bounded_brief: "Bounded brief",
  discovery: "DOS v5 discovery",
  reviewed_design: "Reviewed design",
};

/** Escalations awaiting a coordinator first, then newest. */
export function orderedFindings(findings: PlannerFinding[]): PlannerFinding[] {
  const waiting = (f: PlannerFinding) => (f.escalation && !f.confirmation ? 0 : 1);
  return [...findings].sort((a, b) => waiting(a) - waiting(b) || b.updated_at - a.updated_at);
}

/** Parent issues of a product a review can include, open work first. */
export function reviewable(issues: Issue[], productId: string): Issue[] {
  const rank: Record<string, number> = { backlog: 0, ready: 1, doing: 2, verify: 3, done: 4, legacy_completed: 5, canceled: 6 };
  return issues
    .filter((i) => i.product_id === productId && !i.parent)
    .sort((a, b) => (rank[a.status] ?? 9) - (rank[b.status] ?? 9) || a.key.localeCompare(b.key, undefined, { numeric: true }));
}

export function queueRuns(runs: AgentRun[], productId: string): AgentRun[] {
  return runs.filter((r) => r.queue?.product_id === productId).sort((a, b) => b.created_at - a.created_at);
}

export function outcomeCounts(run: AgentRun) {
  const count = (o: string) => run.actions.filter((a) => a.outcome === o).length;
  return { applied: count("applied"), retained: count("retained"), denied: count("denied") };
}
