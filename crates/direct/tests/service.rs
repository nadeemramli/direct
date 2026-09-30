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
