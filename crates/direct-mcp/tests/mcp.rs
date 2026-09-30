//! End-to-end tests: a real `direct-mcp` child process over stdio against an
//! isolated Direct service in a temporary directory. The owner's workspace is
//! never touched.
use direct::Client;
use direct_core::{Command, Request, Role};
use rmcp::{
    model::{CallToolRequestParams, CallToolResult, ClientConfig},
    service::{RoleClient, RunningService},
    transport::{ConfigureCommandExt, TokioChildProcess},
    ServiceExt,
};
use serde_json::{json, Map, Value};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    thread,
    time::Duration,
};
use tokio::runtime::Runtime;

struct Workspace {
    _temp: tempfile::TempDir,
    dir: PathBuf,
}

/// Run the real local service in-process on its own runtime thread. It keeps
/// running after every MCP child process exits, exactly like the owner's
/// desktop-started service.
fn workspace() -> Workspace {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("workspace");
    let assets = temp.path().join("web");
    let serve_dir = dir.clone();
    thread::spawn(move || {
        Runtime::new()
            .unwrap()
            .block_on(direct::server::serve(&serve_dir, 0, &assets))
            .unwrap();
    });
    for _ in 0..200 {
        if Client::new(&dir).is_ok_and(|c| c.healthy()) {
            return Workspace { _temp: temp, dir };
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("isolated Direct service failed to start");
}

fn req(mut value: Value) -> Request {
    value["actor"] = json!("test-owner-harness");
    value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    serde_json::from_value(value).unwrap()
}

/// Seed one owner-Ready issue (version 3) through the normal service contract.
fn seed_ready_issue(dir: &Path) {
    let client = Client::new(dir).unwrap();
    client
        .call(
            &req(
                json!({"op":"create_issue","product":"DIR","title":"MCP handoff","body":"Use MCP"}),
            ),
            Role::Agent,
        )
        .unwrap();
    client
        .call(
            &req(json!({"op":"update_issue","key":"DIR-1","expected_version":1,"title":"MCP handoff","body":"Use MCP","acceptance":"Works over stdio","owner":"tester","priority":"high"})),
            Role::Agent,
        )
        .unwrap();
    client
        .call(
            &req(json!({"op":"ready","key":"DIR-1","expected_version":2})),
            Role::Human,
        )
        .unwrap();
}

fn context(dir: &Path, key: &str) -> Value {
    Client::new(dir)
        .unwrap()
        .call(
            &Request {
                actor: "test-owner-harness".into(),
                request_id: String::new(),
                command: Command::Context { key: key.into() },
            },
            Role::Agent,
        )
        .unwrap()
}

fn export(dir: &Path) -> Value {
    Client::new(dir)
        .unwrap()
        .call(
            &Request {
                actor: "test-owner-harness".into(),
                request_id: String::new(),
                command: Command::Export,
            },
            Role::Agent,
        )
        .unwrap()
}

type Mcp = RunningService<RoleClient, ClientConfig>;

async fn connect(dir: &Path, actor: &str) -> Mcp {
    let transport = TokioChildProcess::new(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_direct-mcp")).configure(|cmd| {
            cmd.arg("--data-dir")
                .arg(dir)
                .arg("--actor")
                .arg(actor)
                .stderr(Stdio::inherit());
        }),
    )
    .unwrap();
    ClientConfig::default().serve(transport).await.unwrap()
}

fn args(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => unreachable!(),
    }
}

async fn call(mcp: &Mcp, tool: &str, arguments: Value) -> CallToolResult {
    mcp.call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(args(arguments)))
        .await
        .unwrap()
}

fn ok(result: &CallToolResult) -> Value {
    assert_ne!(
        result.is_error,
        Some(true),
        "tool reported an error: {:?}",
        result.content
    );
    result
        .structured_content
        .clone()
        .expect("structured content")
}

fn error_text(result: &CallToolResult) -> String {
    assert_eq!(result.is_error, Some(true), "expected a tool error");
    serde_json::to_string(&result.content).unwrap()
}

