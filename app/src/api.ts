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
  labels?: string[];
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
export interface LabelProductRule {
  product_id: string;
  default_for_new_issues: boolean;
}
export interface LinearLabelOrigin {
  id: string;
  name: string;
}
export interface Label {
  id: string;
  name: string;
  description: string;
  color: string;
  aliases: string[];
  products: LabelProductRule[];
  linear_origins: LinearLabelOrigin[];
  version: number;
  created_at: number;
  updated_at: number;
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
export type ReleaseStatus =
  | "planned"
  | "active"
  | "preview"
  | "production"
  | "retired"
  | "canceled";
export type ReleaseEvidenceKind =
  | "commit"
  | "push"
  | "check"
  | "preview_deployment"
  | "production_deployment"
  | "rollback";
export interface ReleaseRecord {
  id: string;
  product_id: string;
  name: string;
  version_label: string;
  status: ReleaseStatus;
  target_ref: string;
  release_branch: string | null;
  preview_url: string | null;
  notes: string;
  project_ids: string[];
  issue_keys: string[];
  external_source: string | null;
  external_id: string | null;
  external_url: string | null;
  version: number;
  created_at: number;
  updated_at: number;
}
export interface ReleaseEvidence {
  id: string;
  release_id: string;
  kind: ReleaseEvidenceKind;
  issue_key: string | null;
  git_trace_id: string | null;
  verification_id: string | null;
  deployment_ref: string | null;
  commit_sha: string | null;
  target_ref: string | null;
  source_ref: string | null;
  environment: string | null;
  url: string | null;
  approver: string | null;
  outcome: Outcome;
  note: string;
  recorded_by: string;
  recorded_at: number;
}
export interface ReleaseWorkflowConfig {
  product_id: string;
  branch_strategy: "one_branch_per_release" | "external";
  production_ref: string;
  release_branch_pattern: string;
  preview_environment: string;
  preview_url_template: string;
  promotion_policy: "verified_owner_approval" | "external_manual";
  version: number;
  created_at: number;
  updated_at: number;
}
export interface ReleaseProgress {
  release_id: string;
  total: number;
  backlog: number;
  active: number;
  pending_verification: number;
  failed_verification: number;
  completed: number;
  legacy_completed: number;
  canceled: number;
  completion_percent: number;
}
export interface Verification {
  e2e?: {
    build_ref: string;
    delivered_build_ref: string;
    environment: string;
    entrypoint: string;
    scenarios: string;
    outcome: Outcome;
    delivery_check: string;
  } | null;
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
export type DeletionBlockerKind =
  | "status"
  | "active_claim"
  | "comments"
  | "verification_history"
  | "verification_children"
  | "method_findings"
  | "git_traces"
  | "issue_links"
  | "release_references"
  | "release_evidence";
export interface DeletionBlocker {
  kind: DeletionBlockerKind;
  count: number;
  /** Issue keys, release IDs, run IDs, commit SHAs, or the claiming actor. */
  references: string[];
  /** The owner can clear it without losing retained history. */
  removable: boolean;
  message: string;
}
/** Server-computed; delete_issue re-checks it atomically, so this is guidance, not authorization. */
export interface DeletionEligibility {
  key: string;
  version: number;
  eligible: boolean;
  blockers: DeletionBlocker[];
}
export interface Context {
  deletion?: DeletionEligibility;
  issue: Issue;
  product: Product;
  project: Project | null;
  project_progress?: ProjectProgress | null;
  milestone: Milestone | null;
  milestone_progress: MilestoneProgress | null;
  goals: Goal[];
  goal_progress: GoalProgress[];
  issue_links: IssueLinkContext[];
  labels?: Label[];
  project_labels?: Label[];
  comments: { id: string; actor: string; body: string; at: number }[];
  more_comments: boolean;
  verifications: Verification[];
  method_findings: MethodFinding[];
  git_traces?: GitTrace[];
  releases: ReleaseRecord[];
  release_progress: ReleaseProgress[];
  release_evidence: ReleaseEvidence[];
  release_workflow: ReleaseWorkflowConfig | null;
  history: { seq: number; kind: string; actor: string; at: number }[];
}
export interface Snapshot {
  review_ready_runs?: string[];
  workspace_id: string;
  products: Product[];
  projects: Project[];
  project_progress?: ProjectProgress[];
  goals: Goal[];
  goal_progress: GoalProgress[];
  milestones: Milestone[];
  milestone_progress: MilestoneProgress[];
  labels?: Label[];
  theoria_documents: TheoriaDocument[];
  method_findings: MethodFinding[];
  git_traces?: GitTrace[];
  releases: ReleaseRecord[];
  release_progress: ReleaseProgress[];
  release_evidence: ReleaseEvidence[];
  release_workflows: ReleaseWorkflowConfig[];
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
  labels?: string[];
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
