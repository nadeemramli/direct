// Workflow states and progress breakdowns derived from issue status (DIR workflow view).
// Direct stores a native `status` plus a `needs_fix` flag; a failed verification
// returns work to Doing with that flag set. The workflow view shows that combination
// as its own "Needs fix" state, so every parent issue sits in exactly one state.
import type { Issue, Status } from "./api";

export type WorkflowState =
  | "backlog"
  | "ready"
  | "needs_fix"
  | "doing"
  | "verify"
  | "done"
  | "legacy_completed"
  | "canceled";

export type WorkflowCategory = "backlog" | "unstarted" | "started" | "completed" | "canceled";

export interface WorkflowStateInfo {
  id: WorkflowState;
  name: string;
  category: WorkflowCategory;
  description: string;
}

export const WORKFLOW_STATES: Record<WorkflowState, WorkflowStateInfo> = {
  backlog: {
    id: "backlog",
    name: "Backlog",
    category: "backlog",
    description: "Captured work waiting to be shaped",
  },
  ready: {
    id: "ready",
    name: "Ready",
    category: "unstarted",
    description: "Shaped and authorized for an agent to claim",
  },
  needs_fix: {
    id: "needs_fix",
    name: "Needs fix",
    category: "unstarted",
    description: "Failed verification; the agent repairs and resubmits",
  },
  doing: {
    id: "doing",
    name: "Doing",
    category: "started",
    description: "Claimed and in progress",
  },
  verify: {
    id: "verify",
    name: "Verify",
    category: "started",
    description: "Submitted; waiting for agent E2E evidence or owner review",
  },
  done: {
    id: "done",
    name: "Done",
    category: "completed",
    description: "Verified and accepted by the owner",
  },
  legacy_completed: {
    id: "legacy_completed",
    name: "Legacy done",
    category: "completed",
    description: "Completed before Direct verification; not counted in progress",
  },
  canceled: {
    id: "canceled",
    name: "Canceled",
    category: "canceled",
    description: "Stopped without delivery; not counted in progress",
  },
};

export const WORKFLOW_CATEGORIES: { id: WorkflowCategory; name: string; states: WorkflowState[] }[] = [
  { id: "backlog", name: "Backlog", states: ["backlog"] },
  { id: "unstarted", name: "Unstarted", states: ["ready", "needs_fix"] },
  { id: "started", name: "Started", states: ["doing", "verify"] },
  { id: "completed", name: "Completed", states: ["done", "legacy_completed"] },
  { id: "canceled", name: "Canceled", states: ["canceled"] },
];

/** The state new issues start in. */
export const DEFAULT_STATE: WorkflowState = "backlog";

const TERMINAL: Status[] = ["done", "legacy_completed", "canceled"];

export function isWorkflowState(value: string): value is WorkflowState {
  return Object.hasOwn(WORKFLOW_STATES, value);
}

export function workflowState(issue: Pick<Issue, "status" | "needs_fix">): WorkflowState {
  if (issue.needs_fix && !TERMINAL.includes(issue.status)) return "needs_fix";
  return issue.status;
}

export interface WorkflowBreakdown {
  counts: Record<WorkflowState, number>;
  total: number;
  /** Issues that count toward completion: everything but canceled and legacy work. */
  eligible: number;
  /** Same formula as the service: floor(done * 100 / eligible). */
  percent: number;
}

export function emptyCounts(): Record<WorkflowState, number> {
  return {
    backlog: 0,
    ready: 0,
    needs_fix: 0,
    doing: 0,
    verify: 0,
    done: 0,
    legacy_completed: 0,
    canceled: 0,
  };
}

export function breakdown(issues: Iterable<Pick<Issue, "status" | "needs_fix">>): WorkflowBreakdown {
  const counts = emptyCounts();
  let total = 0;
  for (const issue of issues) {
    counts[workflowState(issue)] += 1;
    total += 1;
  }
  const eligible = total - counts.canceled - counts.legacy_completed;
  return {
    counts,
    total,
    eligible,
    percent: eligible > 0 ? Math.floor((counts.done * 100) / eligible) : 0,
  };
}

/** Bar order: furthest along first, so the filled part reads left to right. */
export const PROGRESS_ORDER: WorkflowState[] = ["done", "verify", "needs_fix", "doing", "ready", "backlog"];

export interface ProgressSegment {
  state: WorkflowState;
  count: number;
  /** Share of eligible work, 0..1. */
  share: number;
}

export function progressSegments(b: WorkflowBreakdown): ProgressSegment[] {
  if (b.eligible <= 0) return [];
  return PROGRESS_ORDER.filter((state) => b.counts[state] > 0).map((state) => ({
    state,
    count: b.counts[state],
    share: b.counts[state] / b.eligible,
  }));
}

export function percentLabel(count: number, of: number) {
  if (of <= 0) return "0%";
  const value = (count * 100) / of;
  return value > 0 && value < 1 ? "<1%" : `${Math.round(value)}%`;
}

/** One-line plain-language summary, also used as the bar's accessible name. */
export function progressSummary(b: WorkflowBreakdown) {
  if (!b.total) return "No issues";
  const parts = PROGRESS_ORDER.filter((state) => b.counts[state] > 0).map(
    (state) => `${b.counts[state]} ${WORKFLOW_STATES[state].name.toLowerCase()}`,
  );
  const excluded = [
    b.counts.legacy_completed ? `${b.counts.legacy_completed} legacy done` : "",
    b.counts.canceled ? `${b.counts.canceled} canceled` : "",
  ].filter(Boolean);
  return `${b.percent}% verified done · ${parts.join(", ") || "nothing counted"}${
    excluded.length ? ` · not counted: ${excluded.join(", ")}` : ""
  }`;
}
