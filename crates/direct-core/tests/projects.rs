use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(s: &mut Store, mut value: Value, role: Role) -> Result<Value> {
    value["actor"] = json!("project-builder");
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    s.execute_at(serde_json::from_value(value).unwrap(), role, 100)
}

#[test]
fn project_scope_versions_replays_and_context_survive_restore() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let create = json!({"op":"create_project","product":"DIR","name":"Pilot","description":"Ten real deliveries","request_id":"project-create"});
    assert_eq!(
        send(&mut s, create.clone(), Role::Agent).unwrap_err().code,
        "forbidden"
    );
    let p = send(&mut s, create.clone(), Role::Human).unwrap();
    assert_eq!(send(&mut s, create, Role::Human).unwrap(), p);
    assert_eq!(
        send(
            &mut s,
            json!({"op":"create_project","product":"DIR","name":" pilot "}),
            Role::Human
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    let update = json!({"op":"update_project","id":p["id"],"expected_version":1,"name":"Direct pilot","description":"Deliver with proof"});
    send(&mut s, update.clone(), Role::Human).unwrap();
    assert_eq!(
        send(&mut s, update, Role::Human).unwrap_err().code,
        "conflict"
    );
    let i = send(
        &mut s,
        json!({"op":"create_issue","product":"DIR","title":"Projects"}),
        Role::Agent,
    )
    .unwrap();
    let assign = json!({"op":"set_issue_project","key":i["key"],"expected_version":1,"project_id":p["id"],"request_id":"assign"});
    let assigned = send(&mut s, assign.clone(), Role::Agent).unwrap();
    assert_eq!(send(&mut s, assign, Role::Agent).unwrap(), assigned);
    send(
        &mut s,
        json!({"op":"create_product","key":"ALT","name":"Other"}),
        Role::Human,
    )
    .unwrap();
    let other = send(
        &mut s,
        json!({"op":"create_project","product":"ALT","name":"Other pilot"}),
        Role::Human,
    )
    .unwrap();
    let cross = json!({"op":"set_issue_project","key":i["key"],"expected_version":2,"project_id":other["id"]});
    assert_eq!(
        send(&mut s, cross, Role::Agent).unwrap_err().code,
        "invalid"
    );
    let ctx = send(&mut s, json!({"op":"context","key":i["key"]}), Role::Agent).unwrap();
    assert_eq!(ctx["project"]["name"], "Direct pilot");
    assert_eq!(ctx["issue"]["version"], 2);
    let before = serde_json::to_value(s.export().unwrap()).unwrap();
    drop(s);
    let s = Store::open(&dir.path().join("db")).unwrap();
    assert_eq!(serde_json::to_value(s.export().unwrap()).unwrap(), before);
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(s.export().unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        before
    );
    let mut broken = s.export().unwrap();
    broken.issues[0].project_id = Some("missing".into());
    assert!(restored.restore(broken).is_err());
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        before
    );
    send(
        &mut restored,
        json!({"op":"set_issue_project","key":i["key"],"expected_version":2,"project_id":null}),
        Role::Agent,
    )
    .unwrap();
    let ctx = send(
        &mut restored,
        json!({"op":"context","key":i["key"]}),
        Role::Agent,
    )
    .unwrap();
    assert!(ctx["project"].is_null());
}

#[test]
fn grouping_respects_claims_and_carries_verification_children() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let p = send(
        &mut s,
        json!({"op":"create_project","product":"DIR","name":"Pilot"}),
        Role::Human,
    )
    .unwrap();
    let i = send(
        &mut s,
        json!({"op":"create_issue","product":"DIR","title":"Build","body":"Work"}),
        Role::Agent,
    )
    .unwrap();
    let key = i["key"].as_str().unwrap();
    send(&mut s, json!({"op":"update_issue","key":key,"expected_version":1,"title":"Build","body":"Work","acceptance":"Proof","owner":"reviewer","priority":"high"}), Role::Agent).unwrap();
    send(
        &mut s,
        json!({"op":"ready","key":key,"expected_version":2}),
        Role::Human,
    )
    .unwrap();
    send(
        &mut s,
        json!({"op":"claim","key":key,"expected_version":3}),
        Role::Agent,
    )
    .unwrap();
    let assignment =
        json!({"op":"set_issue_project","key":key,"expected_version":4,"project_id":p["id"]});
    let mut stranger = assignment.clone();
    stranger["actor"] = json!("another-agent");
    stranger["request_id"] = json!("stranger");
    assert_eq!(
        s.execute_at(serde_json::from_value(stranger).unwrap(), Role::Agent, 101)
            .unwrap_err()
            .code,
        "claim_required"
    );
    send(&mut s, assignment, Role::Agent).unwrap();
    let submitted = send(&mut s, json!({"op":"submit","key":key,"expected_version":5,"build_ref":"abc","delivery_ref":"branch","summary":"Done","checks":"Passed","steps":[{"instruction":"Try it","expected":"Works"}]}), Role::Agent).unwrap();
    let archive = s.export().unwrap();
    validate_archive(&archive).unwrap();
    assert!(archive
        .issues
        .iter()
        .all(|i| i.project_id.as_deref() == p["id"].as_str()));
    let remove = json!({"op":"set_issue_project","key":key,"expected_version":6,"project_id":null});
    assert_eq!(
        send(&mut s, remove.clone(), Role::Agent).unwrap_err().code,
        "forbidden"
    );
    let result = send(&mut s, remove, Role::Human).unwrap();
    assert_eq!(result["status"], "verify");
    assert_eq!(result["current_run"], submitted["current_run"]);
    assert!(s
        .export()
        .unwrap()
        .issues
        .iter()
        .all(|i| i.project_id.is_none()));
    validate_archive(&s.export().unwrap()).unwrap();
}

