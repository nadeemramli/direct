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
