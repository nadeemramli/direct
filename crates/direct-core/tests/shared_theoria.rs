//! DIR-57: owner-shared Theoria guidance linkable across products, with
//! product-specific guidance still scoped and existing pins never rewritten.
use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(store: &mut Store, mut value: Value, role: Role, at: i64) -> Result<Value> {
    value["actor"] = json!(if role == Role::Human {
        "owner"
    } else {
        "agent-1"
    });
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    store.execute_at(serde_json::from_value(value).unwrap(), role, at)
}

fn doc(id: &str, fingerprint: &str) -> Value {
    json!({"id":id,"title":id,"category":"workflow","relative_path":format!("{id}.md"),
        "fingerprint":fingerprint,"content":format!("# {id}")})
}

fn sync(store: &mut Store, product: &str, docs: Vec<Value>, at: i64) {
    send(store, json!({"op":"sync_theoria","product":product,"source_root":"C:/dos","catalog_version":1,"documents":docs}), Role::Agent, at).unwrap();
}

/// A Ready issue claimed by agent-1, so agent linking is allowed.
fn claimed(store: &mut Store, product: &str) -> (String, u64) {
    let i = send(store, json!({"op":"create_issue","product":product,"title":"Use guidance","planning_scope":"inbox"}), Role::Agent, 10).unwrap();
    let key = i["key"].as_str().unwrap().to_string();
    send(store, json!({"op":"update_issue","key":key,"expected_version":1,"title":"Use guidance","body":"b","acceptance":"a","owner":"Owner","priority":"medium"}), Role::Agent, 11).unwrap();
    send(
        store,
        json!({"op":"ready","key":key,"expected_version":2}),
        Role::Human,
        12,
    )
    .unwrap();
    send(
        store,
        json!({"op":"claim","key":key,"expected_version":3,"lease_seconds":86400}),
        Role::Agent,
        13,
    )
    .unwrap();
    (key, 4)
}

fn link(store: &mut Store, key: &str, v: u64, document: &str) -> Result<Value> {
    send(
        store,
        json!({"op":"link_theoria","key":key,"expected_version":v,"document_id":document,"playbook_version":"v5"}),
        Role::Agent,
        20,
    )
}

fn share(store: &mut Store, document: &str, shared: bool, role: Role) -> Result<Value> {
    send(
        store,
        json!({"op":"set_theoria_sharing","document_id":document,"shared":shared}),
        role,
        15,
    )
}

#[test]
fn shared_guidance_links_across_products_while_scoped_guidance_stays_scoped() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut s = Store::open(&path).unwrap();
    send(
        &mut s,
        json!({"op":"create_product","key":"INS","name":"Inspo"}),
        Role::Human,
        1,
    )
    .unwrap();
    let (a, b) = ("a".repeat(64), "b".repeat(64));
    sync(
        &mut s,
        "DIR",
        vec![doc("dos-direct-workflow", &a), doc("dir-only", &a)],
        2,
    );
    let (dir_key, dv) = claimed(&mut s, "DIR");
    let (ins_key, iv) = claimed(&mut s, "INS");

    // Before sharing, a cross-product link is refused without any change.
    let refused = link(&mut s, &ins_key, iv, "dos-direct-workflow").unwrap_err();
    assert_eq!(refused.code, "invalid");
    assert!(refused.message.contains("shared with the workspace"));

    // Only the owner shares; retries of the same state are no-ops.
    assert_eq!(
        share(&mut s, "dos-direct-workflow", true, Role::Agent)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        share(&mut s, "dos-direct-workflow", true, Role::Human).unwrap()["shared"],
        true
    );
    assert_eq!(
        share(&mut s, "dos-direct-workflow", true, Role::Human).unwrap()["shared"],
        true
    );
    assert_eq!(
        share(&mut s, "missing", true, Role::Human)
            .unwrap_err()
            .code,
        "not_found"
    );

    // Both products can link the shared document; the unshared one stays scoped.
    let own = link(&mut s, &dir_key, dv, "dos-direct-workflow").unwrap();
    assert!(own["theoria_refs"][0].get("shared").is_none());
    let cross = link(&mut s, &ins_key, iv, "dos-direct-workflow").unwrap();
    assert_eq!(cross["theoria_refs"][0]["shared"], true);
    assert_eq!(cross["theoria_refs"][0]["recorded_fingerprint"], a);
    assert_eq!(cross["theoria_refs"][0]["playbook_version"], "v5");
    assert_eq!(
        link(&mut s, &ins_key, iv + 1, "dir-only").unwrap_err().code,
        "invalid"
    );

    // The source product's re-sync updates content and keeps the sharing flag;
    // the earlier pin keeps its recorded fingerprint, so staleness is visible.
    sync(
        &mut s,
        "DIR",
        vec![doc("dos-direct-workflow", &b), doc("dir-only", &a)],
        30,
    );
    let snapshot = send(&mut s, json!({"op":"snapshot"}), Role::Agent, 31).unwrap();
    let current = snapshot["theoria_documents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == "dos-direct-workflow")
        .unwrap()
        .clone();
    assert_eq!(current["shared"], true);
    assert_eq!(current["fingerprint"], b);
    let pinned = send(
        &mut s,
        json!({"op":"context","key":ins_key}),
        Role::Agent,
        32,
    )
    .unwrap();
    assert_eq!(
        pinned["issue"]["theoria_refs"][0]["recorded_fingerprint"],
        a
    );

    // Another product cannot take over the shared document by syncing its ID.
    let takeover = send(&mut s, json!({"op":"sync_theoria","product":"INS","source_root":"C:/dos","catalog_version":1,"documents":[doc("dos-direct-workflow", &b)]}), Role::Agent, 33).unwrap_err();
    assert_eq!(takeover.code, "conflict");

    // Unsharing blocks new cross-product links but never rewrites existing pins.
    assert_eq!(
        share(&mut s, "dos-direct-workflow", false, Role::Human)
            .unwrap()
            .get("shared"),
        None
    );
    let (late_key, lv) = claimed(&mut s, "INS");
    assert_eq!(
        link(&mut s, &late_key, lv, "dos-direct-workflow")
            .unwrap_err()
            .code,
        "invalid"
    );
    let kept = send(
        &mut s,
        json!({"op":"context","key":ins_key}),
        Role::Agent,
        40,
    )
    .unwrap();
    assert_eq!(
        kept["issue"]["theoria_refs"][0]["document_id"],
        "dos-direct-workflow"
    );
    assert_eq!(kept["issue"]["theoria_refs"][0]["shared"], true);
    let kinds = send(&mut s, json!({"op":"changes","after":0}), Role::Agent, 40).unwrap()["events"]
        .to_string();
    assert!(
        kinds.contains("theoria_guidance_shared") && kinds.contains("theoria_guidance_unshared")
    );

    // Restart and archive recovery keep sharing and cross-product pins.
    share(&mut s, "dos-direct-workflow", true, Role::Human).unwrap();
    drop(s);
    let s = Store::open(&path).unwrap();
    let archive = s.export().unwrap();
    assert_eq!(archive.format, 20);
    validate_archive(&archive).unwrap();
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(archive.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        serde_json::to_value(&archive).unwrap()
    );

    // An older format cannot carry shared guidance.
    let mut old = archive.clone();
    old.format = 17;
    assert!(validate_archive(&old)
        .unwrap_err()
        .message
        .contains("format 18"));
    // A cross-product pin without sharing provenance is rejected.
    let mut forged = archive;
    for issue in &mut forged.issues {
        for reference in &mut issue.theoria_refs {
            reference.shared = false;
        }
    }
    assert!(validate_archive(&forged).is_err());
}
