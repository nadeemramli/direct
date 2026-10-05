//! DIR-71: owner-arranged sidebar product order and sections.
use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(s: &mut Store, mut value: Value, role: Role) -> Result<Value> {
    value["actor"] = json!("sidebar-owner");
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    s.execute_at(serde_json::from_value(value).unwrap(), role, 100)
}

fn snapshot(s: &mut Store) -> Value {
    send(s, json!({"op":"snapshot"}), Role::Agent).unwrap()
}

/// Products in display order as (key, section name or "").
fn sidebar(s: &mut Store) -> Vec<(String, String)> {
    let snap = snapshot(s);
    let sections = snap["product_sections"].as_array().unwrap().clone();
    let mut products = snap["products"].as_array().unwrap().clone();
    products.sort_by_key(|p| {
        (
            p["sort_order"].as_i64().unwrap(),
            p["id"].as_str().unwrap().to_owned(),
        )
    });
    products
        .iter()
        .map(|p| {
            let section = sections
                .iter()
                .find(|s| Some(s["id"].as_str().unwrap()) == p["section_id"].as_str())
                .map(|s| s["name"].as_str().unwrap().to_owned())
                .unwrap_or_default();
            (p["key"].as_str().unwrap().to_owned(), section)
        })
        .collect()
}

fn product_id(s: &mut Store, key: &str) -> String {
    snapshot(s)["products"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == key)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn seeded() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    for (key, name) in [("TER", "Teroka Sales"), ("OPS", "Operations")] {
        send(
            &mut s,
            json!({"op":"create_product","key":key,"name":name}),
            Role::Human,
        )
        .unwrap();
    }
    (dir, s)
}

