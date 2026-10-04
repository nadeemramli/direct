//! DIR-86: owner cancellation of non-deliverable work, with retained history.
use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(s: &mut Store, mut value: Value, role: Role, at: i64) -> Result<Value> {
    value["actor"] = json!(if role == Role::Human {
        "owner"
    } else {
        "agent-1"
    });
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    s.execute_at(serde_json::from_value(value).unwrap(), role, at)
}
fn ready(s: &mut Store, title: &str) -> String {
    let i = send(
        s,
        json!({"op":"create_issue","product":"DIR","title":title,"planning_scope":"inbox"}),
        Role::Agent,
        100,
    )
    .unwrap();
    let key = i["key"].as_str().unwrap().to_string();
    send(s, json!({"op":"update_issue","key":key,"expected_version":1,"title":title,"body":"b","acceptance":"a","owner":"Owner","priority":"medium"}), Role::Agent, 101).unwrap();
    send(
        s,
        json!({"op":"ready","key":key,"expected_version":2}),
        Role::Human,
        102,
    )
    .unwrap();
    key
}
fn version(s: &mut Store, key: &str) -> u64 {
    send(s, json!({"op":"context","key":key}), Role::Agent, 103).unwrap()["issue"]["version"]
        .as_u64()
        .unwrap()
}
fn cancel(key: &str, v: u64, release: bool) -> Value {
    json!({"op":"cancel_issue","key":key,"expected_version":v,"reason":"Test issue; nothing to deliver","release_active_claim":release})
}