#[test]
fn project_first_intake_metadata_and_verified_progress_are_enforced() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let project = send(
        &mut s,
        json!({"op":"create_project","product":"DIR","name":"Release planning","description":"Ship a bounded change","priority":"urgent","sort_order":7}),
        Role::Human,
    )
    .unwrap();
    assert_eq!(project["status"], "planned");
    assert_eq!(project["priority"], "urgent");
    assert_eq!(project["sort_order"], 7);
    let project = send(
        &mut s,
        json!({"op":"update_project","id":project["id"],"expected_version":1,"name":"Release planning","description":"Ship a bounded change","status":"active","priority":"high","sort_order":3}),
        Role::Human,
    )
    .unwrap();
    assert_eq!(project["status"], "active");
    assert_eq!(project["priority"], "high");
    assert_eq!(project["sort_order"], 3);

    let issue = send(
        &mut s,
        json!({"op":"create_issue","product":"DIR","title":"Project delivery","body":"Deliver it","planning_scope":"project","project_id":project["id"]}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(issue["project_id"], project["id"]);
    assert_eq!(issue["planning_scope"], "project");
    let key = issue["key"].as_str().unwrap();
    send(
        &mut s,
        json!({"op":"update_issue","key":key,"expected_version":1,"title":"Project delivery","body":"Deliver it","acceptance":"Verified","owner":"owner","priority":"high"}),
        Role::Agent,
    )
    .unwrap();
    send(
        &mut s,
        json!({"op":"ready","key":key,"expected_version":2}),
        Role::Human,
    )
    .unwrap();
    send(
        &mut s,
        json!({"op":"claim","key":key,"expected_version":3}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(
        send(
            &mut s,
            json!({"op":"set_issue_project","key":key,"expected_version":4,"project_id":null}),
            Role::Agent,
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    let submitted = send(
        &mut s,
        json!({"op":"submit","key":key,"expected_version":4,"build_ref":"commit:abc","delivery_ref":"branch","summary":"Done","checks":"Passed","steps":[{"instruction":"Open it","expected":"It works"}]}),
        Role::Agent,
    )
    .unwrap();
    send(
        &mut s,
        json!({"op":"review","key":key,"expected_version":5,"run_id":submitted["current_run"],"outcome":"passed","results":[{"outcome":"passed","note":"Observed"}],"note":"Accepted"}),
        Role::Human,
    )
    .unwrap();
    let context = send(&mut s, json!({"op":"context","key":key}), Role::Agent).unwrap();
    assert_eq!(context["project"]["status"], "active");
    assert_eq!(context["project_progress"]["total"], 1);
    assert_eq!(context["project_progress"]["completed"], 1);
    assert_eq!(context["project_progress"]["completion_percent"], 100);
    let snapshot = send(&mut s, json!({"op":"snapshot"}), Role::Agent).unwrap();
    assert_eq!(snapshot["project_progress"][0]["total"], 1);

    let ungrouped = send(
        &mut s,
        json!({"op":"create_issue","product":"DIR","title":"Needs a project","body":"Feature work","planning_scope":"project"}),
        Role::Agent,
    )
    .unwrap();
    let ungrouped_key = ungrouped["key"].as_str().unwrap();
    send(
        &mut s,
        json!({"op":"update_issue","key":ungrouped_key,"expected_version":1,"title":"Needs a project","body":"Feature work","acceptance":"Scoped","owner":"owner","priority":"medium"}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(
        send(
            &mut s,
            json!({"op":"ready","key":ungrouped_key,"expected_version":2}),
            Role::Human,
        )
        .unwrap_err()
        .code,
        "invalid"
    );

    let other = send(
        &mut s,
        json!({"op":"create_product","key":"ALT","name":"Other"}),
        Role::Human,
    )
    .unwrap();
    assert_eq!(
        send(
            &mut s,
            json!({"op":"create_issue","product":"ALT","title":"Wrong project","planning_scope":"project","project_id":project["id"]}),
            Role::Agent,
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    assert_eq!(other["key"], "ALT");
    validate_archive(&s.export().unwrap()).unwrap();
}

#[test]
fn legacy_database_and_archive_upgrade_without_losing_identity_or_replays() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let create = json!({"op":"create_issue","product":"DIR","title":"Legacy issue","request_id":"legacy-create"});
    let created = send(&mut s, create.clone(), Role::Agent).unwrap();
    let mut legacy = serde_json::to_value(s.export().unwrap()).unwrap();
    legacy["format"] = json!(1);
    legacy.as_object_mut().unwrap().remove("projects");
    legacy["issues"][0]
        .as_object_mut()
        .unwrap()
        .remove("project_id");
    let mut old_response = created;
    old_response.as_object_mut().unwrap().remove("project_id");
    legacy["requests"][0]["response"] = json!(serde_json::to_string(&old_response).unwrap());
    drop(s);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "DROP TABLE projects; DROP TABLE issue_links; DROP TABLE goals; DROP TABLE milestones; UPDATE meta SET value='1' WHERE key='schema';",
    )
        .unwrap();
    conn.execute(
        "UPDATE issues SET data=?1",
        [serde_json::to_string(&legacy["issues"][0]).unwrap()],
    )
    .unwrap();
    conn.execute(
        "UPDATE requests SET response=?1",
        [serde_json::to_string(&old_response).unwrap()],
    )
    .unwrap();
    drop(conn);
    let mut upgraded = Store::open(&path).unwrap();
    let upgraded_archive = serde_json::to_value(upgraded.export().unwrap()).unwrap();
    assert_eq!(upgraded_archive["workspace_id"], legacy["workspace_id"]);
    assert_eq!(
        upgraded_archive["issues"][0]["id"],
        legacy["issues"][0]["id"]
    );
    assert_eq!(
        send(&mut upgraded, create.clone(), Role::Agent).unwrap(),
        old_response
    );
    let conn = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT value FROM meta WHERE key='schema'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "8"
    );
    let mut restored = Store::open(&dir.path().join("restore-v1")).unwrap();
    restored
        .restore(serde_json::from_value(legacy).unwrap())
        .unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        upgraded_archive
    );
    assert_eq!(
        send(&mut restored, create, Role::Agent).unwrap(),
        old_response
    );
}

#[test]
fn legacy_project_links_upgrade_to_project_planning_scope() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut store = Store::open(&path).unwrap();
    let project = send(
        &mut store,
        json!({"op":"create_project","product":"DIR","name":"Legacy project"}),
        Role::Human,
    )
    .unwrap();
    let issue = send(
        &mut store,
        json!({"op":"create_issue","product":"DIR","title":"Legacy project issue"}),
        Role::Agent,
    )
    .unwrap();
    send(
        &mut store,
        json!({"op":"set_issue_project","key":issue["key"],"expected_version":1,"project_id":project["id"]}),
        Role::Agent,
    )
    .unwrap();

    let mut legacy = serde_json::to_value(store.export().unwrap()).unwrap();
    legacy["format"] = json!(4);
    for field in ["status", "priority", "sort_order"] {
        legacy["projects"][0].as_object_mut().unwrap().remove(field);
    }
    legacy["issues"][0]
        .as_object_mut()
        .unwrap()
        .remove("planning_scope");
    drop(store);

    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute("UPDATE meta SET value='4' WHERE key='schema'", [])
        .unwrap();
    conn.execute(
        "UPDATE projects SET data=?1",
        [serde_json::to_string(&legacy["projects"][0]).unwrap()],
    )
    .unwrap();
    conn.execute(
        "UPDATE issues SET data=?1",
        [serde_json::to_string(&legacy["issues"][0]).unwrap()],
    )
    .unwrap();
    drop(conn);

    let upgraded = Store::open(&path).unwrap().export().unwrap();
    assert_eq!(upgraded.issues[0].planning_scope, PlanningScope::Project);
    assert_eq!(upgraded.projects[0].status, ProjectStatus::Active);
    assert_eq!(upgraded.projects[0].priority, "medium");
    assert_eq!(upgraded.projects[0].sort_order, 0);

    let mut restored = Store::open(&dir.path().join("restored-v4")).unwrap();
    restored
        .restore(serde_json::from_value(legacy).unwrap())
        .unwrap();
    let restored = restored.export().unwrap();
    assert_eq!(restored.issues[0].planning_scope, PlanningScope::Project);
    assert_eq!(restored.projects[0].status, ProjectStatus::Active);
}
