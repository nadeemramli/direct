use direct::{endpoint, Client};
use direct_core::{Command, Request, Role};
use serde_json::{json, Value};
use std::{
    path::Path,
    process::{Child, Command as Process, Stdio},
    sync::{Arc, Barrier},
    thread,
    time::Duration,
};
struct Service(Child);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start(dir: &Path) -> Service {
    let child = direct::hidden(
        Process::new(env!("CARGO_BIN_EXE_direct"))
            .arg("--data-dir")
            .arg(dir)
            .arg("serve"),
    )
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let s = Service(child);
    for _ in 0..100 {
        if Client::new(dir).is_ok_and(|c| c.healthy()) {
            return s;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("Service failed to start");
}
fn req(mut value: Value) -> Request {
    value["actor"] = json!("test-agent");
    value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    serde_json::from_value(value).unwrap()
}

#[test]
fn draft_route_rejects_agent_foreign_origin_and_invalid_input_before_model_execution() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("workspace");
    let _service = start(&dir);
    let endpoint = endpoint(&dir).unwrap();
    let http = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .unwrap();
    let url = format!("http://127.0.0.1:{}/api/draft-brief", endpoint.port);
    let input = json!({"title":"Test title"});
    assert_eq!(
        http.post(&url)
            .bearer_auth(&endpoint.agent_token)
            .json(&input)
            .send()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        http.post(&url)
            .bearer_auth(&endpoint.owner_token)
            .header("Origin", "https://unrelated.example")
            .json(&input)
            .send()
            .unwrap()
            .status(),
        403
    );
    let rejected = http
        .post(&url)
        .bearer_auth(&endpoint.owner_token)
        .json(&json!({"title":""}))
        .send()
        .unwrap();
    assert_eq!(rejected.status(), 400);
    assert!(rejected.json::<Value>().unwrap()["message"]
        .as_str()
        .unwrap()
        .contains("title"));
}

fn native(dir: &Path, args: &[&str]) -> Value {
    let output = Process::new(env!("CARGO_BIN_EXE_direct"))
        .arg("--data-dir")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "native command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn native_agent_handoff_claims_renews_retries_and_submits() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("workspace");
    let _service = start(&dir);
    let client = Client::new(&dir).unwrap();
    client
        .call(
            &req(json!({"op":"create_issue","product":"DIR","title":"Native handoff","body":"Use the CLI"})),
            Role::Agent,
        )
        .unwrap();
    client
        .call(
            &req(json!({"op":"update_issue","key":"DIR-1","expected_version":1,"title":"Native handoff","body":"Use the CLI","acceptance":"One documented path","owner":"tester","priority":"high"})),
            Role::Agent,
        )
        .unwrap();
    client
        .call(
            &req(json!({"op":"ready","key":"DIR-1","expected_version":2})),
            Role::Human,
        )
        .unwrap();

    let claim_args = [
        "--actor",
        "handoff-agent",
        "claim",
        "DIR-1",
        "--expected-version",
        "3",
        "--lease-seconds",
        "7200",
        "--request-id",
        "native-claim-1",
    ];
    let claimed = native(&dir, &claim_args);
    assert_eq!(native(&dir, &claim_args), claimed);
    assert_eq!(claimed["status"], "doing");
    assert_eq!(claimed["claim"]["actor"], "handoff-agent");

    let renew_args = [
        "--actor",
        "handoff-agent",
        "renew",
        "DIR-1",
        "--expected-version",
        "4",
        "--lease-seconds",
        "7200",
        "--request-id",
        "native-renew-1",
    ];
    let renewed = native(&dir, &renew_args);
    assert_eq!(native(&dir, &renew_args), renewed);
    assert_eq!(renewed["version"], 5);

    let submit_args = [
        "--actor",
        "handoff-agent",
        "submit",
        "DIR-1",
        "--expected-version",
        "5",
        "--build-ref",
        "commit:0123456789abcdef0123456789abcdef01234567",
        "--delivery-ref",
        "codex/native-handoff",
        "--summary",
        "Added the native handoff path",
        "--e2e-environment",
        "isolated Windows fixture",
        "--e2e-entrypoint",
        "fixture CLI",
        "--e2e-scenarios",
        "Claim, renew and submit through CLI; persisted context matches",
        "--delivered-build-ref",
        "commit:0123456789abcdef0123456789abcdef01234567",
        "--delivery-check",
        "Fixture client and service launched from this build",
        "--checks",
        "Native integration test passed",
        "--step",
        "Run the documented claim command",
        "The issue moves to Doing",
        "--request-id",
        "native-submit-1",
    ];
    let submitted = native(&dir, &submit_args);
    assert_eq!(native(&dir, &submit_args), submitted);
    assert_eq!(submitted["status"], "verify");
    assert!(submitted["claim"].is_null());
    let context = native(&dir, &["--actor", "handoff-agent", "context", "DIR-1"]);
    assert_eq!(
        context["verifications"][0]["checks"],
        "Native integration test passed"
    );
    assert_eq!(
        context["verifications"][0]["steps"][0]["expected"],
        "The issue moves to Doing"
    );
}

#[test]
fn labels_cross_the_wire_with_owner_definitions_and_agent_assignments() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("workspace");
    let _service = start(&dir);
    let client = Client::new(&dir).unwrap();
    let definition = req(
        json!({"op":"create_label","name":"Bug","color":"#d73a49","aliases":["defect"],"linear_origins":[{"id":"lin-bug","name":"Bug"}]}),
    );
    assert!(client.call(&definition, Role::Agent).is_err());
    let label = client.call(&definition, Role::Human).unwrap();
    assert_eq!(label["version"], 1);
    assert!(client
        .call(
            &req(json!({"op":"create_label","name":"defect"})),
            Role::Human
        )
        .unwrap_err()
        .to_string()
        .starts_with("conflict"));
    client
        .call(
            &req(json!({"op":"create_issue","product":"DIR","title":"Labelled work","body":"Filter me"})),
            Role::Agent,
        )
        .unwrap();
    let attached = client
        .call(
            &req(json!({"op":"attach_issue_label","key":"DIR-1","expected_version":1,"label_id":label["id"]})),
            Role::Agent,
        )
        .unwrap();
    assert_eq!(attached["labels"], json!([label["id"]]));
    assert!(client
        .call(
            &req(json!({"op":"attach_issue_label","key":"DIR-1","expected_version":1,"label_id":label["id"]})),
            Role::Agent,
        )
        .unwrap_err()
        .to_string()
        .starts_with("conflict"));
    let context = native(&dir, &["--actor", "handoff-agent", "context", "DIR-1"]);
    assert_eq!(context["labels"][0]["name"], "Bug");
    assert_eq!(context["labels"][0]["linear_origins"][0]["id"], "lin-bug");
    let snapshot = native(&dir, &["list"]);
    assert_eq!(snapshot["labels"][0]["aliases"], json!(["defect"]));
    assert_eq!(snapshot["issues"][0]["labels"], json!([label["id"]]));
}