#[test]
fn owner_cancels_with_reason_and_history_is_kept() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let key = ready(&mut s, "Archive me");
    let other = ready(&mut s, "Linked work");
    let v = version(&mut s, &key);
    send(
        &mut s,
        json!({"op":"comment","key":key,"expected_version":v,"body":"Keep this comment"}),
        Role::Agent,
        104,
    )
    .unwrap();
    let v = version(&mut s, &key);
    send(&mut s, json!({"op":"create_issue_link","key":key,"expected_version":v,"target_key":other,"kind":"related"}), Role::Agent, 105).unwrap();
    let v = version(&mut s, &key);

    assert_eq!(
        send(&mut s, cancel(&key, v, false), Role::Agent, 106)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        send(
            &mut s,
            json!({"op":"cancel_issue","key":key,"expected_version":v,"reason":"   "}),
            Role::Human,
            106
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    let mut request = cancel(&key, v, false);
    request["request_id"] = json!("cancel-1");
    let canceled = send(&mut s, request.clone(), Role::Human, 107).unwrap();
    // An exact retry replays; a stale version conflicts; nothing is applied twice.
    assert_eq!(send(&mut s, request, Role::Human, 108).unwrap(), canceled);
    assert_eq!(
        send(&mut s, cancel(&key, v, false), Role::Human, 108)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(canceled["status"], "canceled");
    assert_eq!(canceled["version"], v + 1);

    let ctx = send(&mut s, json!({"op":"context","key":key}), Role::Agent, 109).unwrap();
    let comments: Vec<&str> = ctx["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["body"].as_str().unwrap())
        .collect();
    assert!(comments.contains(&"Keep this comment"));
    assert!(comments.contains(&"Canceled: Test issue; nothing to deliver"));
    assert_eq!(ctx["issue_links"].as_array().unwrap().len(), 1);
    let kinds = send(&mut s, json!({"op":"changes","after":0}), Role::Agent, 109).unwrap()
        ["events"]
        .to_string();
    assert!(kinds.contains("issue_canceled"));
    // Canceled again is refused; agents cannot claim canceled work.
    assert_eq!(
        send(&mut s, cancel(&key, v + 1, false), Role::Human, 110)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        send(
            &mut s,
            json!({"op":"claim","key":key,"expected_version":v+1,"lease_seconds":600}),
            Role::Agent,
            110
        )
        .unwrap_err()
        .code,
        "invalid"
    );
}

#[test]
fn active_claims_need_explicit_release_and_verify_or_completed_work_is_refused() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let key = ready(&mut s, "Claimed work");
    let v = version(&mut s, &key);
    send(
        &mut s,
        json!({"op":"claim","key":key,"expected_version":v,"lease_seconds":3600}),
        Role::Agent,
        200,
    )
    .unwrap();
    let v = version(&mut s, &key);
    let refused = send(&mut s, cancel(&key, v, false), Role::Human, 201).unwrap_err();
    assert_eq!(refused.code, "conflict");
    assert!(refused.message.contains("agent-1 holds an active claim"));
    // Nothing changed after the refusal.
    assert_eq!(version(&mut s, &key), v);
    let canceled = send(&mut s, cancel(&key, v, true), Role::Human, 202).unwrap();
    assert_eq!(canceled["claim"], Value::Null);
    let ctx = send(&mut s, json!({"op":"context","key":key}), Role::Agent, 203).unwrap();
    assert!(ctx["comments"]
        .to_string()
        .contains("Released the active claim held by agent-1"));

    // An expired claim does not need the release flag.
    let expired = ready(&mut s, "Expired claim");
    let v = version(&mut s, &expired);
    send(
        &mut s,
        json!({"op":"claim","key":expired,"expected_version":v,"lease_seconds":60}),
        Role::Agent,
        300,
    )
    .unwrap();
    let v = version(&mut s, &expired);
    send(&mut s, cancel(&expired, v, false), Role::Human, 500).unwrap();

    // Submitted work must be reviewed or reopened first; the run stays pending.
    let submitted = ready(&mut s, "Submitted work");
    let v = version(&mut s, &submitted);
    send(
        &mut s,
        json!({"op":"claim","key":submitted,"expected_version":v,"lease_seconds":3600}),
        Role::Agent,
        600,
    )
    .unwrap();
    let v = version(&mut s, &submitted);
    let sub = json!({"op":"submit","key":submitted,"expected_version":v,"build_ref":"commit:abc","delivery_ref":"main","summary":"s","checks":"c",
        "e2e":{"build_ref":"commit:abc","delivered_build_ref":"commit:abc","environment":"fixture","entrypoint":"fixture","scenarios":"all pass","outcome":"passed","delivery_check":"installed"},
        "steps":[{"instruction":"i","expected":"e"}]});
    send(&mut s, sub, Role::Agent, 601).unwrap();
    let v = version(&mut s, &submitted);
    let verify = send(&mut s, cancel(&submitted, v, true), Role::Human, 602).unwrap_err();
    assert_eq!(verify.code, "invalid");
    assert!(verify.message.contains("review it or reopen it"));
    let ctx = send(
        &mut s,
        json!({"op":"context","key":submitted}),
        Role::Agent,
        603,
    )
    .unwrap();
    assert_eq!(ctx["issue"]["status"], "verify");
    assert_eq!(ctx["verifications"][0]["outcome"], "pending");

    // Verification children are managed through their parent.
    let child = ctx["issue"]["verification_key"]
        .as_str()
        .unwrap()
        .to_string();
    let cv = version_raw(&mut s, &child);
    assert_eq!(
        send(&mut s, cancel(&child, cv, false), Role::Human, 604)
            .unwrap_err()
            .code,
        "invalid"
    );
}

fn version_raw(s: &mut Store, key: &str) -> u64 {
    let snap = send(s, json!({"op":"snapshot"}), Role::Agent, 1).unwrap();
    snap["issues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == key)
        .unwrap()["version"]
        .as_u64()
        .unwrap()
}

#[test]
fn canceled_work_leaves_progress_denominators_and_can_be_restored() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let project = send(
        &mut s,
        json!({"op":"create_project","product":"DIR","name":"P"}),
        Role::Human,
        1,
    )
    .unwrap();
    let mut keys = vec![];
    for t in ["a", "b"] {
        let i = send(&mut s, json!({"op":"create_issue","product":"DIR","title":t,"planning_scope":"project","project_id":project["id"]}), Role::Agent, 2).unwrap();
        keys.push(i["key"].as_str().unwrap().to_string());
    }
    let progress = |s: &mut Store| {
        send(s, json!({"op":"snapshot"}), Role::Agent, 3).unwrap()["project_progress"][0].clone()
    };
    let before = progress(&mut s);
    let v = version(&mut s, &keys[0]);
    send(&mut s, cancel(&keys[0], v, false), Role::Human, 4).unwrap();
    let after = progress(&mut s);
    assert_eq!(after["canceled"], 1);
    assert_eq!(after["completed"], 0);
    assert_eq!(after["total"], before["total"]);
    assert_eq!(after["completion_percent"], 0);

    // Restore returns it to Backlog; readiness must be granted again.
    let v = version(&mut s, &keys[0]);
    assert_eq!(
        send(
            &mut s,
            json!({"op":"reopen","key":keys[0],"expected_version":v,"reason":"Needed after all"}),
            Role::Agent,
            5
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    let restored = send(
        &mut s,
        json!({"op":"reopen","key":keys[0],"expected_version":v,"reason":"Needed after all"}),
        Role::Human,
        5,
    )
    .unwrap();
    assert_eq!(restored["status"], "backlog");
    assert_eq!(restored["needs_fix"], false);
    let ctx = send(
        &mut s,
        json!({"op":"context","key":keys[0]}),
        Role::Agent,
        6,
    )
    .unwrap();
    assert!(ctx["comments"]
        .to_string()
        .contains("Restored from Canceled: Needed after all"));
    assert!(
        send(&mut s, json!({"op":"changes","after":0}), Role::Agent, 6).unwrap()["events"]
            .to_string()
            .contains("issue_restored")
    );

    // Canceled state and its audit comment survive archive restore.
    let v = version(&mut s, &keys[1]);
    send(&mut s, cancel(&keys[1], v, false), Role::Human, 7).unwrap();
    let archive = s.export().unwrap();
    validate_archive(&archive).unwrap();
    let other = TempDir::new().unwrap();
    let mut restored_store = Store::open(&other.path().join("db")).unwrap();
    restored_store.restore(archive).unwrap();
    let ctx = send(
        &mut restored_store,
        json!({"op":"context","key":keys[1]}),
        Role::Agent,
        8,
    )
    .unwrap();
    assert_eq!(ctx["issue"]["status"], "canceled");
    assert!(ctx["comments"].to_string().contains("Canceled: Test issue"));
}
