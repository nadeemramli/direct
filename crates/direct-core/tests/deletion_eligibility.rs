//! DIR-51: deletion eligibility is one server-side rule that `context` reports
//! and `delete_issue` re-enforces atomically. Every fixture is an isolated,
//! synthetic workspace.
use direct_core::*;
use serde_json::{json, Value};
use std::path::Path;
use tempfile::TempDir;

fn send(store: &mut Store, mut value: Value, role: Role, at: i64) -> Result<Value> {
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

fn open(path: &Path) -> Store {
    Store::open(path).unwrap()
}

fn inbox(store: &mut Store, title: &str, at: i64) -> String {
    send(
        store,
        json!({"op":"create_issue","product":"DIR","title":title,"planning_scope":"inbox"}),
        Role::Human,
        at,
    )
    .unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string()
}

fn ready(store: &mut Store, title: &str, at: i64) -> String {
    let key = send(
        store,
        json!({
            "op":"create_issue","product":"DIR","title":title,"body":"Problem",
            "acceptance":"Outcome","owner":"owner","planning_scope":"inbox"
        }),
        Role::Human,
        at,
    )
    .unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string();
    send(
        store,
        json!({"op":"ready","key":key,"expected_version":1}),
        Role::Human,
        at + 1,
    )
    .unwrap();
    key
}

fn deletion(store: &mut Store, key: &str, role: Role, at: i64) -> Value {
    send(store, json!({"op":"context","key":key}), role, at).unwrap()["deletion"].clone()
}

fn kinds(eligibility: &Value) -> Vec<String> {
    eligibility["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|blocker| blocker["kind"].as_str().unwrap().to_string())
        .collect()
}

fn blocker<'a>(eligibility: &'a Value, kind: &str) -> &'a Value {
    eligibility["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|blocker| blocker["kind"] == kind)
        .unwrap_or_else(|| panic!("missing {kind} blocker in {eligibility}"))
}

fn delete(store: &mut Store, key: &str, version: u64, at: i64) -> Result<Value> {
    send(
        store,
        json!({"op":"delete_issue","key":key,"expected_version":version}),
        Role::Human,
        at,
    )
}

fn version_of(store: &mut Store, key: &str, at: i64) -> u64 {
    send(store, json!({"op":"context","key":key}), Role::Human, at).unwrap()["issue"]["version"]
        .as_u64()
        .unwrap()
}

const SUBMISSION: &str = r#"{"build_ref":"build","e2e":{"build_ref":"build","environment":"isolated fixture","entrypoint":"fixture client","scenarios":"Fixture scenario observed","outcome":"passed","delivered_build_ref":"build","delivery_check":"Fixture build"},"delivery_ref":"branch","summary":"Done","checks":"Passed","steps":[{"instruction":"Try it","expected":"Works"}]}"#;

#[test]
fn comment_only_issue_reports_the_comment_not_links_or_releases_and_survives_restart() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("db");
    let mut store = open(&db);
    let key = inbox(&mut store, "Commented draft", 100);
    send(
        &mut store,
        json!({"op":"comment","key":key,"expected_version":1,"body":"Audit note"}),
        Role::Human,
        101,
    )
    .unwrap();

    let eligibility = deletion(&mut store, &key, Role::Human, 102);
    assert_eq!(eligibility["eligible"], false);
    assert_eq!(eligibility["version"], 2);
    assert_eq!(kinds(&eligibility), vec!["comments"]);
    let comments = blocker(&eligibility, "comments");
    assert_eq!(comments["count"], 1);
    assert_eq!(comments["removable"], false);

    let refused = delete(&mut store, &key, 2, 103).unwrap_err();
    assert_eq!(refused.code, "conflict");
    assert!(refused.message.contains("1 comment"), "{}", refused.message);
    assert!(refused.message.contains(&key), "{}", refused.message);
    assert!(!refused.message.contains("link"), "{}", refused.message);
    assert!(!refused.message.contains("release"), "{}", refused.message);

    // The refused deletion wrote nothing: same version, comment retained, across a restart.
    drop(store);
    let mut store = open(&db);
    let context = send(
        &mut store,
        json!({"op":"context","key":key}),
        Role::Human,
        104,
    )
    .unwrap();
    assert_eq!(context["issue"]["version"], 2);
    assert_eq!(context["comments"][0]["body"], "Audit note");
    assert_eq!(kinds(&context["deletion"]), vec!["comments"]);
    assert!(!store
        .export()
        .unwrap()
        .events
        .iter()
        .any(|event| event.kind == "issue_deleted"));
}

