use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(store: &mut Store, mut value: Value, role: Role, at: i64) -> Result<Value> {
    value["actor"] = json!(if role == Role::Human {
        "owner"
    } else {
        "release-agent"
    });
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    store.execute_at(serde_json::from_value(value).unwrap(), role, at)
}

#[test]
fn production_requires_distinct_git_preview_and_owner_verified_work() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let issue = send(
        &mut store,
        json!({"op":"create_issue","product":"DIR","title":"Release safely","body":"Ship exact verified work"}),
        Role::Agent,
        100,
    )
    .unwrap();
    let key = issue["key"].as_str().unwrap();
    send(
        &mut store,
        json!({"op":"update_issue","key":key,"expected_version":1,"title":"Release safely","body":"Ship exact verified work","acceptance":"Production requires proof","owner":"owner","priority":"high"}),
        Role::Agent,
        101,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"ready","key":key,"expected_version":2}),
        Role::Human,
        102,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"claim","key":key,"expected_version":3}),
        Role::Agent,
        103,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"record_git_trace","key":key,"expected_version":4,"kind":"commit","repository":"nadeemramli/direct","commit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","branch":"codex/release"}),
        Role::Agent,
        104,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"record_git_trace","key":key,"expected_version":5,"kind":"push","repository":"nadeemramli/direct","commit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","branch":"codex/release","remote":"origin","remote_ref":"refs/heads/codex/release"}),
        Role::Agent,
        105,
    )
    .unwrap();
    let release = send(
        &mut store,
        json!({"op":"create_release","product":"DIR","name":"Pilot release","version_label":"v0.2.0","target_ref":"refs/heads/main","notes":"Verified pilot","issue_keys":[key]}),
        Role::Human,
        106,
    )
    .unwrap();
    let release_id = release["id"].as_str().unwrap();
    let snapshot = send(&mut store, json!({"op":"snapshot"}), Role::Agent, 107).unwrap();
    let traces = snapshot["git_traces"].as_array().unwrap();
    let commit_trace = traces
        .iter()
        .find(|trace| trace["kind"] == "commit")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let push_trace = traces.iter().find(|trace| trace["kind"] == "push").unwrap()["id"]
        .as_str()
        .unwrap();

    let preview_without_trace = send(
        &mut store,
        json!({"op":"record_release_evidence","release_id":release_id,"expected_version":1,"kind":"preview_deployment","deployment_ref":"preview-1","commit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","target_ref":"refs/heads/main","url":"https://preview.example.test"}),
        Role::Human,
        108,
    )
    .unwrap_err();
    assert_eq!(preview_without_trace.code, "invalid");

    let commit_evidence = send(
        &mut store,
        json!({"op":"record_release_evidence","release_id":release_id,"expected_version":1,"kind":"commit","git_trace_id":commit_trace}),
        Role::Human,
        109,
    )
    .unwrap();
    assert_eq!(commit_evidence["release"]["status"], "planned");
    let push_evidence = send(
        &mut store,
        json!({"op":"record_release_evidence","release_id":release_id,"expected_version":2,"kind":"push","git_trace_id":push_trace}),
        Role::Human,
        110,
    )
    .unwrap();
    assert_eq!(push_evidence["release"]["status"], "planned");
    let preview = send(
        &mut store,
        json!({"op":"record_release_evidence","release_id":release_id,"expected_version":3,"kind":"preview_deployment","deployment_ref":"preview-1","commit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","target_ref":"refs/heads/main","url":"https://preview.example.test"}),
        Role::Human,
        111,
    )
    .unwrap();
    assert_eq!(preview["release"]["status"], "preview");

    let premature_production = send(
        &mut store,
        json!({"op":"record_release_evidence","release_id":release_id,"expected_version":4,"kind":"production_deployment","deployment_ref":"production-1","commit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","target_ref":"refs/heads/main","url":"https://direct.example.test"}),
        Role::Human,
        112,
    )
    .unwrap_err();
    assert_eq!(premature_production.code, "invalid");

    let submitted = send(
        &mut store,
        json!({"op":"submit","key":key,"expected_version":6,"build_ref":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","delivery_ref":"origin/codex/release","summary":"Release work","checks":"passed","steps":[{"instruction":"Verify","expected":"Works"}]}),
        Role::Agent,
        113,
    )
    .unwrap();
    let run_id = submitted["current_run"].as_str().unwrap();
    send(
        &mut store,
        json!({"op":"review","key":key,"expected_version":7,"run_id":run_id,"outcome":"passed","results":[{"outcome":"passed","note":"Observed"}],"note":"Accepted"}),
        Role::Human,
        114,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"record_release_evidence","release_id":release_id,"expected_version":4,"kind":"check","verification_id":run_id}),
        Role::Human,
        115,
    )
    .unwrap();
    let production = send(
        &mut store,
        json!({"op":"record_release_evidence","release_id":release_id,"expected_version":5,"kind":"production_deployment","deployment_ref":"production-1","commit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","target_ref":"refs/heads/main","url":"https://direct.example.test"}),
        Role::Human,
        116,
    )
    .unwrap();
    assert_eq!(production["release"]["status"], "production");
    assert_eq!(production["evidence"]["approver"], "owner");

    let context = send(
        &mut store,
        json!({"op":"context","key":key}),
        Role::Agent,
        117,
    )
    .unwrap();
    assert_eq!(context["releases"].as_array().unwrap().len(), 1);
    assert_eq!(context["release_progress"][0]["completed"], 1);
    assert_eq!(context["release_progress"][0]["pending_verification"], 0);
    assert_eq!(context["release_progress"][0]["failed_verification"], 0);
    assert_eq!(context["release_evidence"].as_array().unwrap().len(), 5);

    let archive = store.export().unwrap();
    assert_eq!(archive.format, 9);
    validate_archive(&archive).unwrap();
    let before = serde_json::to_value(&archive).unwrap();
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(archive).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        before
    );
}

