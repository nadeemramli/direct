//! Implementation dispatch of owner-Ready work (DIR-79).
//!
//! A `dispatch_ready` routine launches one local implementation session per
//! product lane. The lane holds at most one implementation writer: while a
//! dispatch run is in flight, or any issue of the product holds an active
//! claim, nothing new is launched there (other products replenish
//! independently). Only owner-Ready parent issues are candidates; each one left
//! out carries its reason (claim, dependency, release, guidance, assignment,
//! open cloud handoff, earlier unknown dispatch).
//!
//! A session takes at most three objectives, two when one is a review repair.
//! It works through the run credential: it may claim its objectives one at a
//! time at the version it was dispatched with, renew, comment and report a
//! pushed branch and PR. It cannot merge, deliver or submit. Claims outlive the
//! session so the work stays single-writer until a coordinator takes it over
//! with `take_over_dispatch`, integrates it, runs E2E at the Windows entrypoint
//! and submits. Further objectives go to a fresh session with its own verified
//! model, after the previous one has stopped, and inherit its handoff.

use super::*;

pub const DISPATCH_ACTOR_PREFIX: &str = "dispatch:";
const CAP: u32 = 3;
const REPAIR_CAP: u32 = 2;
pub(crate) const ACTIONS_MAX: u32 = 60;
/// A dispatched claim outlives its session by this much, so a coordinator can
/// take it over before the issue could look abandoned.
const HANDOFF_LEASE: i64 = 4 * 60 * 60;
/// Prefixes no configuration may allow.
const NEVER_ALLOWED: [&str; 7] = [
    "git push",
    "git reset",
    "git clean",
    "git branch",
    "git rebase",
    "gh pr merge",
    "gh api",
];

/// The branch a dispatched objective must be pushed to.
pub fn dispatch_branch(key: &str, run_id: &str) -> String {
    format!(
        "dispatch/{}-{}",
        key.to_ascii_lowercase(),
        &run_id[..run_id.len().min(8)]
    )
}

fn actor_for(run_id: &str) -> String {
    format!("{DISPATCH_ACTOR_PREFIX}{run_id}")
}

fn absolute(path: &str) -> bool {
    let b = path.as_bytes();
    path.starts_with('/')
        || path.starts_with("\\\\")
        || (b.len() > 2
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && (b[2] == b'\\' || b[2] == b'/'))
}

fn hex40(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Validate an owner dispatch configuration for a product.
pub(crate) fn validate_config(
    conn: &Connection,
    product: &Product,
    c: &DispatchConfig,
) -> Result<DispatchConfig> {
    let repository = c.repository.trim();
    let parts: Vec<&str> = repository.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || !p
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
    {
        return Err(err("invalid", "The repository must be GitHub owner/name"));
    }
    let base_ref = c.base_ref.trim();
    if base_ref.is_empty()
        || base_ref.len() > 200
        || base_ref.starts_with('-')
        || base_ref.chars().any(char::is_whitespace)
    {
        return Err(err(
            "invalid",
            "The base ref must be a ref such as origin/main",
        ));
    }
    let worktree_root = c.worktree_root.trim();
    if !absolute(worktree_root) || worktree_root.len() > 400 {
        return Err(err("invalid", "The worktree root must be an absolute path"));
    }
    let checkout = match c.checkout.as_deref().map(str::trim) {
        Some(path) if !path.is_empty() => path.to_string(),
        _ => product.repo_windows.trim().to_string(),
    };
    if !absolute(&checkout) || checkout.len() > 400 {
        return Err(err(
            "invalid",
            "Set an absolute checkout path (the product has no Windows repository)",
        ));
    }
    if let Some(id) = &c.release_id {
        let r = release(conn, id)?;
        if r.product_id != product.id {
            return Err(err(
                "invalid",
                "The release must belong to the routine's product",
            ));
        }
        if matches!(r.status, ReleaseStatus::Retired | ReleaseStatus::Canceled) {
            return Err(err("invalid", "The release is retired or canceled"));
        }
    }
    if c.allow_commands.len() > 20 {
        return Err(err("invalid", "At most 20 extra command prefixes"));
    }
    let mut allow = Vec::new();
    for command in &c.allow_commands {
        let command = command.trim();
        if command.is_empty()
            || command.len() > 80
            || command.starts_with('-')
            || !command
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b" ._/:=+-".contains(&b))
        {
            return Err(err(
                "invalid",
                "Command prefixes are plain words such as `cargo test` or `npm run build`",
            ));
        }
        if NEVER_ALLOWED.iter().any(|n| command.starts_with(n)) {
            return Err(err(
                "invalid",
                format!("`{command}` cannot be allowed: pushes, history rewrites and merges stay with the runner's rules or the coordinator"),
            ));
        }
        allow.push(command.to_string());
    }
    Ok(DispatchConfig {
        checkout: Some(checkout),
        repository: repository.into(),
        base_ref: base_ref.into(),
        worktree_root: worktree_root.into(),
        release_id: c.release_id.clone(),
        allow_commands: allow,
    })
}

