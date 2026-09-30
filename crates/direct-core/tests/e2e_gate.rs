use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(store: &mut Store, mut value: Value, role: Role) -> Result<Value> {
    value["actor"] = json!("e2e-test-agent");
    value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    store.execute_at(serde_json::from_value(value).unwrap(), role, 100)
}

#[test]
fn submission_requires_passed_e2e_on_the_exact_delivered_build() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("fixture.db");
    let mut store = Store::open(&path).unwrap();
    send(&mut store, json!({"op":"create_issue","product":"DIR","title":"Full flow","body":"Persist the user change","acceptance":"User sees the persisted change","owner":"tester"}), Role::Agent).unwrap();
    send(
        &mut store,
        json!({"op":"ready","key":"DIR-1","expected_version":1}),
        Role::Human,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"claim","key":"DIR-1","expected_version":2}),
        Role::Agent,
    )
    .unwrap();
    let evidence = json!({"build_ref":"build-1","delivered_build_ref":"build-1","environment":"Windows isolated fixture","entrypoint":"fixture UI","scenarios":"Create, reload and inspect persisted issue; actual title matches expected title. Evidence: fixture assertions.","outcome":"passed","delivery_check":"Launched installed build-1 and observed the persisted issue"});
    let submission = json!({"op":"submit","key":"DIR-1","expected_version":3,"build_ref":"build-1","delivery_ref":"installed fixture","summary":"Flow works","checks":"Regression suite passed","e2e":evidence,"steps":[{"instruction":"Open the issue","expected":"Saved title appears"}]});
    let before = serde_json::to_value(store.export().unwrap()).unwrap();
    for field in [
        "missing",
        "outcome",
        "build_ref",
        "delivered_build_ref",
        "environment",
        "entrypoint",
        "scenarios",
        "delivery_check",
    ] {
        let mut invalid = submission.clone();
        match field {
            "missing" => {
                invalid.as_object_mut().unwrap().remove("e2e");
            }
            "outcome" => invalid["e2e"][field] = json!("failed"),
            "build_ref" | "delivered_build_ref" => invalid["e2e"][field] = json!("old-build"),
            _ => invalid["e2e"][field] = json!(" "),
        }
        assert_eq!(
            send(&mut store, invalid, Role::Agent).unwrap_err().code,
            "invalid",
            "{field}"
        );
        assert_eq!(
            serde_json::to_value(store.export().unwrap()).unwrap(),
            before
        );
    }
    for outcome in ["pending", "canceled"] {
        let mut invalid = submission.clone();
        invalid["e2e"]["outcome"] = json!(outcome);
        assert!(send(&mut store, invalid, Role::Agent).is_err());
    }
    let submitted = send(&mut store, submission, Role::Agent).unwrap();
    assert_eq!(submitted["status"], "verify");
    drop(store);
    let store = Store::open(&path).unwrap();
    let archive = store.export().unwrap();
    assert_eq!(
        serde_json::to_value(&archive.verifications[0].e2e).unwrap(),
        evidence
    );
    let mut restored = Store::open(&temp.path().join("restore.db")).unwrap();
    restored.restore(archive.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        serde_json::to_value(&archive).unwrap()
    );
    let mut invalid_archive = archive.clone();
    invalid_archive.verifications[0]
        .e2e
        .as_mut()
        .unwrap()
        .delivered_build_ref = "other".into();
    assert!(validate_archive(&invalid_archive).is_err());
    // Historical evidence remains readable; no E2E result is invented during upgrade.
    let mut historical = archive;
    historical.format = 10;
    historical.verifications[0].e2e = None;
    validate_archive(&historical).unwrap();
    let mut old_store = Store::open(&temp.path().join("historical.db")).unwrap();
    old_store.restore(historical).unwrap();
    let snapshot = send(&mut old_store, json!({"op":"snapshot"}), Role::Agent).unwrap();
    assert_eq!(snapshot["review_ready_runs"], json!([]));
    assert_eq!(send(&mut old_store, json!({"op":"review","key":"DIR-1","expected_version":4,"run_id":submitted["current_run"],"outcome":"passed","results":[{"outcome":"passed"}]}), Role::Human).unwrap_err().code, "invalid");
}
