//! Durable Product Manager routines (DIR-78).
//!
//! A routine is an owner-configured, immutable-revision schedule for Product
//! Planner queue reviews. The service ticks it: a due routine gets exactly one
//! occurrence, several missed slots coalesce into a single latest-state check,
//! an unfinished review of the same product defers it, and a launched
//! occurrence is an ordinary DIR-77 queue review through the DIR-76 boundary.
//! Quiet results stay quiet; new or changed findings, owner decisions and
//! failures become local notices. External delivery is not built.

use super::*;
use jiff::{civil::Weekday, tz::TimeZone, Timestamp};
use serde::Serialize;

const WEEKDAYS: [(&str, Weekday); 7] = [
    ("mon", Weekday::Monday),
    ("tue", Weekday::Tuesday),
    ("wed", Weekday::Wednesday),
    ("thu", Weekday::Thursday),
    ("fri", Weekday::Friday),
    ("sat", Weekday::Saturday),
    ("sun", Weekday::Sunday),
];
/// Safety bound on counting missed slots after a long downtime.
const MAX_COALESCE: u32 = 10_000;

fn put<T: Serialize>(conn: &Connection, table: &str, id: &str, row: &T) -> Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO {table} VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data"
        ),
        params![id, serde_json::to_string(row)?],
    )?;
    Ok(())
}
pub(crate) fn put_routine(conn: &Connection, r: &Routine) -> Result<()> {
    put(conn, "routines", &r.id, r)
}
pub(crate) fn put_occurrence(conn: &Connection, o: &RoutineOccurrence) -> Result<()> {
    put(conn, "routine_occurrences", &o.id, o)
}
pub(crate) fn put_notice(conn: &Connection, n: &RoutineNotice) -> Result<()> {
    put(conn, "routine_notices", &n.id, n)
}

fn routine(conn: &Connection, id: &str) -> Result<Routine> {
    let data: Option<String> = conn
        .query_row("SELECT data FROM routines WHERE id=?1", [id], |r| r.get(0))
        .optional()?;
    Ok(serde_json::from_str(
        &data.ok_or_else(|| err("not_found", "Unknown routine"))?,
    )?)
}

