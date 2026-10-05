//! Delivery ledger: integration and installed build identity (DIR-82).
//!
//! Append-only facts, each with its own observer and time, from worker and
//! working tree through commit, push, PR, integration, check, build, install
//! and the running service. The service rejects facts whose references do not
//! hold (a PR head that was never pushed, an install that matches no built
//! artifact), and no fact implies another. Recording grants no authority.

use super::*;

const TEXT_MAX: usize = 500;

pub(crate) fn put_fact(conn: &Connection, f: &DeliveryFact) -> Result<()> {
    conn.execute(
        "INSERT INTO delivery_facts VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![f.id, serde_json::to_string(f)?],
    )?;
    Ok(())
}

pub(crate) fn for_issue(conn: &Connection, key: &str) -> Result<Vec<DeliveryFact>> {
    // Insertion order is the ledger order: facts observed in the same second
    // must keep the order they were recorded in.
    let mut stmt = conn.prepare("SELECT data FROM delivery_facts ORDER BY rowid")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut facts = Vec::new();
    for row in rows {
        let f: DeliveryFact = serde_json::from_str(&row?)?;
        if f.issue_key == key {
            facts.push(f);
        }
    }
    Ok(facts)
}

fn sha(value: &str, field: &str) -> Result<String> {
    let v = value.trim().to_ascii_lowercase();
    if !valid_commit_sha(&v) {
        return Err(err("invalid", format!("{field} must be a full commit SHA")));
    }
    Ok(v)
}

fn text(value: &str, field: &str) -> Result<String> {
    let v = value.trim();
    required(v, field)?;
    limited(v, field, TEXT_MAX)?;
    Ok(v.to_string())
}

fn kind(d: &DeliveryDetail) -> &'static str {
    match d {
        DeliveryDetail::Worker { .. } => "worker",
        DeliveryDetail::WorkingTree { .. } => "working_tree",
        DeliveryDetail::Commit { .. } => "commit",
        DeliveryDetail::Push { .. } => "push",
        DeliveryDetail::PullRequest { .. } => "pull_request",
        DeliveryDetail::Integration { .. } => "integration",
        DeliveryDetail::Check { .. } => "check",
        DeliveryDetail::Build { .. } => "build",
        DeliveryDetail::Install { .. } => "install",
        DeliveryDetail::Running { .. } => "running",
    }
}

/// Commits the issue has been seen to push: ledger push facts and git traces.
fn pushed(conn: &Connection, key: &str, facts: &[DeliveryFact]) -> Result<Vec<String>> {
    let mut out: Vec<String> = facts
        .iter()
        .filter(|f| f.status == "ok")
        .filter_map(|f| match &f.detail {
            DeliveryDetail::Push { sha, .. } => Some(sha.clone()),
            _ => None,
        })
        .collect();
    out.extend(
        all::<GitTrace>(conn, "git_traces")?
            .into_iter()
            .filter(|t| t.issue_key == key && t.kind == GitTraceKind::Push)
            .map(|t| t.commit_sha.to_ascii_lowercase()),
    );
    Ok(out)
}

fn committed(conn: &Connection, key: &str, facts: &[DeliveryFact]) -> Result<Vec<String>> {
    let mut out = pushed(conn, key, facts)?;
    for f in facts.iter().filter(|f| f.status == "ok") {
        match &f.detail {
            DeliveryDetail::Commit { sha, .. } => out.push(sha.clone()),
            DeliveryDetail::WorkingTree { head: Some(h), .. } => out.push(h.clone()),
            DeliveryDetail::Integration {
                result, sources, ..
            } => {
                out.push(result.clone());
                out.extend(sources.iter().cloned());
            }
            _ => {}
        }
    }
    out.extend(
        all::<GitTrace>(conn, "git_traces")?
            .into_iter()
            .filter(|t| t.issue_key == key)
            .map(|t| t.commit_sha.to_ascii_lowercase()),
    );
    Ok(out)
}

