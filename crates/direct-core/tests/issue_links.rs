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
    assert_eq!(before.format, 21);
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
        json!({"op":"submit","key":key,"expected_version":4,"build_ref":"build","e2e":{"build_ref":"build","environment":"isolated Windows fixture","entrypoint":"fixture client","scenarios":"Exercise the full fixture workflow; expected and observed state transitions match","outcome":"passed","delivered_build_ref":"build","delivery_check":"Fixture service and client use the tested build"},"delivery_ref":"branch","summary":"Done","checks":"Passed","steps":[{"instruction":"Try it","expected":"Works"}]}),
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

#[test]
fn create_issue_writes_requested_relations_atomically_and_replays_exactly() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let parent = issue(&mut store, "DIR", "Parent");
    let blocker = issue(&mut store, "DIR", "Blocker");
    let related = issue(&mut store, "DIR", "Related");
    let create = json!({
        "op":"create_issue","product":"DIR","title":"Child","request_id":"create-with-links",
        "links":[
            {"target_key":parent["key"],"kind":"parent"},
            {"target_key":blocker["key"],"kind":"blocked_by"},
            {"target_key":related["key"],"kind":"related"}
        ]
    });
    let created = send(&mut store, create.clone(), Role::Human).unwrap();
    assert_eq!(created["version"], 1);
    assert_eq!(created["status"], "backlog");

    let context = send(
        &mut store,
        json!({"op":"context","key":created["key"]}),
        Role::Agent,
    )
    .unwrap();
    let links = context["issue_links"].as_array().unwrap();
    assert_eq!(links.len(), 3);
    for (target, kind) in [
        (&parent, "parent"),
        (&blocker, "blocked_by"),
        (&related, "related"),
    ] {
        assert!(links.iter().any(|link| link["direction"] == "outgoing"
            && link["kind"] == kind
            && link["issue"]["key"] == target["key"]
            && link["created_by"] == "relation-builder"));
        let incoming = send(
            &mut store,
            json!({"op":"context","key":target["key"]}),
            Role::Agent,
        )
        .unwrap();
        assert!(incoming["issue_links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|link| link["direction"] == "incoming"
                && link["kind"] == kind
                && link["issue"]["key"] == created["key"]));
        // Linking from a new issue never edits the target.
        assert_eq!(incoming["issue"]["version"], 1);
    }

    // An exact retry replays the stored response and adds nothing.
    let replay = send(&mut store, create.clone(), Role::Human).unwrap();
    assert_eq!(replay, created);
    let archive = store.export().unwrap();
    assert_eq!(archive.issue_links.len(), 3);
    assert_eq!(archive.issues.len(), 4);
    validate_archive(&archive).unwrap();
    let mut changed = create;
    changed["links"].as_array_mut().unwrap().pop();
    assert_eq!(
        send(&mut store, changed, Role::Human).unwrap_err().code,
        "conflict"
    );

    // The created relations obey the same rules afterwards: a second parent is refused.
    assert_eq!(
        send(
            &mut store,
            json!({"op":"create_issue_link","key":created["key"],"expected_version":1,"target_key":related["key"],"kind":"parent"}),
            Role::Agent,
        )
        .unwrap_err()
        .code,
        "conflict"
    );
}

#[test]
fn a_rejected_relation_rejects_the_whole_create() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let a = issue(&mut store, "DIR", "A");
    let b = issue(&mut store, "DIR", "B");
    send(
        &mut store,
        json!({"op":"create_product","key":"ALT","name":"Other"}),
        Role::Human,
    )
    .unwrap();
    let other = issue(&mut store, "ALT", "Other");
    // A new issue has no incoming links, so it cannot close a cycle; every other rule applies.
    let cases = [
        (
            json!([{"target_key":other["key"],"kind":"related"}]),
            "invalid",
            "same product",
        ),
        (
            json!([{"target_key":"DIR-99","kind":"related"}]),
            "not_found",
            "",
        ),
        (
            json!([{"target_key":a["key"],"kind":"related"},{"target_key":a["key"],"kind":"related"}]),
            "conflict",
            "already exists",
        ),
        (
            json!([{"target_key":a["key"],"kind":"parent"},{"target_key":b["key"],"kind":"parent"}]),
            "conflict",
            "only one parent",
        ),
        (
            json!([{"target_key":a["key"],"kind":"legacy_verification"}]),
            "invalid",
            "external provenance",
        ),
        (json!([{"target_key":a["key"],"kind":"sibling"}]), "", ""),
    ];
    for (links, code, message) in cases {
        let request = json!({"op":"create_issue","product":"DIR","title":"Rejected","links":links});
        if code.is_empty() {
            // Unknown kinds fail to parse before reaching the store.
            assert!(serde_json::from_value::<Request>(json!({
                "actor":"a","request_id":"r","op":"create_issue","product":"DIR","title":"x","links":links
            }))
            .is_err());
            continue;
        }
        let error = send(&mut store, request, Role::Human).unwrap_err();
        assert_eq!(error.code, code, "{links}");
        assert!(
            error.message.starts_with("Relation to "),
            "{}",
            error.message
        );
        assert!(error.message.contains(message), "{}", error.message);
    }
    let archive = store.export().unwrap();
    assert_eq!(archive.issues.len(), 3);
    assert!(archive.issue_links.is_empty());
    // No key was consumed by the rejected attempts.
    let next = issue(&mut store, "DIR", "Next");
    assert_eq!(next["key"], "DIR-3");
}

#[test]
fn verification_children_cannot_be_related_at_creation() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let delivery = issue(&mut store, "DIR", "Delivery");
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
        json!({"op":"submit","key":key,"expected_version":4,"build_ref":"build","e2e":{"build_ref":"build","environment":"isolated fixture","entrypoint":"fixture client","scenarios":"Fixture workflow observed","outcome":"passed","delivered_build_ref":"build","delivery_check":"Fixture build"},"delivery_ref":"branch","summary":"Done","checks":"Passed","steps":[{"instruction":"Try it","expected":"Works"}]}),
        Role::Agent,
    )
    .unwrap();
    let child = submitted["verification_key"].as_str().unwrap();
    let error = send(
        &mut store,
        json!({"op":"create_issue","product":"DIR","title":"Rejected","links":[{"target_key":child,"kind":"related"}]}),
        Role::Human,
    )
    .unwrap_err();
    assert_eq!(error.code, "invalid");
    assert!(error.message.contains("verification children"));
    assert!(store.export().unwrap().issue_links.is_empty());
}

#[test]
fn creates_without_relations_keep_their_request_hash() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let legacy = json!({"op":"create_issue","product":"DIR","title":"Plain","request_id":"plain"});
    let created = send(&mut store, legacy.clone(), Role::Human).unwrap();
    // An explicit empty list is the same logical command as an omitted one.
    let mut explicit = legacy;
    explicit["links"] = json!([]);
    assert_eq!(send(&mut store, explicit, Role::Human).unwrap(), created);
    let request: Request = serde_json::from_value(json!({
        "actor":"a","request_id":"r","op":"create_issue","product":"DIR","title":"Plain"
    }))
    .unwrap();
    assert!(serde_json::to_value(&request.command)
        .unwrap()
        .get("links")
        .is_none());
}