const CLAIM_V3: &str =
    r#"{"key":"DIR-1","expected_version":3,"request_id":"mcp-claim-1","lease_seconds":7200}"#;

#[test]
fn initialization_discovers_only_the_allowlisted_tools_and_reads_context() {
    let ws = workspace();
    seed_ready_issue(&ws.dir);
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let mcp = connect(&ws.dir, "mcp-agent").await;
        let info = serde_json::to_value(mcp.peer_info().expect("server info")).unwrap();
        assert_eq!(info["serverInfo"]["name"], "direct-mcp");
        assert!(info["capabilities"]["tools"].is_object());
        let instructions = info["instructions"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(instructions.contains("cannot make work Ready"));
        assert!(instructions.contains("request_id"));

        let tools = mcp.list_all_tools().await.unwrap();
        let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "direct_claim",
                "direct_context",
                "direct_list",
                "direct_renew",
                "direct_submit"
            ]
        );
        for forbidden in [
            "ready", "review", "reopen", "release", "launch", "open", "restore", "product",
            "project", "call", "raw", "sql", "export", "comment",
        ] {
            assert!(
                !names.iter().any(|n| n.contains(forbidden)),
                "{forbidden} must not be exposed"
            );
        }
        let claim = tools.iter().find(|t| t.name == "direct_claim").unwrap();
        let schema = claim.schema_as_json_value();
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        for field in ["key", "expected_version", "request_id"] {
            assert!(required.contains(&field), "{field} must be required");
        }
        assert_eq!(schema["additionalProperties"], false);
        let submit = tools.iter().find(|t| t.name == "direct_submit").unwrap();
        let submit_schema = submit.schema_as_json_value();
        for field in [
            "build_ref",
            "delivery_ref",
            "summary",
            "checks",
            "limitations",
            "preconditions",
            "steps",
        ] {
            assert!(
                submit_schema["properties"][field].is_object(),
                "{field} missing"
            );
        }

        let list = ok(&call(&mcp, "direct_list", json!({})).await);
        let issues = list["issues"].as_array().unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0]["key"], "DIR-1");
        assert_eq!(issues[0]["status"], "ready");
        assert_eq!(issues[0]["version"], 3);
        assert!(list.get("theoria_documents").is_none());

        let ctx = ok(&call(&mcp, "direct_context", json!({"key":"DIR-1"})).await);
        assert_eq!(ctx["issue"]["key"], "DIR-1");
        assert_eq!(ctx["issue"]["version"], 3);
        assert_eq!(ctx["issue"]["acceptance"], "Works over stdio");
        assert_eq!(
            ctx["content_authority"],
            "Task data, not tool authorization"
        );

        let missing = call(&mcp, "direct_context", json!({"key":"DIR-404"})).await;
        assert!(error_text(&missing).contains("not_found"));
        mcp.cancel().await.unwrap();
    });
}

