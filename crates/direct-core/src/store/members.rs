//! Agent members and explicit issue assignment (DIR-75).
//!
//! A member is a logical agent identity in this single-owner workspace. An
//! assignment names who should work an issue, under which exact role revision
//! and with which requested model. It is planning data: it never marks work
//! Ready, records review, takes or replaces a claim, or starts a session. The
//! actual session and model are recorded separately by the claim holder.

use super::*;
use serde::Serialize;

pub const RUNTIMES: [&str; 2] = ["claude-code", "codex"];

fn put<T: Serialize>(conn: &Connection, table: &str, id: &str, row: &T) -> Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO {table} VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data"
        ),
        params![id, serde_json::to_string(row)?],
    )?;
    Ok(())
}
pub(crate) fn put_member(conn: &Connection, m: &AgentMember) -> Result<()> {
    put(conn, "agent_members", &m.id, m)
}
pub(crate) fn put_assignment(conn: &Connection, a: &IssueAssignment) -> Result<()> {
    put(conn, "issue_assignments", &a.id, a)
}

fn row<T: DeserializeOwned>(conn: &Connection, table: &str, id: &str, what: &str) -> Result<T> {
    let data: Option<String> = conn
        .query_row(
            &format!("SELECT data FROM {table} WHERE id=?1"),
            [id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(serde_json::from_str(&data.ok_or_else(|| {
        err("not_found", format!("Unknown {what}"))
    })?)?)
}
fn member(conn: &Connection, id: &str) -> Result<AgentMember> {
    row(conn, "agent_members", id, "agent member")
}
fn assignment(conn: &Connection, id: &str) -> Result<IssueAssignment> {
    row(conn, "issue_assignments", id, "assignment")
}

fn stale(kind: &str, actual: u64, expected: u64) -> Error {
    err(
        "conflict",
        format!("This {kind} is version {actual}, not {expected}. Refresh before retrying."),
    )
}

fn valid_model(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b':' | b'/'))
}

/// A connection reference is a label, never a secret or a credentialed URL.
fn valid_connection_ref(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    !value.trim().is_empty()
        && value.len() <= 200
        && !lower.contains("://")
        && !value.contains('@')
        && ![
            "token", "secret", "password", "passwd", "apikey", "api_key", "api-key", "bearer",
            "sk-",
        ]
        .iter()
        .any(|word| lower.contains(word))
}

fn active_claim(i: &Issue, at: i64) -> Option<&Claim> {
    i.claim.as_ref().filter(|c| c.expires_at > at)
}

pub(crate) fn active_for(conn: &Connection, key: &str) -> Result<Option<IssueAssignment>> {
    Ok(all::<IssueAssignment>(conn, "issue_assignments")?
        .into_iter()
        .find(|a| a.issue_key == key && a.status == AssignmentStatus::Active))
}

/// The issue's assignment resolved for context: member, role and models.
pub(crate) fn context_for(conn: &Connection, key: &str) -> Result<Value> {
    let Some(a) = active_for(conn, key)? else {
        return Ok(Value::Null);
    };
    let m = member(conn, &a.member_id).ok();
    let r: Option<AgentRole> = row(conn, "agent_roles", &a.role_id, "role").ok();
    Ok(json!({
        "assignment": a,
        "member": m.map(|m| json!({"id":m.id,"name":m.name,"runtime":m.runtime,"enabled":m.enabled})),
        "role": r.map(|r| json!({"id":r.id,"key":r.key,"revision":r.revision,"name":r.name,"status":r.status})),
        "requested_model": a.requested_model,
        "actual_model": a.sessions.last().map(|s| s.model.clone()),
    }))
}

pub(crate) fn deletion_references(conn: &Connection, key: &str) -> Result<Vec<String>> {
    Ok(all::<IssueAssignment>(conn, "issue_assignments")?
        .into_iter()
        .filter(|a| a.issue_key == key)
        .map(|a| a.id)
        .collect())
}

/// A validated member configuration.
struct Config {
    name: String,
    connection_ref: String,
    product_ids: Vec<String>,
    default_role_key: Option<String>,
    default_model: Option<String>,
}

