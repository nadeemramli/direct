use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(store: &mut Store, value: Value, role: Role, at: i64) -> Result<Value> {
    let mut value = value;
    value["actor"] = json!("git-agent");
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    store.execute_at(serde_json::from_value(value).unwrap(), role, at)
}

fn prepared(store: &mut Store, at: i64) -> String {
    let issue = send(
        store,
        json!({"op":"create_issue","product":"DIR","title":"Trace Git delivery","body":"Link Git evidence"}),
        Role::Agent,
        at,
    )
    .unwrap();
    let key = issue["key"].as_str().unwrap().to_string();
    send(
        store,
        json!({"op":"update_issue","key":key,"expected_version":1,"title":"Trace Git delivery","body":"Link Git evidence","acceptance":"Commit and push are visible","owner":"owner","priority":"high"}),
        Role::Agent,
        at + 1,
    )
    .unwrap();
    send(
        store,
        json!({"op":"ready","key":key,"expected_version":2}),
        Role::Human,
        at + 2,
    )
    .unwrap();
    key
}

fn commit(key: &str, version: u64) -> Value {
    json!({
        "op":"record_git_trace",
        "request_id":"commit-trace-1",
        "key":key,
        "expected_version":version,
        "kind":"commit",
        "repository":"nadeemramli/direct",
        "commit_sha":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "branch":"codex/git-trace"
    })
}

fn push(key: &str, version: u64) -> Value {
    json!({
        "op":"record_git_trace",
        "request_id":"push-trace-1",
        "key":key,
        "expected_version":version,
        "kind":"push",
        "repository":"nadeemramli/direct",
        "commit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "branch":"codex/git-trace",
        "remote":"origin",
        "remote_ref":"refs/heads/codex/git-trace"
    })
}

#[test]
fn commit_and_successful_push_are_claimed_idempotent_and_visible() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut store = Store::open(&path).unwrap();
    let key = prepared(&mut store, 100);

    assert_eq!(
        send(&mut store, commit(&key, 3), Role::Agent, 103)
            .unwrap_err()
            .code,
        "claim_required"
    );
    send(
        &mut store,
        json!({"op":"claim","key":key,"expected_version":3}),
        Role::Agent,
        104,
    )
    .unwrap();

    let request = commit(&key, 4);
    let committed = send(&mut store, request.clone(), Role::Agent, 105).unwrap();
    assert_eq!(committed["version"], 5);
    assert_eq!(
        send(&mut store, request, Role::Agent, 106).unwrap(),
        committed
    );

    let mut duplicate = commit(&key, 5);
    duplicate["request_id"] = json!("commit-trace-duplicate");
    assert_eq!(
        send(&mut store, duplicate, Role::Agent, 106)
            .unwrap_err()
            .code,
        "conflict"
    );

    let mut malformed = commit(&key, 5);
    malformed["request_id"] = json!("commit-trace-malformed");
    malformed["commit_sha"] = json!("abc123");
    assert_eq!(
        send(&mut store, malformed, Role::Agent, 106)
            .unwrap_err()
            .code,
        "invalid"
    );

    let mut incomplete_push = push(&key, 5);
    incomplete_push["request_id"] = json!("push-trace-incomplete");
    incomplete_push
        .as_object_mut()
        .unwrap()
        .remove("remote_ref");
    assert_eq!(
        send(&mut store, incomplete_push, Role::Agent, 106)
            .unwrap_err()
            .code,
        "invalid"
    );

    let pushed = send(&mut store, push(&key, 5), Role::Agent, 107).unwrap();
    assert_eq!(pushed["version"], 6);
    let context = send(
        &mut store,
        json!({"op":"context","key":key}),
        Role::Agent,
        108,
    )
    .unwrap();
    let traces = context["git_traces"].as_array().unwrap();
    assert_eq!(traces.len(), 2);
    assert_eq!(traces[0]["kind"], "commit");
    assert_eq!(
        traces[0]["commit_sha"],
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert!(traces[0]["remote"].is_null());
    assert_eq!(traces[1]["kind"], "push");
    assert_eq!(traces[1]["remote"], "origin");
    assert_eq!(traces[1]["remote_ref"], "refs/heads/codex/git-trace");
    assert_eq!(context["history"][0]["kind"], "git_push_recorded");
    assert_eq!(context["history"][1]["kind"], "git_commit_recorded");

    let snapshot = send(&mut store, json!({"op":"snapshot"}), Role::Agent, 109).unwrap();
    assert_eq!(snapshot["git_traces"].as_array().unwrap().len(), 2);

    let submitted = send(
        &mut store,
        json!({"op":"submit","key":key,"expected_version":6,"build_ref":"commit:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","e2e":{"build_ref":"commit:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","environment":"isolated Windows fixture","entrypoint":"fixture client","scenarios":"Exercise the full fixture workflow; expected and observed state transitions match","outcome":"passed","delivered_build_ref":"commit:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","delivery_check":"Fixture service and client use the tested build"},"delivery_ref":"origin/codex/git-trace","summary":"Git trace implemented","checks":"passed","steps":[{"instruction":"Open Activity","expected":"Commit and push appear"}]}),
        Role::Agent,
        110,
    )
    .unwrap();
    let child = submitted["verification_key"].as_str().unwrap();
    let child_context = send(
        &mut store,
        json!({"op":"context","key":child}),
        Role::Agent,
        111,
    )
    .unwrap();
    let mut child_trace = commit(child, child_context["issue"]["version"].as_u64().unwrap());
    child_trace["request_id"] = json!("child-trace");
    assert_eq!(
        send(&mut store, child_trace, Role::Agent, 112)
            .unwrap_err()
            .code,
        "invalid"
    );

    let archive = store.export().unwrap();
    assert_eq!(archive.format, 20);
    assert_eq!(archive.git_traces.len(), 2);
    validate_archive(&archive).unwrap();
    let before = serde_json::to_value(&archive).unwrap();
    drop(store);

    let reopened = Store::open(&path).unwrap();
    assert_eq!(
        serde_json::to_value(reopened.export().unwrap()).unwrap(),
        before
    );
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(archive).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        before
    );
}

#[test]
fn old_archives_default_to_no_git_traces_and_malformed_trace_archives_fail() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let mut old = serde_json::to_value(store.export().unwrap()).unwrap();
    old["format"] = json!(3);
    old.as_object_mut().unwrap().remove("git_traces");
    let old: Archive = serde_json::from_value(old).unwrap();
    assert!(old.git_traces.is_empty());
    validate_archive(&old).unwrap();

    let mut broken = store.export().unwrap();
    broken.git_traces.push(GitTrace {
        id: uuid::Uuid::new_v4().to_string(),
        issue_key: "DIR-404".into(),
        kind: GitTraceKind::Push,
        repository: "nadeemramli/direct".into(),
        commit_sha: "a".repeat(40),
        branch: "main".into(),
        remote: Some("origin".into()),
        remote_ref: Some("refs/heads/main".into()),
        recorded_by: "git-agent".into(),
        recorded_at: 10,
    });
    assert_eq!(validate_archive(&broken).unwrap_err().code, "invalid");
}