/// A product lane at one moment: whether it can dispatch, and what.
pub(crate) struct Lane {
    /// ready, running, blocked or exhausted.
    pub state: &'static str,
    pub detail: String,
    pub eligible: Vec<Issue>,
    pub excluded: Vec<BlockedKey>,
}

/// The first `blocked_by` dependency that is not yet delivered for review.
/// Verify counts as met: pending human acceptance never blocks dependents.
pub(crate) fn unmet_dependency(conn: &Connection, i: &Issue) -> Result<Option<String>> {
    for l in all::<IssueLink>(conn, "issue_links")?
        .into_iter()
        .filter(|l| l.source_key == i.key && l.kind == IssueLinkKind::BlockedBy)
    {
        let target = issue(conn, &l.target_key)?;
        match target.status {
            Status::Verify | Status::Done | Status::LegacyCompleted => {}
            Status::Canceled => {
                return Ok(Some(format!(
                    "Blocked by {} (canceled; the owner must re-plan the dependency)",
                    target.key
                )))
            }
            other => {
                return Ok(Some(format!(
                    "Blocked by {} ({})",
                    target.key,
                    serde_json::to_value(other)?.as_str().unwrap_or("open")
                )))
            }
        }
    }
    Ok(None)
}

fn priority_rank(p: &str) -> u8 {
    match p {
        "urgent" => 0,
        "high" => 1,
        "medium" => 2,
        "low" => 3,
        _ => 4,
    }
}

