//! `direct delivery …`: read-only observation for the delivery ledger (DIR-82).
//!
//! These commands only read: `git status`, `rev-parse`, `merge-base` and
//! `rev-list` in a checkout, SHA-256 of files, and the running service's own
//! build report. They never fetch, reset, merge, push, build, install or
//! restart anything; they record what they saw as delivery facts.

use anyhow::{anyhow, bail, Context, Result};
use clap::Subcommand;
use direct_core::{Artifact, Command, DeliveryDetail, Request, Role};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command as Process,
};

#[derive(Subcommand, Clone)]
pub enum DeliveryAction {
    /// Record a checkout's working tree, HEAD commit and push state.
    Observe {
        #[arg(long)]
        issue: String,
        #[arg(long)]
        repo: PathBuf,
        /// Base branch to measure against, e.g. origin/main.
        #[arg(long, default_value = "origin/main")]
        base: String,
    },
    /// Record a build from artifacts; a Direct binary reports its own stamp.
    ObserveBuild {
        #[arg(long)]
        issue: String,
        #[arg(long = "artifact", required = true)]
        artifacts: Vec<PathBuf>,
        /// Commit the build claims, when no artifact carries a stamp.
        #[arg(long)]
        commit: Option<String>,
    },
    /// Record an installed file against a recorded build.
    ObserveInstall {
        #[arg(long)]
        issue: String,
        #[arg(long)]
        build_fact: String,
        #[arg(long)]
        path: PathBuf,
    },
    /// Record what the running service reports and hash its executable.
    ObserveRunning {
        #[arg(long)]
        issue: String,
    },
}

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let out = Process::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn file_sha256(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("Cannot read {}", path.display()))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// A Direct binary's own build stamp, if the artifact is one.
fn stamp(path: &Path) -> Option<Value> {
    let out = Process::new(path).arg("build-info").output().ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

fn record(
    client: &direct::Client,
    actor: &str,
    issue: &str,
    status: &str,
    detail: DeliveryDetail,
    note: &str,
) -> Result<Value> {
    client.call(
        &Request {
            actor: actor.into(),
            request_id: uuid::Uuid::new_v4().to_string(),
            command: Command::RecordDeliveryFact {
                key: issue.into(),
                status: status.into(),
                detail,
                note: note.into(),
            },
        },
        Role::Agent,
    )
}

pub fn run(client: &direct::Client, actor: &str, action: DeliveryAction) -> Result<()> {
    match action {
        DeliveryAction::Observe { issue, repo, base } => {
            let head = git(&repo, &["rev-parse", "HEAD"])
                .ok_or_else(|| anyhow!("{} is not a git checkout", repo.display()))?;
            let branch =
                git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| "HEAD".into());
            let dirty = git(&repo, &["status", "--porcelain"])
                .map(|s| s.lines().count() as u32)
                .unwrap_or(0);
            let base_sha = git(&repo, &["merge-base", "HEAD", &base]);
            let upstream = git(
                &repo,
                &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
            );
            let unpushed = upstream
                .as_ref()
                .and_then(|_| git(&repo, &["rev-list", "--count", "@{u}..HEAD"]))
                .and_then(|n| n.parse().ok());
            let checkout = fs::canonicalize(&repo)
                .map(|p| crate::roles_cli::plain(&p))
                .unwrap_or_else(|_| repo.display().to_string());
            record(
                client,
                actor,
                &issue,
                "ok",
                DeliveryDetail::WorkingTree {
                    checkout,
                    branch: branch.clone(),
                    base: base_sha.map(|s| format!("{base}@{}", &s[..12.min(s.len())])),
                    head: Some(head.clone()),
                    dirty_files: dirty,
                    unpushed_commits: unpushed,
                },
                "",
            )?;
            println!(
                "working tree: {} dirty file(s), {} unpushed",
                dirty,
                unpushed
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "unknown".into())
            );
            record(
                client,
                actor,
                &issue,
                "ok",
                DeliveryDetail::Commit {
                    sha: head.clone(),
                    branch: branch.clone(),
                },
                "",
            )?;
            println!("commit: {head}");
            // A push is recorded only when the upstream already contains HEAD.
            if let Some(upstream) = upstream {
                let contained = Process::new("git")
                    .arg("-C")
                    .arg(&repo)
                    .args(["merge-base", "--is-ancestor", "HEAD", "@{u}"])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                if contained {
                    let (remote, rest) = upstream
                        .split_once('/')
                        .unwrap_or(("origin", upstream.as_str()));
                    record(
                        client,
                        actor,
                        &issue,
                        "ok",
                        DeliveryDetail::Push {
                            sha: head.clone(),
                            remote: remote.into(),
                            remote_ref: format!("refs/heads/{rest}"),
                        },
                        "Upstream contains HEAD",
                    )?;
                    println!("push: {upstream} contains HEAD");
                } else {
                    println!("push: {upstream} does not contain HEAD; not recorded");
                }
            } else {
                println!("push: no upstream; not recorded");
            }
        }
        DeliveryAction::ObserveBuild {
            issue,
            artifacts,
            commit,
        } => {
            let mut stamped: Option<Value> = None;
            let mut list = Vec::new();
            for path in &artifacts {
                if stamped.is_none() {
                    stamped = stamp(path);
                }
                list.push(Artifact {
                    name: path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    sha256: file_sha256(path)?,
                });
            }
            let commit = commit
                .or_else(|| {
                    stamped
                        .as_ref()
                        .and_then(|s| s["commit"].as_str().map(str::to_string))
                })
                .filter(|c| c != "unknown")
                .ok_or_else(|| anyhow!("No artifact reports its commit; pass --commit"))?;
            let dirty = stamped
                .as_ref()
                .and_then(|s| s["dirty"].as_str().map(str::to_string))
                .unwrap_or_else(|| "unknown".into());
            let fact = record(
                client,
                actor,
                &issue,
                "ok",
                DeliveryDetail::Build {
                    commit: commit.clone(),
                    dirty: dirty.clone(),
                    artifacts: list,
                },
                "",
            )?;
            println!(
                "build {}: commit {commit}, dirty {dirty}",
                fact["id"].as_str().unwrap_or("")
            );
        }
        DeliveryAction::ObserveInstall {
            issue,
            build_fact,
            path,
        } => {
            let path = fs::canonicalize(&path)
                .with_context(|| format!("{} does not exist", path.display()))?;
            let fact = record(
                client,
                actor,
                &issue,
                "ok",
                DeliveryDetail::Install {
                    build_fact_id: build_fact,
                    path: crate::roles_cli::plain(&path),
                    sha256: file_sha256(&path)?,
                },
                "",
            )?;
            println!(
                "install {}: {}",
                fact["id"].as_str().unwrap_or(""),
                crate::roles_cli::plain(&path)
            );
        }
        DeliveryAction::ObserveRunning { issue } => {
            let report = client.build()?;
            let exe = report["executable"].as_str().map(PathBuf::from);
            let Some(exe) = exe else {
                bail!("The service did not report its executable")
            };
            let fact = record(
                client,
                actor,
                &issue,
                "ok",
                DeliveryDetail::Running {
                    path: exe.display().to_string(),
                    sha256: file_sha256(&exe).ok(),
                    service_commit: report["service"]["commit"].as_str().map(str::to_string),
                    service_dirty: report["service"]["dirty"].as_str().map(str::to_string),
                    bundle: report["bundle"]["script"].as_str().map(str::to_string),
                    bundle_commit: report["bundle"]["build"]["commit"]
                        .as_str()
                        .map(str::to_string),
                },
                "",
            )?;
            println!(
                "running {}: service {} (dirty {}), bundle {} from {}",
                fact["id"].as_str().unwrap_or(""),
                report["service"]["commit"].as_str().unwrap_or("?"),
                report["service"]["dirty"].as_str().unwrap_or("?"),
                report["bundle"]["script"].as_str().unwrap_or("?"),
                report["bundle"]["build"]["commit"]
                    .as_str()
                    .unwrap_or("unknown"),
            );
        }
    }
    Ok(())
}
