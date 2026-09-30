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
    pub issues: Vec<Issue>,
    pub comments: Vec<Comment>,
    pub verifications: Vec<Verification>,
    pub events: Vec<Event>,
    pub requests: Vec<Replay>,
}