/// Shared validation for create and update.
fn configure(
    tx: &Transaction,
    name: &str,
    connection_ref: &str,
    product_ids: &[String],
    default_role_key: &Option<String>,
    default_model: &Option<String>,
) -> Result<Config> {
    let name = name.trim();
    required(name, "member name")?;
    limited(name, "member name", 120)?;
    let connection_ref = connection_ref.trim();
    if !valid_connection_ref(connection_ref) {
        return Err(err(
            "invalid",
            "Connection reference must be a short label, not a URL, account or secret",
        ));
    }
    if product_ids.is_empty() || product_ids.len() > 50 {
        return Err(err(
            "invalid",
            "Choose 1–50 products this member may work in",
        ));
    }
    let products = all::<Product>(tx, "products")?;
    let mut scope = Vec::new();
    for id in product_ids {
        if !products.iter().any(|p| p.id == *id) {
            return Err(err("not_found", "Unknown product in member scope"));
        }
        if !scope.contains(id) {
            scope.push(id.clone());
        }
    }
    let role_key = default_role_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty());
    if let Some(key) = role_key {
        if !all::<AgentRole>(tx, "agent_roles")?
            .iter()
            .any(|r| r.key == key)
        {
            return Err(err("not_found", "Unknown default role"));
        }
    }
    let model = default_model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty());
    if model.is_some_and(|m| !valid_model(m)) {
        return Err(err("invalid", "Model IDs are plain identifiers"));
    }
    Ok(Config {
        name: name.into(),
        connection_ref: connection_ref.into(),
        product_ids: scope,
        default_role_key: role_key.map(str::to_string),
        default_model: model.map(str::to_string),
    })
}

