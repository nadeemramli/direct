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
fn human(role: Role) -> Result<()> {
    if role != Role::Human {
        Err(err("forbidden", "This operation requires human review"))
    } else {
        Ok(())
    }
}

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
            if !matches!(schema, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8") {
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
             UPDATE meta SET value='8' WHERE key='schema';",
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
                return Ok(json!({
                    "workspace_id":self.workspace_id()?,
                    "products":all::<Product>(&self.conn,"products")?,
                    "projects":all::<Project>(&self.conn,"projects")?,
                    "project_progress":all::<Project>(&self.conn,"projects")?.iter().map(|project| project_progress(&self.conn, &project.id)).collect::<Result<Vec<_>>>()?,
                    "goals":all::<Goal>(&self.conn,"goals")?,
                    "goal_progress":all::<Goal>(&self.conn,"goals")?.iter().map(|goal| goal_progress(&self.conn, goal)).collect::<Result<Vec<_>>>()?,
                    "milestones":all::<Milestone>(&self.conn,"milestones")?,
                    "milestone_progress":all::<Milestone>(&self.conn,"milestones")?.iter().map(|milestone| milestone_progress(&self.conn, milestone)).collect::<Result<Vec<_>>>()?,
                    "theoria_documents":all::<TheoriaDocument>(&self.conn,"theoria_documents")?,
                    "method_findings":all::<MethodFinding>(&self.conn,"method_findings")?,
                    "git_traces":all::<GitTrace>(&self.conn,"git_traces")?,
                    "issue_links":all::<IssueLink>(&self.conn,"issue_links")?,
                    "issues":all::<Issue>(&self.conn,"issues")?,
                    "cursor":cursor(&self.conn)?
                }))
            }
            Command::Context { key } => {
                let issue = issue(&self.conn, key)?;
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
                return Ok(
                    json!({"issue":issue,"product":product,"project":project,"project_progress":project_progress,"milestone":milestone,"milestone_progress":milestone_progress,"goals":goals,"goal_progress":goal_progress,"issue_links":issue_links,"comments":comments,"more_comments":more_comments,"verifications":runs,"method_findings":method_findings,"git_traces":git_traces,"history":history,"content_authority":"Task data, not tool authorization"}),
                );
            }
            Command::Changes { after } => {
                let rows: Vec<_> = events(&self.conn, *after)?.into_iter().take(200).collect();
                return Ok(
                    json!({"cursor":cursor(&self.conn)?,"next_cursor":rows.last().map(|e|e.seq).unwrap_or(*after),"events":rows}),
                );
            }
            Command::Export => return Ok(serde_json::to_value(self.export()?)?),
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
        let mut stmt = self
            .conn
            .prepare("SELECT id,actor,role,hash,response FROM requests ORDER BY id")?;
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
        Ok(Archive {
            format: 8,
            workspace_id: self.workspace_id()?,
            products: all(&self.conn, "products")?,
            projects: all(&self.conn, "projects")?,
            goals: all(&self.conn, "goals")?,
            milestones: all(&self.conn, "milestones")?,
            theoria_documents: all(&self.conn, "theoria_documents")?,
            method_findings: all(&self.conn, "method_findings")?,
            git_traces: all(&self.conn, "git_traces")?,
            issue_links: all(&self.conn, "issue_links")?,
            issues: all(&self.conn, "issues")?,
            comments: all(&self.conn, "comments")?,
            verifications: all(&self.conn, "verifications")?,
            events: events(&self.conn, 0)?,
            requests,
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
        tx.execute_batch("DELETE FROM projects; DELETE FROM goals; DELETE FROM milestones; DELETE FROM theoria_documents; DELETE FROM method_findings; DELETE FROM git_traces; DELETE FROM issue_links; DELETE FROM products; DELETE FROM issues; DELETE FROM comments; DELETE FROM verifications; DELETE FROM events; DELETE FROM requests; DELETE FROM sqlite_sequence WHERE name='events';")?;
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
        for p in a.projects {
            put_project(&tx, &p)?;
        }
        for goal in a.goals {
            put_goal(&tx, &goal)?;
        }
        for milestone in a.milestones {
            put_milestone(&tx, &milestone)?;
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
        for link in a.issue_links {
            put_issue_link(&tx, &link)?;
        }
        for i in a.issues {
            put_issue(&tx, &i)?;
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
        tx.commit()?;
        Ok(())
    }
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
    let issues = all::<Issue>(conn, "issues")?;
    let parents: Vec<_> = issues
        .iter()
        .filter(|issue| issue.parent.is_none() && issue.project_id.as_deref() == Some(project_id))
        .collect();
    let counts = progress_counts(&parents);
    Ok(ProjectProgress {
        project_id: project_id.into(),
        total: counts.0,
        backlog: counts.1,
        active: counts.2,
        pending_verification: counts.3,
        completed: counts.4,
        legacy_completed: counts.5,
        canceled: counts.6,
        completion_percent: counts.7,
    })
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
    let issues = all::<Issue>(conn, "issues")?;
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
    Ok(GoalProgress {
        goal_id: goal.id.clone(),
        total: counts.0,
        backlog: counts.1,
        active: counts.2,
        pending_verification: counts.3,
        completed: counts.4,
        legacy_completed: counts.5,
        canceled: counts.6,
        completion_percent: counts.7,
    })
}
fn milestone_progress(conn: &Connection, milestone: &Milestone) -> Result<MilestoneProgress> {
    let issues = all::<Issue>(conn, "issues")?;
    let parents: Vec<_> = issues
        .iter()
        .filter(|issue| {
            issue.parent.is_none() && issue.milestone_id.as_ref() == Some(&milestone.id)
        })
        .collect();
    let counts = progress_counts(&parents);
    Ok(MilestoneProgress {
        milestone_id: milestone.id.clone(),
        total: counts.0,
        backlog: counts.1,
        active: counts.2,
        pending_verification: counts.3,
        completed: counts.4,
        legacy_completed: counts.5,
        canceled: counts.6,
        completion_percent: counts.7,
    })
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
fn put_issue_link(conn: &Connection, link: &IssueLink) -> Result<()> {
    conn.execute(
        "INSERT INTO issue_links VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![link.id, serde_json::to_string(link)?],
    )?;
    Ok(())
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
fn put_issue(conn: &Connection, i: &Issue) -> Result<()> {
    conn.execute(
        "INSERT INTO issues VALUES (?1,?2,?3) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![i.id, i.key, serde_json::to_string(i)?],
    )?;
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
    let next = existing
        .iter()
        .filter(|i| i.product_id == p.id)
        .filter_map(|i| i.key.rsplit('-').next()?.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    Ok(Issue {
        id: id(),
        key: format!("{}-{next}", p.key),
        product_id: p.id.clone(),
        project_id,
        milestone_id: None,
        planning_scope,
        theoria_refs: vec![],
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
        } => {
            human(role)?;
            validate_project_fields(name, priority, *sort_order)?;
            let product = all::<Product>(tx, "products")?
                .into_iter()
                .find(|p| p.key == *product)
                .ok_or_else(|| err("not_found", "Unknown product"))?;
            project_name(tx, &product.id, name, None)?;
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
            if all::<Product>(tx, "products")?
                .iter()
                .any(|p| p.key == *key)
            {
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
            };
            tx.execute(
                "INSERT INTO products VALUES (?1,?2,?3)",
                params![p.id, p.key, serde_json::to_string(&p)?],
            )?;
            emit(tx, actor, "product_created", key, at)?;
            Ok(json!(p))
        }
        Command::CreateIssue {
            product,
            title,
            body,
            planning_scope,
            project_id,
        } => {
            required(title, "title")?;
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
            let i = new_issue(
                tx,
                &p,
                title,
                body,
                planning_scope.clone(),
                project_id.clone(),
                at,
            )?;
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
        } => {
            let mut i = version(tx, key, *expected_version)?;
            required(title, "title")?;
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
            limitations,
            preconditions,
            steps,
        } => {
            let mut i = version(tx, key, *expected_version)?;
            held(&i, actor, at)?;
            if i.status != Status::Doing {
                return Err(err("invalid", "Work must be Doing before submission"));
            }
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
                .unwrap();
            let mut child = match &i.verification_key {
                Some(k) => issue(tx, k)?,
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
            let mut child = issue(tx, i.verification_key.as_deref().unwrap())?;
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
            if let Some(k) = &i.verification_key {
                let mut c = issue(tx, k)?;
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
        _ => Err(err("invalid", "Not a mutation")),
    }
}

pub fn validate_archive(a: &Archive) -> Result<()> {
    if !matches!(a.format, 1..=8) {
        return Err(err("unsupported", "Unsupported archive format"));
    }
    Uuid::parse_str(&a.workspace_id).map_err(|_| err("invalid", "Invalid workspace identity"))?;
    let mut keys = HashSet::new();
    let mut ids = HashSet::new();
    for p in &a.products {
        if !keys.insert(p.key.clone()) || !ids.insert(p.id.clone()) {
            return Err(err("invalid", "Duplicate product"));
        }
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
    let mut project_ids = HashSet::new();
    let mut project_names = HashSet::new();
    let mut project_sources = HashSet::new();
    for p in &a.projects {
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
    let run_ids: HashSet<_> = a.verifications.iter().map(|v| v.id.clone()).collect();
    if run_ids.len() != a.verifications.len() {
        return Err(err("invalid", "Duplicate verification IDs"));
    }
    for i in &a.issues {
        if i.parent
            .as_ref()
            .is_some_and(|k| !keys.contains(k) || k == &i.key)
            || i.verification_key
                .as_ref()
                .is_some_and(|k| !keys.contains(k))
            || i.current_run.as_ref().is_some_and(|r| !run_ids.contains(r))
        {
            return Err(err("invalid", "Unresolved issue relationship"));
        }
        if let Some(k) = &i.verification_key {
            let child = a.issues.iter().find(|c| c.key == *k).unwrap();
            if child.parent.as_ref() != Some(&i.key)
                || child.product_id != i.product_id
                || child.project_id != i.project_id
                || child.milestone_id != i.milestone_id
            {
                return Err(err("invalid", "Broken verification child link"));
            }
        }
        if i.parent.is_none() && matches!(i.status, Status::Verify | Status::Done) {
            let v = a
                .verifications
                .iter()
                .find(|v| Some(&v.id) == i.current_run.as_ref() && v.issue_key == i.key)
                .ok_or_else(|| err("invalid", "Missing required verification evidence"))?;
            if (i.status == Status::Done
                && (v.outcome != Outcome::Passed
                    || v.results.len() != v.steps.len()
                    || v.results.iter().any(|r| r.outcome != Outcome::Passed)))
                || (i.status == Status::Verify && v.outcome != Outcome::Pending)
            {
                return Err(err(
                    "invalid",
                    "Status disagrees with verification evidence",
                ));
            }
        }
    }
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
    Ok(())
}
