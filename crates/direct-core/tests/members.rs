//! DIR-75: agent members and explicit issue role/model assignment.
use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(s: &mut Store, mut value: Value, role: Role, at: i64) -> Result<Value> {
    if value.get("actor").is_none() {
        value["actor"] = json!(if role == Role::Human {
            "owner"
        } else {
            "worker"
        });
    }
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    s.execute_at(serde_json::from_value(value).unwrap(), role, at)
}

const SKILL: &str = "---\nname: s\ndescription: d\n---\n";

/// DIR and ALT products, an active claude-code role and a draft codex-only role.
fn world(s: &mut Store) -> (String, String, Value, Value) {
    let alt = send(
        s,
        json!({"op":"create_product","key":"ALT","name":"Alt"}),
        Role::Human,
        1,
    )
    .unwrap();
    let snap = send(s, json!({"op":"snapshot"}), Role::Agent, 1).unwrap();
    let dir = snap["products"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == "DIR")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    send(s, json!({"op":"sync_theoria","product":"DIR","source_root":"C:/dos","catalog_version":1,
        "documents":[{"id":"dos","title":"t","category":"workflow","relative_path":"8.md","fingerprint":"a".repeat(64),"content":"x"}]}), Role::Agent, 2).unwrap();
    let skill = send(s, json!({"op":"register_skill_package","name":"s","description":"d","trigger":"t","origin":"local","license":"l","files":[{"path":"SKILL.md","content":SKILL}]}), Role::Agent, 3).unwrap();
    let role = |runtimes: Value| {
        json!({"op":"register_agent_role","key":"product-planner","name":"Planner","responsibilities":["r"],"inputs":["i"],"outputs":["o"],
        "skills":[skill["id"]],"runtime_compatibility":runtimes,"guidance":[{"document_id":"dos","mandatory":true}],"owner_direction":"bounded"})
    };
    let active = send(s, role(json!(["claude-code"])), Role::Agent, 4).unwrap();
    send(
        s,
        json!({"op":"activate_agent_role","id":active["id"],"note":"ok"}),
        Role::Human,
        5,
    )
    .unwrap();
    let draft = send(s, role(json!(["codex"])), Role::Agent, 6).unwrap();
    (dir, alt["id"].as_str().unwrap().to_string(), active, draft)
}

fn ready(s: &mut Store, product: &str) -> String {
    let i = send(
        s,
        json!({"op":"create_issue","product":product,"title":"Work","planning_scope":"inbox"}),
        Role::Agent,
        10,
    )
    .unwrap();
    let key = i["key"].as_str().unwrap().to_string();
    send(s, json!({"op":"update_issue","key":key,"expected_version":1,"title":"Work","body":"b","acceptance":"a","owner":"Nadeem","priority":"medium"}), Role::Agent, 11).unwrap();
    send(
        s,
        json!({"op":"ready","key":key,"expected_version":2}),
        Role::Human,
        12,
    )
    .unwrap();
    key
}

fn create(name: &str, runtime: &str, products: Vec<&str>) -> Value {
    json!({"op":"create_agent_member","name":name,"runtime":runtime,"connection_ref":"local claude-code on Windows",
        "product_ids":products,"default_role_key":"product-planner","default_model":"claude-opus-5-5"})
}

fn assign(key: &str, member: &Value, role: &Value, model: &str, expected: Option<u64>) -> Value {
    json!({"op":"assign_issue_agent","key":key,"expected_assignment_version":expected,"member_id":member["id"],"role_id":role["id"],"requested_model":model})
}

