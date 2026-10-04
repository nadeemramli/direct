use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(s: &mut Store, mut value: Value, role: Role) -> Result<Value> {
    value["actor"] = json!("label-builder");
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    s.execute_at(serde_json::from_value(value).unwrap(), role, 100)
}
fn as_actor(s: &mut Store, mut value: Value, actor: &str, role: Role) -> Result<Value> {
    value["actor"] = json!(actor);
    value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    s.execute_at(serde_json::from_value(value).unwrap(), role, 101)
}
fn label(s: &mut Store, name: &str) -> Value {
    send(
        &mut *s,
        json!({"op":"create_label","name":name}),
        Role::Human,
    )
    .unwrap()
}
fn issue(s: &mut Store, product: &str, title: &str) -> Value {
    send(
        &mut *s,
        json!({"op":"create_issue","product":product,"title":title,"body":"Work"}),
        Role::Agent,
    )
    .unwrap()
}
fn ready_issue(s: &mut Store, title: &str) -> String {
    let i = issue(s, "DIR", title);
    let key = i["key"].as_str().unwrap().to_string();
    send(
        s,
        json!({"op":"update_issue","key":key,"expected_version":1,"title":title,"body":"Work","acceptance":"Proof","owner":"reviewer","priority":"high"}),
        Role::Agent,
    )
    .unwrap();
    send(
        s,
        json!({"op":"ready","key":key,"expected_version":2}),
        Role::Human,
    )
    .unwrap();
    key
}
fn ids(value: &Value) -> Vec<String> {
    value["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn canonical_names_aliases_and_linear_origins_are_unique_and_owner_controlled() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let create = json!({"op":"create_label","name":" Bug ","description":"Defects","color":"#D73A49","aliases":["defect","bugfix"],"linear_origins":[{"id":"lin-bug-1","name":"Bug"}],"request_id":"label-create"});
    assert_eq!(
        send(&mut s, create.clone(), Role::Agent).unwrap_err().code,
        "forbidden"
    );
    let bug = send(&mut s, create.clone(), Role::Human).unwrap();
    assert_eq!(bug["name"], "Bug");
    assert_eq!(bug["color"], "#d73a49");
    assert_eq!(bug["aliases"], json!(["defect", "bugfix"]));
    assert_eq!(bug["version"], 1);
    assert_eq!(send(&mut s, create, Role::Human).unwrap(), bug);

    for (name, aliases, code) in [
        ("bug", json!([]), "conflict"),
        ("Defect", json!([]), "conflict"),
        ("Regression", json!(["BUG"]), "conflict"),
        ("Regression", json!(["bugfix"]), "conflict"),
        ("Regression", json!(["fast", "Fast"]), "invalid"),
        ("Regression", json!(["regression"]), "invalid"),
        ("Regression", json!([""]), "invalid"),
        ("", json!([]), "invalid"),
    ] {
        assert_eq!(
            send(
                &mut s,
                json!({"op":"create_label","name":name,"aliases":aliases}),
                Role::Human
            )
            .unwrap_err()
            .code,
            code,
            "{name} {aliases}"
        );
    }
    assert_eq!(
        send(
            &mut s,
            json!({"op":"create_label","name":"Regression","color":"red"}),
            Role::Human
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    assert_eq!(
        send(
            &mut s,
            json!({"op":"create_label","name":"Regression","linear_origins":[{"id":"lin-bug-1","name":"Bug (other team)"}]}),
            Role::Human
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    assert_eq!(
        send(
            &mut s,
            json!({"op":"create_label","name":"Regression","linear_origins":[{"id":"lin-2"},{"id":"lin-2"}]}),
            Role::Human
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    assert_eq!(
        send(
            &mut s,
            json!({"op":"create_label","name":"Regression","products":[{"product_id":"missing"}]}),
            Role::Human
        )
        .unwrap_err()
        .code,
        "not_found"
    );

    let perf = label(&mut s, "Performance");
    let update = json!({"op":"update_label","id":perf["id"],"expected_version":1,"name":"Speed","aliases":["perf"],"linear_origins":[{"id":"lin-perf","name":"Performance"}]});
    assert_eq!(
        send(&mut s, update.clone(), Role::Agent).unwrap_err().code,
        "forbidden"
    );
    let updated = send(&mut s, update.clone(), Role::Human).unwrap();
    assert_eq!(updated["name"], "Speed");
    assert_eq!(updated["version"], 2);
    assert_eq!(
        send(&mut s, update, Role::Human).unwrap_err().code,
        "conflict"
    );
    assert_eq!(
        send(
            &mut s,
            json!({"op":"update_label","id":perf["id"],"expected_version":2,"name":"Defect"}),
            Role::Human
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    assert_eq!(
        send(
            &mut s,
            json!({"op":"update_label","id":perf["id"],"expected_version":2,"name":"Speed","linear_origins":[{"id":"lin-bug-1"}]}),
            Role::Human
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    // Renaming keeps the same stable ID, and the original Linear identity remains recorded.
    let renamed = send(
        &mut s,
        json!({"op":"update_label","id":perf["id"],"expected_version":2,"name":"Latency","aliases":["perf","speed"],"linear_origins":[{"id":"lin-perf","name":"Performance"}]}),
        Role::Human,
    )
    .unwrap();
    assert_eq!(renamed["id"], perf["id"]);
    assert_eq!(renamed["linear_origins"][0]["name"], "Performance");
    assert_eq!(
        send(
            &mut s,
            json!({"op":"update_label","id":"missing","expected_version":1,"name":"X"}),
            Role::Human
        )
        .unwrap_err()
        .code,
        "not_found"
    );
    let snapshot = send(&mut s, json!({"op":"snapshot"}), Role::Agent).unwrap();
    assert_eq!(snapshot["labels"].as_array().unwrap().len(), 2);
    validate_archive(&s.export().unwrap()).unwrap();
}

#[test]
fn product_applicability_and_defaults_share_one_definition() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let snapshot = send(&mut s, json!({"op":"snapshot"}), Role::Agent).unwrap();
    let dir_id = snapshot["products"][0]["id"].as_str().unwrap().to_string();
    let alt = send(
        &mut s,
        json!({"op":"create_product","key":"ALT","name":"Other"}),
        Role::Human,
    )
    .unwrap();
    let alt_id = alt["id"].as_str().unwrap().to_string();
    send(
        &mut s,
        json!({"op":"create_product","key":"THR","name":"Third"}),
        Role::Human,
    )
    .unwrap();
    let triage = send(
        &mut s,
        json!({"op":"create_label","name":"Needs triage","products":[{"product_id":dir_id,"default_for_new_issues":true},{"product_id":alt_id}]}),
        Role::Human,
    )
    .unwrap();
    let triage_id = triage["id"].as_str().unwrap().to_string();
    assert_eq!(
        send(
            &mut s,
            json!({"op":"create_label","name":"Broken","products":[{"product_id":dir_id},{"product_id":dir_id}]}),
            Role::Human
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    let everywhere = label(&mut s, "Everywhere");

    let dir_issue = issue(&mut s, "DIR", "Captured in Direct");
    assert_eq!(ids(&dir_issue), vec![triage_id.clone()]);
    let alt_issue = issue(&mut s, "ALT", "Captured elsewhere");
    assert!(ids(&alt_issue).is_empty());
    let thr_issue = issue(&mut s, "THR", "Out of scope");
    assert!(ids(&thr_issue).is_empty());

    let attached = send(
        &mut s,
        json!({"op":"attach_issue_label","key":alt_issue["key"],"expected_version":1,"label_id":triage_id}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(ids(&attached), vec![triage_id.clone()]);
    assert_eq!(
        send(
            &mut s,
            json!({"op":"attach_issue_label","key":thr_issue["key"],"expected_version":1,"label_id":triage_id}),
            Role::Agent,
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    send(
        &mut s,
        json!({"op":"attach_issue_label","key":thr_issue["key"],"expected_version":1,"label_id":everywhere["id"]}),
        Role::Agent,
    )
    .unwrap();

    // Still one definition, however many products use it.
    let snapshot = send(&mut s, json!({"op":"snapshot"}), Role::Agent).unwrap();
    assert_eq!(snapshot["labels"].as_array().unwrap().len(), 2);
    let labelled: Vec<_> = snapshot["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| ids(i).contains(&triage_id))
        .map(|i| i["key"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(labelled.len(), 2);

    // Narrowing applicability cannot orphan an existing assignment.
    let narrow = json!({"op":"update_label","id":triage_id,"expected_version":1,"name":"Needs triage","products":[{"product_id":dir_id,"default_for_new_issues":true}]});
    assert_eq!(
        send(&mut s, narrow.clone(), Role::Human).unwrap_err().code,
        "invalid"
    );
    send(
        &mut s,
        json!({"op":"detach_issue_label","key":alt_issue["key"],"expected_version":2,"label_id":triage_id}),
        Role::Agent,
    )
    .unwrap();
    send(&mut s, narrow, Role::Human).unwrap();
    assert_eq!(
        send(
            &mut s,
            json!({"op":"attach_issue_label","key":alt_issue["key"],"expected_version":3,"label_id":triage_id}),
            Role::Agent,
        )
        .unwrap_err()
        .code,
        "invalid"
    );

    // Turning the default off stops future auto-attachment without touching existing work.
    send(
        &mut s,
        json!({"op":"update_label","id":triage_id,"expected_version":2,"name":"Needs triage","products":[{"product_id":dir_id}]}),
        Role::Human,
    )
    .unwrap();
    let later = issue(&mut s, "DIR", "Later capture");
    assert!(ids(&later).is_empty());
    let ctx = send(
        &mut s,
        json!({"op":"context","key":dir_issue["key"]}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(ctx["issue"]["labels"], json!([triage_id]));
    assert_eq!(ctx["labels"][0]["name"], "Needs triage");
    validate_archive(&s.export().unwrap()).unwrap();
}

#[test]
fn attach_and_detach_respect_versions_claims_products_and_history() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let bug = label(&mut s, "Bug");
    let bug_id = bug["id"].as_str().unwrap();
    let key = ready_issue(&mut s, "Fix the thing");
    let attach = json!({"op":"attach_issue_label","key":key,"expected_version":3,"label_id":bug_id,"request_id":"attach-bug"});
    assert_eq!(
        send(
            &mut s,
            json!({"op":"attach_issue_label","key":key,"expected_version":2,"label_id":bug_id}),
            Role::Agent
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    assert_eq!(
        send(
            &mut s,
            json!({"op":"attach_issue_label","key":key,"expected_version":3,"label_id":"missing"}),
            Role::Agent
        )
        .unwrap_err()
        .code,
        "not_found"
    );
    assert_eq!(
        send(
            &mut s,
            json!({"op":"detach_issue_label","key":key,"expected_version":3,"label_id":bug_id}),
            Role::Agent
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    let attached = send(&mut s, attach.clone(), Role::Agent).unwrap();
    assert_eq!(send(&mut s, attach, Role::Agent).unwrap(), attached);
    assert_eq!(attached["version"], 4);
    assert_eq!(
        send(
            &mut s,
            json!({"op":"attach_issue_label","key":key,"expected_version":4,"label_id":bug_id}),
            Role::Agent
        )
        .unwrap_err()
        .code,
        "conflict"
    );

    // Doing work needs the active claim; strangers cannot relabel it.
    as_actor(
        &mut s,
        json!({"op":"claim","key":key,"expected_version":4}),
        "coding-agent",
        Role::Agent,
    )
    .unwrap();
    assert_eq!(
        as_actor(
            &mut s,
            json!({"op":"detach_issue_label","key":key,"expected_version":5,"label_id":bug_id}),
            "another-agent",
            Role::Agent
        )
        .unwrap_err()
        .code,
        "claim_required"
    );
    let detached = as_actor(
        &mut s,
        json!({"op":"detach_issue_label","key":key,"expected_version":5,"label_id":bug_id}),
        "coding-agent",
        Role::Agent,
    )
    .unwrap();
    assert!(ids(&detached).is_empty());
    assert_eq!(detached["status"], "doing");
    assert_eq!(detached["claim"]["actor"], "coding-agent");

    // Submitted work stays owner-only for label changes, like project changes.
    let submitted = as_actor(
        &mut s,
        json!({"op":"submit","key":key,"expected_version":6,"build_ref":"commit:abc","e2e":{"build_ref":"commit:abc","environment":"isolated Windows fixture","entrypoint":"fixture client","scenarios":"Exercise the full fixture workflow; expected and observed state transitions match","outcome":"passed","delivered_build_ref":"commit:abc","delivery_check":"Fixture service and client use the tested build"},"delivery_ref":"branch","summary":"Done","checks":"Passed","steps":[{"instruction":"Try","expected":"Works"}]}),
        "coding-agent",
        Role::Agent,
    )
    .unwrap();
    let relabel =
        json!({"op":"attach_issue_label","key":key,"expected_version":7,"label_id":bug_id});
    assert_eq!(
        send(&mut s, relabel.clone(), Role::Agent).unwrap_err().code,
        "forbidden"
    );
    let relabelled = send(&mut s, relabel, Role::Human).unwrap();
    assert_eq!(relabelled["status"], "verify");
    assert_eq!(relabelled["current_run"], submitted["current_run"]);
    let child_key = relabelled["verification_key"].as_str().unwrap();
    assert_eq!(
        send(
            &mut s,
            json!({"op":"attach_issue_label","key":child_key,"expected_version":2,"label_id":bug_id}),
            Role::Human
        )
        .unwrap_err()
        .code,
        "invalid"
    );

    let ctx = send(&mut s, json!({"op":"context","key":key}), Role::Agent).unwrap();
    assert_eq!(ctx["labels"][0]["id"], bug["id"]);
    let kinds: Vec<_> = ctx["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap().to_string())
        .collect();
    assert!(kinds.contains(&"issue_label_attached".into()));
    assert!(kinds.contains(&"issue_label_detached".into()));

    // Projects: owner-only, versioned, product-scoped.
    let project = send(
        &mut s,
        json!({"op":"create_project","product":"DIR","name":"Pilot"}),
        Role::Human,
    )
    .unwrap();
    let alt = send(
        &mut s,
        json!({"op":"create_product","key":"ALT","name":"Other"}),
        Role::Human,
    )
    .unwrap();
    let alt_only = send(
        &mut s,
        json!({"op":"create_label","name":"Alt only","products":[{"product_id":alt["id"]}]}),
        Role::Human,
    )
    .unwrap();
    let project_attach = json!({"op":"attach_project_label","id":project["id"],"expected_version":1,"label_id":bug_id});
    assert_eq!(
        send(&mut s, project_attach.clone(), Role::Agent)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let labelled_project = send(&mut s, project_attach.clone(), Role::Human).unwrap();
    assert_eq!(labelled_project["version"], 2);
    assert_eq!(ids(&labelled_project), vec![bug_id.to_string()]);
    assert_eq!(
        send(&mut s, project_attach, Role::Human).unwrap_err().code,
        "conflict"
    );
    assert_eq!(
        send(
            &mut s,
            json!({"op":"attach_project_label","id":project["id"],"expected_version":2,"label_id":alt_only["id"]}),
            Role::Human
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    assert_eq!(
        send(
            &mut s,
            json!({"op":"attach_project_label","id":"missing","expected_version":1,"label_id":bug_id}),
            Role::Human
        )
        .unwrap_err()
        .code,
        "not_found"
    );
    assert_eq!(
        send(
            &mut s,
            json!({"op":"detach_project_label","id":project["id"],"expected_version":2,"label_id":alt_only["id"]}),
            Role::Human
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    send(
        &mut s,
        json!({"op":"set_issue_project","key":key,"expected_version":8,"project_id":project["id"]}),
        Role::Human,
    )
    .unwrap();
    let ctx = send(&mut s, json!({"op":"context","key":key}), Role::Agent).unwrap();
    assert_eq!(ctx["project"]["labels"], json!([bug_id]));
    assert_eq!(ctx["project_labels"][0]["name"], "Bug");
    let snapshot = send(&mut s, json!({"op":"snapshot"}), Role::Agent).unwrap();
    assert_eq!(snapshot["projects"][0]["labels"], json!([bug_id]));
    let detached_project = send(
        &mut s,
        json!({"op":"detach_project_label","id":project["id"],"expected_version":2,"label_id":bug_id}),
        Role::Human,
    )
    .unwrap();
    assert!(ids(&detached_project).is_empty());
    let changes = send(&mut s, json!({"op":"changes","after":0}), Role::Agent).unwrap();
    let kinds: Vec<_> = changes["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap().to_string())
        .collect();
    assert!(kinds.contains(&"label_created".into()));
    assert!(kinds.contains(&"project_label_attached".into()));
    assert!(kinds.contains(&"project_label_detached".into()));
    validate_archive(&s.export().unwrap()).unwrap();
}

#[test]
fn labels_never_change_readiness_priority_ownership_or_verification() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let bug = label(&mut s, "Bug");
    let bug_id = bug["id"].as_str().unwrap();
    let key = ready_issue(&mut s, "Ready work");
    let before = send(&mut s, json!({"op":"context","key":key}), Role::Agent).unwrap();
    assert_eq!(before["issue"]["status"], "ready");
    let after = send(
        &mut s,
        json!({"op":"attach_issue_label","key":key,"expected_version":3,"label_id":bug_id}),
        Role::Agent,
    )
    .unwrap();
    // Editing the brief would return Ready work to Backlog; labelling it does not.
    for field in [
        "status",
        "priority",
        "owner",
        "acceptance",
        "body",
        "claim",
        "needs_fix",
    ] {
        assert_eq!(after[field], before["issue"][field], "{field}");
    }
    assert_eq!(after["version"], 4);

    send(
        &mut s,
        json!({"op":"claim","key":key,"expected_version":4}),
        Role::Agent,
    )
    .unwrap();
    let submitted = send(
        &mut s,
        json!({"op":"submit","key":key,"expected_version":5,"build_ref":"commit:abc","e2e":{"build_ref":"commit:abc","environment":"isolated Windows fixture","entrypoint":"fixture client","scenarios":"Exercise the full fixture workflow; expected and observed state transitions match","outcome":"passed","delivered_build_ref":"commit:abc","delivery_check":"Fixture service and client use the tested build"},"delivery_ref":"branch","summary":"Done","checks":"Passed","steps":[{"instruction":"Try","expected":"Works"}]}),
        Role::Agent,
    )
    .unwrap();
    let run_id = submitted["current_run"].as_str().unwrap().to_string();
    let relabelled = send(
        &mut s,
        json!({"op":"detach_issue_label","key":key,"expected_version":6,"label_id":bug_id}),
        Role::Human,
    )
    .unwrap();
    assert_eq!(relabelled["status"], "verify");
    assert_eq!(relabelled["current_run"], run_id);
    assert_eq!(relabelled["needs_fix"], false);
    let ctx = send(&mut s, json!({"op":"context","key":key}), Role::Agent).unwrap();
    assert_eq!(ctx["verifications"][0]["outcome"], "pending");
    assert_eq!(ctx["verifications"].as_array().unwrap().len(), 1);
    // The pending run still reviews normally against the unchanged run ID.
    send(
        &mut s,
        json!({"op":"review","key":key,"expected_version":7,"run_id":run_id,"outcome":"passed","results":[{"outcome":"passed"}]}),
        Role::Human,
    )
    .unwrap();
    let done = send(
        &mut s,
        json!({"op":"attach_issue_label","key":key,"expected_version":8,"label_id":bug_id}),
        Role::Human,
    )
    .unwrap();
    assert_eq!(done["status"], "done");
    let archive = s.export().unwrap();
    validate_archive(&archive).unwrap();
    let child = archive.issues.iter().find(|i| i.parent.is_some()).unwrap();
    assert!(child.labels.is_empty());
}

#[test]
fn definitions_and_assignments_survive_restart_archive_and_legacy_formats() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let snapshot = send(&mut s, json!({"op":"snapshot"}), Role::Agent).unwrap();
    let dir_id = snapshot["products"][0]["id"].as_str().unwrap().to_string();
    let bug = send(
        &mut s,
        json!({"op":"create_label","name":"Bug","description":"Defects","color":"#d73a49","aliases":["defect"],"products":[{"product_id":dir_id,"default_for_new_issues":true}],"linear_origins":[{"id":"lin-bug","name":"Bug"}]}),
        Role::Human,
    )
    .unwrap();
    let project = send(
        &mut s,
        json!({"op":"create_project","product":"DIR","name":"Pilot"}),
        Role::Human,
    )
    .unwrap();
    send(
        &mut s,
        json!({"op":"attach_project_label","id":project["id"],"expected_version":1,"label_id":bug["id"]}),
        Role::Human,
    )
    .unwrap();
    let i = issue(&mut s, "DIR", "Defaulted");
    assert_eq!(ids(&i), vec![bug["id"].as_str().unwrap().to_string()]);
    let before = serde_json::to_value(s.export().unwrap()).unwrap();
    assert_eq!(before["format"], 18);
    assert_eq!(before["labels"][0]["aliases"], json!(["defect"]));
    assert_eq!(before["labels"][0]["linear_origins"][0]["id"], "lin-bug");
    assert_eq!(
        before["labels"][0]["products"][0]["default_for_new_issues"],
        true
    );
    drop(s);

    // Restart.
    let s = Store::open(&path).unwrap();
    assert_eq!(serde_json::to_value(s.export().unwrap()).unwrap(), before);

    // Archive round trip.
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(s.export().unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        before
    );
    let later = issue(&mut restored, "DIR", "After restore");
    assert_eq!(ids(&later), ids(&i));

    // Invalid references never replace the destination.
    let mut broken = s.export().unwrap();
    broken.issues[0].labels = vec!["missing".into()];
    assert!(restored.restore(broken).is_err());
    let mut broken = s.export().unwrap();
    broken.projects[0].labels = vec!["missing".into()];
    assert!(restored.restore(broken).is_err());
    let mut broken = s.export().unwrap();
    broken.labels.push(Label {
        id: uuid::Uuid::new_v4().to_string(),
        name: "bug".into(),
        description: String::new(),
        color: String::new(),
        aliases: vec![],
        products: vec![],
        linear_origins: vec![],
        version: 1,
        created_at: 100,
        updated_at: 100,
    });
    assert!(restored.restore(broken).is_err());
    let mut broken = s.export().unwrap();
    broken.labels[0].products[0].product_id = "other-product".into();
    assert!(restored.restore(broken).is_err());

    // A label defined in another workspace is a cross-workspace reference, even with a valid UUID.
    let mut foreign = Store::open(&dir.path().join("foreign")).unwrap();
    let foreign_label = label(&mut foreign, "Foreign");
    let mut crossed = s.export().unwrap();
    crossed.issues[0]
        .labels
        .push(foreign_label["id"].as_str().unwrap().into());
    assert!(restored.restore(crossed).is_err());
    assert_eq!(
        send(
            &mut restored,
            json!({"op":"attach_issue_label","key":later["key"],"expected_version":1,"label_id":foreign_label["id"]}),
            Role::Agent
        )
        .unwrap_err()
        .code,
        "not_found"
    );
    let restored_export = serde_json::to_value(restored.export().unwrap()).unwrap();
    assert_eq!(restored_export["labels"], before["labels"]);
    let original = restored_export["issues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|issue| issue["key"] == before["issues"][0]["key"])
        .unwrap();
    assert_eq!(*original, before["issues"][0]);

    // Older archives without labels still restore; label data needs format 11.
    let mut legacy = serde_json::to_value(s.export().unwrap()).unwrap();
    legacy["format"] = json!(5);
    legacy.as_object_mut().unwrap().remove("labels");
    legacy["issues"][0]
        .as_object_mut()
        .unwrap()
        .remove("labels");
    legacy["projects"][0]
        .as_object_mut()
        .unwrap()
        .remove("labels");
    let mut legacy_store = Store::open(&dir.path().join("legacy")).unwrap();
    legacy_store
        .restore(serde_json::from_value(legacy.clone()).unwrap())
        .unwrap();
    let upgraded = legacy_store.export().unwrap();
    assert!(upgraded.labels.is_empty());
    assert!(upgraded.issues.iter().all(|i| i.labels.is_empty()));
    assert!(upgraded.projects.iter().all(|p| p.labels.is_empty()));
    assert_eq!(upgraded.issues[0].planning_scope, PlanningScope::Inbox);
    let mut mislabeled = legacy.clone();
    mislabeled["labels"] = before["labels"].clone();
    assert_eq!(
        validate_archive(&serde_json::from_value(mislabeled).unwrap())
            .unwrap_err()
            .code,
        "invalid"
    );
    let mut mislabeled = legacy;
    mislabeled["issues"][0]["labels"] = json!(["x"]);
    assert!(validate_archive(&serde_json::from_value(mislabeled).unwrap()).is_err());

    // A schema-5 database opens, upgrades to schema 11 and keeps issue identity and replays.
    drop(s);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("DROP TABLE labels; UPDATE meta SET value='5' WHERE key='schema';")
        .unwrap();
    let stripped_issue = {
        let mut value = before["issues"][0].clone();
        value.as_object_mut().unwrap().remove("labels");
        value
    };
    conn.execute(
        "UPDATE issues SET data=?1 WHERE key=?2",
        rusqlite::params![
            serde_json::to_string(&stripped_issue).unwrap(),
            before["issues"][0]["key"].as_str().unwrap()
        ],
    )
    .unwrap();
    let stripped_project = {
        let mut value = before["projects"][0].clone();
        value.as_object_mut().unwrap().remove("labels");
        value
    };
    conn.execute(
        "UPDATE projects SET data=?1",
        [serde_json::to_string(&stripped_project).unwrap()],
    )
    .unwrap();
    drop(conn);
    let mut upgraded = Store::open(&path).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT value FROM meta WHERE key='schema'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "16"
    );
    let archive = upgraded.export().unwrap();
    assert_eq!(archive.format, 18);
    assert!(archive.labels.is_empty());
    assert_eq!(archive.issues[0].key, before["issues"][0]["key"]);
    assert!(archive.issues[0].labels.is_empty());
    assert_eq!(archive.issues[0].planning_scope, PlanningScope::Inbox);
    let fresh = label(&mut upgraded, "After upgrade");
    send(
        &mut upgraded,
        json!({"op":"attach_issue_label","key":archive.issues[0].key,"expected_version":archive.issues[0].version,"label_id":fresh["id"]}),
        Role::Agent,
    )
    .unwrap();
    validate_archive(&upgraded.export().unwrap()).unwrap();
}
