import { invoke, isTauri } from "@tauri-apps/api/core";
export type Status =
  | "backlog"
  | "ready"
  | "doing"
  | "verify"
  | "done"
  | "legacy_completed"
  | "canceled";
export type Outcome = "pending" | "passed" | "failed" | "canceled";
export type PlanningScope = "project" | "inbox";
export type ProjectStatus =
  | "planned"
  | "active"
  | "paused"
  | "completed"
  | "canceled";
export interface Product {
  id: string;
  key: string;
  name: string;
  repo_windows: string;
  repo_wsl: string;
  vault_windows: string;
  vault_wsl: string;
}
export interface Issue {
  id: string;
  key: string;
  product_id: string;
  project_id: string | null;
  milestone_id?: string | null;
  planning_scope?: PlanningScope;
  theoria_refs: TheoriaReference[];
  title: string;
  body: string;
  acceptance: string;
  owner: string;
  priority: string;
  status: Status;
  version: number;
  created_at: number;
  updated_at: number;
  claim: null | { actor: string; expires_at: number };
  needs_fix: boolean;
  parent: string | null;
  verification_key: string | null;
  current_run: string | null;
  external?: ExternalIssueRecord | null;
}
export interface ExternalIssueRecord {
  source: string;
  id: string;
  url: string;
  state: { id: string; name: string; type: string };
  created_at: string;
  updated_at: string;
  started_at: string | null;
  completed_at: string | null;
  canceled_at: string | null;
  archived_at: string | null;
  history: unknown[];
}
export type IssueLinkKind =
  | "parent"
  | "blocked_by"
  | "related"
  | "legacy_verification";
export interface IssueLink {
  id: string;
  source_key: string;
  target_key: string;
  kind: IssueLinkKind;
  external_source: string | null;
  external_id: string | null;
  created_by: string;
  created_at: number;
}
export interface IssueLinkContext extends IssueLink {
  direction: "incoming" | "outgoing";
  issue: Issue;
}
export interface TheoriaReference {
  document_id: string;
  recorded_fingerprint: string | null;
  playbook_version: string | null;
  linked_by: string;
  linked_at: number;
}
export interface TheoriaDocument {
  id: string;
  product_id: string;
  title: string;
  description: string;
  category: string;
  source_root: string;
  relative_path: string;
  source_updated: string | null;
  source_modified_at: number | null;
  fingerprint: string | null;
  content: string | null;
  availability: "available" | "unavailable";
  unavailable_reason: string | null;
  catalog_version: number;
  checked_at: number;
  cached_at: number | null;
}
export type FindingClassification =
  | "product_defect"
  | "method_friction"
  | "both";
export type EvidenceKind = "issue" | "build" | "check" | "owner_review";
export interface EvidencePointer {
  kind: EvidenceKind;
  reference: string;
  summary: string;
}
export interface MethodFinding {
  id: string;
  issue_key: string;
  classification: FindingClassification;
  observation: string;
  hypothesis: string;
  proposal: string;
  evidence: EvidencePointer[];
  created_by: string;
  created_at: number;
}
export interface GitTrace {
  id: string;
  issue_key: string;
  kind: "commit" | "push";
  repository: string;
  commit_sha: string;
  branch: string;
  remote: string | null;
  remote_ref: string | null;
  recorded_by: string;
  recorded_at: number;
}
export interface Verification {
  id: string;
  issue_key: string;
  build_ref: string;
  delivery_ref: string;
  summary: string;
  checks: string;
  limitations: string;
  preconditions: string;
  steps: { instruction: string; expected: string }[];
  outcome: Outcome;
  results: { outcome: Outcome; note: string }[];
  submitted_by: string;
  submitted_at: number;
  reviewed_by: string | null;
  reviewed_at: number | null;
  review_note: string;
}
export interface Context {
  issue: Issue;
  product: Product;
  project: Project | null;
  project_progress?: ProjectProgress | null;
  milestone: Milestone | null;
  milestone_progress: MilestoneProgress | null;
  goals: Goal[];
  goal_progress: GoalProgress[];
  issue_links: IssueLinkContext[];
  comments: { id: string; actor: string; body: string; at: number }[];
  more_comments: boolean;
  verifications: Verification[];
  method_findings: MethodFinding[];
  git_traces?: GitTrace[];
  history: { seq: number; kind: string; actor: string; at: number }[];
}
export interface Snapshot {
  workspace_id: string;
  products: Product[];
  projects: Project[];
  project_progress?: ProjectProgress[];
  goals: Goal[];
  goal_progress: GoalProgress[];
  milestones: Milestone[];
  milestone_progress: MilestoneProgress[];
  theoria_documents: TheoriaDocument[];
  method_findings: MethodFinding[];
  git_traces?: GitTrace[];
  issue_links: IssueLink[];
  issues: Issue[];
  cursor: number;
}
export interface Project {
  id: string;
  product_id: string;
  name: string;
  description: string;
  status?: ProjectStatus;
  priority?: string;
  sort_order?: number;
  external_source?: string | null;
  external_id?: string | null;
  external_url?: string | null;
  version: number;
  created_at: number;
  updated_at: number;
}
export interface ProjectProgress {
  project_id: string;
  total: number;
  backlog: number;
  active: number;
  pending_verification: number;
  completed: number;
  legacy_completed: number;
  canceled: number;
  completion_percent: number;
}
export type GoalStatus =
  | "planned"
  | "active"
  | "paused"
  | "completed"
  | "canceled";
export interface Goal {
  id: string;
  product_id: string;
  name: string;
  description: string;
  status: GoalStatus;
  priority: string;
  project_ids: string[];
  external_source: string | null;
  external_id: string | null;
  external_url?: string | null;
  version: number;
  created_at: number;
  updated_at: number;
}
export interface Milestone {
  id: string;
  project_id: string;
  name: string;
  description: string;
  sort_order: number;
  external_source: string | null;
  external_id: string | null;
  external_url?: string | null;
  version: number;
  created_at: number;
  updated_at: number;
}
export interface GoalProgress extends Omit<ProjectProgress, "project_id"> {
  goal_id: string;
}
export interface MilestoneProgress extends Omit<ProjectProgress, "project_id"> {
  milestone_id: string;
}
let token = sessionStorage.getItem("direct.session") || "";
const pending = new Map<string, string>();
export async function connect() {
  if (isTauri()) return;
  const grant = new URLSearchParams(location.hash.slice(1)).get("grant");
  if (grant) {
    history.replaceState(null, "", location.pathname);
    const response = await fetch("/api/session", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ grant }),
    });
    const data = await response.json();
    if (!response.ok) throw new Error(data.message);
    token = data.token;
    sessionStorage.setItem("direct.session", token);
  }
  if (!token)
    throw new Error(
      "Open Direct from the desktop app or run “direct open” to get a fresh local launch link.",
    );
}
export async function api<T = unknown>(
  command: Record<string, unknown>,
  mutation = false,
): Promise<T> {
  const key = JSON.stringify(command);
  const requestId = mutation ? pending.get(key) || crypto.randomUUID() : "";
  if (mutation) pending.set(key, requestId);
  const request = { actor: "owner", request_id: requestId, ...command };
  let data: T;
  if (isTauri()) data = await invoke<T>("direct_command", { request });
  else {
    const response = await fetch("/api/command", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify(request),
    });
    const value = await response.json();
    if (!response.ok)
      throw new Error(
        value.message || "Direct could not complete this action.",
      );
    data = value;
  }
  pending.delete(key);
  return data;
}