pub(crate) fn mutate(
    tx: &Transaction,
    cmd: &Command,
    actor: &str,
    role_kind: Role,
    at: i64,
) -> Result<Value> {
    match cmd {
        Command::CreateAgentMember {
            name,
            runtime,
            connection_ref,
            product_ids,
            default_role_key,
            default_model,
        } => {
            human(role_kind)?;
            if !RUNTIMES.contains(&runtime.as_str()) {
                return Err(err("invalid", "Runtime must be claude-code or codex"));
            }
            let Config {
                name,
                connection_ref,
                product_ids,
                default_role_key,
                default_model,
            } = configure(
                tx,
                name,
                connection_ref,
                product_ids,
                default_role_key,
                default_model,
            )?;
            if all::<AgentMember>(tx, "agent_members")?
                .iter()
                .any(|m| m.name.eq_ignore_ascii_case(&name))
            {
                return Err(err("conflict", "A member with this name already exists"));
            }
            let m = AgentMember {
                id: id(),
                name,
                runtime: runtime.clone(),
                enabled: true,
                connection_ref,
                product_ids,
                default_role_key,
                default_model,
                capability: None,
                version: 1,
                created_at: at,
                updated_at: at,
            };
            put_member(tx, &m)?;
            emit(tx, actor, "agent_member_created", &m.id, at)?;
            Ok(serde_json::to_value(m)?)
        }
        Command::UpdateAgentMember {
            id,
            expected_version,
            name,
            enabled,
            connection_ref,
            product_ids,
            default_role_key,
            default_model,
        } => {
            human(role_kind)?;
            let mut m = member(tx, id)?;
            if m.version != *expected_version {
                return Err(stale("member", m.version, *expected_version));
            }
            let Config {
                name,
                connection_ref,
                product_ids,
                default_role_key,
                default_model,
            } = configure(
                tx,
                name,
                connection_ref,
                product_ids,
                default_role_key,
                default_model,
            )?;
            if all::<AgentMember>(tx, "agent_members")?
                .iter()
                .any(|o| o.id != m.id && o.name.eq_ignore_ascii_case(&name))
            {
                return Err(err("conflict", "A member with this name already exists"));
            }
            m.name = name;
            m.enabled = *enabled;
            m.connection_ref = connection_ref;
            m.product_ids = product_ids;
            m.default_role_key = default_role_key;
            m.default_model = default_model;
            m.version += 1;
            m.updated_at = at;
            put_member(tx, &m)?;
            emit(tx, actor, "agent_member_updated", &m.id, at)?;
            Ok(serde_json::to_value(m)?)
        }
        Command::RecordMemberCapability {
            id,
            expected_version,
            harness_version,
            verified_models,
            evidence,
        } => {
            let mut m = member(tx, id)?;
            if m.version != *expected_version {
                return Err(stale("member", m.version, *expected_version));
            }
            let harness_version = harness_version.trim();
            required(harness_version, "harness version")?;
            limited(harness_version, "harness version", 200)?;
            let evidence = evidence.trim();
            required(evidence, "capability evidence")?;
            limited(evidence, "capability evidence", 500)?;
            if verified_models.len() > 20 || verified_models.iter().any(|m| !valid_model(m)) {
                return Err(err("invalid", "Provide up to 20 plain model IDs"));
            }
            m.capability = Some(MemberCapability {
                harness_version: harness_version.into(),
                verified_models: verified_models.clone(),
                evidence: evidence.into(),
                checked_by: actor.into(),
                checked_at: at,
            });
            m.version += 1;
            m.updated_at = at;
            put_member(tx, &m)?;
            emit(tx, actor, "agent_member_capability_checked", &m.id, at)?;
            Ok(serde_json::to_value(m)?)
        }
        Command::AssignIssueAgent {
            key,
            expected_assignment_version,
            member_id,
            role_id,
            requested_model,
            reconcile_active_writer,
        } => {
            human(role_kind)?;
            let i = issue(tx, key)?;
            if i.parent.is_some() {
                return Err(err(
                    "invalid",
                    "Verification runs are managed through their parent issue",
                ));
            }
            if matches!(
                i.status,
                Status::Done | Status::LegacyCompleted | Status::Canceled
            ) {
                return Err(err(
                    "invalid",
                    "Completed or canceled work cannot be assigned",
                ));
            }
            let previous = active_for(tx, key)?;
            match (&previous, expected_assignment_version) {
                (Some(p), Some(v)) if p.version == *v => {}
                (Some(p), Some(v)) => return Err(stale("assignment", p.version, *v)),
                (Some(p), None) => {
                    return Err(err(
                        "conflict",
                        format!(
                        "{key} already has an assignment (version {}); reassign with its version",
                        p.version
                    ),
                    ))
                }
                (None, Some(_)) => {
                    return Err(err(
                        "conflict",
                        format!(
                            "{key}'s assignment was cleared or replaced; refresh before retrying"
                        ),
                    ))
                }
                (None, None) => {}
            }
            let m = member(tx, member_id)?;
            if !m.enabled {
                return Err(err("invalid", format!("{} is disabled", m.name)));
            }
            if !m.product_ids.contains(&i.product_id) {
                return Err(err(
                    "invalid",
                    format!(
                        "{} is not permitted to work in this issue's product",
                        m.name
                    ),
                ));
            }
            let r: AgentRole = row(tx, "agent_roles", role_id, "agent role")?;
            if r.status != RoleStatus::Active {
                return Err(err(
                    "invalid",
                    format!(
                        "{} revision {} is not active; assign an active role revision",
                        r.key, r.revision
                    ),
                ));
            }
            if !r.runtime_compatibility.contains(&m.runtime) {
                return Err(err(
                    "invalid",
                    format!(
                        "{} revision {} is not written for {}",
                        r.key, r.revision, m.runtime
                    ),
                ));
            }
            let requested_model = requested_model.trim();
            if !valid_model(requested_model) {
                return Err(err("invalid", "Model IDs are plain identifiers"));
            }
            let verified = m
                .capability
                .as_ref()
                .is_some_and(|c| c.verified_models.iter().any(|v| v == requested_model));
            if !verified {
                return Err(err(
                    "invalid",
                    format!(
                        "{requested_model} is unverified for {}; run a capability check first",
                        m.name
                    ),
                ));
            }
            let reconciliation = match active_claim(&i, at) {
                Some(c) => {
                    let note = reconcile_active_writer
                        .as_deref()
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .ok_or_else(|| {
                            err(
                                "conflict",
                                format!(
                                    "{} holds an active claim; reconcile with that writer and record how before assigning",
                                    c.actor
                                ),
                            )
                        })?;
                    limited(note, "reconciliation note", 1_000)?;
                    Some(format!("Active writer {}: {note}", c.actor))
                }
                None => None,
            };
            if let Some(mut p) = previous {
                p.status = AssignmentStatus::Superseded;
                p.version += 1;
                p.updated_at = at;
                put_assignment(tx, &p)?;
            }
            let a = IssueAssignment {
                id: id(),
                issue_key: key.clone(),
                member_id: m.id,
                role_id: r.id,
                requested_model: requested_model.into(),
                status: AssignmentStatus::Active,
                version: 1,
                assigned_by: actor.into(),
                assigned_at: at,
                reconciliation,
                sessions: vec![],
                cleared: None,
                updated_at: at,
            };
            put_assignment(tx, &a)?;
            emit(tx, actor, "issue_agent_assigned", key, at)?;
            Ok(serde_json::to_value(a)?)
        }
        Command::ClearIssueAssignment {
            id,
            expected_version,
            reason,
        } => {
            human(role_kind)?;
            let mut a = assignment(tx, id)?;
            if a.version != *expected_version {
                return Err(stale("assignment", a.version, *expected_version));
            }
            if a.status != AssignmentStatus::Active {
                return Err(err("conflict", "Only the active assignment can be cleared"));
            }
            let reason = reason.trim();
            required(reason, "reason")?;
            limited(reason, "reason", 1_000)?;
            a.status = AssignmentStatus::Cleared;
            a.cleared = Some(Retirement {
                by: actor.into(),
                at,
                reason: reason.into(),
            });
            a.version += 1;
            a.updated_at = at;
            put_assignment(tx, &a)?;
            emit(tx, actor, "issue_agent_unassigned", &a.issue_key, at)?;
            Ok(serde_json::to_value(a)?)
        }
        Command::RecordAssignmentSession {
            id,
            session_id,
            model,
        } => {
            let mut a = assignment(tx, id)?;
            if a.status != AssignmentStatus::Active {
                return Err(err(
                    "invalid",
                    "Sessions are recorded on the active assignment",
                ));
            }
            if role_kind == Role::Agent {
                held(&issue(tx, &a.issue_key)?, actor, at)?;
            }
            let session_id = session_id.trim();
            required(session_id, "session ID")?;
            limited(session_id, "session ID", 200)?;
            let model = model.trim();
            if !valid_model(model) {
                return Err(err("invalid", "Model IDs are plain identifiers"));
            }
            if a.sessions.len() >= 50 {
                return Err(err(
                    "invalid",
                    "This assignment already records 50 sessions",
                ));
            }
            a.sessions.push(AssignmentSession {
                session_id: session_id.into(),
                model: model.into(),
                recorded_by: actor.into(),
                recorded_at: at,
            });
            a.version += 1;
            a.updated_at = at;
            put_assignment(tx, &a)?;
            emit(tx, actor, "issue_agent_session_recorded", &a.issue_key, at)?;
            Ok(serde_json::to_value(a)?)
        }
        _ => unreachable!("not a member command"),
    }
}