/// Normalize and check one fact against the issue's existing facts.
fn validate(
    conn: &Connection,
    key: &str,
    d: &DeliveryDetail,
    facts: &[DeliveryFact],
) -> Result<DeliveryDetail> {
    Ok(match d {
        DeliveryDetail::Worker {
            host,
            runtime,
            session_id,
            model,
            run_id,
        } => DeliveryDetail::Worker {
            host: text(host, "host")?,
            runtime: text(runtime, "runtime")?,
            session_id: session_id
                .as_deref()
                .map(|s| text(s, "session ID"))
                .transpose()?,
            model: model.as_deref().map(|s| text(s, "model")).transpose()?,
            run_id: run_id.clone(),
        },
        DeliveryDetail::WorkingTree {
            checkout,
            branch,
            base,
            head,
            dirty_files,
            unpushed_commits,
        } => DeliveryDetail::WorkingTree {
            checkout: text(checkout, "checkout")?,
            branch: text(branch, "branch")?,
            base: base.as_deref().map(|s| text(s, "base")).transpose()?,
            head: head.as_deref().map(|s| sha(s, "head")).transpose()?,
            dirty_files: *dirty_files,
            unpushed_commits: *unpushed_commits,
        },
        DeliveryDetail::Commit { sha: s, branch } => DeliveryDetail::Commit {
            sha: sha(s, "commit")?,
            branch: text(branch, "branch")?,
        },
        DeliveryDetail::Push {
            sha: s,
            remote,
            remote_ref,
        } => {
            let s = sha(s, "pushed commit")?;
            if !committed(conn, key, facts)?.contains(&s) {
                return Err(err("invalid", "Record the commit before its push"));
            }
            DeliveryDetail::Push {
                sha: s,
                remote: text(remote, "remote")?,
                remote_ref: text(remote_ref, "remote ref")?,
            }
        }
        DeliveryDetail::PullRequest {
            number,
            url,
            target,
            head_sha,
        } => {
            let head = sha(head_sha, "PR head")?;
            if !pushed(conn, key, facts)?.contains(&head) {
                return Err(err(
                    "invalid",
                    "A PR head must be a pushed commit; record the push first",
                ));
            }
            DeliveryDetail::PullRequest {
                number: *number,
                url: text(url, "PR URL")?,
                target: text(target, "PR target")?,
                head_sha: head,
            }
        }
        DeliveryDetail::Integration {
            method,
            sources,
            result,
            target,
        } => {
            if !["merge", "squash", "rebase"].contains(&method.as_str()) {
                return Err(err(
                    "invalid",
                    "Integration method must be merge, squash or rebase",
                ));
            }
            if sources.is_empty() || sources.len() > 100 {
                return Err(err("invalid", "List the 1–100 source commits"));
            }
            let pushed = pushed(conn, key, facts)?;
            let mut normalized = Vec::new();
            for s in sources {
                let s = sha(s, "source commit")?;
                if !pushed.contains(&s) {
                    return Err(err(
                        "invalid",
                        format!(
                            "Source {} was never pushed; integration cannot be claimed from it",
                            &s[..12]
                        ),
                    ));
                }
                normalized.push(s);
            }
            let result = sha(result, "integration result")?;
            if method != "merge" && normalized.contains(&result) {
                return Err(err(
                    "invalid",
                    "A squash or rebase produces a new commit; record the resulting commit",
                ));
            }
            DeliveryDetail::Integration {
                method: method.clone(),
                sources: normalized,
                result,
                target: text(target, "target")?,
            }
        }
        DeliveryDetail::Check {
            commit,
            name,
            outcome,
        } => {
            let c = sha(commit, "checked commit")?;
            if !committed(conn, key, facts)?.contains(&c) {
                return Err(err(
                    "invalid",
                    "A check must name a commit this issue recorded",
                ));
            }
            if !["passed", "failed"].contains(&outcome.as_str()) {
                return Err(err("invalid", "Check outcome must be passed or failed"));
            }
            DeliveryDetail::Check {
                commit: c,
                name: text(name, "check name")?,
                outcome: outcome.clone(),
            }
        }
        DeliveryDetail::Build {
            commit,
            dirty,
            artifacts,
        } => {
            let c = sha(commit, "built commit")?;
            if !committed(conn, key, facts)?.contains(&c) {
                return Err(err(
                    "invalid",
                    "A build must name a commit this issue recorded",
                ));
            }
            if !["true", "false", "unknown"].contains(&dirty.as_str()) {
                return Err(err("invalid", "Build dirty must be true, false or unknown"));
            }
            if artifacts.is_empty() || artifacts.len() > 20 {
                return Err(err("invalid", "List 1–20 built artifacts"));
            }
            let mut out = Vec::new();
            for a in artifacts {
                if !valid_fingerprint(&a.sha256.to_ascii_lowercase()) {
                    return Err(err("invalid", "Artifact hashes must be SHA-256"));
                }
                out.push(Artifact {
                    name: text(&a.name, "artifact name")?,
                    sha256: a.sha256.to_ascii_lowercase(),
                });
            }
            DeliveryDetail::Build {
                commit: c,
                dirty: dirty.clone(),
                artifacts: out,
            }
        }
        DeliveryDetail::Install {
            build_fact_id,
            path,
            sha256,
        } => {
            let hash = sha256.to_ascii_lowercase();
            let build = facts
                .iter()
                .find(|f| f.id == *build_fact_id)
                .ok_or_else(|| err("invalid", "Install must reference a build of this issue"))?;
            let DeliveryDetail::Build { artifacts, .. } = &build.detail else {
                return Err(err("invalid", "Install must reference a build fact"));
            };
            if build.status != "ok" {
                return Err(err(
                    "invalid",
                    "Install cannot come from a failed or unknown build",
                ));
            }
            if !artifacts.iter().any(|a| a.sha256 == hash) {
                return Err(err(
                    "invalid",
                    "The installed file matches none of the build's artifacts",
                ));
            }
            DeliveryDetail::Install {
                build_fact_id: build_fact_id.clone(),
                path: text(path, "install path")?,
                sha256: hash,
            }
        }
        DeliveryDetail::Running {
            path,
            sha256,
            service_commit,
            service_dirty,
            bundle,
            bundle_commit,
        } => DeliveryDetail::Running {
            path: text(path, "running path")?,
            sha256: sha256.as_ref().map(|h| h.to_ascii_lowercase()),
            service_commit: service_commit.clone(),
            service_dirty: service_dirty.clone(),
            bundle: bundle.clone(),
            bundle_commit: bundle_commit.clone(),
        },
    })
}

