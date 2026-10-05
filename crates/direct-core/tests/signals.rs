//! DIR-24: customer requests captured as provenance-bearing signals.
use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(s: &mut Store, mut value: Value, role: Role) -> Result<Value> {
    value["actor"] = json!("signal-tester");
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    s.execute_at(serde_json::from_value(value).unwrap(), role, 1_000)
}

fn context(s: &mut Store, key: &str) -> Value {
    send(s, json!({"op":"context","key":key}), Role::Agent).unwrap()
}

fn signals(s: &mut Store) -> Vec<Value> {
    send(s, json!({"op":"snapshot"}), Role::Agent).unwrap()["customer_signals"]
        .as_array()
        .unwrap()
        .clone()
}

fn capture(s: &mut Store, summary: &str) -> Value {
    send(
        s,
        json!({"op":"capture_signal","product":"DIR","source_kind":"support","source_reference":"Ticket 4412",
               "summary":summary,"received_at":900,"customer_reference":"Acme account"}),
        Role::Agent,
    )
    .unwrap()
}

fn seeded() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    send(
        &mut s,
        json!({"op":"create_product","key":"ALT","name":"Other"}),
        Role::Human,
    )
    .unwrap();
    (dir, s)
}

#[test]
fn capture_validates_fields_and_replays_exactly() {
    let (_dir, mut s) = seeded();
    let request = json!({"op":"capture_signal","product":"DIR","source_kind":"call","source_reference":" Call notes 3 Oct ",
        "summary":"  Export the weekly report as CSV  ","received_at":900,"customer_reference":"Enterprise segment",
        "request_id":"capture-1"});
    let first = send(&mut s, request.clone(), Role::Agent).unwrap();
    assert_eq!(send(&mut s, request, Role::Agent).unwrap(), first);
    assert_eq!(first["summary"], "Export the weekly report as CSV");
    assert_eq!(first["source_reference"], "Call notes 3 Oct");
    assert_eq!(first["source_kind"], "call");
    assert_eq!(first["version"], 1);
    assert_eq!(first["archived"], false);
    assert_eq!(signals(&mut s).len(), 1);

    let base = json!({"op":"capture_signal","product":"DIR","source_kind":"email","summary":"x","received_at":900});
    let with = |field: &str, value: Value| {
        let mut v = base.clone();
        v[field] = value;
        v
    };
    for (request, code) in [
        (with("summary", json!("   ")), "invalid"),
        (with("summary", json!("y".repeat(2001))), "invalid"),
        (
            with("customer_reference", json!("jane@acme.com")),
            "invalid",
        ),
        (
            with("customer_reference", json!("+1 (415) 555-0100")),
            "invalid",
        ),
        (with("received_at", json!(0)), "invalid"),
        (with("product", json!("NOPE")), "not_found"),
        (with("external_source", json!("zendesk")), "invalid"),
    ] {
        assert_eq!(
            send(&mut s, request.clone(), Role::Agent).unwrap_err().code,
            code,
            "{request}"
        );
    }
    let bad_kind = with("source_kind", json!("carrier_pigeon"));
    assert!(serde_json::from_value::<Request>(json!({"actor":"a","request_id":"r","op":"capture_signal","product":"DIR","source_kind":bad_kind["source_kind"],"summary":"x","received_at":1})).is_err());
    assert_eq!(signals(&mut s).len(), 1);
}

