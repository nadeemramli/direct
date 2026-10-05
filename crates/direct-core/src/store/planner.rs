//! Product Planner queue reviews and retained findings (DIR-77).
//!
//! A queue review is an agent run (DIR-76 boundary: tool-less executor,
//! service-judged proposals) over an explicit set of one product's issues.
//! Findings are stored here, never written into issues. Under the
//! `refine_backlog` policy a run may also comment on, or rewrite the brief and
//! acceptance of, scoped Backlog issues at the version it saw; the previous
//! text is preserved in an attributed comment and the original intake is
//! never touched. Everything else is denied and recorded.

use super::*;

pub const KINDS: [&str; 6] = [
    "unclear_outcome",
    "oversized",
    "missing_criteria",
    "duplicate",
    "dependency",
    "route_mismatch",
];
pub const ROUTES: [&str; 3] = ["bounded_brief", "discovery", "reviewed_design"];
const QUEUE_MAX: usize = 20;

pub(crate) fn put_finding(conn: &Connection, f: &PlannerFinding) -> Result<()> {
    conn.execute(
        "INSERT INTO planner_findings VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![f.id, serde_json::to_string(f)?],
    )?;
    Ok(())
}

pub(crate) fn for_issue(conn: &Connection, key: &str) -> Result<Vec<PlannerFinding>> {
    Ok(all::<PlannerFinding>(conn, "planner_findings")?
        .into_iter()
        .filter(|f| f.issue_key == key)
        .collect())
}

