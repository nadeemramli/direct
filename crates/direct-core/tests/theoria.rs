use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(store: &mut Store, mut value: Value, role: Role, at: i64) -> Result<Value> {
    value["actor"] = json!("theoria-builder");
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    store.execute_at(serde_json::from_value(value).unwrap(), role, at)
}

fn sync(fingerprint: Option<&str>, content: Option<&str>, reason: Option<&str>) -> Value {
    json!({
        "op":"sync_theoria",
        "product":"DIR",
        "source_root":"C:/knowledge/development-operating-system",
        "catalog_version":1,
        "documents":[{
            "id":"dos-direct-workflow",
            "title":"Direct agent workflow",
            "description":"Claims, evidence, and owner review",
            "category":"workflow",
            "relative_path":"8. Direct Agent Workflow.md",
            "source_updated":"2026-09-29",
            "source_modified_at":100,
            "fingerprint":fingerprint,
            "content":content,
            "unavailable_reason":reason
        }]
    })
}

fn prepared(store: &mut Store) -> String {
    let issue = send(
        store,
        json!({"op":"create_issue","product":"DIR","title":"Use Theoria","body":"Open relevant guidance"}),
        Role::Agent,
        101,
    )
    .unwrap();
    let key = issue["key"].as_str().unwrap().to_string();
    send(
        store,
        json!({"op":"update_issue","key":key,"expected_version":1,"title":"Use Theoria","body":"Open relevant guidance","acceptance":"A proposal traces to its source","owner":"reviewer","priority":"high"}),
        Role::Agent,
        102,
    )
    .unwrap();
    send(
        store,
        json!({"op":"ready","key":key,"expected_version":2}),
        Role::Human,
        103,
    )
    .unwrap();
    key
}

#[test]
fn issue_guidance_and_method_proposals_are_traceable_without_becoming_decisions() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut store = Store::open(&path).unwrap();
    let first_fingerprint = "a".repeat(64);
    send(
        &mut store,
        sync(
            Some(&first_fingerprint),
            Some("# Workflow\n\nOwner review remains required."),
            None,
        ),
        Role::Agent,
        100,
    )
    .unwrap();
    let key = prepared(&mut store);
    let link = json!({"op":"link_theoria","key":key,"expected_version":3,"document_id":"dos-direct-workflow","playbook_version":null});
    assert_eq!(
        send(&mut store, link.clone(), Role::Agent, 104)
            .unwrap_err()
            .code,
        "claim_required"
    );
    send(
        &mut store,
        json!({"op":"claim","key":key,"expected_version":3}),
        Role::Agent,
        105,
    )
    .unwrap();
    let mut linked_request = link;
    linked_request["expected_version"] = json!(4);
    let linked = send(&mut store, linked_request, Role::Agent, 106).unwrap();
    assert_eq!(linked["theoria_refs"][0]["playbook_version"], Value::Null);
    assert_eq!(
        linked["theoria_refs"][0]["recorded_fingerprint"],
        first_fingerprint
    );
    send(
        &mut store,
        json!({
            "op":"create_method_finding",
            "key":key,
            "expected_version":5,
            "classification":"both",
            "observation":"The issue did not expose the workflow used.",
            "hypothesis":"A narrow guidance link reduces context reconstruction.",
            "proposal":"Show the linked workflow and its recorded fingerprint in the issue.",
            "evidence":[{"kind":"check","reference":"cargo:test-theoria","summary":"Persistence and authorization regression"}]
        }),
        Role::Agent,
        107,
    )
    .unwrap();
    let context = send(
        &mut store,
        json!({"op":"context","key":key}),
        Role::Agent,
        108,
    )
    .unwrap();
    assert_eq!(context["method_findings"][0]["classification"], "both");
    assert_eq!(
        context["method_findings"][0]["evidence"][0]["kind"],
        "issue"
    );
    assert!(context["method_findings"][0].get("accepted_at").is_none());

    let second_fingerprint = "b".repeat(64);
    send(
        &mut store,
        sync(
            Some(&second_fingerprint),
            Some("# Workflow\n\nRevised source."),
            None,
        ),
        Role::Agent,
        109,
    )
    .unwrap();
    let snapshot = send(&mut store, json!({"op":"snapshot"}), Role::Agent, 110).unwrap();
    assert_eq!(
        snapshot["issues"][0]["theoria_refs"][0]["recorded_fingerprint"],
        first_fingerprint
    );
    assert_eq!(
        snapshot["theoria_documents"][0]["fingerprint"],
        second_fingerprint
    );

    send(
        &mut store,
        sync(None, None, Some("source unavailable")),
        Role::Agent,
        111,
    )
    .unwrap();
    let unavailable = send(&mut store, json!({"op":"snapshot"}), Role::Agent, 112).unwrap();
    assert_eq!(
        unavailable["theoria_documents"][0]["availability"],
        "unavailable"
    );
    assert_eq!(
        unavailable["theoria_documents"][0]["fingerprint"],
        second_fingerprint
    );
    assert_eq!(
        unavailable["theoria_documents"][0]["content"],
        "# Workflow\n\nRevised source."
    );
    assert_eq!(
        unavailable["theoria_documents"][0]["source_updated"],
        "2026-09-29"
    );
    assert_eq!(
        unavailable["theoria_documents"][0]["source_modified_at"],
        100
    );

    let archive = store.export().unwrap();
    assert_eq!(archive.format, 6);
    validate_archive(&archive).unwrap();
    drop(store);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(
        serde_json::to_value(reopened.export().unwrap()).unwrap(),
        serde_json::to_value(&archive).unwrap()
    );
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(archive.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        serde_json::to_value(archive).unwrap()
    );
}

#[test]
fn malformed_or_cross_product_theoria_records_are_rejected() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    assert_eq!(
        send(
            &mut store,
            sync(Some("short"), Some("content"), None),
            Role::Agent,
            100,
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    let fingerprint = "c".repeat(64);
    send(
        &mut store,
        sync(Some(&fingerprint), Some("content"), None),
        Role::Agent,
        101,
    )
    .unwrap();
    let mut broken = store.export().unwrap();
    broken.issues.push(Issue {
        id: uuid::Uuid::new_v4().to_string(),
        key: "DIR-1".into(),
        product_id: broken.products[0].id.clone(),
        project_id: None,
        planning_scope: PlanningScope::Inbox,
        theoria_refs: vec![TheoriaReference {
            document_id: "missing".into(),
            recorded_fingerprint: None,
            playbook_version: None,
            linked_by: "test".into(),
            linked_at: 1,
        }],
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
    });
    assert_eq!(validate_archive(&broken).unwrap_err().code, "invalid");
}
