//! End-to-end rehearsal of the whole-workspace Linear import through the real
//! CLI: package verification → isolated database → export → service restart →
//! recovery check → replay. All data is synthetic.

#[path = "support/linear_fixture.rs"]
mod support;

use direct::Client;
use direct_core::{Command, Request, Role};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};
use support::*;

#[test]
fn whole_workspace_rehearsal_reconciles_preserves_restores_and_replays() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join("owner-data");
    let source = temp.path().join("capture");
    fixture().write(&source);
    let output = temp.path().join("rehearsal");

    // C1/C4/C5: first run through the real CLI.
    let first = succeeded(import(&data_dir, &source, &output));
    assert_eq!(first["mode"], "whole_workspace");
    assert_eq!(first["idempotent_replay"], false);
    assert!(first.get("verified").is_none(), "no blanket verified flag");
    assert_eq!(first["cutover_readiness"]["status"], "not_complete");
    assert_eq!(first["cutover_readiness"]["ready"], false);
    assert_eq!(first["rehearsal"]["checks"]["restore_byte_match"], true);
    assert_eq!(
        first["rehearsal"]["checks"]["records_accounted"],
        first["rehearsal"]["checks"]["records_in_source"]
    );
    assert!(
        !data_dir.exists(),
        "the owner data directory is never created or touched"
    );

    let archive: Value =
        serde_json::from_slice(&fs::read(output.join("direct-import.json")).unwrap()).unwrap();
    let report: Value =
        serde_json::from_slice(&fs::read(output.join("reconciliation.json")).unwrap()).unwrap();
    let accounting: Value =
        serde_json::from_slice(&fs::read(output.join("accounting.json")).unwrap()).unwrap();

    // C1: all teams, projects (incl. archived and multi-team), issues (incl. unprojected/archived).
    assert_eq!(archive["products"].as_array().unwrap().len(), 3);
    assert_eq!(archive["projects"].as_array().unwrap().len(), 4);
    assert_eq!(archive["issues"].as_array().unwrap().len(), 7);
    assert!(archive["verifications"].as_array().unwrap().is_empty());
    assert!(archive["events"].as_array().unwrap().is_empty());
    let eng1 = issue_by_key(&archive, "ENG-1");
    assert_eq!(eng1["status"], "legacy_completed");
    assert_eq!(eng1["title"], "<script>alert(1)</script> Parent work");
    assert_eq!(eng1["owner"], "Ada");
    assert_eq!(eng1["priority"], "urgent");
    assert_eq!(eng1["milestone_id"], "m-alpha-1");
    assert_eq!(eng1["external"]["created_at"], CREATED);
    assert_eq!(eng1["external"]["history"].as_array().unwrap().len(), 2);
    assert_eq!(eng1["labels"].as_array().unwrap().len(), 2);
    let eng2 = issue_by_key(&archive, "ENG-2");
    assert_eq!(eng2["status"], "backlog");
    assert_eq!(eng2["external"]["state"]["type"], "started");
    assert_eq!(eng2["priority"], "low");
    assert!(eng2["claim"].is_null());
    let eng3 = issue_by_key(&archive, "ENG-3");
    assert!(eng3["project_id"].is_null());
    assert_eq!(eng3["planning_scope"], "inbox");
    assert_eq!(eng3["external"]["archived_at"], "2026-01-07T00:00:00Z");
    assert_eq!(issue_by_key(&archive, "ENG-4")["status"], "canceled");
    assert!(
        issue_by_key(&archive, "ENG-4")["project_id"].is_null(),
        "cross-product project membership is not forced"
    );
    assert_eq!(
        issue_by_key(&archive, "OPS-1")["project_id"],
        "40000000-0000-4000-8000-00000000000c"
    );
    let shared = archive["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|project| project["id"] == "40000000-0000-4000-8000-00000000000c")
        .unwrap();
    assert_eq!(
        shared["product_id"], "10000000-0000-4000-8000-000000000002",
        "multi-team project goes to its majority team"
    );
    let beta = archive["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|project| project["id"] == "40000000-0000-4000-8000-00000000000b")
        .unwrap();
    assert_eq!(beta["status"], "completed");
    assert_eq!(
        beta["external_url"],
        "https://linear.app/example/project/40000000-0000-4000-8000-00000000000b"
    );
    // Initiative spanning ENG and OPS projects is split per product; orphan is unresolved.
    let goals = archive["goals"].as_array().unwrap();
    assert_eq!(goals.len(), 2);
    assert!(goals.iter().all(|goal| goal["name"] == "Growth"));
    assert_eq!(
        entry(&accounting, "data/initiatives.json", "i-orphan")["classification"],
        "unresolved"
    );
    let milestone_names: Vec<_> = archive["milestones"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap().to_lowercase())
        .collect();
    assert_eq!(
        milestone_names
            .iter()
            .filter(|name| name.starts_with("m1"))
            .count(),
        2
    );
    assert_eq!(report["imported_counts"]["owner_verified_done"], 0);
    assert_eq!(report["imported_counts"]["ready"], 0);
    assert_eq!(report["imported_counts"]["claims"], 0);
    assert_eq!(
        report["source_counts"]["history_entries"],
        report["imported_counts"]["history_entries"]
    );
    assert_eq!(report["source_counts"]["unprojected_issues"], 1);

    // C2: same-name labels stay distinct with origins, scope and a disambiguation report.
    let labels = archive["labels"].as_array().unwrap();
    assert_eq!(labels.len(), 5);
    let bugs: Vec<_> = labels
        .iter()
        .filter(|label| {
            label["linear_origins"][0]["name"]
                .as_str()
                .unwrap()
                .eq_ignore_ascii_case("bug")
        })
        .collect();
    assert_eq!(bugs.len(), 3);
    let mut bug_names: Vec<_> = bugs
        .iter()
        .map(|label| label["name"].as_str().unwrap())
        .collect();
    bug_names.sort();
    assert_eq!(bug_names, ["Bug (ENG)", "Bug (workspace)", "bug (OPS)"]);
    let workspace_bug = bugs
        .iter()
        .find(|label| label["name"] == "Bug (workspace)")
        .unwrap();
    assert!(workspace_bug["products"].as_array().unwrap().is_empty());
    assert_eq!(report["label_disambiguations"].as_array().unwrap().len(), 3);

    // C2: links across projects in a product, directions, dedupe, unsupported and cross-product.
    let links = archive["issue_links"].as_array().unwrap();
    let find = |kind: &str| {
        links
            .iter()
            .filter(|link| link["kind"] == kind)
            .collect::<Vec<_>>()
    };
    assert_eq!(find("parent").len(), 1);
    assert_eq!(
        (
            find("parent")[0]["source_key"].as_str(),
            find("parent")[0]["target_key"].as_str()
        ),
        (Some("ENG-2"), Some("ENG-1"))
    );
    assert_eq!(find("blocked_by").len(), 1);
    assert_eq!(
        (
            find("blocked_by")[0]["source_key"].as_str(),
            find("blocked_by")[0]["target_key"].as_str()
        ),
        (Some("ENG-2"), Some("ENG-1")),
        "ENG-1 blocks ENG-2 is stored on ENG-2"
    );
    assert_eq!(find("related").len(), 1);
    assert_eq!(links.len(), 3);
    let relation_class =
        |id: &str| entry(&accounting, "data/issue-relations.json", id)["classification"].clone();
    let represented = entry(
        &accounting,
        "data/issue-relations.json",
        "30000000-0000-4000-8000-000000000001",
    );
    assert_eq!(
        represented["direct"]["link"],
        "30000000-0000-4000-8000-000000000001"
    );
    assert_eq!(represented["direct"]["kind"], "blocked_by");
    assert_eq!(
        represented["classification"], "transformed",
        "relation updatedAt has no Direct field"
    );
    assert_eq!(
        relation_class("30000000-0000-4000-8000-000000000003"),
        "transformed",
        "semantic duplicate"
    );
    assert_eq!(
        relation_class("rel-dup-blocks"),
        "transformed",
        "duplicate blocks relation"
    );
    assert_eq!(
        relation_class("30000000-0000-4000-8000-000000000004"),
        "preserved",
        "duplicate-of has no Direct kind"
    );
    assert_eq!(
        relation_class("30000000-0000-4000-8000-000000000005"),
        "unresolved",
        "cross-product"
    );
    assert_eq!(
        relation_class("30000000-0000-4000-8000-000000000006"),
        "unresolved",
        "cycle"
    );
    assert_eq!(
        relation_class("30000000-0000-4000-8000-000000000008"),
        "preserved",
        "archived relation"
    );
    assert_eq!(
        report["relations"]["unsupported_semantics"][0]["type"],
        "duplicate"
    );
    assert_eq!(
        report["relations"]["deduplicated"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let cross: Vec<_> = report["cross_product_mappings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["kind"].as_str().unwrap().to_owned())
        .collect();
    for kind in [
        "multi_team_project",
        "multi_product_initiative",
        "issue_project_membership",
        "label_assignment",
        "relation",
    ] {
        assert!(
            cross.contains(&kind.to_owned()),
            "missing cross-product report {kind}: {cross:?}"
        );
    }
    assert!(report["cross_product_mappings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["relation_id"] == "30000000-0000-4000-8000-000000000005"));

    // C4: record/field accounting and honest readiness.
    assert_eq!(report["accounting"]["complete"], true);
    assert_eq!(report["accounting"]["records_in_source"], 48);
    assert_eq!(
        entry(&accounting, "data/users.json", "u-ada")["classification"],
        "preserved"
    );
    assert_eq!(
        entry(&accounting, "data/workflow-states.json", "s-eng-started")["classification"],
        "transformed"
    );
    assert_eq!(
        entry(&accounting, "data/documents.json", "d-1")["classification"],
        "preserved"
    );
    assert_eq!(
        entry(&accounting, "data/project-updates.json", "pu-1")["classification"],
        "preserved"
    );
    assert_eq!(
        entry(&accounting, "data/comments.json", "c-3")["classification"],
        "preserved"
    );
    assert_eq!(
        entry(&accounting, "data/comments.json", "c-2")["classification"],
        "transformed"
    );
    assert_eq!(
        entry(&accounting, "data/issues.json", "iss-eng-4")["classification"],
        "unresolved"
    );
    let eng3_entry = entry(&accounting, "data/issues.json", "iss-eng-3");
    assert_eq!(
        eng3_entry["classification"], "transformed",
        "populated fields without a Direct field make the record lossy"
    );
    assert!(eng3_entry["preserved_fields"]
        .as_array()
        .unwrap()
        .contains(&json!("dueDate")));
    let components = |kind: &str| {
        accounting["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["kind"] == kind)
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(components("previous_identifier").len(), 1);
    assert_eq!(
        components("previous_identifier")[0]["source_label"],
        "OPS-9"
    );
    assert_eq!(report["previous_identifiers"][0]["direct_key"], "ENG-1");
    let uploads = components("uploaded_file");
    assert_eq!(uploads.len(), 3);
    assert_eq!(
        uploads
            .iter()
            .filter(|upload| upload["classification"] == "unresolved")
            .count(),
        1
    );
    assert_eq!(
        components("label_assignment")
            .iter()
            .filter(|a| a["classification"] == "unresolved")
            .count(),
        1
    );
    let unsupported: Vec<_> = report["unsupported_fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| {
            format!(
                "{}.{}",
                field["kind"].as_str().unwrap(),
                field["field"].as_str().unwrap()
            )
        })
        .collect();
    for field in [
        "issue.dueDate",
        "issue.estimate",
        "project.content",
        "milestone.targetDate",
    ] {
        assert!(
            unsupported.contains(&field.to_owned()),
            "{field} not reported: {unsupported:?}"
        );
    }
    let codes = |list: &str| -> Vec<String> {
        report["cutover_readiness"][list]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["code"].as_str().unwrap().to_owned())
            .collect()
    };
    let blockers = codes("blockers");
    for code in [
        "final_capture_freshness",
        "live_application_pending",
        "owner_cutover_decision_required",
        "missing_source_bytes",
    ] {
        assert!(
            blockers.contains(&code.to_owned()),
            "missing blocker {code}"
        );
    }
    let review = codes("review_items");
    for code in [
        "unresolved_records",
        "unsupported_relation_semantics",
        "cross_product_mapping",
        "source_limitations",
        "unsupported_fields",
    ] {
        assert!(
            review.contains(&code.to_owned()),
            "missing review item {code}"
        );
    }
    let gates = &report["cutover_readiness"]["gates"];
    assert_eq!(gates["preservation"]["status"], "verified");
    assert_eq!(
        gates["access"]["status"], "incomplete",
        "one upload was never downloaded"
    );
    assert_eq!(gates["access"]["records"]["missing"], 1);
    assert_eq!(gates["freshness"]["status"], "unverified");
    assert_eq!(gates["application"]["status"], "not_applied");
    assert_eq!(report["native_access"]["status"], "incomplete");
    // The isolated workspace carries the retained bundle natively.
    assert_eq!(archive["format"], 21);
    assert_eq!(archive["source_bundles"].as_array().unwrap().len(), 1);
    assert_eq!(
        archive["source_files"].as_array().unwrap().len(),
        files_under(&source).len()
    );

    // C3: the retained bundle holds every captured file byte for byte.
    let source_files = files_under(&source);
    let bundle_files = files_under(&output.join("source-bundle"));
    assert_eq!(source_files, bundle_files);
    let sums = fs::read_to_string(output.join("source-bundle.sha256")).unwrap();
    assert_eq!(sums.lines().count(), source_files.len());
    for line in sums.lines() {
        let (digest, path) = line.split_once("  ").unwrap();
        assert_eq!(sha(&bundle_files[path]), digest);
    }

    // C3: the index is inert and escaped.
    let html = fs::read_to_string(output.join("index.html")).unwrap();
    assert!(html.contains("default-src 'none'"));
    assert!(
        !html.to_lowercase().contains("<script"),
        "raw script tag in index"
    );
    assert!(!html.contains("<img"), "raw source markup in index");
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(!html.contains("href=\"http"), "index links to a remote URL");
    assert!(!html.contains("src="), "index loads a resource");
    assert!(html.contains(&format!(
        "href=\"source-bundle/{}\"",
        fixture().upload_files[0].0
    )));
    assert!(html.contains("Cutover readiness: BLOCKED"));

    // C5: restart the isolated workspace through the service and read it back.
    let workspace = output.join("workspace");
    {
        let _service = serve(&workspace);
        let client = Client::new(&workspace).unwrap();
        let snapshot = client
            .call(
                &Request {
                    actor: "rehearsal-check".into(),
                    request_id: String::new(),
                    command: Command::Snapshot,
                },
                Role::Agent,
            )
            .unwrap();
        assert_eq!(snapshot["issues"].as_array().unwrap().len(), 7);
        assert_eq!(snapshot["issue_links"].as_array().unwrap().len(), 3);
        assert_eq!(snapshot["labels"].as_array().unwrap().len(), 5);
        let context = client
            .call(
                &Request {
                    actor: "rehearsal-check".into(),
                    request_id: String::new(),
                    command: Command::Context {
                        key: "ENG-2".into(),
                    },
                },
                Role::Agent,
            )
            .unwrap();
        assert_eq!(context["issue"]["status"], "backlog");
    }

    // C5: deterministic replay after the restart adds nothing and changes nothing.
    let before = files_under(&output);
    let second = succeeded(import(&data_dir, &source, &output));
    assert_eq!(second["idempotent_replay"], true);
    assert_eq!(second["duplicates_added"], 0);
    assert_eq!(second["archive_sha256"], first["archive_sha256"]);
    let after = files_under(&output);
    for (path, bytes) in &before {
        if !path.starts_with("workspace/") {
            assert_eq!(after.get(path), Some(bytes), "{path} changed on replay");
        }
    }

    // C5: recovery check restores the generated archive byte for byte.
    let recovered = succeeded(direct(
        &data_dir,
        &[
            "recovery-check",
            output.join("direct-import.json").to_str().unwrap(),
            temp.path().join("recovered").to_str().unwrap(),
        ],
    ));
    assert_eq!(recovered["byte_for_byte_archive_match"], true);
    assert_eq!(recovered["checksum_present_and_verified"], true);

    // C5: a changed source is refused instead of being merged into the output.
    let mut changed = fixture();
    changed.data.get_mut("issues.json").unwrap()[0]["title"] = json!("Changed upstream");
    let changed_source = temp.path().join("changed-capture");
    changed.write(&changed_source);
    let error = failed(import(&data_dir, &changed_source, &output));
    assert!(error.contains("different source"), "{error}");
    assert_eq!(
        files_under(&output).get("direct-import.json"),
        before.get("direct-import.json")
    );

    // C5: a tampered retained bundle is detected on replay.
    let tampered = temp.path().join("tampered");
    succeeded(import(&data_dir, &source, &tampered));
    fs::write(tampered.join("source-bundle/data/users.json"), b"{}\n").unwrap();
    let error = failed(import(&data_dir, &source, &tampered));
    assert!(
        error.contains("source bundle file data/users.json"),
        "{error}"
    );

    // C5: an incomplete output (no completion marker) is refused.
    let incomplete = temp.path().join("incomplete");
    succeeded(import(&data_dir, &source, &incomplete));
    fs::remove_file(incomplete.join("import-complete.json")).unwrap();
    let error = failed(import(&data_dir, &source, &incomplete));
    assert!(error.contains("incomplete"), "{error}");

    // C5: an edited isolated workspace is refused rather than overwritten.
    let edited = temp.path().join("edited");
    succeeded(import(&data_dir, &source, &edited));
    {
        let mut store = direct_core::Store::open(&edited.join("workspace/direct.db")).unwrap();
        store
            .execute(
                Request {
                    actor: "owner".into(),
                    request_id: "edit-1".into(),
                    command: Command::CreateIssue {
                        intake: None,
                        product: "ENG".into(),
                        title: "Added after import".into(),
                        body: String::new(),
                        acceptance: String::new(),
                        owner: String::new(),
                        priority: "medium".into(),
                        planning_scope: direct_core::PlanningScope::Inbox,
                        project_id: None,
                        template: None,
                        links: vec![],
                    },
                },
                Role::Human,
            )
            .unwrap();
    }
    let error = failed(import(&data_dir, &source, &edited));
    assert!(error.contains("isolated workspace differs"), "{error}");

    // No staging directories are left behind.
    assert!(fs::read_dir(temp.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains(".partial-")));
}

fn expect_rejected(name: &str, prepare: impl FnOnce(&Path), message: &str) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("capture");
    fixture().write(&source);
    prepare(&source);
    let output = temp.path().join("output");
    let error = failed(import(&temp.path().join("owner-data"), &source, &output));
    assert!(
        error.contains(message),
        "{name}: expected {message:?}, got {error}"
    );
    assert!(
        !output.exists(),
        "{name}: output was created for a rejected package"
    );
    assert!(
        fs::read_dir(temp.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".partial-")),
        "{name}: staging left behind"
    );
}

#[test]
fn unsafe_or_incomplete_packages_are_rejected_before_any_output() {
    for malformed in [
        json!({}),
        json!({"records": null}),
        json!({"records": [null]}),
        json!({"records": [], "count": "0"}),
    ] {
        expect_rejected(
            "malformed entity envelope",
            |root| {
                // Keep the manifest internally checksummed and omit its optional
                // count: malformed data must not become a successful empty import.
                let bytes = pretty(&malformed);
                fs::write(root.join("data/documents.json"), &bytes).unwrap();
                edit_manifest(root, |manifest| {
                    manifest["counts"]
                        .as_object_mut()
                        .unwrap()
                        .remove("documents");
                    let entry = manifest["integrity"]["data_files"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|entry| entry["path"] == "data/documents.json")
                        .unwrap();
                    entry["bytes"] = json!(bytes.len());
                    entry["sha256"] = json!(sha(&bytes));
                });
            },
            "data/documents.json",
        );
    }
    expect_rejected(
        "checksum corruption",
        |root| {
            let path = root.join("data/documents.json");
            let mut bytes = fs::read(&path).unwrap();
            bytes[10] ^= 1;
            fs::write(path, bytes).unwrap();
        },
        "integrity check failed for data/documents.json",
    );
    expect_rejected(
        "traversal path",
        |root| {
            fs::write(root.join("../escape.json"), b"{}").unwrap();
            edit_manifest(root, |manifest| {
                manifest["integrity"]["data_files"]
                    .as_array_mut()
                    .unwrap()
                    .push(
                        json!({"path": "data/../../escape.json", "bytes": 2, "sha256": sha(b"{}")}),
                    );
            });
        },
        "Unsafe path",
    );
    expect_rejected(
        "missing attachment bytes",
        |root| {
            let path = fixture().upload_files[0].0.clone();
            fs::remove_file(root.join(path)).unwrap();
        },
        "is missing",
    );
    expect_rejected(
        "capture error",
        |root| {
            edit_manifest(root, |manifest| {
                manifest["errors"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"scope": "issues.labels capture", "message": "rate limited"}));
            });
        },
        "issues.labels capture",
    );
    expect_rejected(
        "truncated nested connection",
        |root| {
            let mut package = fixture();
            package.data.get_mut("issues.json").unwrap()[0]["labels"]["pageInfo"]["hasNextPage"] =
                json!(true);
            fs::remove_dir_all(root).unwrap();
            package.write(root);
        },
        "truncated nested connection",
    );
    expect_rejected(
        "coverage reports truncation",
        |root| {
            edit_manifest(root, |manifest| {
                manifest["query_coverage"][0]["truncated_nested_connections"] =
                    json!(["issues.ENG-1.history"]);
            });
        },
        "truncated_nested_connections",
    );
    expect_rejected(
        "unlisted data file",
        |root| fs::write(root.join("data/extra.json"), b"{}").unwrap(),
        "unlisted file data/extra.json",
    );
    expect_rejected(
        "upload manifest disagreement",
        |root| {
            let mut uploads: Value =
                serde_json::from_slice(&fs::read(root.join("attachment-manifest.json")).unwrap())
                    .unwrap();
            uploads[0]["sha256"] = json!("0".repeat(64));
            fs::write(root.join("attachment-manifest.json"), pretty(&uploads)).unwrap();
        },
        "does not match the manifest integrity list",
    );
    expect_rejected(
        "missing required record type",
        |root| {
            fs::remove_file(root.join("data/users.json")).unwrap();
        },
        "data/users.json",
    );
    #[cfg(unix)]
    expect_rejected(
        "symlinked data file",
        |root| {
            let path = root.join("data/documents.json");
            let real = root.join("../documents-real.json");
            fs::rename(&path, &real).unwrap();
            std::os::unix::fs::symlink(&real, &path).unwrap();
        },
        "symlink",
    );
}

#[cfg(any(unix, windows))]
fn link_directory(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        // Junctions are available without symlink privileges on Windows.
        let result = direct::hidden(std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command",
                "$ErrorActionPreference = 'Stop'; New-Item -ItemType Junction -Path $env:DIRECT_TEST_LINK -Target $env:DIRECT_TEST_TARGET | Out-Null"])
            .env("DIRECT_TEST_LINK", link)
            .env("DIRECT_TEST_TARGET", target))
            .output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[cfg(any(unix, windows))]
#[test]
fn replay_rejects_redirected_workspace_before_opening_another_database() {
    for whole in [true, false] {
        for redirect_output in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("capture");
            fixture().write(&source);
            let output = temp.path().join("rehearsal");
            let data_dir = temp.path().join("owner-data");
            let run = |output: &Path| {
                if whole {
                    import(&data_dir, &source, output)
                } else {
                    direct(
                        &data_dir,
                        &[
                            "linear-import-dry-run",
                            "--project-id",
                            "40000000-0000-4000-8000-00000000000b",
                            "--source",
                            source.to_str().unwrap(),
                            "--output",
                            output.to_str().unwrap(),
                        ],
                    )
                }
            };
            succeeded(run(&output));
            let external = temp.path().join("external");
            let link = if redirect_output {
                fs::rename(&output, &external).unwrap();
                output.clone()
            } else {
                fs::rename(output.join("workspace"), &external).unwrap();
                output.join("workspace")
            };
            link_directory(&external, &link);
            // An untouched empty database is a strong sentinel: Store::open
            // would initialize it even if the later archive comparison failed.
            let database = if redirect_output {
                external.join("workspace/direct.db")
            } else {
                external.join("direct.db")
            };
            fs::write(&database, []).unwrap();
            let before = files_under(&external);
            let error = failed(run(&output));
            assert!(
                error.contains("symlink") || error.contains("reparse"),
                "{error}"
            );
            assert_eq!(before, files_under(&external));
            // Remove only the link, before TempDir recursively cleans its tree.
            #[cfg(windows)]
            fs::remove_dir(&link).unwrap();
            #[cfg(unix)]
            fs::remove_file(&link).unwrap();
        }
    }
}

