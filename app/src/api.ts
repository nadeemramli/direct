import { invoke, isTauri } from "@tauri-apps/api/core";
import { SessionExpired } from "./connection";
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
  /** Sidebar position (DIR-71); older services omit it. */
  sort_order?: number;
  section_id?: string | null;
}
/** Context documents (DIR-23): typed links to durable documents. */
export type ContextTargetKind = "issue" | "project" | "goal" | "release";
export type ContextSource =
  | { kind: "obsidian"; product_id: string; path: string }
  | { kind: "retained_record"; record_id: string }
  | { kind: "url"; url: string };
export interface ContextObservation {
  available: boolean;
  fingerprint?: string | null;
  bytes?: number | null;
  checked_at: number;
  reason: string;
}
export interface ContextLink {
  id: string;
  target_kind: ContextTargetKind;
  target: string;
  source: ContextSource;
  title: string;
  note: string;
  source_id?: string | null;
  url?: string | null;
  pinned_fingerprint?: string | null;
  observation: ContextObservation;
  version: number;
  created_by: string;
  created_at: number;
  updated_at: number;
}
/** Customer requests captured as provenance-bearing signals (DIR-24). */
export type SignalSourceKind =
  | "email"
  | "call"
  | "chat"
  | "support"
  | "sales"
  | "interview"
  | "survey"
  | "social"
  | "other";