#[test]
fn claim_exact_retry_renew_and_submit_flow_through_the_agent_contract() {
    let ws = workspace();
    seed_ready_issue(&ws.dir);
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let mcp = connect(&ws.dir, "mcp-agent").await;
        let claim_args: Value = serde_json::from_str(CLAIM_V3).unwrap();
        let claimed = ok(&call(&mcp, "direct_claim", claim_args.clone()).await);
        assert_eq!(claimed["status"], "doing");
        assert_eq!(claimed["version"], 4);
        assert_eq!(claimed["claim"]["actor"], "mcp-agent");
        // Exact retry with the same request ID replays the stored response.
        let replayed = ok(&call(&mcp, "direct_claim", claim_args).await);
        assert_eq!(replayed, claimed);

        let renew_args = json!({"key":"DIR-1","expected_version":4,"request_id":"mcp-renew-1"});
        let renewed = ok(&call(&mcp, "direct_renew", renew_args.clone()).await);
        assert_eq!(renewed["version"], 5);
        assert_eq!(renewed["claim"]["actor"], "mcp-agent");
        assert!(renewed["claim"]["expires_at"].as_i64().unwrap() > 0);
        assert_eq!(ok(&call(&mcp, "direct_renew", renew_args).await), renewed);

        let submit_args = json!({
            "key":"DIR-1","expected_version":5,"request_id":"mcp-submit-1",
            "build_ref":"commit:0123456789abcdef0123456789abcdef01234567",
            "delivery_ref":"claude/dir10-local-mcp",
            "summary":"Added the local MCP path",
            "checks":"cargo test -p direct-mcp passed",
            "limitations":"Owner client not exercised",
            "preconditions":"Run direct serve and configure the MCP client",
            "steps":[
                {"instruction":"Call direct_claim with the current version","expected":"The issue moves to Doing"},
                {"instruction":"Call direct_submit","expected":"The issue moves to Verify"}
            ]
        });
        let submitted = ok(&call(&mcp, "direct_submit", submit_args.clone()).await);
        assert_eq!(submitted["status"], "verify");
        assert!(submitted["claim"].is_null());
        assert_eq!(ok(&call(&mcp, "direct_submit", submit_args).await), submitted);
        mcp.cancel().await.unwrap();
    });

    // The service, read independently of MCP, shows one run and no duplicate effects.
    let ctx = context(&ws.dir, "DIR-1");
    assert_eq!(ctx["issue"]["status"], "verify");
    let runs = ctx["verifications"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["checks"], "cargo test -p direct-mcp passed");
    assert_eq!(runs[0]["limitations"], "Owner client not exercised");
    assert_eq!(
        runs[0]["preconditions"],
        "Run direct serve and configure the MCP client"
    );
    assert_eq!(runs[0]["steps"].as_array().unwrap().len(), 2);
    assert_eq!(runs[0]["submitted_by"], "mcp-agent");
    assert_eq!(runs[0]["outcome"], "pending");
    let kinds: Vec<&str> = ctx["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds.iter().filter(|k| **k == "work_claimed").count(), 1);
    assert_eq!(kinds.iter().filter(|k| **k == "claim_renewed").count(), 1);
    assert_eq!(
        kinds
            .iter()
            .filter(|k| **k == "verification_requested")
            .count(),
        1
    );
    // The verification child is Ready for the owner; nothing is Done.
    let child = context(&ws.dir, ctx["issue"]["verification_key"].as_str().unwrap());
    assert_eq!(child["issue"]["status"], "ready");
}

#[test]
fn changed_payloads_stale_versions_and_other_actors_are_rejected() {
    let ws = workspace();
    seed_ready_issue(&ws.dir);
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let mcp = connect(&ws.dir, "mcp-agent").await;
        let other = connect(&ws.dir, "other-agent").await;
        ok(&call(&mcp, "direct_claim", serde_json::from_str(CLAIM_V3).unwrap()).await);

        // Same request ID, changed payload.
        let changed = call(
            &mcp,
            "direct_claim",
            json!({"key":"DIR-1","expected_version":3,"request_id":"mcp-claim-1","lease_seconds":3600}),
        )
        .await;
        let text = error_text(&changed);
        assert!(text.contains("conflict"), "{text}");
        assert!(text.contains("already used"), "{text}");

        // Stale expected version with a fresh request ID.
        let stale = call(
            &mcp,
            "direct_renew",
            json!({"key":"DIR-1","expected_version":3,"request_id":"mcp-renew-stale"}),
        )
        .await;
        let text = error_text(&stale);
        assert!(text.contains("conflict"), "{text}");
        assert!(text.contains("is version 4"), "{text}");

        // A different actor cannot renew or submit this claim.
        let foreign_renew = call(
            &other,
            "direct_renew",
            json!({"key":"DIR-1","expected_version":4,"request_id":"other-renew-1"}),
        )
        .await;
        assert!(error_text(&foreign_renew).contains("claim_required"));
        let foreign_submit = call(
            &other,
            "direct_submit",
            json!({
                "key":"DIR-1","expected_version":4,"request_id":"other-submit-1",
                "build_ref":"commit:0123456789abcdef0123456789abcdef01234567",
                "delivery_ref":"branch","summary":"s","checks":"c",
                "steps":[{"instruction":"i","expected":"e"}]
            }),
        )
        .await;
        assert!(error_text(&foreign_submit).contains("claim_required"));
        // Nor can it take the claim while it is active.
        let foreign_claim = call(
            &other,
            "direct_claim",
            json!({"key":"DIR-1","expected_version":4,"request_id":"other-claim-1"}),
        )
        .await;
        assert!(error_text(&foreign_claim).contains("active claim"));

        // Missing request IDs never get a generated replacement.
        let blank = call(
            &mcp,
            "direct_renew",
            json!({"key":"DIR-1","expected_version":4,"request_id":"  "}),
        )
        .await;
        assert!(error_text(&blank).contains("request_id is required"));

        // Unknown fields cannot smuggle an operation or role.
        let smuggled = call(
            &mcp,
            "direct_renew",
            json!({"key":"DIR-1","expected_version":4,"request_id":"x","op":"ready"}),
        )
        .await;
        assert_eq!(smuggled.is_error, Some(true));

        mcp.cancel().await.unwrap();
        other.cancel().await.unwrap();
    });
    let ctx = context(&ws.dir, "DIR-1");
    assert_eq!(ctx["issue"]["version"], 4);
    assert_eq!(ctx["issue"]["claim"]["actor"], "mcp-agent");
    assert!(ctx["verifications"].as_array().unwrap().is_empty());
}

