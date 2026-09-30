use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(store: &mut Store, mut value: Value, role: Role) -> Result<Value> {
    value["actor"] = json!("relation-builder");
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    store.execute_at(serde_json::from_value(value).unwrap(), role, 100)
}

fn issue(store: &mut Store, product: &str, title: &str) -> Value {
    send(
        store,
        json!({"op":"create_issue","product":product,"title":title}),
        Role::Agent,
    )
    .unwrap()
}

#[test]
fn issue_links_are_inspectable_cycle_safe_and_round_trip() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let a = issue(&mut store, "DIR", "A");
    let b = issue(&mut store, "DIR", "B");
    let c = issue(&mut store, "DIR", "C");
    send(
        &mut store,
        json!({"op":"create_product","key":"ALT","name":"Other"}),
        Role::Human,
    )
    .unwrap();
    let other = issue(&mut store, "ALT", "Other");

    let parent = send(
        &mut store,
        json!({"op":"create_issue_link","key":a["key"],"expected_version":1,"target_key":b["key"],"kind":"parent"}),
        Role::Agent,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"create_issue_link","key":b["key"],"expected_version":1,"target_key":c["key"],"kind":"parent"}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(
        send(
            &mut store,
            json!({"op":"create_issue_link","key":c["key"],"expected_version":1,"target_key":a["key"],"kind":"parent"}),
            Role::Agent,
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    assert_eq!(
        send(
            &mut store,
            json!({"op":"create_issue_link","key":a["key"],"expected_version":2,"target_key":c["key"],"kind":"parent"}),
            Role::Agent,
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    for target in [&a["key"], &other["key"]] {
        assert_eq!(
            send(
                &mut store,
                json!({"op":"create_issue_link","key":a["key"],"expected_version":2,"target_key":target,"kind":"related"}),
                Role::Agent,
            )
            .unwrap_err()
            .code,
            "invalid"
        );
    }

    send(
        &mut store,
        json!({"op":"create_issue_link","key":a["key"],"expected_version":2,"target_key":c["key"],"kind":"related"}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(
        send(
            &mut store,
            json!({"op":"create_issue_link","key":c["key"],"expected_version":1,"target_key":a["key"],"kind":"related"}),
            Role::Agent,
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    assert_eq!(
        send(
            &mut store,
            json!({"op":"create_issue_link","key":a["key"],"expected_version":3,"target_key":c["key"],"kind":"legacy_verification"}),
            Role::Agent,
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    send(
        &mut store,
        json!({"op":"create_issue_link","key":a["key"],"expected_version":3,"target_key":c["key"],"kind":"legacy_verification","external_source":"linear","external_id":"legacy-relation-1"}),
        Role::Agent,
    )
    .unwrap();

    let context = send(
        &mut store,
        json!({"op":"context","key":a["key"]}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(context["issue_links"].as_array().unwrap().len(), 3);
    assert_eq!(context["issue_links"][0]["direction"], "outgoing");
    let incoming = send(
        &mut store,
        json!({"op":"context","key":b["key"]}),
        Role::Agent,
    )
    .unwrap();
    assert!(incoming["issue_links"]
        .as_array()
        .unwrap()
        .iter()
        .any(|link| {
            link["id"] == parent["id"]
                && link["direction"] == "incoming"
                && link["issue"]["key"] == a["key"]
        }));

    let snapshot = send(&mut store, json!({"op":"snapshot"}), Role::Agent).unwrap();
    assert_eq!(snapshot["issue_links"].as_array().unwrap().len(), 4);
    let before = store.export().unwrap();
    assert_eq!(before.format, 7);
    validate_archive(&before).unwrap();
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(before.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        serde_json::to_value(before).unwrap()
    );

    let delete_incoming = json!({"op":"delete_issue_link","key":b["key"],"expected_version":2,"link_id":parent["id"]});
    assert_eq!(
        send(&mut restored, delete_incoming.clone(), Role::Agent)
            .unwrap_err()
            .code,
        "forbidden"
    );
    send(&mut restored, delete_incoming, Role::Human).unwrap();
    assert_eq!(restored.export().unwrap().issue_links.len(), 3);
}

#[test]
fn blocker_cycles_and_malformed_archives_are_rejected() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let a = issue(&mut store, "DIR", "A");
    let b = issue(&mut store, "DIR", "B");
    send(
        &mut store,
        json!({"op":"create_issue_link","key":a["key"],"expected_version":1,"target_key":b["key"],"kind":"blocked_by"}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(
        send(
            &mut store,
            json!({"op":"create_issue_link","key":b["key"],"expected_version":1,"target_key":a["key"],"kind":"blocked_by"}),
            Role::Agent,
        )
        .unwrap_err()
        .code,
        "invalid"
    );

    let archive = store.export().unwrap();
    let mut old = archive.clone();
    old.format = 5;
    assert_eq!(validate_archive(&old).unwrap_err().code, "invalid");

    let mut cyclic = archive;
    cyclic.issue_links.push(IssueLink {
        id: uuid::Uuid::new_v4().to_string(),
        source_key: b["key"].as_str().unwrap().into(),
        target_key: a["key"].as_str().unwrap().into(),
        kind: IssueLinkKind::BlockedBy,
        external_source: None,
        external_id: None,
        created_by: "fixture".into(),
        created_at: 100,
    });
    assert_eq!(validate_archive(&cyclic).unwrap_err().code, "invalid");
}

#[test]
fn direct_verification_runs_remain_distinct_from_imported_legacy_links() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let delivery = issue(&mut store, "DIR", "Delivery");
    let legacy_test = issue(&mut store, "DIR", "Legacy test issue");
    let key = delivery["key"].as_str().unwrap();
    send(
        &mut store,
        json!({"op":"update_issue","key":key,"expected_version":1,"title":"Delivery","body":"Build it","acceptance":"It works","owner":"owner","priority":"high"}),
        Role::Agent,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"ready","key":key,"expected_version":2}),
        Role::Human,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"claim","key":key,"expected_version":3}),
        Role::Agent,
    )
    .unwrap();
    let submitted = send(
        &mut store,
        json!({"op":"submit","key":key,"expected_version":4,"build_ref":"build","delivery_ref":"branch","summary":"Done","checks":"Passed","steps":[{"instruction":"Try it","expected":"Works"}]}),
        Role::Agent,
    )
    .unwrap();
    let after_submit = store.export().unwrap();
    assert_eq!(after_submit.verifications.len(), 1);
    assert!(after_submit.issue_links.is_empty());
    assert!(after_submit
        .issues
        .iter()
        .any(|issue| issue.parent.as_deref() == Some(key)));

    send(
        &mut store,
        json!({"op":"create_issue_link","key":key,"expected_version":submitted["version"],"target_key":legacy_test["key"],"kind":"legacy_verification","external_source":"linear","external_id":"legacy-test-link"}),
        Role::Human,
    )
    .unwrap();
    let archive = store.export().unwrap();
    assert_eq!(archive.verifications.len(), 1);
    assert_eq!(archive.issue_links.len(), 1);
    assert_eq!(
        archive.issue_links[0].kind,
        IssueLinkKind::LegacyVerification
    );
    assert_eq!(
        archive
            .issues
            .iter()
            .filter(|issue| issue.parent.as_deref() == Some(key))
            .count(),
        1
    );
    validate_archive(&archive).unwrap();
}