#[cfg(unix)]
#[test]
fn replay_rejects_redirected_database_and_sqlite_sidecars() {
    for name in [
        "direct.db",
        "direct.db-wal",
        "direct.db-shm",
        "direct.db-journal",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("capture");
        fixture().write(&source);
        let output = temp.path().join("rehearsal");
        let data_dir = temp.path().join("owner-data");
        succeeded(import(&data_dir, &source, &output));
        let external = temp.path().join("external-sentinel");
        fs::write(&external, b"untouched").unwrap();
        let link = output.join("workspace").join(name);
        if link.exists() {
            fs::remove_file(&link).unwrap();
        }
        std::os::unix::fs::symlink(&external, &link).unwrap();
        let error = failed(import(&data_dir, &source, &output));
        assert!(error.contains("symlink"), "{error}");
        assert_eq!(fs::read(&external).unwrap(), b"untouched");
    }
}

#[test]
fn cli_modes_and_output_guards() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join("owner-data");
    let source = temp.path().join("capture");
    fixture().write(&source);
    let source_arg = source.to_str().unwrap();

    // Exactly one mode is required.
    let both = temp.path().join("both");
    failed(direct(
        &data_dir,
        &[
            "linear-import-dry-run",
            "--whole-workspace",
            "--project-id",
            "40000000-0000-4000-8000-00000000000b",
            "--source",
            source_arg,
            "--output",
            both.to_str().unwrap(),
        ],
    ));
    failed(direct(
        &data_dir,
        &[
            "linear-import-dry-run",
            "--source",
            source_arg,
            "--output",
            both.to_str().unwrap(),
        ],
    ));
    assert!(!both.exists());

    // The bounded single-project mode remains available beside the whole-workspace mode.
    let project = temp.path().join("project");
    let single = succeeded(direct(
        &data_dir,
        &[
            "linear-import-dry-run",
            "--project-id",
            "40000000-0000-4000-8000-00000000000b",
            "--source",
            source_arg,
            "--output",
            project.to_str().unwrap(),
        ],
    ));
    assert_eq!(single["cutover_ready"], false);
    assert_eq!(single["reconciliation"]["imported_counts"]["issues"], 1);

    // Output inside the owner's data directory is refused.
    fs::create_dir_all(&data_dir).unwrap();
    let inside = data_dir.join("import");
    let error = failed(import(&data_dir, &source, &inside));
    assert!(error.contains("Direct data directory"), "{error}");
    assert!(!inside.exists());

    // Output inside the repository (the test's working directory) is refused.
    let repository_output: PathBuf = std::env::current_dir()
        .unwrap()
        .join("target-linear-import-should-not-exist");
    let error = failed(import(&data_dir, &source, &repository_output));
    assert!(error.contains("inside the repository"), "{error}");
    assert!(!repository_output.exists());

    // Output overlapping the source package is refused.
    let error = failed(import(&data_dir, &source, &source.join("out")));
    assert!(
        error.contains("repository") || error.contains("overlap"),
        "{error}"
    );
}