#[test]
fn owner_only_operations_cannot_be_invoked_and_submission_cannot_self_approve() {
    let ws = workspace();
    seed_ready_issue(&ws.dir);
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let mcp = connect(&ws.dir, "mcp-agent").await;
        for (tool, arguments) in [
            ("direct_ready", json!({"key":"DIR-1","expected_version":3,"request_id":"r"})),
            ("ready", json!({"key":"DIR-1","expected_version":3,"request_id":"r"})),
            (
                "direct_review",
                json!({"key":"DIR-1","expected_version":3,"request_id":"r","run_id":"x","outcome":"passed","results":[]}),
            ),
            ("direct_reopen", json!({"key":"DIR-1","expected_version":3,"request_id":"r","reason":"x"})),
            ("direct_release", json!({"key":"DIR-1","expected_version":3,"request_id":"r"})),
            ("direct_create_product", json!({"key":"X","name":"X"})),
            ("direct_create_project", json!({"product":"DIR","name":"X"})),
            ("direct_open", json!({})),
            ("direct_launch", json!({})),
            ("direct_restore", json!({"from":"x"})),
            ("direct_export", json!({})),
            ("direct_call", json!({"op":"ready","key":"DIR-1","expected_version":3,"request_id":"r"})),
            ("call", json!({"op":"snapshot"})),
        ] {
            let outcome = mcp
                .call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(args(arguments)))
                .await;
            let message = format!("{outcome:?}");
            assert!(outcome.is_err(), "{tool} must not be callable: {message}");
            assert!(message.contains("tool not found"), "{tool}: {message}");
        }

        // Claim, then submit; the result stays in Verify and nothing lets the agent pass it.
        ok(&call(&mcp, "direct_claim", serde_json::from_str(CLAIM_V3).unwrap()).await);
        let submitted = ok(&call(
            &mcp,
            "direct_submit",
            json!({
                "key":"DIR-1","expected_version":4,"request_id":"mcp-submit-1",
                "build_ref":"commit:0123456789abcdef0123456789abcdef01234567",
                "delivery_ref":"branch","summary":"s","checks":"c",
                "steps":[{"instruction":"i","expected":"e"}]
            }),
        )
        .await);
        assert_eq!(submitted["status"], "verify");
        mcp.cancel().await.unwrap();
    });
    let ctx = context(&ws.dir, "DIR-1");
    assert_eq!(ctx["issue"]["status"], "verify");
    assert_eq!(ctx["verifications"][0]["outcome"], "pending");
    assert!(ctx["verifications"][0]["reviewed_by"].is_null());
    assert!(ctx["issue"]["needs_fix"] == false);
    // Even the raw agent capability cannot review; only the owner path can.
    let review = req(
        json!({"op":"review","key":"DIR-1","expected_version":5,"run_id":ctx["verifications"][0]["id"],"outcome":"passed","results":[{"outcome":"passed"}]}),
    );
    assert!(Client::new(&ws.dir)
        .unwrap()
        .call(&review, Role::Agent)
        .unwrap_err()
        .to_string()
        .contains("forbidden"));
}

