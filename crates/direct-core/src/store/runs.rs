//! Dispatched agent runs with scoped authority (DIR-76).
//!
//! A run is recorded as an intent before anything launches. The runner starts
//! the member's harness with no tools, so the executor can only *propose*
//! actions. Each proposal reaches the service through `run_action` with the
//! run's credential (only its SHA-256 is stored, never archived), and the
//! service alone decides what is allowed: a comment on the assigned issue, or
//! a body/acceptance edit while that issue is in Backlog. Everything else is
//! denied and recorded. The run ID is the harness session ID, so a dropped
//! launch is matched to its session and never relaunched.

use super::*;

const OBJECTIVE_MAX: usize = 2_000;
const ACTIONS_MAX: u32 = 10;
const CREDENTIAL_TTL: i64 = 2 * 60 * 60;

pub(crate) fn put_run(conn: &Connection, r: &AgentRun) -> Result<()> {
    conn.execute(
        "INSERT INTO agent_runs VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![r.id, serde_json::to_string(r)?],
    )?;
    Ok(())
}

fn run(conn: &Connection, id: &str) -> Result<AgentRun> {
    let data: Option<String> = conn
        .query_row("SELECT data FROM agent_runs WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(serde_json::from_str(
        &data.ok_or_else(|| err("not_found", "Unknown agent run"))?,
    )?)
}

