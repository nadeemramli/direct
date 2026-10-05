//! `direct runs execute|reconcile`: the trusted runner for agent runs (DIR-76).
//!
//! The executor session gets no tools at all (`--safe-mode --tools ""
//! --strict-mcp-config`), so it cannot touch files, the shell, the network,
//! subagents, connectors or Direct. It returns proposals; this runner submits
//! each one to the service under the run's credential, and the service decides
//! what is allowed. The run ID is the harness session ID: a dropped launch is
//! matched to its transcript, never relaunched.

use anyhow::{anyhow, bail, Context, Result};
use clap::Subcommand;
use direct_core::{
    AgentMember, AgentRole, AgentRun, Command, Request, Role, RunDispatch, RunPolicy, RunState,
    SkillPackage, TheoriaDocument,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command as Process, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

#[derive(Subcommand, Clone)]
pub enum RunsAction {
    /// Launch one recorded run intent through its member's harness.
    Execute { run_id: String },
    /// Match in-flight runs whose runner is gone to their harness sessions.
    Reconcile,
}

const GUIDANCE_EXCERPT: usize = 3_000;
const MAX_PROPOSALS: usize = 10;
const QUEUE_PROPOSALS: usize = 40;

fn claude_program() -> String {
    std::env::var("DIRECT_CLAUDE_BIN").unwrap_or_else(|_| {
        if cfg!(windows) {
            "claude.cmd".into()
        } else {
            "claude".into()
        }
    })
}

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

struct Runner<'a> {
    client: &'a direct::Client,
    actor: &'a str,
}

impl Runner<'_> {
    fn call(&self, request_id: &str, command: Command) -> Result<Value> {
        self.client.call(
            &Request {
                actor: self.actor.into(),
                request_id: request_id.into(),
                command,
            },
            Role::Agent,
        )
    }
    fn read(&self, command: Command) -> Result<Value> {
        self.call("", command)
    }
    fn run(&self, id: &str) -> Result<(AgentRun, Value)> {
        let snap = self.read(Command::Snapshot)?;
        let runs: Vec<AgentRun> = serde_json::from_value(snap["agent_runs"].clone())?;
        let run = runs
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| anyhow!("Unknown run {id}"))?;
        Ok((run, snap))
    }
    /// Run a state transition with a fresh request ID and return the new run.
    fn step(&self, command: Command) -> Result<AgentRun> {
        let value = self.call(&uuid::Uuid::new_v4().to_string(), command)?;
        Ok(serde_json::from_value(value)?)
    }
}

/// The bounded packet the executor receives on stdin. Only the assigned
/// issue, its role, skill text and pinned guidance excerpts: no service
/// address, credentials, paths or other records.
pub fn packet(run: &AgentRun, snap: &Value, context: &Value) -> Result<String> {
    let roles: Vec<AgentRole> = serde_json::from_value(snap["agent_roles"].clone())?;
    let skills: Vec<SkillPackage> = serde_json::from_value(snap["skill_packages"].clone())?;
    let documents: Vec<TheoriaDocument> =
        serde_json::from_value(snap["theoria_documents"].clone())?;
    let role = roles
        .iter()
        .find(|r| r.id == run.role_id)
        .ok_or_else(|| anyhow!("The run's role revision is missing"))?;
    let issue = &context["issue"];
    let mut out = format!(
        "You are working as the Direct role `{}` (revision {}) on one issue. You have no tools. \
Reply with a single JSON object and nothing else.\n\n# Objective\n\n{}\n\n# Issue {} (version {}, status {})\n\nTitle: {}\n\n## Brief\n\n{}\n\n## Acceptance\n\n{}\n\n# Role\n\nResponsibilities:\n",
        role.key,
        role.revision,
        run.objective,
        run.issue_key,
        issue["version"],
        issue["status"].as_str().unwrap_or(""),
        issue["title"].as_str().unwrap_or(""),
        issue["body"].as_str().unwrap_or(""),
        issue["acceptance"].as_str().unwrap_or(""),
    );
    for r in &role.responsibilities {
        out += &format!("- {r}\n");
    }
    out += &format!("\nOwner direction: {}\n", role.owner_direction);
    for id in &role.skills {
        if let Some(s) = skills.iter().find(|s| s.id == *id) {
            for f in s.files.iter().filter(|f| f.path == "SKILL.md") {
                out += &format!(
                    "\n# Skill {} (revision {})\n\n{}\n",
                    s.name, s.revision, f.content
                );
            }
        }
    }
    out += "\n# Pinned guidance (excerpts from the Development Operating System)\n";
    for pin in &role.guidance {
        let doc = documents.iter().find(|d| d.id == pin.document_id);
        let excerpt: String = doc
            .and_then(|d| d.content.as_deref())
            .unwrap_or("Unavailable")
            .chars()
            .take(GUIDANCE_EXCERPT)
            .collect();
        out += &format!(
            "\n## {} (fingerprint {})\n\n{}\n",
            doc.map(|d| d.title.as_str()).unwrap_or(&pin.document_id),
            pin.recorded_fingerprint.as_deref().unwrap_or("unavailable"),
            excerpt
        );
    }
    out += &format!(
        "\n# Reply format\n\nReply with only this JSON object:\n\
{{\"summary\": \"one or two sentences\", \"proposals\": [ ... ]}}\n\
Each proposal is one of:\n\
- {{\"op\": \"comment\", \"body\": \"text\"}} to comment on {key}\n\
- {{\"op\": \"update_backlog\", \"body\": \"new brief\", \"acceptance\": \"new acceptance\"}} to rewrite {key}'s brief or acceptance (only while it is in Backlog; either field may be omitted)\n\
At most {MAX_PROPOSALS} proposals. Direct applies only these, only to {key}; anything else is refused.\n",
        key = run.issue_key
    );
    Ok(out)
}