#[test]
fn new_products_append_and_sections_are_owner_only() {
    let (_dir, mut s) = seeded();
    let order: Vec<_> = sidebar(&mut s).into_iter().map(|(k, _)| k).collect();
    assert_eq!(order, ["DIR", "TER", "OPS"]);

    let create = json!({"op":"create_product_section","name":"Teroka","request_id":"sec-1"});
    assert_eq!(
        send(&mut s, create.clone(), Role::Agent).unwrap_err().code,
        "forbidden"
    );
    let section = send(&mut s, create.clone(), Role::Human).unwrap();
    // Exact replay returns the same record instead of creating a second section.
    assert_eq!(send(&mut s, create, Role::Human).unwrap(), section);
    assert_eq!(section["name"], "Teroka");
    assert_eq!(section["version"], 1);

    for (name, code) in [
        ("  ", "invalid"),
        (" teroka ", "conflict"),
        (&"x".repeat(81) as &str, "invalid"),
    ] {
        assert_eq!(
            send(
                &mut s,
                json!({"op":"create_product_section","name":name}),
                Role::Human
            )
            .unwrap_err()
            .code,
            code,
            "{name:?}"
        );
    }
    let arrange = json!({"op":"arrange_products","sections":[],"products":[]});
    assert_eq!(
        send(&mut s, arrange, Role::Agent).unwrap_err().code,
        "forbidden"
    );
    assert_eq!(
        snapshot(&mut s)["product_sections"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn arrangement_moves_products_into_sections_and_rejects_stale_layouts() {
    let (_dir, mut s) = seeded();
    let dir_id = product_id(&mut s, "DIR");
    let ter = product_id(&mut s, "TER");
    let ops = product_id(&mut s, "OPS");
    let teroka = send(
        &mut s,
        json!({"op":"create_product_section","name":"Teroka"}),
        Role::Human,
    )
    .unwrap();
    let internal = send(
        &mut s,
        json!({"op":"create_product_section","name":"Internal"}),
        Role::Human,
    )
    .unwrap();
    let (teroka_id, internal_id) = (teroka["id"].clone(), internal["id"].clone());

    let arranged = send(
        &mut s,
        json!({"op":"arrange_products","sections":[internal_id, teroka_id],"products":[
            {"product_id":ops,"section_id":null},
            {"product_id":ter,"section_id":teroka_id},
            {"product_id":dir_id,"section_id":internal_id}
        ]}),
        Role::Human,
    )
    .unwrap();
    assert_eq!(arranged["products"].as_array().unwrap().len(), 3);
    assert_eq!(
        sidebar(&mut s),
        [
            ("OPS".to_owned(), String::new()),
            ("TER".to_owned(), "Teroka".to_owned()),
            ("DIR".to_owned(), "Internal".to_owned())
        ]
    );
    let snap = snapshot(&mut s);
    let sections = snap["product_sections"].as_array().unwrap();
    let internal_now = sections.iter().find(|x| x["id"] == internal_id).unwrap();
    assert_eq!(internal_now["sort_order"], 0);
    // Reordering a section bumps its version so stale renames are refused.
    assert_eq!(internal_now["version"], 2);
    assert_eq!(
        send(&mut s, json!({"op":"update_product_section","id":internal_id,"expected_version":1,"name":"Core"}), Role::Human)
            .unwrap_err()
            .code,
        "conflict"
    );

    // A product created after the layout was loaded makes that layout stale.
    send(
        &mut s,
        json!({"op":"create_product","key":"NEW","name":"Late"}),
        Role::Human,
    )
    .unwrap();
    let stale = json!({"op":"arrange_products","sections":[teroka_id, internal_id],"products":[
        {"product_id":dir_id},{"product_id":ter},{"product_id":ops}
    ]});
    assert_eq!(
        send(&mut s, stale, Role::Human).unwrap_err().code,
        "conflict"
    );
    assert_eq!(
        sidebar(&mut s).last().unwrap(),
        &("NEW".to_owned(), String::new())
    );

    let new = product_id(&mut s, "NEW");
    let all = |sections: Value, products: Value| json!({"op":"arrange_products","sections":sections,"products":products});
    // Duplicate product, missing section, unknown section reference.
    for (request, code) in [
        (
            all(
                json!([teroka_id, internal_id]),
                json!([{"product_id":dir_id},{"product_id":dir_id},{"product_id":ops},{"product_id":new}]),
            ),
            "conflict",
        ),
        (
            all(
                json!([teroka_id]),
                json!([{"product_id":dir_id},{"product_id":ter},{"product_id":ops},{"product_id":new}]),
            ),
            "conflict",
        ),
        (
            all(
                json!([teroka_id, internal_id]),
                json!([{"product_id":dir_id,"section_id":"missing"},{"product_id":ter},{"product_id":ops},{"product_id":new}]),
            ),
            "invalid",
        ),
    ] {
        assert_eq!(send(&mut s, request, Role::Human).unwrap_err().code, code);
    }
    // Nothing changed after the refusals.
    assert_eq!(sidebar(&mut s)[0], ("OPS".to_owned(), String::new()));
}

#[test]
fn renaming_and_deleting_a_section_keeps_every_product() {
    let (_dir, mut s) = seeded();
    let ter = product_id(&mut s, "TER");
    let ops = product_id(&mut s, "OPS");
    let dir_id = product_id(&mut s, "DIR");
    let section = send(
        &mut s,
        json!({"op":"create_product_section","name":"Teroka"}),
        Role::Human,
    )
    .unwrap();
    let id = section["id"].clone();
    send(
        &mut s,
        json!({"op":"arrange_products","sections":[id],"products":[
            {"product_id":dir_id},{"product_id":ter,"section_id":id},{"product_id":ops,"section_id":id}
        ]}),
        Role::Human,
    )
    .unwrap();
    let renamed = send(
        &mut s,
        json!({"op":"update_product_section","id":id,"expected_version":1,"name":"Teroka Project"}),
        Role::Human,
    )
    .unwrap();
    assert_eq!(renamed["version"], 2);
    assert_eq!(
        send(
            &mut s,
            json!({"op":"delete_product_section","id":id,"expected_version":1}),
            Role::Agent
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    assert_eq!(
        send(
            &mut s,
            json!({"op":"delete_product_section","id":id,"expected_version":1}),
            Role::Human
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    let deleted = send(
        &mut s,
        json!({"op":"delete_product_section","id":id,"expected_version":2}),
        Role::Human,
    )
    .unwrap();
    assert_eq!(deleted["ungrouped_products"], json!(["TER", "OPS"]));
    assert_eq!(
        sidebar(&mut s),
        [
            ("DIR".to_owned(), String::new()),
            ("TER".to_owned(), String::new()),
            ("OPS".to_owned(), String::new())
        ]
    );
    assert!(snapshot(&mut s)["product_sections"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        send(
            &mut s,
            json!({"op":"update_product_section","id":id,"expected_version":2,"name":"Gone"}),
            Role::Human
        )
        .unwrap_err()
        .code,
        "not_found"
    );
    // The change feed records every sidebar mutation.
    let kinds: Vec<String> = send(&mut s, json!({"op":"changes","after":0}), Role::Agent).unwrap()
        ["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap().to_owned())
        .collect();
    for kind in [
        "product_section_created",
        "products_arranged",
        "product_section_updated",
        "product_section_deleted",
    ] {
        assert!(kinds.iter().any(|k| k == kind), "{kind}");
    }
}

#[test]
fn arrangement_survives_reopen_and_archive_round_trip() {
    let (dir, mut s) = seeded();
    let ter = product_id(&mut s, "TER");
    let ops = product_id(&mut s, "OPS");
    let dir_id = product_id(&mut s, "DIR");
    let section = send(
        &mut s,
        json!({"op":"create_product_section","name":"Teroka"}),
        Role::Human,
    )
    .unwrap();
    send(
        &mut s,
        json!({"op":"arrange_products","sections":[section["id"]],"products":[
            {"product_id":ter,"section_id":section["id"]},{"product_id":ops},{"product_id":dir_id}
        ]}),
        Role::Human,
    )
    .unwrap();
    let expected = sidebar(&mut s);
    drop(s);
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    assert_eq!(sidebar(&mut s), expected);

    let archive = s.export().unwrap();
    assert_eq!(archive.format, ARCHIVE_FORMAT);
    assert_eq!(archive.format, 23);
    assert_eq!(archive.product_sections.len(), 1);
    validate_archive(&archive).unwrap();
    let other = TempDir::new().unwrap();
    let mut restored = Store::open(&other.path().join("db")).unwrap();
    restored.restore(archive.clone()).unwrap();
    assert_eq!(sidebar(&mut restored), expected);
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        serde_json::to_value(&archive).unwrap()
    );

    // Older formats cannot carry an arrangement…
    let mut older = archive.clone();
    older.format = 14;
    assert_eq!(validate_archive(&older).unwrap_err().code, "invalid");
    // …but a genuine format-14 archive (no arrangement) still restores.
    older.product_sections.clear();
    for p in &mut older.products {
        p.sort_order = 0;
        p.section_id = None;
    }
    validate_archive(&older).unwrap();
    let legacy = TempDir::new().unwrap();
    Store::open(&legacy.path().join("db"))
        .unwrap()
        .restore(older)
        .unwrap();

    // Dangling or duplicate section data is refused before anything is written.
    let mut dangling = archive.clone();
    dangling.product_sections.clear();
    assert_eq!(validate_archive(&dangling).unwrap_err().code, "invalid");
    let mut duplicate = archive.clone();
    let mut copy = duplicate.product_sections[0].clone();
    copy.id = uuid::Uuid::new_v4().to_string();
    copy.name = "TEROKA".into();
    duplicate.product_sections.push(copy);
    assert_eq!(validate_archive(&duplicate).unwrap_err().code, "invalid");
}

#[test]
fn schema_13_workspace_upgrades_in_place() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    {
        let mut s = Store::open(&path).unwrap();
        send(
            &mut s,
            json!({"op":"create_product","key":"TER","name":"Teroka"}),
            Role::Human,
        )
        .unwrap();
    }
    {
        // Recreate what a schema-13 binary left behind.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TABLE product_sections; UPDATE meta SET value='13' WHERE key='schema';",
        )
        .unwrap();
    }
    let mut s = Store::open(&path).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    let schema: String = conn
        .query_row("SELECT value FROM meta WHERE key='schema'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(schema, "21");
    assert_eq!(sidebar(&mut s).len(), 2);
    send(
        &mut s,
        json!({"op":"create_product_section","name":"Teroka"}),
        Role::Human,
    )
    .unwrap();
}
