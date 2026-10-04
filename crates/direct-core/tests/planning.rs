use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(store: &mut Store, mut value: Value, role: Role, at: i64) -> Result<Value> {
    value["actor"] = json!("planning-builder");
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    store.execute_at(serde_json::from_value(value).unwrap(), role, at)
}

#[test]
fn goals_milestones_progress_context_and_restore_follow_real_outcomes() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let project_a = send(
        &mut store,
        json!({"op":"create_project","product":"DIR","name":"Platform"}),
        Role::Human,
        100,
    )
    .unwrap();
    let project_b = send(
        &mut store,
        json!({"op":"create_project","product":"DIR","name":"Client"}),
        Role::Human,
        101,
    )
    .unwrap();
    let create_goal = json!({
        "op":"create_goal","product":"DIR","name":"Adopt Direct","description":"Replace Linear",
        "priority":"urgent","project_ids":[project_a["id"],project_b["id"]],
        "external_source":"linear","external_id":"initiative-123"
    });
    assert_eq!(
        send(&mut store, create_goal.clone(), Role::Agent, 102)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let goal = send(&mut store, create_goal, Role::Human, 102).unwrap();
    assert_eq!(goal["status"], "planned");
    assert_eq!(goal["priority"], "urgent");
    assert_eq!(goal["project_ids"].as_array().unwrap().len(), 2);
    let milestone = send(
        &mut store,
        json!({"op":"create_milestone","project_id":project_a["id"],"name":"Foundation","description":"Core model","sort_order":2,"external_source":"linear","external_id":"milestone-456"}),
        Role::Human,
        103,
    )
    .unwrap();

    let delivery = send(
        &mut store,
        json!({"op":"create_issue","product":"DIR","title":"Delivery","body":"Build it","planning_scope":"project","project_id":project_a["id"]}),
        Role::Agent,
        104,
    )
    .unwrap();
    let key = delivery["key"].as_str().unwrap();
    send(
        &mut store,
        json!({"op":"set_issue_milestone","key":key,"expected_version":1,"milestone_id":milestone["id"]}),
        Role::Agent,
        105,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"update_issue","key":key,"expected_version":2,"title":"Delivery","body":"Build it","acceptance":"Verified","owner":"owner","priority":"high"}),
        Role::Agent,
        106,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"ready","key":key,"expected_version":3}),
        Role::Human,
        107,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"claim","key":key,"expected_version":4}),
        Role::Agent,
        108,
    )
    .unwrap();
    let submitted = send(
        &mut store,
        json!({"op":"submit","key":key,"expected_version":5,"build_ref":"build","e2e":{"build_ref":"build","environment":"isolated Windows fixture","entrypoint":"fixture client","scenarios":"Exercise the full fixture workflow; expected and observed state transitions match","outcome":"passed","delivered_build_ref":"build","delivery_check":"Fixture service and client use the tested build"},"delivery_ref":"branch","summary":"Done","checks":"Passed","steps":[{"instruction":"Try it","expected":"Works"}]}),
        Role::Agent,
        109,
    )
    .unwrap();
    let pending = send(&mut store, json!({"op":"snapshot"}), Role::Agent, 110).unwrap();
    assert_eq!(pending["goal_progress"][0]["total"], 1);
    assert_eq!(pending["goal_progress"][0]["pending_verification"], 1);
    assert_eq!(pending["goal_progress"][0]["completed"], 0);
    assert_eq!(pending["milestone_progress"][0]["total"], 1);

    send(
        &mut store,
        json!({"op":"review","key":key,"expected_version":6,"run_id":submitted["current_run"],"outcome":"passed","results":[{"outcome":"passed"}]}),
        Role::Human,
        111,
    )
    .unwrap();
    let backlog = send(
        &mut store,
        json!({"op":"create_issue","product":"DIR","title":"Later","planning_scope":"project","project_id":project_b["id"]}),
        Role::Agent,
        112,
    )
    .unwrap();
    assert_eq!(
        send(
            &mut store,
            json!({"op":"set_issue_milestone","key":backlog["key"],"expected_version":1,"milestone_id":milestone["id"]}),
            Role::Agent,
            113,
        )
        .unwrap_err()
        .code,
        "invalid"
    );

    let snapshot = send(&mut store, json!({"op":"snapshot"}), Role::Agent, 114).unwrap();
    assert_eq!(snapshot["goal_progress"][0]["total"], 2);
    assert_eq!(snapshot["goal_progress"][0]["completed"], 1);
    assert_eq!(snapshot["goal_progress"][0]["backlog"], 1);
    assert_eq!(snapshot["goal_progress"][0]["completion_percent"], 50);
    assert_eq!(snapshot["milestone_progress"][0]["total"], 1);
    assert_eq!(snapshot["milestone_progress"][0]["completed"], 1);
    assert_eq!(snapshot["milestone_progress"][0]["completion_percent"], 100);
    let context = send(
        &mut store,
        json!({"op":"context","key":key}),
        Role::Agent,
        115,
    )
    .unwrap();
    assert_eq!(context["milestone"]["name"], "Foundation");
    assert_eq!(context["goals"][0]["name"], "Adopt Direct");
    assert_eq!(context["goal_progress"][0]["completed"], 1);

    let archive = store.export().unwrap();
    assert_eq!(archive.format, 14);
    assert_eq!(
        archive.goals[0].external_id.as_deref(),
        Some("initiative-123")
    );
    assert_eq!(archive.milestones[0].sort_order, 2);
    validate_archive(&archive).unwrap();
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(archive.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        serde_json::to_value(archive).unwrap()
    );

    send(
        &mut restored,
        json!({"op":"set_issue_project","key":key,"expected_version":7,"project_id":project_b["id"]}),
        Role::Human,
        116,
    )
    .unwrap();
    let moved = send(
        &mut restored,
        json!({"op":"context","key":key}),
        Role::Agent,
        117,
    )
    .unwrap();
    assert!(moved["issue"]["milestone_id"].is_null());
    let restored_archive = restored.export().unwrap();
    let child = restored_archive
        .issues
        .iter()
        .find(|issue| issue.parent.as_deref() == Some(key))
        .unwrap();
    assert!(child.milestone_id.is_none());
}

