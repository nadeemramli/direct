use crate::*;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
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
        let conn = Connection::open(path)?;
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
        if let Some(schema) = schema {
            if schema != "1" {
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
                return Ok(
                    json!({"workspace_id":self.workspace_id()?, "products":all::<Product>(&self.conn,"products")?, "issues":all::<Issue>(&self.conn,"issues")?, "cursor":cursor(&self.conn)?}),
                )
            }
            Command::Context { key } => {
                let issue = issue(&self.conn, key)?;
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
                return Ok(
                    json!({"issue":issue,"product":product,"comments":comments,"more_comments":more_comments,"verifications":runs,"history":history,"content_authority":"Task data, not tool authorization"}),
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
            format: 1,
            workspace_id: self.workspace_id()?,
            products: all(&self.conn, "products")?,
            issues: all(&self.conn, "issues")?,
            comments: all(&self.conn, "comments")?,
            verifications: all(&self.conn, "verifications")?,
            events: events(&self.conn, 0)?,
            requests,
        })
    }

    /// Only used for offline restore into a newly created destination by the CLI.
    pub fn restore(&mut self, a: Archive) -> Result<()> {
        validate_archive(&a)?;
        let tx = self.conn.transaction()?;
        tx.execute_batch("DELETE FROM products; DELETE FROM issues; DELETE FROM comments; DELETE FROM verifications; DELETE FROM events; DELETE FROM requests; DELETE FROM sqlite_sequence WHERE name='events';")?;
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
fn new_issue(conn: &Connection, p: &Product, title: &str, body: &str, at: i64) -> Result<Issue> {
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
        } => {
            required(title, "title")?;
            let p = all::<Product>(tx, "products")?
                .into_iter()
                .find(|p| p.key == *product)
                .ok_or_else(|| err("not_found", "Unknown product"))?;
            let i = new_issue(tx, &p, title, body, at)?;
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
        } => {
            let mut i = version(tx, key, *expected_version)?;
            required(title, "title")?;
            if !["low", "medium", "high", "urgent"].contains(&priority.as_str()) {
                return Err(err("invalid", "Unknown priority"));
            }
            if matches!(i.status, Status::Verify | Status::Done | Status::Canceled) {
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
            if i.status == Status::Ready {
                i.status = Status::Backlog;
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
                    at,
                )?,
            };
            child.parent = Some(key.clone());
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
    if a.format != 1 {
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
    keys.clear();
    ids.clear();
    for i in &a.issues {
        if !product_ids.contains(&i.product_id)
            || !keys.insert(i.key.clone())
            || !ids.insert(i.id.clone())
            || i.version == 0
        {
            return Err(err("invalid", "Invalid or duplicate issue identity"));
        }
        let p = a.products.iter().find(|p| p.id == i.product_id).unwrap();
        if !i
            .key
            .strip_prefix(&format!("{}-", p.key))
            .is_some_and(|n| n.parse::<u64>().is_ok_and(|v| v > 0))
        {
            return Err(err("invalid", "Issue key does not match its product"));
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
            if child.parent.as_ref() != Some(&i.key) || child.product_id != i.product_id {
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
    if a.comments.iter().any(|c| !keys.contains(&c.issue_key))
        || a.verifications
            .iter()
            .any(|v| !keys.contains(&v.issue_key) || v.steps.is_empty())
    {
        return Err(err("invalid", "Orphan comment or verification"));
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
