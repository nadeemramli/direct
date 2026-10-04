//! DIR-23: typed context-document links on issues, projects, goals and releases.
use direct_core::*;
use serde_json::{json, Value};
use std::fs;
use tempfile::TempDir;

fn send(s: &mut Store, mut value: Value, role: Role) -> Result<Value> {
    value["actor"] = json!("context-tester");
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    s.execute_at(serde_json::from_value(value).unwrap(), role, 5_000)
}

struct Fixture {
    _dir: TempDir,
    vault: TempDir,
    db: std::path::PathBuf,
    s: Store,
    product_id: String,
    issue: String,
    project: String,
    goal: String,
    release: String,
}

fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let vault = TempDir::new().unwrap();
    fs::create_dir_all(vault.path().join("Specs")).unwrap();
    fs::write(
        vault.path().join("Specs/Reporting.md"),
        "---\ntitle: Reporting spec\n---\n# Ignored heading\nWeekly CSV export.\n",
    )
    .unwrap();
    fs::write(
        vault.path().join("Notes.md"),
        "# Meeting notes\nDecisions.\n",
    )
    .unwrap();
    // A byte-order mark (common from Windows editors) must not hide the front matter.
    fs::write(
        vault.path().join("Bom.md"),
        "\u{feff}---\ntitle: BOM titled\n---\nBody\n",
    )
    .unwrap();
    let db = dir.path().join("db");
    let mut s = Store::open(&db).unwrap();
    let product = send(
        &mut s,
        json!({"op":"create_product","key":"KNO","name":"Knowledge","vault_windows":vault.path().to_string_lossy()}),
        Role::Human,
    )
    .unwrap();
    let project = send(
        &mut s,
        json!({"op":"create_project","product":"KNO","name":"Reporting"}),
        Role::Human,
    )
    .unwrap();
    let goal = send(
        &mut s,
        json!({"op":"create_goal","product":"KNO","name":"Self-serve data","project_ids":[project["id"]]}),
        Role::Human,
    )
    .unwrap();
    let release = send(
        &mut s,
        json!({"op":"create_release","product":"KNO","name":"Spring","version_label":"1.0","target_ref":"main","project_ids":[project["id"]]}),
        Role::Human,
    )
    .unwrap();
    let issue = send(
        &mut s,
        json!({"op":"create_issue","product":"KNO","title":"CSV export","planning_scope":"project","project_id":project["id"]}),
        Role::Agent,
    )
    .unwrap();
    Fixture {
        _dir: dir,
        vault,
        db,
        s,
        product_id: product["id"].as_str().unwrap().into(),
        issue: issue["key"].as_str().unwrap().into(),
        project: project["id"].as_str().unwrap().into(),
        goal: goal["id"].as_str().unwrap().into(),
        release: release["id"].as_str().unwrap().into(),
    }
}

fn note(product_id: &str, path: &str) -> Value {
    json!({"kind":"obsidian","product_id":product_id,"path":path})
}

#[test]
fn obsidian_links_record_provenance_and_attach_to_every_target_kind() {
    let mut f = fixture();
    let add = json!({"op":"add_context_link","target_kind":"project","target":f.project,
        "source":note(&f.product_id,"Specs/Reporting.md"),"note":"Canonical spec","request_id":"add-1"});
    assert_eq!(
        send(&mut f.s, add.clone(), Role::Agent).unwrap_err().code,
        "forbidden"
    );
    let link = send(&mut f.s, add.clone(), Role::Human).unwrap();
    assert_eq!(send(&mut f.s, add, Role::Human).unwrap(), link);
    assert_eq!(link["title"], "Reporting spec");
    assert_eq!(link["observation"]["available"], true);
    assert_eq!(link["observation"]["checked_at"], 5_000);
    let fp = link["pinned_fingerprint"].as_str().unwrap().to_owned();
    assert_eq!(fp.len(), 64);
    assert_eq!(link["observation"]["fingerprint"], fp.as_str());

    for (kind, target) in [
        ("issue", f.issue.clone()),
        ("goal", f.goal.clone()),
        ("release", f.release.clone()),
    ] {
        let added = send(
            &mut f.s,
            json!({"op":"add_context_link","target_kind":kind,"target":target,"source":note(&f.product_id,"Notes.md")}),
            Role::Human,
        )
        .unwrap();
        assert_eq!(added["title"], "Meeting notes");
    }
    let url = send(
        &mut f.s,
        json!({"op":"add_context_link","target_kind":"issue","target":f.issue,
               "source":{"kind":"url","url":"https://example.com/design/csv"},"title":"Design board"}),
        Role::Human,
    )
    .unwrap();
    let bom = send(
        &mut f.s,
        json!({"op":"add_context_link","target_kind":"issue","target":f.issue,"source":note(&f.product_id,"Bom.md")}),
        Role::Human,
    )
    .unwrap();
    assert_eq!(bom["title"], "BOM titled");
    assert_eq!(url["observation"]["fingerprint"], Value::Null);
    assert_eq!(url["url"], "https://example.com/design/csv");

    // The issue sees its own links plus those inherited from project, goal and release,
    // separately from Theoria guidance.
    let ctx = send(&mut f.s, json!({"op":"context","key":f.issue}), Role::Agent).unwrap();
    let kinds: Vec<&str> = ctx["context_links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["target_kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds.len(), 6);
    for k in ["issue", "project", "goal", "release"] {
        assert!(kinds.contains(&k), "{k}");
    }
    assert!(ctx["issue"]["theoria_refs"].as_array().unwrap().is_empty());
    assert!(ctx["context_authority"]
        .as_str()
        .unwrap()
        .contains("not Theoria guidance"));
    // Obsidian content is linked, never stored: the archive has the path and fingerprint only.
    let archive = serde_json::to_string(&f.s.export().unwrap()).unwrap();
    assert!(archive.contains("Specs/Reporting.md"));
    assert!(!archive.contains("Weekly CSV export"));
}