#[test]
fn responses_and_errors_never_leak_bearer_tokens_or_endpoint_contents() {
    let ws = workspace();
    seed_ready_issue(&ws.dir);
    let endpoint = direct::endpoint(&ws.dir).unwrap();
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let mcp = connect(&ws.dir, "mcp-agent").await;
        let info = serde_json::to_string(&mcp.peer_info()).unwrap();
        let tools = serde_json::to_string(&mcp.list_all_tools().await.unwrap()).unwrap();
        let mut texts = vec![info, tools];
        for (tool, arguments) in [
            ("direct_list", json!({})),
            ("direct_context", json!({"key":"DIR-1"})),
            ("direct_context", json!({"key":"DIR-404"})),
            (
                "direct_claim",
                json!({"key":"DIR-1","expected_version":9,"request_id":"stale"}),
            ),
            (
                "direct_renew",
                json!({"key":"DIR-1","expected_version":3,"request_id":"unheld"}),
            ),
            ("direct_claim", json!({"key":"DIR-1"})),
        ] {
            let result = call(&mcp, tool, arguments).await;
            texts.push(serde_json::to_string(&result).unwrap());
        }
        for text in texts {
            assert!(!text.contains(&endpoint.agent_token), "agent token leaked");
            assert!(!text.contains(&endpoint.owner_token), "owner token leaked");
            assert!(!text.contains("endpoint.json"), "endpoint file named");
            assert!(!text.contains("agent_token"), "endpoint field leaked");
        }
        mcp.cancel().await.unwrap();
    });
}

#[test]
fn mcp_shutdown_leaves_the_direct_service_running_and_unchanged() {
    let ws = workspace();
    seed_ready_issue(&ws.dir);
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let mcp = connect(&ws.dir, "mcp-agent").await;
        ok(&call(
            &mcp,
            "direct_claim",
            serde_json::from_str(CLAIM_V3).unwrap(),
        )
        .await);
        // Graceful close: the client closes stdin and the child exits.
        mcp.cancel().await.unwrap();
    });
    let before = export(&ws.dir);
    rt.block_on(async {
        // Abrupt termination of a second MCP process.
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_direct-mcp"))
            .arg("--data-dir")
            .arg(&ws.dir)
            .arg("--actor")
            .arg("mcp-agent")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        child.kill().await.unwrap();
        child.wait().await.unwrap();
    });
    let client = Client::new(&ws.dir).unwrap();
    assert!(client.healthy(), "service must survive MCP shutdown");
    assert_eq!(export(&ws.dir), before);
    // The CLI-style client still writes normally afterwards.
    let renewed = client
        .call(
            &Request {
                actor: "mcp-agent".into(),
                request_id: "after-mcp-renew".into(),
                command: Command::Renew {
                    key: "DIR-1".into(),
                    expected_version: 4,
                    lease_seconds: 3600,
                },
            },
            Role::Agent,
        )
        .unwrap();
    assert_eq!(renewed["version"], 5);
}

#[test]
fn binary_requires_a_distinct_actor_and_keeps_stdout_for_the_protocol() {
    let temp = tempfile::tempdir().unwrap();
    let run = |extra: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_direct-mcp"))
            .arg("--data-dir")
            .arg(temp.path())
            .args(extra)
            .env_remove("DIRECT_MCP_ACTOR")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let missing = run(&[]);
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("--actor"));

    let shared = run(&["--actor", "local-agent"]);
    assert!(!shared.status.success());
    assert!(shared.stdout.is_empty());
    assert!(String::from_utf8_lossy(&shared.stderr).contains("distinct"));

    // With a valid actor and no client input, the server exits quietly on EOF.
    let quiet = run(&["--actor", "mcp-agent"]);
    assert!(quiet.stdout.is_empty());
}