pub(crate) fn lane(conn: &Connection, rev: &RoutineRevision, at: i64) -> Result<Lane> {
    let config = rev
        .dispatch
        .as_ref()
        .ok_or_else(|| err("invalid", "This routine has no dispatch configuration"))?;
    let runs = all::<AgentRun>(conn, "agent_runs")?;
    let issues: Vec<Issue> = all::<Issue>(conn, "issues")?
        .into_iter()
        .filter(|i| i.product_id == rev.product_id)
        .collect();
    let running = |detail: String| Lane {
        state: "running",
        detail,
        eligible: vec![],
        excluded: vec![],
    };
    if let Some(r) = runs.iter().find(|r| {
        r.dispatch
            .as_ref()
            .is_some_and(|d| d.product_id == rev.product_id)
            && !r.state.terminal()
    }) {
        return Ok(running(format!(
            "Dispatch session {} is still in flight",
            &r.id[..8]
        )));
    }
    if let Some((key, actor)) = issues.iter().find_map(|i| {
        i.claim
            .as_ref()
            .filter(|c| c.expires_at > at)
            .map(|c| (i.key.clone(), c.actor.clone()))
    }) {
        return Ok(running(format!(
            "{key} is claimed by {actor}; one implementation writer per product"
        )));
    }
    let release = config
        .release_id
        .as_deref()
        .map(|id| release(conn, id))
        .transpose()?;
    let open_handoffs: HashSet<String> = all::<CloudHandoff>(conn, "cloud_handoffs")?
        .into_iter()
        .filter(|h| h.status == CloudHandoffStatus::Prepared)
        .map(|h| h.issue_key)
        .collect();
    let unknown: HashSet<String> = runs
        .iter()
        .filter(|r| r.state == RunState::Unknown)
        .filter_map(|r| r.dispatch.as_ref())
        .flat_map(|d| {
            d.objectives
                .iter()
                .filter(|o| o.state != "pending")
                .map(|o| o.key.clone())
        })
        .collect();
    let mut eligible = Vec::new();
    let mut excluded = Vec::new();
    let exclude = |excluded: &mut Vec<BlockedKey>, key: &str, reason: String| {
        excluded.push(BlockedKey {
            key: key.into(),
            reason,
        })
    };
    for i in issues.iter().filter(|i| {
        i.parent.is_none()
            && rev
                .project_id
                .as_ref()
                .is_none_or(|p| i.project_id.as_ref() == Some(p))
    }) {
        match i.status {
            Status::Ready => {}
            Status::Doing => {
                let reason = match &i.claim {
                    Some(c) => format!(
                        "The claim by {} expired; reconcile that writer before redispatch",
                        c.actor
                    ),
                    None => "In Doing without a claim; reconcile before redispatch".into(),
                };
                exclude(&mut excluded, &i.key, reason);
                continue;
            }
            ref other if rev.states.contains(other) => {
                exclude(
                    &mut excluded,
                    &i.key,
                    format!(
                        "Not authorized: {} is not owner-Ready work",
                        serde_json::to_value(other)?.as_str().unwrap_or("")
                    ),
                );
                continue;
            }
            _ => continue,
        }
        if let Some(a) = super::members::active_for(conn, &i.key)? {
            if a.member_id != rev.member_id {
                let name = super::members::member_row(conn, &a.member_id)
                    .map(|m| m.name)
                    .unwrap_or(a.member_id);
                exclude(
                    &mut excluded,
                    &i.key,
                    format!("Not authorized for this member: assigned to {name}"),
                );
                continue;
            }
        }
        if let Some(r) = &release {
            let in_release = r.issue_keys.contains(&i.key)
                || i.project_id
                    .as_ref()
                    .is_some_and(|p| r.project_ids.contains(p));
            if !in_release {
                exclude(&mut excluded, &i.key, format!("Outside release {}", r.name));
                continue;
            }
        }
        if let Some(reason) = unmet_dependency(conn, i)? {
            exclude(&mut excluded, &i.key, reason);
            continue;
        }
        if let Some(reason) = super::planner::guidance_block(conn, i)? {
            exclude(&mut excluded, &i.key, reason);
            continue;
        }
        if open_handoffs.contains(&i.key) {
            exclude(
                &mut excluded,
                &i.key,
                "A cloud handoff is open; reconcile or withdraw it first".into(),
            );
            continue;
        }
        if unknown.contains(&i.key) {
            exclude(
                &mut excluded,
                &i.key,
                "An earlier dispatch of it ended unknown; reconcile before redispatch".into(),
            );
            continue;
        }
        eligible.push(i.clone());
    }
    eligible.sort_by(|a, b| {
        priority_rank(&a.priority)
            .cmp(&priority_rank(&b.priority))
            .then(a.created_at.cmp(&b.created_at))
    });
    let (state, detail) = match (eligible.len(), excluded.len()) {
        (0, 0) => ("exhausted", "No owner-Ready work is waiting".to_string()),
        (0, n) => ("blocked", format!("Every candidate is excluded ({n})")),
        (n, _) => ("ready", format!("{n} eligible")),
    };
    Ok(Lane {
        state,
        detail,
        eligible,
        excluded,
    })
}

/// The objectives one session takes, and its cap.
pub(crate) fn select(eligible: &[Issue], max_issues: u32) -> (Vec<Issue>, u32) {
    let mut chosen: Vec<Issue> = Vec::new();
    let mut repair = false;
    for i in eligible {
        let with = repair || i.needs_fix;
        let cap = if with { REPAIR_CAP } else { CAP }.min(max_issues);
        if chosen.len() as u32 + 1 > cap {
            continue;
        }
        chosen.push(i.clone());
        repair = with;
    }
    (chosen, if repair { REPAIR_CAP } else { CAP })
}