fn parse_time(value: &str) -> Option<(i8, i8)> {
    let (h, m) = value.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (i8, i8) = (h.parse().ok()?, m.parse().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some((h, m))
}

fn validate_trigger(t: &RoutineTrigger) -> Result<()> {
    TimeZone::get(&t.timezone)
        .map_err(|_| err("invalid", format!("Unknown timezone {}", t.timezone)))?;
    if parse_time(&t.time).is_none() {
        return Err(err("invalid", "Time must be HH:MM (24-hour)"));
    }
    match (t.kind.as_str(), t.weekday.as_deref()) {
        ("daily", None) => Ok(()),
        ("weekly", Some(day)) if WEEKDAYS.iter().any(|(d, _)| *d == day) => Ok(()),
        ("weekly", _) => Err(err("invalid", "A weekly routine needs a weekday mon..sun")),
        ("daily", Some(_)) => Err(err("invalid", "A daily routine has no weekday")),
        _ => Err(err("invalid", "Trigger kind must be daily or weekly")),
    }
}

/// The first due instant strictly after `after` (Unix seconds).
pub fn next_due(after: i64, t: &RoutineTrigger) -> Result<i64> {
    let tz = TimeZone::get(&t.timezone).map_err(|_| err("invalid", "Unknown timezone"))?;
    let (h, m) = parse_time(&t.time).ok_or_else(|| err("invalid", "Bad trigger time"))?;
    let now = Timestamp::from_second(after)
        .map_err(|_| err("invalid", "Bad timestamp"))?
        .to_zoned(tz.clone());
    let mut date = now.date();
    let target = t.weekday.as_deref().and_then(|d| {
        WEEKDAYS
            .iter()
            .find(|(name, _)| *name == d)
            .map(|(_, w)| *w)
    });
    for _ in 0..16 {
        let matches_day = target.is_none_or(|w| date.weekday() == w);
        if matches_day {
            let candidate = date
                .at(h, m, 0, 0)
                .to_zoned(tz.clone())
                .map_err(|_| err("invalid", "Cannot resolve local time"))?
                .timestamp()
                .as_second();
            if candidate > after {
                return Ok(candidate);
            }
        }
        date = date
            .tomorrow()
            .map_err(|_| err("invalid", "Date overflow"))?;
    }
    Err(err("invalid", "No due time found"))
}

/// Validate an owner configuration into a revision.
fn revision(
    tx: &Connection,
    c: &RoutineConfig,
    number: u32,
    actor: &str,
    at: i64,
) -> Result<RoutineRevision> {
    let p = all::<Product>(tx, "products")?
        .into_iter()
        .find(|p| p.key == c.product)
        .ok_or_else(|| err("not_found", "Unknown product"))?;
    if c.states.is_empty() || c.states.len() > 7 {
        return Err(err("invalid", "Choose the issue states to review"));
    }
    if let Some(project) = &c.project_id {
        if !all::<Project>(tx, "projects")?
            .iter()
            .any(|x| x.id == *project && x.product_id == p.id)
        {
            return Err(err(
                "invalid",
                "The project must belong to the routine's product",
            ));
        }
    }
    let m = super::members::member_row(tx, &c.member_id)?;
    if !m.product_ids.contains(&p.id) {
        return Err(err(
            "invalid",
            format!("{} is not permitted to work in {}", m.name, p.key),
        ));
    }
    let r = super::roles::role_row(tx, &c.role_id)?;
    if !r.runtime_compatibility.contains(&m.runtime) {
        return Err(err(
            "invalid",
            "The role revision is not written for the member's runtime",
        ));
    }
    let objective = c.objective.trim();
    required(objective, "objective")?;
    limited(objective, "objective", 2_000)?;
    validate_trigger(&c.trigger)?;
    if !(1..=20).contains(&c.limits.max_issues) || !(1..=120).contains(&c.limits.max_minutes) {
        return Err(err(
            "invalid",
            "Limits: 1–20 issues and 1–120 minutes per run",
        ));
    }
    if c.limits
        .max_cost_usd
        .is_some_and(|v| !(v > 0.0 && v <= 1_000.0))
    {
        return Err(err(
            "invalid",
            "A cost limit must be between 0 and 1000 USD",
        ));
    }
    if c.notify != "local" {
        return Err(err(
            "invalid",
            "Only local notices are available; external notifications need separate authorization",
        ));
    }
    Ok(RoutineRevision {
        revision: number,
        product_id: p.id,
        states: c.states.clone(),
        project_id: c.project_id.clone(),
        member_id: m.id,
        role_id: r.id,
        requested_model: c.requested_model.trim().into(),
        policy: c.policy.clone(),
        objective: objective.into(),
        trigger: c.trigger.clone(),
        limits: c.limits.clone(),
        notify: "local".into(),
        created_by: actor.into(),
        created_at: at,
    })
}

/// Issues a revision would review now, and the ones it would exclude.
fn resolve(conn: &Connection, rev: &RoutineRevision) -> Result<(Vec<String>, Vec<BlockedKey>)> {
    let mut issues: Vec<Issue> = all::<Issue>(conn, "issues")?
        .into_iter()
        .filter(|i| {
            i.product_id == rev.product_id
                && i.parent.is_none()
                && rev.states.contains(&i.status)
                && rev
                    .project_id
                    .as_ref()
                    .is_none_or(|p| i.project_id.as_ref() == Some(p))
        })
        .collect();
    issues.sort_by_key(|i| std::cmp::Reverse(i.updated_at));
    let mut keys = Vec::new();
    let mut excluded = Vec::new();
    for i in issues {
        if keys.len() as u32 >= rev.limits.max_issues {
            break;
        }
        match super::planner::guidance_block(conn, &i)? {
            Some(reason) => excluded.push(BlockedKey { key: i.key, reason }),
            None => keys.push(i.key),
        }
    }
    Ok((keys, excluded))
}

/// Why the routine cannot dispatch right now, if anything.
fn dispatch_block(conn: &Connection, rev: &RoutineRevision) -> Result<Option<String>> {
    let m = super::members::member_row(conn, &rev.member_id)?;
    if !m.enabled {
        return Ok(Some(format!(
            "{} is disabled; new dispatch is blocked",
            m.name
        )));
    }
    let Some(c) = m
        .capability
        .as_ref()
        .filter(|c| c.verified_models.contains(&rev.requested_model))
    else {
        return Ok(Some(format!(
            "{} is unverified for {}; run a capability check",
            rev.requested_model, m.name
        )));
    };
    if super::runs::restored_at(conn)?.is_some_and(|r| c.checked_at < r) {
        return Ok(Some(format!(
            "{}'s capability predates the last restore; re-run its capability check",
            m.name
        )));
    }
    let r = super::roles::role_row(conn, &rev.role_id)?;
    if r.status != RoleStatus::Active {
        return Ok(Some(format!(
            "{} revision {} is not active",
            r.key, r.revision
        )));
    }
    Ok(None)
}

#[allow(clippy::too_many_arguments)]
fn notice(
    conn: &Connection,
    routine_id: &str,
    occurrence_id: Option<&str>,
    kind: &str,
    message: String,
    issue_key: Option<&str>,
    action: Option<&str>,
    at: i64,
) -> Result<()> {
    put_notice(
        conn,
        &RoutineNotice {
            id: Uuid::new_v4().to_string(),
            routine_id: routine_id.into(),
            occurrence_id: occurrence_id.map(str::to_string),
            kind: kind.into(),
            message,
            issue_key: issue_key.map(str::to_string),
            action: action.map(str::to_string),
            at,
            acknowledged: false,
        },
    )
}

pub(crate) fn preview(conn: &Connection, config: &RoutineConfig, at: i64) -> Result<Value> {
    let rev = revision(conn, config, 0, "preview", at)?;
    let (keys, excluded) = resolve(conn, &rev)?;
    Ok(json!({
        "keys": keys,
        "excluded": excluded,
        "next_due_at": next_due(at, &rev.trigger)?,
        "dispatch_block": dispatch_block(conn, &rev)?,
    }))
}

/// Request cancellation of an occurrence's unfinished run (pause/retire).
fn cancel_active(tx: &Transaction, routine_id: &str, actor: &str, at: i64) -> Result<()> {
    for o in all::<RoutineOccurrence>(tx, "routine_occurrences")?
        .into_iter()
        .filter(|o| o.routine_id == routine_id)
    {
        if o.state == "pending" || o.state == "deferred" {
            let mut o = o;
            o.state = "canceled".into();
            o.reason = Some("Routine paused before launch".into());
            o.updated_at = at;
            put_occurrence(tx, &o)?;
            continue;
        }
        let Some(run_id) = &o.run_id else { continue };
        super::runs::request_cancel(tx, run_id, actor, at)?;
    }
    Ok(())
}

pub(crate) fn mutate(
    tx: &Transaction,
    cmd: &Command,
    actor: &str,
    role: Role,
    at: i64,
) -> Result<Value> {
    match cmd {
        Command::CreateRoutine { name, config } => {
            human(role)?;
            let name = name.trim();
            required(name, "routine name")?;
            limited(name, "routine name", 120)?;
            let r = Routine {
                id: Uuid::new_v4().to_string(),
                name: name.into(),
                status: RoutineStatus::Paused,
                revisions: vec![revision(tx, config, 1, actor, at)?],
                next_due_at: None,
                activated_at: None,
                held_reason: None,
                version: 1,
                created_at: at,
                updated_at: at,
            };
            put_routine(tx, &r)?;
            emit(tx, actor, "routine_created", &r.id, at)?;
            Ok(serde_json::to_value(r)?)
        }
        Command::ReviseRoutine {
            id,
            expected_version,
            config,
        } => {
            human(role)?;
            let mut r = current(tx, id, *expected_version)?;
            if r.status == RoutineStatus::Retired {
                return Err(err("invalid", "A retired routine cannot be revised"));
            }
            let number = r.revisions.len() as u32 + 1;
            let rev = revision(tx, config, number, actor, at)?;
            if r.status == RoutineStatus::Active {
                r.next_due_at = Some(next_due(at, &rev.trigger)?);
            }
            r.revisions.push(rev);
            r.version += 1;
            r.updated_at = at;
            put_routine(tx, &r)?;
            emit(tx, actor, "routine_revised", &r.id, at)?;
            Ok(serde_json::to_value(r)?)
        }
        Command::SetRoutineStatus {
            id,
            expected_version,
            status,
        } => {
            human(role)?;
            let mut r = current(tx, id, *expected_version)?;
            if r.status == RoutineStatus::Retired {
                return Err(err("invalid", "A retired routine stays retired"));
            }
            let rev = r
                .revisions
                .last()
                .cloned()
                .expect("a routine has a revision");
            match status {
                RoutineStatus::Active => {
                    if let Some(reason) = dispatch_block(tx, &rev)? {
                        return Err(err("invalid", format!("Cannot activate: {reason}")));
                    }
                    r.activated_at = Some(at);
                    r.held_reason = None;
                    // Resume never backfills: the next due slot is after now.
                    r.next_due_at = Some(next_due(at, &rev.trigger)?);
                }
                RoutineStatus::Paused | RoutineStatus::Retired => {
                    r.next_due_at = None;
                    cancel_active(tx, &r.id, actor, at)?;
                }
            }
            r.status = status.clone();
            r.version += 1;
            r.updated_at = at;
            put_routine(tx, &r)?;
            let kind = format!(
                "routine_{}",
                serde_json::to_value(status)?.as_str().unwrap_or("updated")
            );
            emit(tx, actor, &kind, &r.id, at)?;
            Ok(serde_json::to_value(r)?)
        }
        Command::RunRoutineNow { id } => {
            human(role)?;
            let r = routine(tx, id)?;
            if r.status == RoutineStatus::Retired {
                return Err(err("invalid", "A retired routine cannot run"));
            }
            if all::<RoutineOccurrence>(tx, "routine_occurrences")?
                .iter()
                .any(|o| o.routine_id == r.id && (o.state == "pending" || o.state == "deferred"))
            {
                return Err(err(
                    "conflict",
                    "An occurrence of this routine is already waiting",
                ));
            }
            let o = RoutineOccurrence {
                id: Uuid::new_v4().to_string(),
                routine_id: r.id.clone(),
                revision: r.revisions.last().map(|v| v.revision).unwrap_or(1),
                due_at: at,
                coalesced: 0,
                manual: true,
                state: "pending".into(),
                run_id: None,
                reason: None,
                keys: vec![],
                created_at: at,
                updated_at: at,
            };
            put_occurrence(tx, &o)?;
            emit(tx, actor, "routine_run_requested", &r.id, at)?;
            Ok(serde_json::to_value(o)?)
        }
        Command::AcknowledgeRoutineNotice { id } => {
            human(role)?;
            let data: Option<String> = tx
                .query_row("SELECT data FROM routine_notices WHERE id=?1", [id], |r| {
                    r.get(0)
                })
                .optional()?;
            let mut n: RoutineNotice =
                serde_json::from_str(&data.ok_or_else(|| err("not_found", "Unknown notice"))?)?;
            n.acknowledged = true;
            put_notice(tx, &n)?;
            Ok(serde_json::to_value(n)?)
        }
        _ => unreachable!("not a routine command"),
    }
}

fn current(conn: &Connection, id: &str, expected: u64) -> Result<Routine> {
    let r = routine(conn, id)?;
    if r.version != expected {
        return Err(err(
            "conflict",
            format!(
                "This routine is version {}, not {expected}. Refresh before retrying.",
                r.version
            ),
        ));
    }
    Ok(r)
}

/// One scheduler pass at `at`. Returns the IDs of runs to start.
pub(crate) fn tick(tx: &Transaction, at: i64) -> Result<Vec<String>> {
    let restored = super::runs::restored_at(tx)?;
    let mut launched = Vec::new();
    for mut r in all::<Routine>(tx, "routines")?
        .into_iter()
        .filter(|r| r.status == RoutineStatus::Active)
    {
        // A routine activated before the last restore stays paused until reactivated.
        if restored.is_some_and(|x| r.activated_at.is_none_or(|a| a < x)) || r.held_reason.is_some()
        {
            continue;
        }
        let Some(due) = r.next_due_at.filter(|d| *d <= at) else {
            continue;
        };
        let rev = r
            .revisions
            .last()
            .cloned()
            .expect("a routine has a revision");
        let (mut slot, mut last, mut missed) = (due, due, 0u32);
        while slot <= at && missed < MAX_COALESCE {
            last = slot;
            slot = next_due(slot, &rev.trigger)?;
            missed += 1;
        }
        let waiting = all::<RoutineOccurrence>(tx, "routine_occurrences")?
            .into_iter()
            .find(|o| o.routine_id == r.id && (o.state == "pending" || o.state == "deferred"));
        match waiting {
            // An occurrence is already waiting: fold these slots into it.
            Some(mut o) => {
                o.coalesced += missed;
                o.due_at = last;
                o.updated_at = at;
                put_occurrence(tx, &o)?;
            }
            None => put_occurrence(
                tx,
                &RoutineOccurrence {
                    id: Uuid::new_v4().to_string(),
                    routine_id: r.id.clone(),
                    revision: rev.revision,
                    due_at: last,
                    coalesced: missed - 1,
                    manual: false,
                    state: "pending".into(),
                    run_id: None,
                    reason: None,
                    keys: vec![],
                    created_at: at,
                    updated_at: at,
                },
            )?,
        }
        r.next_due_at = Some(slot);
        r.updated_at = at;
        put_routine(tx, &r)?;
    }

    let mut waiting: Vec<RoutineOccurrence> = all::<RoutineOccurrence>(tx, "routine_occurrences")?
        .into_iter()
        .filter(|o| o.state == "pending" || o.state == "deferred")
        .collect();
    waiting.sort_by_key(|o| o.due_at);
    for mut o in waiting {
        let r = routine(tx, &o.routine_id)?;
        if r.status == RoutineStatus::Retired || (!o.manual && r.status != RoutineStatus::Active) {
            continue;
        }
        let rev = r
            .revisions
            .iter()
            .find(|v| v.revision == o.revision)
            .cloned()
            .unwrap_or_else(|| r.revisions.last().cloned().expect("revision"));
        o.updated_at = at;
        if let Some(reason) = dispatch_block(tx, &rev)? {
            o.state = "blocked".into();
            o.reason = Some(reason.clone());
            put_occurrence(tx, &o)?;
            notice(
                tx,
                &r.id,
                Some(&o.id),
                "blocked",
                format!("{}: {reason}", r.name),
                None,
                Some("Fix the member or role, then run the routine again"),
                at,
            )?;
            continue;
        }
        let busy = all::<AgentRun>(tx, "agent_runs")?.iter().any(|x| {
            x.queue
                .as_ref()
                .is_some_and(|q| q.product_id == rev.product_id)
                && !x.state.terminal()
        });
        if busy {
            // Serialize overlapping planner scopes: wait for the next tick.
            if o.state != "deferred" {
                o.state = "deferred".into();
                o.reason = Some("Another review of this product is still running".into());
                put_occurrence(tx, &o)?;
            }
            continue;
        }
        let (keys, _) = resolve(tx, &rev)?;
        if keys.is_empty() {
            o.state = "noop".into();
            o.reason = Some("No issues in scope".into());
            put_occurrence(tx, &o)?;
            continue;
        }
        let product = all::<Product>(tx, "products")?
            .into_iter()
            .find(|p| p.id == rev.product_id)
            .ok_or_else(|| err("invalid", "The routine's product is missing"))?;
        let cmd = Command::CreateQueueRun {
            product: product.key,
            keys: keys.clone(),
            member_id: rev.member_id.clone(),
            role_id: rev.role_id.clone(),
            requested_model: rev.requested_model.clone(),
            policy: rev.policy.clone(),
            objective: rev.objective.clone(),
        };
        let routine_actor = format!("routine:{}", r.id);
        let mut run = super::planner::create_queue_run(tx, &cmd, &routine_actor, Role::Human, at)?;
        run.occurrence_id = Some(o.id.clone());
        run.max_seconds = Some(u64::from(rev.limits.max_minutes) * 60);
        super::runs::put_run(tx, &run)?;
        emit(tx, &routine_actor, "agent_run_intent", &run.issue_key, at)?;
        o.state = "launched".into();
        o.run_id = Some(run.id.clone());
        o.keys = keys;
        o.reason = None;
        put_occurrence(tx, &o)?;
        launched.push(run.id);
    }
    Ok(launched)
}

/// Turn a finished routine run into notices. Quiet when nothing changed.
pub(crate) fn on_run_finished(tx: &Transaction, run: &AgentRun, at: i64) -> Result<()> {
    let Some(occurrence) = &run.occurrence_id else {
        return Ok(());
    };
    let data: Option<String> = tx
        .query_row(
            "SELECT data FROM routine_occurrences WHERE id=?1",
            [occurrence],
            |r| r.get(0),
        )
        .optional()?;
    let Some(o) = data
        .map(|d| serde_json::from_str::<RoutineOccurrence>(&d))
        .transpose()?
    else {
        return Ok(());
    };
    let mut r = routine(tx, &o.routine_id)?;
    match run.state {
        RunState::Succeeded => {
            for a in run.actions.iter().filter(|a| a.outcome == "applied") {
                let key = a.proposal["issue_key"].as_str();
                let message = match a.proposal["op"].as_str() {
                    Some("finding") => format!(
                        "{}: {} — {}",
                        key.unwrap_or("?"),
                        a.proposal["kind"].as_str().unwrap_or("finding"),
                        a.proposal["summary"].as_str().unwrap_or("")
                    ),
                    Some(op) => format!("{}: {op} applied", key.unwrap_or("?")),
                    None => continue,
                };
                notice(
                    tx,
                    &r.id,
                    Some(&o.id),
                    "changed_finding",
                    message,
                    key,
                    None,
                    at,
                )?;
            }
        }
        RunState::Failed | RunState::Unknown | RunState::Blocked => {
            let action = match run.state {
                RunState::Unknown => {
                    "Reconcile or reassign before the next run; redispatch is suspended"
                }
                RunState::Blocked => "Check the member, model and channel, then run again",
                _ => "Open the run for its reason, then run the routine again",
            };
            notice(
                tx,
                &r.id,
                Some(&o.id),
                "failure",
                format!(
                    "{}: review {:?} — {}",
                    r.name,
                    run.state,
                    run.reason.clone().unwrap_or_default()
                ),
                None,
                Some(action),
                at,
            )?;
        }
        _ => {}
    }
    let rev = r
        .revisions
        .iter()
        .find(|v| v.revision == o.revision)
        .cloned();
    if let (Some(limit), Some(cost)) = (rev.and_then(|v| v.limits.max_cost_usd), run.cost_usd) {
        if cost > limit {
            r.held_reason = Some(format!(
                "Last run cost ${cost:.2}, over the ${limit:.2} limit"
            ));
            r.version += 1;
            r.updated_at = at;
            put_routine(tx, &r)?;
            notice(
                tx,
                &r.id,
                Some(&o.id),
                "failure",
                format!("{}: cost limit exceeded (${cost:.2} > ${limit:.2})", r.name),
                None,
                Some("Review the cost, then reactivate the routine"),
                at,
            )?;
        }
    }
    Ok(())
}

/// A confirmed owner decision on a routine's escalation is worth a notice.
pub(crate) fn on_finding_confirmed(tx: &Transaction, f: &PlannerFinding, at: i64) -> Result<()> {
    let data: Option<String> = tx
        .query_row(
            "SELECT data FROM agent_runs WHERE id=?1",
            [&f.last_run],
            |r| r.get(0),
        )
        .optional()?;
    let Some(run) = data
        .map(|d| serde_json::from_str::<AgentRun>(&d))
        .transpose()?
    else {
        return Ok(());
    };
    let Some(occurrence) = run.occurrence_id else {
        return Ok(());
    };
    let routine_id: Option<String> = tx
        .query_row(
            "SELECT data FROM routine_occurrences WHERE id=?1",
            [&occurrence],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .and_then(|d| serde_json::from_str::<RoutineOccurrence>(&d).ok())
        .map(|o| o.routine_id);
    if let (Some(routine_id), Some(c)) = (routine_id, &f.confirmation) {
        notice(
            tx,
            &routine_id,
            Some(&occurrence),
            "owner_decision",
            format!(
                "{}: escalation {} by {} — {}",
                f.issue_key,
                if c.confirmed {
                    "confirmed"
                } else {
                    "dismissed"
                },
                c.by,
                c.note
            ),
            Some(&f.issue_key),
            None,
            at,
        )?;
    }
    Ok(())
}

pub(crate) fn validate_archive(a: &Archive) -> Result<()> {
    if a.format < 24
        && (!a.routines.is_empty()
            || !a.routine_occurrences.is_empty()
            || !a.routine_notices.is_empty())
    {
        return Err(err("invalid", "Routines require archive format 24"));
    }
    let mut ids = HashSet::new();
    for r in &a.routines {
        if !ids.insert(r.id.as_str()) || r.revisions.is_empty() || r.version == 0 {
            return Err(err("invalid", "Invalid routine archive"));
        }
        for v in &r.revisions {
            if !a.products.iter().any(|p| p.id == v.product_id)
                || !a.agent_members.iter().any(|m| m.id == v.member_id)
                || !a.agent_roles.iter().any(|x| x.id == v.role_id)
                || validate_trigger(&v.trigger).is_err()
            {
                return Err(err("invalid", "Invalid routine revision archive"));
            }
        }
    }
    for o in &a.routine_occurrences {
        if !ids.insert(o.id.as_str()) || !a.routines.iter().any(|r| r.id == o.routine_id) {
            return Err(err("invalid", "Invalid routine occurrence archive"));
        }
    }
    for n in &a.routine_notices {
        if !ids.insert(n.id.as_str()) || !a.routines.iter().any(|r| r.id == n.routine_id) {
            return Err(err("invalid", "Invalid routine notice archive"));
        }
    }
    Ok(())
}
