use crate::IntakeContext;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Human,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Backlog,
    Ready,
    Doing,
    Verify,
    Done,
    LegacyCompleted,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    pub id: String,
    pub key: String,
    pub name: String,
    pub repo_windows: String,
    pub repo_wsl: String,
    pub vault_windows: String,
    pub vault_wsl: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub actor: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    Planned,
    #[default]
    Active,
    Paused,
    Completed,
    Canceled,
}

fn default_project_priority() -> String {
    "medium".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub product_id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub status: ProjectStatus,
    #[serde(default = "default_project_priority")]
    pub priority: String,
    #[serde(default)]
    pub sort_order: i64,
    #[serde(default)]
    pub external_source: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub external_url: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    /// Intake template provenance (format 13). Absent for untemplated records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<TemplateUse>,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectProgress {
    pub project_id: String,
    pub total: u64,
    pub backlog: u64,
    pub active: u64,
    pub pending_verification: u64,
    pub completed: u64,
    pub legacy_completed: u64,
    pub canceled: u64,
    pub completion_percent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Planned,
    #[default]
    Active,
    Paused,
    Completed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub product_id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub status: GoalStatus,
    #[serde(default = "default_project_priority")]
    pub priority: String,
    #[serde(default)]
    pub project_ids: Vec<String>,
    #[serde(default)]
    pub external_source: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub external_url: Option<String>,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub description: String,
    pub sort_order: i64,
    #[serde(default)]
    pub external_source: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub external_url: Option<String>,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalProgress {
    pub goal_id: String,
    pub total: u64,
    pub backlog: u64,
    pub active: u64,
    pub pending_verification: u64,
    pub completed: u64,
    pub legacy_completed: u64,
    pub canceled: u64,
    pub completion_percent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MilestoneProgress {
    pub milestone_id: String,
    pub total: u64,
    pub backlog: u64,
    pub active: u64,
    pub pending_verification: u64,
    pub completed: u64,
    pub legacy_completed: u64,
    pub canceled: u64,
    pub completion_percent: u8,
}

/// Optional per-product rule for a workspace-level label. A label with no rules
/// applies to every product; listed rules restrict it to those products.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LabelProductRule {
    pub product_id: String,
    /// Attach automatically to new issues captured in this product.
    #[serde(default)]
    pub default_for_new_issues: bool,
}

/// Original Linear label identity preserved for a later import.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LinearLabelOrigin {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// One canonical definition shared by issues and projects across products.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub products: Vec<LabelProductRule>,
    #[serde(default)]
    pub linear_origins: Vec<LinearLabelOrigin>,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanningScope {
    Project,
    #[default]
    Inbox,
}

/// The record kind a workspace template shapes. Fixed for the template's lifetime.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateTarget {
    Issue,
    Project,
}

/// The intake shape a template revision describes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateShape {
    Delivery,
    DiscoveryProbe,
    Bug,
    Release,
}

/// How the work is expected to be carried out. `prototype` keeps the
/// prototype-as-planning route: a bounded probe whose outcome is learning.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Agent,
    Owner,
    Paired,
    Prototype,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TemplateStatus {
    #[default]
    Active,
    Retired,
}

/// The shared base of one template revision: concise prompts and suggestions only.
/// Nothing here grants authority, makes work Ready, or counts as verification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct TemplateContent {
    #[serde(default)]
    pub intent: String,
    #[serde(default)]
    pub execution_mode: Option<ExecutionMode>,
    #[serde(default)]
    pub boundaries: String,
    #[serde(default)]
    pub verification: String,
    #[serde(default)]
    pub checklist: Vec<String>,
    #[serde(default)]
    pub suggested_priority: Option<String>,
    /// Issue templates only.
    #[serde(default)]
    pub suggested_planning_scope: Option<PlanningScope>,
    /// Workspace label IDs.
    #[serde(default)]
    pub suggested_labels: Vec<String>,
}