#[test]
fn links_and_explicit_release_references_are_named_and_removable_but_project_releases_are_not_references(
) {
    let dir = TempDir::new().unwrap();
    let mut store = open(&dir.path().join("db"));
    let linked = inbox(&mut store, "Linked", 100);
    let other = inbox(&mut store, "Other end", 101);
    let released = inbox(&mut store, "Released", 102);
    let in_project = inbox(&mut store, "Only in a released project", 103);

    send(
        &mut store,
        json!({"op":"create_issue_link","key":linked,"expected_version":1,"target_key":other,"kind":"blocked_by"}),
        Role::Human,
        104,
    )
    .unwrap();
    for key in [&linked, &other] {
        let eligibility = deletion(&mut store, key, Role::Human, 105);
        assert_eq!(kinds(&eligibility), vec!["issue_links"]);
        let links = blocker(&eligibility, "issue_links");
        assert_eq!(links["removable"], true);
        assert_eq!(links["count"], 1);
    }
    assert_eq!(
        blocker(
            &deletion(&mut store, &linked, Role::Human, 105),
            "issue_links"
        )["references"],
        json!([other])
    );
    let refused = delete(&mut store, &other, 1, 106).unwrap_err();
    assert_eq!(refused.code, "conflict");
    assert!(
        refused.message.contains("1 issue link"),
        "{}",
        refused.message
    );
    assert!(refused.message.contains(&linked), "{}", refused.message);
    assert!(!refused.message.contains("release"), "{}", refused.message);

    // Removing the only blocker makes the same issue deletable.
    let link_id = store.export().unwrap().issue_links[0].id.clone();
    send(
        &mut store,
        json!({"op":"delete_issue_link","key":linked,"expected_version":2,"link_id":link_id}),
        Role::Human,
        107,
    )
    .unwrap();
    assert_eq!(
        deletion(&mut store, &linked, Role::Human, 108)["eligible"],
        true
    );

    let project = send(
        &mut store,
        json!({"op":"create_project","product":"DIR","name":"Released project"}),
        Role::Human,
        109,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"set_issue_project","key":in_project,"expected_version":1,"project_id":project["id"]}),
        Role::Human,
        110,
    )
    .unwrap();
    let release = send(
        &mut store,
        json!({
            "op":"create_release","product":"DIR","name":"Pilot","version_label":"v9.9.9",
            "target_ref":"refs/heads/main","project_ids":[project["id"]],"issue_keys":[released]
        }),
        Role::Human,
        111,
    )
    .unwrap();

    let explicit = deletion(&mut store, &released, Role::Human, 112);
    assert_eq!(kinds(&explicit), vec!["release_references"]);
    let references = blocker(&explicit, "release_references");
    assert_eq!(references["references"], json!([release["id"]]));
    assert_eq!(references["removable"], true);
    let refused = delete(&mut store, &released, 1, 113).unwrap_err();
    assert_eq!(refused.code, "conflict");
    assert!(refused.message.contains("v9.9.9"), "{}", refused.message);
    assert!(!refused.message.contains("link"), "{}", refused.message);

    // Context shows the project release, but it is not an explicit issue reference.
    let context = send(
        &mut store,
        json!({"op":"context","key":in_project}),
        Role::Human,
        114,
    )
    .unwrap();
    assert_eq!(context["releases"][0]["id"], release["id"]);
    assert_eq!(context["deletion"]["eligible"], true);
    assert_eq!(context["deletion"]["blockers"], json!([]));
    delete(&mut store, &in_project, 2, 115).unwrap();

    // Removing the issue from the release clears the explicit reference.
    send(
        &mut store,
        json!({
            "op":"update_release","id":release["id"],"expected_version":1,"name":"Pilot",
            "version_label":"v9.9.9","status":"planned","target_ref":"refs/heads/main",
            "project_ids":[project["id"]],"issue_keys":[]
        }),
        Role::Human,
        116,
    )
    .unwrap();
    assert_eq!(
        deletion(&mut store, &released, Role::Human, 117)["eligible"],
        true
    );
    delete(&mut store, &released, 1, 118).unwrap();
}

