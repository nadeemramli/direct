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
    AgentMember, AgentRole, AgentRun, Command, Request, Role, RunState, SkillPackage,
    TheoriaDocument,
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

/// The JSON object inside the executor's final text, fenced or bare.
pub fn parse_reply(text: &str) -> Option<(String, Vec<Value>)> {
    let trimmed = text.trim();
    let body = trimmed
        .split("```json")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .unwrap_or(trimmed);
    let value: Value = serde_json::from_str(body.trim()).ok()?;
    let proposals = value["proposals"].as_array()?.clone();
    if proposals.len() > MAX_PROPOSALS {
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
) -> Result<(usize, usize)> {
    let (mut applied, mut denied) = (0, 0);
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
            _ => denied += 1,
        }
    }
    Ok((applied, denied))
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
    let context = runner.read(Command::Context {
        key: run.issue_key.clone(),
    })?;
    let input = packet(&run, &snap, &context)?;
    let credential = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    let run = runner.step(Command::StartAgentRun {
        id: run.id.clone(),
        expected_version: run.version,
        launcher: format!("pid {} on {}", std::process::id(), hostname()),
        harness_version: version,
        credential_sha256: sha(&credential),
    })?;

    let workdir = data_dir.join("runs").join(&run.id);
    fs::create_dir_all(&workdir)?;
    let mut child = Process::new(&program)
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
        .current_dir(&workdir)
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

    // Watch for an owner cancel while the harness runs.
    let done = Arc::new(AtomicBool::new(false));
    let canceled = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (done, canceled, child) = (done.clone(), canceled.clone(), child.clone());
        let client = runner.client.clone();
        let (actor, id) = (runner.actor.to_string(), run.id.clone());
        std::thread::spawn(move || {
            let runner = Runner {
                client: &client,
                actor: &actor,
            };
            while !done.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(2));
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
    finish(runner, &run, &credential, final_text)
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
        .and_then(|(text, _)| parse_reply(text));
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
    let (applied, denied) = submit(runner, run, credential, &proposals)?;
    let (run, _) = runner.run(&run.id)?;
    let summary = format!("{summary} ({applied} applied, {denied} denied)");
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
        let found = transcript(&run.id).and_then(|p| fs::read_to_string(p).ok());
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
        assert_eq!(parse_reply(bare).unwrap().1.len(), 1);
        let fenced = format!("Here you go:\n```json\n{bare}\n```\n");
        assert_eq!(parse_reply(&fenced).unwrap().0, "ok");
        assert!(parse_reply("I cannot do that").is_none());
        let flood = json!({"proposals": vec![json!({"op":"comment","body":"x"}); 11]}).to_string();
        assert!(parse_reply(&flood).is_none());
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