#[test]
fn invalid_paths_targets_and_sources_are_refused() {
    let mut f = fixture();
    let add = |target_kind: &str, target: &str, source: Value| json!({"op":"add_context_link","target_kind":target_kind,"target":target,"source":source});
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("secret.md"), "private").unwrap();
    let escape = format!(
        "../{}/secret.md",
        outside.path().file_name().unwrap().to_string_lossy()
    );
    for (request, code) in [
        (
            add("project", &f.project, note(&f.product_id, "../outside.md")),
            "invalid",
        ),
        (
            add("project", &f.project, note(&f.product_id, &escape)),
            "invalid",
        ),
        (
            add(
                "project",
                &f.project,
                note(&f.product_id, "C:/Windows/win.ini"),
            ),
            "invalid",
        ),
        (
            add("project", &f.project, note(&f.product_id, "Missing.md")),
            "not_found",
        ),
        (
            add("project", &f.project, note(&f.product_id, "Specs")),
            "not_found",
        ),
        (
            add(
                "project",
                &f.project,
                json!({"kind":"url","url":"ftp://example.com"}),
            ),
            "invalid",
        ),
        (
            add(
                "project",
                &f.project,
                json!({"kind":"url","url":"javascript:alert(1)"}),
            ),
            "invalid",
        ),
        (
            add(
                "project",
                &f.project,
                json!({"kind":"retained_record","record_id":"nope"}),
            ),
            "not_found",
        ),
        (
            add("issue", "KNO-999", note(&f.product_id, "Notes.md")),
            "not_found",
        ),
        (
            add("goal", "missing", note(&f.product_id, "Notes.md")),
            "not_found",
        ),
    ] {
        assert_eq!(
            send(&mut f.s, request.clone(), Role::Human)
                .unwrap_err()
                .code,
            code,
            "{request}"
        );
    }
    let ok = add("project", &f.project, note(&f.product_id, "Notes.md"));
    send(&mut f.s, ok.clone(), Role::Human).unwrap();
    assert_eq!(
        send(&mut f.s, ok, Role::Human).unwrap_err().code,
        "conflict"
    );
    // A product without a vault path cannot resolve notes.
    let no_vault = send(
        &mut f.s,
        json!({"op":"create_project","product":"DIR","name":"Plain"}),
        Role::Human,
    )
    .unwrap();
    let dir_product = send(&mut f.s, json!({"op":"snapshot"}), Role::Agent).unwrap()["products"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == "DIR")
        .unwrap()["id"]
        .clone();
    let err = send(
        &mut f.s,
        add(
            "project",
            no_vault["id"].as_str().unwrap(),
            json!({"kind":"obsidian","product_id":dir_product,"path":"Notes.md"}),
        ),
        Role::Human,
    )
    .unwrap_err();
    assert_eq!(err.code, "not_found");
    assert!(err.message.contains("no knowledge vault"));
}

#[cfg(windows)]
#[test]
fn symlinks_cannot_escape_the_vault() {
    let mut f = fixture();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("secret.md"), "private").unwrap();
    // Creating symlinks needs Developer Mode or elevation; a junction does not.
    let link = f.vault.path().join("escape");
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(status.status.success(), "junction creation failed");
    let err = send(
        &mut f.s,
        json!({"op":"add_context_link","target_kind":"project","target":f.project,"source":note(&f.product_id,"escape/secret.md")}),
        Role::Human,
    )
    .unwrap_err();
    assert_eq!(err.code, "not_found");
    assert!(
        err.message.contains("outside the product vault"),
        "{}",
        err.message
    );
}

