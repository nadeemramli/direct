use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(store: &mut Store, value: Value, role: Role, at: i64) -> Result<Value> {
    let mut value = value;
    value["actor"] = json!("builder");
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    store.execute_at(serde_json::from_value(value).unwrap(), role, at)
}
fn prepared(store: &mut Store) -> Value {
    let i=send(store,json!({"op":"create_issue","product":"DIR","title":"Verify a native update","body":"Keep the UI synchronized"}),Role::Agent,100).unwrap();
    let key = i["key"].as_str().unwrap();
    send(store,json!({"op":"update_issue","key":key,"expected_version":1,"title":"Verify a native update","body":"Keep the UI synchronized","acceptance":"A change from WSL appears","owner":"reviewer","priority":"high"}),Role::Agent,101).unwrap();
    send(
        store,
        json!({"op":"ready","key":key,"expected_version":2}),
        Role::Human,
        102,
    )
    .unwrap()
}
fn submission(key: &str, version: u64) -> Value {
    json!({"op":"submit","request_id":"handoff-1","key":key,"expected_version":version,"build_ref":"commit:abc123","delivery_ref":"main:abc123","summary":"Live updates implemented","checks":"4 checks passed","steps":[{"instruction":"Create an issue from WSL","expected":"It appears without reload"}]})
}

#[test]
fn human_verification_and_cancellation_cannot_be_bypassed() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let i = prepared(&mut s);
    let key = i["key"].as_str().unwrap();
    send(
        &mut s,
        json!({"op":"claim","key":key,"expected_version":3}),
        Role::Agent,
        103,
    )
    .unwrap();
    let submitted = send(&mut s, submission(key, 4), Role::Agent, 104).unwrap();
    assert_eq!(submitted["status"], "verify");
    let pass = json!({"op":"review","key":key,"expected_version":5,"run_id":submitted["current_run"],"outcome":"passed","results":[{"outcome":"passed"}]});
    assert_eq!(
        send(&mut s, pass.clone(), Role::Agent, 105)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let mut missing = pass.clone();
    missing["results"] = json!([]);
    assert_eq!(
        send(&mut s, missing, Role::Human, 105).unwrap_err().code,
        "invalid"
    );
    let canceled=send(&mut s,json!({"op":"review","key":key,"expected_version":5,"run_id":submitted["current_run"],"outcome":"canceled","results":[],"note":"Need a fresh installer"}),Role::Human,106).unwrap();
    assert_eq!(canceled["status"], "doing");
    send(
        &mut s,
        json!({"op":"claim","key":key,"expected_version":6}),
        Role::Agent,
        107,
    )
    .unwrap();
    let mut new = submission(key, 7);
    new["request_id"] = json!("handoff-2");
    new["build_ref"] = json!("commit:def456");
    let new_run = send(&mut s, new, Role::Agent, 108).unwrap();
    let mut pass2 = pass;
    pass2["expected_version"] = json!(8);
    assert_eq!(
        send(&mut s, pass2.clone(), Role::Human, 109)
            .unwrap_err()
            .code,
        "conflict"
    );
    pass2["run_id"] = new_run["current_run"].clone();
    let done = send(&mut s, pass2, Role::Human, 109).unwrap();
    assert_eq!(done["status"], "done");
    let a = s.export().unwrap();
    assert_eq!(a.issues.len(), 2);
    assert_eq!(a.verifications.len(), 2);
    validate_archive(&a).unwrap();
    send(
        &mut s,
        json!({"op":"reopen","key":key,"expected_version":9,"reason":"Implementation changed"}),
        Role::Human,
        110,
    )
    .unwrap();
    let context = send(&mut s, json!({"op":"context","key":key}), Role::Agent, 111).unwrap();
    assert_eq!(context["issue"]["status"], "doing");
    assert!(context["issue"]["current_run"].is_null());
    assert_eq!(context["verifications"].as_array().unwrap().len(), 2);
}

#[test]
fn retry_is_idempotent_and_stale_writers_do_not_win() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let i = prepared(&mut s);
    let key = i["key"].as_str().unwrap();
    let claim = json!({"op":"claim","request_id":"claim-1","key":key,"expected_version":3,"lease_seconds":30});
    let first = send(&mut s, claim.clone(), Role::Agent, 103).unwrap();
    assert_eq!(
        send(&mut s, claim.clone(), Role::Agent, 104).unwrap(),
        first
    );
    let mut conflicting = claim.clone();
    conflicting["lease_seconds"] = json!(60);
    assert_eq!(
        send(&mut s, conflicting, Role::Agent, 104)
            .unwrap_err()
            .code,
        "conflict"
    );
    let stale = json!({"op":"claim","key":key,"expected_version":3});
    assert_eq!(
        send(&mut s, stale, Role::Agent, 104).unwrap_err().code,
        "conflict"
    );
    assert_eq!(
        send(&mut s, submission(key, 4), Role::Agent, 134)
            .unwrap_err()
            .code,
        "claim_required"
    );
    send(
        &mut s,
        json!({"op":"claim","key":key,"expected_version":4}),
        Role::Agent,
        135,
    )
    .unwrap();
    let req = submission(key, 5);
    let first = send(&mut s, req.clone(), Role::Agent, 136).unwrap();
    let replay = send(&mut s, req, Role::Agent, 137).unwrap();
    assert_eq!(first, replay);
    let a = s.export().unwrap();
    assert_eq!(a.verifications.len(), 1);
    assert_eq!(a.issues.len(), 2);
}

#[test]
fn failure_is_actionable_and_retest_preserves_history() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let i = prepared(&mut s);
    let key = i["key"].as_str().unwrap();
    send(
        &mut s,
        json!({"op":"claim","key":key,"expected_version":3}),
        Role::Agent,
        103,
    )
    .unwrap();
    let submitted = send(&mut s, submission(key, 4), Role::Agent, 104).unwrap();
    send(&mut s,json!({"op":"review","key":key,"expected_version":5,"run_id":submitted["current_run"],"outcome":"failed","results":[{"outcome":"failed","note":"The view did not update"}],"note":"Stale content after reconnect"}),Role::Human,105).unwrap();
    let ctx = send(&mut s, json!({"op":"context","key":key}), Role::Agent, 106).unwrap();
    assert_eq!(ctx["issue"]["needs_fix"], true);
    assert_eq!(
        ctx["verifications"][0]["review_note"],
        "Stale content after reconnect"
    );
}

#[test]
fn restart_and_restore_preserve_identity_evidence_cursors_and_replays() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let i = prepared(&mut s);
    let key = i["key"].as_str().unwrap();
    let comment = json!({"op":"comment","request_id":"comment-retry","key":key,"expected_version":3,"body":"Decision: test WSL first"});
    let response = send(&mut s, comment.clone(), Role::Agent, 103).unwrap();
    let before = serde_json::to_value(s.export().unwrap()).unwrap();
    drop(s);
    let s = Store::open(&path).unwrap();
    assert_eq!(serde_json::to_value(s.export().unwrap()).unwrap(), before);
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(s.export().unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        before
    );
    assert_eq!(
        send(&mut restored, comment, Role::Agent, 120).unwrap(),
        response
    );
    let mut broken = s.export().unwrap();
    broken.issues[0].product_id = "missing".into();
    assert!(restored.restore(broken).is_err());
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        before
    );
}
