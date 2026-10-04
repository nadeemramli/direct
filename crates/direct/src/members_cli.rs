//! `direct members check`: a real runtime adapter capability check (DIR-75).
//!
//! Availability is never assumed. For Claude Code, each requested model is run
//! once through the installed harness and counts as verified only when the
//! harness's own usage report names that model. Runtimes without an adapter
//! report that plainly and verify nothing.

use anyhow::{anyhow, bail, Context, Result};
use clap::Subcommand;
use direct_core::{AgentMember, Command, Request, Role};
use serde_json::Value;
use std::process::Command as Process;

#[derive(Subcommand, Clone)]
pub enum MembersAction {
    /// Run the member's harness for each model and record what actually ran.
    Check {
        #[arg(long)]
        member_id: String,
        /// Model to verify; repeat for several.
        #[arg(long = "model", required = true)]
        models: Vec<String>,
        #[arg(long)]
        request_id: String,
    },
}

fn claude_program() -> String {
    std::env::var("DIRECT_CLAUDE_BIN").unwrap_or_else(|_| {
        if cfg!(windows) {
            "claude.cmd".into()
        } else {
            "claude".into()
        }
    })
}

/// The session ID when the harness output proves `model` actually ran.
pub fn verified_session(output: &str, model: &str) -> Option<String> {
    let value: Value = serde_json::from_str(output.trim()).ok()?;
    if value["is_error"].as_bool() == Some(true) {
        return None;
    }
    value["modelUsage"].as_object()?.get(model)?;
    value["session_id"].as_str().map(str::to_string)
}

fn run(program: &str, args: &[&str]) -> Result<String> {
    let out = Process::new(program)
        .args(args)
        .output()
        .with_context(|| format!("Could not start {program}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn run_action(client: &direct::Client, actor: &str, action: MembersAction) -> Result<()> {
    let MembersAction::Check {
        member_id,
        models,
        request_id,
    } = action;
    let snap = client.call(
        &Request {
            actor: actor.into(),
            request_id: String::new(),
            command: Command::Snapshot,
        },
        Role::Agent,
    )?;
    let members: Vec<AgentMember> = serde_json::from_value(snap["agent_members"].clone())?;
    let member = members
        .iter()
        .find(|m| m.id == member_id)
        .ok_or_else(|| anyhow!("Unknown member {member_id}"))?;
    if member.runtime != "claude-code" {
        bail!(
            "No capability adapter for {} yet; its models stay unverified and cannot be assigned",
            member.runtime
        );
    }
    let program = claude_program();
    let harness_version = run(&program, &["--version"])?.trim().to_string();
    if harness_version.is_empty() {
        bail!("{program} --version returned nothing; the harness is unavailable");
    }
    let mut verified = Vec::new();
    let mut evidence = Vec::new();
    for model in &models {
        let output = run(
            &program,
            &[
                "-p",
                "Reply with the single word READY.",
                "--model",
                model,
                "--output-format",
                "json",
                "--max-turns",
                "1",
            ],
        )?;
        match verified_session(&output, model) {
            Some(session) => {
                println!("{model}: verified (session {session})");
                evidence.push(format!("{model}={session}"));
                verified.push(model.clone());
            }
            None => println!("{model}: unverified (the harness did not report running it)"),
        }
    }
    let value = client.call(
        &Request {
            actor: actor.into(),
            request_id,
            command: Command::RecordMemberCapability {
                id: member.id.clone(),
                expected_version: member.version,
                harness_version: harness_version.clone(),
                verified_models: verified,
                evidence: format!(
                    "claude-code {harness_version}: {}",
                    if evidence.is_empty() {
                        "no model verified".into()
                    } else {
                        evidence.join(", ")
                    }
                ),
            },
        },
        Role::Agent,
    )?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_reported_model_counts_as_verified() {
        let ok = r#"{"is_error":false,"session_id":"s1","modelUsage":{"claude-opus-5-5":{}}}"#;
        assert_eq!(
            verified_session(ok, "claude-opus-5-5").as_deref(),
            Some("s1")
        );
        assert_eq!(verified_session(ok, "claude-sonnet-5-5"), None);
        let failed = r#"{"is_error":true,"session_id":"s2","modelUsage":{"claude-opus-5-5":{}}}"#;
        assert_eq!(verified_session(failed, "claude-opus-5-5"), None);
        assert_eq!(verified_session("not json", "claude-opus-5-5"), None);
    }
}