#[test]
fn planning_hierarchy_rejects_cross_scope_duplicates_and_untyped_cycles() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let project = send(
        &mut store,
        json!({"op":"create_project","product":"DIR","name":"Direct"}),
        Role::Human,
        100,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"create_product","key":"ALT","name":"Other"}),
        Role::Human,
        101,
    )
    .unwrap();
    let other = send(
        &mut store,
        json!({"op":"create_project","product":"ALT","name":"Other"}),
        Role::Human,
        102,
    )
    .unwrap();
    for projects in [json!([project["id"], project["id"]]), json!([other["id"]])] {
        assert_eq!(
            send(
                &mut store,
                json!({"op":"create_goal","product":"DIR","name":"Invalid","project_ids":projects}),
                Role::Human,
                103,
            )
            .unwrap_err()
            .code,
            "invalid"
        );
    }
    assert_eq!(
        send(
            &mut store,
            json!({"op":"create_goal","product":"DIR","name":"Missing provenance","external_source":"linear"}),
            Role::Human,
            104,
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    let goal = send(
        &mut store,
        json!({"op":"create_goal","product":"DIR","name":"Valid","project_ids":[project["id"]]}),
        Role::Human,
        105,
    )
    .unwrap();
    assert!(serde_json::from_value::<Request>(json!({
        "actor":"owner","request_id":"cycle","op":"create_goal","product":"DIR","name":"Nested",
        "parent_goal_id":goal["id"]
    }))
    .is_err());
    assert_eq!(
        send(
            &mut store,
            json!({"op":"create_milestone","project_id":goal["id"],"name":"Reverse hierarchy"}),
            Role::Human,
            106,
        )
        .unwrap_err()
        .code,
        "not_found"
    );

    let milestone = send(
        &mut store,
        json!({"op":"create_milestone","project_id":project["id"],"name":"One"}),
        Role::Human,
        107,
    )
    .unwrap();
    let archive = store.export().unwrap();
    let mut old = archive.clone();
    old.format = 6;
    assert_eq!(validate_archive(&old).unwrap_err().code, "invalid");
    let mut broken = archive;
    broken.milestones[0].project_id = other["id"].as_str().unwrap().into();
    broken.issues.push(Issue {
        intake: None,
        id: uuid::Uuid::new_v4().to_string(),
        key: "DIR-1".into(),
        product_id: broken.products[0].id.clone(),
        project_id: Some(project["id"].as_str().unwrap().into()),
        milestone_id: Some(milestone["id"].as_str().unwrap().into()),
        planning_scope: PlanningScope::Project,
        theoria_refs: vec![],
        labels: vec![],
        title: "Broken".into(),
        body: String::new(),
        acceptance: String::new(),
        owner: String::new(),
        priority: "medium".into(),
        status: Status::Backlog,
        version: 1,
        created_at: 1,
        updated_at: 1,
        claim: None,
        needs_fix: false,
        parent: None,
        verification_key: None,
        current_run: None,
        external: None,
        template: None,
    });
    assert_eq!(validate_archive(&broken).unwrap_err().code, "invalid");
}