#[test]
fn templates_cross_the_wire_owner_defines_agents_apply_and_restart_preserves_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("workspace");
    let service = start(&dir);
    let client = Client::new(&dir).unwrap();
    for (key, name) in [("ALP", "Synthetic Alpha"), ("BET", "Synthetic Beta")] {
        client
            .call(
                &req(json!({"op":"create_product","key":key,"name":name})),
                Role::Human,
            )
            .unwrap();
    }
    let definition = req(
        json!({"op":"create_template","target":"issue","name":"Discovery probe","shape":"discovery_probe",
               "content":{"intent":"What do we need to learn?","execution_mode":"prototype","boundaries":"Throwaway; no production data"}}),
    );
    // The agent capability cannot define templates over HTTP.
    assert!(client
        .call(&definition, Role::Agent)
        .unwrap_err()
        .to_string()
        .starts_with("forbidden"));
    let created = client.call(&definition, Role::Human).unwrap();
    let id = created["template"]["id"].as_str().unwrap().to_string();
    let listed = native(&dir, &["--actor", "template-agent", "templates"]);
    assert_eq!(listed["templates"][0]["name"], "Discovery probe");
    assert_eq!(
        listed["template_revisions"][0]["content"]["execution_mode"],
        "prototype"
    );
    // Native CLI applies the shared template in two products.
    for product in ["ALP", "BET"] {
        let issue = native(
            &dir,
            &[
                "--actor",
                "template-agent",
                "create",
                "--product",
                product,
                "Probe",
                "--body",
                "Learn",
                "--template-id",
                &id,
                "--template-revision",
                "1",
            ],
        );
        assert_eq!(issue["status"], "backlog");
        assert_eq!(issue["template"]["revision"], 1);
        assert_eq!(issue["template"]["applied_by"], "template-agent");
    }
    client
        .call(
            &req(json!({"op":"revise_template","id":id,"expected_version":1,"name":"Discovery probe","shape":"discovery_probe",
                        "content":{"intent":"Question, probe and decision"}})),
            Role::Human,
        )
        .unwrap();
    let before = client
        .call(&req(json!({"op":"export"})), Role::Agent)
        .unwrap();
    assert_eq!(before["format"], 16);
    drop(service);
    let _restart = start(&dir);
    let after = Client::new(&dir)
        .unwrap()
        .call(&req(json!({"op":"export"})), Role::Agent)
        .unwrap();
    assert_eq!(before, after);
    let context = native(&dir, &["context", "ALP-1"]);
    assert_eq!(context["template"]["revision"]["revision"], 1);
    assert_eq!(context["template"]["outdated"], true);
}

