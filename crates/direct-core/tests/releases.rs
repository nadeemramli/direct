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
        json!({"op":"record_release_evidence","release_id":release_id,"expected_version":1,"kind":"preview_deployment","deployment_ref":"preview-1","commit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","target_ref":"refs/heads/main","environment":"preview","url":"https://preview.example.test"}),
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
    let failed_preview = send(
        &mut store,
        json!({"op":"record_release_evidence","release_id":release_id,"expected_version":3,"kind":"preview_deployment","deployment_ref":"preview-failed","commit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","target_ref":"refs/heads/main","environment":"preview","url":"https://preview.example.test","outcome":"failed","note":"Provider rejected the deployment"}),
        Role::Human,
        111,
    )
    .unwrap();
    assert_eq!(failed_preview["release"]["status"], "planned");
    assert_eq!(failed_preview["evidence"]["outcome"], "failed");
    let preview = send(
        &mut store,
        json!({"op":"record_release_evidence","release_id":release_id,"expected_version":4,"kind":"preview_deployment","deployment_ref":"preview-1","commit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","target_ref":"refs/heads/main","environment":"preview","url":"https://preview.example.test"}),
        Role::Human,
        111,
    )
    .unwrap();
    assert_eq!(preview["release"]["status"], "preview");

    let premature_production = send(
        &mut store,
        json!({"op":"record_release_evidence","release_id":release_id,"expected_version":5,"kind":"production_deployment","deployment_ref":"production-1","commit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","target_ref":"refs/heads/main","environment":"production","url":"https://direct.example.test"}),
        Role::Human,
        112,
    )
    .unwrap_err();
    assert_eq!(premature_production.code, "invalid");

    let submitted = send(
        &mut store,
        json!({"op":"submit","key":key,"expected_version":6,"build_ref":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","e2e":{"build_ref":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","environment":"isolated Windows fixture","entrypoint":"fixture client","scenarios":"Exercise the full fixture workflow; expected and observed state transitions match","outcome":"passed","delivered_build_ref":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","delivery_check":"Fixture service and client use the tested build"},"delivery_ref":"origin/codex/release","summary":"Release work","checks":"passed","steps":[{"instruction":"Verify","expected":"Works"}]}),
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
        json!({"op":"record_release_evidence","release_id":release_id,"expected_version":5,"kind":"check","verification_id":run_id}),
        Role::Human,
        115,
    )
    .unwrap();
    let production = send(
        &mut store,
        json!({"op":"record_release_evidence","release_id":release_id,"expected_version":6,"kind":"production_deployment","deployment_ref":"production-1","commit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","target_ref":"refs/heads/main","environment":"production","url":"https://direct.example.test"}),
        Role::Human,
        116,
    )
    .unwrap();
    assert_eq!(production["release"]["status"], "production");
    assert_eq!(production["evidence"]["approver"], "owner");
    let rollback = send(
        &mut store,
        json!({"op":"record_release_evidence","release_id":release_id,"expected_version":7,"kind":"rollback","deployment_ref":"rollback-1","commit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","source_ref":"refs/tags/v0.2.0","target_ref":"refs/heads/main","environment":"production","outcome":"passed","note":"Restored prior production ref"}),
        Role::Human,
        117,
    )
    .unwrap();
    assert_eq!(rollback["release"]["status"], "production");

    let context = send(
        &mut store,
        json!({"op":"context","key":key}),
        Role::Agent,
        118,
    )
    .unwrap();
    assert_eq!(context["releases"].as_array().unwrap().len(), 1);
    assert_eq!(context["release_progress"][0]["completed"], 1);
    assert_eq!(context["release_progress"][0]["pending_verification"], 0);
    assert_eq!(context["release_progress"][0]["failed_verification"], 0);
    assert_eq!(context["release_evidence"].as_array().unwrap().len(), 7);

    let archive = store.export().unwrap();
    assert_eq!(archive.format, 22);
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
        json!({"op":"submit","key":key,"expected_version":4,"build_ref":"build-1","e2e":{"build_ref":"build-1","environment":"isolated Windows fixture","entrypoint":"fixture client","scenarios":"Exercise the full fixture workflow; expected and observed state transitions match","outcome":"passed","delivered_build_ref":"build-1","delivery_check":"Fixture service and client use the tested build"},"delivery_ref":"preview-1","summary":"Candidate","checks":"automated checks passed","steps":[{"instruction":"Exercise flow","expected":"Works"}]}),
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

#[test]
fn product_workflow_controls_refs_and_prevents_ambiguous_branch_ownership() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let workflow = send(
        &mut store,
        json!({"op":"set_release_workflow_config","product":"DIR","branch_strategy":"one_branch_per_release","production_ref":"refs/heads/production","release_branch_pattern":"refs/heads/release/{version}","preview_environment":"staging","preview_url_template":"https://preview.example.test/{version}","promotion_policy":"verified_owner_approval"}),
        Role::Human,
        400,
    )
    .unwrap();
    assert_eq!(workflow["version"], 1);

    let wrong_branch = send(
        &mut store,
        json!({"op":"create_release","product":"DIR","name":"Wrong","version_label":"v1","target_ref":"refs/heads/production","release_branch":"refs/heads/release/not-v1"}),
        Role::Human,
        401,
    )
    .unwrap_err();
    assert_eq!(wrong_branch.code, "invalid");
    send(
        &mut store,
        json!({"op":"create_release","product":"DIR","name":"Release one","version_label":"v1","target_ref":"refs/heads/production","release_branch":"refs/heads/release/v1"}),
        Role::Human,
        402,
    )
    .unwrap();

    send(
        &mut store,
        json!({"op":"set_release_workflow_config","product":"DIR","expected_version":1,"branch_strategy":"external","production_ref":"refs/heads/production","release_branch_pattern":"","preview_environment":"staging","preview_url_template":"","promotion_policy":"external_manual"}),
        Role::Human,
        403,
    )
    .unwrap();
    let conflict = send(
        &mut store,
        json!({"op":"create_release","product":"DIR","name":"Release two","version_label":"v2","target_ref":"refs/heads/production","release_branch":"refs/heads/release/v1"}),
        Role::Human,
        404,
    )
    .unwrap_err();
    assert_eq!(conflict.code, "conflict");

    let snapshot = send(&mut store, json!({"op":"snapshot"}), Role::Agent, 405).unwrap();
    assert_eq!(snapshot["release_workflows"].as_array().unwrap().len(), 1);
    assert_eq!(
        snapshot["release_workflows"][0]["promotion_policy"],
        "external_manual"
    );
    validate_archive(&store.export().unwrap()).unwrap();
}
