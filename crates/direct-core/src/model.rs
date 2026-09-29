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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub product_id: String,
    pub name: String,
    pub description: String,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub key: String,
    pub product_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
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
    },
    UpdateProject {
        id: String,
        expected_version: u64,
        name: String,
        description: String,
    },
    SetIssueProject {
        key: String,
        expected_version: u64,
        project_id: Option<String>,
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
    },
    UpdateIssue {
        key: String,
        expected_version: u64,
        title: String,
        body: String,
        acceptance: String,
        owner: String,
        priority: String,
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
    pub issues: Vec<Issue>,
    pub comments: Vec<Comment>,
    pub verifications: Vec<Verification>,
    pub events: Vec<Event>,
    pub requests: Vec<Replay>,
}