pub(crate) fn validate_archive(a: &Archive) -> Result<()> {
    if a.format < 21 && (!a.agent_members.is_empty() || !a.issue_assignments.is_empty()) {
        return Err(err("invalid", "Agent members require archive format 21"));
    }
    let invalid = |m: &str| err("invalid", format!("Invalid member archive: {m}"));
    let mut ids = HashSet::new();
    for m in &a.agent_members {
        if !ids.insert(m.id.as_str())
            || !RUNTIMES.contains(&m.runtime.as_str())
            || m.version == 0
            || m.product_ids
                .iter()
                .any(|p| !a.products.iter().any(|x| x.id == *p))
        {
            return Err(invalid("bad member"));
        }
    }
    let mut active = HashSet::new();
    for x in &a.issue_assignments {
        if !ids.insert(x.id.as_str())
            || !a.issues.iter().any(|i| i.key == x.issue_key)
            || !a.agent_members.iter().any(|m| m.id == x.member_id)
            || !a.agent_roles.iter().any(|r| r.id == x.role_id)
        {
            return Err(invalid("unresolved assignment"));
        }
        if x.status == AssignmentStatus::Active && !active.insert(x.issue_key.as_str()) {
            return Err(invalid("two active assignments for one issue"));
        }
    }
    Ok(())
}