#[test]
fn retained_submission_blocks_even_after_the_issue_returns_to_ready() {
    let dir = TempDir::new().unwrap();
    let mut store = open(&dir.path().join("db"));
    let key = ready(&mut store, "Submitted once", 100);
    send(
        &mut store,
        json!({"op":"claim","key":key,"expected_version":2,"lease_seconds":3600}),
        Role::Agent,
        110,
    )
    .unwrap();
    let mut submit: Value = serde_json::from_str(SUBMISSION).unwrap();
    submit["op"] = json!("submit");
    submit["key"] = json!(key);
    submit["expected_version"] = json!(3);
    let submitted = send(&mut store, submit, Role::Agent, 111).unwrap();
    let child = submitted["verification_key"].as_str().unwrap().to_string();

    let verifying = deletion(&mut store, &key, Role::Human, 112);
    assert_eq!(
        kinds(&verifying),
        vec!["status", "verification_history", "verification_children"]
    );
    assert!(blocker(&verifying, "status")["message"]
        .as_str()
        .unwrap()
        .contains("It is verify"));
    assert_eq!(
        blocker(&verifying, "verification_children")["references"],
        json!([child])
    );
    // The generated child carries the run too, so it can never be deleted on its own.
    assert!(kinds(&deletion(&mut store, &child, Role::Human, 112))
        .contains(&"verification_history".to_string()));

    send(
        &mut store,
        json!({"op":"reopen","key":key,"expected_version":4,"reason":"Rework"}),
        Role::Human,
        113,
    )
    .unwrap();
    let version = version_of(&mut store, &key, 114);
    send(
        &mut store,
        json!({"op":"claim","key":key,"expected_version":version,"lease_seconds":3600}),
        Role::Agent,
        115,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"release","key":key,"expected_version":version + 1}),
        Role::Agent,
        116,
    )
    .unwrap();
    let context = send(
        &mut store,
        json!({"op":"context","key":key}),
        Role::Human,
        117,
    )
    .unwrap();
    assert_eq!(context["issue"]["status"], "ready");
    let eligibility = &context["deletion"];
    assert_eq!(
        kinds(eligibility),
        vec!["comments", "verification_history", "verification_children"]
    );
    let refused = delete(&mut store, &key, version + 2, 118).unwrap_err();
    assert_eq!(refused.code, "conflict");
    assert!(
        refused.message.contains("1 verification submission"),
        "{}",
        refused.message
    );
    assert!(refused.message.contains("comment"), "{}", refused.message);
    assert_eq!(store.export().unwrap().verifications.len(), 1);
}

#[test]
fn active_claims_block_but_expired_claims_do_not() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("db");
    let mut store = open(&db);
    let key = ready(&mut store, "Claimed", 100);
    send(
        &mut store,
        json!({"op":"claim","key":key,"expected_version":2,"lease_seconds":60}),
        Role::Agent,
        200,
    )
    .unwrap();

    let active = deletion(&mut store, &key, Role::Human, 259);
    assert_eq!(kinds(&active), vec!["status", "active_claim"]);
    assert_eq!(
        blocker(&active, "active_claim")["references"],
        json!(["agent"])
    );
    let refused = delete(&mut store, &key, 3, 259).unwrap_err();
    assert_eq!(refused.code, "invalid");
    assert!(
        refused.message.contains("active claim"),
        "{}",
        refused.message
    );

    let expired = deletion(&mut store, &key, Role::Human, 260);
    assert_eq!(kinds(&expired), vec!["status"]);

    // An expired lease left on unstarted work (for example from a restored archive)
    // must not block deletion, matching the claim rule used everywhere else.
    let mut archive = store.export().unwrap();
    let issue = archive
        .issues
        .iter_mut()
        .find(|issue| issue.key == key)
        .unwrap();
    issue.status = Status::Ready;
    assert!(issue
        .claim
        .as_ref()
        .is_some_and(|claim| claim.expires_at == 260));
    let mut restored = open(&dir.path().join("restored"));
    restored.restore(archive).unwrap();
    let before = deletion(&mut restored, &key, Role::Human, 261);
    assert_eq!(before["eligible"], true, "{before}");
    let early = deletion(&mut restored, &key, Role::Human, 259);
    assert_eq!(kinds(&early), vec!["active_claim"]);
    assert_eq!(
        delete(&mut restored, &key, 3, 259).unwrap_err().code,
        "conflict"
    );
    delete(&mut restored, &key, 3, 261).unwrap();
}