export type SignalTargetKind = "issue" | "project";
export interface SignalLink {
  kind: SignalTargetKind;
  /** Issue key or project ID. */
  target: string;
  linked_by: string;
  linked_at: number;
}
export interface UnresolvedMapping {
  external_source: string;
  external_id: string;
  note: string;
}
export interface CustomerSignal {
  id: string;
  product_id: string;
  source_kind: SignalSourceKind;
  source_reference: string;
  summary: string;
  received_at: number;
  customer_reference: string;
  external_source?: string | null;
  external_id?: string | null;
  unresolved_mappings: UnresolvedMapping[];
  links: SignalLink[];
  promoted_issue_key?: string | null;
  archived: boolean;
  version: number;
  created_by: string;
  created_at: number;
  updated_at: number;
}
/** An owner-defined sidebar group of products (DIR-71). */
export interface ProductSection {
  id: string;
  name: string;
  sort_order: number;
  version: number;
  created_at: number;
  updated_at: number;
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
  intake?: IntakeContext;
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
  /** Exact intake template provenance; absent for untemplated work. */
  template?: TemplateUse;
}
export interface IntakeImage { data_url: string; caption: string }
export interface IntakeContext { text: string; images: IntakeImage[] }
export interface DraftBriefResult {
  problem: string; expected_outcome: string; acceptance: string[]; questions: string[]; model: string;
}
export async function draftBrief(input: {
  title: string; intake: IntakeContext; body: string; acceptance: string; product: string; project: string;
}, signal?: AbortSignal): Promise<DraftBriefResult> {
  if (isTauri()) return abandonable(invoke<DraftBriefResult>("direct_draft_brief", { input }), signal);
  const response = await fetch("/api/draft-brief", {
    method: "POST", headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
    body: JSON.stringify(input), signal,
  });
  const result = await response.json().catch(() => undefined);
  if (!response.ok || !result) throw new Error(result?.message || "AI drafting failed; your input is unchanged.");
  return result as DraftBriefResult;
}
export type TemplateTarget = "issue" | "project";
export type TemplateShape = "delivery" | "discovery_probe" | "bug" | "release";
export type ExecutionMode = "agent" | "owner" | "paired" | "prototype";
export interface TemplateContent {
  intent: string;
  execution_mode: ExecutionMode | null;
  boundaries: string;
  verification: string;
  checklist: string[];
  suggested_priority: string | null;
  suggested_planning_scope: PlanningScope | null;
  suggested_labels: string[];
}
export interface TemplateSupplement {
  product_id: string;
  note: string;
  checklist: string[];
  suggested_labels: string[];
}
export interface WorkspaceTemplate {
  id: string;
  target: TemplateTarget;
  name: string;
  shape: TemplateShape;
  status: "active" | "retired";
  current_revision: number;
  retired_reason: string | null;
  retired_by: string | null;
  retired_at: number | null;
  version: number;
  created_at: number;
  updated_at: number;
}
export interface TemplateRevision {
  template_id: string;
  revision: number;
  target: TemplateTarget;
  name: string;
  description: string;
  shape: TemplateShape;
  content: TemplateContent;
  supplements: TemplateSupplement[];
  note: string;
  created_by: string;
  created_at: number;
}
export interface TemplateSelection {
  template_id: string;
  revision: number;
  /** Omit to accept the suggestion; null explicitly clears it. */
  execution_mode?: ExecutionMode | null;
  labels?: string[];
}
export interface TemplateUse {
  template_id: string;
  revision: number;
  supplement_product_id: string | null;
  execution_mode: ExecutionMode | null;
  overrides: string[];
  applied_by: string;
  applied_at: number;
}
export interface TemplateProvenance {
  use: TemplateUse;
  template: WorkspaceTemplate;
  revision: TemplateRevision;
  current_revision: number;
  outdated: boolean;
  retired: boolean;
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
  /** Snapshot polling omits large imported history; context keeps it. */
  history_entries?: number;
  history_omitted?: boolean;
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
  /** Linked from another product through workspace sharing (DIR-57). */
  shared?: boolean;
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
  /** Owner shared it with every product (DIR-57). */
  shared?: boolean;
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
  | "release_evidence"
  | "customer_signals"
  | "context_links"
  | "cloud_handoffs"
  | "assignments"
  | "agent_runs"
  | "planner_findings";
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
export type CloudHandoffStatus = "prepared" | "reconciled" | "withdrawn";
export interface CloudGuidancePin {
  document_id: string;
  title: string;
  relative_path: string;
  recorded_fingerprint: string | null;
  playbook_version: string | null;
}
export interface CloudPacket {
  format: number;
  issue_key: string;
  product_key: string;
  title: string;
  body: string;
  acceptance: string;
  issue_version: number;
  claim_actor: string;
  claim_expires_at: number;
  repository: string;
  base_ref: string;
  required_model: string;
  guidance: CloudGuidancePin[];
  evidence_plan: string;
  constraints: string[];
}
export interface CloudCheck {
  name: string;
  outcome: "passed" | "failed" | "skipped";
  environment: "cloud" | "local";
  detail: string;
}
export interface CloudReconciliation {
  session_id: string;
  model: string;
  pr_url: string;
  tested_sha: string;
  checks: CloudCheck[];
  cloud_verdict: "passed" | "failed" | "blocked";
  summary: string;
  reconciled_by: string;
  reconciled_at: number;
}
export interface CloudHandoff {
  id: string;
  issue_key: string;
  product_id: string;
  version: number;
  status: CloudHandoffStatus;
  packet: CloudPacket;
  packet_sha256: string;
  brief_sha256: string;
  prepared_by: string;
  prepared_at: number;
  updated_at: number;
  reconciliation: CloudReconciliation | null;
  withdrawn_reason: string | null;
}
export interface Context {
  deletion?: DeletionEligibility;
  /** Customer requests linked to or promoted into this issue (DIR-24). */
  customer_signals?: CustomerSignal[];
  /** Context documents on the issue and inherited from its project, goals and releases. */
  context_links?: ContextLink[];
  /** Bounded cloud-session packets and their reconciliation (DIR-58). */
  cloud_handoffs?: CloudHandoff[];
  assignment?: AssignmentContext | null;
  agent_runs?: AgentRun[];
  planner_findings?: PlannerFinding[];
  context_authority?: string;
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
  retained_sources?: SourceRecordSummary[];
  template?: TemplateProvenance | null;
  project_template?: TemplateProvenance | null;
}
export interface SourceBundleOverview {
  id: string;
  source: string;
  label: string;
  captured_at: string;
  manifest_sha256: string;
  file_count: number;
  total_bytes: number;
  record_count: number;
  imported_by: string;
  imported_at: number;
  applied_cursor: number | null;
}
export interface SourceFileMeta {
  bundle_id: string;
  path: string;
  sha256: string;
  bytes: number;
  content_type: string;
  role: string;
  original_name?: string | null;
}
export interface SourceBundleDetail extends SourceBundleOverview {
  summary: Record<string, any>;
  application: {
    artifact_sha256: string;
    baseline_cursor: number;
    applied_cursor: number;
    backup: string | null;
    rollback_available: boolean;
    entities: Record<string, number>;
  } | null;
  records_by_access: Record<string, number>;
  records_by_kind: Record<string, number>;
  files: SourceFileMeta[];
}
export interface SourceRecordSummary {
  id: string;
  bundle_id: string;
  kind: string;
  level: string;
  source_id: string | null;
  label: string | null;
  title: string | null;
  classification: string;
  access: string;
  issue_keys: string[];
  download_path: string | null;
}
export interface SourceRecordView {
  record: SourceRecordSummary & {
    file: string;
    pointer: string;
    reasons: string[];
    preserved_fields: string[];
    direct: unknown;
  };
  file: SourceFileMeta;
  download: SourceFileMeta | null;
  content: string;
  content_bytes: number;
  truncated: boolean;
  component: unknown;
  readable: { field: string; text: string }[];
  history_entries: number | null;
  authority: string;
}
export interface MigrationPreview {
  status: "ready_to_apply" | "already_applied";
  artifact_sha256: string;
  workspace_id: string;
  prepared_baseline_cursor: number;
  expected_cursor: number;
  workspace_changed_since_preparation: boolean;
  bundle: { id: string; source: string; captured_at: string; manifest_sha256: string };
  counts: Record<string, any>;
  summary: Record<string, any>;
}
export interface Retirement { by: string; at: number; reason: string }
export interface SkillFile { path: string; sha256: string; content: string }
export interface SkillPackage {
  id: string;
  name: string;
  revision: number;
  description: string;
  trigger: string;
  origin: "local" | "upstream";
  upstream: { repository: string; commit: string; path: string } | null;
  license: string;
  adaptations: string;
  files: SkillFile[];
  bundle_sha256: string;
  registered_by: string;
  registered_at: number;
  retired: Retirement | null;
}
export interface RoleGuidancePin {
  document_id: string;
  recorded_fingerprint: string | null;
  playbook_version: string | null;
  mandatory: boolean;
}
export type RoleStatus = "draft" | "active" | "superseded" | "retired";
export interface AgentRole {
  id: string;
  key: string;
  revision: number;
  name: string;
  responsibilities: string[];
  inputs: string[];
  outputs: string[];
  skills: string[];
  runtime_compatibility: string[];
  guidance: RoleGuidancePin[];
  owner_direction: string;
  status: RoleStatus;
  registered_by: string;
  registered_at: number;
  activation: { by: string; at: number; note: string } | null;
  retired: Retirement | null;
}
export interface ActivationEvidence {
  session_id: string;
  model: string;
  harness_version: string;
  marker: string;
  output_excerpt: string;
  output_sha256: string;
  recorded_by: string;
  recorded_at: number;
}
export interface RolePublication {
  id: string;
  role_id: string;
  harness: string;
  destination: string;
  introduced: { path: string; sha256: string }[];
  published_by: string;
  published_at: number;
  evidence: ActivationEvidence[];
  rollback: { by: string; at: number; removed: string[]; kept_modified: string[] } | null;
}
export interface MemberCapability {
  harness_version: string;
  verified_models: string[];
  evidence: string;
  checked_by: string;
  checked_at: number;
}
export interface AgentMember {
  id: string;
  name: string;
  runtime: string;
  enabled: boolean;
  connection_ref: string;
  product_ids: string[];
  default_role_key: string | null;
  default_model: string | null;
  capability: MemberCapability | null;
  version: number;
  created_at: number;
  updated_at: number;
}
export interface IssueAssignment {
  id: string;
  issue_key: string;
  member_id: string;
  role_id: string;
  requested_model: string;
  status: "active" | "superseded" | "cleared";
  version: number;
  assigned_by: string;
  assigned_at: number;
  reconciliation: string | null;
  sessions: { session_id: string; model: string; recorded_by: string; recorded_at: number }[];
  cleared: Retirement | null;
  updated_at: number;
}
/** An issue's resolved assignment in context (DIR-75). */
export interface AssignmentContext {
  assignment: IssueAssignment;
  member: { id: string; name: string; runtime: string; enabled: boolean } | null;
  role: { id: string; key: string; revision: number; name: string; status: RoleStatus } | null;
  requested_model: string;
  actual_model: string | null;
}
export type RunState =
  | "intent"
  | "launching"
  | "running"
  | "succeeded"
  | "failed"
  | "blocked"
  | "unknown"
  | "cancel_pending"
  | "canceled";
export interface AgentRun {
  id: string;
  issue_key: string;
  assignment_id: string;
  member_id: string;
  role_id: string;
  skill_bundles: string[];
  guidance: RoleGuidancePin[];
  requested_model: string;
  fallback_models: string[];
  input_issue_version: number;
  objective: string;
  state: RunState;
  version: number;
  created_by: string;
  created_at: number;
  launcher: string | null;
  harness_version: string | null;
  session_id: string | null;
  actual_model: string | null;
  actions: { index: number; proposal: unknown; outcome: string; detail: string; at: number }[];
  summary: string;
  reason: string | null;
  cancel_requested_by: string | null;
  queue: RunQueue | null;
  updated_at: number;
}
export interface PlannerFinding {
  id: string;
  product_id: string;
  issue_key: string;
  kind: string;
  route: string | null;
  summary: string;
  evidence: string[];
  recommendation: string;
  escalation: unknown;
  confirmation: { by: string; at: number; confirmed: boolean; note: string } | null;
  input_fingerprint: string;
  first_run: string;
  last_run: string;
  seen: number;
  revisions: { run_id: string; at: number; summary: string; input_fingerprint: string }[];
  version: number;
  created_at: number;
  updated_at: number;
}
export interface RunQueue {
  product_id: string;
  keys: string[];
  versions: number[];
  policy: "inspect_only" | "refine_backlog";
  blocked: { key: string; reason: string }[];
}
export interface Snapshot {
  review_ready_runs?: string[];
  workspace_id: string;
  products: Product[];
  product_sections?: ProductSection[];
  customer_signals?: CustomerSignal[];
  context_links?: ContextLink[];
  /** Role and skill revisions with their publications (DIR-74). */
  skill_packages?: SkillPackage[];
  agent_roles?: AgentRole[];
  role_publications?: RolePublication[];
  /** Agent members and issue assignments (DIR-75). */
  agent_members?: AgentMember[];
  issue_assignments?: IssueAssignment[];
  agent_runs?: AgentRun[];
  planner_findings?: PlannerFinding[];
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
  templates?: WorkspaceTemplate[];
  template_revisions?: TemplateRevision[];
  issues: Issue[];
  source_bundles?: SourceBundleOverview[];
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
  template?: TemplateUse;
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
// A launch grant is single-use and expires quickly. It is kept only until
// Direct answers the exchange, so an exchange lost to an outage is retried;
// Direct itself refuses a grant that was already used.
let launchGrant: string | null = null;
const NEW_LINK =
  "Open Direct from the desktop app or run “direct open” to get a fresh local launch link.";
export async function connect(signal?: AbortSignal) {
  if (isTauri()) return;
  const grant = new URLSearchParams(location.hash.slice(1)).get("grant");
  if (grant) {
    launchGrant = grant;
    history.replaceState(null, "", location.pathname);
  }
  if (launchGrant) {
    const response = await fetch("/api/session", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ grant: launchGrant }),
      signal,
    }).catch((e) => {
      throw signal?.aborted ? e : new DirectError(UNREACHABLE, "unknown", "unavailable");
    });
    const data = await response.json().catch(() => undefined);
    signal?.throwIfAborted();
    if (response.status < 500 && !response.ok) {
      launchGrant = null;
      throw new SessionExpired(
        `${data?.message || "This launch link can no longer be used."} ${NEW_LINK}`,
      );
    }
    if (!response.ok || !data?.token)
      throw new DirectError(UNREACHABLE, "unknown", "unavailable");
    launchGrant = null;
    token = data.token;
    sessionStorage.setItem("direct.session", token);
  }
  if (!token) throw new SessionExpired(NEW_LINK);
}
/**
 * A failed command. `outcome` says what the caller may assume:
 * - `rejected`: Direct answered with a definitive refusal; nothing changed.
 * - `unknown`: no usable answer (connection lost, reply dropped, service
 *   failure). The write may have been applied. Retrying the exact command
 *   reuses its request ID, so Direct applies it at most once.
 */
