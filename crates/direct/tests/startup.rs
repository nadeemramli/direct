//! Service liveness and startup on isolated synthetic workspaces (DIR-60).
use direct::Client;
use direct_core::{Command, Request, Role};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command as Process, Stdio},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
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
    value["actor"] = json!("startup-agent");
    value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    serde_json::from_value(value).unwrap()
}
fn export(dir: &Path) -> Value {
    Client::new(dir)
        .unwrap()
        .call(
            &Request {
                actor: "startup-agent".into(),
                request_id: String::new(),
                command: Command::Export,
            },
            Role::Agent,
        )
        .unwrap()
}

/// Keeps the store deliberately busy without bulk data: an outside connection
/// holds SQLite's write lock, so each queued write waits inside the service
/// for the store's busy timeout while holding the store lock.
struct Busy {
    db: Option<rusqlite::Connection>,
    writers: Vec<JoinHandle<Value>>,
}
fn occupy_store(dir: &Path, writers: usize) -> Busy {
    let db = rusqlite::Connection::open(dir.join("direct.db")).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let writers = (0..writers)
        .map(|i| {
            let dir: PathBuf = dir.to_path_buf();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(50 * i as u64));
                let create = req(
                    json!({"op":"create_issue","product":"DIR","title":"Queued while busy","body":"synthetic"}),
                );
                let result = Client::new(&dir).unwrap().call(&create, Role::Agent);
                match result {
                    Ok(value) => value,
                    Err(error) => direct::command_failure(&error),
                }
            })
        })
        .collect();
    // Let the first write reach the store and start waiting.
    thread::sleep(Duration::from_millis(500));
    Busy {
        db: Some(db),
        writers,
    }
}
impl Busy {
    /// Let the first write run out its busy timeout, then release the outside
    /// lock and collect every queued write's actual result.
    fn finish(mut self) -> Vec<Value> {
        let mut writers = self.writers.drain(..);
        let first = writers.next().unwrap().join().unwrap();
        if let Some(db) = self.db.take() {
            db.execute_batch("ROLLBACK").unwrap();
        }
        std::iter::once(first)
            .chain(writers.map(|writer| writer.join().unwrap()))
            .collect()
    }
}

fn seeded(dir: &Path) -> Value {
    Client::new(dir)
        .unwrap()
        .call(
            &req(json!({"op":"create_issue","product":"DIR","title":"Synthetic startup fixture","body":"Must survive"})),
            Role::Agent,
        )
        .unwrap();
    export(dir)
}

