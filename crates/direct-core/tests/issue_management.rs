use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(store: &mut Store, value: Value, role: Role, at: i64) -> Result<Value> {
    let mut value = value;
    value["actor"] = json!(if role == Role::Human {
        "owner"
    } else {
        "agent"
    });
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    store.execute_at(serde_json::from_value(value).unwrap(), role, at)
}

#[test]
fn comprehensive_issue_capture_is_backward_compatible() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let issue = send(
        &mut store,
        json!({
            "op": "create_issue",
            "product": "DIR",
            "title": "Find work by issue key",
            "body": "Issue-key search is hidden by the current view.",
            "acceptance": "An exact key opens the matching issue.",
            "owner": "Nadeem",
            "priority": "high",
            "planning_scope": "inbox"
        }),
        Role::Human,
        100,
    )
    .unwrap();
    assert_eq!(
        issue["acceptance"],
        "An exact key opens the matching issue."
    );
    assert_eq!(issue["owner"], "Nadeem");
    assert_eq!(issue["priority"], "high");

    let legacy = send(
        &mut store,
        json!({"op":"create_issue","product":"DIR","title":"A minimal draft"}),
        Role::Agent,
        101,
    )
    .unwrap();
    assert_eq!(legacy["acceptance"], "");
    assert_eq!(legacy["priority"], "medium");
}

#[test]
fn owner_can_delete_only_unstarted_unreferenced_work_without_reusing_keys() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let issue = send(
        &mut store,
        json!({"op":"create_issue","product":"DIR","title":"Disposable","planning_scope":"inbox"}),
        Role::Human,
        100,
    )
    .unwrap();
    let key = issue["key"].as_str().unwrap();

    assert_eq!(
        send(
            &mut store,
            json!({"op":"delete_issue","key":key,"expected_version":1}),
            Role::Agent,
            101,
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    let delete_request = json!({
        "actor":"owner",
        "request_id":"delete-disposable",
        "op":"delete_issue",
        "key":key,
        "expected_version":1
    });
    let deleted = store
        .execute_at(
            serde_json::from_value(delete_request.clone()).unwrap(),
            Role::Human,
            102,
        )
        .unwrap();
    let replay = store
        .execute_at(
            serde_json::from_value(delete_request).unwrap(),
            Role::Human,
            103,
        )
        .unwrap();
    assert_eq!(replay, deleted);
    assert_eq!(deleted["deleted_key"], key);
    assert_eq!(
        send(
            &mut store,
            json!({"op":"context","key":key}),
            Role::Agent,
            104,
        )
        .unwrap_err()
        .code,
        "not_found"
    );

    let next = send(
        &mut store,
        json!({"op":"create_issue","product":"DIR","title":"Next","planning_scope":"inbox"}),
        Role::Human,
        105,
    )
    .unwrap();
    assert_eq!(next["key"], "DIR-2");
    let archive = store.export().unwrap();
    validate_archive(&archive).unwrap();
    assert!(archive
        .events
        .iter()
        .any(|event| event.kind == "issue_deleted" && event.entity == key));
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(archive).unwrap();
    let after_restore = send(
        &mut restored,
        json!({"op":"create_issue","product":"DIR","title":"After restore","planning_scope":"inbox"}),
        Role::Human,
        106,
    )
    .unwrap();
    assert_eq!(after_restore["key"], "DIR-3");
}

#[test]
fn deletion_refuses_claimed_linked_or_submitted_work() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let first = send(
        &mut store,
        json!({"op":"create_issue","product":"DIR","title":"First","planning_scope":"inbox"}),
        Role::Human,
        100,
    )
    .unwrap();
    let second = send(
        &mut store,
        json!({"op":"create_issue","product":"DIR","title":"Second","planning_scope":"inbox"}),
        Role::Human,
        101,
    )
    .unwrap();
    let first_key = first["key"].as_str().unwrap();
    let second_key = second["key"].as_str().unwrap();
    send(
        &mut store,
        json!({"op":"create_issue_link","key":first_key,"expected_version":1,"target_key":second_key,"kind":"related"}),
        Role::Human,
        102,
    )
    .unwrap();
    assert_eq!(
        send(
            &mut store,
            json!({"op":"delete_issue","key":first_key,"expected_version":2}),
            Role::Human,
            103,
        )
        .unwrap_err()
        .code,
        "conflict"
    );

    let claimable = send(
        &mut store,
        json!({
            "op":"create_issue","product":"DIR","title":"Claimed","body":"Body",
            "acceptance":"Done","owner":"Nadeem","planning_scope":"inbox"
        }),
        Role::Human,
        104,
    )
    .unwrap();
    let claimed_key = claimable["key"].as_str().unwrap();
    send(
        &mut store,
        json!({"op":"ready","key":claimed_key,"expected_version":1}),
        Role::Human,
        105,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"claim","key":claimed_key,"expected_version":2}),
        Role::Agent,
        106,
    )
    .unwrap();
    assert_eq!(
        send(
            &mut store,
            json!({"op":"delete_issue","key":claimed_key,"expected_version":3}),
            Role::Human,
            107,
        )
        .unwrap_err()
        .code,
        "invalid"
    );
}