export class DirectError extends Error {
  constructor(
    message: string,
    readonly outcome: "rejected" | "unknown",
    readonly code = "",
  ) {
    super(message);
  }
}
const UNKNOWN =
  "Direct did not confirm the result (the connection was lost before a reply).";
const UNREACHABLE = "Direct is unavailable right now. Check that the local service is running.";
function asDirectError(error: unknown, mutation: boolean): DirectError {
  if (error instanceof DirectError) return error;
  // Desktop transport: `{code, message, outcome}` (direct::command_failure).
  if (error && typeof error === "object" && "outcome" in error) {
    const e = error as { code?: string; message?: string; outcome?: string };
    return new DirectError(
      e.message || (mutation ? UNKNOWN : UNREACHABLE),
      e.outcome === "rejected" ? "rejected" : "unknown",
      e.code || "",
    );
  }
  return new DirectError(mutation ? UNKNOWN : UNREACHABLE, "unknown", "unavailable");
}
/**
 * Request identity is kept per user operation (`intent`) and exact payload
 * until Direct gives a definitive answer, so retrying an unconfirmed write
 * replays it instead of applying it twice, while a separate operation with
 * identical content still gets its own request.
 */
export async function api<T = unknown>(
  command: Record<string, unknown>,
  mutation = false,
  intent = "",
  /** Abandons a read (never a write) when it fires, including its body. */
  signal?: AbortSignal,
): Promise<T> {
  const read = mutation ? undefined : signal;
  const key = `${intent}\u0000${JSON.stringify(command)}`;
  const requestId = mutation ? pending.get(key) || crypto.randomUUID() : "";
  if (mutation) pending.set(key, requestId);
  const request = { actor: "owner", request_id: requestId, ...command };
  try {
    const data = isTauri()
      ? await abandonable(invoke<T>("direct_command", { request }), read)
      : await post<T>(request, mutation, read);
    pending.delete(key);
    return data;
  } catch (e) {
    if (e instanceof SessionExpired) throw e;
    const error = asDirectError(e, mutation);
    if (error.outcome === "rejected") pending.delete(key);
    throw error;
  }
}
// A write that gets no reply in this time is reported as an unknown outcome
// (like the desktop transport's timeout) instead of holding its form forever.
const WRITE_TIMEOUT_MS = 20_000;
/**
 * The desktop IPC call cannot be cancelled (its client times out by itself),
 * so an abandoned read just stops being awaited; its late result is dropped.
 */