/// Record a dispatch run for a lane that is ready.
pub(crate) fn create_run(
    tx: &Transaction,
    rev: &RoutineRevision,
    lane: &Lane,
    actor: &str,
    at: i64,
) -> Result<AgentRun> {
    let config = rev.dispatch.clone().expect("dispatch revision");
    let product = all::<Product>(tx, "products")?
        .into_iter()
        .find(|p| p.id == rev.product_id)
        .ok_or_else(|| err("invalid", "The routine's product is missing"))?;
    let (chosen, cap) = select(&lane.eligible, rev.limits.max_issues);
    let role = super::roles::role_row(tx, &rev.role_id)?;
    let mut skills = Vec::new();
    for id in &role.skills {
        skills.push(super::roles::skill_row(tx, id)?.bundle_sha256);
    }
    let previous = all::<AgentRun>(tx, "agent_runs")?
        .into_iter()
        .filter(|r| {
            r.dispatch
                .as_ref()
                .is_some_and(|d| d.product_id == rev.product_id)
        })
        .max_by_key(|r| r.created_at)
        .map(|r| r.id);
    Ok(AgentRun {
        id: Uuid::new_v4().to_string(),
        issue_key: product.key,
        assignment_id: String::new(),
        member_id: rev.member_id.clone(),
        role_id: role.id,
        skill_bundles: skills,
        guidance: role.guidance,
        requested_model: rev.requested_model.clone(),
        fallback_models: vec![],
        input_issue_version: 0,
        objective: rev.objective.clone(),
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
        queue: None,
        occurrence_id: None,
        max_seconds: Some(u64::from(rev.limits.max_minutes) * 60),
        cost_usd: None,
        dispatch: Some(RunDispatch {
            product_id: rev.product_id.clone(),
            config,
            cap,
            objectives: chosen
                .into_iter()
                .map(|i| DispatchObjective {
                    key: i.key,
                    version: i.version,
                    review_repair: i.needs_fix,
                    state: "pending".into(),
                    guidance: i.theoria_refs,
                    claim_actor: None,
                    branch: None,
                    base_sha: None,
                    head_sha: None,
                    pr_url: None,
                    evidence: String::new(),
                    verified_push: None,
                    detail: String::new(),
                    taken_over_by: None,
                })
                .collect(),
            excluded: lane.excluded.clone(),
            handoff_from: previous,
            worktree: None,
            base_sha: None,
        }),
        updated_at: at,
    })
}

/// The runner prepared the worktree: record it at launch.
pub(crate) fn start(
    r: &mut AgentRun,
    worktree: &Option<String>,
    base_sha: &Option<String>,
) -> Result<i64> {
    let Some(d) = r.dispatch.as_mut() else {
        if worktree.is_some() || base_sha.is_some() {
            return Err(err("invalid", "Only dispatch runs take a worktree"));
        }
        return Ok(0);
    };
    let (Some(worktree), Some(base)) = (worktree, base_sha) else {
        return Err(err(
            "invalid",
            "A dispatch run starts with its prepared worktree and base commit",
        ));
    };
    let base = base.trim().to_ascii_lowercase();
    if !absolute(worktree) || worktree.len() > 500 || !hex40(&base) {
        return Err(err(
            "invalid",
            "The worktree must be an absolute path and the base a full commit SHA",
        ));
    }
    d.worktree = Some(worktree.trim().into());
    d.base_sha = Some(base);
    // The credential lives as long as the session may.
    Ok(r.max_seconds.unwrap_or(0) as i64 + 600)
}

fn lease(r: &AgentRun) -> i64 {
    (r.max_seconds.unwrap_or(0) as i64 + HANDOFF_LEASE).min(86_400)
}

fn field<'a>(o: &'a serde_json::Map<String, Value>, name: &str) -> Option<&'a str> {
    o.get(name).and_then(Value::as_str).map(str::trim)
}