#[test]
fn rechecks_show_changes_and_loss_without_repinning_and_reads_are_on_demand() {
    let mut f = fixture();
    let link = send(
        &mut f.s,
        json!({"op":"add_context_link","target_kind":"issue","target":f.issue,"source":note(&f.product_id,"Notes.md")}),
        Role::Human,
    )
    .unwrap();
    let id = link["id"].clone();
    let pinned = link["pinned_fingerprint"].clone();
    let read = send(
        &mut f.s,
        json!({"op":"read_context_link","id":id}),
        Role::Agent,
    )
    .unwrap();
    assert!(read["content"].as_str().unwrap().contains("Decisions."));
    assert_eq!(read["changed_since_linked"], false);
    assert!(read["authority"]
        .as_str()
        .unwrap()
        .contains("grants no tool authority"));

    fs::write(
        f.vault.path().join("Notes.md"),
        "# Meeting notes\nDecisions, revised.\n",
    )
    .unwrap();
    let read = send(
        &mut f.s,
        json!({"op":"read_context_link","id":id}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(read["changed_since_linked"], true);
    // Reading never writes.
    assert_eq!(
        send(&mut f.s, json!({"op":"snapshot"}), Role::Agent).unwrap()["context_links"][0]
            ["version"],
        1
    );

    let checked = send(
        &mut f.s,
        json!({"op":"check_context_link","id":id,"expected_version":1}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(checked["version"], 2);
    assert_ne!(checked["observation"]["fingerprint"], pinned);
    assert_eq!(checked["pinned_fingerprint"], pinned);

    fs::remove_file(f.vault.path().join("Notes.md")).unwrap();
    let lost = send(
        &mut f.s,
        json!({"op":"check_context_link","id":id,"expected_version":2}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(lost["observation"]["available"], false);
    assert!(lost["observation"]["reason"]
        .as_str()
        .unwrap()
        .contains("Note not found"));
    assert_eq!(lost["pinned_fingerprint"], pinned);
    let read = send(
        &mut f.s,
        json!({"op":"read_context_link","id":id}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(read["content"], Value::Null);
    assert_eq!(
        send(
            &mut f.s,
            json!({"op":"check_context_link","id":id,"expected_version":2}),
            Role::Agent
        )
        .unwrap_err()
        .code,
        "conflict"
    );

    // An issue with its own context links shows a removable deletion blocker.
    let deletion =
        &send(&mut f.s, json!({"op":"context","key":f.issue}), Role::Agent).unwrap()["deletion"];
    let blocker = deletion["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["kind"] == "context_links")
        .unwrap();
    assert_eq!(blocker["removable"], true);
    assert_eq!(
        send(
            &mut f.s,
            json!({"op":"remove_context_link","id":id,"expected_version":3}),
            Role::Agent
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    send(
        &mut f.s,
        json!({"op":"remove_context_link","id":id,"expected_version":3}),
        Role::Human,
    )
    .unwrap();
    let ctx = send(&mut f.s, json!({"op":"context","key":f.issue}), Role::Agent).unwrap();
    assert!(ctx["context_links"].as_array().unwrap().is_empty());
}

#[test]
fn retained_linear_documents_link_read_only_with_original_ids() {
    let mut f = fixture();
    // Insert a retained Linear document the way the source importer stores it.
    let doc = br#"{"id":"lin-doc-1","title":"Reporting PRD","url":"https://linear.app/acme/document/reporting-prd","content":"Weekly CSV export for finance."}"#;
    {
        let conn = rusqlite::Connection::open(&f.db).unwrap();
        let bundle = "00000000-0000-4000-8000-0000000000b1";
        let file = json!({"bundle_id":bundle,"path":"data/documents.json","sha256":format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(doc)),"bytes":doc.len(),"content_type":"application/json","role":"data"});
        conn.execute(
            "INSERT INTO source_files VALUES (?1,?2,?3)",
            rusqlite::params![
                format!("{bundle}/data/documents.json"),
                bundle,
                file.to_string()
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO source_chunks VALUES (?1,0,?2)",
            rusqlite::params![format!("{bundle}/data/documents.json"), &doc[..]],
        )
        .unwrap();
        let record = json!({"id":"rec-doc-1","bundle_id":bundle,"kind":"document","level":"record","source_id":"lin-doc-1","label":"Reporting PRD","classification":"preserved","access":"retained","file":"data/documents.json","start":0,"end":doc.len()});
        conn.execute("INSERT INTO source_records VALUES ('rec-doc-1',?1,'document','preserved','reporting prd',?2)", rusqlite::params![bundle, record.to_string()]).unwrap();
    }
    let link = send(
        &mut f.s,
        json!({"op":"add_context_link","target_kind":"project","target":f.project,"source":{"kind":"retained_record","record_id":"rec-doc-1"}}),
        Role::Human,
    )
    .unwrap();
    assert_eq!(link["title"], "Reporting PRD");
    assert_eq!(link["source_id"], "lin-doc-1");
    assert_eq!(
        link["url"],
        "https://linear.app/acme/document/reporting-prd"
    );
    assert_eq!(link["observation"]["available"], true);
    let read = send(
        &mut f.s,
        json!({"op":"read_context_link","id":link["id"]}),
        Role::Agent,
    )
    .unwrap();
    assert!(read["content"]
        .as_str()
        .unwrap()
        .contains("Weekly CSV export for finance."));

    // A retained record that disappears is reported, not hidden.
    rusqlite::Connection::open(&f.db)
        .unwrap()
        .execute("DELETE FROM source_chunks", [])
        .unwrap();
    let checked = send(
        &mut f.s,
        json!({"op":"check_context_link","id":link["id"],"expected_version":1}),
        Role::Agent,
    )
    .unwrap();
    assert_eq!(checked["observation"]["available"], false);
    assert!(checked["observation"]["reason"]
        .as_str()
        .unwrap()
        .starts_with("Retained record unavailable"));
}

#[test]
fn links_survive_reopen_and_archive_round_trip_with_format_checks() {
    let mut f = fixture();
    send(&mut f.s, json!({"op":"add_context_link","target_kind":"goal","target":f.goal,"source":note(&f.product_id,"Notes.md")}), Role::Human).unwrap();
    send(
        &mut f.s,
        json!({"op":"add_context_link","target_kind":"release","target":f.release,"source":{"kind":"url","url":"https://example.com/release-notes"}}),
        Role::Human,
    )
    .unwrap();
    let before =
        send(&mut f.s, json!({"op":"snapshot"}), Role::Agent).unwrap()["context_links"].clone();
    let db = f.db.clone();
    drop(f.s);
    let mut s = Store::open(&db).unwrap();
    assert_eq!(
        send(&mut s, json!({"op":"snapshot"}), Role::Agent).unwrap()["context_links"],
        before
    );

    let archive = s.export().unwrap();
    assert_eq!(archive.format, ARCHIVE_FORMAT);
    assert_eq!(archive.format, 17);
    assert_eq!(archive.context_links.len(), 2);
    validate_archive(&archive).unwrap();
    let other = TempDir::new().unwrap();
    let mut restored = Store::open(&other.path().join("db")).unwrap();
    restored.restore(archive.clone()).unwrap();
    assert_eq!(
        send(&mut restored, json!({"op":"snapshot"}), Role::Agent).unwrap()["context_links"],
        before
    );
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        serde_json::to_value(&archive).unwrap()
    );
    let invalid = |change: &dyn Fn(&mut Archive)| {
        let mut a = archive.clone();
        change(&mut a);
        validate_archive(&a).unwrap_err().code
    };
    assert_eq!(invalid(&|a| a.format = 16), "invalid");
    assert_eq!(
        invalid(&|a| a.context_links[0].target = "missing".into()),
        "invalid"
    );
    assert_eq!(
        invalid(
            &|a| a.context_links[0].source = ContextSource::RetainedRecord {
                record_id: "none".into()
            }
        ),
        "invalid"
    );
    assert_eq!(
        invalid(&|a| a.context_links[0].source = ContextSource::Obsidian {
            product_id: a.products[0].id.clone(),
            path: "../x.md".into()
        }),
        "invalid"
    );
    assert_eq!(
        invalid(&|a| {
            let c = a.context_links[0].clone();
            a.context_links.push(c);
        }),
        "invalid"
    );
    let mut older = archive.clone();
    older.format = 16;
    older.context_links.clear();
    validate_archive(&older).unwrap();
}

#[test]
fn schema_15_workspace_upgrades_in_place() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    drop(Store::open(&path).unwrap());
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TABLE context_links; UPDATE meta SET value='15' WHERE key='schema';",
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
    assert_eq!(schema, "16");
    assert!(
        send(&mut s, json!({"op":"snapshot"}), Role::Agent).unwrap()["context_links"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