pub(crate) fn deletion_references(conn: &Connection, key: &str) -> Result<Vec<String>> {
    Ok(for_issue(conn, key)?.into_iter().map(|f| f.id).collect())
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The issue's own content and links. Comments and version bumps alone do not
/// change it, so identical evidence keeps one finding.
pub(crate) fn input_fingerprint(conn: &Connection, i: &Issue) -> Result<String> {
    let mut links: Vec<(String, String, String)> = all::<Value>(conn, "issue_links")?
        .into_iter()
        .filter(|l| l["source_key"] == i.key.as_str() || l["target_key"] == i.key.as_str())
        .map(|l| {
            (
                l["kind"].as_str().unwrap_or("").to_string(),
                l["source_key"].as_str().unwrap_or("").to_string(),
                l["target_key"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    links.sort();
    let status = serde_json::to_value(&i.status)?;
    Ok(hex(serde_json::to_vec(&(
        &i.title,
        &i.body,
        &i.acceptance,
        status,
        links,
    ))?))
}

fn guidance_block(conn: &Connection, i: &Issue) -> Result<Option<String>> {
    for r in &i.theoria_refs {
        let document = all::<TheoriaDocument>(conn, "theoria_documents")?
            .into_iter()
            .find(|d| d.id == r.document_id);
        match document {
            None => {
                return Ok(Some(format!(
                    "Pinned guidance {} is missing",
                    r.document_id
                )))
            }
            Some(d) if d.availability != TheoriaAvailability::Available => {
                return Ok(Some(format!("Pinned guidance {} is unavailable", d.id)))
            }
            Some(d)
                if r.recorded_fingerprint.is_some() && d.fingerprint != r.recorded_fingerprint =>
            {
                return Ok(Some(format!(
                    "Pinned guidance {} changed since it was linked; re-pin it before review",
                    d.id
                )))
            }
            _ => {}
        }
    }
    Ok(None)
}

/// Validate and record a queue review run. Called by the runs module.
pub(crate) fn create_queue_run(
    tx: &Transaction,
    cmd: &Command,
    actor: &str,
    role: Role,
    at: i64,
) -> Result<AgentRun> {
    let Command::CreateQueueRun {
        product,
        keys,
        member_id,
        role_id,
        requested_model,
        policy,
        objective,
    } = cmd
    else {
        unreachable!()
    };
    human(role)?;
    let objective = objective.trim();
    required(objective, "objective")?;
    limited(objective, "objective", 2_000)?;
    if keys.is_empty() || keys.len() > QUEUE_MAX {
        return Err(err("invalid", "Choose 1–20 issues to review"));
    }
    let p = all::<Product>(tx, "products")?
        .into_iter()
        .find(|p| p.key == *product)
        .ok_or_else(|| err("not_found", "Unknown product"))?;
    let m = super::members::member_row(tx, member_id)?;
    if !m.enabled {
        return Err(err("invalid", format!("{} is disabled", m.name)));
    }
    if !m.product_ids.contains(&p.id) {
        return Err(err(
            "invalid",
            format!("{} is not permitted to work in {}", m.name, p.key),
        ));
    }
    let requested_model = requested_model.trim();
    if !m
        .capability
        .as_ref()
        .is_some_and(|c| c.verified_models.iter().any(|v| v == requested_model))
    {
        return Err(err(
            "invalid",
            format!(
                "{requested_model} is unverified for {}; run a capability check first",
                m.name
            ),
        ));
    }
    let r = super::roles::role_row(tx, role_id)?;
    if r.status != RoleStatus::Active || !r.runtime_compatibility.contains(&m.runtime) {
        return Err(err(
            "invalid",
            "Choose an active role revision written for the member's runtime",
        ));
    }
    if all::<AgentRun>(tx, "agent_runs")?
        .iter()
        .any(|x| x.queue.as_ref().is_some_and(|q| q.product_id == p.id) && !x.state.terminal())
    {
        return Err(err(
            "conflict",
            format!(
                "A queue review of {} has not finished; wait for it or cancel it",
                p.key
            ),
        ));
    }
    let mut scope = Vec::new();
    let mut versions = Vec::new();
    let mut blocked = Vec::new();
    for key in keys {
        if scope.contains(key) || blocked.iter().any(|b: &BlockedKey| b.key == *key) {
            continue;
        }
        let i = issue(tx, key)?;
        if i.product_id != p.id || i.parent.is_some() {
            return Err(err(
                "invalid",
                format!("{key} is not a parent issue of {}", p.key),
            ));
        }
        match guidance_block(tx, &i)? {
            Some(reason) => blocked.push(BlockedKey {
                key: key.clone(),
                reason,
            }),
            None => {
                scope.push(key.clone());
                versions.push(i.version);
            }
        }
    }
    if scope.is_empty() {
        return Err(err(
            "invalid",
            "Every chosen issue is blocked by stale or unavailable guidance",
        ));
    }
    let mut skills = Vec::new();
    for id in &r.skills {
        skills.push(super::roles::skill_row(tx, id)?.bundle_sha256);
    }
    Ok(AgentRun {
        id: Uuid::new_v4().to_string(),
        issue_key: p.key.clone(),
        assignment_id: String::new(),
        member_id: m.id,
        role_id: r.id,
        skill_bundles: skills,
        guidance: r.guidance,
        requested_model: requested_model.into(),
        fallback_models: vec![],
        input_issue_version: 0,
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
        queue: Some(RunQueue {
            product_id: p.id,
            keys: scope,
            versions,
            policy: policy.clone(),
            blocked,
        }),
        updated_at: at,
    })
}

fn text(
    object: &serde_json::Map<String, Value>,
    name: &str,
    max: usize,
) -> std::result::Result<Option<String>, String> {
    match object.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !s.trim().is_empty() && s.len() <= max => {
            Ok(Some(s.trim().to_string()))
        }
        Some(_) => Err(format!(
            "{name} must be non-empty text of at most {max} characters"
        )),
    }
}

fn only(
    object: &serde_json::Map<String, Value>,
    fields: &[&str],
) -> std::result::Result<(), String> {
    match object
        .keys()
        .find(|k| !fields.contains(&k.as_str()) && *k != "op")
    {
        Some(extra) => Err(format!("Unknown field {extra} is not allowed")),
        None => Ok(()),
    }
}

fn escalation(value: Option<&Value>) -> std::result::Result<Option<Value>, String> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let o = value.as_object().ok_or("escalation must be an object")?;
    for field in [
        "criterion",
        "evidence",
        "impact",
        "options",
        "recommendation",
    ] {
        let ok = match o.get(field) {
            Some(Value::String(s)) => !s.trim().is_empty() && s.len() <= 2_000,
            Some(Value::Array(a)) => {
                !a.is_empty()
                    && a.len() <= 10
                    && a.iter().all(|x| {
                        x.as_str()
                            .is_some_and(|s| !s.trim().is_empty() && s.len() <= 500)
                    })
            }
            _ => false,
        };
        if !ok {
            return Err(format!("escalation.{field} is required"));
        }
    }
    if o.len() != 5 {
        return Err(
            "escalation has only criterion, evidence, impact, options and recommendation".into(),
        );
    }
    Ok(Some(value.clone()))
}

/// Judge one queue proposal and apply it when allowed. Returns the recorded
/// outcome (`applied`, `retained` or `denied`) and a reason.
pub(crate) fn apply(
    tx: &Transaction,
    run: &AgentRun,
    queue: &RunQueue,
    proposal: &Value,
    at: i64,
) -> Result<(&'static str, String)> {
    let deny = |reason: String| Ok(("denied", reason));
    let Some(object) = proposal.as_object() else {
        return deny("A proposal must be a JSON object".into());
    };
    let op = object.get("op").and_then(Value::as_str).unwrap_or("");
    if !["finding", "comment", "update_backlog"].contains(&op) {
        return deny(format!(
            "Outside this run's scope: {} is not a planning action (allowed: finding, comment, update_backlog)",
            if op.is_empty() { "a proposal without an op" } else { op }
        ));
    }
    let key = object
        .get("issue_key")
        .and_then(Value::as_str)
        .unwrap_or("");
    if let Some(b) = queue.blocked.iter().find(|b| b.key == key) {
        return deny(format!("{key} was excluded from this review: {}", b.reason));
    }
    let Some(position) = queue.keys.iter().position(|k| k == key) else {
        return deny(format!(
            "Outside this run's scope: {key:?} is not in the reviewed queue"
        ));
    };
    let mut i = issue(tx, key)?;
    let run_actor = format!("run:{}", run.id);
    match op {
        "finding" => {
            if let Err(e) = only(
                object,
                &[
                    "issue_key",
                    "kind",
                    "route",
                    "summary",
                    "evidence",
                    "recommendation",
                    "escalation",
                    "uncertain",
                ],
            ) {
                return deny(e);
            }
            let kind = object.get("kind").and_then(Value::as_str).unwrap_or("");
            if !KINDS.contains(&kind) {
                return deny(format!("Unknown finding kind {kind:?}"));
            }
            let route = match object.get("route").and_then(Value::as_str) {
                Some(r) if ROUTES.contains(&r) => Some(r.to_string()),
                Some(r) => return deny(format!("Unknown route {r:?}")),
                None => None,
            };
            let summary = match text(object, "summary", 2_000) {
                Ok(Some(s)) => s,
                Ok(None) => return deny("A finding needs a summary".into()),
                Err(e) => return deny(e),
            };
            let recommendation = match text(object, "recommendation", 2_000) {
                Ok(Some(s)) => s,
                Ok(None) => return deny("A finding needs a recommendation".into()),
                Err(e) => return deny(e),
            };
            let evidence: Vec<String> = match object.get("evidence").and_then(Value::as_array) {
                Some(a)
                    if !a.is_empty()
                        && a.len() <= 10
                        && a.iter().all(|e| {
                            e.as_str()
                                .is_some_and(|s| !s.trim().is_empty() && s.len() <= 300)
                        }) =>
                {
                    a.iter()
                        .filter_map(|e| e.as_str())
                        .map(|s| s.trim().to_string())
                        .collect()
                }
                _ => return deny("A finding needs 1–10 evidence references".into()),
            };
            let uncertain = object
                .get("uncertain")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let escalation = match escalation(object.get("escalation")) {
                Ok(e) => e,
                Err(reason) => return deny(reason),
            };
            if (uncertain || route.as_deref() == Some("reviewed_design")) && escalation.is_none() {
                return deny("Uncertain or reviewed-design findings need an escalation with criterion, evidence, impact, options and recommendation".into());
            }
            let fingerprint = input_fingerprint(tx, &i)?;
            let existing = all::<PlannerFinding>(tx, "planner_findings")?
                .into_iter()
                .find(|f| f.product_id == queue.product_id && f.issue_key == key && f.kind == kind);
            match existing {
                Some(mut f) if f.input_fingerprint == fingerprint => {
                    f.seen += 1;
                    f.last_run = run.id.clone();
                    f.version += 1;
                    f.updated_at = at;
                    put_finding(tx, &f)?;
                    Ok((
                        "retained",
                        format!(
                            "Finding already recorded for these inputs; retained (seen {} times)",
                            f.seen
                        ),
                    ))
                }
                Some(mut f) => {
                    f.revisions.push(FindingRevision {
                        run_id: f.last_run.clone(),
                        at: f.updated_at,
                        summary: f.summary.clone(),
                        input_fingerprint: f.input_fingerprint.clone(),
                    });
                    f.route = route;
                    f.summary = summary;
                    f.evidence = evidence;
                    f.recommendation = recommendation;
                    if escalation != f.escalation {
                        f.confirmation = None;
                    }
                    f.escalation = escalation;
                    f.input_fingerprint = fingerprint;
                    f.last_run = run.id.clone();
                    f.seen += 1;
                    f.version += 1;
                    f.updated_at = at;
                    put_finding(tx, &f)?;
                    emit(tx, &run_actor, "planner_finding_updated", key, at)?;
                    Ok((
                        "applied",
                        "Finding updated: its inputs changed (previous version kept)".into(),
                    ))
                }
                None => {
                    let f = PlannerFinding {
                        id: Uuid::new_v4().to_string(),
                        product_id: queue.product_id.clone(),
                        issue_key: key.into(),
                        kind: kind.into(),
                        route,
                        summary,
                        evidence,
                        recommendation,
                        escalation,
                        confirmation: None,
                        input_fingerprint: fingerprint,
                        first_run: run.id.clone(),
                        last_run: run.id.clone(),
                        seen: 1,
                        revisions: vec![],
                        version: 1,
                        created_at: at,
                        updated_at: at,
                    };
                    put_finding(tx, &f)?;
                    emit(tx, &run_actor, "planner_finding_recorded", key, at)?;
                    Ok(("applied", "Finding recorded".into()))
                }
            }
        }
        _ if queue.policy == RunPolicy::InspectOnly => {
            deny("This review is inspect-only; issue writes are not allowed".into())
        }
        "comment" => {
            if let Err(e) = only(object, &["issue_key", "body"]) {
                return deny(e);
            }
            let body = match text(object, "body", 10_000) {
                Ok(Some(b)) => b,
                Ok(None) => return deny("A comment needs a body".into()),
                Err(e) => return deny(e),
            };
            add_comment(tx, key, &run_actor, &body, at)?;
            emit(tx, &run_actor, "comment_added", key, at)?;
            Ok(("applied", "Comment added".into()))
        }
        _ => {
            if let Err(e) = only(
                object,
                &["issue_key", "expected_version", "body", "acceptance"],
            ) {
                return deny(e);
            }
            if i.status != Status::Backlog {
                return deny(format!("{key} is not in Backlog; submitted, ready or completed scope is never rewritten"));
            }
            let expected = object.get("expected_version").and_then(Value::as_u64);
            if expected != Some(i.version) || queue.versions[position] != i.version {
                return deny(format!(
                    "{key} changed since this review read it (now version {}); reread before editing, nothing was overwritten",
                    i.version
                ));
            }
            if let Some(c) = i.claim.as_ref().filter(|c| c.expires_at > at) {
                return deny(format!(
                    "{} holds an active claim on {key}; not overwriting their work",
                    c.actor
                ));
            }
            let body = match text(object, "body", 20_000) {
                Ok(b) => b,
                Err(e) => return deny(e),
            };
            let acceptance = match text(object, "acceptance", 10_000) {
                Ok(a) => a,
                Err(e) => return deny(e),
            };
            if body.is_none() && acceptance.is_none() {
                return deny("update_backlog needs a body or acceptance".into());
            }
            let mut preserved = format!("Previous text preserved before {run_actor} rewrote it:");
            if let Some(b) = &body {
                preserved += &format!(
                    "\n\n## Previous brief\n\n{}",
                    if i.body.is_empty() {
                        "(empty)"
                    } else {
                        &i.body
                    }
                );
                i.body = b.clone();
            }
            if let Some(a) = &acceptance {
                preserved += &format!(
                    "\n\n## Previous acceptance\n\n{}",
                    if i.acceptance.is_empty() {
                        "(empty)"
                    } else {
                        &i.acceptance
                    }
                );
                i.acceptance = a.clone();
            }
            add_comment(tx, key, &run_actor, &preserved, at)?;
            save(tx, i, &run_actor, "issue_updated", at)?;
            Ok((
                "applied",
                "Backlog brief updated; previous text preserved in a comment".into(),
            ))
        }
    }
}

pub(crate) fn confirm(tx: &Transaction, cmd: &Command, actor: &str, at: i64) -> Result<Value> {
    let Command::ConfirmPlannerFinding {
        id,
        expected_version,
        confirmed,
        note,
    } = cmd
    else {
        unreachable!()
    };
    if actor.starts_with("run:") {
        return Err(err("forbidden", "A run cannot confirm its own findings"));
    }
    let data: Option<String> = tx
        .query_row("SELECT data FROM planner_findings WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    let mut f: PlannerFinding =
        serde_json::from_str(&data.ok_or_else(|| err("not_found", "Unknown planner finding"))?)?;
    if f.version != *expected_version {
        return Err(err(
            "conflict",
            format!(
                "This finding is version {}, not {expected_version}. Refresh before retrying.",
                f.version
            ),
        ));
    }
    if f.escalation.is_none() {
        return Err(err("invalid", "Only escalations need confirmation"));
    }
    let note = note.trim();
    required(note, "note")?;
    limited(note, "note", 1_000)?;
    f.confirmation = Some(FindingConfirmation {
        by: actor.into(),
        at,
        confirmed: *confirmed,
        note: note.into(),
    });
    f.version += 1;
    f.updated_at = at;
    put_finding(tx, &f)?;
    emit(tx, actor, "planner_finding_confirmed", &f.issue_key, at)?;
    Ok(serde_json::to_value(f)?)
}

pub(crate) fn validate_archive(a: &Archive) -> Result<()> {
    if a.format < 23
        && (!a.planner_findings.is_empty() || a.agent_runs.iter().any(|r| r.queue.is_some()))
    {
        return Err(err("invalid", "Planner reviews require archive format 23"));
    }
    let mut ids = HashSet::new();
    let mut keys = HashSet::new();
    for f in &a.planner_findings {
        if !ids.insert(f.id.as_str())
            || !keys.insert((f.product_id.as_str(), f.issue_key.as_str(), f.kind.as_str()))
            || !KINDS.contains(&f.kind.as_str())
            || !a
                .issues
                .iter()
                .any(|i| i.key == f.issue_key && i.product_id == f.product_id)
        {
            return Err(err("invalid", "Invalid planner finding archive"));
        }
    }
    Ok(())
}
