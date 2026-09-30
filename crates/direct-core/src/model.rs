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
    pub canceled: u64,
    pub completion_percent: u8,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub key: String,
    pub product_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub planning_scope: PlanningScope,
    #[serde(default)]
    pub theoria_refs: Vec<TheoriaReference>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub issue_key: String,
    pub actor: String,
    pub body: String,
    pub at: i64,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub outcome: Outcome,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verification {
    pub id: String,
    pub issue_key: String,
    pub build_ref: String,
    pub delivery_ref: String,
    pub summary: String,
    pub checks: String,
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
    pub theoria_documents: Vec<TheoriaDocument>,
    #[serde(default)]
    pub method_findings: Vec<MethodFinding>,
    #[serde(default)]
    pub git_traces: Vec<GitTrace>,
    pub issues: Vec<Issue>,
    pub comments: Vec<Comment>,
    pub verifications: Vec<Verification>,
    pub events: Vec<Event>,
    pub requests: Vec<Replay>,
}