/// Judge and apply one session action. Returns the outcome and its detail.
pub(crate) fn apply(
    tx: &Transaction,
    r: &mut AgentRun,
    proposal: &Value,
    at: i64,
) -> Result<(&'static str, String)> {
    let run_actor = actor_for(&r.id);
    let lease = lease(r);
    let run_id = r.id.clone();
    let d = r.dispatch.as_mut().expect("dispatch run");
    let Some(object) = proposal.as_object() else {
        return Ok(("denied", "An action must be a JSON object".into()));
    };
    let op = field(object, "op").unwrap_or("");
    let allowed: &[&str] = match op {
        "claim" | "renew" => &["key"],
        "comment" => &["key", "body"],
        "report" => &[
            "key", "outcome", "summary", "branch", "base_sha", "head_sha", "pr_url",
        ],
        _ => {
            return Ok((
                "denied",
                format!("Outside this session's authority: `{op}` (allowed: claim, renew, comment, report). Merge, delivery and submission stay with the coordinator."),
            ))
        }
    };
    if let Some(extra) = object
        .keys()
        .find(|k| *k != "op" && !allowed.contains(&k.as_str()))
    {
        return Ok(("denied", format!("Unknown field {extra} is not allowed")));
    }
    let key = field(object, "key").unwrap_or("");
    let Some(pos) = d.objectives.iter().position(|o| o.key == key) else {
        return Ok((
            "denied",
            format!("{key} is not an objective of this session"),
        ));
    };
    let state = d.objectives[pos].state.clone();
    match op {
        "claim" => {
            if state != "pending" {
                return Ok(("denied", format!("{key} is already {state}")));
            }
            if let Some(open) = d.objectives.iter().find(|o| o.state == "claimed") {
                return Ok((
                    "denied",
                    format!("Report {} before claiming the next objective", open.key),
                ));
            }
            let taken = d
                .objectives
                .iter()
                .filter(|o| !matches!(o.state.as_str(), "pending" | "skipped"))
                .count() as u32;
            if taken >= d.cap {
                return Ok((
                    "denied",
                    format!(
                        "Task cap reached ({}); a fresh verified session must take further objectives",
                        d.cap
                    ),
                ));
            }
            let mut i = issue(tx, key)?;
            let skip = if i.version != d.objectives[pos].version {
                Some(format!(
                    "Stale: {key} changed from version {} to {} since dispatch",
                    d.objectives[pos].version, i.version
                ))
            } else if i.status != Status::Ready {
                Some(format!("{key} is no longer Ready"))
            } else if i.claim.as_ref().is_some_and(|c| c.expires_at > at) {
                Some(format!("{key} was claimed by another writer"))
            } else {
                unmet_dependency(tx, &i)?
            };
            if let Some(reason) = skip {
                let o = &mut d.objectives[pos];
                o.state = "skipped".into();
                o.detail = reason.clone();
                return Ok(("denied", reason));
            }
            i.claim = Some(Claim {
                actor: run_actor.clone(),
                expires_at: at + lease,
            });
            i.status = Status::Doing;
            save(tx, i, &run_actor, "work_claimed", at)?;
            let o = &mut d.objectives[pos];
            o.state = "claimed".into();
            o.claim_actor = Some(run_actor);
            Ok(("applied", format!("Claimed {key}")))
        }
        "renew" => {
            let mut i = issue(tx, key)?;
            if state != "claimed" || i.claim.as_ref().is_none_or(|c| c.actor != run_actor) {
                return Ok(("denied", format!("This session does not hold {key}")));
            }
            i.claim.as_mut().unwrap().expires_at = at + lease;
            save(tx, i, &run_actor, "claim_renewed", at)?;
            Ok(("applied", format!("Renewed {key}")))
        }
        "comment" => {
            if state == "pending" || state == "skipped" {
                return Ok(("denied", format!("Claim {key} before commenting on it")));
            }
            let body = field(object, "body").unwrap_or("");
            if body.is_empty() || body.len() > 10_000 {
                return Ok((
                    "denied",
                    "A comment needs a body of at most 10000 characters".into(),
                ));
            }
            add_comment(tx, key, &run_actor, body, at)?;
            emit(tx, &run_actor, "comment_added", key, at)?;
            Ok(("applied", "Comment added".into()))
        }
        "report" => {
            if state != "claimed" {
                return Ok((
                    "denied",
                    format!("{key} is {state}; only a claimed objective is reported"),
                ));
            }
            let outcome = field(object, "outcome").unwrap_or("");
            let summary = field(object, "summary").unwrap_or("");
            if summary.is_empty() || summary.len() > 6_000 {
                return Ok(("denied", "A report needs a summary of at most 6000 characters: criteria, checks and limitations".into()));
            }
            match outcome {
                "pr_opened" => {
                    let branch = field(object, "branch").unwrap_or("");
                    let expected = dispatch_branch(key, &run_id);
                    if branch != expected {
                        return Ok(("denied", format!("Dispatched work is pushed to {expected}")));
                    }
                    let base = field(object, "base_sha").unwrap_or("").to_ascii_lowercase();
                    let head = field(object, "head_sha").unwrap_or("").to_ascii_lowercase();
                    if !hex40(&base) || !hex40(&head) || head == base {
                        return Ok((
                            "denied",
                            "base_sha and head_sha must be different full commit SHAs".into(),
                        ));
                    }
                    if d.base_sha.as_deref().is_some_and(|b| b != base) {
                        return Ok((
                            "denied",
                            "The base must be the commit this session was dispatched at".into(),
                        ));
                    }
                    let pr = field(object, "pr_url").unwrap_or("");
                    let prefix = format!("https://github.com/{}/pull/", d.config.repository);
                    let number = pr
                        .strip_prefix(&prefix)
                        .and_then(|n| n.parse::<u64>().ok())
                        .filter(|n| *n > 0);
                    let Some(number) = number else {
                        return Ok(("denied", format!("The PR must be {prefix}<number>")));
                    };
                    let repository = format!("https://github.com/{}", d.config.repository);
                    let remote_ref = format!("refs/heads/{branch}");
                    for (kind, remote, rref) in [
                        (GitTraceKind::Commit, None, None),
                        (
                            GitTraceKind::Push,
                            Some("origin"),
                            Some(remote_ref.as_str()),
                        ),
                    ] {
                        validate_git_trace_fields(&kind, &repository, &head, branch, remote, rref)?;
                        put_git_trace(
                            tx,
                            &GitTrace {
                                id: id(),
                                issue_key: key.into(),
                                kind,
                                repository: repository.clone(),
                                commit_sha: head.clone(),
                                branch: branch.into(),
                                remote: remote.map(str::to_string),
                                remote_ref: rref.map(str::to_string),
                                recorded_by: run_actor.clone(),
                                recorded_at: at,
                            },
                        )?;
                    }
                    let target = d
                        .config
                        .base_ref
                        .strip_prefix("origin/")
                        .unwrap_or(&d.config.base_ref)
                        .to_string();
                    super::delivery::record(
                        tx,
                        &Command::RecordDeliveryFact {
                            key: key.into(),
                            status: "ok".into(),
                            detail: DeliveryDetail::PullRequest {
                                number,
                                url: pr.into(),
                                target: target.clone(),
                                head_sha: head.clone(),
                            },
                            note: format!("Opened by dispatch session {}", &run_id[..8]),
                        },
                        &run_actor,
                        at,
                    )?;
                    add_comment(
                        tx,
                        key,
                        &run_actor,
                        &format!(
                            "Dispatch handoff: PR {pr} on {branch} (base {base}, tested head {head}, target {target}). The coordinator takes over the claim to integrate, test at the Windows entrypoint and submit.\n\n{summary}"
                        ),
                        at,
                    )?;
                    let o = &mut d.objectives[pos];
                    o.branch = Some(branch.into());
                    o.base_sha = Some(base);
                    o.head_sha = Some(head);
                    o.pr_url = Some(pr.into());
                    o.evidence = summary.into();
                    o.state = "pr_opened".into();
                    Ok(("applied", format!("{key}: PR recorded for the coordinator")))
                }
                "failed" | "blocked" => {
                    add_comment(
                        tx,
                        key,
                        &run_actor,
                        &format!("Dispatch session stopped ({outcome}); the claim stays for the coordinator.\n\n{summary}"),
                        at,
                    )?;
                    let o = &mut d.objectives[pos];
                    o.evidence = summary.into();
                    o.state = outcome.into();
                    Ok(("applied", format!("{key}: {outcome} recorded")))
                }
                _ => Ok((
                    "denied",
                    "outcome must be pr_opened, failed or blocked".into(),
                )),
            }
        }
        _ => unreachable!(),
    }
}