/// An explicit, bounded, additive product supplement. It cannot replace any
/// base field; it only appends guidance, checklist items and label suggestions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TemplateSupplement {
    pub product_id: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub checklist: Vec<String>,
    #[serde(default)]
    pub suggested_labels: Vec<String>,
}

/// Mutable head of an owner-managed template. Definitions live in immutable revisions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceTemplate {
    pub id: String,
    pub target: TemplateTarget,
    /// Mirrors the current revision for listing.
    pub name: String,
    pub shape: TemplateShape,
    pub status: TemplateStatus,
    pub current_revision: u32,
    #[serde(default)]
    pub retired_reason: Option<String>,
    #[serde(default)]
    pub retired_by: Option<String>,
    #[serde(default)]
    pub retired_at: Option<i64>,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One immutable template revision. Never rewritten or deleted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TemplateRevision {
    pub template_id: String,
    pub revision: u32,
    pub target: TemplateTarget,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub shape: TemplateShape,
    pub content: TemplateContent,
    #[serde(default)]
    pub supplements: Vec<TemplateSupplement>,
    #[serde(default)]
    pub note: String,
    pub created_by: String,
    pub created_at: i64,
}

/// What a creator asks for when applying a template at intake.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TemplateSelection {
    pub template_id: String,
    /// Must be the template's current revision; a stale form is refused.
    pub revision: u32,
    /// Omit to accept the template's suggestion; `null` explicitly clears it;
    /// a mode explicitly chooses it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "explicit_value"
    )]
    pub execution_mode: Option<Option<ExecutionMode>>,
    /// Suggested labels (base or product supplement) the creator keeps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

