//! DIR-56: archive restoration must reject malformed verification
//! relationships, and review must never panic (and poison the service) on an
//! inconsistent store. Every workspace here is an isolated synthetic fixture
//! built through the real service with synthetic agent and owner actors.
use direct::Client;
use direct_core::{Request, Role};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::{Child, Command as Process, Output, Stdio},
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
fn cli(dir: &Path, args: &[&str]) -> Output {
    Process::new(env!("CARGO_BIN_EXE_direct"))
        .arg("--data-dir")
        .arg(dir)
        .args(args)
        .output()
        .unwrap()
}
fn call(client: &Client, actor: &str, role: Role, mut value: Value) -> anyhow::Result<Value> {
    value["actor"] = json!(actor);
    value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    let request: Request = serde_json::from_value(value).unwrap();
    client.call(&request, role)
}
fn agent(client: &Client, value: Value) -> Value {
    call(client, "synthetic-agent", Role::Agent, value).unwrap()
}
fn owner(client: &Client, value: Value) -> Value {
    call(client, "owner", Role::Human, value).unwrap()
}
const BUILD: &str = "commit:0123456789abcdef0123456789abcdef01234567";

fn submit(client: &Client, key: &str, version: u64, summary: &str) -> Value {
    agent(
        client,
        json!({"op":"submit","key":key,"expected_version":version,"build_ref":BUILD,
          "delivery_ref":"claude/synthetic","summary":summary,"checks":"synthetic checks passed",
          "e2e":{"build_ref":BUILD,"delivered_build_ref":BUILD,"environment":"isolated fixture",
                 "entrypoint":"fixture service","scenarios":"synthetic scenario observed","outcome":"passed",
                 "delivery_check":"fixture build launched"},
          "steps":[{"instruction":"Open the issue","expected":"It renders"},
                   {"instruction":"Run the flow","expected":"It completes"}]}),
    )
}
fn review(client: &Client, issue: &Value, outcome: &str, steps: [&str; 2], note: &str) -> Value {
    owner(
        client,
        json!({"op":"review","key":issue["key"],"expected_version":issue["version"],
          "run_id":issue["current_run"],"outcome":outcome,
          "results":[{"outcome":steps[0],"note":""},{"outcome":steps[1],"note":"synthetic"}],
          "note":note}),
    )
}
fn ready_issue(client: &Client, title: &str) -> Value {
    let created = agent(
        client,
        json!({"op":"create_issue","product":"DIR","title":title,"body":"Synthetic body",
          "acceptance":"Synthetic acceptance","owner":"synthetic-owner","planning_scope":"inbox"}),
    );
    owner(
        client,
        json!({"op":"ready","key":created["key"],"expected_version":created["version"]}),
    )
}
fn claim(client: &Client, issue: &Value) -> Value {
    agent(
        client,
        json!({"op":"claim","key":issue["key"],"expected_version":issue["version"],"lease_seconds":3600}),
    )
}

/// Builds the fixture through the real service and returns its export:
/// DIR-1 submit → fail → resubmit → pass (Done, two runs);
/// a parent left in Verify with a pending run; and a reopened parent whose
/// verification child is Canceled with no current run.
fn lifecycle(dir: &Path) -> Value {
    let _service = start(dir);
    let client = Client::new(dir).unwrap();

    let done = ready_issue(&client, "Submit, fail, resubmit, pass");
    let done = claim(&client, &done);
    let done = submit(
        &client,
        done["key"].as_str().unwrap(),
        done["version"].as_u64().unwrap(),
        "first run",
    );
    let failed = review(
        &client,
        &done,
        "failed",
        ["passed", "failed"],
        "Step two failed",
    );
    assert_eq!(failed["status"], "doing");
    assert!(failed["needs_fix"].as_bool().unwrap());
    let done = claim(&client, &failed);
    let done = submit(
        &client,
        done["key"].as_str().unwrap(),
        done["version"].as_u64().unwrap(),
        "second run",
    );
    let done = review(&client, &done, "passed", ["passed", "passed"], "");
    assert_eq!(done["status"], "done");

    let pending = ready_issue(&client, "Left awaiting review");
    let pending = claim(&client, &pending);
    let pending = submit(
        &client,
        pending["key"].as_str().unwrap(),
        pending["version"].as_u64().unwrap(),
        "pending run",
    );
    assert_eq!(pending["status"], "verify");

    let reopened = ready_issue(&client, "Submitted then reopened");
    let reopened = claim(&client, &reopened);
    let reopened = submit(
        &client,
        reopened["key"].as_str().unwrap(),
        reopened["version"].as_u64().unwrap(),
        "reopened run",
    );
    let reopened = owner(
        &client,
        json!({"op":"reopen","key":reopened["key"],"expected_version":reopened["version"],"reason":"Needs another pass"}),
    );
    assert_eq!(reopened["status"], "doing");
    assert!(reopened["current_run"].is_null());

    owner(&client, json!({"op":"export"}))
}