#[test]
fn two_real_clients_claim_once_and_http_enforces_local_capabilities() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("workspace");
    let service = start(&dir);
    let client = Client::new(&dir).unwrap();
    client.call(&req(json!({"op":"create_issue","product":"DIR","title":"Race test","body":"An exclusive claim"})),Role::Agent).unwrap();
    client.call(&req(json!({"op":"update_issue","key":"DIR-1","expected_version":1,"title":"Race test","body":"An exclusive claim","acceptance":"One writer wins","owner":"tester","priority":"high"})),Role::Agent).unwrap();
    let ready = req(json!({"op":"ready","key":"DIR-1","expected_version":2}));
    assert!(client.call(&ready, Role::Agent).is_err());
    client.call(&ready, Role::Human).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let dir = dir.clone();
            let b = barrier.clone();
            thread::spawn(move || {
                let c = Client::new(&dir).unwrap();
                b.wait();
                c.call(
                    &req(json!({"op":"claim","key":"DIR-1","expected_version":3})),
                    Role::Agent,
                )
                .is_ok()
            })
        })
        .collect();
    assert_eq!(
        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .filter(|ok| *ok)
            .count(),
        1
    );
    let e = endpoint(&dir).unwrap();
    let http = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .unwrap();
    let url = format!("http://127.0.0.1:{}/api/command", e.port);
    let read = req(json!({"op":"snapshot"}));
    assert_eq!(http.post(&url).json(&read).send().unwrap().status(), 401);
    assert_eq!(
        http.post(&url)
            .bearer_auth(&e.agent_token)
            .header("Origin", "https://example.com")
            .json(&read)
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        http.post(&url)
            .bearer_auth(&e.agent_token)
            .header("Host", "attacker.example")
            .json(&read)
            .send()
            .unwrap()
            .status(),
        401
    );
    let launch = client.launch_url().unwrap();
    let grant = launch.split("#grant=").nth(1).unwrap();
    let session = format!("http://127.0.0.1:{}/api/session", e.port);
    assert!(http
        .post(&session)
        .json(&json!({"grant":grant}))
        .send()
        .unwrap()
        .status()
        .is_success());
    assert_eq!(
        http.post(&session)
            .json(&json!({"grant":grant}))
            .send()
            .unwrap()
            .status(),
        401
    );
    let before = client
        .call(&req(json!({"op":"export"})), Role::Agent)
        .unwrap();
    drop(service);
    let _restart = start(&dir);
    let after = Client::new(&dir)
        .unwrap()
        .call(
            &Request {
                actor: "test".into(),
                request_id: String::new(),
                command: Command::Export,
            },
            Role::Agent,
        )
        .unwrap();
    assert_eq!(before, after);
}

/// Forwards one HTTP request to the real service, waits for its complete
/// reply, then closes the client connection without relaying it: the write is
/// committed but the caller never learns the outcome (DIR-55).
fn dropping_proxy(service_port: u16) -> u16 {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    fn read_message(stream: &mut TcpStream) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let n = stream.read(&mut buffer).unwrap();
            assert!(n > 0, "connection closed before a complete message");
            bytes.extend_from_slice(&buffer[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .map(|v| v.trim().parse::<usize>().unwrap())
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    return bytes;
                }
            }
        }
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        // The service only accepts its own Host; present it as a direct caller.
        let request = String::from_utf8(read_message(&mut client))
            .unwrap()
            .replace(
                &format!("127.0.0.1:{port}"),
                &format!("127.0.0.1:{service_port}"),
            )
            .into_bytes();
        let mut upstream = TcpStream::connect(("127.0.0.1", service_port)).unwrap();
        upstream.write_all(&request).unwrap();
        let _reply = read_message(&mut upstream);
        drop(client);
    });
    port
}