/// Distinguishes an explicit `null` (`Some(None)`) from an absent field (`None`).
fn explicit_value<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Exact provenance recorded on an issue or project created from a template.
/// Later revisions never rewrite it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TemplateUse {
    pub template_id: String,
    pub revision: u32,
    /// Set when the revision carried a supplement for the record's product.
    #[serde(default)]
    pub supplement_product_id: Option<String>,
    #[serde(default)]
    pub execution_mode: Option<ExecutionMode>,
    /// Suggested fields the creator explicitly changed: `priority`,
    /// `planning_scope`, `execution_mode`, `labels`.
    #[serde(default)]
    pub overrides: Vec<String>,
    pub applied_by: String,
    pub applied_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TheoriaAvailability {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoriaDocumentInput {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub category: String,
    pub relative_path: String,
    #[serde(default)]
    pub source_updated: Option<String>,
    #[serde(default)]
    pub source_modified_at: Option<i64>,
    #[serde(default)]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoriaDocument {
    pub id: String,
    pub product_id: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub source_root: String,
    pub relative_path: String,
    pub source_updated: Option<String>,
    pub source_modified_at: Option<i64>,
    pub fingerprint: Option<String>,
    pub content: Option<String>,
    pub availability: TheoriaAvailability,
    pub unavailable_reason: Option<String>,
    pub catalog_version: u32,
    pub checked_at: i64,
    pub cached_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoriaReference {
    pub document_id: String,
    pub recorded_fingerprint: Option<String>,
    pub playbook_version: Option<String>,
    pub linked_by: String,
    pub linked_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingClassification {
    ProductDefect,
    MethodFriction,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Issue,
    Build,
    Check,
    OwnerReview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidencePointer {
    pub kind: EvidenceKind,
    pub reference: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodFinding {
    pub id: String,
    pub issue_key: String,
    pub classification: FindingClassification,
    pub observation: String,
    pub hypothesis: String,
    pub proposal: String,
    pub evidence: Vec<EvidencePointer>,
    pub created_by: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GitTraceKind {
    Commit,
    Push,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitTrace {
    pub id: String,
    pub issue_key: String,
    pub kind: GitTraceKind,
    pub repository: String,
    pub commit_sha: String,
    pub branch: String,
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(default)]
    pub remote_ref: Option<String>,
    pub recorded_by: String,
    pub recorded_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseStatus {
    #[default]
    Planned,
    Active,
    Preview,
    Production,
    Retired,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseRecord {
    pub id: String,
    pub product_id: String,
    pub name: String,
    pub version_label: String,
    pub status: ReleaseStatus,
    pub target_ref: String,
    #[serde(default)]
    pub release_branch: Option<String>,
    #[serde(default)]
    pub preview_url: Option<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub project_ids: Vec<String>,
    #[serde(default)]
    pub issue_keys: Vec<String>,
    #[serde(default)]
    pub external_source: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub external_url: Option<String>,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseEvidenceKind {
    Commit,
    Push,
    Check,
    PreviewDeployment,
    ProductionDeployment,
    Rollback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseEvidence {
    pub id: String,
    pub release_id: String,
    pub kind: ReleaseEvidenceKind,
    #[serde(default)]
    pub issue_key: Option<String>,
    #[serde(default)]
    pub git_trace_id: Option<String>,
    #[serde(default)]
    pub verification_id: Option<String>,
    #[serde(default)]
    pub deployment_ref: Option<String>,
    #[serde(default)]
    pub commit_sha: Option<String>,
    #[serde(default)]
    pub target_ref: Option<String>,
    #[serde(default)]
    pub source_ref: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub approver: Option<String>,
    #[serde(default = "passed_outcome")]
    pub outcome: Outcome,
    #[serde(default)]
    pub note: String,
    pub recorded_by: String,
    pub recorded_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseBranchStrategy {
    #[default]
    OneBranchPerRelease,
    External,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromotionPolicy {
    #[default]
    VerifiedOwnerApproval,
    ExternalManual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseWorkflowConfig {
    pub product_id: String,
    pub branch_strategy: ReleaseBranchStrategy,
    pub production_ref: String,
    pub release_branch_pattern: String,
    pub preview_environment: String,
    pub preview_url_template: String,
    pub promotion_policy: PromotionPolicy,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseProgress {
    pub release_id: String,
    pub total: u64,
    pub backlog: u64,
    pub active: u64,
    pub pending_verification: u64,
    pub failed_verification: u64,
    pub completed: u64,
    pub legacy_completed: u64,
    pub canceled: u64,
    pub completion_percent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub key: String,
    pub product_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub milestone_id: Option<String>,
    #[serde(default)]
    pub planning_scope: PlanningScope,
    #[serde(default)]
    pub theoria_refs: Vec<TheoriaReference>,
    #[serde(default)]
    pub labels: Vec<String>,
    pub title: String,
    pub body: String,
    /// Original owner context and pasted screenshots, distinct from the shaped brief.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intake: Option<IntakeContext>,
    pub acceptance: String,
    pub owner: String,
    pub priority: String,
    pub status: Status,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub claim: Option<Claim>,
    pub needs_fix: bool,
    pub parent: Option<String>,
    pub verification_key: Option<String>,
    pub current_run: Option<String>,
    #[serde(default)]
    pub external: Option<ExternalIssueRecord>,
    /// Intake template provenance (format 13). Absent for untemplated records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<TemplateUse>,
}

/// Why an issue cannot be deleted right now. Issue `context` and the atomic
/// `delete_issue` command derive this from the same calculation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeletionBlockerKind {
    /// Only unstarted Backlog or Ready issues can be deleted.
    Status,
    /// A lease that has not yet expired. Expired claims never block.
    ActiveClaim,
    /// Retained discussion history.
    Comments,
    /// Submissions, verification runs, or a linked verification issue.
    VerificationHistory,
    /// Generated verification issues whose parent is this issue.
    VerificationChildren,
    MethodFindings,
    GitTraces,
    /// Parent, blocker, related, or legacy links where this issue is either end.
    IssueLinks,
    /// Releases that list this issue explicitly. Releases that only link the
    /// issue's project are visible in context but are not references.
    ReleaseReferences,
    ReleaseEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeletionBlocker {
    pub kind: DeletionBlockerKind,
    pub count: u64,
    /// Identifiers the owner can act on: issue keys, release IDs, run IDs, commit SHAs.
    pub references: Vec<String>,
    /// True when the owner can clear this blocker without losing retained history
    /// (unlink an issue, remove the issue from a release, release a claim).
    pub removable: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeletionEligibility {
    pub key: String,
    pub version: u64,
    pub eligible: bool,
    pub blockers: Vec<DeletionBlocker>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalIssueState {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalIssueRecord {
    pub source: String,
    pub id: String,
    pub url: String,
    pub state: ExternalIssueState,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub canceled_at: Option<String>,
    #[serde(default)]
    pub archived_at: Option<String>,
    #[serde(default)]
    pub history: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum IssueLinkKind {
    Parent,
    BlockedBy,
    Related,
    LegacyVerification,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueLink {
    pub id: String,
    pub source_key: String,
    pub target_key: String,
    pub kind: IssueLinkKind,
    #[serde(default)]
    pub external_source: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    pub created_by: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub issue_key: String,
    pub actor: String,
    pub body: String,
    pub at: i64,
    #[serde(default)]
    pub external_source: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub external_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub instruction: String,
    pub expected: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pending,
    Passed,
    Failed,
    Canceled,
}
fn passed_outcome() -> Outcome {
    Outcome::Passed
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub outcome: Outcome,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct E2eEvidence {
    pub build_ref: String,
    pub environment: String,
    pub entrypoint: String,
    pub scenarios: String,
    pub outcome: Outcome,
    pub delivered_build_ref: String,
    pub delivery_check: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verification {
    pub id: String,
    pub issue_key: String,
    pub build_ref: String,
    pub delivery_ref: String,
    pub summary: String,
    pub checks: String,
    #[serde(default)]
    pub e2e: Option<E2eEvidence>,
    pub limitations: String,
    pub preconditions: String,
    pub steps: Vec<Step>,
    pub submitted_by: String,
    pub submitted_at: i64,
    pub outcome: Outcome,
    pub results: Vec<StepResult>,
    pub review_note: String,
    pub reviewed_by: Option<String>,
    pub reviewed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub at: i64,
    pub actor: String,
    pub kind: String,
    pub entity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub request_id: String,
    pub actor: String,
    #[serde(flatten)]
    pub command: Command,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Snapshot,
    Context {
        key: String,
    },
    Changes {
        after: u64,
    },
    Export,
    CreateProject {
        product: String,
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default = "default_project_priority")]
        priority: String,
        #[serde(default)]
        sort_order: i64,
        /// Omitted from the request hash when absent so legacy retries replay exactly.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        template: Option<TemplateSelection>,
    },
    UpdateProject {
        id: String,
        expected_version: u64,
        name: String,
        description: String,
        #[serde(default)]
        status: Option<ProjectStatus>,
        #[serde(default)]
        priority: Option<String>,
        #[serde(default)]
        sort_order: Option<i64>,
    },
    SetIssueProject {
        key: String,
        expected_version: u64,
        project_id: Option<String>,
    },
    CreateGoal {
        product: String,
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default = "default_project_priority")]
        priority: String,
        #[serde(default)]
        project_ids: Vec<String>,
        #[serde(default)]
        external_source: Option<String>,
        #[serde(default)]
        external_id: Option<String>,
    },
    UpdateGoal {
        id: String,
        expected_version: u64,
        name: String,
        description: String,
        status: GoalStatus,
        priority: String,
        project_ids: Vec<String>,
    },
    CreateMilestone {
        project_id: String,
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        sort_order: i64,
        #[serde(default)]
        external_source: Option<String>,
        #[serde(default)]
        external_id: Option<String>,
    },
    UpdateMilestone {
        id: String,
        expected_version: u64,
        name: String,
        description: String,
        sort_order: i64,
    },
    SetIssueMilestone {
        key: String,
        expected_version: u64,
        milestone_id: Option<String>,
    },
    CreateIssueLink {
        key: String,
        expected_version: u64,
        target_key: String,
        kind: IssueLinkKind,
        #[serde(default)]
        external_source: Option<String>,
        #[serde(default)]
        external_id: Option<String>,
    },
    DeleteIssueLink {
        key: String,
        expected_version: u64,
        link_id: String,
    },
    CreateLabel {
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        color: String,
        #[serde(default)]
        aliases: Vec<String>,
        #[serde(default)]
        products: Vec<LabelProductRule>,
        #[serde(default)]
        linear_origins: Vec<LinearLabelOrigin>,
    },
    UpdateLabel {
        id: String,
        expected_version: u64,
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        color: String,
        #[serde(default)]
        aliases: Vec<String>,
        #[serde(default)]
        products: Vec<LabelProductRule>,
        #[serde(default)]
        linear_origins: Vec<LinearLabelOrigin>,
    },
    AttachIssueLabel {
        key: String,
        expected_version: u64,
        label_id: String,
    },
    DetachIssueLabel {
        key: String,
        expected_version: u64,
        label_id: String,
    },
    AttachProjectLabel {
        id: String,
        expected_version: u64,
        label_id: String,
    },
    DetachProjectLabel {
        id: String,
        expected_version: u64,
        label_id: String,
    },
    SyncTheoria {
        product: String,
        source_root: String,
        catalog_version: u32,
        documents: Vec<TheoriaDocumentInput>,
    },
    LinkTheoria {
        key: String,
        expected_version: u64,
        document_id: String,
        #[serde(default)]
        playbook_version: Option<String>,
    },
    CreateMethodFinding {
        key: String,
        expected_version: u64,
        classification: FindingClassification,
        observation: String,
        #[serde(default)]
        hypothesis: String,
        proposal: String,
        evidence: Vec<EvidencePointer>,
    },
    RecordGitTrace {
        key: String,
        expected_version: u64,
        kind: GitTraceKind,
        repository: String,
        commit_sha: String,
        branch: String,
        #[serde(default)]
        remote: Option<String>,
        #[serde(default)]
        remote_ref: Option<String>,
    },
    CreateRelease {
        product: String,
        name: String,
        version_label: String,
        target_ref: String,
        #[serde(default)]
        release_branch: Option<String>,
        #[serde(default)]
        preview_url: Option<String>,
        #[serde(default)]
        notes: String,
        #[serde(default)]
        project_ids: Vec<String>,
        #[serde(default)]
        issue_keys: Vec<String>,
        #[serde(default)]
        external_source: Option<String>,
        #[serde(default)]
        external_id: Option<String>,
    },
    UpdateRelease {
        id: String,
        expected_version: u64,
        name: String,
        version_label: String,
        status: ReleaseStatus,
        target_ref: String,
        #[serde(default)]
        release_branch: Option<String>,
        #[serde(default)]
        preview_url: Option<String>,
        #[serde(default)]
        notes: String,
        project_ids: Vec<String>,
        issue_keys: Vec<String>,
    },
    RecordReleaseEvidence {
        release_id: String,
        expected_version: u64,
        kind: ReleaseEvidenceKind,
        #[serde(default)]
        git_trace_id: Option<String>,
        #[serde(default)]
        verification_id: Option<String>,
        #[serde(default)]
        deployment_ref: Option<String>,
        #[serde(default)]
        commit_sha: Option<String>,
        #[serde(default)]
        target_ref: Option<String>,
        #[serde(default)]
        source_ref: Option<String>,
        #[serde(default)]
        environment: Option<String>,
        #[serde(default)]
        url: Option<String>,
        #[serde(default = "passed_outcome")]
        outcome: Outcome,
        #[serde(default)]
        note: String,
    },
    SetReleaseWorkflowConfig {
        product: String,
        #[serde(default)]
        expected_version: Option<u64>,
        branch_strategy: ReleaseBranchStrategy,
        production_ref: String,
        release_branch_pattern: String,
        preview_environment: String,
        preview_url_template: String,
        promotion_policy: PromotionPolicy,
    },
    CreateProduct {
        key: String,
        name: String,
        #[serde(default)]
        repo_windows: String,
        #[serde(default)]
        repo_wsl: String,
        #[serde(default)]
        vault_windows: String,
        #[serde(default)]
        vault_wsl: String,
    },
    CreateIssue {
        product: String,
        title: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        acceptance: String,
        #[serde(default)]
        owner: String,
        #[serde(default = "default_project_priority")]
        priority: String,
        #[serde(default)]
        planning_scope: PlanningScope,
        #[serde(default)]
        project_id: Option<String>,
        /// Omitted from the request hash when absent so legacy retries replay exactly.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        template: Option<TemplateSelection>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        intake: Option<IntakeContext>,
    },
    UpdateIssue {
        key: String,
        expected_version: u64,
        title: String,
        body: String,
        acceptance: String,
        owner: String,
        priority: String,
        #[serde(default)]
        planning_scope: Option<PlanningScope>,
        /// Omission preserves context; an empty object explicitly clears it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        intake: Option<IntakeContext>,
    },
    DeleteIssue {
        key: String,
        expected_version: u64,
    },
    Ready {
        key: String,
        expected_version: u64,
    },
    Claim {
        key: String,
        expected_version: u64,
        #[serde(default = "lease")]
        lease_seconds: i64,
    },
    Renew {
        key: String,
        expected_version: u64,
        #[serde(default = "lease")]
        lease_seconds: i64,
    },
    Release {
        key: String,
        expected_version: u64,
    },
    Comment {
        key: String,
        expected_version: u64,
        body: String,
    },
    Submit {
        key: String,
        expected_version: u64,
        build_ref: String,
        delivery_ref: String,
        summary: String,
        checks: String,
        #[serde(default)]
        e2e: Option<E2eEvidence>,
        #[serde(default)]
        limitations: String,
        #[serde(default)]
        preconditions: String,
        steps: Vec<Step>,
    },
    Review {
        key: String,
        expected_version: u64,
        run_id: String,
        outcome: Outcome,
        results: Vec<StepResult>,
        #[serde(default)]
        note: String,
    },
    Reopen {
        key: String,
        expected_version: u64,
        reason: String,
    },
    /// Read-only list of retained source bundles with their summaries.
    SourceBundles,
    /// Read-only bounded search over retained source records.
    SearchSources {
        #[serde(default)]
        query: String,
        #[serde(default)]
        bundle_id: Option<String>,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        classification: Option<String>,
        #[serde(default)]
        issue_key: Option<String>,
        #[serde(default = "source_page")]
        limit: u32,
        #[serde(default)]
        offset: u32,
    },
    /// Read-only: one retained record with its exact original source bytes.
    SourceRecord {
        id: String,
    },
    /// Owner only: undo an applied migration while nothing has changed since.
    RollbackMigration {
        bundle_id: String,
        expected_cursor: u64,
    },
    /// Read-only: every template head and immutable revision, including retired ones.
    Templates,
    /// Owner only: define a template; its first immutable revision is 1.
    CreateTemplate {
        target: TemplateTarget,
        name: String,
        #[serde(default)]
        description: String,
        shape: TemplateShape,
        content: TemplateContent,
        #[serde(default)]
        supplements: Vec<TemplateSupplement>,
        #[serde(default)]
        note: String,
    },
    /// Owner only: append the next immutable revision. Existing records keep theirs.
    ReviseTemplate {
        id: String,
        expected_version: u64,
        name: String,
        #[serde(default)]
        description: String,
        shape: TemplateShape,
        content: TemplateContent,
        #[serde(default)]
        supplements: Vec<TemplateSupplement>,
        #[serde(default)]
        note: String,
    },
    /// Owner only: stop offering a template for new work. Nothing is deleted.
    RetireTemplate {
        id: String,
        expected_version: u64,
        reason: String,
    },
}
fn source_page() -> u32 {
    50
}
fn lease() -> i64 {
    3600
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replay {
    pub id: String,
    pub actor: String,
    pub role: String,
    pub hash: String,
    pub response: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Archive {
    pub format: u32,
    pub workspace_id: String,
    pub products: Vec<Product>,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub goals: Vec<Goal>,
    #[serde(default)]
    pub milestones: Vec<Milestone>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub theoria_documents: Vec<TheoriaDocument>,
    #[serde(default)]
    pub method_findings: Vec<MethodFinding>,
    #[serde(default)]
    pub git_traces: Vec<GitTrace>,
    #[serde(default)]
    pub releases: Vec<ReleaseRecord>,
    #[serde(default)]
    pub release_evidence: Vec<ReleaseEvidence>,
    #[serde(default)]
    pub release_workflows: Vec<ReleaseWorkflowConfig>,
    #[serde(default)]
    pub issue_links: Vec<IssueLink>,
    /// Workspace intake templates and their immutable revisions (format 13).
    #[serde(default)]
    pub templates: Vec<WorkspaceTemplate>,
    #[serde(default)]
    pub template_revisions: Vec<TemplateRevision>,
    pub issues: Vec<Issue>,
    pub comments: Vec<Comment>,
    pub verifications: Vec<Verification>,
    pub events: Vec<Event>,
    pub requests: Vec<Replay>,
    /// Retained external sources (format 12). Files carry base64 bytes in archives.
    #[serde(default)]
    pub source_bundles: Vec<SourceBundle>,
    #[serde(default)]
    pub source_files: Vec<SourceFile>,
    #[serde(default)]
    pub source_records: Vec<SourceRecord>,
}

/// A verified external source package retained for native, read-only access.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceBundle {
    pub id: String,
    pub source: String,
    pub label: String,
    pub captured_at: String,
    pub manifest_sha256: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub record_count: u64,
    /// Bounded reconciliation summary: preservation, access, freshness and gates.
    #[serde(default)]
    pub summary: serde_json::Value,
    /// Present when the bundle arrived through a migration applied to this workspace.
    #[serde(default)]
    pub application: Option<MigrationApplication>,
    pub imported_by: String,
    pub imported_at: i64,
}

/// IDs of every record a migration added, so replay and rollback are exact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MigrationEntities {
    #[serde(default)]
    pub products: Vec<String>,
    #[serde(default)]
    pub projects: Vec<String>,
    #[serde(default)]
    pub goals: Vec<String>,
    #[serde(default)]
    pub milestones: Vec<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub issues: Vec<String>,
    #[serde(default)]
    pub comments: Vec<String>,
    #[serde(default)]
    pub issue_links: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MigrationApplication {
    pub artifact_sha256: String,
    /// Event cursor the owner confirmed before applying.
    pub baseline_cursor: u64,
    /// Event cursor immediately after the import transaction.
    pub applied_cursor: u64,
    /// File name of the pre-import backup written by the service, when one was written.
    #[serde(default)]
    pub backup: Option<String>,
    pub entities: MigrationEntities,
}

/// One retained file. `data` (base64) is present in archives and absent in metadata views.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceFile {
    pub bundle_id: String,
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub content_type: String,
    /// `manifest`, `data` or `upload`.
    pub role: String,
    #[serde(default)]
    pub original_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

/// Index entry for one source record: an exact byte span of a retained file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceRecord {
    pub id: String,
    pub bundle_id: String,
    pub kind: String,
    /// `record`, `component` or `file`.
    pub level: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// `native`, `transformed`, `preserved` or `unresolved`.
    pub classification: String,
    /// `native` (also a Direct record), `retained` (source view only) or `missing`.
    pub access: String,
    pub file: String,
    pub start: u64,
    pub end: u64,
    #[serde(default)]
    pub pointer: String,
    #[serde(default)]
    pub direct: serde_json::Value,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub preserved_fields: Vec<String>,
    #[serde(default)]
    pub issue_keys: Vec<String>,
    /// Retained file this record describes (for uploaded files).
    #[serde(default)]
    pub download_path: Option<String>,
    #[serde(default)]
    pub search_text: String,
}