/// A finished session leaves claimed-but-unreported objectives to the coordinator.
pub(crate) fn close(d: &mut RunDispatch) {
    for o in d.objectives.iter_mut().filter(|o| o.state == "claimed") {
        o.state = "unreported".into();
        o.detail =
            "The session stopped before reporting; the claim stays for the coordinator".into();
    }
}

fn dispatch_run(conn: &Connection, id: &str) -> Result<AgentRun> {
    let data: Option<String> = conn
        .query_row("SELECT data FROM agent_runs WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    let r: AgentRun =
        serde_json::from_str(&data.ok_or_else(|| err("not_found", "Unknown agent run"))?)?;
    if r.dispatch.is_none() {
        return Err(err("invalid", "Not a dispatch run"));
    }
    Ok(r)
}

pub(crate) fn mutate(
    tx: &Transaction,
    cmd: &Command,
    actor: &str,
    _role: Role,
    at: i64,
) -> Result<Value> {
    match cmd {
        Command::VerifyDispatchPush {
            id,
            expected_version,
            key,
            verified,
            detail,
        } => {
            let mut r = dispatch_run(tx, id)?;
            if r.version != *expected_version {
                return Err(err("conflict", "The run changed; refresh before retrying"));
            }
            limited(detail, "detail", 1_000)?;
            let d = r.dispatch.as_mut().unwrap();
            let o = d
                .objectives
                .iter_mut()
                .find(|o| o.key == *key && o.state == "pr_opened")
                .ok_or_else(|| err("invalid", "Only a reported PR is verified"))?;
            o.verified_push = Some(*verified);
            o.detail = detail.trim().into();
            r.version += 1;
            r.updated_at = at;
            super::runs::put_run(tx, &r)?;
            emit(tx, actor, "dispatch_push_verified", key, at)?;
            Ok(serde_json::to_value(r)?)
        }
        Command::TakeOverDispatch {
            run_id,
            key,
            expected_version,
            lease_seconds,
        } => {
            check_lease(*lease_seconds)?;
            let mut r = dispatch_run(tx, run_id)?;
            if !r.state.terminal() {
                return Err(err(
                    "conflict",
                    "The dispatch session is still running; wait for it to stop",
                ));
            }
            let run_actor = actor_for(&r.id);
            if actor == run_actor {
                return Err(err("forbidden", "A session cannot take over its own work"));
            }
            let mut i = version(tx, key, *expected_version)?;
            let d = r.dispatch.as_mut().unwrap();
            let o = d
                .objectives
                .iter_mut()
                .find(|o| o.key == *key && o.claim_actor.is_some() && o.taken_over_by.is_none())
                .ok_or_else(|| {
                    err(
                        "invalid",
                        format!("{key} has no dispatched claim to take over"),
                    )
                })?;
            if i.claim.as_ref().is_none_or(|c| c.actor != run_actor) {
                return Err(err(
                    "conflict",
                    "The dispatch claim is no longer held; read current context",
                ));
            }
            i.claim = Some(Claim {
                actor: actor.into(),
                expires_at: at + lease_seconds,
            });
            i.status = Status::Doing;
            o.taken_over_by = Some(actor.into());
            let note = format!(
                "Took over from dispatch session {} ({}): branch {}, PR {}, base {}, tested head {}, push verified {}.",
                &r.id[..8],
                o.state,
                o.branch.as_deref().unwrap_or("none"),
                o.pr_url.as_deref().unwrap_or("none"),
                o.base_sha.as_deref().unwrap_or("none"),
                o.head_sha.as_deref().unwrap_or("none"),
                o.verified_push.map(|v| v.to_string()).unwrap_or_else(|| "not checked".into()),
            );
            add_comment(tx, key, actor, &note, at)?;
            r.version += 1;
            r.updated_at = at;
            super::runs::put_run(tx, &r)?;
            save(tx, i, actor, "dispatch_taken_over", at)
        }
        _ => unreachable!("not a dispatch command"),
    }
}