pub(crate) fn record(tx: &Transaction, cmd: &Command, actor: &str, at: i64) -> Result<Value> {
    let Command::RecordDeliveryFact {
        key,
        status,
        detail,
        note,
    } = cmd
    else {
        unreachable!()
    };
    issue(tx, key)?;
    if !["ok", "failed", "unknown"].contains(&status.as_str()) {
        return Err(err("invalid", "Status must be ok, failed or unknown"));
    }
    limited(note, "note", 1_000)?;
    let facts = for_issue(tx, key)?;
    let detail = validate(tx, key, detail, &facts)?;
    let f = DeliveryFact {
        id: Uuid::new_v4().to_string(),
        issue_key: key.clone(),
        status: status.clone(),
        detail,
        note: note.trim().into(),
        observed_by: actor.into(),
        observed_at: at,
    };
    put_fact(tx, &f)?;
    emit(tx, actor, &format!("delivery_{}", kind(&f.detail)), key, at)?;
    Ok(serde_json::to_value(f)?)
}

/// The first delivery gap for an issue, with the evidence it rests on and the
/// last known-good install. Facts never imply later stages.
pub fn delivery_state(facts: &[DeliveryFact]) -> Value {
    let latest = |k: &str| facts.iter().rev().find(|f| kind(&f.detail) == k);
    let ok = |f: &&DeliveryFact| f.status == "ok";
    let known_good = facts.iter().rev().filter(ok).find_map(|f| match &f.detail {
        DeliveryDetail::Install { build_fact_id, sha256, path } => {
            let commit = facts.iter().find(|b| b.id == *build_fact_id).and_then(|b| match &b.detail {
                DeliveryDetail::Build { commit, .. } => Some(commit.clone()),
                _ => None,
            });
            Some(json!({"install": f.id, "path": path, "sha256": sha256, "commit": commit, "at": f.observed_at}))
        }
        _ => None,
    });
    let gap = |gap: &str, detail: String| json!({"gap": gap, "detail": detail, "last_known_good": known_good});
    if facts.is_empty() {
        return gap("unknown", "No delivery facts recorded".into());
    }
    if let Some(f) = facts.iter().rev().find(|f| f.status == "failed") {
        let newer_ok = facts.iter().any(|g| {
            kind(&g.detail) == kind(&f.detail) && g.status == "ok" && g.observed_at > f.observed_at
        });
        if !newer_ok {
            return gap(
                "failed",
                format!(
                    "Latest {} failed{}",
                    kind(&f.detail),
                    if f.note.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", f.note)
                    }
                ),
            );
        }
    }
    if let Some(DeliveryDetail::WorkingTree {
        dirty_files,
        branch,
        checkout,
        ..
    }) = latest("working_tree").map(|f| &f.detail)
    {
        if *dirty_files > 0 {
            return gap(
                "unsaved",
                format!("{dirty_files} uncommitted file(s) on {branch} in {checkout}"),
            );
        }
    }
    let head = facts.iter().rev().find_map(|f| match &f.detail {
        DeliveryDetail::Commit { sha, .. } => Some(sha.clone()),
        DeliveryDetail::WorkingTree { head: Some(h), .. } => Some(h.clone()),
        _ => None,
    });
    let pushes: Vec<&String> = facts
        .iter()
        .filter(ok)
        .filter_map(|f| match &f.detail {
            DeliveryDetail::Push { sha, .. } => Some(sha),
            _ => None,
        })
        .collect();
    if let Some(h) = &head {
        if !pushes.contains(&h) {
            return gap(
                "unpushed",
                format!("Commit {} has no recorded push", &h[..12]),
            );
        }
    }
    let integration = facts.iter().rev().filter(ok).find_map(|f| match &f.detail {
        DeliveryDetail::Integration {
            sources,
            result,
            method,
            ..
        } if head
            .as_ref()
            .is_none_or(|h| sources.contains(h) || result == h) =>
        {
            Some((result.clone(), method.clone()))
        }
        _ => None,
    });
    let Some((result, method)) = integration else {
        return gap(
            "unmerged",
            "No integration includes the latest pushed commit".into(),
        );
    };
    let checks: Vec<(&String, &String)> = facts
        .iter()
        .filter(ok)
        .filter_map(|f| match &f.detail {
            DeliveryDetail::Check {
                commit, outcome, ..
            } => Some((commit, outcome)),
            _ => None,
        })
        .collect();
    if !checks.iter().any(|(c, o)| **c == result && *o == "passed") {
        let on_source = checks.iter().any(|(c, o)| **c != result && *o == "passed");
        return gap(
            "unverified",
            if on_source {
                format!(
                    "Checks passed on the source, not on the {method} result {}",
                    &result[..12]
                )
            } else {
                format!(
                    "No passing check on the integration result {}",
                    &result[..12]
                )
            },
        );
    }
    let build =
        facts.iter().rev().filter(ok).find(
            |f| matches!(&f.detail, DeliveryDetail::Build { commit, .. } if *commit == result),
        );
    let Some(build) = build else {
        return gap("unbuilt", format!("No build of {}", &result[..12]));
    };
    if let DeliveryDetail::Build { dirty, .. } = &build.detail {
        if dirty != "false" {
            return gap(
                "unverified",
                format!(
                    "The build of {} is {} — not a clean commit build",
                    &result[..12],
                    if dirty == "true" {
                        "dirty"
                    } else {
                        "of unknown provenance"
                    }
                ),
            );
        }
    }
    let install = facts.iter().rev().filter(ok).find(|f| matches!(&f.detail, DeliveryDetail::Install { build_fact_id, .. } if *build_fact_id == build.id));
    let Some(install) = install else {
        return gap(
            "uninstalled",
            format!("The build of {} is not installed", &result[..12]),
        );
    };
    let Some(running) = latest("running") else {
        return gap(
            "unknown",
            "The running service has not been observed".into(),
        );
    };
    if running.observed_at < install.observed_at {
        return gap(
            "stale",
            "The running observation predates the latest install".into(),
        );
    }
    if let (
        DeliveryDetail::Running {
            sha256,
            service_commit,
            bundle_commit,
            ..
        },
        DeliveryDetail::Install {
            sha256: installed, ..
        },
    ) = (&running.detail, &install.detail)
    {
        if sha256.as_ref().is_some_and(|s| s != installed) {
            return gap(
                "drifted",
                "The running executable differs from the installed one".into(),
            );
        }
        if service_commit.as_ref().is_some_and(|c| *c != result) {
            return gap(
                "drifted",
                format!(
                    "The service reports {} instead of {}",
                    service_commit.as_deref().unwrap_or("?"),
                    &result[..12]
                ),
            );
        }
        if bundle_commit.as_ref().is_some_and(|c| *c != result) {
            return gap(
                "drifted",
                "The served UI bundle was built from a different commit".into(),
            );
        }
    }
    gap(
        "delivered",
        format!(
            "{} integrated, checked, built clean, installed and running",
            &result[..12]
        ),
    )
}

/// An issue's ledger and its computed state, for context.
pub(crate) fn context_for(conn: &Connection, key: &str) -> Result<Value> {
    let facts = for_issue(conn, key)?;
    Ok(json!({"state": delivery_state(&facts), "facts": facts}))
}

pub(crate) fn validate_archive(a: &Archive) -> Result<()> {
    if a.format < 25 && !a.delivery_facts.is_empty() {
        return Err(err("invalid", "Delivery facts require archive format 25"));
    }
    let mut ids = HashSet::new();
    for f in &a.delivery_facts {
        if !ids.insert(f.id.as_str())
            || !a.issues.iter().any(|i| i.key == f.issue_key)
            || !["ok", "failed", "unknown"].contains(&f.status.as_str())
        {
            return Err(err("invalid", "Invalid delivery fact archive"));
        }
        if let DeliveryDetail::Install { build_fact_id, .. } = &f.detail {
            if !a
                .delivery_facts
                .iter()
                .any(|b| b.id == *build_fact_id && matches!(b.detail, DeliveryDetail::Build { .. }))
            {
                return Err(err(
                    "invalid",
                    "Delivery install references a missing build",
                ));
            }
        }
    }
    Ok(())
}