#[test]
fn stale_version_and_agent_role_are_refused_without_side_effects() {
    let dir = TempDir::new().unwrap();
    let mut store = open(&dir.path().join("db"));
    let key = inbox(&mut store, "Disposable", 100);
    send(
        &mut store,
        json!({"op":"update_issue","key":key,"expected_version":1,"title":"Disposable v2","body":"","acceptance":"","owner":"","priority":"low"}),
        Role::Human,
        101,
    )
    .unwrap();

    // Agents can read the reason but never delete, even when the issue is eligible.
    let seen_by_agent = deletion(&mut store, &key, Role::Agent, 102);
    assert_eq!(seen_by_agent["eligible"], true);
    let denied = send(
        &mut store,
        json!({"op":"delete_issue","key":key,"expected_version":2}),
        Role::Agent,
        103,
    )
    .unwrap_err();
    assert_eq!(denied.code, "forbidden");

    let stale = delete(&mut store, &key, 1, 104).unwrap_err();
    assert_eq!(stale.code, "conflict");
    assert!(
        stale.message.contains("version 2, not 1"),
        "{}",
        stale.message
    );
    assert_eq!(version_of(&mut store, &key, 105), 2);
}

#[test]
fn preflight_eligibility_does_not_authorize_a_later_deletion() {
    let dir = TempDir::new().unwrap();
    let mut store = open(&dir.path().join("db"));
    let key = inbox(&mut store, "Looks deletable", 100);
    let preview = deletion(&mut store, &key, Role::Human, 101);
    assert_eq!(preview["eligible"], true);
    assert_eq!(preview["version"], 1);

    // A release references the issue after the preview without changing the issue version.
    send(
        &mut store,
        json!({"op":"create_release","product":"DIR","name":"Late","version_label":"v-late","target_ref":"refs/heads/main","issue_keys":[key]}),
        Role::Human,
        102,
    )
    .unwrap();
    assert_eq!(version_of(&mut store, &key, 103), 1);
    let refused = delete(&mut store, &key, 1, 104).unwrap_err();
    assert_eq!(refused.code, "conflict");
    assert!(refused.message.contains("v-late"), "{}", refused.message);
    assert_eq!(version_of(&mut store, &key, 105), 1);
}

#[test]
fn successful_deletion_is_idempotent_survives_restart_and_never_reuses_the_key() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("db");
    let mut store = open(&db);
    let key = inbox(&mut store, "Disposable", 100);
    let keep = inbox(&mut store, "Keep", 101);
    assert_eq!(
        deletion(&mut store, &key, Role::Human, 102),
        json!({"key":key,"version":1,"eligible":true,"blockers":[]})
    );
    let request = json!({
        "actor":"owner","request_id":"dir-51-delete","op":"delete_issue",
        "key":key,"expected_version":1
    });
    let deleted = store
        .execute_at(
            serde_json::from_value(request.clone()).unwrap(),
            Role::Human,
            103,
        )
        .unwrap();
    assert_eq!(deleted["deleted_key"], key);
    let replay = store
        .execute_at(
            serde_json::from_value(request.clone()).unwrap(),
            Role::Human,
            104,
        )
        .unwrap();
    assert_eq!(replay, deleted);

    drop(store);
    let mut store = open(&db);
    let replay_after_restart = store
        .execute_at(serde_json::from_value(request).unwrap(), Role::Human, 105)
        .unwrap();
    assert_eq!(replay_after_restart, deleted);
    assert_eq!(
        send(
            &mut store,
            json!({"op":"context","key":key}),
            Role::Human,
            106
        )
        .unwrap_err()
        .code,
        "not_found"
    );
    assert_eq!(version_of(&mut store, &keep, 107), 1);
    assert_eq!(inbox(&mut store, "Next", 108), "DIR-3");

    let archive = store.export().unwrap();
    validate_archive(&archive).unwrap();
    let mut restored = open(&dir.path().join("restored"));
    restored.restore(archive).unwrap();
    assert_eq!(
        send(
            &mut restored,
            json!({"op":"context","key":key}),
            Role::Human,
            109
        )
        .unwrap_err()
        .code,
        "not_found"
    );
    assert_eq!(inbox(&mut restored, "After restore", 110), "DIR-4");
}