#[test]
fn members_are_owner_configured_and_capability_comes_from_checks() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let (dir_id, _, _, _) = world(&mut s);
    assert_eq!(
        send(
            &mut s,
            create("Claude", "claude-code", vec![&dir_id]),
            Role::Agent,
            20
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    for (field, value) in [
        ("runtime", json!("gpt-shell")),
        ("connection_ref", json!("https://user:pw@host")),
        ("connection_ref", json!("token=abc")),
        ("product_ids", json!([])),
        ("product_ids", json!(["missing"])),
        ("default_role_key", json!("nope")),
    ] {
        let mut bad = create("Claude", "claude-code", vec![&dir_id]);
        bad[field] = value;
        assert!(
            send(&mut s, bad, Role::Human, 20).is_err(),
            "{field} accepted"
        );
    }
    let m = send(
        &mut s,
        create("Claude", "claude-code", vec![&dir_id]),
        Role::Human,
        21,
    )
    .unwrap();
    assert_eq!(m["enabled"], true);
    assert!(m["capability"].is_null());
    assert_eq!(
        send(
            &mut s,
            create("claude", "codex", vec![&dir_id]),
            Role::Human,
            21
        )
        .unwrap_err()
        .code,
        "conflict"
    );

    let check = |v: u64| json!({"op":"record_member_capability","id":m["id"],"expected_version":v,"harness_version":"2.1.289 (Claude Code)","verified_models":["claude-opus-5-5"],"evidence":"claude-opus-5-5=sess-1"});
    assert_eq!(
        send(&mut s, check(9), Role::Agent, 22).unwrap_err().code,
        "conflict"
    );
    let checked = send(&mut s, check(1), Role::Agent, 22).unwrap();
    assert_eq!(
        checked["capability"]["verified_models"][0],
        "claude-opus-5-5"
    );
    assert_eq!(checked["version"], 2);
    // A stale update conflicts and keeps the stored configuration.
    let update = |v: u64, enabled: bool| {
        json!({"op":"update_agent_member","id":m["id"],"expected_version":v,"name":"Claude","enabled":enabled,
        "connection_ref":"local claude-code on Windows","product_ids":[dir_id],"default_role_key":"product-planner","default_model":"claude-opus-5-5"})
    };
    assert_eq!(
        send(&mut s, update(1, false), Role::Human, 23)
            .unwrap_err()
            .code,
        "conflict"
    );
    let disabled = send(&mut s, update(2, false), Role::Human, 24).unwrap();
    assert_eq!(disabled["enabled"], false);
    assert_eq!(
        disabled["capability"]["verified_models"][0],
        "claude-opus-5-5"
    );
}

#[test]
fn assignment_is_validated_by_the_service_and_never_touches_workflow_state() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let (dir_id, alt_id, active, draft) = world(&mut s);
    let old = ready(&mut s, "DIR");
    let key = ready(&mut s, "DIR");
    let alt = ready(&mut s, "ALT");
    let m = send(
        &mut s,
        create("Claude", "claude-code", vec![&dir_id]),
        Role::Human,
        20,
    )
    .unwrap();
    let codex = send(
        &mut s,
        create("Codex", "codex", vec![&dir_id, &alt_id]),
        Role::Human,
        20,
    )
    .unwrap();

    // Unverified capability is explicit and blocks executable assignment.
    let unverified = send(
        &mut s,
        assign(&key, &m, &active, "claude-opus-5-5", None),
        Role::Human,
        21,
    )
    .unwrap_err();
    assert!(unverified.message.contains("unverified"));
    let m = send(&mut s, json!({"op":"record_member_capability","id":m["id"],"expected_version":1,"harness_version":"2.1.289","verified_models":["claude-opus-5-5"],"evidence":"e"}), Role::Agent, 22).unwrap();

    let before =
        send(&mut s, json!({"op":"context","key":key}), Role::Agent, 23).unwrap()["issue"].clone();
    assert_eq!(
        send(
            &mut s,
            assign(&key, &m, &active, "claude-opus-5-5", None),
            Role::Agent,
            24
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    for (bad, why) in [
        (
            assign(&key, &m, &active, "claude-sonnet-5-5", None),
            "unverified",
        ),
        (
            assign(&key, &m, &draft, "claude-opus-5-5", None),
            "not active",
        ),
        (
            assign(&key, &codex, &active, "claude-opus-5-5", None),
            "not written for codex",
        ),
        (
            assign(&alt, &m, &active, "claude-opus-5-5", None),
            "not permitted",
        ),
        (
            assign(
                &key,
                &json!({"id":"missing"}),
                &active,
                "claude-opus-5-5",
                None,
            ),
            "Unknown agent member",
        ),
        (
            assign(&key, &m, &json!({"id":"missing"}), "claude-opus-5-5", None),
            "Unknown agent role",
        ),
    ] {
        let e = send(&mut s, bad, Role::Human, 25).unwrap_err();
        assert!(e.message.contains(why), "{why}: {}", e.message);
    }
    let a = send(
        &mut s,
        assign(&key, &m, &active, "claude-opus-5-5", None),
        Role::Human,
        26,
    )
    .unwrap();
    assert_eq!(a["status"], "active");
    // Readiness, claim, owner and the issue version are unchanged.
    let after = send(&mut s, json!({"op":"context","key":key}), Role::Agent, 27).unwrap();
    assert_eq!(after["issue"], before);
    assert_eq!(after["assignment"]["member"]["name"], "Claude");
    assert_eq!(after["assignment"]["role"]["revision"], 1);
    assert_eq!(after["assignment"]["requested_model"], "claude-opus-5-5");
    assert!(after["assignment"]["actual_model"].is_null());
    assert!(
        send(&mut s, json!({"op":"context","key":old}), Role::Agent, 27).unwrap()["assignment"]
            .is_null()
    );
    assert_eq!(
        send(&mut s, json!({"op":"context","key":old}), Role::Agent, 27).unwrap()["issue"]["owner"],
        "Nadeem"
    );

    // Concurrent edits: a second assignment needs the current version.
    assert_eq!(
        send(
            &mut s,
            assign(&key, &m, &active, "claude-opus-5-5", None),
            Role::Human,
            28
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    assert_eq!(
        send(
            &mut s,
            assign(&key, &m, &active, "claude-opus-5-5", Some(9)),
            Role::Human,
            28
        )
        .unwrap_err()
        .code,
        "conflict"
    );

    // An active writer requires explicit reconciliation; the claim is untouched.
    let v = after["issue"]["version"].as_u64().unwrap();
    send(&mut s, json!({"op":"claim","key":key,"expected_version":v,"lease_seconds":3600,"actor":"codex-worker"}), Role::Agent, 29).unwrap();
    let busy = send(
        &mut s,
        assign(&key, &m, &active, "claude-opus-5-5", Some(1)),
        Role::Human,
        30,
    )
    .unwrap_err();
    assert!(busy.message.contains("codex-worker holds an active claim"));
    let mut reconciled = assign(&key, &m, &active, "claude-opus-5-5", Some(1));
    reconciled["reconcile_active_writer"] =
        json!("codex-worker agreed to hand over after its current commit");
    let a2 = send(&mut s, reconciled, Role::Human, 31).unwrap();
    assert!(a2["reconciliation"]
        .as_str()
        .unwrap()
        .starts_with("Active writer codex-worker:"));
    let ctx = send(&mut s, json!({"op":"context","key":key}), Role::Agent, 32).unwrap();
    assert_eq!(ctx["issue"]["claim"]["actor"], "codex-worker");
    assert_eq!(ctx["issue"]["status"], "doing");

    // Only the claim holder records the actual session; requested and actual stay separate.
    let session = |actor: &str| json!({"op":"record_assignment_session","id":a2["id"],"session_id":"sess-9","model":"claude-sonnet-5-5","actor":actor});
    assert_eq!(
        send(&mut s, session("worker"), Role::Agent, 33)
            .unwrap_err()
            .code,
        "claim_required"
    );
    send(&mut s, session("codex-worker"), Role::Agent, 33).unwrap();
    let ctx = send(&mut s, json!({"op":"context","key":key}), Role::Agent, 34).unwrap();
    assert_eq!(ctx["assignment"]["requested_model"], "claude-opus-5-5");
    assert_eq!(ctx["assignment"]["actual_model"], "claude-sonnet-5-5");

    // Disabled members cannot receive new assignments; clearing needs the version.
    send(&mut s, json!({"op":"update_agent_member","id":m["id"],"expected_version":2,"name":"Claude","enabled":false,"connection_ref":"local","product_ids":[dir_id]}), Role::Human, 35).unwrap();
    let other = ready(&mut s, "DIR");
    assert!(send(
        &mut s,
        assign(&other, &m, &active, "claude-opus-5-5", None),
        Role::Human,
        36
    )
    .unwrap_err()
    .message
    .contains("disabled"));
    let current = ctx["assignment"]["assignment"]["version"].as_u64().unwrap();
    assert_eq!(send(&mut s, json!({"op":"clear_issue_assignment","id":a2["id"],"expected_version":current + 5,"reason":"x"}), Role::Human, 37).unwrap_err().code, "conflict");
    send(&mut s, json!({"op":"clear_issue_assignment","id":a2["id"],"expected_version":current,"reason":"Paused"}), Role::Human, 37).unwrap();
    assert!(
        send(&mut s, json!({"op":"context","key":key}), Role::Agent, 38).unwrap()["assignment"]
            .is_null()
    );

    // Restart and archive recovery keep members and assignment history.
    drop(s);
    let s = Store::open(&path).unwrap();
    let archive = s.export().unwrap();
    assert_eq!(archive.format, 22);
    assert_eq!(archive.agent_members.len(), 2);
    assert_eq!(archive.issue_assignments.len(), 2);
    validate_archive(&archive).unwrap();
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(archive.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        serde_json::to_value(&archive).unwrap()
    );
    let mut old_format = archive.clone();
    old_format.format = 20;
    assert!(validate_archive(&old_format).is_err());
    let mut dangling = archive.clone();
    dangling.issue_assignments[0].member_id = "ghost".into();
    assert!(validate_archive(&dangling).is_err());
    let mut doubled = archive;
    for a in &mut doubled.issue_assignments {
        a.status = AssignmentStatus::Active;
    }
    assert!(validate_archive(&doubled)
        .unwrap_err()
        .message
        .contains("two active"));
}