fn current(conn: &Connection, id: &str, expected: u64) -> Result<AgentRun> {
    let r = run(conn, id)?;
    if r.version != expected {
        return Err(err(
            "conflict",
            format!(
                "This run is version {}, not {expected}. Refresh before retrying.",
                r.version
            ),
        ));
    }
    Ok(r)
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn revoke(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM run_credentials WHERE run_id=?1", [id])?;
    Ok(())
}

fn advance(
    tx: &Transaction,
    mut r: AgentRun,
    state: RunState,
    actor: &str,
    at: i64,
) -> Result<Value> {
    let kind = format!(
        "agent_run_{}",
        serde_json::to_value(&state)?.as_str().unwrap_or("updated")
    );
    if state.terminal() {
        revoke(tx, &r.id)?;
    }
    r.state = state;
    r.version += 1;
    r.updated_at = at;
    put_run(tx, &r)?;
    emit(tx, actor, &kind, &r.issue_key, at)?;
    Ok(serde_json::to_value(r)?)
}

pub(crate) fn for_issue(conn: &Connection, key: &str) -> Result<Vec<AgentRun>> {
    let mut runs: Vec<AgentRun> = all::<AgentRun>(conn, "agent_runs")?
        .into_iter()
        .filter(|r| r.issue_key == key)
        .collect();
    runs.sort_by_key(|r| r.created_at);
    Ok(runs)
}

pub(crate) fn deletion_references(conn: &Connection, key: &str) -> Result<Vec<String>> {
    Ok(for_issue(conn, key)?.into_iter().map(|r| r.id).collect())
}

fn restored_at(conn: &Connection) -> Result<Option<i64>> {
    let value: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key='restored_at'", [], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(value.and_then(|v| v.parse().ok()))
}

pub(crate) fn mark_restored(conn: &Connection, at: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO meta VALUES ('restored_at',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [at.to_string()],
    )?;
    Ok(())
}

/// Validate one executor proposal against the run's scope. `Ok` means the
/// proposal is allowed; `Err` is the recorded denial reason.
enum Allowed {
    Comment(String),
    Backlog {
        body: Option<String>,
        acceptance: Option<String>,
    },
}

fn judge(proposal: &Value, issue: &Issue) -> std::result::Result<Allowed, String> {
    let object = proposal
        .as_object()
        .ok_or("A proposal must be a JSON object")?;
    let op = object.get("op").and_then(Value::as_str).unwrap_or("");
    if let Some(key) = object.get("key") {
        if key.as_str() != Some(issue.key.as_str()) {
            return Err(format!(
                "Outside this run's scope: only {} may be changed",
                issue.key
            ));
        }
    }
    let text = |name: &str, max: usize| -> std::result::Result<Option<String>, String> {
        match object.get(name) {
            None => Ok(None),
            Some(Value::String(s)) if !s.trim().is_empty() && s.len() <= max => {
                Ok(Some(s.trim().to_string()))
            }
            Some(_) => Err(format!(
                "{name} must be non-empty text of at most {max} characters"
            )),
        }
    };
    let only = |fields: &[&str]| -> std::result::Result<(), String> {
        match object
            .keys()
            .find(|k| !fields.contains(&k.as_str()) && *k != "op" && *k != "key")
        {
            Some(extra) => Err(format!("Unknown field {extra} is not allowed")),
            None => Ok(()),
        }
    };
    match op {
        "comment" => {
            only(&["body"])?;
            let body = text("body", 10_000)?.ok_or("A comment needs a body")?;
            Ok(Allowed::Comment(body))
        }
        "update_backlog" => {
            only(&["body", "acceptance"])?;
            if issue.status != Status::Backlog {
                return Err(format!(
                    "{} is not in Backlog; planning runs only edit Backlog issues",
                    issue.key
                ));
            }
            let body = text("body", 20_000)?;
            let acceptance = text("acceptance", 10_000)?;
            if body.is_none() && acceptance.is_none() {
                return Err("update_backlog needs a body or acceptance".into());
            }
            Ok(Allowed::Backlog { body, acceptance })
        }
        "" => Err("A proposal needs an op".into()),
        other => Err(format!(
            "Outside this run's scope: {other} is not a planning action (allowed: comment, update_backlog)"
        )),
    }
}

pub(crate) fn mutate(
    tx: &Transaction,
    cmd: &Command,
    actor: &str,
    role: Role,
    at: i64,
) -> Result<Value> {
    match cmd {
        Command::CreateAgentRun {
            key,
            assignment_id,
            objective,
        } => {
            human(role)?;
            let objective = objective.trim();
            required(objective, "objective")?;
            limited(objective, "objective", OBJECTIVE_MAX)?;
            let issue = issue(tx, key)?;
            let assignment = members::active_for(tx, key)?
                .filter(|a| a.id == *assignment_id)
                .ok_or_else(|| {
                    err(
                        "conflict",
                        "That assignment is no longer active for this issue",
                    )
                })?;
            let member: AgentMember = super::members::member_row(tx, &assignment.member_id)?;
            if !member.enabled {
                return Err(err("invalid", format!("{} is disabled", member.name)));
            }
            let capability = member
                .capability
                .as_ref()
                .filter(|c| c.verified_models.contains(&assignment.requested_model));
            let Some(capability) = capability else {
                return Err(err(
                    "invalid",
                    format!(
                        "{} is unverified for {}; run a capability check first",
                        assignment.requested_model, member.name
                    ),
                ));
            };
            if restored_at(tx)?.is_some_and(|r| capability.checked_at < r) {
                return Err(err(
                    "invalid",
                    format!(
                        "{}'s capability predates the last restore; re-run its capability check",
                        member.name
                    ),
                ));
            }
            let role_row: AgentRole = super::roles::role_row(tx, &assignment.role_id)?;
            if role_row.status != RoleStatus::Active {
                return Err(err(
                    "invalid",
                    "The assigned role revision is no longer active",
                ));
            }
            let existing = for_issue(tx, key)?;
            if let Some(open) = existing.iter().find(|r| !r.state.terminal()) {
                return Err(err(
                    "conflict",
                    format!(
                        "Run {} for {key} has not finished; wait for it or cancel it",
                        open.id
                    ),
                ));
            }
            if existing
                .iter()
                .any(|r| r.state == RunState::Unknown && r.assignment_id == assignment.id)
            {
                return Err(err(
                    "conflict",
                    "A previous run of this assignment has an unknown outcome; redispatch is suspended until it is reconciled or the issue is reassigned",
                ));
            }
            if let Some(c) = issue.claim.as_ref().filter(|c| c.expires_at > at) {
                return Err(err(
                    "conflict",
                    format!(
                        "{} holds an active claim; reconcile with that writer before dispatching",
                        c.actor
                    ),
                ));
            }
            let mut skills = Vec::new();
            for id in &role_row.skills {
                skills.push(super::roles::skill_row(tx, id)?.bundle_sha256);
            }
            let r = AgentRun {
                id: Uuid::new_v4().to_string(),
                issue_key: key.clone(),
                assignment_id: assignment.id,
                member_id: member.id,
                role_id: role_row.id,
                skill_bundles: skills,
                guidance: role_row.guidance,
                requested_model: assignment.requested_model,
                fallback_models: vec![],
                input_issue_version: issue.version,
                objective: objective.into(),
                state: RunState::Intent,
                version: 1,
                created_by: actor.into(),
                created_at: at,
                launcher: None,
                harness_version: None,
                session_id: None,
                actual_model: None,
                actions: vec![],
                summary: String::new(),
                reason: None,
                cancel_requested_by: None,
                updated_at: at,
            };
            put_run(tx, &r)?;
            emit(tx, actor, "agent_run_intent", key, at)?;
            Ok(serde_json::to_value(r)?)
        }
        Command::StartAgentRun {
            id,
            expected_version,
            launcher,
            harness_version,
            credential_sha256,
        } => {
            let mut r = current(tx, id, *expected_version)?;
            if r.state != RunState::Intent {
                return Err(err(
                    "conflict",
                    "Only a recorded intent can be launched, once",
                ));
            }
            if !valid_fingerprint(credential_sha256) {
                return Err(err("invalid", "The credential digest must be SHA-256 hex"));
            }
            required(launcher, "launcher")?;
            limited(launcher, "launcher", 200)?;
            required(harness_version, "harness version")?;
            limited(harness_version, "harness version", 200)?;
            tx.execute(
                "INSERT INTO run_credentials VALUES (?1,?2,?3) ON CONFLICT(run_id) DO UPDATE SET sha256=excluded.sha256, expires_at=excluded.expires_at",
                params![r.id, credential_sha256.to_ascii_lowercase(), at + CREDENTIAL_TTL],
            )?;
            r.launcher = Some(launcher.trim().into());
            r.harness_version = Some(harness_version.trim().into());
            advance(tx, r, RunState::Launching, actor, at)
        }
        Command::RotateRunCredential {
            id,
            expected_version,
            credential_sha256,
        } => {
            let mut r = current(tx, id, *expected_version)?;
            if !matches!(r.state, RunState::Launching | RunState::Running) {
                return Err(err(
                    "conflict",
                    "Only an in-flight run can rotate its credential",
                ));
            }
            if !valid_fingerprint(credential_sha256) {
                return Err(err("invalid", "The credential digest must be SHA-256 hex"));
            }
            tx.execute(
                "INSERT INTO run_credentials VALUES (?1,?2,?3) ON CONFLICT(run_id) DO UPDATE SET sha256=excluded.sha256, expires_at=excluded.expires_at",
                params![r.id, credential_sha256.to_ascii_lowercase(), at + CREDENTIAL_TTL],
            )?;
            r.version += 1;
            r.updated_at = at;
            put_run(tx, &r)?;
            emit(tx, actor, "agent_run_credential_rotated", &r.issue_key, at)?;
            Ok(serde_json::to_value(r)?)
        }
        Command::RecordRunSession {
            id,
            expected_version,
            session_id,
            actual_model,
        } => {
            let mut r = current(tx, id, *expected_version)?;
            if r.state != RunState::Launching {
                return Err(err(
                    "conflict",
                    "The session is recorded once, while launching",
                ));
            }
            if session_id != &r.id {
                return Err(err(
                    "invalid",
                    "The harness session must use the run ID; this session cannot be matched",
                ));
            }
            let actual_model = actual_model.trim();
            required(actual_model, "actual model")?;
            limited(actual_model, "actual model", 80)?;
            r.session_id = Some(session_id.clone());
            r.actual_model = Some(actual_model.into());
            if actual_model != r.requested_model
                && !r.fallback_models.iter().any(|m| m == actual_model)
            {
                r.reason = Some(format!(
                    "Model substitution: requested {}, the session ran {actual_model}; no fallback is permitted",
                    r.requested_model
                ));
                return advance(tx, r, RunState::Blocked, actor, at);
            }
            advance(tx, r, RunState::Running, actor, at)
        }
        Command::RunAction {
            run_id,
            credential,
            index,
            proposal,
        } => {
            let mut r = run(tx, run_id)?;
            let stored: Option<(String, i64)> = tx
                .query_row(
                    "SELECT sha256, expires_at FROM run_credentials WHERE run_id=?1",
                    [run_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let valid = stored
                .is_some_and(|(sha, expires)| expires > at && sha == hex(credential.as_bytes()));
            if !valid || r.state != RunState::Running {
                return Err(err(
                    "forbidden",
                    "This run credential is not valid for a running run",
                ));
            }
            if *index >= ACTIONS_MAX {
                return Err(err("invalid", "A run may submit at most 10 proposals"));
            }
            if r.actions.iter().any(|a| a.index == *index) {
                return Err(err(
                    "conflict",
                    format!("Proposal {index} was already submitted"),
                ));
            }
            let mut i = issue(tx, &r.issue_key)?;
            let run_actor = format!("run:{}", r.id);
            let (outcome, detail) = match judge(proposal, &i) {
                Err(reason) => ("denied", reason),
                Ok(Allowed::Comment(body)) => {
                    add_comment(tx, &i.key, &run_actor, &body, at)?;
                    emit(tx, &run_actor, "comment_added", &i.key, at)?;
                    ("applied", "Comment added".to_string())
                }
                Ok(Allowed::Backlog { body, acceptance }) => {
                    if let Some(b) = body {
                        i.body = b;
                    }
                    if let Some(a) = acceptance {
                        i.acceptance = a;
                    }
                    save(tx, i, &run_actor, "issue_updated", at)?;
                    ("applied", "Backlog brief updated".to_string())
                }
            };
            r.actions.push(RunAction {
                index: *index,
                proposal: proposal.clone(),
                outcome: outcome.into(),
                detail: detail.clone(),
                at,
            });
            r.version += 1;
            r.updated_at = at;
            put_run(tx, &r)?;
            emit(tx, actor, "agent_run_action", &r.issue_key, at)?;
            Ok(
                json!({"run_id": r.id, "index": index, "outcome": outcome, "detail": detail, "version": r.version}),
            )
        }
        Command::FinishAgentRun {
            id,
            expected_version,
            succeeded,
            summary,
        } => {
            let mut r = current(tx, id, *expected_version)?;
            if r.state != RunState::Running {
                return Err(err("conflict", "Only a running run can finish"));
            }
            limited(summary, "summary", 4_000)?;
            r.summary = summary.trim().into();
            if !succeeded {
                r.reason = Some("The executor result could not be used".into());
            }
            let state = if *succeeded {
                RunState::Succeeded
            } else {
                RunState::Failed
            };
            advance(tx, r, state, actor, at)
        }
        Command::BlockAgentRun {
            id,
            expected_version,
            reason,
        } => {
            let mut r = current(tx, id, *expected_version)?;
            if !matches!(r.state, RunState::Intent | RunState::Launching) {
                return Err(err(
                    "conflict",
                    "Only a run that has not started working can be blocked",
                ));
            }
            required(reason.trim(), "reason")?;
            limited(reason, "reason", 1_000)?;
            r.reason = Some(reason.trim().into());
            advance(tx, r, RunState::Blocked, actor, at)
        }
        Command::MarkAgentRunUnknown {
            id,
            expected_version,
            reason,
        } => {
            let mut r = current(tx, id, *expected_version)?;
            if !matches!(
                r.state,
                RunState::Launching | RunState::Running | RunState::CancelPending
            ) {
                return Err(err("conflict", "Only an in-flight run can become unknown"));
            }
            required(reason.trim(), "reason")?;
            limited(reason, "reason", 1_000)?;
            r.reason = Some(reason.trim().into());
            advance(tx, r, RunState::Unknown, actor, at)
        }
        Command::CancelAgentRun {
            id,
            expected_version,
        } => {
            human(role)?;
            let mut r = current(tx, id, *expected_version)?;
            if r.state.terminal() || r.state == RunState::CancelPending {
                return Err(err(
                    "conflict",
                    "This run has already finished or is canceling",
                ));
            }
            r.cancel_requested_by = Some(actor.into());
            if r.state == RunState::Intent {
                // Nothing was launched, so there is nothing to wait for.
                return advance(tx, r, RunState::Canceled, actor, at);
            }
            advance(tx, r, RunState::CancelPending, actor, at)
        }
        Command::AcknowledgeRunCancel {
            id,
            expected_version,
        } => {
            let r = current(tx, id, *expected_version)?;
            if r.state != RunState::CancelPending {
                return Err(err("conflict", "No cancellation is pending for this run"));
            }
            advance(tx, r, RunState::Canceled, actor, at)
        }
        _ => unreachable!("not a run command"),
    }
}

pub(crate) fn validate_archive(a: &Archive) -> Result<()> {
    if a.format < 22 && !a.agent_runs.is_empty() {
        return Err(err("invalid", "Agent runs require archive format 22"));
    }
    let mut ids = HashSet::new();
    for r in &a.agent_runs {
        if !ids.insert(r.id.as_str())
            || r.version == 0
            || !a.issues.iter().any(|i| i.key == r.issue_key)
            || !a.agent_members.iter().any(|m| m.id == r.member_id)
            || !a.agent_roles.iter().any(|x| x.id == r.role_id)
            || !a.issue_assignments.iter().any(|x| x.id == r.assignment_id)
        {
            return Err(err("invalid", "Invalid agent run archive: unresolved run"));
        }
    }
    Ok(())
}