fn issue_index(archive: &Value, key: &str) -> usize {
    archive["issues"]
        .as_array()
        .unwrap()
        .iter()
        .position(|issue| issue["key"] == key)
        .unwrap_or_else(|| panic!("{key} missing from fixture"))
}
fn issue<'a>(archive: &'a mut Value, key: &str) -> &'a mut Value {
    let index = issue_index(archive, key);
    &mut archive["issues"][index]
}
fn find(archive: &Value, title: &str) -> Value {
    archive["issues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|issue| issue["title"] == title)
        .unwrap()
        .clone()
}
fn runs_for(archive: &Value, key: &str) -> Vec<Value> {
    archive["verifications"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|run| run["issue_key"] == key)
        .cloned()
        .collect()
}

struct Keys {
    done: String,
    done_child: String,
    pending: String,
    pending_child: String,
    reopened: String,
    reopened_child: String,
    first_done_run: String,
    pending_run: String,
}
fn keys(archive: &Value) -> Keys {
    let done = find(archive, "Submit, fail, resubmit, pass");
    let pending = find(archive, "Left awaiting review");
    let reopened = find(archive, "Submitted then reopened");
    let done_runs = runs_for(archive, done["key"].as_str().unwrap());
    assert_eq!(done_runs.len(), 2);
    let first_done_run = done_runs
        .iter()
        .find(|run| run["outcome"] == "failed")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let text = |v: &Value| v.as_str().unwrap().to_string();
    Keys {
        done: text(&done["key"]),
        done_child: text(&done["verification_key"]),
        pending: text(&pending["key"]),
        pending_child: text(&pending["verification_key"]),
        reopened: text(&reopened["key"]),
        reopened_child: text(&reopened["verification_key"]),
        first_done_run,
        pending_run: text(&pending["current_run"]),
    }
}

type Corruption = fn(&mut Value, &Keys);
/// Each case is a distinct malformed relationship and the fragment the
/// refusal must name so the operator can locate it.
fn malformed_cases() -> Vec<(&'static str, Corruption, &'static str)> {
    vec![
        (
            "submitted parent without its verification child",
            |a, k| {
                issue(a, &k.pending)["verification_key"] = Value::Null;
                // Leave the child in place: it now points at a parent that does not own it.
            },
            "verification child",
        ),
        (
            "submitted parent and child both missing",
            |a, k| {
                issue(a, &k.pending)["verification_key"] = Value::Null;
                let child = issue_index(a, &k.pending_child);
                a["issues"].as_array_mut().unwrap().remove(child);
            },
            "verification child",
        ),
        (
            "dangling verification child key",
            |a, k| {
                issue(a, &k.pending)["verification_key"] = json!("DIR-999");
            },
            "relationship",
        ),
        (
            "parent points at another parent's child",
            |a, k| {
                issue(a, &k.pending)["verification_key"] = json!(k.done_child);
            },
            "verification child",
        ),
        (
            "child names a different parent",
            |a, k| {
                issue(a, &k.pending_child)["parent"] = json!(k.done);
            },
            "verification child",
        ),
        (
            "second child claims the same parent",
            |a, k| {
                issue(a, &k.reopened_child)["parent"] = json!(k.done);
                issue(a, &k.reopened)["verification_key"] = Value::Null;
            },
            "verification child",
        ),
        (
            "child of a child",
            |a, k| {
                issue(a, &k.pending_child)["verification_key"] = json!(k.done_child);
            },
            "verification child",
        ),
        (
            "current run belongs to another issue",
            |a, k| {
                let parent = issue(a, &k.done);
                parent["status"] = json!("doing");
                parent["current_run"] = json!(k.pending_run);
            },
            "run",
        ),
        (
            "child current run differs from its parent's",
            |a, k| {
                issue(a, &k.pending_child)["current_run"] = json!(k.first_done_run);
            },
            "run",
        ),
        (
            "child current run is dangling",
            |a, k| {
                issue(a, &k.pending_child)["current_run"] =
                    json!("00000000-0000-4000-8000-000000000000");
            },
            "relationship",
        ),
        (
            "pending run on a Doing parent",
            |a, k| {
                issue(a, &k.pending)["status"] = json!("doing");
            },
            "run",
        ),
        (
            "child status disagrees with the current run",
            |a, k| {
                issue(a, &k.pending_child)["status"] = json!("done");
            },
            "status",
        ),
        (
            "Done parent with an unfinished child",
            |a, k| {
                issue(a, &k.done_child)["status"] = json!("ready");
            },
            "status",
        ),
        (
            "reopened parent with an active child",
            |a, k| {
                issue(a, &k.reopened_child)["status"] = json!("ready");
            },
            "status",
        ),
        (
            "run recorded against a verification child",
            |a, k| {
                let index = a["verifications"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .position(|run| run["id"] == k.first_done_run.as_str())
                    .unwrap();
                a["verifications"][index]["issue_key"] = json!(k.done_child);
            },
            "run",
        ),
        (
            "run history without a verification child",
            |a, k| {
                issue(a, &k.reopened)["verification_key"] = Value::Null;
                let child = issue_index(a, &k.reopened_child);
                a["issues"].as_array_mut().unwrap().remove(child);
            },
            "verification child",
        ),
    ]
}

#[test]
fn restore_rejects_malformed_verification_relationships_before_creating_the_destination() {
    let temp = tempfile::tempdir().unwrap();
    let archive = lifecycle(&temp.path().join("source"));
    let k = keys(&archive);

    // The unmodified export is a supported archive.
    let valid = temp.path().join("valid.json");
    fs::write(&valid, serde_json::to_vec_pretty(&archive).unwrap()).unwrap();
    let restored = temp.path().join("valid-restore");
    let output = cli(&restored, &["restore", valid.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut failures = Vec::new();
    for (index, (name, corrupt, fragment)) in malformed_cases().into_iter().enumerate() {
        let mut malformed = archive.clone();
        corrupt(&mut malformed, &k);
        let path = temp.path().join(format!("malformed-{index}.json"));
        fs::write(&path, serde_json::to_vec_pretty(&malformed).unwrap()).unwrap();

        let destination = temp.path().join(format!("restore-{index}"));
        let restore = cli(&destination, &["restore", path.to_str().unwrap()]);
        let drill = temp.path().join(format!("drill-{index}"));
        let check = cli(
            &temp.path().join("unused"),
            &[
                "recovery-check",
                path.to_str().unwrap(),
                drill.to_str().unwrap(),
            ],
        );
        let message = String::from_utf8_lossy(&restore.stderr).to_string();
        let check_message = String::from_utf8_lossy(&check.stderr).to_string();
        println!("{name}: {}", message.trim());
        let ok = !restore.status.success()
            && !check.status.success()
            && !destination.exists()
            && !drill.exists()
            && message.to_lowercase().contains(fragment)
            && message == check_message;
        if !ok {
            failures.push(format!(
                "{name}: restore exit {:?} ({}), recovery-check exit {:?} ({}), destination created {}, drill created {}",
                restore.status.code(),
                message.trim(),
                check.status.code(),
                check_message.trim(),
                destination.exists(),
                drill.exists()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "malformed archives were not refused cleanly:\n{}",
        failures.join("\n")
    );
}

#[test]
fn valid_fail_resubmit_pass_round_trip_preserves_history_and_distinct_outcomes() {
    let temp = tempfile::tempdir().unwrap();
    let archive = lifecycle(&temp.path().join("source"));
    let k = keys(&archive);
    let path = temp.path().join("lifecycle.json");
    fs::write(&path, serde_json::to_vec_pretty(&archive).unwrap()).unwrap();

    let drill = temp.path().join("drill");
    let check = cli(
        &temp.path().join("unused"),
        &[
            "recovery-check",
            path.to_str().unwrap(),
            drill.to_str().unwrap(),
        ],
    );
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let report: Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(report["semantic_archive_match"], true, "{report}");

    let restored = temp.path().join("restored");
    let output = cli(&restored, &["restore", path.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _service = start(&restored);
    let client = Client::new(&restored).unwrap();
    let context = owner(&client, json!({"op":"context","key":k.done}));
    let runs = context["verifications"].as_array().unwrap();
    let outcomes: Vec<_> = runs
        .iter()
        .map(|run| run["outcome"].as_str().unwrap())
        .collect();
    assert_eq!(runs.len(), 2, "{outcomes:?}");
    assert!(
        outcomes.contains(&"failed") && outcomes.contains(&"passed"),
        "{outcomes:?}"
    );
    assert!(runs
        .iter()
        .any(|run| run["review_note"] == "Step two failed" && run["summary"] == "first run"));
    assert!(runs
        .iter()
        .any(|run| run["outcome"] == "passed" && run["summary"] == "second run"));
    let history: Vec<_> = context["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["kind"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        history
            .iter()
            .filter(|kind| *kind == "verification_requested")
            .count(),
        2,
        "{history:?}"
    );
    assert_eq!(
        history
            .iter()
            .filter(|kind| *kind == "verification_reviewed")
            .count(),
        2,
        "{history:?}"
    );

    // The restored pending run can still be reviewed by the owner, distinctly
    // from the agent's first-pass evidence.
    let pending = owner(&client, json!({"op":"context","key":k.pending}))["issue"].clone();
    assert_eq!(pending["current_run"], k.pending_run.as_str());
    let canceled = review(
        &client,
        &pending,
        "canceled",
        ["pending", "pending"],
        "Superseded",
    );
    assert_eq!(canceled["status"], "doing");
    let child = owner(&client, json!({"op":"context","key":k.pending_child}));
    assert_eq!(child["issue"]["status"], "canceled");
    let run = &owner(&client, json!({"op":"context","key":k.pending}))["verifications"][0];
    assert_eq!(run["outcome"], "canceled");
    assert_eq!(run["e2e"]["outcome"], "passed");
    assert_eq!(run["reviewed_by"], "owner");

    // A second export of the restored, further-reviewed workspace still validates.
    let again = owner(&client, json!({"op":"export"}));
    let again_path = temp.path().join("again.json");
    fs::write(&again_path, serde_json::to_vec_pretty(&again).unwrap()).unwrap();
    let check = cli(
        &temp.path().join("unused"),
        &[
            "recovery-check",
            again_path.to_str().unwrap(),
            temp.path().join("drill-again").to_str().unwrap(),
        ],
    );
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn review_of_an_inconsistent_store_is_refused_and_the_service_keeps_serving() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("source");
    let archive = lifecycle(&dir);
    let k = keys(&archive);

    // Simulate a store that reached this state without restoration (for
    // example an older build's restore): tamper with the isolated fixture
    // database while its service is stopped.
    {
        let db = rusqlite::Connection::open(dir.join("direct.db")).unwrap();
        let data: String = db
            .query_row("SELECT data FROM issues WHERE key=?1", [&k.pending], |r| {
                r.get(0)
            })
            .unwrap();
        let mut value: Value = serde_json::from_str(&data).unwrap();
        value["verification_key"] = Value::Null;
        db.execute(
            "UPDATE issues SET data=?1 WHERE key=?2",
            rusqlite::params![value.to_string(), k.pending],
        )
        .unwrap();
    }

    let _service = start(&dir);
    let client = Client::new(&dir).unwrap();
    let pending = owner(&client, json!({"op":"context","key":k.pending}))["issue"].clone();
    let refused = call(
        &client,
        "owner",
        Role::Human,
        json!({"op":"review","key":k.pending,"expected_version":pending["version"],
          "run_id":k.pending_run,"outcome":"failed",
          "results":[{"outcome":"passed","note":""},{"outcome":"failed","note":""}],
          "note":"Synthetic failure"}),
    )
    .expect_err("review of an inconsistent issue must be refused");
    let message = refused.to_string();
    assert!(
        message.starts_with("invalid:") && message.contains("verification child"),
        "{message}"
    );

    // The service is not poisoned: list/context/export and other writes still work.
    let snapshot = owner(&client, json!({"op":"snapshot"}));
    assert!(snapshot["issues"].as_array().unwrap().len() >= 6);
    let context = owner(&client, json!({"op":"context","key":k.done}));
    assert_eq!(context["issue"]["status"], "done");
    let after = owner(&client, json!({"op":"context","key":k.pending}));
    assert_eq!(after["issue"]["status"], "verify");
    assert_eq!(
        after["verifications"][0]["outcome"], "pending",
        "refused review changed nothing"
    );
    owner(
        &client,
        json!({"op":"comment","key":k.done,"expected_version":context["issue"]["version"],"body":"Still writable"}),
    );
    let listed = cli(&dir, &["--actor", "synthetic-agent", "list"]);
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
}