#[test]
fn release_links_are_product_scoped_and_agent_mutation_is_forbidden() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let other = send(
        &mut store,
        json!({"op":"create_product","key":"OTHER","name":"Other"}),
        Role::Human,
        200,
    )
    .unwrap();
    let issue = send(
        &mut store,
        json!({"op":"create_issue","product":"OTHER","title":"Other work"}),
        Role::Agent,
        201,
    )
    .unwrap();
    assert!(!other["id"].as_str().unwrap().is_empty());
    let cross_product = send(
        &mut store,
        json!({"op":"create_release","product":"DIR","name":"Bad release","version_label":"v1","target_ref":"refs/heads/main","issue_keys":[issue["key"]]}),
        Role::Human,
        202,
    )
    .unwrap_err();
    assert_eq!(cross_product.code, "invalid");
    let forbidden = send(
        &mut store,
        json!({"op":"create_release","product":"DIR","name":"Agent release","version_label":"v2","target_ref":"refs/heads/main"}),
        Role::Agent,
        203,
    )
    .unwrap_err();
    assert_eq!(forbidden.code, "forbidden");
}

#[test]
fn release_progress_keeps_pending_and_failed_verification_separate() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let issue = send(
        &mut store,
        json!({"op":"create_issue","product":"DIR","title":"Verify release work","body":"Exercise release progress"}),
        Role::Agent,
        300,
    )
    .unwrap();
    let key = issue["key"].as_str().unwrap();
    send(
        &mut store,
        json!({"op":"update_issue","key":key,"expected_version":1,"title":"Verify release work","body":"Exercise release progress","acceptance":"Failure stays visible","owner":"owner","priority":"medium"}),
        Role::Agent,
        301,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"ready","key":key,"expected_version":2}),
        Role::Human,
        302,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"claim","key":key,"expected_version":3}),
        Role::Agent,
        303,
    )
    .unwrap();
    let release = send(
        &mut store,
        json!({"op":"create_release","product":"DIR","name":"Progress release","version_label":"v-progress","target_ref":"refs/heads/main","issue_keys":[key]}),
        Role::Human,
        304,
    )
    .unwrap();
    let release_id = release["id"].as_str().unwrap();
    let submitted = send(
        &mut store,
        json!({"op":"submit","key":key,"expected_version":4,"build_ref":"build-1","delivery_ref":"preview-1","summary":"Candidate","checks":"automated checks passed","steps":[{"instruction":"Exercise flow","expected":"Works"}]}),
        Role::Agent,
        305,
    )
    .unwrap();
    let run_id = submitted["current_run"].as_str().unwrap();
    let pending = send(&mut store, json!({"op":"snapshot"}), Role::Agent, 306).unwrap();
    let progress = pending["release_progress"]
        .as_array()
        .unwrap()
        .iter()
        .find(|progress| progress["release_id"] == release_id)
        .unwrap();
    assert_eq!(progress["pending_verification"], 1);
    assert_eq!(progress["failed_verification"], 0);

    send(
        &mut store,
        json!({"op":"review","key":key,"expected_version":5,"run_id":run_id,"outcome":"failed","results":[{"outcome":"failed","note":"Broken"}],"note":"Needs repair"}),
        Role::Human,
        307,
    )
    .unwrap();
    let failed = send(&mut store, json!({"op":"snapshot"}), Role::Agent, 308).unwrap();
    let progress = failed["release_progress"]
        .as_array()
        .unwrap()
        .iter()
        .find(|progress| progress["release_id"] == release_id)
        .unwrap();
    assert_eq!(progress["pending_verification"], 0);
    assert_eq!(progress["failed_verification"], 1);
    assert_eq!(progress["active"], 0);
    assert_eq!(progress["completed"], 0);
}