#[test]
fn deployed_release_reference_is_not_advertised_as_removable() {
    let dir = TempDir::new().unwrap();
    let mut store = open(&dir.path().join("db"));
    let draft = inbox(&mut store, "Unstarted release item", 100);
    let work = ready(&mut store, "Work that supplied the preview commit", 101);
    send(
        &mut store,
        json!({"op":"claim","key":work,"expected_version":2}),
        Role::Agent,
        103,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"record_git_trace","key":work,"expected_version":3,"kind":"commit","repository":"example/direct","commit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","branch":"codex/release"}),
        Role::Agent,
        104,
    ).unwrap();
    let trace = store.export().unwrap().git_traces[0].id.clone();
    let release = send(
        &mut store,
        json!({"op":"create_release","product":"DIR","name":"Preview","version_label":"v-preview","target_ref":"refs/heads/main","issue_keys":[draft,work]}),
        Role::Human,
        105,
    ).unwrap();
    send(
        &mut store,
        json!({"op":"record_release_evidence","release_id":release["id"],"expected_version":1,"kind":"commit","git_trace_id":trace}),
        Role::Human,
        106,
    ).unwrap();
    send(
        &mut store,
        json!({"op":"record_release_evidence","release_id":release["id"],"expected_version":2,"kind":"preview_deployment","deployment_ref":"preview-1","commit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","target_ref":"refs/heads/main","environment":"preview"}),
        Role::Human,
        107,
    ).unwrap();
    let planned = send(
        &mut store,
        json!({"op":"create_release","product":"DIR","name":"Planned","version_label":"v-planned","target_ref":"refs/heads/main","issue_keys":[draft]}),
        Role::Human,
        108,
    ).unwrap();

    let eligibility = deletion(&mut store, &draft, Role::Human, 109);
    assert_eq!(eligibility["eligible"], false);
    let blockers = eligibility["blockers"].as_array().unwrap();
    assert_eq!(blockers.len(), 2);
    let frozen = blockers
        .iter()
        .find(|item| item["removable"] == false)
        .unwrap();
    assert_eq!(frozen["kind"], "release_references");
    assert_eq!(frozen["references"], json!([release["id"]]));
    assert!(frozen["message"].as_str().unwrap().contains("frozen"));
    assert!(!frozen["message"]
        .as_str()
        .unwrap()
        .contains("remove the issue"));
    let editable = blockers
        .iter()
        .find(|item| item["removable"] == true)
        .unwrap();
    assert_eq!(editable["references"], json!([planned["id"]]));
    assert_eq!(editable["count"], 1);
    let refused = delete(&mut store, &draft, 1, 110).unwrap_err();
    assert!(refused.message.contains("frozen"));
    let unlink = send(
        &mut store,
        json!({"op":"update_release","id":release["id"],"expected_version":3,"name":"Preview","version_label":"v-preview","status":"preview","target_ref":"refs/heads/main","project_ids":[],"issue_keys":[work]}),
        Role::Human,
        111,
    ).unwrap_err();
    assert_eq!(unlink.code, "invalid");
    assert!(unlink.message.contains("frozen"));
    assert_eq!(version_of(&mut store, &draft, 112), 1);
}
