use crate::*;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

mod context;
mod migration;
mod signals;
mod sources;
mod templates;
pub use migration::{
    artifact_sha256, encode_migration_artifact, parse_migration_artifact, ArtifactFile,
    MigrationArtifact, MIGRATION_MAGIC,
};
pub use sources::{
    safe_source_path, MAX_MIGRATION_ARTIFACT_BYTES, MAX_SOURCE_BUNDLE_BYTES, MAX_SOURCE_FILE_BYTES,
    MAX_SOURCE_RECORD_VIEW_BYTES, SOURCE_CHUNK_BYTES,
};

#[derive(Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}
pub type Result<T> = std::result::Result<T, Error>;
fn err(code: &'static str, message: impl Into<String>) -> Error {
    Error {
        code,
        message: message.into(),
    }
}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        err("storage", e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        err("invalid", e.to_string())
    }
}
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn validate_e2e(evidence: &E2eEvidence, build_ref: &str) -> Result<()> {
    if evidence.outcome != Outcome::Passed {
        return Err(err(
            "invalid",
            "End-to-end checks must pass before submission",
        ));
    }
    if evidence.build_ref != build_ref || evidence.delivered_build_ref != build_ref {
        return Err(err(
            "invalid",
            "Tested, delivered, and submitted builds must match exactly",
        ));
    }
    for (value, name) in [
        (&evidence.build_ref, "E2E build"),
        (&evidence.environment, "E2E environment"),
        (&evidence.entrypoint, "E2E entrypoint"),
        (
            &evidence.scenarios,
            "acceptance scenarios and observed results",
        ),
        (&evidence.delivery_check, "delivered-build check"),
    ] {
        required(value, name)?;
        limited(value, name, 20_000)?;
    }
    Ok(())
}
fn required(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(err("invalid", format!("{field} is required")));
    }
    if value.len() > 100_000 {
        return Err(err("invalid", format!("{field} exceeds 100 KB")));
    }
    Ok(())
}
fn limited(value: &str, field: &str, max: usize) -> Result<()> {
    if value.len() > max {
        return Err(err("invalid", format!("{field} is too long")));
    }
    Ok(())
}
fn valid_priority(value: &str) -> bool {
    ["low", "medium", "high", "urgent"].contains(&value)
}
fn validate_project_fields(name: &str, priority: &str, sort_order: i64) -> Result<()> {
    required(name, "project name")?;
    limited(name, "project name", 160)?;
    if !valid_priority(priority) {
        return Err(err("invalid", "Unknown project priority"));
    }
    if !(0..=1_000_000).contains(&sort_order) {
        return Err(err(
            "invalid",
            "Project order must be between 0 and 1000000",
        ));
    }
    Ok(())
}
fn validate_goal_fields(name: &str, priority: &str) -> Result<()> {
    required(name, "goal name")?;
    limited(name, "goal name", 160)?;
    if !valid_priority(priority) {
        return Err(err("invalid", "Unknown goal priority"));
    }
    Ok(())
}
fn validate_milestone_fields(name: &str, sort_order: i64) -> Result<()> {
    required(name, "milestone name")?;
    limited(name, "milestone name", 160)?;
    if !(0..=1_000_000).contains(&sort_order) {
        return Err(err(
            "invalid",
            "Milestone order must be between 0 and 1000000",
        ));
    }
    Ok(())
}
fn validate_optional_provenance(source: Option<&str>, external_id: Option<&str>) -> Result<()> {
    if source.is_some() != external_id.is_some() {
        return Err(err(
            "invalid",
            "External source and external ID must be provided together",
        ));
    }
    if let (Some(source), Some(external_id)) = (source, external_id) {
        required(source, "external source")?;
        required(external_id, "external ID")?;
        limited(source, "external source", 120)?;
        limited(external_id, "external ID", 512)?;
    }
    Ok(())
}
fn valid_color(value: &str) -> bool {
    value.is_empty()
        || (value.len() == 7
            && value.starts_with('#')
            && value[1..].bytes().all(|c| c.is_ascii_hexdigit()))
}
fn normalized_label_name(value: &str) -> String {
    value.trim().to_lowercase()
}
/// A label definition after validation, with its name, aliases and origins trimmed.
struct LabelFields {
    name: String,
    aliases: Vec<String>,
    products: Vec<LabelProductRule>,
    linear_origins: Vec<LinearLabelOrigin>,
}
fn validate_label_fields(
    name: &str,
    description: &str,
    color: &str,
    aliases: &[String],
    products: &[LabelProductRule],
    linear_origins: &[LinearLabelOrigin],
) -> Result<LabelFields> {
    required(name, "label name")?;
    let name = name.trim().to_string();
    if name.len() > 80 {
        return Err(err("invalid", "Label name must be at most 80 bytes"));
    }
    limited(description, "label description", 1_000)?;
    if !valid_color(color) {
        return Err(err("invalid", "Label color must be #rrggbb or empty"));
    }
    if aliases.len() > 20 {
        return Err(err("invalid", "Provide at most 20 label aliases"));
    }
    let mut seen = HashSet::new();
    seen.insert(normalized_label_name(&name));
    let mut trimmed_aliases = Vec::with_capacity(aliases.len());
    for alias in aliases {
        required(alias, "label alias")?;
        let alias = alias.trim().to_string();
        if alias.len() > 80 {
            return Err(err("invalid", "Label aliases must be at most 80 bytes"));
        }
        if !seen.insert(normalized_label_name(&alias)) {
            return Err(err(
                "invalid",
                "Label aliases must differ from each other and from the canonical name",
            ));
        }
        trimmed_aliases.push(alias);
    }
    if products.len() > 50 {
        return Err(err("invalid", "Provide at most 50 label product rules"));
    }
    let mut product_ids = HashSet::new();
    for rule in products {
        required(&rule.product_id, "label product")?;
        if !product_ids.insert(rule.product_id.clone()) {
            return Err(err("invalid", "Each product may appear once per label"));
        }
    }
    if linear_origins.len() > 50 {
        return Err(err("invalid", "Provide at most 50 Linear label origins"));
    }
    let mut origin_ids = HashSet::new();
    let mut trimmed_origins = Vec::with_capacity(linear_origins.len());
    for origin in linear_origins {
        required(&origin.id, "Linear label ID")?;
        let id = origin.id.trim().to_string();
        limited(&id, "Linear label ID", 120)?;
        limited(&origin.name, "Linear label name", 200)?;
        if !origin_ids.insert(id.clone()) {
            return Err(err("invalid", "Each Linear label ID may appear once"));
        }
        trimmed_origins.push(LinearLabelOrigin {
            id,
            name: origin.name.trim().to_string(),
        });
    }
    Ok(LabelFields {
        name,
        aliases: trimmed_aliases,
        products: products.to_vec(),
        linear_origins: trimmed_origins,
    })
}
fn label_applies(label: &Label, product_id: &str) -> bool {
    label.products.is_empty()
        || label
            .products
            .iter()
            .any(|rule| rule.product_id == product_id)
}
fn stable_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
fn valid_fingerprint(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}
fn valid_commit_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|c| c.is_ascii_hexdigit())
}
fn validate_git_trace_fields(
    kind: &GitTraceKind,
    repository: &str,
    commit_sha: &str,
    branch: &str,
    remote: Option<&str>,
    remote_ref: Option<&str>,
) -> Result<()> {
    required(repository, "Git repository")?;
    required(branch, "Git branch")?;
    limited(repository, "Git repository", 512)?;
    limited(branch, "Git branch", 255)?;
    if !valid_commit_sha(commit_sha) {
        return Err(err(
            "invalid",
            "Git commit SHA must be a full 40- or 64-character hexadecimal object ID",
        ));
    }
    match kind {
        GitTraceKind::Commit if remote.is_some() || remote_ref.is_some() => Err(err(
            "invalid",
            "Commit evidence cannot claim a remote or remote ref",
        )),
        GitTraceKind::Push => {
            let remote = remote.ok_or_else(|| err("invalid", "Push evidence needs a remote"))?;
            let remote_ref =
                remote_ref.ok_or_else(|| err("invalid", "Push evidence needs a remote ref"))?;
            required(remote, "Git remote")?;
            required(remote_ref, "Git remote ref")?;
            limited(remote, "Git remote", 255)?;
            limited(remote_ref, "Git remote ref", 512)
        }
        GitTraceKind::Commit => Ok(()),
    }
}
fn validate_release_fields(
    name: &str,
    version_label: &str,
    target_ref: &str,
    preview_url: Option<&str>,
    notes: &str,
) -> Result<()> {
    required(name, "release name")?;
    required(version_label, "release version")?;
    required(target_ref, "release target ref")?;
    limited(name, "release name", 160)?;
    limited(version_label, "release version", 80)?;
    limited(target_ref, "release target ref", 512)?;
    limited(notes, "release notes", 20_000)?;
    if preview_url.is_some_and(|url| url.trim().is_empty() || url.len() > 2_000) {
        return Err(err(
            "invalid",
            "Preview URL must be non-empty and at most 2000 bytes",
        ));
    }
    Ok(())
}
fn validate_release_workflow_fields(
    strategy: &ReleaseBranchStrategy,
    production_ref: &str,
    pattern: &str,
    preview_environment: &str,
    preview_url_template: &str,
) -> Result<()> {
    required(production_ref, "production ref")?;
    required(preview_environment, "preview environment")?;
    limited(production_ref, "production ref", 512)?;
    limited(preview_environment, "preview environment", 120)?;
    limited(pattern, "release branch pattern", 512)?;
    limited(preview_url_template, "preview URL template", 2_000)?;
    if *strategy == ReleaseBranchStrategy::OneBranchPerRelease
        && (!pattern.contains("{version}") || pattern.matches("{version}").count() != 1)
    {
        return Err(err(
            "invalid",
            "One-branch-per-release patterns require exactly one {version} placeholder",
        ));
    }
    if !preview_url_template.is_empty() && preview_url_template.matches("{version}").count() > 1 {
        return Err(err(
            "invalid",
            "Preview URL template has repeated {version}",
        ));
    }
    Ok(())
}
fn human(role: Role) -> Result<()> {
    if role != Role::Human {
        Err(err("forbidden", "This operation requires human review"))
    } else {
        Ok(())
    }
}

/// The archive format this build exports. Format 15 adds the sidebar
/// arrangement (product order and sections); every older format restores.
pub const ARCHIVE_FORMAT: u32 = 17;