#[test]
fn many_requests_link_to_one_issue_and_one_request_to_many_items() {
    let (_dir, mut s) = seeded();
    let issue = send(
        &mut s,
        json!({"op":"create_issue","product":"DIR","title":"CSV export"}),
        Role::Agent,
    )
    .unwrap();
    let key = issue["key"].as_str().unwrap().to_owned();
    let other = send(
        &mut s,
        json!({"op":"create_issue","product":"DIR","title":"Scheduling"}),
        Role::Agent,
    )
    .unwrap();
    let project = send(
        &mut s,
        json!({"op":"create_project","product":"DIR","name":"Reporting"}),
        Role::Human,
    )
    .unwrap();
    let foreign = send(
        &mut s,
        json!({"op":"create_issue","product":"ALT","title":"Elsewhere"}),
        Role::Agent,
    )
    .unwrap();
    let a = capture(&mut s, "Need CSV export");
    let b = capture(&mut s, "CSV for finance team");

    let link = |id: &Value, version: u64, kind: &str, target: &Value| json!({"op":"link_signal","id":id,"expected_version":version,"kind":kind,"target":target});
    let a2 = send(&mut s, link(&a["id"], 1, "issue", &json!(key)), Role::Agent).unwrap();
    send(&mut s, link(&b["id"], 1, "issue", &json!(key)), Role::Agent).unwrap();
    let a3 = send(
        &mut s,
        link(&a["id"], 2, "issue", &other["key"]),
        Role::Agent,
    )
    .unwrap();
    let a4 = send(
        &mut s,
        link(&a["id"], 3, "project", &project["id"]),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(a2["version"], 2);
    assert_eq!(a4["links"].as_array().unwrap().len(), 3);
    // The text is stored once, on the request, never copied into the issue.
    let ctx = context(&mut s, &key);
    assert_eq!(ctx["customer_signals"].as_array().unwrap().len(), 2);
    assert_eq!(ctx["issue"]["body"], "");
    // Linking never changes the issue itself (version, status, readiness).
    assert_eq!(ctx["issue"]["version"], issue["version"]);
    assert_eq!(ctx["issue"]["status"], "backlog");

    for (request, code) in [
        (link(&a["id"], 3, "project", &project["id"]), "conflict"), // stale version
        (link(&a["id"], 4, "issue", &json!(key)), "conflict"),      // duplicate link
        (link(&a["id"], 4, "issue", &foreign["key"]), "invalid"),   // other product
        (link(&a["id"], 4, "issue", &json!("DIR-999")), "not_found"),
        (link(&a["id"], 4, "project", &json!("missing")), "not_found"),
        (
            link(&json!("missing"), 1, "issue", &json!(key)),
            "not_found",
        ),
    ] {
        assert_eq!(
            send(&mut s, request.clone(), Role::Agent).unwrap_err().code,
            code,
            "{request}"
        );
    }

    // A linked issue cannot be deleted until the link is removed.
    let deletion = &context(&mut s, &key)["deletion"];
    assert_eq!(deletion["eligible"], false);
    let blocker = &deletion["blockers"].as_array().unwrap()[0];
    assert_eq!(blocker["kind"], "customer_signals");
    assert_eq!(blocker["removable"], true);
    let unlinked = send(
        &mut s,
        json!({"op":"unlink_signal","id":a["id"],"expected_version":a3["version"].as_u64().unwrap() + 1,"kind":"issue","target":key}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(unlinked["links"].as_array().unwrap().len(), 2);
    assert_eq!(
        send(&mut s, json!({"op":"unlink_signal","id":a["id"],"expected_version":5,"kind":"issue","target":key}), Role::Agent)
            .unwrap_err()
            .code,
        "not_found"
    );
}

#[test]
fn promotion_creates_one_inbox_issue_with_provenance() {
    let (_dir, mut s) = seeded();
    let signal = capture(
        &mut s,
        "Bulk edit labels\nCustomers want to relabel many issues at once.",
    );
    let promote = json!({"op":"promote_signal","id":signal["id"],"expected_version":1,"request_id":"promote-1"});
    let result = send(&mut s, promote.clone(), Role::Agent).unwrap();
    // An exact retry replays the same answer instead of creating a second issue.
    assert_eq!(send(&mut s, promote, Role::Agent).unwrap(), result);
    let issue = &result["issue"];
    let key = issue["key"].as_str().unwrap();
    assert_eq!(issue["title"], "Bulk edit labels");
    assert_eq!(issue["status"], "backlog");
    assert_eq!(issue["planning_scope"], "inbox");
    assert_eq!(issue["priority"], "medium");
    assert!(issue["claim"].is_null());
    let body = issue["body"].as_str().unwrap();
    assert!(body.contains("Customers want to relabel many issues at once."));
    assert!(body.contains(signal["id"].as_str().unwrap()));
    assert!(body.contains("support · Ticket 4412"));
    assert_eq!(result["signal"]["promoted_issue_key"], key);
    assert_eq!(result["signal"]["links"][0]["target"], key);

    // A second promotion under a new request is refused.
    assert_eq!(
        send(
            &mut s,
            json!({"op":"promote_signal","id":signal["id"],"expected_version":2}),
            Role::Agent
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    // The promoted issue keeps its provenance: deletion is refused even after unlinking.
    send(&mut s, json!({"op":"unlink_signal","id":signal["id"],"expected_version":2,"kind":"issue","target":key}), Role::Agent).unwrap();
    let deletion = &context(&mut s, key)["deletion"];
    assert_eq!(deletion["eligible"], false);
    assert_eq!(deletion["blockers"][0]["kind"], "customer_signals");
    assert_eq!(deletion["blockers"][0]["removable"], false);
    assert_eq!(
        send(
            &mut s,
            json!({"op":"delete_issue","key":key,"expected_version":1}),
            Role::Human
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    // A custom title is honoured; archived requests must be restored first.
    let other = capture(&mut s, "Dark mode");
    send(
        &mut s,
        json!({"op":"archive_signal","id":other["id"],"expected_version":1,"archived":true}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(
        send(
            &mut s,
            json!({"op":"promote_signal","id":other["id"],"expected_version":2}),
            Role::Agent
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    send(
        &mut s,
        json!({"op":"archive_signal","id":other["id"],"expected_version":2,"archived":false}),
        Role::Agent,
    )
    .unwrap();
    let titled = send(
        &mut s,
        json!({"op":"promote_signal","id":other["id"],"expected_version":3,"title":"Offer a dark theme"}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(titled["issue"]["title"], "Offer a dark theme");
}

#[test]
fn signals_never_grant_readiness_review_or_owner_actions() {
    let (_dir, mut s) = seeded();
    let signal = capture(&mut s, "Faster search");
    let promoted = send(
        &mut s,
        json!({"op":"promote_signal","id":signal["id"],"expected_version":1}),
        Role::Agent,
    )
    .unwrap();
    let key = promoted["issue"]["key"].as_str().unwrap();
    // The agent still cannot make the promoted work Ready.
    assert_eq!(
        send(
            &mut s,
            json!({"op":"ready","key":key,"expected_version":1}),
            Role::Agent
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    let ctx = context(&mut s, key);
    assert_eq!(ctx["issue"]["status"], "backlog");
    assert!(ctx["verifications"].as_array().unwrap().is_empty());
}

#[test]
fn imports_keep_source_ids_and_unresolved_mappings_and_refuse_duplicates() {
    let (_dir, mut s) = seeded();
    let imported = send(
        &mut s,
        json!({"op":"capture_signal","product":"DIR","source_kind":"support","summary":"Calendar sync",
               "received_at":800,"external_source":"linear-ask","external_id":"ASK-77",
               "unresolved_mappings":[{"external_source":"linear","external_id":"LIN-404","note":"Issue not imported"},
                                      {"external_source":"linear","external_id":"LIN-404","note":"Issue not imported"}]}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(imported["external_id"], "ASK-77");
    assert_eq!(imported["unresolved_mappings"].as_array().unwrap().len(), 1);
    let duplicate = json!({"op":"capture_signal","product":"DIR","source_kind":"support","summary":"Calendar sync again",
        "received_at":800,"external_source":"linear-ask","external_id":"ASK-77"});
    assert_eq!(
        send(&mut s, duplicate, Role::Agent).unwrap_err().code,
        "conflict"
    );
    // The same external ID in another product is a different source record.
    send(
        &mut s,
        json!({"op":"capture_signal","product":"ALT","source_kind":"support","summary":"Calendar sync",
               "received_at":800,"external_source":"linear-ask","external_id":"ASK-77"}),
        Role::Agent,
    )
    .unwrap();
    // Mappings survive linking the request to real work.
    let issue = send(
        &mut s,
        json!({"op":"create_issue","product":"DIR","title":"Calendar"}),
        Role::Agent,
    )
    .unwrap();
    let linked = send(
        &mut s,
        json!({"op":"link_signal","id":imported["id"],"expected_version":1,"kind":"issue","target":issue["key"]}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(linked["unresolved_mappings"][0]["external_id"], "LIN-404");
}

#[test]
fn signals_survive_reopen_archive_and_restore_with_format_checks() {
    let (dir, mut s) = seeded();
    let issue = send(
        &mut s,
        json!({"op":"create_issue","product":"DIR","title":"Reports"}),
        Role::Agent,
    )
    .unwrap();
    let a = capture(&mut s, "Weekly report");
    send(&mut s, json!({"op":"link_signal","id":a["id"],"expected_version":1,"kind":"issue","target":issue["key"]}), Role::Agent).unwrap();
    let b = capture(&mut s, "Old request");
    send(
        &mut s,
        json!({"op":"archive_signal","id":b["id"],"expected_version":1,"archived":true}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(
        send(
            &mut s,
            json!({"op":"archive_signal","id":b["id"],"expected_version":2,"archived":true}),
            Role::Agent
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    let c = capture(&mut s, "Promote me");
    send(
        &mut s,
        json!({"op":"promote_signal","id":c["id"],"expected_version":1}),
        Role::Agent,
    )
    .unwrap();
    let before = signals(&mut s);
    drop(s);
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    assert_eq!(signals(&mut s), before);

    let archive = s.export().unwrap();
    assert_eq!(archive.format, ARCHIVE_FORMAT);
    assert_eq!(archive.format, 22);
    assert_eq!(archive.customer_signals.len(), 3);
    validate_archive(&archive).unwrap();
    let other = TempDir::new().unwrap();
    let mut restored = Store::open(&other.path().join("db")).unwrap();
    restored.restore(archive.clone()).unwrap();
    assert_eq!(signals(&mut restored), before);
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        serde_json::to_value(&archive).unwrap()
    );
    let kinds: Vec<String> = send(
        &mut restored,
        json!({"op":"changes","after":0}),
        Role::Agent,
    )
    .unwrap()["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap().to_owned())
        .collect();
    for kind in [
        "signal_captured",
        "signal_linked",
        "signal_archived",
        "signal_promoted",
    ] {
        assert!(kinds.iter().any(|k| k == kind), "{kind}");
    }

    let invalid = |mut a: Archive, change: &dyn Fn(&mut Archive)| {
        change(&mut a);
        validate_archive(&a).unwrap_err().code
    };
    assert_eq!(invalid(archive.clone(), &|a| a.format = 15), "invalid");
    assert_eq!(
        invalid(archive.clone(), &|a| {
            // Archive order follows IDs; pick a request that actually has a link.
            let linked = a
                .customer_signals
                .iter()
                .position(|s| !s.links.is_empty())
                .unwrap();
            a.customer_signals[linked].links[0].target = "DIR-999".into();
        }),
        "invalid"
    );
    assert_eq!(
        invalid(archive.clone(), &|a| a.customer_signals[1].product_id =
            "missing".into()),
        "invalid"
    );
    assert_eq!(
        invalid(archive.clone(), &|a| a.customer_signals[0]
            .customer_reference =
            "x@y.com".into()),
        "invalid"
    );
    assert_eq!(
        invalid(archive.clone(), &|a| {
            let copy = a.customer_signals[0].clone();
            a.customer_signals.push(copy);
        }),
        "invalid"
    );
    // A genuine format-15 archive (no customer requests) still restores.
    let mut older = archive.clone();
    older.format = 15;
    older.customer_signals.clear();
    validate_archive(&older).unwrap();
}

#[test]
fn schema_14_workspace_upgrades_in_place() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    {
        let mut s = Store::open(&path).unwrap();
        send(
            &mut s,
            json!({"op":"create_issue","product":"DIR","title":"Existing"}),
            Role::Agent,
        )
        .unwrap();
    }
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TABLE customer_signals; UPDATE meta SET value='14' WHERE key='schema';",
        )
        .unwrap();
    }
    let mut s = Store::open(&path).unwrap();
    let schema: String = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row("SELECT value FROM meta WHERE key='schema'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(schema, "20");
    assert!(signals(&mut s).is_empty());
    capture(&mut s, "Works after upgrade");
}

#[test]
fn captured_requests_can_be_corrected_without_touching_provenance_or_links() {
    let (_dir, mut s) = seeded();
    let issue = send(
        &mut s,
        json!({"op":"create_issue","product":"DIR","title":"Exports"}),
        Role::Agent,
    )
    .unwrap();
    let imported = send(
        &mut s,
        json!({"op":"capture_signal","product":"DIR","source_kind":"support","summary":"Exprot to CSV",
               "received_at":800,"customer_reference":"Acme","external_source":"zendesk","external_id":"Z-1",
               "unresolved_mappings":[{"external_source":"linear","external_id":"LIN-9","note":""}]}),
        Role::Agent,
    )
    .unwrap();
    send(
        &mut s,
        json!({"op":"link_signal","id":imported["id"],"expected_version":1,"kind":"issue","target":issue["key"]}),
        Role::Agent,
    )
    .unwrap();
    let edit = |version: u64, summary: &str, customer: &str| {
        json!({"op":"update_signal","id":imported["id"],"expected_version":version,"source_kind":"call",
               "source_reference":"Call 12","summary":summary,"received_at":850,"customer_reference":customer})
    };
    let updated = send(
        &mut s,
        edit(2, " Export to CSV ", "Acme finance"),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(updated["summary"], "Export to CSV");
    assert_eq!(updated["source_kind"], "call");
    assert_eq!(updated["source_reference"], "Call 12");
    assert_eq!(updated["received_at"], 850);
    assert_eq!(updated["customer_reference"], "Acme finance");
    assert_eq!(updated["version"], 3);
    // Provenance, links and mappings are untouched.
    assert_eq!(updated["external_id"], "Z-1");
    assert_eq!(updated["unresolved_mappings"][0]["external_id"], "LIN-9");
    assert_eq!(updated["links"][0]["target"], issue["key"]);
    for (request, code) in [
        (edit(2, "Stale edit", "Acme"), "conflict"),
        (edit(3, "Export to CSV", "Acme finance"), "conflict"),
        (edit(3, "Export to CSV", "ops@acme.com"), "invalid"),
        (edit(3, "   ", "Acme"), "invalid"),
    ] {
        assert_eq!(
            send(&mut s, request.clone(), Role::Agent).unwrap_err().code,
            code,
            "{request}"
        );
    }
    let ctx = send(
        &mut s,
        json!({"op":"context","key":issue["key"]}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(ctx["customer_signals"][0]["summary"], "Export to CSV");
    let kinds =
        send(&mut s, json!({"op":"changes","after":0}), Role::Agent).unwrap()["events"].to_string();
    assert!(kinds.contains("signal_updated"));
}