function abandonable<T>(work: Promise<T>, signal?: AbortSignal): Promise<T> {
  if (!signal) return work;
  signal.throwIfAborted();
  return new Promise<T>((resolve, reject) => {
    const abandon = () => reject(signal.reason);
    signal.addEventListener("abort", abandon, { once: true });
    work
      .then(resolve, reject)
      .finally(() => signal.removeEventListener("abort", abandon));
  });
}
async function post<T>(
  request: Record<string, unknown>,
  mutation: boolean,
  read?: AbortSignal,
): Promise<T> {
  const response = await fetch("/api/command", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${token}`,
    },
    body: JSON.stringify(request),
    signal: mutation ? AbortSignal.timeout(WRITE_TIMEOUT_MS) : read,
  });
  const value = await response.json().catch(() => undefined);
  if (response.ok) {
    if (value === undefined) throw new DirectError(UNKNOWN, "unknown");
    return value as T;
  }
  // The service no longer accepts this browser session (it restarted or the
  // session ended). A write keeps its definitive refusal below.
  if (response.status === 401 && !mutation) {
    token = "";
    sessionStorage.removeItem("direct.session");
    throw new SessionExpired(
      `This browser session is no longer valid; Direct may have restarted or the session ended. ${NEW_LINK}`,
    );
  }
  // Only a client-error status is a definitive refusal; a service failure
  // may have happened after the write committed.
  throw new DirectError(
    value?.message || "Direct could not complete this action.",
    response.status < 500 ? "rejected" : "unknown",
    value?.code || "",
  );
}

async function failure(response: Response): Promise<Error> {
  const value = await response.json().catch(() => ({}));
  return new Error(value.message || "Direct could not complete this action.");
}
/** Retained source file bytes, fetched with the local session. */
export async function sourceFile(bundleId: string, path: string): Promise<Blob> {
  if (isTauri()) {
    const bytes = await invoke<ArrayBuffer>("direct_source_file", {
      bundleId,
      path,
    });
    return new Blob([bytes], { type: "application/octet-stream" });
  }
  const response = await fetch("/api/source-file", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${token}`,
    },
    body: JSON.stringify({ bundle_id: bundleId, path }),
  });
  if (!response.ok) throw await failure(response);
  // Always an opaque download: never rendered or sniffed as a document.
  return new Blob([await response.arrayBuffer()], {
    type: "application/octet-stream",
  });
}
/** Owner-only migration preview (no `expected`) or apply. */
export async function migration<T>(
  artifact: ArrayBuffer,
  expected?: { cursor: number; sha256: string },
): Promise<T> {
  if (isTauri()) {
    const headers: Record<string, string> = expected
      ? {
          "expected-cursor": String(expected.cursor),
          "artifact-sha256": expected.sha256,
        }
      : {};
    return invoke<T>("direct_migration", new Uint8Array(artifact), { headers });
  }
  const url = expected
    ? `/api/migration/apply?expected_cursor=${expected.cursor}&artifact_sha256=${encodeURIComponent(expected.sha256)}`
    : "/api/migration/preview";
  const response = await fetch(url, {
    method: "POST",
    headers: {
      "Content-Type": "application/octet-stream",
      Authorization: `Bearer ${token}`,
    },
    body: artifact,
  });
  if (!response.ok) throw await failure(response);
  return response.json();
}