pub struct Store {
    conn: Connection,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
          CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS products (id TEXT PRIMARY KEY, key TEXT UNIQUE NOT NULL, data TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS issues (id TEXT PRIMARY KEY, key TEXT UNIQUE NOT NULL, data TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS comments (id TEXT PRIMARY KEY, data TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS verifications (id TEXT PRIMARY KEY, data TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS events (seq INTEGER PRIMARY KEY AUTOINCREMENT, at INTEGER NOT NULL, actor TEXT NOT NULL, kind TEXT NOT NULL, entity TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS requests (id TEXT PRIMARY KEY, actor TEXT NOT NULL, role TEXT NOT NULL, hash TEXT NOT NULL, response TEXT NOT NULL);")?;
        let schema: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key='schema'", [], |r| {
                r.get(0)
            })
            .optional()?;
        let upgrade_legacy_project_planning = schema
            .as_deref()
            .is_some_and(|schema| matches!(schema, "1" | "2" | "3" | "4"));
        if let Some(schema) = schema.as_deref() {
            if !matches!(
                schema,
                "1" | "2"
                    | "3"
                    | "4"
                    | "5"
                    | "6"
                    | "7"
                    | "8"
                    | "9"
                    | "10"
                    | "11"
                    | "12"
                    | "13"
                    | "14"
                    | "15"
                    | "16"
            ) {
                return Err(err("unsupported", "Unsupported database schema"));
            }
        } else {
            conn.execute("INSERT INTO meta VALUES ('schema','1')", [])?;
            conn.execute("INSERT INTO meta VALUES ('workspace_id',?1)", [id()])?;
            let p = Product {
                id: id(),
                key: "DIR".into(),
                name: "Direct".into(),
                repo_windows: String::new(),
                repo_wsl: String::new(),
                vault_windows: String::new(),
                vault_wsl: String::new(),
                sort_order: 0,
                section_id: None,
            };
            conn.execute(
                "INSERT INTO products VALUES (?1,?2,?3)",
                params![p.id, p.key, serde_json::to_string(&p)?],
            )?;
        }
        // Upgrade atomically. Older binaries reject newer schemas instead of dropping data.
        let tx = conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS theoria_documents (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS method_findings (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS git_traces (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS issue_links (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS goals (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS milestones (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS releases (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS release_evidence (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS release_workflows (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS labels (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS source_bundles (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS source_files (id TEXT PRIMARY KEY, bundle_id TEXT NOT NULL, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS source_chunks (file_id TEXT NOT NULL, seq INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY(file_id, seq));
             CREATE TABLE IF NOT EXISTS source_records (id TEXT PRIMARY KEY, bundle_id TEXT NOT NULL, kind TEXT NOT NULL, classification TEXT NOT NULL, search TEXT NOT NULL, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS source_record_issues (issue_key TEXT NOT NULL, record_id TEXT NOT NULL, PRIMARY KEY(issue_key, record_id));
             CREATE INDEX IF NOT EXISTS source_records_bundle ON source_records(bundle_id, kind);
             CREATE TABLE IF NOT EXISTS issue_histories (issue_id TEXT PRIMARY KEY, entries INTEGER NOT NULL, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS templates (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS template_revisions (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS product_sections (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS customer_signals (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS context_links (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             UPDATE meta SET value='16' WHERE key='schema';",
        )?;
        if upgrade_legacy_project_planning {
            // Before schema 5, a project link was the only way to express project-scoped work.
            // Preserve that meaning instead of deserializing every legacy issue as Inbox work.
            for mut issue in all::<Issue>(&tx, "issues")? {
                issue.planning_scope = if issue.project_id.is_some() {
                    PlanningScope::Project
                } else {
                    PlanningScope::Inbox
                };
                put_issue(&tx, &issue)?;
            }
        }
        // Imported external history lives beside the issue row so ordinary reads
        // (snapshot, progress, every mutation) never parse it. Move any history
        // still embedded in issue rows exactly once.
        let split: Option<String> = tx
            .query_row(
                "SELECT value FROM meta WHERE key='issue_history_split'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if split.is_none() {
            let mut statement = tx.prepare(
                "SELECT data FROM issues WHERE json_array_length(data, '$.external.history') > 0",
            )?;
            let embedded = statement
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            drop(statement);
            for data in embedded {
                put_issue(&tx, &serde_json::from_str::<Issue>(&data)?)?;
            }
            tx.execute("INSERT INTO meta VALUES ('issue_history_split','1')", [])?;
        }
        tx.commit()?;
        Ok(Self { conn })
    }

    pub fn execute(&mut self, request: Request, role: Role) -> Result<Value> {
        self.execute_at(request, role, now())
    }
    pub fn execute_at(&mut self, request: Request, role: Role, at: i64) -> Result<Value> {
        required(&request.actor, "actor")?;
        if request.actor.len() > 120 {
            return Err(err("invalid", "Actor must be at most 120 characters"));
        }
        match &request.command {
            Command::Snapshot => {
                // One lightweight issue load serves every progress figure.
                let issues = all::<Issue>(&self.conn, "issues")?;
                let review_ready_runs: Vec<String> =
                    all::<Verification>(&self.conn, "verifications")?
                        .into_iter()
                        .filter(|run| run.outcome == Outcome::Pending && run.e2e.is_some())
                        .map(|run| run.id)
                        .collect();
                return Ok(json!({
                    "review_ready_runs":review_ready_runs,
                    "workspace_id":self.workspace_id()?,
                    "products":all::<Product>(&self.conn,"products")?,
                    "product_sections":all::<ProductSection>(&self.conn,"product_sections")?,
                    "customer_signals":all::<CustomerSignal>(&self.conn,"customer_signals")?,
                    "context_links":all::<ContextLink>(&self.conn,"context_links")?,
                    "projects":all::<Project>(&self.conn,"projects")?,
                    "project_progress":all::<Project>(&self.conn,"projects")?.iter().map(|project| project_progress_in(&issues, &project.id)).collect::<Vec<_>>(),
                    "goals":all::<Goal>(&self.conn,"goals")?,
                    "goal_progress":all::<Goal>(&self.conn,"goals")?.iter().map(|goal| goal_progress_in(&issues, goal)).collect::<Vec<_>>(),
                    "milestones":all::<Milestone>(&self.conn,"milestones")?,
                    "milestone_progress":all::<Milestone>(&self.conn,"milestones")?.iter().map(|milestone| milestone_progress_in(&issues, milestone)).collect::<Vec<_>>(),
                    "labels":all::<Label>(&self.conn,"labels")?,
                    "theoria_documents":all::<TheoriaDocument>(&self.conn,"theoria_documents")?,
                    "method_findings":all::<MethodFinding>(&self.conn,"method_findings")?,
                    "git_traces":all::<GitTrace>(&self.conn,"git_traces")?,
                    "releases":all::<ReleaseRecord>(&self.conn,"releases")?,
                    "release_progress":all::<ReleaseRecord>(&self.conn,"releases")?.iter().map(|release| release_progress_in(&issues, release)).collect::<Vec<_>>(),
                    "release_evidence":all::<ReleaseEvidence>(&self.conn,"release_evidence")?,
                    "release_workflows":all::<ReleaseWorkflowConfig>(&self.conn,"release_workflows")?,
                    "issue_links":all::<IssueLink>(&self.conn,"issue_links")?,
                    "templates":all::<WorkspaceTemplate>(&self.conn,"templates")?,
                    "template_revisions":all::<TemplateRevision>(&self.conn,"template_revisions")?,
                    "issues":snapshot_issues(&self.conn, issues)?,
                    "source_bundles":sources::overview(&self.conn)?,
                    "cursor":cursor(&self.conn)?
                }));
            }
            Command::Context { key } => {
                let mut issue = issue(&self.conn, key)?;
                attach_history(&self.conn, &mut issue)?;
                let project = issue
                    .project_id
                    .as_deref()
                    .map(|id| project(&self.conn, id))
                    .transpose()?;
                let product = all::<Product>(&self.conn, "products")?
                    .into_iter()
                    .find(|p| p.id == issue.product_id);
                let mut comments: Vec<_> = all::<Comment>(&self.conn, "comments")?
                    .into_iter()
                    .filter(|c| c.issue_key == *key)
                    .collect();
                comments.sort_by_key(|c| c.at);
                let more_comments = comments.len() > 100;
                let comments: Vec<_> = comments
                    .into_iter()
                    .rev()
                    .take(100)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let mut runs: Vec<_> = all::<Verification>(&self.conn, "verifications")?
                    .into_iter()
                    .filter(|v| v.issue_key == *key)
                    .collect();
                runs.sort_by_key(|v| v.submitted_at);
                let history: Vec<_> = events(&self.conn, 0)?
                    .into_iter()
                    .filter(|e| e.entity == *key)
                    .rev()
                    .take(50)
                    .collect();
                let method_findings: Vec<_> = all::<MethodFinding>(&self.conn, "method_findings")?
                    .into_iter()
                    .filter(|finding| finding.issue_key == *key)
                    .collect();
                let mut git_traces: Vec<_> = all::<GitTrace>(&self.conn, "git_traces")?
                    .into_iter()
                    .filter(|trace| trace.issue_key == *key)
                    .collect();
                git_traces.sort_by_key(|trace| trace.recorded_at);
                let project_progress = project
                    .as_ref()
                    .map(|project| project_progress(&self.conn, &project.id))
                    .transpose()?;
                let definitions = all::<Label>(&self.conn, "labels")?;
                let resolve = |ids: &[String]| -> Vec<Label> {
                    ids.iter()
                        .filter_map(|id| definitions.iter().find(|label| label.id == *id))
                        .cloned()
                        .collect()
                };
                let labels = resolve(&issue.labels);
                let project_labels = project
                    .as_ref()
                    .map(|project| resolve(&project.labels))
                    .unwrap_or_default();
                let milestone = issue
                    .milestone_id
                    .as_deref()
                    .map(|id| milestone(&self.conn, id))
                    .transpose()?;
                let milestone_progress = milestone
                    .as_ref()
                    .map(|milestone| milestone_progress(&self.conn, milestone))
                    .transpose()?;
                let goals: Vec<_> = all::<Goal>(&self.conn, "goals")?
                    .into_iter()
                    .filter(|goal| {
                        issue
                            .project_id
                            .as_ref()
                            .is_some_and(|id| goal.project_ids.contains(id))
                    })
                    .collect();
                let goal_progress = goals
                    .iter()
                    .map(|goal| goal_progress(&self.conn, goal))
                    .collect::<Result<Vec<_>>>()?;
                let issue_links = issue_link_context(&self.conn, key)?;
                let release_workflow = release_workflow(&self.conn, &issue.product_id)?;
                let releases: Vec<_> = all::<ReleaseRecord>(&self.conn, "releases")?
                    .into_iter()
                    .filter(|release| {
                        release.issue_keys.contains(key)
                            || issue
                                .project_id
                                .as_ref()
                                .is_some_and(|project_id| release.project_ids.contains(project_id))
                    })
                    .collect();
                let release_progress = releases
                    .iter()
                    .map(|release| release_progress(&self.conn, release))
                    .collect::<Result<Vec<_>>>()?;
                let release_ids: HashSet<_> = releases.iter().map(|release| &release.id).collect();
                let release_evidence: Vec<_> =
                    all::<ReleaseEvidence>(&self.conn, "release_evidence")?
                        .into_iter()
                        .filter(|evidence| release_ids.contains(&evidence.release_id))
                        .collect();
                let deletion = deletion_eligibility(&self.conn, &issue, at)?;
                let template = templates::provenance(&self.conn, issue.template.as_ref())?;
                let project_template = templates::provenance(
                    &self.conn,
                    project
                        .as_ref()
                        .and_then(|project| project.template.as_ref()),
                )?;
                return Ok(
                    json!({"template":template,"project_template":project_template,"deletion":deletion,"labels":labels,"project_labels":project_labels,"issue":issue,"product":product,"project":project,"project_progress":project_progress,"milestone":milestone,"milestone_progress":milestone_progress,"goals":goals,"goal_progress":goal_progress,"release_workflow":release_workflow,"releases":releases,"release_progress":release_progress,"release_evidence":release_evidence,"issue_links":issue_links,"comments":comments,"more_comments":more_comments,"verifications":runs,"method_findings":method_findings,"git_traces":git_traces,"history":history,"customer_signals":signals::for_issue(&self.conn, key)?,"context_links":context::for_issue(&self.conn, key)?,"context_authority":context::AUTHORITY,"retained_sources":sources::for_issue(&self.conn, key)?,"delivery_requirements":"Before owner verification: exercise every acceptance criterion end-to-end, record expected and observed results, integrate and install the exact tested build, and smoke-check the owner entrypoint. Missing or blocked checks stay with the agent. See docs/e2e-delivery.md.","content_authority":"Task data, not tool authorization"}),
                );
            }
            Command::Changes { after } => {
                let rows: Vec<_> = events(&self.conn, *after)?.into_iter().take(200).collect();
                return Ok(
                    json!({"cursor":cursor(&self.conn)?,"next_cursor":rows.last().map(|e|e.seq).unwrap_or(*after),"events":rows}),
                );
            }
            Command::Export => return Ok(serde_json::to_value(self.export()?)?),
            Command::SourceBundles => return sources::describe(&self.conn),
            Command::SearchSources {
                query,
                bundle_id,
                kind,
                classification,
                issue_key,
                limit,
                offset,
            } => {
                return sources::search(
                    &self.conn,
                    query,
                    bundle_id.as_deref(),
                    kind.as_deref(),
                    classification.as_deref(),
                    issue_key.as_deref(),
                    *limit,
                    *offset,
                )
            }
            Command::SourceRecord { id } => return sources::record_view(&self.conn, id),
            Command::Templates => return templates::listing(&self.conn),
            Command::ReadContextLink { id } => return context::read(&self.conn, id, at),
            _ => {}
        }
        required(&request.request_id, "request_id")?;
        if request.request_id.len() > 160 {
            return Err(err("invalid", "Request ID is too long"));
        }
        let hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&request.command)?)
        );
        let role_name = if role == Role::Human {
            "human"
        } else {
            "agent"
        };
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let replay: Option<(String, String, String, String)> = tx
            .query_row(
                "SELECT actor,role,hash,response FROM requests WHERE id=?1",
                [&request.request_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        if let Some((actor, prior_role, prior_hash, response)) = replay {
            if actor != request.actor || prior_role != role_name || prior_hash != hash {
                return Err(err(
                    "conflict",
                    "Request ID already used for a different command or actor",
                ));
            }
            return Ok(serde_json::from_str(&response)?);
        }
        let result = mutate(&tx, &request.command, &request.actor, role, at)?;
        tx.execute(
            "INSERT INTO requests VALUES (?1,?2,?3,?4,?5)",
            params![
                request.request_id,
                request.actor,
                role_name,
                hash,
                serde_json::to_string(&result)?
            ],
        )?;
        tx.commit()?;
        Ok(result)
    }

    fn workspace_id(&self) -> Result<String> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key='workspace_id'", [], |r| {
                r.get(0)
            })?)
    }
    pub fn export(&self) -> Result<Archive> {
        Self::export_from(&self.conn, true)
    }

    /// Retained file bytes and metadata, verified against the recorded checksum.
    pub fn source_file(&self, bundle_id: &str, path: &str) -> Result<(SourceFile, Vec<u8>)> {
        if !safe_source_path(path) {
            return Err(err("not_found", "Unknown retained file"));
        }
        let file = sources::file_meta(&self.conn, bundle_id, path)?;
        let bytes = sources::file_bytes(&self.conn, &file)?;
        Ok((file, bytes))
    }

    pub(crate) fn export_from(conn: &Connection, include_source_bytes: bool) -> Result<Archive> {
        let mut stmt =
            conn.prepare("SELECT id,actor,role,hash,response FROM requests ORDER BY id")?;
        let requests = stmt
            .query_map([], |r| {
                Ok(Replay {
                    id: r.get(0)?,
                    actor: r.get(1)?,
                    role: r.get(2)?,
                    hash: r.get(3)?,
                    response: r.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let (source_bundles, source_files, source_records) =
            sources::export_sources(conn, include_source_bytes)?;
        Ok(Archive {
            format: ARCHIVE_FORMAT,
            workspace_id: conn.query_row(
                "SELECT value FROM meta WHERE key='workspace_id'",
                [],
                |r| r.get(0),
            )?,
            products: all(conn, "products")?,
            product_sections: all(conn, "product_sections")?,
            customer_signals: all(conn, "customer_signals")?,
            context_links: all(conn, "context_links")?,
            projects: all(conn, "projects")?,
            goals: all(conn, "goals")?,
            milestones: all(conn, "milestones")?,
            labels: all(conn, "labels")?,
            theoria_documents: all(conn, "theoria_documents")?,
            method_findings: all(conn, "method_findings")?,
            git_traces: all(conn, "git_traces")?,
            releases: all(conn, "releases")?,
            release_evidence: all(conn, "release_evidence")?,
            release_workflows: all(conn, "release_workflows")?,
            issue_links: all(conn, "issue_links")?,
            templates: all(conn, "templates")?,
            template_revisions: all(conn, "template_revisions")?,
            issues: {
                let mut issues: Vec<Issue> = all(conn, "issues")?;
                attach_histories(conn, &mut issues)?;
                issues
            },
            comments: all(conn, "comments")?,
            verifications: all(conn, "verifications")?,
            events: events(conn, 0)?,
            requests,
            source_bundles,
            source_files,
            source_records,
        })
    }

    /// Only used for offline restore into a newly created destination by the CLI.
    pub fn restore(&mut self, mut a: Archive) -> Result<()> {
        validate_archive(&a)?;
        if a.format < 5 {
            for issue in &mut a.issues {
                issue.planning_scope = if issue.project_id.is_some() {
                    PlanningScope::Project
                } else {
                    PlanningScope::Inbox
                };
            }
        }
        let tx = self.conn.transaction()?;
        tx.execute_batch("DELETE FROM labels; DELETE FROM projects; DELETE FROM goals; DELETE FROM milestones; DELETE FROM theoria_documents; DELETE FROM method_findings; DELETE FROM git_traces; DELETE FROM releases; DELETE FROM release_evidence; DELETE FROM release_workflows; DELETE FROM issue_links; DELETE FROM products; DELETE FROM issues; DELETE FROM comments; DELETE FROM verifications; DELETE FROM events; DELETE FROM requests; DELETE FROM sqlite_sequence WHERE name='events'; DELETE FROM source_record_issues; DELETE FROM source_records; DELETE FROM source_chunks; DELETE FROM source_files; DELETE FROM source_bundles; DELETE FROM issue_histories; DELETE FROM templates; DELETE FROM template_revisions; DELETE FROM product_sections; DELETE FROM customer_signals; DELETE FROM context_links;")?;
        tx.execute(
            "UPDATE meta SET value=?1 WHERE key='workspace_id'",
            [a.workspace_id],
        )?;
        for p in a.products {
            tx.execute(
                "INSERT INTO products VALUES (?1,?2,?3)",
                params![p.id, p.key, serde_json::to_string(&p)?],
            )?;
        }
        for section in a.product_sections {
            put_product_section(&tx, &section)?;
        }
        for p in a.projects {
            put_project(&tx, &p)?;
        }
        for goal in a.goals {
            put_goal(&tx, &goal)?;
        }
        for milestone in a.milestones {
            put_milestone(&tx, &milestone)?;
        }
        for label in a.labels {
            put_label(&tx, &label)?;
        }
        for document in a.theoria_documents {
            put_theoria_document(&tx, &document)?;
        }
        for finding in a.method_findings {
            put_method_finding(&tx, &finding)?;
        }
        for trace in a.git_traces {
            put_git_trace(&tx, &trace)?;
        }
        for release in a.releases {
            put_release(&tx, &release)?;
        }
        for evidence in a.release_evidence {
            put_release_evidence(&tx, &evidence)?;
        }
        for workflow in a.release_workflows {
            put_release_workflow(&tx, &workflow)?;
        }
        for link in a.issue_links {
            put_issue_link(&tx, &link)?;
        }
        for template in a.templates {
            templates::put_template(&tx, &template)?;
        }
        for revision in a.template_revisions {
            templates::insert_revision(&tx, &revision)?;
        }
        for i in a.issues {
            put_issue(&tx, &i)?;
        }
        for signal in &a.customer_signals {
            signals::put_signal(&tx, signal)?;
        }
        for link in &a.context_links {
            context::restore_link(&tx, link)?;
        }
        for c in a.comments {
            tx.execute(
                "INSERT INTO comments VALUES (?1,?2)",
                params![c.id, serde_json::to_string(&c)?],
            )?;
        }
        for v in a.verifications {
            put_run(&tx, &v)?;
        }
        for e in a.events {
            tx.execute(
                "INSERT INTO events VALUES (?1,?2,?3,?4,?5)",
                params![e.seq, e.at, e.actor, e.kind, e.entity],
            )?;
        }
        for r in a.requests {
            tx.execute(
                "INSERT INTO requests VALUES (?1,?2,?3,?4,?5)",
                params![r.id, r.actor, r.role, r.hash, r.response],
            )?;
        }
        sources::restore_sources(&tx, &a.source_bundles, &a.source_files, &a.source_records)?;
        tx.commit()?;
        Ok(())
    }
}

/// Snapshot polling omits imported external history (it can be large); issue
/// context, export and the retained source keep every original value.
fn snapshot_issues(conn: &Connection, issues: Vec<Issue>) -> Result<Vec<Value>> {
    let mut statement = conn.prepare("SELECT issue_id, entries FROM issue_histories")?;
    let entries: HashMap<String, i64> = statement
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    issues
        .into_iter()
        .map(|issue| {
            let count = entries.get(&issue.id).copied().unwrap_or(0);
            let mut value = serde_json::to_value(&issue)?;
            // Screenshot bytes belong in selected issue context/export, never repeated in polling lists.
            value.as_object_mut().unwrap().remove("intake");
            if let Some(external) = value.get_mut("external").and_then(Value::as_object_mut) {
                external.insert("history_entries".into(), json!(count));
                external.insert("history_omitted".into(), json!(count > 0));
            }
            Ok(value)
        })
        .collect()
}
fn all<T: DeserializeOwned>(conn: &Connection, table: &str) -> Result<Vec<T>> {
    // Table names are internal constants, never request input.
    let mut stmt = conn.prepare(&format!("SELECT data FROM {table} ORDER BY id"))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|s| serde_json::from_str(&s).map_err(Into::into))
        .collect()
}
fn cursor(conn: &Connection) -> Result<u64> {
    Ok(conn.query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |r| r.get(0))?)
}
fn events(conn: &Connection, after: u64) -> Result<Vec<Event>> {
    let mut stmt =
        conn.prepare("SELECT seq,at,actor,kind,entity FROM events WHERE seq>?1 ORDER BY seq")?;
    let rows = stmt
        .query_map([after], |r| {
            Ok(Event {
                seq: r.get(0)?,
                at: r.get(1)?,
                actor: r.get(2)?,
                kind: r.get(3)?,
                entity: r.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}
fn issue(conn: &Connection, key: &str) -> Result<Issue> {
    let data: Option<String> = conn
        .query_row("SELECT data FROM issues WHERE key=?1", [key], |r| r.get(0))
        .optional()?;
    serde_json::from_str(
        &data.ok_or_else(|| err("not_found", format!("Issue {key} does not exist")))?,
    )
    .map_err(Into::into)
}
/// The parent's generated verification child. An inconsistent store (a
/// missing, dangling or foreign child) is refused with an error instead of a
/// panic, so the service keeps serving every other request.
fn verification_child(conn: &Connection, parent: &Issue) -> Result<Issue> {
    let inconsistent = |detail: String| {
        err(
            "invalid",
            format!("{detail}; the workspace is inconsistent and nothing was changed. Restore a consistent backup before continuing this verification."),
        )
    };
    let key = parent
        .verification_key
        .as_deref()
        .ok_or_else(|| inconsistent(format!("{}'s verification child is missing", parent.key)))?;
    let child = issue(conn, key).map_err(|_| {
        inconsistent(format!(
            "{}'s verification child {key} does not exist",
            parent.key
        ))
    })?;
    if child.parent.as_deref() != Some(parent.key.as_str()) {
        return Err(inconsistent(format!(
            "{key} is not {}'s verification child",
            parent.key
        )));
    }
    Ok(child)
}
fn run(conn: &Connection, id: &str) -> Result<Verification> {
    let data: String = conn.query_row("SELECT data FROM verifications WHERE id=?1", [id], |r| {
        r.get(0)
    })?;
    Ok(serde_json::from_str(&data)?)
}
fn project(conn: &Connection, id: &str) -> Result<Project> {
    let data: Option<String> = conn
        .query_row("SELECT data FROM projects WHERE id=?1", [id], |r| r.get(0))
        .optional()?;
    Ok(serde_json::from_str(
        &data.ok_or_else(|| err("not_found", "Unknown project"))?,
    )?)
}
fn project_progress(conn: &Connection, project_id: &str) -> Result<ProjectProgress> {
    Ok(project_progress_in(
        &all::<Issue>(conn, "issues")?,
        project_id,
    ))
}
fn project_progress_in(issues: &[Issue], project_id: &str) -> ProjectProgress {
    let parents: Vec<_> = issues
        .iter()
        .filter(|issue| issue.parent.is_none() && issue.project_id.as_deref() == Some(project_id))
        .collect();
    let counts = progress_counts(&parents);
    ProjectProgress {
        project_id: project_id.into(),
        total: counts.0,
        backlog: counts.1,
        active: counts.2,
        pending_verification: counts.3,
        completed: counts.4,
        legacy_completed: counts.5,
        canceled: counts.6,
        completion_percent: counts.7,
    }
}
fn progress_counts(issues: &[&Issue]) -> (u64, u64, u64, u64, u64, u64, u64, u8) {
    let total = issues.len() as u64;
    let backlog = issues
        .iter()
        .filter(|issue| issue.status == Status::Backlog)
        .count() as u64;
    let active = issues
        .iter()
        .filter(|issue| matches!(issue.status, Status::Ready | Status::Doing))
        .count() as u64;
    let pending_verification = issues
        .iter()
        .filter(|issue| issue.status == Status::Verify)
        .count() as u64;
    let completed = issues
        .iter()
        .filter(|issue| issue.status == Status::Done)
        .count() as u64;
    let legacy_completed = issues
        .iter()
        .filter(|issue| issue.status == Status::LegacyCompleted)
        .count() as u64;
    let canceled = issues
        .iter()
        .filter(|issue| issue.status == Status::Canceled)
        .count() as u64;
    let eligible = total.saturating_sub(canceled + legacy_completed);
    let completion_percent = completed
        .checked_mul(100)
        .and_then(|value| value.checked_div(eligible))
        .unwrap_or(0) as u8;
    (
        total,
        backlog,
        active,
        pending_verification,
        completed,
        legacy_completed,
        canceled,
        completion_percent,
    )
}
fn goal_progress(conn: &Connection, goal: &Goal) -> Result<GoalProgress> {
    Ok(goal_progress_in(&all::<Issue>(conn, "issues")?, goal))
}
fn goal_progress_in(issues: &[Issue], goal: &Goal) -> GoalProgress {
    let parents: Vec<_> = issues
        .iter()
        .filter(|issue| {
            issue.parent.is_none()
                && issue
                    .project_id
                    .as_ref()
                    .is_some_and(|id| goal.project_ids.contains(id))
        })
        .collect();
    let counts = progress_counts(&parents);
    GoalProgress {
        goal_id: goal.id.clone(),
        total: counts.0,
        backlog: counts.1,
        active: counts.2,
        pending_verification: counts.3,
        completed: counts.4,
        legacy_completed: counts.5,
        canceled: counts.6,
        completion_percent: counts.7,
    }
}
fn milestone_progress(conn: &Connection, milestone: &Milestone) -> Result<MilestoneProgress> {
    Ok(milestone_progress_in(
        &all::<Issue>(conn, "issues")?,
        milestone,
    ))
}
fn milestone_progress_in(issues: &[Issue], milestone: &Milestone) -> MilestoneProgress {
    let parents: Vec<_> = issues
        .iter()
        .filter(|issue| {
            issue.parent.is_none() && issue.milestone_id.as_ref() == Some(&milestone.id)
        })
        .collect();
    let counts = progress_counts(&parents);
    MilestoneProgress {
        milestone_id: milestone.id.clone(),
        total: counts.0,
        backlog: counts.1,
        active: counts.2,
        pending_verification: counts.3,
        completed: counts.4,
        legacy_completed: counts.5,
        canceled: counts.6,
        completion_percent: counts.7,
    }
}
fn release_issues<'a>(release: &ReleaseRecord, issues: &'a [Issue]) -> Vec<&'a Issue> {
    issues
        .iter()
        .filter(|issue| {
            issue.parent.is_none()
                && (release.issue_keys.contains(&issue.key)
                    || issue
                        .project_id
                        .as_ref()
                        .is_some_and(|project_id| release.project_ids.contains(project_id)))
        })
        .collect()
}
fn release_progress(conn: &Connection, release: &ReleaseRecord) -> Result<ReleaseProgress> {
    Ok(release_progress_in(&all::<Issue>(conn, "issues")?, release))
}
fn release_progress_in(issues: &[Issue], release: &ReleaseRecord) -> ReleaseProgress {
    let linked = release_issues(release, issues);
    let total = linked.len() as u64;
    let backlog = linked
        .iter()
        .filter(|issue| issue.status == Status::Backlog)
        .count() as u64;
    let failed_verification = linked.iter().filter(|issue| issue.needs_fix).count() as u64;
    let active = linked
        .iter()
        .filter(|issue| matches!(issue.status, Status::Ready | Status::Doing) && !issue.needs_fix)
        .count() as u64;
    let pending_verification = linked
        .iter()
        .filter(|issue| issue.status == Status::Verify)
        .count() as u64;
    let completed = linked
        .iter()
        .filter(|issue| issue.status == Status::Done)
        .count() as u64;
    let legacy_completed = linked
        .iter()
        .filter(|issue| issue.status == Status::LegacyCompleted)
        .count() as u64;
    let canceled = linked
        .iter()
        .filter(|issue| issue.status == Status::Canceled)
        .count() as u64;
    let eligible = total.saturating_sub(canceled + legacy_completed);
    ReleaseProgress {
        release_id: release.id.clone(),
        total,
        backlog,
        active,
        pending_verification,
        failed_verification,
        completed,
        legacy_completed,
        canceled,
        completion_percent: completed
            .saturating_mul(100)
            .checked_div(eligible)
            .unwrap_or(0) as u8,
    }
}
fn put_project(conn: &Connection, p: &Project) -> Result<()> {
    conn.execute(
        "INSERT INTO projects VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![p.id, serde_json::to_string(p)?],
    )?;
    Ok(())
}
fn goal(conn: &Connection, id: &str) -> Result<Goal> {
    let data: Option<String> = conn
        .query_row("SELECT data FROM goals WHERE id=?1", [id], |r| r.get(0))
        .optional()?;
    Ok(serde_json::from_str(
        &data.ok_or_else(|| err("not_found", "Unknown goal"))?,
    )?)
}
fn put_goal(conn: &Connection, goal: &Goal) -> Result<()> {
    conn.execute(
        "INSERT INTO goals VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![goal.id, serde_json::to_string(goal)?],
    )?;
    Ok(())
}
fn milestone(conn: &Connection, id: &str) -> Result<Milestone> {
    let data: Option<String> = conn
        .query_row("SELECT data FROM milestones WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(serde_json::from_str(
        &data.ok_or_else(|| err("not_found", "Unknown milestone"))?,
    )?)
}
fn put_milestone(conn: &Connection, milestone: &Milestone) -> Result<()> {
    conn.execute(
        "INSERT INTO milestones VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![milestone.id, serde_json::to_string(milestone)?],
    )?;
    Ok(())
}
fn label(conn: &Connection, id: &str) -> Result<Label> {
    let data: Option<String> = conn
        .query_row("SELECT data FROM labels WHERE id=?1", [id], |r| r.get(0))
        .optional()?;
    Ok(serde_json::from_str(
        &data.ok_or_else(|| err("not_found", "Unknown label"))?,
    )?)
}
fn put_label(conn: &Connection, label: &Label) -> Result<()> {
    conn.execute(
        "INSERT INTO labels VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![label.id, serde_json::to_string(label)?],
    )?;
    Ok(())
}
fn put_product(conn: &Connection, product: &Product) -> Result<()> {
    conn.execute(
        "INSERT INTO products VALUES (?1,?2,?3) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![product.id, product.key, serde_json::to_string(product)?],
    )?;
    Ok(())
}
fn product_section(conn: &Connection, id: &str) -> Result<ProductSection> {
    let data: Option<String> = conn
        .query_row("SELECT data FROM product_sections WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(serde_json::from_str(&data.ok_or_else(|| {
        err("not_found", "Unknown sidebar section")
    })?)?)
}
fn put_product_section(conn: &Connection, section: &ProductSection) -> Result<()> {
    conn.execute(
        "INSERT INTO product_sections VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![section.id, serde_json::to_string(section)?],
    )?;
    Ok(())
}
/// Trimmed section name, unique case-insensitively among the other sections.
fn product_section_name(
    existing: &[ProductSection],
    name: &str,
    except: Option<&str>,
) -> Result<String> {
    let name = name.trim();
    required(name, "name")?;
    if name.chars().count() > 80 {
        return Err(err("invalid", "Section name must be at most 80 characters"));
    }
    if existing.iter().any(|section| {
        Some(section.id.as_str()) != except && section.name.to_lowercase() == name.to_lowercase()
    }) {
        return Err(err(
            "conflict",
            "A sidebar section with this name already exists",
        ));
    }
    Ok(name.into())
}
/// Canonical names, aliases and Linear origins are unique across the whole workspace taxonomy.
fn label_taxonomy_conflicts(
    existing: &[Label],
    name: &str,
    aliases: &[String],
    linear_origins: &[LinearLabelOrigin],
    except: Option<&str>,
) -> Result<()> {
    let mut terms: Vec<String> = vec![normalized_label_name(name)];
    terms.extend(aliases.iter().map(|alias| normalized_label_name(alias)));
    for other in existing
        .iter()
        .filter(|other| Some(other.id.as_str()) != except)
    {
        if terms.contains(&normalized_label_name(&other.name)) {
            return Err(err(
                "conflict",
                format!(
                    "Label name or alias collides with the canonical label “{}”",
                    other.name
                ),
            ));
        }
        if other
            .aliases
            .iter()
            .any(|alias| terms.contains(&normalized_label_name(alias)))
        {
            return Err(err(
                "conflict",
                format!(
                    "Label name or alias collides with an alias of “{}”",
                    other.name
                ),
            ));
        }
        if other.linear_origins.iter().any(|origin| {
            linear_origins
                .iter()
                .any(|candidate| candidate.id == origin.id)
        }) {
            return Err(err(
                "conflict",
                format!("Linear label ID is already mapped to “{}”", other.name),
            ));
        }
    }
    Ok(())
}
fn check_label_products(conn: &Connection, products: &[LabelProductRule]) -> Result<()> {
    let known = all::<Product>(conn, "products")?;
    for rule in products {
        if !known.iter().any(|product| product.id == rule.product_id) {
            return Err(err("not_found", "Unknown product in label rule"));
        }
    }
    Ok(())
}
fn theoria_document(conn: &Connection, id: &str) -> Result<TheoriaDocument> {
    let data: Option<String> = conn
        .query_row(
            "SELECT data FROM theoria_documents WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(serde_json::from_str(&data.ok_or_else(|| {
        err("not_found", "Unknown Theoria document")
    })?)?)
}
fn put_theoria_document(conn: &Connection, document: &TheoriaDocument) -> Result<()> {
    conn.execute(
        "INSERT INTO theoria_documents VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![document.id, serde_json::to_string(document)?],
    )?;
    Ok(())
}
fn put_method_finding(conn: &Connection, finding: &MethodFinding) -> Result<()> {
    conn.execute(
        "INSERT INTO method_findings VALUES (?1,?2)",
        params![finding.id, serde_json::to_string(finding)?],
    )?;
    Ok(())
}
fn put_git_trace(conn: &Connection, trace: &GitTrace) -> Result<()> {
    conn.execute(
        "INSERT INTO git_traces VALUES (?1,?2)",
        params![trace.id, serde_json::to_string(trace)?],
    )?;
    Ok(())
}
fn release(conn: &Connection, id: &str) -> Result<ReleaseRecord> {
    let data: Option<String> = conn
        .query_row("SELECT data FROM releases WHERE id=?1", [id], |r| r.get(0))
        .optional()?;
    Ok(serde_json::from_str(
        &data.ok_or_else(|| err("not_found", "Unknown release"))?,
    )?)
}
fn put_release(conn: &Connection, release: &ReleaseRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO releases VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![release.id, serde_json::to_string(release)?],
    )?;
    Ok(())
}
fn put_release_evidence(conn: &Connection, evidence: &ReleaseEvidence) -> Result<()> {
    conn.execute(
        "INSERT INTO release_evidence VALUES (?1,?2)",
        params![evidence.id, serde_json::to_string(evidence)?],
    )?;
    Ok(())
}
fn release_workflow(conn: &Connection, product_id: &str) -> Result<Option<ReleaseWorkflowConfig>> {
    let data: Option<String> = conn
        .query_row(
            "SELECT data FROM release_workflows WHERE id=?1",
            [product_id],
            |row| row.get(0),
        )
        .optional()?;
    data.map(|value| serde_json::from_str(&value).map_err(Into::into))
        .transpose()
}
fn put_release_workflow(conn: &Connection, workflow: &ReleaseWorkflowConfig) -> Result<()> {
    conn.execute(
        "INSERT INTO release_workflows VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![workflow.product_id, serde_json::to_string(workflow)?],
    )?;
    Ok(())
}
fn put_issue_link(conn: &Connection, link: &IssueLink) -> Result<()> {
    conn.execute(
        "INSERT INTO issue_links VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![link.id, serde_json::to_string(link)?],
    )?;
    Ok(())
}
fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}
fn listed(references: &[String]) -> String {
    const SHOWN: usize = 5;
    let mut text = references
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if references.len() > SHOWN {
        text.push_str(&format!(" and {} more", references.len() - SHOWN));
    }
    text
}
/// The single deletion rule. `context` reports it, and `delete_issue` recomputes it
/// inside the deleting transaction, so an earlier preview never authorizes a deletion.
pub(crate) fn deletion_eligibility(
    conn: &Connection,
    issue: &Issue,
    at: i64,
) -> Result<DeletionEligibility> {
    let key = issue.key.as_str();
    let mut blockers = Vec::new();
    let mut block = |kind, references: Vec<String>, count: usize, removable, message| {
        blockers.push(DeletionBlocker {
            kind,
            count: count as u64,
            references,
            removable,
            message,
        })
    };
    if !matches!(issue.status, Status::Backlog | Status::Ready) {
        let status = serde_json::to_value(&issue.status)?
            .as_str()
            .unwrap_or_default()
            .replace('_', " ");
        block(
            DeletionBlockerKind::Status,
            vec![],
            1,
            false,
            format!("It is {status}; only unstarted Backlog or Ready issues can be deleted"),
        );
    }
    if let Some(claim) = issue.claim.as_ref().filter(|claim| claim.expires_at > at) {
        block(
            DeletionBlockerKind::ActiveClaim,
            vec![claim.actor.clone()],
            1,
            true,
            format!(
                "{} holds an active claim; it must be released or expire first",
                claim.actor
            ),
        );
    }
    let comments = all::<Comment>(conn, "comments")?
        .into_iter()
        .filter(|comment| comment.issue_key == key)
        .count();
    if comments > 0 {
        block(
            DeletionBlockerKind::Comments,
            vec![],
            comments,
            false,
            format!(
                "It has {}; discussion history is retained and is not deleted",
                plural(comments, "comment", "comments")
            ),
        );
    }
    let mut runs: Vec<String> = all::<Verification>(conn, "verifications")?
        .into_iter()
        .filter(|run| run.issue_key == key)
        .map(|run| run.id)
        .collect();
    for id in issue.current_run.iter() {
        if !runs.contains(id) {
            runs.push(id.clone());
        }
    }
    if !runs.is_empty() || issue.verification_key.is_some() {
        let mut references = runs.clone();
        references.extend(issue.verification_key.iter().cloned());
        block(
            DeletionBlockerKind::VerificationHistory,
            references,
            runs.len().max(1),
            false,
            format!(
                "It has {}; submitted work is retained",
                if runs.is_empty() {
                    "a linked verification issue".to_string()
                } else {
                    plural(
                        runs.len(),
                        "verification submission",
                        "verification submissions",
                    )
                }
            ),
        );
    }
    let children: Vec<String> = all::<Issue>(conn, "issues")?
        .into_iter()
        .filter(|candidate| candidate.parent.as_deref() == Some(key))
        .map(|candidate| candidate.key)
        .collect();
    if !children.is_empty() {
        block(
            DeletionBlockerKind::VerificationChildren,
            children.clone(),
            children.len(),
            false,
            format!(
                "Verification issue {} belongs to its history",
                listed(&children)
            ),
        );
    }
    let findings: Vec<String> = all::<MethodFinding>(conn, "method_findings")?
        .into_iter()
        .filter(|finding| finding.issue_key == key)
        .map(|finding| finding.id)
        .collect();
    if !findings.is_empty() {
        block(
            DeletionBlockerKind::MethodFindings,
            findings.clone(),
            findings.len(),
            false,
            format!(
                "It has {} recorded against it",
                plural(findings.len(), "method finding", "method findings")
            ),
        );
    }
    let traces: Vec<String> = all::<GitTrace>(conn, "git_traces")?
        .into_iter()
        .filter(|trace| trace.issue_key == key)
        .map(|trace| trace.commit_sha)
        .collect();
    if !traces.is_empty() {
        block(
            DeletionBlockerKind::GitTraces,
            traces.clone(),
            traces.len(),
            false,
            format!(
                "It has {} recorded against it",
                plural(traces.len(), "Git trace", "Git traces")
            ),
        );
    }
    let links: Vec<String> = all::<IssueLink>(conn, "issue_links")?
        .into_iter()
        .filter_map(|link| {
            if link.source_key == key {
                Some(link.target_key)
            } else if link.target_key == key {
                Some(link.source_key)
            } else {
                None
            }
        })
        .collect();
    if !links.is_empty() {
        block(
            DeletionBlockerKind::IssueLinks,
            links.clone(),
            links.len(),
            true,
            format!(
                "It has {} ({}); remove them in Relations first",
                plural(links.len(), "issue link", "issue links"),
                listed(&links)
            ),
        );
    }
    let releases: Vec<ReleaseRecord> = all::<ReleaseRecord>(conn, "releases")?
        .into_iter()
        .filter(|release| release.issue_keys.iter().any(|item| item == key))
        .collect();
    // Deployed releases freeze linked work. Do not offer an unlink action that
    // UpdateRelease will refuse; keep editable and frozen references distinct.
    for frozen in [false, true] {
        let group: Vec<&ReleaseRecord> = releases
            .iter()
            .filter(|release| {
                matches!(
                    release.status,
                    ReleaseStatus::Preview | ReleaseStatus::Production
                ) == frozen
            })
            .collect();
        if group.is_empty() {
            continue;
        }
        let labels: Vec<String> = group
            .iter()
            .map(|release| release.version_label.clone())
            .collect();
        let message = if frozen {
            format!(
                "{} explicitly {} it ({}); linked work is frozen while the release is in Preview or Production",
                plural(group.len(), "deployed release", "deployed releases"),
                if group.len() == 1 { "includes" } else { "include" },
                listed(&labels),
            )
        } else {
            format!(
                "{} explicitly {} it ({}); remove the issue from {} first",
                plural(group.len(), "release", "releases"),
                if group.len() == 1 {
                    "includes"
                } else {
                    "include"
                },
                listed(&labels),
                if group.len() == 1 {
                    "that release"
                } else {
                    "those releases"
                },
            )
        };
        block(
            DeletionBlockerKind::ReleaseReferences,
            group.iter().map(|release| release.id.clone()).collect(),
            group.len(),
            !frozen,
            message,
        );
    }
    let evidence: Vec<String> = all::<ReleaseEvidence>(conn, "release_evidence")?
        .into_iter()
        .filter(|evidence| evidence.issue_key.as_deref() == Some(key))
        .map(|evidence| evidence.id)
        .collect();
    if !evidence.is_empty() {
        block(
            DeletionBlockerKind::ReleaseEvidence,
            evidence.clone(),
            evidence.len(),
            false,
            format!(
                "It has {}; release evidence is retained",
                plural(
                    evidence.len(),
                    "release evidence record",
                    "release evidence records"
                )
            ),
        );
    }
    let documents = context::deletion_references(conn, key)?;
    if !documents.is_empty() {
        block(
            DeletionBlockerKind::ContextLinks,
            documents.clone(),
            documents.len(),
            true,
            format!(
                "It has {} ({}); remove them first",
                plural(documents.len(), "context document", "context documents"),
                listed(&documents)
            ),
        );
    }
    let (linked_signals, promoted_signals) = signals::deletion_references(conn, key)?;
    if !linked_signals.is_empty() {
        block(
            DeletionBlockerKind::CustomerSignals,
            linked_signals.clone(),
            linked_signals.len(),
            true,
            format!(
                "{} linked to it; unlink them first",
                plural(
                    linked_signals.len(),
                    "customer request is",
                    "customer requests are"
                )
            ),
        );
    }
    if !promoted_signals.is_empty() {
        block(
            DeletionBlockerKind::CustomerSignals,
            promoted_signals.clone(),
            promoted_signals.len(),
            false,
            "It was promoted from a customer request; that provenance is retained".into(),
        );
    }
    Ok(DeletionEligibility {
        key: issue.key.clone(),
        version: issue.version,
        eligible: blockers.is_empty(),
        blockers,
    })
}
fn issue_link_context(conn: &Connection, key: &str) -> Result<Vec<Value>> {
    let issues: HashMap<_, _> = all::<Issue>(conn, "issues")?
        .into_iter()
        .map(|issue| (issue.key.clone(), issue))
        .collect();
    let mut result = Vec::new();
    for link in all::<IssueLink>(conn, "issue_links")? {
        let (direction, other_key) = if link.source_key == key {
            ("outgoing", &link.target_key)
        } else if link.target_key == key {
            ("incoming", &link.source_key)
        } else {
            continue;
        };
        let other = issues
            .get(other_key)
            .ok_or_else(|| err("invalid", "Unresolved issue link"))?;
        result.push(json!({
            "id": link.id,
            "source_key": link.source_key,
            "target_key": link.target_key,
            "kind": link.kind,
            "external_source": link.external_source,
            "external_id": link.external_id,
            "created_by": link.created_by,
            "created_at": link.created_at,
            "direction": direction,
            "issue": other,
        }));
    }
    result.sort_by(|a, b| {
        a["kind"]
            .as_str()
            .cmp(&b["kind"].as_str())
            .then_with(|| a["issue"]["key"].as_str().cmp(&b["issue"]["key"].as_str()))
    });
    Ok(result)
}
fn same_logical_link(
    link: &IssueLink,
    source_key: &str,
    target_key: &str,
    kind: &IssueLinkKind,
) -> bool {
    if link.kind != *kind {
        return false;
    }
    if *kind == IssueLinkKind::Related {
        (link.source_key == source_key && link.target_key == target_key)
            || (link.source_key == target_key && link.target_key == source_key)
    } else {
        link.source_key == source_key && link.target_key == target_key
    }
}
fn validate_external_provenance(
    kind: &IssueLinkKind,
    external_source: Option<&str>,
    external_id: Option<&str>,
) -> Result<()> {
    validate_optional_provenance(external_source, external_id)?;
    if *kind == IssueLinkKind::LegacyVerification && external_source.is_none() {
        return Err(err(
            "invalid",
            "Legacy verification links require external provenance",
        ));
    }
    Ok(())
}
fn would_create_cycle(
    links: &[IssueLink],
    source_key: &str,
    target_key: &str,
    kind: &IssueLinkKind,
) -> bool {
    if !matches!(kind, IssueLinkKind::Parent | IssueLinkKind::BlockedBy) {
        return false;
    }
    let mut stack = vec![target_key];
    let mut visited = HashSet::new();
    while let Some(current) = stack.pop() {
        if current == source_key {
            return true;
        }
        if !visited.insert(current) {
            continue;
        }
        stack.extend(
            links
                .iter()
                .filter(|link| link.kind == *kind && link.source_key == current)
                .map(|link| link.target_key.as_str()),
        );
    }
    false
}
fn project_name(
    conn: &Connection,
    product_id: &str,
    name: &str,
    except: Option<&str>,
) -> Result<()> {
    required(name, "project name")?;
    if name.trim().len() > 160 {
        return Err(err("invalid", "Project name must be at most 160 bytes"));
    }
    if all::<Project>(conn, "projects")?.iter().any(|p| {
        p.product_id == product_id
            && Some(p.id.as_str()) != except
            && p.name.to_lowercase() == name.trim().to_lowercase()
    }) {
        return Err(err(
            "conflict",
            "A project with this name already exists in this product",
        ));
    }
    Ok(())
}
fn goal_name(conn: &Connection, product_id: &str, name: &str, except: Option<&str>) -> Result<()> {
    if all::<Goal>(conn, "goals")?.iter().any(|goal| {
        goal.product_id == product_id
            && Some(goal.id.as_str()) != except
            && goal.name.to_lowercase() == name.trim().to_lowercase()
    }) {
        return Err(err(
            "conflict",
            "A goal with this name already exists in this product",
        ));
    }
    Ok(())
}
fn milestone_name(
    conn: &Connection,
    project_id: &str,
    name: &str,
    except: Option<&str>,
) -> Result<()> {
    if all::<Milestone>(conn, "milestones")?
        .iter()
        .any(|milestone| {
            milestone.project_id == project_id
                && Some(milestone.id.as_str()) != except
                && milestone.name.to_lowercase() == name.trim().to_lowercase()
        })
    {
        return Err(err(
            "conflict",
            "A milestone with this name already exists in this project",
        ));
    }
    Ok(())
}
fn validate_goal_projects(
    conn: &Connection,
    product_id: &str,
    project_ids: &[String],
) -> Result<()> {
    let mut unique = HashSet::new();
    for id in project_ids {
        if !unique.insert(id) || project(conn, id)?.product_id != product_id {
            return Err(err(
                "invalid",
                "Goal projects must be unique and belong to the goal's product",
            ));
        }
    }
    Ok(())
}
fn release_name(
    conn: &Connection,
    product_id: &str,
    version_label: &str,
    except: Option<&str>,
) -> Result<()> {
    if all::<ReleaseRecord>(conn, "releases")?
        .iter()
        .any(|release| {
            release.product_id == product_id
                && Some(release.id.as_str()) != except
                && release
                    .version_label
                    .eq_ignore_ascii_case(version_label.trim())
        })
    {
        return Err(err(
            "conflict",
            "A release with this version already exists in this product",
        ));
    }
    Ok(())
}
fn validate_release_links(
    conn: &Connection,
    product_id: &str,
    project_ids: &[String],
    issue_keys: &[String],
) -> Result<()> {
    let mut unique_projects = HashSet::new();
    for id in project_ids {
        if !unique_projects.insert(id) || project(conn, id)?.product_id != product_id {
            return Err(err(
                "invalid",
                "Release projects must be unique and belong to the release product",
            ));
        }
    }
    let mut unique_issues = HashSet::new();
    for key in issue_keys {
        let linked = issue(conn, key)?;
        if !unique_issues.insert(key) || linked.product_id != product_id || linked.parent.is_some()
        {
            return Err(err(
                "invalid",
                "Release issues must be unique real issues in the release product",
            ));
        }
    }
    Ok(())
}
fn validate_release_branch(
    conn: &Connection,
    product_id: &str,
    release_id: Option<&str>,
    version_label: &str,
    target_ref: &str,
    release_branch: Option<&str>,
) -> Result<()> {
    let branch = release_branch
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(workflow) = release_workflow(conn, product_id)? {
        if target_ref.trim() != workflow.production_ref {
            return Err(err(
                "invalid",
                "Release target ref must match the product production ref",
            ));
        }
        if workflow.branch_strategy == ReleaseBranchStrategy::OneBranchPerRelease {
            let expected = workflow
                .release_branch_pattern
                .replace("{version}", version_label.trim());
            if branch != Some(expected.as_str()) {
                return Err(err(
                    "invalid",
                    format!("Release branch must follow the product convention: {expected}"),
                ));
            }
        }
    }
    if let Some(branch) = branch {
        limited(branch, "release branch", 512)?;
        if all::<ReleaseRecord>(conn, "releases")?
            .iter()
            .any(|release| {
                release.product_id == product_id
                    && Some(release.id.as_str()) != release_id
                    && release.release_branch.as_deref() == Some(branch)
                    && !matches!(
                        release.status,
                        ReleaseStatus::Retired | ReleaseStatus::Canceled
                    )
            })
        {
            return Err(err(
                "conflict",
                "An active release already owns this release branch",
            ));
        }
    }
    Ok(())
}
/// Issue rows never embed external history: it is stored beside the row and
/// only attached for context and export. An issue loaded without its history
/// and saved again leaves the stored history untouched.
fn put_issue(conn: &Connection, i: &Issue) -> Result<()> {
    let history = i
        .external
        .as_ref()
        .map(|external| external.history.as_slice())
        .unwrap_or_default();
    let data = if history.is_empty() {
        serde_json::to_string(i)?
    } else {
        conn.execute(
            "INSERT INTO issue_histories VALUES (?1,?2,?3) ON CONFLICT(issue_id) DO UPDATE SET entries=excluded.entries, data=excluded.data",
            params![i.id, history.len() as i64, serde_json::to_string(history)?],
        )?;
        let mut row = serde_json::to_value(i)?;
        row["external"]["history"] = json!([]);
        serde_json::to_string(&row)?
    };
    conn.execute(
        "INSERT INTO issues VALUES (?1,?2,?3) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![i.id, i.key, data],
    )?;
    Ok(())
}
fn attach_history(conn: &Connection, issue: &mut Issue) -> Result<()> {
    if let Some(external) = issue.external.as_mut() {
        let data: Option<String> = conn
            .query_row(
                "SELECT data FROM issue_histories WHERE issue_id=?1",
                [&issue.id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(data) = data {
            external.history = serde_json::from_str(&data)?;
        }
    }
    Ok(())
}
fn attach_histories(conn: &Connection, issues: &mut [Issue]) -> Result<()> {
    let mut statement = conn.prepare("SELECT issue_id, data FROM issue_histories")?;
    let mut histories: HashMap<String, String> = statement
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    for issue in issues {
        if let (Some(external), Some(data)) = (issue.external.as_mut(), histories.remove(&issue.id))
        {
            external.history = serde_json::from_str(&data)?;
        }
    }
    Ok(())
}
fn put_run(conn: &Connection, v: &Verification) -> Result<()> {
    conn.execute(
        "INSERT INTO verifications VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![v.id, serde_json::to_string(v)?],
    )?;
    Ok(())
}
fn emit(conn: &Connection, actor: &str, kind: &str, entity: &str, at: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO events(at,actor,kind,entity) VALUES (?1,?2,?3,?4)",
        params![at, actor, kind, entity],
    )?;
    Ok(())
}
fn version(conn: &Connection, key: &str, expected: u64) -> Result<Issue> {
    let i = issue(conn, key)?;
    if i.version != expected {
        return Err(err(
            "conflict",
            format!(
                "{key} is version {}, not {expected}. Read current context before retrying.",
                i.version
            ),
        ));
    }
    if i.parent.is_some() {
        return Err(err(
            "invalid",
            "Verification children are managed through their parent",
        ));
    }
    Ok(i)
}
fn held(i: &Issue, actor: &str, at: i64) -> Result<()> {
    match &i.claim {
        Some(c) if c.actor == actor && c.expires_at > at => Ok(()),
        _ => Err(err(
            "claim_required",
            "Acquire an active claim for this actor first",
        )),
    }
}
fn check_lease(seconds: i64) -> Result<()> {
    if !(30..=86400).contains(&seconds) {
        Err(err("invalid", "Lease must be between 30 and 86400 seconds"))
    } else {
        Ok(())
    }
}
fn new_issue(
    conn: &Connection,
    p: &Product,
    title: &str,
    body: &str,
    planning_scope: PlanningScope,
    project_id: Option<String>,
    at: i64,
) -> Result<Issue> {
    let existing = all::<Issue>(conn, "issues")?;
    let current_max = existing
        .iter()
        .filter(|i| i.product_id == p.id)
        .filter_map(|i| i.key.rsplit('-').next()?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    // Deleted issues remain represented by their event history. Include those keys so
    // a destructive cleanup can never cause a previously issued identifier to be reused.
    let historical_max = events(conn, 0)?
        .iter()
        .filter_map(|event| {
            event
                .entity
                .strip_prefix(&format!("{}-", p.key))?
                .parse::<u64>()
                .ok()
        })
        .max()
        .unwrap_or(0);
    let next = current_max.max(historical_max) + 1;
    Ok(Issue {
        intake: None,
        id: id(),
        key: format!("{}-{next}", p.key),
        product_id: p.id.clone(),
        project_id,
        milestone_id: None,
        planning_scope,
        theoria_refs: vec![],
        labels: vec![],
        title: title.trim().into(),
        body: body.into(),
        acceptance: String::new(),
        owner: String::new(),
        priority: "medium".into(),
        status: Status::Backlog,
        version: 1,
        created_at: at,
        updated_at: at,
        claim: None,
        needs_fix: false,
        parent: None,
        verification_key: None,
        current_run: None,
        external: None,
        template: None,
    })
}
fn save(conn: &Connection, mut i: Issue, actor: &str, kind: &str, at: i64) -> Result<Value> {
    i.version += 1;
    i.updated_at = at;
    put_issue(conn, &i)?;
    emit(conn, actor, kind, &i.key, at)?;
    Ok(json!(i))
}

fn mutate(tx: &Transaction, cmd: &Command, actor: &str, role: Role, at: i64) -> Result<Value> {
    match cmd {
        Command::CreateProject {
            product,
            name,
            description,
            priority,
            sort_order,
            template,
        } => {
            human(role)?;
            validate_project_fields(name, priority, *sort_order)?;
            let product = all::<Product>(tx, "products")?
                .into_iter()
                .find(|p| p.key == *product)
                .ok_or_else(|| err("not_found", "Unknown product"))?;
            project_name(tx, &product.id, name, None)?;
            let (template, labels) = match template {
                Some(selection) => {
                    let (used, kept) = templates::apply(
                        tx,
                        selection,
                        templates::Intake {
                            target: TemplateTarget::Project,
                            product: &product,
                            priority,
                            planning_scope: None,
                        },
                        actor,
                        at,
                    )?;
                    (Some(used), kept)
                }
                None => (None, vec![]),
            };
            let p = Project {
                id: id(),
                product_id: product.id,
                name: name.trim().into(),
                description: description.clone(),
                status: ProjectStatus::Planned,
                priority: priority.clone(),
                sort_order: *sort_order,
                external_source: None,
                external_id: None,
                external_url: None,
                labels,
                template,
                version: 1,
                created_at: at,
                updated_at: at,
            };
            put_project(tx, &p)?;
            emit(tx, actor, "project_created", &p.id, at)?;
            Ok(json!(p))
        }
        Command::UpdateProject {
            id,
            expected_version,
            name,
            description,
            status,
            priority,
            sort_order,
        } => {
            human(role)?;
            let mut p = project(tx, id)?;
            if p.version != *expected_version {
                return Err(err(
                    "conflict",
                    "Project changed; reopen its editor before retrying",
                ));
            }
            let next_priority = priority.clone().unwrap_or_else(|| p.priority.clone());
            let next_sort_order = sort_order.unwrap_or(p.sort_order);
            validate_project_fields(name, &next_priority, next_sort_order)?;
            project_name(tx, &p.product_id, name, Some(id))?;
            p.name = name.trim().into();
            p.description = description.clone();
            if let Some(status) = status {
                p.status = status.clone();
            }
            p.priority = next_priority;
            p.sort_order = next_sort_order;
            p.version += 1;
            p.updated_at = at;
            put_project(tx, &p)?;
            emit(tx, actor, "project_updated", id, at)?;
            Ok(json!(p))
        }
        Command::CreateGoal {
            product,
            name,
            description,
            priority,
            project_ids,
            external_source,
            external_id,
        } => {
            human(role)?;
            validate_goal_fields(name, priority)?;
            validate_optional_provenance(external_source.as_deref(), external_id.as_deref())?;
            let product = all::<Product>(tx, "products")?
                .into_iter()
                .find(|candidate| candidate.key == *product)
                .ok_or_else(|| err("not_found", "Unknown product"))?;
            goal_name(tx, &product.id, name, None)?;
            validate_goal_projects(tx, &product.id, project_ids)?;
            let goal = Goal {
                id: id(),
                product_id: product.id,
                name: name.trim().into(),
                description: description.clone(),
                status: GoalStatus::Planned,
                priority: priority.clone(),
                project_ids: project_ids.clone(),
                external_source: external_source.clone(),
                external_id: external_id.clone(),
                external_url: None,
                version: 1,
                created_at: at,
                updated_at: at,
            };
            put_goal(tx, &goal)?;
            emit(tx, actor, "goal_created", &goal.id, at)?;
            Ok(json!(goal))
        }
        Command::UpdateGoal {
            id,
            expected_version,
            name,
            description,
            status,
            priority,
            project_ids,
        } => {
            human(role)?;
            let mut goal = goal(tx, id)?;
            if goal.version != *expected_version {
                return Err(err("conflict", "Goal version changed; refresh and retry"));
            }
            validate_goal_fields(name, priority)?;
            goal_name(tx, &goal.product_id, name, Some(id))?;
            validate_goal_projects(tx, &goal.product_id, project_ids)?;
            goal.name = name.trim().into();
            goal.description = description.clone();
            goal.status = status.clone();
            goal.priority = priority.clone();
            goal.project_ids = project_ids.clone();
            goal.version += 1;
            goal.updated_at = at;
            put_goal(tx, &goal)?;
            emit(tx, actor, "goal_updated", id, at)?;
            Ok(json!(goal))
        }
        Command::CreateMilestone {
            project_id,
            name,
            description,
            sort_order,
            external_source,
            external_id,
        } => {
            human(role)?;
            validate_milestone_fields(name, *sort_order)?;
            validate_optional_provenance(external_source.as_deref(), external_id.as_deref())?;
            project(tx, project_id)?;
            milestone_name(tx, project_id, name, None)?;
            let milestone = Milestone {
                id: id(),
                project_id: project_id.clone(),
                name: name.trim().into(),
                description: description.clone(),
                sort_order: *sort_order,
                external_source: external_source.clone(),
                external_id: external_id.clone(),
                external_url: None,
                version: 1,
                created_at: at,
                updated_at: at,
            };
            put_milestone(tx, &milestone)?;
            emit(tx, actor, "milestone_created", &milestone.id, at)?;
            Ok(json!(milestone))
        }
        Command::UpdateMilestone {
            id,
            expected_version,
            name,
            description,
            sort_order,
        } => {
            human(role)?;
            let mut milestone = milestone(tx, id)?;
            if milestone.version != *expected_version {
                return Err(err(
                    "conflict",
                    "Milestone version changed; refresh and retry",
                ));
            }
            validate_milestone_fields(name, *sort_order)?;
            milestone_name(tx, &milestone.project_id, name, Some(id))?;
            milestone.name = name.trim().into();
            milestone.description = description.clone();
            milestone.sort_order = *sort_order;
            milestone.version += 1;
            milestone.updated_at = at;
            put_milestone(tx, &milestone)?;
            emit(tx, actor, "milestone_updated", id, at)?;
            Ok(json!(milestone))
        }
        Command::SetIssueProject {
            key,
            expected_version,
            project_id,
        } => {
            let mut i = version(tx, key, *expected_version)?;
            if role == Role::Agent {
                if matches!(
                    i.status,
                    Status::Verify | Status::Done | Status::LegacyCompleted | Status::Canceled
                ) {
                    human(role)?;
                }
                if i.status == Status::Doing {
                    held(&i, actor, at)?;
                }
            }
            if let Some(id) = project_id {
                let p = project(tx, id)?;
                if p.product_id != i.product_id {
                    return Err(err("invalid", "Project must belong to the issue's product"));
                }
            } else if i.planning_scope == PlanningScope::Project
                && !matches!(
                    i.status,
                    Status::Backlog | Status::LegacyCompleted | Status::Canceled
                )
            {
                return Err(err(
                    "invalid",
                    "Active project work cannot remove its project",
                ));
            }
            i.project_id = project_id.clone();
            if let Some(id) = &i.milestone_id {
                if Some(milestone(tx, id)?.project_id) != *project_id {
                    i.milestone_id = None;
                }
            }
            if let Some(key) = &i.verification_key {
                let mut child = issue(tx, key)?;
                child.project_id = project_id.clone();
                child.milestone_id = i.milestone_id.clone();
                child.version += 1;
                child.updated_at = at;
                put_issue(tx, &child)?;
            }
            // Planning metadata does not change readiness or invalidate test evidence.
            save(tx, i, actor, "issue_project_changed", at)
        }
        Command::SetIssueMilestone {
            key,
            expected_version,
            milestone_id,
        } => {
            let mut i = version(tx, key, *expected_version)?;
            if role == Role::Agent {
                if matches!(
                    i.status,
                    Status::Verify | Status::Done | Status::LegacyCompleted | Status::Canceled
                ) {
                    human(role)?;
                }
                if i.status == Status::Doing {
                    held(&i, actor, at)?;
                }
            }
            if let Some(id) = milestone_id {
                let milestone = milestone(tx, id)?;
                if Some(milestone.project_id) != i.project_id {
                    return Err(err(
                        "invalid",
                        "Milestone must belong to the issue's project",
                    ));
                }
            }
            i.milestone_id = milestone_id.clone();
            if let Some(key) = &i.verification_key {
                let mut child = issue(tx, key)?;
                child.milestone_id = milestone_id.clone();
                child.version += 1;
                child.updated_at = at;
                put_issue(tx, &child)?;
            }
            save(tx, i, actor, "issue_milestone_changed", at)
        }
        Command::CreateIssueLink {
            key,
            expected_version,
            target_key,
            kind,
            external_source,
            external_id,
        } => {
            let i = version(tx, key, *expected_version)?;
            if role == Role::Agent {
                if matches!(
                    i.status,
                    Status::Verify | Status::Done | Status::LegacyCompleted | Status::Canceled
                ) {
                    human(role)?;
                }
                if i.status == Status::Doing {
                    held(&i, actor, at)?;
                }
            }
            if key == target_key {
                return Err(err("invalid", "An issue cannot link to itself"));
            }
            let target = issue(tx, target_key)?;
            if target.parent.is_some() {
                return Err(err(
                    "invalid",
                    "Generated verification children cannot be linked as general work",
                ));
            }
            if target.product_id != i.product_id {
                return Err(err(
                    "invalid",
                    "Linked issues must belong to the same product",
                ));
            }
            validate_external_provenance(kind, external_source.as_deref(), external_id.as_deref())?;
            let links = all::<IssueLink>(tx, "issue_links")?;
            if links
                .iter()
                .any(|link| same_logical_link(link, key, target_key, kind))
            {
                return Err(err("conflict", "This issue link already exists"));
            }
            if *kind == IssueLinkKind::Parent
                && links
                    .iter()
                    .any(|link| link.kind == IssueLinkKind::Parent && link.source_key == *key)
            {
                return Err(err("conflict", "An issue can have only one parent"));
            }
            if would_create_cycle(&links, key, target_key, kind) {
                return Err(err("invalid", "Issue link would create a dependency cycle"));
            }
            let link = IssueLink {
                id: id(),
                source_key: key.clone(),
                target_key: target_key.clone(),
                kind: kind.clone(),
                external_source: external_source.clone(),
                external_id: external_id.clone(),
                created_by: actor.into(),
                created_at: at,
            };
            put_issue_link(tx, &link)?;
            save(tx, i, actor, "issue_link_created", at)?;
            Ok(json!(link))
        }
        Command::DeleteIssueLink {
            key,
            expected_version,
            link_id,
        } => {
            let i = version(tx, key, *expected_version)?;
            if role == Role::Agent {
                if matches!(
                    i.status,
                    Status::Verify | Status::Done | Status::LegacyCompleted | Status::Canceled
                ) {
                    human(role)?;
                }
                if i.status == Status::Doing {
                    held(&i, actor, at)?;
                }
            }
            let link = all::<IssueLink>(tx, "issue_links")?
                .into_iter()
                .find(|link| link.id == *link_id)
                .ok_or_else(|| err("not_found", "Unknown issue link"))?;
            if link.source_key != *key && link.target_key != *key {
                return Err(err("invalid", "Issue link does not belong to this issue"));
            }
            if role == Role::Agent && link.source_key != *key {
                return Err(err(
                    "forbidden",
                    "Agents must update an issue link from its source issue",
                ));
            }
            tx.execute("DELETE FROM issue_links WHERE id=?1", [link_id])?;
            save(tx, i, actor, "issue_link_deleted", at)
        }
        Command::CreateLabel {
            name,
            description,
            color,
            aliases,
            products,
            linear_origins,
        } => {
            human(role)?;
            let fields =
                validate_label_fields(name, description, color, aliases, products, linear_origins)?;
            check_label_products(tx, &fields.products)?;
            let existing = all::<Label>(tx, "labels")?;
            label_taxonomy_conflicts(
                &existing,
                &fields.name,
                &fields.aliases,
                &fields.linear_origins,
                None,
            )?;
            let label = Label {
                id: id(),
                name: fields.name,
                description: description.clone(),
                color: color.to_lowercase(),
                aliases: fields.aliases,
                products: fields.products,
                linear_origins: fields.linear_origins,
                version: 1,
                created_at: at,
                updated_at: at,
            };
            put_label(tx, &label)?;
            emit(tx, actor, "label_created", &label.id, at)?;
            Ok(json!(label))
        }
        Command::UpdateLabel {
            id,
            expected_version,
            name,
            description,
            color,
            aliases,
            products,
            linear_origins,
        } => {
            human(role)?;
            let mut label = label(tx, id)?;
            if label.version != *expected_version {
                return Err(err(
                    "conflict",
                    "Label changed; reopen its editor before retrying",
                ));
            }
            let fields =
                validate_label_fields(name, description, color, aliases, products, linear_origins)?;
            check_label_products(tx, &fields.products)?;
            let existing = all::<Label>(tx, "labels")?;
            label_taxonomy_conflicts(
                &existing,
                &fields.name,
                &fields.aliases,
                &fields.linear_origins,
                Some(id),
            )?;
            // Narrowing applicability must not orphan existing assignments; detach first.
            if !fields.products.is_empty() {
                let allowed: HashSet<&str> = fields
                    .products
                    .iter()
                    .map(|rule| rule.product_id.as_str())
                    .collect();
                let issue_conflict = all::<Issue>(tx, "issues")?.into_iter().find(|issue| {
                    issue.labels.contains(id) && !allowed.contains(issue.product_id.as_str())
                });
                if let Some(issue) = issue_conflict {
                    return Err(err(
                        "invalid",
                        format!("Label is attached to {} outside the selected products; detach it first", issue.key),
                    ));
                }
                let project_conflict =
                    all::<Project>(tx, "projects")?.into_iter().find(|project| {
                        project.labels.contains(id)
                            && !allowed.contains(project.product_id.as_str())
                    });
                if let Some(project) = project_conflict {
                    return Err(err(
                        "invalid",
                        format!("Label is attached to project “{}” outside the selected products; detach it first", project.name),
                    ));
                }
            }
            label.name = fields.name;
            label.description = description.clone();
            label.color = color.to_lowercase();
            label.aliases = fields.aliases;
            label.products = fields.products;
            label.linear_origins = fields.linear_origins;
            label.version += 1;
            label.updated_at = at;
            put_label(tx, &label)?;
            emit(tx, actor, "label_updated", id, at)?;
            Ok(json!(label))
        }
        Command::AttachIssueLabel {
            key,
            expected_version,
            label_id,
        }
        | Command::DetachIssueLabel {
            key,
            expected_version,
            label_id,
        } => {
            let attach = matches!(cmd, Command::AttachIssueLabel { .. });
            let mut i = version(tx, key, *expected_version)?;
            if role == Role::Agent {
                if matches!(
                    i.status,
                    Status::Verify | Status::Done | Status::LegacyCompleted | Status::Canceled
                ) {
                    human(role)?;
                }
                if i.status == Status::Doing {
                    held(&i, actor, at)?;
                }
            }
            let label = label(tx, label_id)?;
            let event = if attach {
                if !label_applies(&label, &i.product_id) {
                    return Err(err("invalid", "Label does not apply to this product"));
                }
                if i.labels.contains(label_id) {
                    return Err(err("conflict", "Label is already attached"));
                }
                i.labels.push(label.id);
                "issue_label_attached"
            } else {
                if !i.labels.contains(label_id) {
                    return Err(err("conflict", "Label is not attached"));
                }
                i.labels.retain(|id| id != label_id);
                "issue_label_detached"
            };
            // Labels are filtering metadata: readiness, priority, ownership and evidence stay as they are.
            save(tx, i, actor, event, at)
        }
        Command::AttachProjectLabel {
            id,
            expected_version,
            label_id,
        }
        | Command::DetachProjectLabel {
            id,
            expected_version,
            label_id,
        } => {
            human(role)?;
            let attach = matches!(cmd, Command::AttachProjectLabel { .. });
            let mut p = project(tx, id)?;
            if p.version != *expected_version {
                return Err(err(
                    "conflict",
                    "Project changed; reopen its editor before retrying",
                ));
            }
            let label = label(tx, label_id)?;
            let event = if attach {
                if !label_applies(&label, &p.product_id) {
                    return Err(err("invalid", "Label does not apply to this product"));
                }
                if p.labels.contains(label_id) {
                    return Err(err("conflict", "Label is already attached"));
                }
                p.labels.push(label.id);
                "project_label_attached"
            } else {
                if !p.labels.contains(label_id) {
                    return Err(err("conflict", "Label is not attached"));
                }
                p.labels.retain(|id| id != label_id);
                "project_label_detached"
            };
            p.version += 1;
            p.updated_at = at;
            put_project(tx, &p)?;
            emit(tx, actor, event, id, at)?;
            Ok(json!(p))
        }
        Command::SyncTheoria {
            product,
            source_root,
            catalog_version,
            documents,
        } => {
            required(source_root, "Theoria source root")?;
            limited(source_root, "Theoria source root", 2_048)?;
            if *catalog_version == 0 {
                return Err(err("invalid", "Catalog version must be positive"));
            }
            if documents.is_empty() || documents.len() > 50 {
                return Err(err("invalid", "Provide 1–50 Theoria documents"));
            }
            let product = all::<Product>(tx, "products")?
                .into_iter()
                .find(|candidate| candidate.key == *product)
                .ok_or_else(|| err("not_found", "Unknown product"))?;
            let existing = all::<TheoriaDocument>(tx, "theoria_documents")?;
            let mut seen = HashSet::new();
            let mut synced = Vec::with_capacity(documents.len());
            for input in documents {
                if !stable_id(&input.id) || !seen.insert(input.id.clone()) {
                    return Err(err(
                        "invalid",
                        "Theoria document IDs must be unique lowercase slugs",
                    ));
                }
                required(&input.title, "Theoria title")?;
                limited(&input.title, "Theoria title", 200)?;
                limited(&input.description, "Theoria description", 1_000)?;
                if ![
                    "principles",
                    "workflow",
                    "playbook",
                    "verification",
                    "context",
                    "governance",
                ]
                .contains(&input.category.as_str())
                {
                    return Err(err("invalid", "Unknown Theoria category"));
                }
                required(&input.relative_path, "Theoria relative path")?;
                limited(&input.relative_path, "Theoria relative path", 512)?;
                if input.relative_path.starts_with('/')
                    || input.relative_path.starts_with('\\')
                    || input
                        .relative_path
                        .split(['/', '\\'])
                        .any(|part| part == "..")
                {
                    return Err(err(
                        "invalid",
                        "Theoria paths must stay beneath the declared source root",
                    ));
                }
                if let Some(updated) = &input.source_updated {
                    limited(updated, "Theoria source update", 80)?;
                }
                let prior = existing.iter().find(|document| document.id == input.id);
                if prior.is_some_and(|document| document.product_id != product.id) {
                    return Err(err(
                        "conflict",
                        "Theoria document ID already belongs to another product",
                    ));
                }
                let (
                    availability,
                    source_updated,
                    source_modified_at,
                    fingerprint,
                    content,
                    cached_at,
                    reason,
                ) = match (&input.fingerprint, &input.content) {
                    (Some(fingerprint), Some(content)) => {
                        if !valid_fingerprint(fingerprint) {
                            return Err(err("invalid", "Theoria fingerprints must be SHA-256 hex"));
                        }
                        limited(content, "Theoria cached content", 400_000)?;
                        (
                            TheoriaAvailability::Available,
                            input.source_updated.clone(),
                            input.source_modified_at,
                            Some(fingerprint.clone()),
                            Some(content.clone()),
                            Some(at),
                            None,
                        )
                    }
                    (None, None) => {
                        let reason = input
                            .unavailable_reason
                            .as_deref()
                            .unwrap_or("source unavailable");
                        required(reason, "Theoria unavailable reason")?;
                        limited(reason, "Theoria unavailable reason", 300)?;
                        (
                            TheoriaAvailability::Unavailable,
                            input.source_updated.clone().or_else(|| {
                                prior.and_then(|document| document.source_updated.clone())
                            }),
                            input
                                .source_modified_at
                                .or_else(|| prior.and_then(|document| document.source_modified_at)),
                            prior.and_then(|document| document.fingerprint.clone()),
                            prior.and_then(|document| document.content.clone()),
                            prior.and_then(|document| document.cached_at),
                            Some(reason.to_string()),
                        )
                    }
                    _ => {
                        return Err(err(
                            "invalid",
                            "Theoria content and fingerprint must be supplied together",
                        ))
                    }
                };
                let document = TheoriaDocument {
                    id: input.id.clone(),
                    product_id: product.id.clone(),
                    title: input.title.trim().into(),
                    description: input.description.clone(),
                    category: input.category.clone(),
                    source_root: source_root.clone(),
                    relative_path: input.relative_path.clone(),
                    source_updated,
                    source_modified_at,
                    fingerprint,
                    content,
                    availability,
                    unavailable_reason: reason,
                    catalog_version: *catalog_version,
                    checked_at: at,
                    cached_at,
                };
                put_theoria_document(tx, &document)?;
                synced.push(document);
            }
            emit(tx, actor, "theoria_catalog_synced", &product.key, at)?;
            Ok(
                json!({"documents":synced,"source_authority":"Imported read-only cache; maintained content remains external"}),
            )
        }
        Command::LinkTheoria {
            key,
            expected_version,
            document_id,
            playbook_version,
        } => {
            let mut i = version(tx, key, *expected_version)?;
            if role == Role::Agent {
                held(&i, actor, at)?;
            }
            let document = theoria_document(tx, document_id)?;
            if document.product_id != i.product_id {
                return Err(err(
                    "invalid",
                    "Theoria guidance must belong to the issue's product",
                ));
            }
            if i.theoria_refs
                .iter()
                .any(|reference| reference.document_id == *document_id)
            {
                return Err(err("conflict", "Guidance is already linked"));
            }
            let playbook_version = playbook_version
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            if let Some(value) = &playbook_version {
                limited(value, "playbook version", 80)?;
            }
            i.theoria_refs.push(TheoriaReference {
                document_id: document.id,
                recorded_fingerprint: document.fingerprint,
                playbook_version,
                linked_by: actor.into(),
                linked_at: at,
            });
            save(tx, i, actor, "theoria_guidance_linked", at)
        }
        Command::CreateMethodFinding {
            key,
            expected_version,
            classification,
            observation,
            hypothesis,
            proposal,
            evidence,
        } => {
            let i = version(tx, key, *expected_version)?;
            if role == Role::Agent {
                held(&i, actor, at)?;
            }
            required(observation, "observed fact")?;
            required(proposal, "proposed improvement")?;
            limited(observation, "observed fact", 4_000)?;
            limited(hypothesis, "hypothesis", 4_000)?;
            limited(proposal, "proposed improvement", 4_000)?;
            if evidence.len() > 10 {
                return Err(err("invalid", "Provide at most 10 evidence pointers"));
            }
            let mut evidence = evidence.clone();
            for pointer in &evidence {
                required(&pointer.reference, "evidence reference")?;
                limited(&pointer.reference, "evidence reference", 500)?;
                limited(&pointer.summary, "evidence summary", 1_000)?;
            }
            if !evidence
                .iter()
                .any(|pointer| pointer.kind == EvidenceKind::Issue && pointer.reference == *key)
            {
                evidence.insert(
                    0,
                    EvidencePointer {
                        kind: EvidenceKind::Issue,
                        reference: key.clone(),
                        summary: "Originating Direct issue".into(),
                    },
                );
            }
            if evidence.len() > 10 {
                return Err(err("invalid", "Provide at most 10 evidence pointers"));
            }
            let finding = MethodFinding {
                id: id(),
                issue_key: key.clone(),
                classification: classification.clone(),
                observation: observation.clone(),
                hypothesis: hypothesis.clone(),
                proposal: proposal.clone(),
                evidence,
                created_by: actor.into(),
                created_at: at,
            };
            put_method_finding(tx, &finding)?;
            save(tx, i, actor, "method_finding_proposed", at)
        }
        Command::RecordGitTrace {
            key,
            expected_version,
            kind,
            repository,
            commit_sha,
            branch,
            remote,
            remote_ref,
        } => {
            let i = version(tx, key, *expected_version)?;
            held(&i, actor, at)?;
            if i.status != Status::Doing {
                return Err(err("invalid", "Git evidence requires active Doing work"));
            }
            let repository = repository.trim();
            let commit_sha = commit_sha.trim().to_ascii_lowercase();
            let branch = branch.trim();
            let remote = remote.as_deref().map(str::trim);
            let remote_ref = remote_ref.as_deref().map(str::trim);
            validate_git_trace_fields(kind, repository, &commit_sha, branch, remote, remote_ref)?;
            let duplicate = all::<GitTrace>(tx, "git_traces")?.into_iter().any(|trace| {
                trace.issue_key == *key
                    && trace.kind == *kind
                    && trace.repository == repository
                    && trace.commit_sha == commit_sha
                    && trace.branch == branch
                    && trace.remote.as_deref() == remote
                    && trace.remote_ref.as_deref() == remote_ref
            });
            if duplicate {
                return Err(err(
                    "conflict",
                    "This Git evidence is already linked to the issue",
                ));
            }
            let trace = GitTrace {
                id: id(),
                issue_key: key.clone(),
                kind: kind.clone(),
                repository: repository.into(),
                commit_sha,
                branch: branch.into(),
                remote: remote.map(str::to_string),
                remote_ref: remote_ref.map(str::to_string),
                recorded_by: actor.into(),
                recorded_at: at,
            };
            put_git_trace(tx, &trace)?;
            let event = match kind {
                GitTraceKind::Commit => "git_commit_recorded",
                GitTraceKind::Push => "git_push_recorded",
            };
            save(tx, i, actor, event, at)
        }
        Command::SetReleaseWorkflowConfig {
            product,
            expected_version,
            branch_strategy,
            production_ref,
            release_branch_pattern,
            preview_environment,
            preview_url_template,
            promotion_policy,
        } => {
            human(role)?;
            validate_release_workflow_fields(
                branch_strategy,
                production_ref,
                release_branch_pattern,
                preview_environment,
                preview_url_template,
            )?;
            let product = all::<Product>(tx, "products")?
                .into_iter()
                .find(|candidate| candidate.key == *product)
                .ok_or_else(|| err("not_found", "Unknown product"))?;
            let prior = release_workflow(tx, &product.id)?;
            match (&prior, expected_version) {
                (Some(prior), Some(expected)) if prior.version == *expected => {}
                (None, None) => {}
                (Some(_), None) => {
                    return Err(err(
                        "conflict",
                        "Existing workflow requires expected_version",
                    ))
                }
                _ => {
                    return Err(err(
                        "conflict",
                        "Release workflow version changed; refresh and retry",
                    ))
                }
            }
            let workflow = ReleaseWorkflowConfig {
                product_id: product.id.clone(),
                branch_strategy: branch_strategy.clone(),
                production_ref: production_ref.trim().into(),
                release_branch_pattern: release_branch_pattern.trim().into(),
                preview_environment: preview_environment.trim().into(),
                preview_url_template: preview_url_template.trim().into(),
                promotion_policy: promotion_policy.clone(),
                version: prior.as_ref().map_or(1, |prior| prior.version + 1),
                created_at: prior.as_ref().map_or(at, |prior| prior.created_at),
                updated_at: at,
            };
            put_release_workflow(tx, &workflow)?;
            for release in all::<ReleaseRecord>(tx, "releases")?
                .iter()
                .filter(|release| {
                    release.product_id == product.id
                        && !matches!(
                            release.status,
                            ReleaseStatus::Retired | ReleaseStatus::Canceled
                        )
                })
            {
                validate_release_branch(
                    tx,
                    &product.id,
                    Some(&release.id),
                    &release.version_label,
                    &release.target_ref,
                    release.release_branch.as_deref(),
                )?;
            }
            emit(tx, actor, "release_workflow_configured", &product.key, at)?;
            Ok(json!(workflow))
        }
        Command::CreateRelease {
            product,
            name,
            version_label,
            target_ref,
            release_branch,
            preview_url,
            notes,
            project_ids,
            issue_keys,
            external_source,
            external_id,
        } => {
            human(role)?;
            validate_release_fields(
                name,
                version_label,
                target_ref,
                preview_url.as_deref(),
                notes,
            )?;
            validate_optional_provenance(external_source.as_deref(), external_id.as_deref())?;
            let product = all::<Product>(tx, "products")?
                .into_iter()
                .find(|candidate| candidate.key == *product)
                .ok_or_else(|| err("not_found", "Unknown product"))?;
            release_name(tx, &product.id, version_label, None)?;
            validate_release_links(tx, &product.id, project_ids, issue_keys)?;
            validate_release_branch(
                tx,
                &product.id,
                None,
                version_label,
                target_ref,
                release_branch.as_deref(),
            )?;
            let release = ReleaseRecord {
                id: id(),
                product_id: product.id,
                name: name.trim().into(),
                version_label: version_label.trim().into(),
                status: ReleaseStatus::Planned,
                target_ref: target_ref.trim().into(),
                release_branch: release_branch
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                preview_url: preview_url
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                notes: notes.clone(),
                project_ids: project_ids.clone(),
                issue_keys: issue_keys.clone(),
                external_source: external_source.clone(),
                external_id: external_id.clone(),
                external_url: None,
                version: 1,
                created_at: at,
                updated_at: at,
            };
            put_release(tx, &release)?;
            emit(tx, actor, "release_created", &release.id, at)?;
            Ok(json!(release))
        }
        Command::UpdateRelease {
            id,
            expected_version,
            name,
            version_label,
            status,
            target_ref,
            release_branch,
            preview_url,
            notes,
            project_ids,
            issue_keys,
        } => {
            human(role)?;
            let mut release = release(tx, id)?;
            if release.version != *expected_version {
                return Err(err(
                    "conflict",
                    "Release version changed; refresh and retry",
                ));
            }
            validate_release_fields(
                name,
                version_label,
                target_ref,
                preview_url.as_deref(),
                notes,
            )?;
            release_name(tx, &release.product_id, version_label, Some(id))?;
            validate_release_links(tx, &release.product_id, project_ids, issue_keys)?;
            validate_release_branch(
                tx,
                &release.product_id,
                Some(id),
                version_label,
                target_ref,
                release_branch.as_deref(),
            )?;
            if matches!(
                release.status,
                ReleaseStatus::Preview | ReleaseStatus::Production
            ) && (release.version_label != version_label.trim()
                || release.target_ref != target_ref.trim()
                || release.release_branch.as_deref()
                    != release_branch
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                || release.project_ids != *project_ids
                || release.issue_keys != *issue_keys)
            {
                return Err(err(
                    "invalid",
                    "Release version, target, and linked work are frozen after deployment evidence",
                ));
            }
            let status_allowed = matches!(
                (&release.status, status),
                (
                    ReleaseStatus::Planned,
                    ReleaseStatus::Planned | ReleaseStatus::Active | ReleaseStatus::Canceled,
                ) | (
                    ReleaseStatus::Active,
                    ReleaseStatus::Active | ReleaseStatus::Canceled
                ) | (
                    ReleaseStatus::Preview,
                    ReleaseStatus::Preview | ReleaseStatus::Canceled
                ) | (
                    ReleaseStatus::Production,
                    ReleaseStatus::Production | ReleaseStatus::Retired
                ) | (ReleaseStatus::Retired, ReleaseStatus::Retired)
                    | (ReleaseStatus::Canceled, ReleaseStatus::Canceled)
            );
            if !status_allowed {
                return Err(err(
                    "invalid",
                    "Preview and production status require deployment evidence; completed lifecycle states cannot be reopened",
                ));
            }
            release.name = name.trim().into();
            release.version_label = version_label.trim().into();
            release.status = status.clone();
            release.target_ref = target_ref.trim().into();
            release.release_branch = release_branch
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            release.preview_url = preview_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            release.notes = notes.clone();
            release.project_ids = project_ids.clone();
            release.issue_keys = issue_keys.clone();
            release.version += 1;
            release.updated_at = at;
            put_release(tx, &release)?;
            emit(tx, actor, "release_updated", id, at)?;
            Ok(json!(release))
        }
        Command::RecordReleaseEvidence {
            release_id,
            expected_version,
            kind,
            git_trace_id,
            verification_id,
            deployment_ref,
            commit_sha,
            target_ref,
            source_ref,
            environment,
            url,
            outcome,
            note,
        } => {
            human(role)?;
            let mut release = release(tx, release_id)?;
            if release.version != *expected_version {
                return Err(err(
                    "conflict",
                    "Release version changed; refresh and retry",
                ));
            }
            if matches!(
                release.status,
                ReleaseStatus::Retired | ReleaseStatus::Canceled
            ) {
                return Err(err(
                    "invalid",
                    "Evidence cannot be added to a closed release",
                ));
            }
            let existing = all::<ReleaseEvidence>(tx, "release_evidence")?;
            let all_issues = all::<Issue>(tx, "issues")?;
            let linked_issues = release_issues(&release, &all_issues)
                .into_iter()
                .map(|issue| issue.key.clone())
                .collect::<HashSet<_>>();
            let mut issue_key = None;
            let mut normalized_trace = None;
            let mut normalized_verification = None;
            let mut normalized_deployment = None;
            let mut normalized_commit = None;
            let mut normalized_target = None;
            let mut normalized_source = None;
            let mut normalized_environment = None;
            let mut normalized_url = None;
            let mut approver = None;
            match kind {
                ReleaseEvidenceKind::Commit | ReleaseEvidenceKind::Push => {
                    if *outcome != Outcome::Passed {
                        return Err(err(
                            "invalid",
                            "Git evidence records only successful commands",
                        ));
                    }
                    let trace_id = git_trace_id.as_deref().ok_or_else(|| {
                        err("invalid", "Git release evidence requires a Git trace")
                    })?;
                    if verification_id.is_some()
                        || deployment_ref.is_some()
                        || commit_sha.is_some()
                        || target_ref.is_some()
                        || source_ref.is_some()
                        || environment.is_some()
                        || url.is_some()
                    {
                        return Err(err(
                            "invalid",
                            "Git release evidence only accepts git_trace_id",
                        ));
                    }
                    let trace = all::<GitTrace>(tx, "git_traces")?
                        .into_iter()
                        .find(|trace| trace.id == trace_id)
                        .ok_or_else(|| err("not_found", "Unknown Git trace"))?;
                    let expected_kind = if *kind == ReleaseEvidenceKind::Commit {
                        GitTraceKind::Commit
                    } else {
                        GitTraceKind::Push
                    };
                    if trace.kind != expected_kind || !linked_issues.contains(&trace.issue_key) {
                        return Err(err(
                            "invalid",
                            "Git evidence must match its kind and a real issue in this release",
                        ));
                    }
                    if release
                        .release_branch
                        .as_deref()
                        .is_some_and(|branch| branch != trace.branch)
                    {
                        return Err(err(
                            "invalid",
                            "Git evidence branch must match the release-owned branch",
                        ));
                    }
                    let traces = all::<GitTrace>(tx, "git_traces")?;
                    let releases = all::<ReleaseRecord>(tx, "releases")?;
                    if existing.iter().any(|evidence| {
                        evidence.release_id != *release_id
                            && evidence.git_trace_id.as_deref().is_some_and(|id| {
                                traces.iter().any(|prior| {
                                    prior.id == id && prior.commit_sha == trace.commit_sha
                                })
                            })
                            && releases.iter().any(|other| {
                                other.id == evidence.release_id
                                    && !matches!(
                                        other.status,
                                        ReleaseStatus::Retired | ReleaseStatus::Canceled
                                    )
                            })
                    }) {
                        return Err(err(
                            "conflict",
                            "This commit is already owned by another active release",
                        ));
                    }
                    issue_key = Some(trace.issue_key);
                    normalized_trace = Some(trace.id);
                }
                ReleaseEvidenceKind::Check => {
                    if *outcome != Outcome::Passed {
                        return Err(err("invalid", "Check evidence must be owner-passed"));
                    }
                    let verification_id = verification_id.as_deref().ok_or_else(|| {
                        err("invalid", "Check evidence requires a verification run")
                    })?;
                    if git_trace_id.is_some()
                        || deployment_ref.is_some()
                        || commit_sha.is_some()
                        || target_ref.is_some()
                        || source_ref.is_some()
                        || environment.is_some()
                        || url.is_some()
                    {
                        return Err(err(
                            "invalid",
                            "Check evidence only accepts verification_id",
                        ));
                    }
                    let verification = run(tx, verification_id)?;
                    let linked = issue(tx, &verification.issue_key)?;
                    if !linked_issues.contains(&verification.issue_key)
                        || linked.status != Status::Done
                        || verification.outcome != Outcome::Passed
                        || verification.results.len() != verification.steps.len()
                        || verification
                            .results
                            .iter()
                            .any(|result| result.outcome != Outcome::Passed)
                    {
                        return Err(err(
                            "invalid",
                            "Release checks require owner-passed verification for a linked Done issue",
                        ));
                    }
                    issue_key = Some(verification.issue_key);
                    normalized_verification = Some(verification.id);
                }
                ReleaseEvidenceKind::PreviewDeployment
                | ReleaseEvidenceKind::ProductionDeployment
                | ReleaseEvidenceKind::Rollback => {
                    if git_trace_id.is_some() || verification_id.is_some() {
                        return Err(err(
                            "invalid",
                            "Deployment evidence cannot substitute a Git trace or verification run",
                        ));
                    }
                    let deployment = deployment_ref.as_deref().ok_or_else(|| {
                        err(
                            "invalid",
                            "Deployment evidence requires a deployment reference",
                        )
                    })?;
                    let commit = commit_sha
                        .as_deref()
                        .map(str::trim)
                        .map(str::to_ascii_lowercase)
                        .ok_or_else(|| {
                            err("invalid", "Deployment evidence requires a commit SHA")
                        })?;
                    let deployed_ref = target_ref.as_deref().ok_or_else(|| {
                        err(
                            "invalid",
                            "Deployment evidence requires the deployed target ref",
                        )
                    })?;
                    let deployed_environment = environment.as_deref().ok_or_else(|| {
                        err("invalid", "Deployment evidence requires its environment")
                    })?;
                    let deployment_url = url.as_deref();
                    required(deployment, "deployment reference")?;
                    required(deployed_environment, "deployment environment")?;
                    limited(deployment, "deployment reference", 512)?;
                    if let Some(url) = deployment_url {
                        required(url, "deployment URL")?;
                        limited(url, "deployment URL", 2_000)?;
                    }
                    let expected_ref = if *kind == ReleaseEvidenceKind::PreviewDeployment {
                        release
                            .release_branch
                            .as_deref()
                            .unwrap_or(&release.target_ref)
                    } else {
                        &release.target_ref
                    };
                    if !valid_commit_sha(&commit)
                        || (*kind != ReleaseEvidenceKind::Rollback && deployed_ref != expected_ref)
                    {
                        return Err(err(
                            "invalid",
                            "Deployment must identify a full commit SHA and the release target ref",
                        ));
                    }
                    if *kind == ReleaseEvidenceKind::Rollback
                        && source_ref
                            .as_deref()
                            .is_none_or(|value| value.trim().is_empty())
                    {
                        return Err(err("invalid", "Rollback evidence requires the prior ref"));
                    }
                    if *outcome != Outcome::Passed && note.trim().is_empty() {
                        return Err(err("invalid", "Failed or canceled attempts require a note"));
                    }
                    if let Some(workflow) = release_workflow(tx, &release.product_id)? {
                        if *kind == ReleaseEvidenceKind::PreviewDeployment
                            && deployed_environment != workflow.preview_environment
                        {
                            return Err(err(
                                "invalid",
                                "Preview environment must match the product workflow",
                            ));
                        }
                    }
                    let traces = all::<GitTrace>(tx, "git_traces")?;
                    let has_linked_trace = existing.iter().any(|evidence| {
                        evidence.release_id == *release_id
                            && matches!(
                                evidence.kind,
                                ReleaseEvidenceKind::Commit | ReleaseEvidenceKind::Push
                            )
                            && evidence.git_trace_id.as_ref().is_some_and(|trace_id| {
                                traces.iter().any(|trace| {
                                    trace.id == *trace_id && trace.commit_sha == commit
                                })
                            })
                    });
                    if !has_linked_trace {
                        return Err(err(
                            "invalid",
                            "Deployment commit must first be attached as release Git evidence",
                        ));
                    }
                    if *outcome == Outcome::Passed
                        && *kind == ReleaseEvidenceKind::ProductionDeployment
                    {
                        let has_push = existing.iter().any(|evidence| {
                            evidence.release_id == *release_id
                                && evidence.kind == ReleaseEvidenceKind::Push
                                && evidence.outcome == Outcome::Passed
                                && evidence.git_trace_id.as_ref().is_some_and(|trace_id| {
                                    traces.iter().any(|trace| {
                                        trace.id == *trace_id && trace.commit_sha == commit
                                    })
                                })
                        });
                        let has_preview = existing.iter().any(|evidence| {
                            evidence.release_id == *release_id
                                && evidence.kind == ReleaseEvidenceKind::PreviewDeployment
                                && evidence.outcome == Outcome::Passed
                                && evidence.commit_sha.as_deref() == Some(commit.as_str())
                        });
                        let issues = all::<Issue>(tx, "issues")?;
                        let work = release_issues(&release, &issues);
                        if !has_push
                            || !has_preview
                            || work.is_empty()
                            || work.iter().any(|i| i.status != Status::Done)
                        {
                            return Err(err(
                                "invalid",
                                "Production requires a successful push, matching preview, and owner-verified Done for every linked issue",
                            ));
                        }
                        approver = Some(actor.to_string());
                        release.status = ReleaseStatus::Production;
                    } else if *outcome == Outcome::Passed
                        && *kind == ReleaseEvidenceKind::PreviewDeployment
                    {
                        release.status = ReleaseStatus::Preview;
                        release.preview_url = deployment_url.map(|url| url.trim().into());
                    }
                    normalized_deployment = Some(deployment.trim().into());
                    normalized_commit = Some(commit);
                    normalized_target = Some(deployed_ref.trim().into());
                    normalized_source = source_ref.as_deref().map(str::trim).map(str::to_string);
                    normalized_environment = Some(deployed_environment.trim().into());
                    normalized_url = deployment_url.map(|url| url.trim().into());
                }
            }
            let evidence = ReleaseEvidence {
                id: id(),
                release_id: release_id.clone(),
                kind: kind.clone(),
                issue_key,
                git_trace_id: normalized_trace,
                verification_id: normalized_verification,
                deployment_ref: normalized_deployment,
                commit_sha: normalized_commit,
                target_ref: normalized_target,
                source_ref: normalized_source,
                environment: normalized_environment,
                url: normalized_url,
                approver,
                outcome: outcome.clone(),
                note: note.clone(),
                recorded_by: actor.into(),
                recorded_at: at,
            };
            if existing.iter().any(|prior| {
                prior.release_id == evidence.release_id
                    && prior.kind == evidence.kind
                    && prior.git_trace_id == evidence.git_trace_id
                    && prior.verification_id == evidence.verification_id
                    && prior.deployment_ref == evidence.deployment_ref
            }) {
                return Err(err("conflict", "This release evidence is already recorded"));
            }
            put_release_evidence(tx, &evidence)?;
            release.version += 1;
            release.updated_at = at;
            put_release(tx, &release)?;
            let event = match kind {
                ReleaseEvidenceKind::Commit => "release_commit_recorded",
                ReleaseEvidenceKind::Push => "release_push_recorded",
                ReleaseEvidenceKind::Check => "release_check_recorded",
                ReleaseEvidenceKind::PreviewDeployment => "release_preview_recorded",
                ReleaseEvidenceKind::ProductionDeployment => "release_production_recorded",
                ReleaseEvidenceKind::Rollback => "release_rollback_recorded",
            };
            emit(tx, actor, event, release_id, at)?;
            Ok(json!({"release": release, "evidence": evidence}))
        }
        Command::CreateProduct {
            key,
            name,
            repo_windows,
            repo_wsl,
            vault_windows,
            vault_wsl,
        } => {
            human(role)?;
            required(name, "name")?;
            if key.is_empty() || key.len() > 8 || !key.bytes().all(|c| c.is_ascii_uppercase()) {
                return Err(err("invalid", "Product key must be 1–8 uppercase letters"));
            }
            let products = all::<Product>(tx, "products")?;
            if products.iter().any(|p| p.key == *key) {
                return Err(err("conflict", "Product key already exists"));
            }
            let p = Product {
                id: id(),
                key: key.clone(),
                name: name.trim().into(),
                repo_windows: repo_windows.clone(),
                repo_wsl: repo_wsl.clone(),
                vault_windows: vault_windows.clone(),
                vault_wsl: vault_wsl.clone(),
                // New products join the end of the ungrouped sidebar list.
                sort_order: products.iter().map(|p| p.sort_order + 1).max().unwrap_or(0),
                section_id: None,
            };
            tx.execute(
                "INSERT INTO products VALUES (?1,?2,?3)",
                params![p.id, p.key, serde_json::to_string(&p)?],
            )?;
            emit(tx, actor, "product_created", key, at)?;
            Ok(json!(p))
        }
        Command::UpdateProductPaths {
            product,
            repo_windows,
            repo_wsl,
            vault_windows,
            vault_wsl,
        } => {
            human(role)?;
            let mut p = all::<Product>(tx, "products")?
                .into_iter()
                .find(|p| p.key == *product)
                .ok_or_else(|| err("not_found", "Unknown product"))?;
            let clean = |value: &str, field: &str| -> Result<String> {
                let value = value.trim();
                if value.chars().count() > 500 || value.contains('\0') {
                    return Err(err("invalid", format!("{field} is too long or invalid")));
                }
                Ok(value.into())
            };
            let next = (
                clean(repo_windows, "Windows repository path")?,
                clean(repo_wsl, "WSL repository path")?,
                clean(vault_windows, "Windows vault path")?,
                clean(vault_wsl, "WSL vault path")?,
            );
            if (
                p.repo_windows.as_str(),
                p.repo_wsl.as_str(),
                p.vault_windows.as_str(),
                p.vault_wsl.as_str(),
            ) == (
                next.0.as_str(),
                next.1.as_str(),
                next.2.as_str(),
                next.3.as_str(),
            ) {
                return Err(err("conflict", "Nothing changed"));
            }
            (p.repo_windows, p.repo_wsl, p.vault_windows, p.vault_wsl) = next;
            put_product(tx, &p)?;
            emit(tx, actor, "product_paths_updated", &p.key, at)?;
            Ok(json!(p))
        }
        Command::CreateProductSection { name } => {
            human(role)?;
            let sections = all::<ProductSection>(tx, "product_sections")?;
            let section = ProductSection {
                id: id(),
                name: product_section_name(&sections, name, None)?,
                sort_order: sections.iter().map(|s| s.sort_order + 1).max().unwrap_or(0),
                version: 1,
                created_at: at,
                updated_at: at,
            };
            put_product_section(tx, &section)?;
            emit(tx, actor, "product_section_created", &section.id, at)?;
            Ok(json!(section))
        }
        Command::UpdateProductSection {
            id,
            expected_version,
            name,
        } => {
            human(role)?;
            let mut section = product_section(tx, id)?;
            if section.version != *expected_version {
                return Err(err(
                    "conflict",
                    "Sidebar section changed; refresh and retry",
                ));
            }
            let sections = all::<ProductSection>(tx, "product_sections")?;
            section.name = product_section_name(&sections, name, Some(id))?;
            section.version += 1;
            section.updated_at = at;
            put_product_section(tx, &section)?;
            emit(tx, actor, "product_section_updated", id, at)?;
            Ok(json!(section))
        }
        Command::DeleteProductSection {
            id,
            expected_version,
        } => {
            human(role)?;
            let section = product_section(tx, id)?;
            if section.version != *expected_version {
                return Err(err(
                    "conflict",
                    "Sidebar section changed; refresh and retry",
                ));
            }
            let mut ungrouped = Vec::new();
            let mut products = all::<Product>(tx, "products")?;
            products.sort_by_key(|p| p.sort_order);
            for mut product in products {
                if product.section_id.as_deref() == Some(id.as_str()) {
                    product.section_id = None;
                    put_product(tx, &product)?;
                    ungrouped.push(product.key);
                }
            }
            tx.execute("DELETE FROM product_sections WHERE id=?1", [id])?;
            emit(tx, actor, "product_section_deleted", id, at)?;
            Ok(json!({"deleted": section, "ungrouped_products": ungrouped}))
        }
        Command::ArrangeProducts { sections, products } => {
            human(role)?;
            let current_sections = all::<ProductSection>(tx, "product_sections")?;
            let current_products = all::<Product>(tx, "products")?;
            let stale = || {
                err(
                    "conflict",
                    "The sidebar changed since it was loaded; refresh and arrange again",
                )
            };
            let mut seen = HashSet::new();
            if sections.len() != current_sections.len()
                || !sections.iter().all(|id| {
                    seen.insert(id.as_str()) && current_sections.iter().any(|s| s.id == *id)
                })
            {
                return Err(stale());
            }
            let mut seen = HashSet::new();
            if products.len() != current_products.len()
                || !products.iter().all(|placement| {
                    seen.insert(placement.product_id.as_str())
                        && current_products
                            .iter()
                            .any(|p| p.id == placement.product_id)
                })
            {
                return Err(stale());
            }
            if products.iter().any(|placement| {
                placement
                    .section_id
                    .as_ref()
                    .is_some_and(|section| !sections.contains(section))
            }) {
                return Err(err("invalid", "A product names an unknown sidebar section"));
            }
            for (index, section_id) in sections.iter().enumerate() {
                let mut section = current_sections
                    .iter()
                    .find(|s| s.id == *section_id)
                    .cloned()
                    .expect("checked above");
                if section.sort_order != index as i64 {
                    section.sort_order = index as i64;
                    section.version += 1;
                    section.updated_at = at;
                    put_product_section(tx, &section)?;
                }
            }
            for (index, placement) in products.iter().enumerate() {
                let mut product = current_products
                    .iter()
                    .find(|p| p.id == placement.product_id)
                    .cloned()
                    .expect("checked above");
                if product.sort_order != index as i64 || product.section_id != placement.section_id
                {
                    product.sort_order = index as i64;
                    product.section_id = placement.section_id.clone();
                    put_product(tx, &product)?;
                }
            }
            emit(tx, actor, "products_arranged", "sidebar", at)?;
            Ok(json!({
                "products": all::<Product>(tx, "products")?,
                "product_sections": all::<ProductSection>(tx, "product_sections")?,
            }))
        }
        Command::CreateIssue {
            product,
            title,
            body,
            acceptance,
            owner,
            priority,
            planning_scope,
            project_id,
            template,
            intake,
        } => {
            required(title, "title")?;
            if let Some(context) = intake {
                context.validate().map_err(|e| err("invalid", e))?;
            }
            if !valid_priority(priority) {
                return Err(err("invalid", "Unknown priority"));
            }
            let p = all::<Product>(tx, "products")?
                .into_iter()
                .find(|p| p.key == *product)
                .ok_or_else(|| err("not_found", "Unknown product"))?;
            if let Some(id) = project_id {
                let selected = project(tx, id)?;
                if selected.product_id != p.id {
                    return Err(err("invalid", "Project must belong to the issue's product"));
                }
            }
            let mut i = new_issue(
                tx,
                &p,
                title,
                body,
                planning_scope.clone(),
                project_id.clone(),
                at,
            )?;
            i.acceptance = acceptance.clone();
            i.intake = intake.clone();
            i.owner = owner.clone();
            i.priority = priority.clone();
            // Product defaults attach the shared definition; they never copy it per product.
            i.labels = all::<Label>(tx, "labels")?
                .into_iter()
                .filter(|label| {
                    label
                        .products
                        .iter()
                        .any(|rule| rule.product_id == p.id && rule.default_for_new_issues)
                })
                .map(|label| label.id)
                .collect();
            // A template records provenance and kept label suggestions only; the
            // issue still starts in Backlog with no claim or verification state.
            if let Some(selection) = template {
                let (used, kept) = templates::apply(
                    tx,
                    selection,
                    templates::Intake {
                        target: TemplateTarget::Issue,
                        product: &p,
                        priority,
                        planning_scope: Some(planning_scope),
                    },
                    actor,
                    at,
                )?;
                for id in kept {
                    if !i.labels.contains(&id) {
                        i.labels.push(id);
                    }
                }
                i.template = Some(used);
            }
            put_issue(tx, &i)?;
            emit(tx, actor, "issue_created", &i.key, at)?;
            Ok(json!(i))
        }
        Command::UpdateIssue {
            key,
            expected_version,
            title,
            body,
            acceptance,
            owner,
            priority,
            planning_scope,
            intake,
        } => {
            let mut i = version(tx, key, *expected_version)?;
            required(title, "title")?;
            if let Some(context) = intake {
                context.validate().map_err(|e| err("invalid", e))?;
                i.intake = Some(context.clone());
            }
            if !valid_priority(priority) {
                return Err(err("invalid", "Unknown priority"));
            }
            if matches!(
                i.status,
                Status::Verify | Status::Done | Status::LegacyCompleted | Status::Canceled
            ) {
                return Err(err(
                    "invalid",
                    "Reopen before changing submitted or completed work",
                ));
            }
            if role == Role::Agent && i.status == Status::Doing {
                held(&i, actor, at)?;
            }
            i.title = title.trim().into();
            i.body = body.clone();
            i.acceptance = acceptance.clone();
            i.owner = owner.clone();
            i.priority = priority.clone();
            if let Some(scope) = planning_scope {
                i.planning_scope = scope.clone();
            }
            if i.status == Status::Ready {
                i.status = Status::Backlog;
            }
            if i.planning_scope == PlanningScope::Project
                && i.project_id.is_none()
                && !matches!(
                    i.status,
                    Status::Backlog | Status::LegacyCompleted | Status::Canceled
                )
            {
                return Err(err("invalid", "Active project work must have a project"));
            }
            save(tx, i, actor, "issue_updated", at)
        }
        Command::DeleteIssue {
            key,
            expected_version,
        } => {
            human(role)?;
            let i = version(tx, key, *expected_version)?;
            // Recomputed inside this transaction: a context preview cannot authorize deletion.
            let eligibility = deletion_eligibility(tx, &i, at)?;
            if !eligibility.eligible {
                let code = if eligibility
                    .blockers
                    .iter()
                    .any(|blocker| blocker.kind == DeletionBlockerKind::Status)
                {
                    "invalid"
                } else {
                    "conflict"
                };
                let reasons = eligibility
                    .blockers
                    .iter()
                    .map(|blocker| blocker.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(err(code, format!("{key} cannot be deleted: {reasons}.")));
            }
            tx.execute("DELETE FROM issues WHERE id=?1", [&i.id])?;
            tx.execute("DELETE FROM issue_histories WHERE issue_id=?1", [&i.id])?;
            emit(tx, actor, "issue_deleted", key, at)?;
            Ok(json!({
                "deleted_key": i.key,
                "deleted_id": i.id,
                "deleted_at": at,
            }))
        }
        Command::Ready {
            key,
            expected_version,
        } => {
            human(role)?;
            let mut i = version(tx, key, *expected_version)?;
            if i.status != Status::Backlog {
                return Err(err("invalid", "Only Backlog work can be made Ready"));
            }
            required(&i.body, "problem / outcome")?;
            required(&i.acceptance, "acceptance criteria")?;
            required(&i.owner, "human owner")?;
            if i.planning_scope == PlanningScope::Project && i.project_id.is_none() {
                return Err(err(
                    "invalid",
                    "Project-scoped work must choose a project before Ready",
                ));
            }
            i.status = Status::Ready;
            save(tx, i, actor, "issue_ready", at)
        }
        Command::Claim {
            key,
            expected_version,
            lease_seconds,
        } => {
            check_lease(*lease_seconds)?;
            let mut i = version(tx, key, *expected_version)?;
            if !matches!(i.status, Status::Ready | Status::Doing) {
                return Err(err(
                    "invalid",
                    "Only Ready or unclaimed Doing work can be claimed",
                ));
            }
            if i.claim.as_ref().is_some_and(|c| c.expires_at > at) {
                return Err(err("conflict", "An active claim already exists"));
            }
            i.claim = Some(Claim {
                actor: actor.into(),
                expires_at: at + lease_seconds,
            });
            i.status = Status::Doing;
            save(tx, i, actor, "work_claimed", at)
        }
        Command::Renew {
            key,
            expected_version,
            lease_seconds,
        } => {
            check_lease(*lease_seconds)?;
            let mut i = version(tx, key, *expected_version)?;
            held(&i, actor, at)?;
            i.claim.as_mut().unwrap().expires_at = at + lease_seconds;
            save(tx, i, actor, "claim_renewed", at)
        }
        Command::Release {
            key,
            expected_version,
        } => {
            let mut i = version(tx, key, *expected_version)?;
            held(&i, actor, at)?;
            i.claim = None;
            i.status = Status::Ready;
            save(tx, i, actor, "claim_released", at)
        }
        Command::Comment {
            key,
            expected_version,
            body,
        } => {
            let i = version(tx, key, *expected_version)?;
            required(body, "comment")?;
            let c = Comment {
                id: id(),
                issue_key: key.clone(),
                actor: actor.into(),
                body: body.clone(),
                at,
                external_source: None,
                external_id: None,
                external_url: None,
            };
            tx.execute(
                "INSERT INTO comments VALUES (?1,?2)",
                params![c.id, serde_json::to_string(&c)?],
            )?;
            save(tx, i, actor, "comment_added", at)
        }
        Command::Submit {
            key,
            expected_version,
            build_ref,
            delivery_ref,
            summary,
            checks,
            e2e,
            limitations,
            preconditions,
            steps,
        } => {
            let mut i = version(tx, key, *expected_version)?;
            held(&i, actor, at)?;
            if i.status != Status::Doing {
                return Err(err("invalid", "Work must be Doing before submission"));
            }
            let evidence = e2e.as_ref().ok_or_else(|| {
                err(
                    "invalid",
                    "End-to-end and delivered-build evidence is required before owner verification",
                )
            })?;
            validate_e2e(evidence, build_ref)?;
            for (v, n) in [
                (build_ref, "tested build"),
                (delivery_ref, "delivery reference"),
                (summary, "summary"),
                (checks, "checks"),
            ] {
                required(v, n)?;
            }
            if steps.is_empty() || steps.len() > 50 {
                return Err(err("invalid", "Provide 1–50 verification steps"));
            }
            for s in steps {
                required(&s.instruction, "test instruction")?;
                required(&s.expected, "expected result")?;
            }
            let p = all::<Product>(tx, "products")?
                .into_iter()
                .find(|p| p.id == i.product_id)
                .ok_or_else(|| err("invalid", format!("{key}'s product is missing")))?;
            let mut child = match &i.verification_key {
                Some(_) => verification_child(tx, &i)?,
                None => new_issue(
                    tx,
                    &p,
                    &format!("Verify: {}", i.title),
                    "Human verification",
                    i.planning_scope.clone(),
                    i.project_id.clone(),
                    at,
                )?,
            };
            child.parent = Some(key.clone());
            child.project_id = i.project_id.clone();
            child.milestone_id = i.milestone_id.clone();
            child.owner = i.owner.clone();
            child.status = Status::Ready;
            child.version += 1;
            child.updated_at = at;
            let v = Verification {
                id: id(),
                issue_key: key.clone(),
                build_ref: build_ref.clone(),
                delivery_ref: delivery_ref.clone(),
                summary: summary.clone(),
                checks: checks.clone(),
                e2e: e2e.clone(),
                limitations: limitations.clone(),
                preconditions: preconditions.clone(),
                steps: steps.clone(),
                submitted_by: actor.into(),
                submitted_at: at,
                outcome: Outcome::Pending,
                results: vec![],
                review_note: String::new(),
                reviewed_by: None,
                reviewed_at: None,
            };
            i.current_run = Some(v.id.clone());
            child.current_run = Some(v.id.clone());
            i.verification_key = Some(child.key.clone());
            i.status = Status::Verify;
            i.claim = None;
            i.needs_fix = false;
            put_run(tx, &v)?;
            put_issue(tx, &child)?;
            save(tx, i, actor, "verification_requested", at)
        }
        Command::Review {
            key,
            expected_version,
            run_id,
            outcome,
            results,
            note,
        } => {
            human(role)?;
            let mut i = version(tx, key, *expected_version)?;
            if i.status != Status::Verify {
                return Err(err("invalid", "Only submitted work can be reviewed"));
            }
            if i.current_run.as_deref() != Some(run_id.as_str()) {
                return Err(err(
                    "conflict",
                    "The submitted build changed; review the current test run",
                ));
            }
            let mut v = run(
                tx,
                i.current_run
                    .as_deref()
                    .ok_or_else(|| err("invalid", "Missing verification"))?,
            )?;
            if v.outcome != Outcome::Pending {
                return Err(err("conflict", "This test run is already closed"));
            }
            if *outcome == Outcome::Passed && v.e2e.is_none() {
                return Err(err("invalid", "This historical handoff needs agent E2E evidence and a new submission before acceptance"));
            }
            if *outcome == Outcome::Pending {
                return Err(err("invalid", "Select a review outcome"));
            }
            if *outcome == Outcome::Passed
                && (results.len() != v.steps.len()
                    || results.iter().any(|r| r.outcome != Outcome::Passed))
            {
                return Err(err(
                    "invalid",
                    "Every step must pass before work can be Done",
                ));
            }
            if *outcome == Outcome::Failed
                && (results.len() != v.steps.len()
                    || !results.iter().any(|r| r.outcome == Outcome::Failed))
            {
                return Err(err("invalid", "Record at least one failing step"));
            }
            if *outcome != Outcome::Passed {
                required(note, "review reason")?;
            }
            v.outcome = outcome.clone();
            v.results = results.clone();
            v.review_note = note.clone();
            v.reviewed_by = Some(actor.into());
            v.reviewed_at = Some(at);
            let mut child = verification_child(tx, &i)?;
            if *outcome == Outcome::Passed {
                i.status = Status::Done;
                child.status = Status::Done;
                i.needs_fix = false;
            } else {
                i.status = Status::Doing;
                i.needs_fix = true;
                child.status = if *outcome == Outcome::Canceled {
                    Status::Canceled
                } else {
                    Status::Doing
                };
            }
            child.version += 1;
            child.updated_at = at;
            put_run(tx, &v)?;
            put_issue(tx, &child)?;
            save(tx, i, actor, "verification_reviewed", at)
        }
        Command::Reopen {
            key,
            expected_version,
            reason,
        } => {
            human(role)?;
            let mut i = version(tx, key, *expected_version)?;
            required(reason, "reopen reason")?;
            if !matches!(i.status, Status::Done | Status::Verify) {
                return Err(err(
                    "invalid",
                    "Only submitted or completed work can be reopened",
                ));
            }
            if let Some(r) = &i.current_run {
                let mut v = run(tx, r)?;
                if v.outcome == Outcome::Pending {
                    v.outcome = Outcome::Canceled;
                    v.review_note = reason.clone();
                    v.reviewed_by = Some(actor.into());
                    v.reviewed_at = Some(at);
                    put_run(tx, &v)?;
                }
            }
            if i.verification_key.is_some() {
                let mut c = verification_child(tx, &i)?;
                c.status = Status::Canceled;
                c.version += 1;
                c.updated_at = at;
                put_issue(tx, &c)?;
            }
            i.current_run = None;
            i.status = Status::Doing;
            i.claim = None;
            i.needs_fix = true;
            let c = Comment {
                id: id(),
                issue_key: key.clone(),
                actor: actor.into(),
                body: format!("Reopened: {reason}"),
                at,
                external_source: None,
                external_id: None,
                external_url: None,
            };
            tx.execute(
                "INSERT INTO comments VALUES (?1,?2)",
                params![c.id, serde_json::to_string(&c)?],
            )?;
            save(tx, i, actor, "issue_reopened", at)
        }
        Command::CreateTemplate { .. }
        | Command::ReviseTemplate { .. }
        | Command::RetireTemplate { .. } => templates::mutate(tx, cmd, actor, role, at),
        Command::CaptureSignal { .. }
        | Command::UpdateSignal { .. }
        | Command::LinkSignal { .. }
        | Command::UnlinkSignal { .. }
        | Command::PromoteSignal { .. }
        | Command::ArchiveSignal { .. } => signals::mutate(tx, cmd, actor, role, at),
        Command::AddContextLink { .. }
        | Command::RemoveContextLink { .. }
        | Command::CheckContextLink { .. } => context::mutate(tx, cmd, actor, role, at),
        Command::ReadContextLink { .. } => {
            unreachable!("read-only command handled before the write path")
        }
        Command::RollbackMigration {
            bundle_id,
            expected_cursor,
        } => {
            human(role)?;
            migration::rollback(tx, bundle_id, *expected_cursor, actor, at)
        }
        _ => Err(err("invalid", "Not a mutation")),
    }
}

/// Full archive validation, including retained file bytes and record spans.
/// The status a generated verification child holds while `outcome` is its
/// parent's current run (set by submit and review; reopen clears the run and
/// cancels the child).
fn child_status_for(outcome: &Outcome) -> Status {
    match outcome {
        Outcome::Pending => Status::Ready,
        Outcome::Passed => Status::Done,
        Outcome::Failed => Status::Doing,
        Outcome::Canceled => Status::Canceled,
    }
}

fn inconsistent(message: String) -> Error {
    err(
        "invalid",
        format!("Inconsistent verification relationship: {message}"),
    )
}

/// Complete reciprocal parent ↔ verification child ↔ run validation, so a
/// restored workspace can never reach submit, review or reopen with a
/// relationship those commands cannot resolve.
fn validate_verification_relationships(a: &Archive) -> Result<()> {
    let issues: HashMap<_, _> = a.issues.iter().map(|i| (i.key.as_str(), i)).collect();
    let runs: HashMap<_, _> = a.verifications.iter().map(|v| (v.id.as_str(), v)).collect();
    if runs.len() != a.verifications.len() {
        return Err(err("invalid", "Duplicate verification IDs"));
    }
    for i in &a.issues {
        if i.parent
            .as_ref()
            .is_some_and(|k| !issues.contains_key(k.as_str()) || k == &i.key)
            || i.verification_key
                .as_ref()
                .is_some_and(|k| !issues.contains_key(k.as_str()) || k == &i.key)
            || i.current_run
                .as_ref()
                .is_some_and(|r| !runs.contains_key(r.as_str()))
        {
            return Err(err(
                "invalid",
                format!(
                    "Unresolved issue relationship on {}: its parent, verification child or current run does not exist",
                    i.key
                ),
            ));
        }
    }
    for i in &a.issues {
        let Some(parent_key) = &i.parent else {
            continue;
        };
        // A generated verification child.
        let parent = issues[parent_key.as_str()];
        if parent.parent.is_some() {
            return Err(inconsistent(format!(
                "{} names {parent_key} as its parent, but {parent_key} is itself a verification child",
                i.key
            )));
        }
        if parent.verification_key.as_deref() != Some(i.key.as_str()) {
            return Err(inconsistent(format!(
                "verification child {} names {parent_key} as its parent, but {parent_key}'s verification child is {}",
                i.key,
                parent.verification_key.as_deref().unwrap_or("missing")
            )));
        }
        if i.verification_key.is_some() {
            return Err(inconsistent(format!(
                "verification child {} cannot have its own verification child",
                i.key
            )));
        }
        if let Some(run) = &i.current_run {
            if runs[run.as_str()].issue_key != *parent_key {
                return Err(inconsistent(format!(
                    "verification child {}'s current run belongs to {}, not its parent {parent_key}",
                    i.key, runs[run.as_str()].issue_key
                )));
            }
        }
    }
    for v in &a.verifications {
        let Some(owner) = issues.get(v.issue_key.as_str()) else {
            return Err(err("invalid", "Orphan comment or verification"));
        };
        if owner.parent.is_some() {
            return Err(inconsistent(format!(
                "run {} is recorded against verification child {} instead of its parent",
                v.id, v.issue_key
            )));
        }
        if owner.verification_key.is_none() {
            return Err(inconsistent(format!(
                "{} has verification run history but no verification child",
                v.issue_key
            )));
        }
        // Review closes a run and reopen cancels a pending one, so an open
        // (pending) run is only ever its parent's current run.
        if v.outcome == Outcome::Pending && owner.current_run.as_ref() != Some(&v.id) {
            return Err(inconsistent(format!(
                "run {} for {} is still pending but is not its current run; a reopened or resubmitted run must be closed",
                v.id, v.issue_key
            )));
        }
    }
    for i in a.issues.iter().filter(|i| i.parent.is_none()) {
        let child = i.verification_key.as_deref().map(|k| issues[k]);
        if let Some(child) = child {
            if child.parent.as_ref() != Some(&i.key) {
                return Err(inconsistent(format!(
                    "{}'s verification child {} belongs to {}",
                    i.key,
                    child.key,
                    child.parent.as_deref().unwrap_or("no parent")
                )));
            }
            if child.product_id != i.product_id
                || child.project_id != i.project_id
                || child.milestone_id != i.milestone_id
            {
                return Err(inconsistent(format!(
                    "{}'s verification child {} has a different product, project or milestone",
                    i.key, child.key
                )));
            }
        }
        let Some(run_id) = &i.current_run else {
            if matches!(i.status, Status::Verify | Status::Done) {
                return Err(err(
                    "invalid",
                    format!("Missing required verification evidence for {}", i.key),
                ));
            }
            if let Some(child) = child.filter(|c| c.status != Status::Canceled) {
                return Err(inconsistent(format!(
                    "{} has no current run, but its verification child {} has status {:?} instead of Canceled",
                    i.key, child.key, child.status
                )));
            }
            continue;
        };
        let v = runs[run_id.as_str()];
        if v.issue_key != i.key {
            return Err(inconsistent(format!(
                "{}'s current run {run_id} belongs to {}",
                i.key, v.issue_key
            )));
        }
        let Some(child) = child else {
            return Err(inconsistent(format!(
                "{} has a current run but no verification child",
                i.key
            )));
        };
        if child.current_run.as_ref() != Some(run_id) {
            return Err(inconsistent(format!(
                "{}'s verification child {} records a different current run",
                i.key, child.key
            )));
        }
        let expected_parent = match v.outcome {
            Outcome::Pending => Some(Status::Verify),
            Outcome::Passed => Some(Status::Done),
            Outcome::Failed | Outcome::Canceled => None,
        };
        if expected_parent.as_ref().is_some_and(|s| *s != i.status)
            || (expected_parent.is_none() && matches!(i.status, Status::Verify | Status::Done))
        {
            return Err(err(
                "invalid",
                format!(
                    "Status disagrees with verification evidence: {} is {:?} but its current run is {:?}",
                    i.key, i.status, v.outcome
                ),
            ));
        }
        if i.status == Status::Done
            && (v.results.len() != v.steps.len()
                || v.results.iter().any(|r| r.outcome != Outcome::Passed))
        {
            return Err(err(
                "invalid",
                format!("Status disagrees with verification evidence: {} is Done without every step passing", i.key),
            ));
        }
        if child.status != child_status_for(&v.outcome) {
            return Err(inconsistent(format!(
                "verification child {} has status {:?} but {}'s current run is {:?}",
                child.key, child.status, i.key, v.outcome
            )));
        }
    }
    Ok(())
}

pub fn validate_archive(a: &Archive) -> Result<()> {
    validate_archive_structure(a)?;
    sources::validate_source_archive_bytes(a)
}

/// Every invariant except retained file bytes (used for merged previews).
pub(crate) fn validate_archive_structure(a: &Archive) -> Result<()> {
    if !matches!(a.format, 1..=ARCHIVE_FORMAT) {
        return Err(err("unsupported", "Unsupported archive format"));
    }
    for issue in &a.issues {
        if let Some(context) = &issue.intake {
            if a.format < 14 {
                return Err(err(
                    "invalid",
                    "Original intake context requires archive format 14",
                ));
            }
            context.validate().map_err(|e| err("invalid", e))?;
        }
    }
    Uuid::parse_str(&a.workspace_id).map_err(|_| err("invalid", "Invalid workspace identity"))?;
    let mut keys = HashSet::new();
    let mut ids = HashSet::new();
    for p in &a.products {
        if !keys.insert(p.key.clone()) || !ids.insert(p.id.clone()) {
            return Err(err("invalid", "Duplicate product"));
        }
    }
    if a.format < 15
        && (!a.product_sections.is_empty()
            || a.products
                .iter()
                .any(|p| p.sort_order != 0 || p.section_id.is_some()))
    {
        return Err(err(
            "invalid",
            "Sidebar arrangement requires archive format 15",
        ));
    }
    let mut section_ids = HashSet::new();
    for (index, section) in a.product_sections.iter().enumerate() {
        let name = product_section_name(&a.product_sections[..index], &section.name, None)
            .map_err(|_| err("invalid", "Invalid or duplicate sidebar section"))?;
        if Uuid::parse_str(&section.id).is_err()
            || !section_ids.insert(section.id.as_str())
            || section.version == 0
            || name != section.name
        {
            return Err(err("invalid", "Invalid or duplicate sidebar section"));
        }
    }
    if a.products.iter().any(|p| {
        p.section_id
            .as_deref()
            .is_some_and(|section| !section_ids.contains(section))
    }) {
        return Err(err("invalid", "Unresolved sidebar section reference"));
    }
    let product_ids = ids.clone();
    let mut document_ids = HashSet::new();
    for document in &a.theoria_documents {
        required(&document.title, "Theoria title")?;
        required(&document.source_root, "Theoria source root")?;
        required(&document.relative_path, "Theoria relative path")?;
        if !stable_id(&document.id)
            || !document_ids.insert(document.id.clone())
            || !product_ids.contains(&document.product_id)
            || document.catalog_version == 0
            || document.checked_at <= 0
            || document.relative_path.starts_with('/')
            || document.relative_path.starts_with('\\')
            || document
                .relative_path
                .split(['/', '\\'])
                .any(|part| part == "..")
            || ![
                "principles",
                "workflow",
                "playbook",
                "verification",
                "context",
                "governance",
            ]
            .contains(&document.category.as_str())
            || document
                .fingerprint
                .as_deref()
                .is_some_and(|value| !valid_fingerprint(value))
            || document.fingerprint.is_some() != document.content.is_some()
            || document.content.is_some() != document.cached_at.is_some()
            || document
                .cached_at
                .is_some_and(|cached| cached <= 0 || cached > document.checked_at)
            || (document.availability == TheoriaAvailability::Available
                && document.content.is_none())
            || (document.availability == TheoriaAvailability::Unavailable
                && document
                    .unavailable_reason
                    .as_deref()
                    .is_none_or(|reason| reason.trim().is_empty()))
        {
            return Err(err("invalid", "Invalid Theoria document cache record"));
        }
    }
    keys.clear();
    ids.clear();
    let mut issue_sources = HashSet::new();
    for i in &a.issues {
        if !product_ids.contains(&i.product_id)
            || !keys.insert(i.key.clone())
            || !ids.insert(i.id.clone())
            || i.version == 0
        {
            return Err(err("invalid", "Invalid or duplicate issue identity"));
        }
        if let Some(external) = &i.external {
            required(&external.source, "external issue source")?;
            required(&external.id, "external issue ID")?;
            required(&external.url, "external issue URL")?;
            required(&external.state.id, "external issue state ID")?;
            required(&external.state.name, "external issue state name")?;
            required(&external.state.kind, "external issue state type")?;
            required(&external.created_at, "external issue created timestamp")?;
            required(&external.updated_at, "external issue updated timestamp")?;
            if !issue_sources.insert((external.source.clone(), external.id.clone())) {
                return Err(err("invalid", "Duplicate external issue provenance"));
            }
        }
        if i.status == Status::LegacyCompleted
            && i.external.as_ref().is_none_or(|external| {
                external.state.kind != "completed" || external.completed_at.is_none()
            })
        {
            return Err(err(
                "invalid",
                "Legacy-completed issues require external completion provenance",
            ));
        }
        let p = a.products.iter().find(|p| p.id == i.product_id).unwrap();
        if !i
            .key
            .strip_prefix(&format!("{}-", p.key))
            .is_some_and(|n| n.parse::<u64>().is_ok_and(|v| v > 0))
        {
            return Err(err("invalid", "Issue key does not match its product"));
        }
        let mut linked = HashSet::new();
        for reference in &i.theoria_refs {
            let document = a
                .theoria_documents
                .iter()
                .find(|document| document.id == reference.document_id)
                .ok_or_else(|| err("invalid", "Unresolved Theoria guidance link"))?;
            if document.product_id != i.product_id
                || !linked.insert(reference.document_id.clone())
                || reference
                    .recorded_fingerprint
                    .as_deref()
                    .is_some_and(|value| !valid_fingerprint(value))
            {
                return Err(err("invalid", "Invalid Theoria guidance link"));
            }
        }
    }
    if a.format == 1 && (!a.projects.is_empty() || a.issues.iter().any(|i| i.project_id.is_some()))
    {
        return Err(err("invalid", "Project data requires archive format 2"));
    }
    if a.format < 3
        && (!a.theoria_documents.is_empty()
            || !a.method_findings.is_empty()
            || a.issues.iter().any(|issue| !issue.theoria_refs.is_empty()))
    {
        return Err(err("invalid", "Theoria data requires archive format 3"));
    }
    if a.format < 4 && !a.git_traces.is_empty() {
        return Err(err("invalid", "Git trace data requires archive format 4"));
    }
    if a.format < 5
        && (a.projects.iter().any(|project| {
            project.status != ProjectStatus::Active
                || project.priority != "medium"
                || project.sort_order != 0
        }) || a
            .issues
            .iter()
            .any(|issue| issue.planning_scope != PlanningScope::Inbox))
    {
        return Err(err(
            "invalid",
            "Project planning data requires archive format 5",
        ));
    }
    if a.format < 6 && !a.issue_links.is_empty() {
        return Err(err("invalid", "Issue links require archive format 6"));
    }
    if a.format < 7
        && (!a.goals.is_empty()
            || !a.milestones.is_empty()
            || a.issues.iter().any(|issue| issue.milestone_id.is_some()))
    {
        return Err(err(
            "invalid",
            "Goal and milestone data requires archive format 7",
        ));
    }
    if a.format < 8
        && (a.projects.iter().any(|project| {
            project.external_source.is_some()
                || project.external_id.is_some()
                || project.external_url.is_some()
        }) || a.goals.iter().any(|goal| goal.external_url.is_some())
            || a.milestones
                .iter()
                .any(|milestone| milestone.external_url.is_some())
            || a.issues
                .iter()
                .any(|issue| issue.external.is_some() || issue.status == Status::LegacyCompleted)
            || a.comments.iter().any(|comment| {
                comment.external_source.is_some()
                    || comment.external_id.is_some()
                    || comment.external_url.is_some()
            }))
    {
        return Err(err(
            "invalid",
            "External migration provenance requires archive format 8",
        ));
    }
    if a.format < 9 && (!a.releases.is_empty() || !a.release_evidence.is_empty()) {
        return Err(err("invalid", "Release data requires archive format 9"));
    }
    if a.format < 10 && !a.release_workflows.is_empty() {
        return Err(err(
            "invalid",
            "Release workflow data requires archive format 10",
        ));
    }
    if a.format < 11
        && (!a.labels.is_empty()
            || a.issues.iter().any(|issue| !issue.labels.is_empty())
            || a.projects.iter().any(|project| !project.labels.is_empty()))
    {
        return Err(err("invalid", "Label data requires archive format 11"));
    }
    templates::validate_archive(a)?;
    signals::validate_archive(a)?;
    context::validate_archive(a)?;
    let mut label_ids = HashSet::new();
    for (index, label) in a.labels.iter().enumerate() {
        let fields = validate_label_fields(
            &label.name,
            &label.description,
            &label.color,
            &label.aliases,
            &label.products,
            &label.linear_origins,
        )?;
        if Uuid::parse_str(&label.id).is_err()
            || !label_ids.insert(label.id.clone())
            || label.version == 0
            || fields.name != label.name
            || fields.aliases != label.aliases
            || fields.linear_origins != label.linear_origins
            || fields
                .products
                .iter()
                .any(|rule| !product_ids.contains(&rule.product_id))
        {
            return Err(err("invalid", "Invalid or duplicate label"));
        }
        label_taxonomy_conflicts(
            &a.labels[..index],
            &label.name,
            &label.aliases,
            &label.linear_origins,
            None,
        )?;
    }
    for i in &a.issues {
        let mut attached = HashSet::new();
        for id in &i.labels {
            let label = a
                .labels
                .iter()
                .find(|label| label.id == *id)
                .ok_or_else(|| err("invalid", "Unresolved label reference"))?;
            if !attached.insert(id.clone()) || !label_applies(label, &i.product_id) {
                return Err(err("invalid", "Invalid label assignment"));
            }
        }
    }
    let mut project_ids = HashSet::new();
    let mut project_names = HashSet::new();
    let mut project_sources = HashSet::new();
    for p in &a.projects {
        let mut attached = HashSet::new();
        for id in &p.labels {
            let label = a
                .labels
                .iter()
                .find(|label| label.id == *id)
                .ok_or_else(|| err("invalid", "Unresolved label reference"))?;
            if !attached.insert(id.clone()) || !label_applies(label, &p.product_id) {
                return Err(err("invalid", "Invalid label assignment"));
            }
        }
        required(&p.id, "project ID")?;
        validate_project_fields(&p.name, &p.priority, p.sort_order)?;
        validate_optional_provenance(p.external_source.as_deref(), p.external_id.as_deref())?;
        if p.version == 0
            || p.name.len() > 160
            || !a.products.iter().any(|product| product.id == p.product_id)
            || !project_ids.insert(p.id.clone())
            || !project_names.insert((p.product_id.clone(), p.name.trim().to_lowercase()))
            || p.external_source.as_ref().is_some_and(|source| {
                !project_sources.insert((source.clone(), p.external_id.clone().unwrap()))
            })
            || p.external_url
                .as_deref()
                .is_some_and(|url| url.trim().is_empty())
        {
            return Err(err("invalid", "Invalid or duplicate project"));
        }
    }
    let mut goal_ids = HashSet::new();
    let mut goal_names = HashSet::new();
    let mut goal_sources = HashSet::new();
    for goal in &a.goals {
        validate_goal_fields(&goal.name, &goal.priority)?;
        validate_optional_provenance(goal.external_source.as_deref(), goal.external_id.as_deref())?;
        let mut linked_projects = HashSet::new();
        if goal.version == 0
            || !product_ids.contains(&goal.product_id)
            || !goal_ids.insert(goal.id.clone())
            || !goal_names.insert((goal.product_id.clone(), goal.name.trim().to_lowercase()))
            || goal.external_source.as_ref().is_some_and(|source| {
                !goal_sources.insert((source.clone(), goal.external_id.clone().unwrap()))
            })
            || goal
                .external_url
                .as_deref()
                .is_some_and(|url| url.trim().is_empty())
            || goal.project_ids.iter().any(|id| {
                !linked_projects.insert(id)
                    || !a
                        .projects
                        .iter()
                        .any(|project| project.id == *id && project.product_id == goal.product_id)
            })
        {
            return Err(err("invalid", "Invalid or duplicate goal"));
        }
    }
    let mut milestone_ids = HashSet::new();
    let mut milestone_names = HashSet::new();
    let mut milestone_sources = HashSet::new();
    for milestone in &a.milestones {
        validate_milestone_fields(&milestone.name, milestone.sort_order)?;
        validate_optional_provenance(
            milestone.external_source.as_deref(),
            milestone.external_id.as_deref(),
        )?;
        if milestone.version == 0
            || !project_ids.contains(&milestone.project_id)
            || !milestone_ids.insert(milestone.id.clone())
            || !milestone_names.insert((
                milestone.project_id.clone(),
                milestone.name.trim().to_lowercase(),
            ))
            || milestone.external_source.as_ref().is_some_and(|source| {
                !milestone_sources.insert((source.clone(), milestone.external_id.clone().unwrap()))
            })
            || milestone
                .external_url
                .as_deref()
                .is_some_and(|url| url.trim().is_empty())
        {
            return Err(err("invalid", "Invalid or duplicate milestone"));
        }
    }
    let mut workflow_products = HashSet::new();
    for workflow in &a.release_workflows {
        validate_release_workflow_fields(
            &workflow.branch_strategy,
            &workflow.production_ref,
            &workflow.release_branch_pattern,
            &workflow.preview_environment,
            &workflow.preview_url_template,
        )?;
        if workflow.version == 0
            || !product_ids.contains(&workflow.product_id)
            || !workflow_products.insert(workflow.product_id.clone())
        {
            return Err(err("invalid", "Invalid or duplicate release workflow"));
        }
    }
    let mut release_ids = HashSet::new();
    let mut release_versions = HashSet::new();
    let mut release_sources = HashSet::new();
    let mut active_release_branches = HashSet::new();
    for release in &a.releases {
        validate_release_fields(
            &release.name,
            &release.version_label,
            &release.target_ref,
            release.preview_url.as_deref(),
            &release.notes,
        )?;
        validate_optional_provenance(
            release.external_source.as_deref(),
            release.external_id.as_deref(),
        )?;
        let mut linked_projects = HashSet::new();
        let mut linked_issues = HashSet::new();
        if Uuid::parse_str(&release.id).is_err()
            || release.version == 0
            || !product_ids.contains(&release.product_id)
            || !release_ids.insert(release.id.clone())
            || !release_versions.insert((
                release.product_id.clone(),
                release.version_label.trim().to_lowercase(),
            ))
            || release.external_source.as_ref().is_some_and(|source| {
                !release_sources.insert((source.clone(), release.external_id.clone().unwrap()))
            })
            || release
                .external_url
                .as_deref()
                .is_some_and(|url| url.trim().is_empty())
            || release.release_branch.as_deref().is_some_and(|branch| {
                branch.trim().is_empty()
                    || (!matches!(
                        release.status,
                        ReleaseStatus::Retired | ReleaseStatus::Canceled
                    ) && !active_release_branches
                        .insert((release.product_id.clone(), branch.to_string())))
            })
            || release.project_ids.iter().any(|id| {
                !linked_projects.insert(id)
                    || !a.projects.iter().any(|project| {
                        project.id == *id && project.product_id == release.product_id
                    })
            })
            || release.issue_keys.iter().any(|key| {
                !linked_issues.insert(key)
                    || !a.issues.iter().any(|issue| {
                        issue.key == *key
                            && issue.product_id == release.product_id
                            && issue.parent.is_none()
                    })
            })
        {
            return Err(err(
                "invalid",
                "Invalid, duplicate, or cross-product release",
            ));
        }
        if let Some(workflow) = a
            .release_workflows
            .iter()
            .find(|workflow| workflow.product_id == release.product_id)
        {
            let expected_branch = workflow
                .release_branch_pattern
                .replace("{version}", &release.version_label);
            if release.target_ref != workflow.production_ref
                || (workflow.branch_strategy == ReleaseBranchStrategy::OneBranchPerRelease
                    && release.release_branch.as_deref() != Some(expected_branch.as_str()))
            {
                return Err(err(
                    "invalid",
                    "Release disagrees with its product workflow",
                ));
            }
        }
    }
    for i in &a.issues {
        if let Some(id) = &i.project_id {
            if !a
                .projects
                .iter()
                .any(|p| p.id == *id && p.product_id == i.product_id)
            {
                return Err(err("invalid", "Unresolved or cross-product project link"));
            }
        }
        if let Some(id) = &i.milestone_id {
            if !a.milestones.iter().any(|milestone| {
                milestone.id == *id && Some(&milestone.project_id) == i.project_id.as_ref()
            }) {
                return Err(err("invalid", "Unresolved or cross-project milestone link"));
            }
        }
        if i.planning_scope == PlanningScope::Project
            && i.project_id.is_none()
            && !matches!(
                i.status,
                Status::Backlog | Status::LegacyCompleted | Status::Canceled
            )
        {
            return Err(err(
                "invalid",
                "Project-scoped active work is missing its project",
            ));
        }
    }
    let issue_products: HashMap<_, _> = a
        .issues
        .iter()
        .map(|issue| (issue.key.as_str(), issue.product_id.as_str()))
        .collect();
    let generated_children: HashSet<_> = a
        .issues
        .iter()
        .filter(|issue| issue.parent.is_some())
        .map(|issue| issue.key.as_str())
        .collect();
    let mut link_ids = HashSet::new();
    let mut parent_sources = HashSet::new();
    for (index, link) in a.issue_links.iter().enumerate() {
        validate_external_provenance(
            &link.kind,
            link.external_source.as_deref(),
            link.external_id.as_deref(),
        )?;
        let source_product = issue_products.get(link.source_key.as_str());
        let target_product = issue_products.get(link.target_key.as_str());
        if Uuid::parse_str(&link.id).is_err()
            || !link_ids.insert(link.id.clone())
            || link.source_key == link.target_key
            || source_product.is_none()
            || target_product.is_none()
            || source_product != target_product
            || generated_children.contains(link.source_key.as_str())
            || generated_children.contains(link.target_key.as_str())
            || link.created_by.trim().is_empty()
            || link.created_at <= 0
            || a.issue_links[..index].iter().any(|prior| {
                same_logical_link(prior, &link.source_key, &link.target_key, &link.kind)
            })
            || (link.kind == IssueLinkKind::Parent
                && !parent_sources.insert(link.source_key.as_str()))
            || would_create_cycle(
                &a.issue_links,
                &link.source_key,
                &link.target_key,
                &link.kind,
            )
        {
            return Err(err("invalid", "Invalid, duplicate, or cyclic issue link"));
        }
    }
    for verification in &a.verifications {
        if let Some(evidence) = &verification.e2e {
            if a.format < 11 {
                return Err(err("invalid", "E2E evidence requires archive format 11"));
            }
            validate_e2e(evidence, &verification.build_ref)?;
        }
    }
    validate_verification_relationships(a)?;
    let mut comment_ids = HashSet::new();
    let mut comment_sources = HashSet::new();
    for comment in &a.comments {
        validate_optional_provenance(
            comment.external_source.as_deref(),
            comment.external_id.as_deref(),
        )?;
        if !comment_ids.insert(comment.id.clone())
            || !keys.contains(&comment.issue_key)
            || comment.external_source.as_ref().is_some_and(|source| {
                !comment_sources.insert((source.clone(), comment.external_id.clone().unwrap()))
            })
            || comment
                .external_url
                .as_deref()
                .is_some_and(|url| url.trim().is_empty())
        {
            return Err(err("invalid", "Invalid or duplicate comment"));
        }
    }
    if a.verifications
        .iter()
        .any(|v| !keys.contains(&v.issue_key) || v.steps.is_empty())
    {
        return Err(err("invalid", "Orphan comment or verification"));
    }
    let mut trace_ids = HashSet::new();
    let mut logical_traces = HashSet::new();
    for trace in &a.git_traces {
        required(&trace.recorded_by, "Git trace actor")?;
        validate_git_trace_fields(
            &trace.kind,
            &trace.repository,
            &trace.commit_sha,
            &trace.branch,
            trace.remote.as_deref(),
            trace.remote_ref.as_deref(),
        )?;
        let logical = (
            trace.issue_key.clone(),
            trace.kind.clone(),
            trace.repository.clone(),
            trace.commit_sha.to_ascii_lowercase(),
            trace.branch.clone(),
            trace.remote.clone(),
            trace.remote_ref.clone(),
        );
        if Uuid::parse_str(&trace.id).is_err()
            || !trace_ids.insert(trace.id.clone())
            || !logical_traces.insert(logical)
            || !keys.contains(&trace.issue_key)
            || trace.recorded_at <= 0
        {
            return Err(err("invalid", "Invalid or duplicate Git trace"));
        }
    }
    let traces: HashMap<_, _> = a
        .git_traces
        .iter()
        .map(|trace| (trace.id.as_str(), trace))
        .collect();
    let runs: HashMap<_, _> = a
        .verifications
        .iter()
        .map(|run| (run.id.as_str(), run))
        .collect();
    let mut evidence_ids = HashSet::new();
    let mut logical_evidence = HashSet::new();
    let mut active_commit_owners: HashMap<String, String> = HashMap::new();
    for evidence in &a.release_evidence {
        let release = a
            .releases
            .iter()
            .find(|release| release.id == evidence.release_id)
            .ok_or_else(|| err("invalid", "Release evidence has no release"))?;
        let linked_issue_keys: HashSet<_> = release_issues(release, &a.issues)
            .into_iter()
            .map(|issue| issue.key.as_str())
            .collect();
        let valid = match evidence.kind {
            ReleaseEvidenceKind::Commit | ReleaseEvidenceKind::Push => {
                evidence.verification_id.is_none()
                    && evidence.deployment_ref.is_none()
                    && evidence.commit_sha.is_none()
                    && evidence.target_ref.is_none()
                    && evidence.source_ref.is_none()
                    && evidence.environment.is_none()
                    && evidence.url.is_none()
                    && evidence.approver.is_none()
                    && evidence.outcome == Outcome::Passed
                    && evidence.git_trace_id.as_deref().is_some_and(|id| {
                        traces.get(id).is_some_and(|trace| {
                            evidence.issue_key.as_deref() == Some(trace.issue_key.as_str())
                                && linked_issue_keys.contains(trace.issue_key.as_str())
                                && ((evidence.kind == ReleaseEvidenceKind::Commit
                                    && trace.kind == GitTraceKind::Commit)
                                    || (evidence.kind == ReleaseEvidenceKind::Push
                                        && trace.kind == GitTraceKind::Push))
                                && release
                                    .release_branch
                                    .as_deref()
                                    .is_none_or(|branch| branch == trace.branch)
                        })
                    })
            }
            ReleaseEvidenceKind::Check => {
                evidence.git_trace_id.is_none()
                    && evidence.deployment_ref.is_none()
                    && evidence.commit_sha.is_none()
                    && evidence.target_ref.is_none()
                    && evidence.source_ref.is_none()
                    && evidence.environment.is_none()
                    && evidence.url.is_none()
                    && evidence.approver.is_none()
                    && evidence.outcome == Outcome::Passed
                    && evidence.verification_id.as_deref().is_some_and(|id| {
                        runs.get(id).is_some_and(|run| {
                            evidence.issue_key.as_deref() == Some(run.issue_key.as_str())
                                && linked_issue_keys.contains(run.issue_key.as_str())
                                && run.outcome == Outcome::Passed
                                && run.results.len() == run.steps.len()
                                && run
                                    .results
                                    .iter()
                                    .all(|result| result.outcome == Outcome::Passed)
                        })
                    })
            }
            ReleaseEvidenceKind::PreviewDeployment
            | ReleaseEvidenceKind::ProductionDeployment
            | ReleaseEvidenceKind::Rollback => {
                let expected_ref = if evidence.kind == ReleaseEvidenceKind::PreviewDeployment {
                    release
                        .release_branch
                        .as_deref()
                        .unwrap_or(&release.target_ref)
                } else {
                    &release.target_ref
                };
                evidence.issue_key.is_none()
                    && evidence.git_trace_id.is_none()
                    && evidence.verification_id.is_none()
                    && evidence
                        .deployment_ref
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty())
                    && evidence.commit_sha.as_deref().is_some_and(valid_commit_sha)
                    && (evidence.kind == ReleaseEvidenceKind::Rollback
                        || evidence.target_ref.as_deref() == Some(expected_ref))
                    && (a.format < 10
                        || evidence
                            .environment
                            .as_deref()
                            .is_some_and(|value| !value.trim().is_empty()))
                    && evidence
                        .url
                        .as_deref()
                        .is_none_or(|value| !value.trim().is_empty())
                    && (evidence.kind != ReleaseEvidenceKind::Rollback
                        || evidence
                            .source_ref
                            .as_deref()
                            .is_some_and(|value| !value.trim().is_empty()))
                    && (evidence.kind == ReleaseEvidenceKind::Rollback
                        || evidence.source_ref.is_none())
                    && (evidence.kind != ReleaseEvidenceKind::ProductionDeployment
                        || evidence.outcome != Outcome::Passed
                        || evidence
                            .approver
                            .as_deref()
                            .is_some_and(|value| !value.trim().is_empty()))
                    && (evidence.kind == ReleaseEvidenceKind::ProductionDeployment
                        && evidence.outcome == Outcome::Passed
                        || evidence.approver.is_none())
                    && (evidence.outcome == Outcome::Passed || !evidence.note.trim().is_empty())
                    && (evidence.outcome != Outcome::Pending)
                    && (evidence.kind != ReleaseEvidenceKind::PreviewDeployment
                        || evidence.approver.is_none())
            }
        };
        let logical = (
            evidence.release_id.clone(),
            evidence.kind.clone(),
            evidence.git_trace_id.clone(),
            evidence.verification_id.clone(),
            evidence.deployment_ref.clone(),
        );
        if Uuid::parse_str(&evidence.id).is_err()
            || !evidence_ids.insert(evidence.id.clone())
            || !logical_evidence.insert(logical)
            || evidence.recorded_by.trim().is_empty()
            || evidence.recorded_at <= 0
            || !valid
        {
            return Err(err("invalid", "Invalid or duplicate release evidence"));
        }
        if !matches!(
            release.status,
            ReleaseStatus::Retired | ReleaseStatus::Canceled
        ) && evidence.git_trace_id.as_deref().is_some_and(|id| {
            traces.get(id).is_some_and(|trace| {
                active_commit_owners
                    .insert(trace.commit_sha.clone(), release.id.clone())
                    .is_some_and(|owner| owner != release.id)
            })
        }) {
            return Err(err(
                "invalid",
                "A commit belongs to more than one active release",
            ));
        }
    }
    for release in &a.releases {
        let evidence: Vec<_> = a
            .release_evidence
            .iter()
            .filter(|evidence| evidence.release_id == release.id)
            .collect();
        if release.status == ReleaseStatus::Preview
            && !evidence.iter().any(|evidence| {
                evidence.kind == ReleaseEvidenceKind::PreviewDeployment
                    && evidence.outcome == Outcome::Passed
            })
        {
            return Err(err(
                "invalid",
                "Preview release is missing preview deployment evidence",
            ));
        }
        if release.status == ReleaseStatus::Production {
            let production = evidence
                .iter()
                .rev()
                .find(|evidence| {
                    evidence.kind == ReleaseEvidenceKind::ProductionDeployment
                        && evidence.outcome == Outcome::Passed
                })
                .ok_or_else(|| {
                    err(
                        "invalid",
                        "Production release is missing deployment evidence",
                    )
                })?;
            let commit = production.commit_sha.as_deref().unwrap();
            let has_preview = evidence.iter().any(|evidence| {
                evidence.kind == ReleaseEvidenceKind::PreviewDeployment
                    && evidence.outcome == Outcome::Passed
                    && evidence.commit_sha.as_deref() == Some(commit)
            });
            let has_push = evidence.iter().any(|evidence| {
                evidence.kind == ReleaseEvidenceKind::Push
                    && evidence.outcome == Outcome::Passed
                    && evidence.git_trace_id.as_deref().is_some_and(|id| {
                        traces
                            .get(id)
                            .is_some_and(|trace| trace.commit_sha == commit)
                    })
            });
            if !has_preview || !has_push || release_issues(release, &a.issues).is_empty() {
                return Err(err(
                    "invalid",
                    "Production release lacks matching push, preview, or verified work",
                ));
            }
        }
    }
    let mut finding_ids = HashSet::new();
    for finding in &a.method_findings {
        required(&finding.observation, "observed fact")?;
        required(&finding.proposal, "proposed improvement")?;
        if Uuid::parse_str(&finding.id).is_err()
            || !finding_ids.insert(finding.id.clone())
            || !keys.contains(&finding.issue_key)
            || finding.evidence.is_empty()
            || finding.evidence.len() > 10
        {
            return Err(err("invalid", "Invalid method finding"));
        }
        for pointer in &finding.evidence {
            required(&pointer.reference, "evidence reference")?;
        }
    }
    let mut seq = 0;
    for e in &a.events {
        if e.seq <= seq {
            return Err(err("invalid", "Event cursors must increase"));
        }
        seq = e.seq;
    }
    // Duplicate row IDs and malformed replay JSON are rejected before any destination is replaced.
    for unique in [
        a.comments.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        a.requests.iter().map(|r| r.id.as_str()).collect(),
    ] {
        if unique.iter().collect::<HashSet<_>>().len() != unique.len() {
            return Err(err("invalid", "Duplicate archive row"));
        }
    }
    for r in &a.requests {
        let _: Value = serde_json::from_str(&r.response)?;
    }
    sources::validate_source_metadata(a)
}