#[test]
fn client_classifies_write_outcomes_and_an_exact_retry_is_applied_once() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("workspace");
    let service = start(&dir);
    let client = Client::new(&dir).unwrap();
    let create = |request_id: &str| -> Request {
        serde_json::from_value(json!({"actor":"owner","request_id":request_id,"op":"create_issue",
            "product":"DIR","title":"Identical intent","body":"Same text","planning_scope":"inbox"}))
        .unwrap()
    };
    let titled = |snapshot: &Value| {
        snapshot["issues"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|issue| issue["title"] == "Identical intent")
            .count()
    };

    // The reply is dropped after the service committed: outcome unknown.
    let mut dropped = Client::new(&dir).unwrap();
    dropped.endpoint.port = dropping_proxy(client.endpoint.port);
    let failure = dropped
        .call(&create("dir-55-create-1"), Role::Human)
        .expect_err("the dropped reply must not look like success");
    let shaped = direct::command_failure(&failure);
    assert_eq!(shaped["outcome"], "unknown", "{shaped}");
    let snapshot = client
        .call(&req(json!({"op":"snapshot"})), Role::Human)
        .unwrap();
    assert_eq!(
        titled(&snapshot),
        1,
        "the write was committed before the reply was lost"
    );

    // Retrying the exact payload and request ID replays the stored result.
    let retried = client
        .call(&create("dir-55-create-1"), Role::Human)
        .unwrap();
    assert_eq!(retried["key"], "DIR-1");
    let snapshot = client
        .call(&req(json!({"op":"snapshot"})), Role::Human)
        .unwrap();
    assert_eq!(titled(&snapshot), 1, "retry must not create a second issue");

    // A distinct user operation (new request ID) is a distinct record.
    let distinct = client
        .call(&create("dir-55-create-2"), Role::Human)
        .unwrap();
    assert_eq!(distinct["key"], "DIR-2");
    let snapshot = client
        .call(&req(json!({"op":"snapshot"})), Role::Human)
        .unwrap();
    assert_eq!(titled(&snapshot), 2);

    // Reusing a request ID for different work is a definitive refusal.
    let mut changed = create("dir-55-create-1");
    changed.command = serde_json::from_value(json!({"op":"create_issue","product":"DIR",
        "title":"Different","body":"","planning_scope":"inbox"}))
    .unwrap();
    let refused = client.call(&changed, Role::Human).unwrap_err();
    let shaped = direct::command_failure(&refused);
    assert_eq!(shaped["outcome"], "rejected", "{shaped}");
    assert_eq!(shaped["code"], "conflict");

    // A stale version is a definitive refusal that names the conflict.
    let stale = client
        .call(
            &req(
                json!({"op":"update_issue","key":"DIR-1","expected_version":99,
                "title":"Stale","body":"","acceptance":"","owner":"","priority":"low"}),
            ),
            Role::Human,
        )
        .unwrap_err();
    let shaped = direct::command_failure(&stale);
    assert_eq!(shaped["outcome"], "rejected");
    assert_eq!(shaped["code"], "conflict");
    assert!(
        shaped["message"].as_str().unwrap().contains("version"),
        "{shaped}"
    );
    // CLI and MCP keep the established `code: message` text.
    assert!(
        stale.to_string().starts_with("conflict: DIR-1 is version"),
        "{stale}"
    );

    // An unreachable service is an unknown outcome, never a refusal.
    drop(service);
    let gone = client
        .call(&req(json!({"op":"snapshot"})), Role::Human)
        .unwrap_err();
    let shaped = direct::command_failure(&gone);
    assert_eq!(shaped["outcome"], "unknown", "{shaped}");
    assert_eq!(shaped["code"], "unavailable");

    // After restart the persisted state is unchanged: two records, no duplicate.
    let _service = start(&dir);
    let client = Client::new(&dir).unwrap();
    let snapshot = client
        .call(&req(json!({"op":"snapshot"})), Role::Human)
        .unwrap();
    assert_eq!(titled(&snapshot), 2);
    let replay = client
        .call(&create("dir-55-create-1"), Role::Human)
        .unwrap();
    assert_eq!(replay["key"], "DIR-1", "the replay record survives restart");
}