/// The bounded packet for a Product Planner queue review (DIR-77): each scoped
/// issue's current text, original intake, links and recent comments, plus the
/// role, skill, pinned guidance and routing rules. No service address,
/// credentials, paths or unscoped records.
pub fn queue_packet(run: &AgentRun, snap: &Value, contexts: &[Value]) -> Result<String> {
    let queue = run
        .queue
        .as_ref()
        .ok_or_else(|| anyhow!("Not a queue review"))?;
    let roles: Vec<AgentRole> = serde_json::from_value(snap["agent_roles"].clone())?;
    let skills: Vec<SkillPackage> = serde_json::from_value(snap["skill_packages"].clone())?;
    let documents: Vec<TheoriaDocument> =
        serde_json::from_value(snap["theoria_documents"].clone())?;
    let role = roles
        .iter()
        .find(|r| r.id == run.role_id)
        .ok_or_else(|| anyhow!("The run's role revision is missing"))?;
    let writes = match queue.policy {
        RunPolicy::InspectOnly => "This review is INSPECT-ONLY: propose findings only; any issue write is refused.",
        RunPolicy::RefineBacklog => "This review may REFINE BACKLOG: besides findings, you may comment on scoped issues and rewrite the brief/acceptance of scoped Backlog issues (include the expected_version you were shown; Direct preserves the previous text).",
        RunPolicy::DispatchReady => bail!("A queue review never dispatches implementation"),
    };
    let mut out = format!(
        "You are working as the Direct role `{}` (revision {}) reviewing a queue of {} issues. You have no tools. \
Reply with a single JSON object and nothing else.\n\n# Objective\n\n{}\n\n{writes}\n\n# Role\n\nResponsibilities:\n",
        role.key, role.revision, run.issue_key, run.objective
    );
    for r in &role.responsibilities {
        out += &format!("- {r}\n");
    }
    out += &format!("\nOwner direction: {}\n", role.owner_direction);
    for id in &role.skills {
        if let Some(s) = skills.iter().find(|s| s.id == *id) {
            for f in s.files.iter().filter(|f| f.path == "SKILL.md") {
                out += &format!(
                    "\n# Skill {} (revision {})\n\n{}\n",
                    s.name, s.revision, f.content
                );
            }
        }
    }
    out += "\n# Pinned guidance (excerpts from the Development Operating System)\n";
    for pin in &role.guidance {
        let doc = documents.iter().find(|d| d.id == pin.document_id);
        let excerpt: String = doc
            .and_then(|d| d.content.as_deref())
            .unwrap_or("Unavailable")
            .chars()
            .take(GUIDANCE_EXCERPT)
            .collect();
        out += &format!(
            "\n## {} (fingerprint {})\n\n{}\n",
            doc.map(|d| d.title.as_str()).unwrap_or(&pin.document_id),
            pin.recorded_fingerprint.as_deref().unwrap_or("unavailable"),
            excerpt
        );
    }
    out += "\n# Queue\n";
    for c in contexts {
        let i = &c["issue"];
        let key = i["key"].as_str().unwrap_or("");
        out += &format!(
            "\n## {key} (version {}, status {})\n\nTitle: {}\n\nBrief:\n{}\n\nAcceptance:\n{}\n",
            i["version"],
            i["status"].as_str().unwrap_or(""),
            i["title"].as_str().unwrap_or(""),
            i["body"].as_str().unwrap_or(""),
            i["acceptance"].as_str().unwrap_or(""),
        );
        if let Some(intake) = i["intake"]["text"]
            .as_str()
            .filter(|t| !t.trim().is_empty())
        {
            let text: String = intake.chars().take(2_000).collect();
            out += &format!("\nOriginal intake (preserved, never rewrite it):\n{text}\n");
        }
        let links: Vec<String> = c["issue_links"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|l| {
                        format!(
                            "{} {} {}",
                            l["source_key"].as_str().unwrap_or(""),
                            l["kind"].as_str().unwrap_or(""),
                            l["target_key"].as_str().unwrap_or("")
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !links.is_empty() {
            out += &format!("\nLinks: {}\n", links.join("; "));
        }
        if let Some(comments) = c["comments"].as_array() {
            for comment in comments.iter().rev().take(3) {
                let body: String = comment["body"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(400)
                    .collect();
                out += &format!(
                    "\nRecent comment by {}: {body}\n",
                    comment["actor"].as_str().unwrap_or("")
                );
            }
        }
    }
    if !queue.blocked.is_empty() {
        out += "\n# Excluded from this review (do not propose anything for these)\n";
        for b in &queue.blocked {
            out += &format!("- {}: {}\n", b.key, b.reason);
        }
    }
    out += &format!(
        "\n# Routing\n\nKnown work gets a bounded brief. Uncertain but reversible work gets a DOS v5 discovery proposal (route `discovery`). \
Consequential work (security, auth, payments, destructive or irreversible changes, privacy) gets reviewed-design scope (route `reviewed_design`). \
Do not invent busywork: a clear, well-scoped issue needs no finding.\n\n# Reply format\n\n\
Reply with only this JSON object: {{\"summary\": \"one or two sentences\", \"proposals\": [ ... ]}}\nEach proposal is one of:\n\
- {{\"op\": \"finding\", \"issue_key\": \"KEY\", \"kind\": \"unclear_outcome|oversized|missing_criteria|duplicate|dependency|route_mismatch\", \"route\": \"bounded_brief|discovery|reviewed_design\", \"summary\": \"...\", \"evidence\": [\"what in the issue shows it\"], \"recommendation\": \"...\", \"uncertain\": false}}\n\
  Add \"escalation\": {{\"criterion\": \"...\", \"evidence\": \"...\", \"impact\": \"...\", \"options\": [\"...\"], \"recommendation\": \"...\"}} when route is reviewed_design or uncertain is true.\n\
- {{\"op\": \"comment\", \"issue_key\": \"KEY\", \"body\": \"...\"}}\n\
- {{\"op\": \"update_backlog\", \"issue_key\": \"KEY\", \"expected_version\": N, \"body\": \"...\", \"acceptance\": \"...\"}} (either text field may be omitted)\n\
At most {QUEUE_PROPOSALS} proposals, only for the issues in the queue above. Never mark work Ready, reopen, merge or delete duplicates, or rewrite submitted or completed scope.\n"
    );
    Ok(out)
}

fn git(dir: &Path, args: &[&str]) -> std::result::Result<String, String> {
    let out = Process::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git is unavailable: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Fetch the base and add a fresh detached worktree for a dispatch run.
/// Read-only towards the remote; nothing existing is reset or deleted.
fn prepare_worktree(
    run: &AgentRun,
    d: &RunDispatch,
) -> std::result::Result<(String, String), String> {
    let checkout = PathBuf::from(d.config.checkout.as_deref().unwrap_or_default());
    if git(&checkout, &["rev-parse", "--git-dir"]).is_err() {
        return Err(format!("{} is not a git checkout", checkout.display()));
    }
    if let Some((remote, _)) = d.config.base_ref.split_once('/') {
        if git(&checkout, &["remote"])?.lines().any(|r| r == remote) {
            git(&checkout, &["fetch", "--quiet", remote])?;
        }
    }
    let base = git(
        &checkout,
        &["rev-parse", &format!("{}^{{commit}}", d.config.base_ref)],
    )?;
    let path = Path::new(&d.config.worktree_root).join(format!(
        "{}-{}",
        run.issue_key.to_ascii_lowercase(),
        &run.id[..8]
    ));
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    fs::create_dir_all(&d.config.worktree_root).map_err(|e| e.to_string())?;
    git(
        &checkout,
        &[
            "worktree",
            "add",
            "--detach",
            &path.to_string_lossy(),
            &base,
        ],
    )?;
    Ok((path.to_string_lossy().into_owned(), base))
}

/// The tool rules for a dispatched session: file tools (confined to the
/// worktree by `--restricted`), git on its own branches, PR creation, Direct
/// actions and the owner's extra command prefixes. Everything else is denied.
pub fn dispatch_tool_rules(
    run: &AgentRun,
    d: &RunDispatch,
    base: &str,
) -> (Vec<String>, Vec<String>) {
    let mut allow: Vec<String> = ["Read", "Glob", "Grep", "Edit", "Write"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for prefix in [
        "git status",
        "git diff",
        "git log",
        "git show",
        "git add",
        "git commit",
        "git rev-parse",
        "gh pr create",
        "gh pr view",
        "gh pr checks",
        "direct run-action",
    ] {
        allow.push(format!("Bash({prefix}:*)"));
    }
    for o in &d.objectives {
        let branch = direct_core::dispatch_branch(&o.key, &run.id);
        allow.push(format!("Bash(git switch -c {branch} {base})"));
        allow.push(format!("Bash(git switch {branch})"));
        allow.push(format!("Bash(git push -u origin {branch})"));
        allow.push(format!("Bash(git push origin {branch})"));
    }
    for prefix in &d.config.allow_commands {
        allow.push(format!("Bash({prefix}:*)"));
    }
    let deny = [
        "Bash(git push --force:*)",
        "Bash(git push -f:*)",
        "Bash(git reset:*)",
        "Bash(git clean:*)",
        "Bash(git branch -D:*)",
        "Bash(git rebase:*)",
        "Bash(gh pr merge:*)",
        "Bash(gh api:*)",
        "Bash(curl:*)",
        "WebFetch",
        "WebSearch",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    (allow, deny)
}

/// The packet a dispatched implementation session receives on stdin: its
/// authority, cap, objectives with criteria and guidance pins, the previous
/// session's handoff and the exact workflow. No service address, data paths,
/// credentials or unrelated records.
pub fn dispatch_packet(
    run: &AgentRun,
    snap: &Value,
    contexts: &[Value],
    base: &str,
) -> Result<String> {
    let d = run
        .dispatch
        .as_ref()
        .ok_or_else(|| anyhow!("Not a dispatch run"))?;
    let roles: Vec<AgentRole> = serde_json::from_value(snap["agent_roles"].clone())?;
    let skills: Vec<SkillPackage> = serde_json::from_value(snap["skill_packages"].clone())?;
    let documents: Vec<TheoriaDocument> =
        serde_json::from_value(snap["theoria_documents"].clone())?;
    let runs: Vec<AgentRun> = serde_json::from_value(snap["agent_runs"].clone())?;
    let role = roles
        .iter()
        .find(|r| r.id == run.role_id)
        .ok_or_else(|| anyhow!("The run's role revision is missing"))?;
    let target = d
        .config
        .base_ref
        .strip_prefix("origin/")
        .unwrap_or(&d.config.base_ref);
    let mut out = format!(
        "You are an implementation agent dispatched by Direct as the role `{}` (revision {}) for product {}. \
You work only in the current directory: a fresh git worktree of {} detached at base {base} ({}).\n\n\
# Owner objective\n\n{}\n\n# Authority\n\n\
You may claim your objectives one at a time, implement them, run the project's checks, commit, push each objective's own branch and open a PR, then report to Direct. \
You may NOT merge, deliver or install builds, restart services, submit for review, change other branches, rewrite history or touch anything outside this worktree. \
Commands outside your allowlist are refused automatically; do not try to work around a refusal. \
This session takes at most {} objectives; anything further goes to a fresh session.\n\n\
# Talking to Direct\n\n\
Run `direct run-action --json '<json>'`. Your session credential is already in the environment; never print or copy it. Each call prints the outcome (applied or denied) and why.\n\
- Claim: {{\"op\":\"claim\",\"key\":\"KEY\"}} (denied if the issue changed, is no longer Ready or has an unmet dependency: then move on)\n\
- Renew during long work: {{\"op\":\"renew\",\"key\":\"KEY\"}}\n\
- Comment: {{\"op\":\"comment\",\"key\":\"KEY\",\"body\":\"...\"}}\n\
- Report: {{\"op\":\"report\",\"key\":\"KEY\",\"outcome\":\"pr_opened\",\"branch\":\"BRANCH\",\"base_sha\":\"{base}\",\"head_sha\":\"<git rev-parse HEAD>\",\"pr_url\":\"https://github.com/{}/pull/N\",\"summary\":\"each criterion -> evidence, checks run with results, limitations\"}}\n\
  or outcome failed / blocked with the reason in summary.\n\n\
# Workflow, for each objective in order\n\n\
1. Claim it.\n2. `git switch -c BRANCH {base}` with the branch named below.\n\
3. Read the repository's AGENTS.md and docs it points to; implement the acceptance criteria.\n\
4. Run the project's checks that your allowlist permits and fix failures.\n\
5. `git add` and `git commit -m \"KEY: ...\"`.\n6. `git push -u origin BRANCH`.\n\
7. `gh pr create --base {target} --head BRANCH --title \"KEY: ...\" --body \"...\"`.\n\
8. Report. Then continue with the next objective, or stop when none remain or the cap is reached.\n\n\
Finish with a short plain-text summary of what you did.\n\n# Role\n\nResponsibilities:\n",
        role.key,
        role.revision,
        run.issue_key,
        d.config.repository,
        d.config.base_ref,
        run.objective,
        d.cap,
        d.config.repository,
    );
    for r in &role.responsibilities {
        out += &format!("- {r}\n");
    }
    out += &format!("\nOwner direction: {}\n", role.owner_direction);
    for id in &role.skills {
        if let Some(s) = skills.iter().find(|s| s.id == *id) {
            for f in s.files.iter().filter(|f| f.path == "SKILL.md") {
                out += &format!(
                    "\n# Skill {} (revision {})\n\n{}\n",
                    s.name, s.revision, f.content
                );
            }
        }
    }
    out += "\n# Objectives\n";
    for (o, c) in d.objectives.iter().zip(contexts) {
        let i = &c["issue"];
        out += &format!(
            "\n## {} (version {}) — branch `{}`\n\nTitle: {}\n\nBrief:\n{}\n\nAcceptance:\n{}\n",
            o.key,
            o.version,
            direct_core::dispatch_branch(&o.key, &run.id),
            i["title"].as_str().unwrap_or(""),
            i["body"].as_str().unwrap_or(""),
            i["acceptance"].as_str().unwrap_or(""),
        );
        if o.review_repair {
            out += "\nThis is a REVIEW REPAIR: the owner requested changes. Read the latest review feedback in the comments first.\n";
        }
        if let Some(comments) = c["comments"].as_array() {
            for comment in comments.iter().rev().take(4) {
                let body: String = comment["body"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(800)
                    .collect();
                out += &format!(
                    "\nRecent comment by {}: {body}\n",
                    comment["actor"].as_str().unwrap_or("")
                );
            }
        }
        for pin in &o.guidance {
            let doc = documents.iter().find(|x| x.id == pin.document_id);
            let excerpt: String = doc
                .and_then(|x| x.content.as_deref())
                .unwrap_or("Unavailable")
                .chars()
                .take(GUIDANCE_EXCERPT)
                .collect();
            out += &format!(
                "\n### Guidance {} (fingerprint {}{})\n\n{}\n",
                doc.map(|x| x.title.as_str()).unwrap_or(&pin.document_id),
                pin.recorded_fingerprint.as_deref().unwrap_or("unpinned"),
                if pin.shared { ", shared" } else { "" },
                excerpt
            );
        }
    }
    if let Some(previous) = d
        .handoff_from
        .as_deref()
        .and_then(|id| runs.iter().find(|r| r.id == id))
        .and_then(|r| r.dispatch.as_ref().map(|pd| (r, pd)))
    {
        out += &format!(
            "\n# Handoff from the previous session {} ({:?})\n\n",
            &previous.0.id[..8],
            previous.0.state
        );
        for o in &previous.1.objectives {
            out += &format!(
                "- {}: {} branch {} PR {} base {} head {}{}\n",
                o.key,
                o.state,
                o.branch.as_deref().unwrap_or("-"),
                o.pr_url.as_deref().unwrap_or("-"),
                o.base_sha.as_deref().unwrap_or("-"),
                o.head_sha.as_deref().unwrap_or("-"),
                o.taken_over_by
                    .as_deref()
                    .map(|a| format!(", now held by {a}"))
                    .unwrap_or_default()
            );
        }
    }
    if !d.excluded.is_empty() {
        out += "\n# Not dispatched (do not work on these)\n";
        for b in &d.excluded {
            out += &format!("- {}: {}\n", b.key, b.reason);
        }
    }
    Ok(out)
}

/// The JSON object inside the executor's final text, fenced or bare.
pub fn parse_reply(text: &str, max: usize) -> Option<(String, Vec<Value>)> {
    let trimmed = text.trim();
    let body = trimmed
        .split("```json")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .unwrap_or(trimmed);
    let value: Value = serde_json::from_str(body.trim()).ok()?;
    let proposals = value["proposals"].as_array()?.clone();
    if proposals.len() > max {
        return None;
    }
    Some((
        value["summary"].as_str().unwrap_or("").to_string(),
        proposals,
    ))
}

fn kill_tree(child: &mut Child) {
    if cfg!(windows) {
        let _ = Process::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

fn pid_alive(pid: u32) -> bool {
    if cfg!(windows) {
        Process::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    } else {
        Path::new(&format!("/proc/{pid}")).exists()
    }
}

/// Submit proposals not yet recorded on the run, under `credential`.
fn submit(
    runner: &Runner,
    run: &AgentRun,
    credential: &str,
    proposals: &[Value],
) -> Result<(usize, usize, usize)> {
    let (mut applied, mut retained, mut denied) = (0, 0, 0);
    for (index, proposal) in proposals.iter().enumerate() {
        if run.actions.iter().any(|a| a.index as usize == index) {
            continue;
        }
        let request_id = format!("run-{}-action-{index}", run.id);
        let command = Command::RunAction {
            run_id: run.id.clone(),
            credential: credential.into(),
            index: index as u32,
            proposal: proposal.clone(),
        };
        // A dropped response retries the identical request; the service replays it.
        let mut outcome = Err(anyhow!("not sent"));
        for _ in 0..3 {
            outcome = runner.call(&request_id, command.clone());
            if outcome.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        match outcome?["outcome"].as_str() {
            Some("applied") => applied += 1,
            Some("retained") => retained += 1,
            _ => denied += 1,
        }
    }
    Ok((applied, retained, denied))
}

fn execute(runner: &Runner, data_dir: &Path, run_id: &str) -> Result<()> {
    let (run, snap) = runner.run(run_id)?;
    if run.state != RunState::Intent {
        bail!(
            "Run {run_id} is {:?}; only an intent is launched, once",
            run.state
        );
    }
    let members: Vec<AgentMember> = serde_json::from_value(snap["agent_members"].clone())?;
    let member = members.iter().find(|m| m.id == run.member_id);
    let program = claude_program();
    let version = Process::new(&program)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|v| !v.is_empty());
    let channel = match (member.map(|m| m.runtime.as_str()), version) {
        (Some("claude-code"), Some(v)) => Ok(v),
        (Some("claude-code"), None) => {
            Err("The Claude Code channel is unavailable on this machine".to_string())
        }
        (Some(other), _) => Err(format!("No supported adapter for {other}")),
        (None, _) => Err("The run's member is missing".to_string()),
    };
    let version = match channel {
        Ok(v) => v,
        Err(reason) => {
            runner.step(Command::BlockAgentRun {
                id: run.id.clone(),
                expected_version: run.version,
                reason: reason.clone(),
            })?;
            bail!("Blocked, not dispatched: {reason}");
        }
    };
    let (mut worktree, mut base) = (None, None);
    let input = match (&run.queue, &run.dispatch) {
        (Some(queue), _) => {
            let mut contexts = Vec::new();
            for key in &queue.keys {
                contexts.push(runner.read(Command::Context { key: key.clone() })?);
            }
            queue_packet(&run, &snap, &contexts)?
        }
        (None, Some(d)) => {
            let (path, sha) = match prepare_worktree(&run, d) {
                Ok(prepared) => prepared,
                Err(reason) => {
                    let reason = format!("The dispatch worktree could not be prepared: {reason}");
                    runner.step(Command::BlockAgentRun {
                        id: run.id.clone(),
                        expected_version: run.version,
                        reason: reason.chars().take(1_000).collect(),
                    })?;
                    bail!("Blocked, not dispatched: {reason}");
                }
            };
            let mut contexts = Vec::new();
            for o in &d.objectives {
                contexts.push(runner.read(Command::Context { key: o.key.clone() })?);
            }
            let text = dispatch_packet(&run, &snap, &contexts, &sha)?;
            worktree = Some(path);
            base = Some(sha);
            text
        }
        (None, None) => {
            let context = runner.read(Command::Context {
                key: run.issue_key.clone(),
            })?;
            packet(&run, &snap, &context)?
        }
    };
    let credential = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    let run = runner.step(Command::StartAgentRun {
        id: run.id.clone(),
        expected_version: run.version,
        launcher: format!("pid {} on {}", std::process::id(), hostname()),
        harness_version: version,
        credential_sha256: sha(&credential),
        worktree: worktree.clone(),
        base_sha: base.clone(),
    })?;

    let mut command = Process::new(&program);
    match (&run.dispatch, &worktree, &base) {
        (Some(d), Some(path), Some(base)) => {
            // A write-capable session, confined by Claude Code's restricted mode,
            // dontAsk permissions and an explicit allowlist (DIR-79).
            let (allow, deny) = dispatch_tool_rules(&run, d, base);
            command
                .args([
                    "-p",
                    "--restricted",
                    "--tools",
                    "Read,Edit,Write,Glob,Grep,Bash",
                    "--strict-mcp-config",
                    "--permission-mode",
                    "dontAsk",
                    "--permission-prompts",
                    "none",
                    "--allowedTools",
                ])
                .args(&allow)
                .arg("--disallowedTools")
                .args(&deny)
                .args([
                    "--session-id",
                    &run.id,
                    "--model",
                    &run.requested_model,
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--max-turns",
                    "500",
                ])
                .current_dir(path)
                .env("DIRECT_RUN_ID", &run.id)
                .env("DIRECT_RUN_CREDENTIAL", &credential)
                .env("DIRECT_DATA_DIR", data_dir);
            if let Some(dir) = std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(Path::to_path_buf))
            {
                let path = std::env::var_os("PATH").unwrap_or_default();
                let mut paths = vec![dir];
                paths.extend(std::env::split_paths(&path));
                command.env("PATH", std::env::join_paths(paths)?);
            }
        }
        _ => {
            let workdir = data_dir.join("runs").join(&run.id);
            fs::create_dir_all(&workdir)?;
            command
                .args([
                    "-p",
                    "--safe-mode",
                    "--tools",
                    "",
                    "--strict-mcp-config",
                    "--session-id",
                    &run.id,
                    "--model",
                    &run.requested_model,
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--max-turns",
                    "1",
                ])
                .current_dir(&workdir);
        }
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Could not start the Claude Code harness")?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input.as_bytes())?;
    let stdout = child.stdout.take().expect("piped stdout");
    let child = Arc::new(Mutex::new(child));

    // Watch for an owner cancel, and the run's time limit, while the harness runs.
    let done = Arc::new(AtomicBool::new(false));
    let canceled = Arc::new(AtomicBool::new(false));
    let timed_out = Arc::new(AtomicBool::new(false));
    let deadline = run
        .max_seconds
        .map(|s| std::time::Instant::now() + Duration::from_secs(s));
    let watcher = {
        let (done, canceled, child) = (done.clone(), canceled.clone(), child.clone());
        let timed_out = timed_out.clone();
        let client = runner.client.clone();
        let (actor, id) = (runner.actor.to_string(), run.id.clone());
        std::thread::spawn(move || {
            let runner = Runner {
                client: &client,
                actor: &actor,
            };
            while !done.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(2));
                if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                    timed_out.store(true, Ordering::SeqCst);
                    kill_tree(&mut child.lock().unwrap());
                    return;
                }
                if let Ok((r, _)) = runner.run(&id) {
                    if r.state == RunState::CancelPending {
                        canceled.store(true, Ordering::SeqCst);
                        kill_tree(&mut child.lock().unwrap());
                        return;
                    }
                }
            }
        })
    };

    let mut run = run;
    let mut final_text: Option<(String, bool)> = None;
    let mut cost: Option<f64> = None;
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if event["type"] == "system"
            && event["subtype"] == "init"
            && run.state == RunState::Launching
        {
            run = runner.step(Command::RecordRunSession {
                id: run.id.clone(),
                expected_version: run.version,
                session_id: event["session_id"].as_str().unwrap_or("").into(),
                actual_model: event["model"].as_str().unwrap_or("").into(),
            })?;
            if run.state != RunState::Running {
                // Model gate: stop before any output is used.
                kill_tree(&mut child.lock().unwrap());
                break;
            }
        }
        if event["type"] == "result" {
            cost = event["total_cost_usd"].as_f64();
            final_text = Some((
                event["result"].as_str().unwrap_or("").to_string(),
                event["is_error"].as_bool().unwrap_or(true),
            ));
        }
    }
    done.store(true, Ordering::SeqCst);
    let _ = child.lock().unwrap().wait();
    let _ = watcher.join();

    let (run, _) = runner.run(&run.id)?;
    if canceled.load(Ordering::SeqCst) || run.state == RunState::CancelPending {
        runner.step(Command::AcknowledgeRunCancel {
            id: run.id.clone(),
            expected_version: run.version,
        })?;
        println!("Run {} canceled", run.id);
        return Ok(());
    }
    if matches!(run.state, RunState::Running) {
        // Truthful cost: what the harness reported, or unknown.
        runner.step(Command::RecordRunCost {
            id: run.id.clone(),
            cost_usd: cost,
        })?;
    }
    let (run, _) = runner.run(&run.id)?;
    if timed_out.load(Ordering::SeqCst) && run.state == RunState::Running {
        let minutes = run.max_seconds.unwrap_or(0) / 60;
        runner.step(Command::FinishAgentRun {
            id: run.id.clone(),
            expected_version: run.version,
            succeeded: false,
            summary: format!(
                "Stopped at the {minutes}-minute time limit; nothing further was applied"
            ),
        })?;
        bail!("Run {} hit its time limit", run.id);
    }
    match run.state {
        RunState::Running => {}
        RunState::Launching => {
            runner.step(Command::MarkAgentRunUnknown {
                id: run.id.clone(),
                expected_version: run.version,
                reason: "The harness exited without reporting a session".into(),
            })?;
            bail!("Run {} outcome unknown", run.id);
        }
        other => {
            println!(
                "Run {} ended {:?}: {}",
                run.id,
                other,
                run.reason.unwrap_or_default()
            );
            return Ok(());
        }
    }
    if run.dispatch.is_some() {
        return finish_dispatch(runner, &run, final_text);
    }
    finish(runner, &run, &credential, final_text)
}

/// Close a dispatch session: verify each reported push with read-only git,
/// then finish with the session's own summary. Claims stay for the coordinator.
fn finish_dispatch(
    runner: &Runner,
    run: &AgentRun,
    final_text: Option<(String, bool)>,
) -> Result<()> {
    let (mut run, _) = runner.run(&run.id)?;
    let d = run.dispatch.clone().expect("dispatch run");
    let checkout = PathBuf::from(d.config.checkout.as_deref().unwrap_or_default());
    for o in d.objectives.iter().filter(|o| o.state == "pr_opened") {
        let branch = o.branch.clone().unwrap_or_default();
        let (verified, detail) = match git(
            &checkout,
            &["ls-remote", "origin", &format!("refs/heads/{branch}")],
        ) {
            Ok(out) => {
                let remote = out.split_whitespace().next().unwrap_or("").to_string();
                let head = o.head_sha.clone().unwrap_or_default();
                if remote == head {
                    (true, format!("origin/{branch} is at {head}"))
                } else if remote.is_empty() {
                    (false, format!("origin/{branch} does not exist"))
                } else {
                    (
                        false,
                        format!("origin/{branch} is at {remote}, not the reported {head}"),
                    )
                }
            }
            Err(e) => (false, format!("Remote check failed: {e}")),
        };
        run = runner.step(Command::VerifyDispatchPush {
            id: run.id.clone(),
            expected_version: run.version,
            key: o.key.clone(),
            verified,
            detail,
        })?;
    }
    let (summary, ok) = match final_text {
        Some((text, error)) => (text, !error),
        None => ("The session ended without a final message".into(), false),
    };
    let summary: String = summary.trim().chars().take(4_000).collect();
    runner.step(Command::FinishAgentRun {
        id: run.id.clone(),
        expected_version: run.version,
        succeeded: ok,
        summary: summary.clone(),
    })?;
    println!("Dispatch run {} finished: {summary}", run.id);
    Ok(())
}

/// `direct run-action`: one action from inside a dispatched session.
pub fn session_action(
    client: &direct::Client,
    json: Option<&str>,
    file: Option<&Path>,
) -> Result<()> {
    let run_id = std::env::var("DIRECT_RUN_ID").map_err(|_| {
        anyhow!("DIRECT_RUN_ID is not set: run-action only works inside a dispatched session")
    })?;
    let credential = std::env::var("DIRECT_RUN_CREDENTIAL")
        .map_err(|_| anyhow!("DIRECT_RUN_CREDENTIAL is not set"))?;
    let text = match (json, file) {
        (Some(j), None) => j.to_string(),
        (None, Some(f)) => fs::read_to_string(f)?,
        _ => bail!("Pass exactly one of --json or --file"),
    };
    let proposal: Value =
        serde_json::from_str(&text).context("The action must be a JSON object")?;
    let actor = format!("run:{run_id}");
    let runner = Runner {
        client,
        actor: &actor,
    };
    for _ in 0..3 {
        let (run, _) = runner.run(&run_id)?;
        let index = run.actions.iter().map(|a| a.index + 1).max().unwrap_or(0);
        let result = runner.call(
            &format!("run-{run_id}-action-{index}"),
            Command::RunAction {
                run_id: run_id.clone(),
                credential: credential.clone(),
                index,
                proposal: proposal.clone(),
            },
        );
        match result {
            Ok(v) => {
                println!(
                    "{}: {}",
                    v["outcome"].as_str().unwrap_or("?"),
                    v["detail"].as_str().unwrap_or("")
                );
                return Ok(());
            }
            // Another action took this index first: take the next one.
            Err(e) if e.to_string().contains("already submitted") => continue,
            Err(e) => return Err(e),
        }
    }
    bail!("Could not record the action; try again")
}

fn finish(
    runner: &Runner,
    run: &AgentRun,
    credential: &str,
    final_text: Option<(String, bool)>,
) -> Result<()> {
    let parsed = final_text
        .as_ref()
        .filter(|(_, error)| !error)
        .and_then(|(text, _)| {
            parse_reply(
                text,
                if run.queue.is_some() {
                    QUEUE_PROPOSALS
                } else {
                    MAX_PROPOSALS
                },
            )
        });
    let Some((summary, proposals)) = parsed else {
        let (run, _) = runner.run(&run.id)?;
        runner.step(Command::FinishAgentRun {
            id: run.id.clone(),
            expected_version: run.version,
            succeeded: false,
            summary: "The executor reply was missing or not the required JSON object".into(),
        })?;
        bail!("Run {} failed: unusable executor reply", run.id);
    };
    let (applied, retained, denied) = submit(runner, run, credential, &proposals)?;
    let (run, _) = runner.run(&run.id)?;
    let summary = if retained > 0 {
        format!("{summary} ({applied} applied, {retained} retained, {denied} denied)")
    } else {
        format!("{summary} ({applied} applied, {denied} denied)")
    };
    runner.step(Command::FinishAgentRun {
        id: run.id.clone(),
        expected_version: run.version,
        succeeded: true,
        summary: summary.trim().into(),
    })?;
    println!("Run {} succeeded: {summary}", run.id);
    Ok(())
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "this machine".into())
}

/// The harness transcript for a run ID, if Claude Code wrote one.
fn transcript(run_id: &str) -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let projects = PathBuf::from(home).join(".claude").join("projects");
    fs::read_dir(projects)
        .ok()?
        .flatten()
        .map(|d| d.path().join(format!("{run_id}.jsonl")))
        .find(|p| p.is_file())
}

/// Actual model and final assistant text recorded in a transcript.
pub fn read_transcript(content: &str) -> (Option<String>, Option<String>) {
    let mut model = None;
    let mut text = None;
    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["type"] != "assistant" || v["isSidechain"] == true {
            continue;
        }
        if model.is_none() {
            model = v["message"]["model"].as_str().map(str::to_string);
        }
        if let Some(parts) = v["message"]["content"].as_array() {
            let joined: String = parts
                .iter()
                .filter(|p| p["type"] == "text")
                .filter_map(|p| p["text"].as_str())
                .collect();
            if !joined.trim().is_empty() {
                text = Some(joined);
            }
        }
    }
    (model, text)
}

fn reconcile(runner: &Runner) -> Result<()> {
    let snap = runner.read(Command::Snapshot)?;
    let runs: Vec<AgentRun> = serde_json::from_value(snap["agent_runs"].clone())?;
    for run in runs.into_iter().filter(|r| {
        matches!(
            r.state,
            RunState::Launching | RunState::Running | RunState::CancelPending
        )
    }) {
        let pid = run
            .launcher
            .as_deref()
            .and_then(|l| l.strip_prefix("pid "))
            .and_then(|l| l.split_whitespace().next())
            .and_then(|p| p.parse::<u32>().ok());
        if pid.is_some_and(pid_alive) {
            println!("{}: runner still active, left alone", run.id);
            continue;
        }
        if run.state == RunState::CancelPending {
            runner.step(Command::AcknowledgeRunCancel {
                id: run.id.clone(),
                expected_version: run.version,
            })?;
            println!("{}: runner gone; cancellation acknowledged", run.id);
            continue;
        }
        let path = transcript(&run.id);
        // A dispatched session can outlive its runner: leave it while it still writes.
        let recent = path
            .as_ref()
            .and_then(|p| fs::metadata(p).ok()?.modified().ok()?.elapsed().ok())
            .is_some_and(|age| age < Duration::from_secs(120));
        if run.dispatch.is_some() && recent {
            println!("{}: dispatch session still active, left alone", run.id);
            continue;
        }
        let found = path.and_then(|p| fs::read_to_string(p).ok());
        let Some(content) = found else {
            runner.step(Command::MarkAgentRunUnknown {
                id: run.id.clone(),
                expected_version: run.version,
                reason: "No harness session exists for the run ID; launch outcome unknown, redispatch suspended".into(),
            })?;
            println!("{}: no session found; marked unknown", run.id);
            continue;
        };
        let (model, text) = read_transcript(&content);
        let credential = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let mut run = runner.step(Command::RotateRunCredential {
            id: run.id.clone(),
            expected_version: run.version,
            credential_sha256: sha(&credential),
        })?;
        if run.state == RunState::Launching {
            run = runner.step(Command::RecordRunSession {
                id: run.id.clone(),
                expected_version: run.version,
                session_id: run.id.clone(),
                actual_model: model.unwrap_or_default(),
            })?;
            if run.state != RunState::Running {
                println!("{}: session matched; blocked by the model gate", run.id);
                continue;
            }
        }
        match text {
            Some(text) => {
                println!("{}: session matched; ingesting its reply once", run.id);
                finish(runner, &run, &credential, Some((text, false)))?;
            }
            None => {
                runner.step(Command::MarkAgentRunUnknown {
                    id: run.id.clone(),
                    expected_version: run.version,
                    reason: "The session exists but never produced a reply; redispatch suspended"
                        .into(),
                })?;
                println!("{}: incomplete session; marked unknown", run.id);
            }
        }
    }
    Ok(())
}

pub fn run_action(
    client: &direct::Client,
    actor: &str,
    data_dir: &Path,
    action: RunsAction,
) -> Result<()> {
    let runner = Runner { client, actor };
    match action {
        RunsAction::Execute { run_id } => execute(&runner, data_dir, &run_id),
        RunsAction::Reconcile => reconcile(&runner),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn replies_parse_fenced_or_bare_and_reject_floods() {
        let bare = r#"{"summary":"ok","proposals":[{"op":"comment","body":"x"}]}"#;
        assert_eq!(parse_reply(bare, MAX_PROPOSALS).unwrap().1.len(), 1);
        let fenced = format!("Here you go:\n```json\n{bare}\n```\n");
        assert_eq!(parse_reply(&fenced, MAX_PROPOSALS).unwrap().0, "ok");
        assert!(parse_reply("I cannot do that", MAX_PROPOSALS).is_none());
        let flood = json!({"proposals": vec![json!({"op":"comment","body":"x"}); 11]}).to_string();
        assert!(parse_reply(&flood, MAX_PROPOSALS).is_none());
    }

    #[test]
    fn dispatch_rules_allow_only_own_branches_and_never_merge() {
        let base = "1".repeat(40);
        let run: AgentRun = serde_json::from_value(json!({
            "id":"abcdef12-0000-0000-0000-000000000000","issue_key":"DIR","assignment_id":"","member_id":"m","role_id":"r",
            "skill_bundles":[],"guidance":[],"requested_model":"claude-opus-5-5","input_issue_version":0,"objective":"o",
            "state":"intent","version":1,"created_by":"routine:x","created_at":0,"updated_at":0,
            "dispatch":{"product_id":"p","cap":3,"config":{"repository":"o/r","base_ref":"origin/main","worktree_root":"C:/w","allow_commands":["cargo test"]},
                "objectives":[{"key":"DIR-7","version":3,"review_repair":false,"state":"pending"}]}
        }))
        .unwrap();
        let (allow, deny) = dispatch_tool_rules(&run, run.dispatch.as_ref().unwrap(), &base);
        assert!(allow.contains(&"Bash(git push -u origin dispatch/dir-7-abcdef12)".to_string()));
        assert!(allow.contains(&format!(
            "Bash(git switch -c dispatch/dir-7-abcdef12 {base})"
        )));
        assert!(allow.contains(&"Bash(cargo test:*)".to_string()));
        assert!(allow.contains(&"Bash(direct run-action:*)".to_string()));
        assert!(
            !allow
                .iter()
                .any(|a| a == "Bash(git push:*)" || a.contains("merge") || a == "Bash"),
            "no general push, merge or unrestricted shell"
        );
        assert!(deny.contains(&"Bash(gh pr merge:*)".to_string()));
        assert!(deny.contains(&"Bash(git push --force:*)".to_string()));
    }

    #[test]
    fn transcripts_yield_model_and_final_text_ignoring_subagents() {
        let lines = [
            json!({"type":"user","message":{"content":"hi"}}),
            json!({"type":"assistant","isSidechain":true,"message":{"model":"other","content":[{"type":"text","text":"side"}]}}),
            json!({"type":"assistant","message":{"model":"claude-opus-5-5","content":[{"type":"text","text":"{\"proposals\":[]}"}]}}),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        let (model, text) = read_transcript(&lines);
        assert_eq!(model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(text.as_deref(), Some("{\"proposals\":[]}"));
    }
}