/// A launcher that must not be used: any attempt to start it fails loudly.
fn no_launch(temp: &Path) -> PathBuf {
    temp.join("must-not-launch.exe")
}
/// Starts the service through `start_service` and stops it on drop.
struct Started(Option<Child>);
impl Drop for Started {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
fn launch(dir: &Path, assets: &Path) -> (Started, Duration) {
    let started = Instant::now();
    let child = direct::start_service(
        dir,
        Path::new(env!("CARGO_BIN_EXE_direct")),
        assets,
        direct::STARTUP_DEADLINE,
    )
    .unwrap();
    (Started(child), started.elapsed())
}
/// A local HTTP responder that is not Direct, answering every request with
/// the given status line and body.
fn impostor(status: &'static str, body: &'static str) -> u16 {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    port
}
fn rewrite_endpoint(dir: &Path, change: impl FnOnce(&mut Value)) -> Vec<u8> {
    let original = std::fs::read(dir.join("endpoint.json")).unwrap();
    let mut value: Value = serde_json::from_slice(&original).unwrap();
    change(&mut value);
    std::fs::write(
        dir.join("endpoint.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    original
}

#[test]
fn a_busy_store_is_not_mistaken_for_a_dead_service() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("workspace");
    let _service = start(&dir);
    let before = seeded(&dir);
    let busy = occupy_store(&dir, 3);

    let started = Instant::now();
    let result = direct::ensure_service(&dir);
    let elapsed = started.elapsed();
    eprintln!("ensure_service while busy: {result:?} after {elapsed:?}");
    let probe = Instant::now();
    let liveness = Client::new(&dir).unwrap().liveness(Duration::from_secs(1));
    let probe = probe.elapsed();
    // No launcher exists, so any attempt to start a competing service fails.
    let started = Instant::now();
    let reused = direct::start_service(
        &dir,
        &no_launch(temp.path()),
        temp.path(),
        Duration::from_secs(5),
    );
    let reuse_elapsed = started.elapsed();
    assert!(
        direct::service_running(&dir).unwrap(),
        "lock held throughout"
    );

    let writes = busy.finish();
    assert!(result.is_ok(), "{result:?} after {elapsed:?}");
    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
    assert_eq!(liveness, direct::Liveness::Alive);
    assert!(probe < Duration::from_secs(1), "probe took {probe:?}");
    assert!(
        matches!(reused, Ok(None)),
        "a busy service is reused, never relaunched: {:?}",
        reused.map(|c| c.is_some())
    );
    assert!(
        reuse_elapsed < Duration::from_secs(2),
        "took {reuse_elapsed:?}"
    );
    // Ordinary writes keep their real semantics: the one held past the busy
    // timeout fails with an unknown outcome, and any still queued when the
    // store frees up is applied exactly once.
    let failed: Vec<_> = writes.iter().filter(|w| w.get("code").is_some()).collect();
    assert!(!failed.is_empty(), "{writes:?}");
    for write in &failed {
        assert_eq!(write["outcome"], "unknown", "{write}");
    }
    let applied = writes.len() - failed.len();
    let after = export(&dir);
    let queued = |archive: &Value| {
        archive["issues"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|issue| issue["title"] == "Queued while busy")
            .count()
    };
    assert_eq!(queued(&after), applied, "{writes:?}");
    assert_eq!(queued(&before), 0);
    assert!(after["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["title"] == "Synthetic startup fixture"));
}

#[test]
fn a_dead_service_restarts_from_its_stale_endpoint_with_data_intact() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("workspace");
    let service = start(&dir);
    let before = seeded(&dir);
    let old = direct::endpoint(&dir).unwrap();
    drop(service);
    // The endpoint record survives the process: it is now stale.
    assert!(dir.join("endpoint.json").exists());
    assert!(!direct::service_running(&dir).unwrap());
    assert_eq!(
        Client::new(&dir).unwrap().liveness(Duration::from_secs(1)),
        direct::Liveness::Unreachable
    );

    let (_restarted, elapsed) = launch(&dir, temp.path());
    assert!(elapsed < direct::STARTUP_DEADLINE, "took {elapsed:?}");
    let new = direct::endpoint(&dir).unwrap();
    assert_ne!(new.agent_token, old.agent_token, "a fresh service answered");
    assert!(direct::service_running(&dir).unwrap());
    assert_eq!(export(&dir), before, "persisted workspace unchanged");
}

#[test]
fn a_stale_endpoint_answered_by_another_program_starts_the_real_service() {
    for (status, body) in [
        ("200 OK", "{}"),
        ("401 Unauthorized", r#"{"code":"unauthorized"}"#),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("workspace");
        let service = start(&dir);
        let before = seeded(&dir);
        drop(service);
        let port = impostor(status, body);
        rewrite_endpoint(&dir, |e| e["port"] = json!(port));
        let expected = if status.starts_with("200") {
            direct::Liveness::Unreachable
        } else {
            direct::Liveness::Refused
        };
        assert_eq!(
            Client::new(&dir).unwrap().liveness(Duration::from_secs(1)),
            expected,
            "{status}"
        );
        assert!(!Client::new(&dir).unwrap().healthy());

        let (_started, _) = launch(&dir, temp.path());
        assert_ne!(direct::endpoint(&dir).unwrap().port, port, "{status}");
        assert_eq!(export(&dir), before, "{status}");
    }
}

#[test]
fn wrong_credentials_for_a_running_service_fail_fast_without_a_duplicate() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("workspace");
    let _service = start(&dir);
    let before = seeded(&dir);
    let original = rewrite_endpoint(&dir, |e| {
        e["agent_token"] = json!("not-the-agent-token");
        e["owner_token"] = json!("not-the-owner-token");
    });
    assert_eq!(
        Client::new(&dir).unwrap().liveness(Duration::from_secs(1)),
        direct::Liveness::Refused
    );
    let deadline = Duration::from_secs(2);
    let started = Instant::now();
    let error = direct::start_service(&dir, &no_launch(temp.path()), temp.path(), deadline)
        .map(|_| ())
        .unwrap_err()
        .to_string();
    let elapsed = started.elapsed();
    assert!(error.contains("refused the credentials"), "{error}");
    assert!(elapsed >= deadline, "kept waiting for the record to settle");
    assert!(
        elapsed < deadline + Duration::from_secs(2),
        "took {elapsed:?}"
    );

    std::fs::write(dir.join("endpoint.json"), original).unwrap();
    assert!(Client::new(&dir).unwrap().healthy());
    assert_eq!(export(&dir), before);
}

#[test]
fn liveness_requires_local_credentials_and_reveals_nothing_else() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("workspace");
    let _service = start(&dir);
    seeded(&dir);
    let e = direct::endpoint(&dir).unwrap();
    let http = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .unwrap();
    let url = format!("http://127.0.0.1:{}/api/health", e.port);
    let alive = json!({"service":"direct","status":"alive"});
    let session = {
        let launch = Client::new(&dir).unwrap().launch_url().unwrap();
        let grant = launch.split("#grant=").nth(1).unwrap().to_string();
        let reply: Value = http
            .post(format!("http://127.0.0.1:{}/api/session", e.port))
            .json(&json!({ "grant": grant }))
            .send()
            .unwrap()
            .json()
            .unwrap();
        reply["token"].as_str().unwrap().to_string()
    };
    for token in [&e.agent_token, &e.owner_token, &session] {
        let reply = http.post(&url).bearer_auth(token).send().unwrap();
        assert_eq!(reply.status(), 200);
        assert_eq!(reply.headers()["cache-control"], "no-store", "never cached");
        assert_eq!(reply.json::<Value>().unwrap(), alive, "exactly this body");
    }
    let denied = [
        http.post(&url).send().unwrap(),
        http.post(&url).bearer_auth("guess").send().unwrap(),
        http.post(&url)
            .bearer_auth(&e.agent_token)
            .header("Origin", "https://example.com")
            .send()
            .unwrap(),
        http.post(&url)
            .bearer_auth(&e.agent_token)
            .header("Host", "attacker.example")
            .send()
            .unwrap(),
    ];
    for reply in denied {
        assert_eq!(reply.status(), 401);
        let body = reply.text().unwrap();
        assert!(
            !body.contains("alive") && !body.contains("Synthetic"),
            "{body}"
        );
    }
}

#[test]
fn a_service_that_cannot_start_reports_promptly_and_leaves_data_alone() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("workspace");
    std::fs::create_dir_all(&dir).unwrap();
    let corrupt = b"synthetic bytes that are not a SQLite database".repeat(64);
    std::fs::write(dir.join("direct.db"), &corrupt).unwrap();
    let started = Instant::now();
    let error = direct::start_service(
        &dir,
        Path::new(env!("CARGO_BIN_EXE_direct")),
        temp.path(),
        direct::STARTUP_DEADLINE,
    )
    .map(|_| ())
    .unwrap_err()
    .to_string();
    let elapsed = started.elapsed();
    assert!(error.contains("exited during startup"), "{error}");
    assert!(elapsed < Duration::from_secs(10), "took {elapsed:?}");
    assert!(!direct::service_running(&dir).unwrap());
    assert_eq!(std::fs::read(dir.join("direct.db")).unwrap(), corrupt);
}

#[test]
fn a_silent_lock_holder_is_bounded_by_one_total_deadline() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("workspace");
    std::fs::create_dir_all(&dir).unwrap();
    // Something holds the service lock but never answers: a hung or still
    // starting service whose state stays unknown. No endpoint is recorded.
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("service.lock"))
        .unwrap();
    lock.lock().unwrap();
    assert!(direct::service_running(&dir).unwrap());
    let deadline = Duration::from_secs(2);
    let started = Instant::now();
    let error = direct::start_service(&dir, &no_launch(temp.path()), temp.path(), deadline)
        .map(|_| ())
        .unwrap_err()
        .to_string();
    let elapsed = started.elapsed();
    assert!(error.contains("did not answer within 2 seconds"), "{error}");
    assert!(elapsed >= deadline, "took {elapsed:?}");
    // One total deadline plus at most one short final probe.
    assert!(
        elapsed < deadline + Duration::from_millis(600),
        "took {elapsed:?}"
    );
    lock.unlock().unwrap();
}
