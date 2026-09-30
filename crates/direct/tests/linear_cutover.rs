//! Cutover into an EXISTING, non-empty workspace through the real CLI and
//! service: seed real history → owner export → prepare → owner preview/apply →
//! compare every existing record → native retained-source reads and downloads
//! → restart/export/recovery → exact replay → rollback → refusals. Synthetic data.

#[path = "support/linear_fixture.rs"]
mod support;

use direct::Client;
use direct_core::{encode_migration_artifact, parse_migration_artifact, Archive, Request, Role};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use support::*;

const ENG_TEAM: &str = "10000000-0000-4000-8000-000000000001";
const OPS_TEAM: &str = "10000000-0000-4000-8000-000000000002";

fn req(value: Value) -> Request {
    let mut value = value;
    value["actor"] = json!("seed");
    value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    serde_json::from_value(value).unwrap()
}

fn call(client: &Client, value: Value, role: Role) -> Value {
    client.call(&req(value), role).unwrap()
}

fn version(client: &Client, key: &str) -> u64 {
    call(client, json!({"op": "context", "key": key}), Role::Agent)["issue"]["version"]
        .as_u64()
        .unwrap()
}

fn export(client: &Client) -> Archive {
    serde_json::from_value(call(client, json!({"op": "export"}), Role::Agent)).unwrap()
}

/// A realistic existing workspace: several products, planning records, labels,
/// owner-verified Done work, pending verification, an active claim, Git
/// evidence, links, comments and request replays.
fn seed(client: &Client) -> (String, String) {
    // Like the real adoption: an existing product per Linear team, same key,
    // verified repository paths, and no issues yet.
    let eng = call(
        client,
        json!({"op": "create_product", "key": "ENG", "name": "Engineering (adopted)", "repo_windows": "C:\\src\\eng", "repo_wsl": "/mnt/c/src/eng"}),
        Role::Human,
    );
    let ops = call(
        client,
        json!({"op": "create_product", "key": "OPS", "name": "Operations (existing)"}),
        Role::Human,
    );
    call(
        client,
        json!({"op": "create_product", "key": "SITE", "name": "Website"}),
        Role::Human,
    );
    for title in ["Existing ops 1", "Existing ops 2", "Existing ops 3"] {
        call(
            client,
            json!({"op": "create_issue", "product": "OPS", "title": title}),
            Role::Agent,
        );
    }
    let project = call(
        client,
        json!({"op": "create_project", "product": "DIR", "name": "Pilot"}),
        Role::Human,
    );
    call(
        client,
        json!({"op": "create_goal", "product": "DIR", "name": "Adopt Direct", "project_ids": [project["id"]]}),
        Role::Human,
    );
    call(
        client,
        json!({"op": "create_milestone", "project_id": project["id"], "name": "M1"}),
        Role::Human,
    );
    let bug = call(
        client,
        json!({"op": "create_label", "name": "Bug", "color": "#d73a49"}),
        Role::Human,
    );
    call(
        client,
        json!({"op": "create_label", "name": "Feature", "linear_origins": [{"id": "20000000-0000-4000-8000-000000000004", "name": "Feature <b>"}]}),
        Role::Human,
    );
    for title in [
        "Verified work",
        "Pending verification",
        "Claimed work",
        "Backlog work",
    ] {
        call(
            client,
            json!({"op": "create_issue", "product": "DIR", "title": title, "body": "Seeded"}),
            Role::Agent,
        );
    }
    for key in ["DIR-1", "DIR-2", "DIR-3"] {
        let v = version(client, key);
        call(
            client,
            json!({"op": "update_issue", "key": key, "expected_version": v, "title": key, "body": "Seeded", "acceptance": "Works", "owner": "owner", "priority": "high"}),
            Role::Agent,
        );
        let v = version(client, key);
        call(
            client,
            json!({"op": "ready", "key": key, "expected_version": v}),
            Role::Human,
        );
        let v = version(client, key);
        call(
            client,
            json!({"op": "claim", "key": key, "expected_version": v, "lease_seconds": 7200}),
            Role::Agent,
        );
    }
    let v = version(client, "DIR-3");
    call(
        client,
        json!({"op": "record_git_trace", "key": "DIR-3", "expected_version": v, "kind": "commit", "repository": "example/direct", "commit_sha": "0123456789abcdef0123456789abcdef01234567", "branch": "work"}),
        Role::Agent,
    );
    let build = "commit:0123456789abcdef0123456789abcdef01234567";
    for key in ["DIR-1", "DIR-2"] {
        let v = version(client, key);
        call(
            client,
            json!({"op": "submit", "key": key, "expected_version": v, "build_ref": build, "delivery_ref": "branch", "summary": "Done", "checks": "Passed",
            "e2e": {"build_ref": build, "delivered_build_ref": build, "environment": "fixture", "entrypoint": "fixture", "scenarios": "all", "outcome": "passed", "delivery_check": "installed"},
            "steps": [{"instruction": "Look", "expected": "It works"}]}),
            Role::Agent,
        );
    }
    let context = call(
        client,
        json!({"op": "context", "key": "DIR-1"}),
        Role::Agent,
    );
    let run = context["issue"]["current_run"].clone();
    call(
        client,
        json!({"op": "review", "key": "DIR-1", "expected_version": context["issue"]["version"], "run_id": run, "outcome": "passed", "results": [{"outcome": "passed", "note": "Looks right"}], "note": "Accepted by the owner"}),
        Role::Human,
    );
    let v = version(client, "DIR-4");
    call(
        client,
        json!({"op": "comment", "key": "DIR-4", "expected_version": v, "body": "Existing discussion"}),
        Role::Agent,
    );
    let v = version(client, "DIR-4");
    call(
        client,
        json!({"op": "attach_issue_label", "key": "DIR-4", "expected_version": v, "label_id": bug["id"]}),
        Role::Agent,
    );
    let v = version(client, "DIR-4");
    call(
        client,
        json!({"op": "create_issue_link", "key": "DIR-4", "expected_version": v, "target_key": "DIR-1", "kind": "related"}),
        Role::Agent,
    );
    (
        eng["id"].as_str().unwrap().to_owned(),
        ops["id"].as_str().unwrap().to_owned(),
    )
}

fn backup(data_dir: &Path, directory: &Path) -> PathBuf {
    let report = succeeded(direct(data_dir, &["backup", directory.to_str().unwrap()]));
    PathBuf::from(report["archive"].as_str().unwrap())
}

fn prepare(
    data_dir: &Path,
    source: &Path,
    target: &Path,
    output: &Path,
    maps: &[String],
) -> std::process::Output {
    let mut args = vec![
        "linear-migration-prepare".to_owned(),
        "--source".into(),
        source.display().to_string(),
        "--target-export".into(),
        target.display().to_string(),
        "--output".into(),
        output.display().to_string(),
    ];
    for map in maps {
        args.push("--map-team".into());
        args.push(map.clone());
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    direct(data_dir, &args)
}

/// Every record that existed in `before` is present and byte-identical in `after`.
fn assert_existing_preserved(before: &Archive, after: &Archive) {
    let before = serde_json::to_value(before).unwrap();
    let after = serde_json::to_value(after).unwrap();
    assert_eq!(before["workspace_id"], after["workspace_id"]);
    for (collection, key) in [
        ("products", "id"),
        ("projects", "id"),
        ("goals", "id"),
        ("milestones", "id"),
        ("labels", "id"),
        ("theoria_documents", "id"),
        ("method_findings", "id"),
        ("git_traces", "id"),
        ("releases", "id"),
        ("release_evidence", "id"),
        ("release_workflows", "product_id"),
        ("issue_links", "id"),
        ("issues", "id"),
        ("comments", "id"),
        ("verifications", "id"),
        ("events", "seq"),
        ("requests", "id"),
        ("source_bundles", "id"),
        ("source_records", "id"),
    ] {
        let now: HashMap<String, &Value> = after[collection]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| (item[key].to_string(), item))
            .collect();
        for item in before[collection].as_array().unwrap() {
            assert_eq!(
                now.get(&item[key].to_string()),
                Some(&item),
                "{collection} {} changed or vanished",
                item[key]
            );
        }
    }
}

fn http_status(port: u16, path: &str, token: Option<&str>, body: Vec<u8>) -> u16 {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .unwrap();
    let mut request = client
        .post(format!("http://127.0.0.1:{port}{path}"))
        .header("content-type", "application/json")
        .body(body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request.send().unwrap().status().as_u16()
}

#[test]
fn prepared_migration_merges_into_existing_workspace_with_native_retained_access() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let source = temp.path().join("capture");
    fixture().write(&source);
    let service = serve(&workspace);
    let client = Client::new(&workspace).unwrap();
    let (eng_product, ops_product) = seed(&client);
    let seeded = export(&client);
    assert_eq!(seeded.verifications.len(), 2);
    assert!(seeded.issues.iter().any(|issue| issue.claim.is_some()));

    // Preparation binds to a fresh owner export and never opens the live database.
    let exports = temp.path().join("exports");
    let target = backup(&workspace, &exports);
    let without_map = prepare(
        &workspace,
        &source,
        &target,
        &temp.path().join("refused"),
        &[],
    );
    let error = failed(without_map);
    assert!(
        error.contains("collides with existing Direct product") && error.contains("--map-team"),
        "{error}"
    );
    assert!(!temp.path().join("refused").exists());

    let prepared_dir = temp.path().join("prepared");
    let maps = vec![
        format!("{ENG_TEAM}={eng_product}"),
        format!("{OPS_TEAM}={ops_product}"),
    ];
    let prepared = succeeded(prepare(&workspace, &source, &target, &prepared_dir, &maps));
    assert_eq!(prepared["rehearsal"]["status"], "passed", "{prepared}");
    assert_eq!(
        prepared["rehearsal"]["existing_records_unchanged_after_apply"],
        true
    );
    assert_eq!(
        prepared["rehearsal"]["rollback_restored_existing_state"],
        true
    );
    assert_eq!(prepared["cutover_readiness"]["status"], "not_complete");
    assert_eq!(
        prepared["cutover_readiness"]["gates"]["application"]["status"],
        "prepared"
    );
    assert_eq!(
        prepared["cutover_readiness"]["gates"]["freshness"]["status"],
        "unverified"
    );
    let again = succeeded(prepare(&workspace, &source, &target, &prepared_dir, &maps));
    assert_eq!(again["idempotent_replay"], true);
    assert_eq!(again["artifact_sha256"], prepared["artifact_sha256"]);
    let artifact = fs::read(prepared_dir.join("linear-migration.direct-migration")).unwrap();
    let digest = prepared["artifact_sha256"].as_str().unwrap().to_owned();
    let report: Value =
        serde_json::from_slice(&fs::read(prepared_dir.join("reconciliation.json")).unwrap())
            .unwrap();
    let renumbered: Vec<_> = report["previous_identifiers"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["reason"].is_string())
        .collect();
    assert_eq!(
        renumbered.len(),
        3,
        "OPS-1..3 collide with existing keys: {renumbered:?}"
    );
    assert!(renumbered
        .iter()
        .all(|item| item["direct_key"].as_str().unwrap()[4..]
            .parse::<u64>()
            .unwrap()
            > 3));

    // Capability checks: agents and anonymous callers cannot preview or apply.
    let port = client.endpoint.port;
    assert_eq!(
        http_status(port, "/api/migration/preview", None, artifact.clone()),
        401
    );
    assert_eq!(
        http_status(
            port,
            "/api/migration/preview",
            Some(&client.endpoint.agent_token),
            artifact.clone()
        ),
        403
    );
    assert!(client
        .migration(artifact.clone(), None, Role::Agent)
        .is_err());

    // Refusals leave the workspace byte-identical.
    let before = export(&client);
    let check_unchanged = |label: &str| {
        let now = export(&client);
        assert_eq!(
            serde_json::to_value(&now).unwrap(),
            serde_json::to_value(&before).unwrap(),
            "{label} changed the workspace"
        );
    };
    let (header, payload) = parse_migration_artifact(&artifact).unwrap();
    let mut tampered = artifact.clone();
    *tampered.last_mut().unwrap() ^= 1;
    let error = client
        .migration(tampered, None, Role::Human)
        .unwrap_err()
        .to_string();
    assert!(error.contains("checksum"), "{error}");
    let mut traversal = header.clone();
    traversal.files[0].path = "../../outside.json".into();
    let error = client
        .migration(
            encode_migration_artifact(&traversal, payload).unwrap(),
            None,
            Role::Human,
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("invalid"), "{error}");
    let mut foreign = header.clone();
    foreign.target_workspace_id = uuid::Uuid::new_v4().to_string();
    let error = client
        .migration(
            encode_migration_artifact(&foreign, payload).unwrap(),
            None,
            Role::Human,
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("prepared for workspace"), "{error}");
    let mut hijack = header.clone();
    hijack.records[0].issue_keys = vec!["DIR-1".into()];
    let error = client
        .migration(
            encode_migration_artifact(&hijack, payload).unwrap(),
            None,
            Role::Human,
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("outside the import"), "{error}");
    let mut claimed = header.clone();
    claimed.issues[0].status = direct_core::Status::Ready;
    let error = client
        .migration(
            encode_migration_artifact(&claimed, payload).unwrap(),
            None,
            Role::Human,
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("readiness"), "{error}");
    let preview = client
        .migration(artifact.clone(), None, Role::Human)
        .unwrap();
    assert_eq!(preview["status"], "ready_to_apply");
    assert_eq!(preview["artifact_sha256"], digest);
    let cursor = preview["expected_cursor"].as_u64().unwrap();
    assert!(client
        .migration(artifact.clone(), Some((cursor + 1, &digest)), Role::Human)
        .unwrap_err()
        .to_string()
        .contains("changed since preview"));
    assert!(client
        .migration(
            artifact.clone(),
            Some((cursor, &"0".repeat(64))),
            Role::Human
        )
        .unwrap_err()
        .to_string()
        .contains("differs from the previewed"));
    assert!(client
        .migration(artifact.clone(), Some((cursor, &digest)), Role::Agent)
        .is_err());
    check_unchanged("refused previews and applies");
    let backups = workspace.join("migration-backups");
    assert!(
        !backups.exists() || fs::read_dir(&backups).unwrap().next().is_none(),
        "failed applies leave no backup"
    );

    // Apply: one transaction, pre-import backup, every existing record preserved.
    let applied = client
        .migration(artifact.clone(), Some((cursor, &digest)), Role::Human)
        .unwrap();
    assert_eq!(applied["status"], "applied", "{applied}");
    let backup_path = PathBuf::from(applied["backup_path"].as_str().unwrap());
    let pre_import: Archive = serde_json::from_slice(&fs::read(&backup_path).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(&pre_import).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    assert!(backup_path.with_extension("sha256").is_file());
    let after = export(&client);
    assert_existing_preserved(&before, &after);
    assert_eq!(after.issues.len(), before.issues.len() + 7);
    assert_eq!(
        after.verifications.len(),
        before.verifications.len(),
        "no verification runs are fabricated"
    );
    assert_eq!(
        after.products.len(),
        before.products.len() + 1,
        "only WEB is new; ENG and OPS map to existing products"
    );
    let eng_issues: Vec<_> = after
        .issues
        .iter()
        .filter(|issue| issue.product_id == eng_product)
        .map(|issue| issue.key.as_str())
        .collect();
    assert_eq!(
        eng_issues.len(),
        4,
        "ENG issues land in the existing product"
    );
    assert!(
        ["ENG-1", "ENG-2", "ENG-3", "ENG-4"]
            .iter()
            .all(|key| eng_issues.contains(key)),
        "identifiers are kept when the existing product's key space is free"
    );
    let imported: Vec<_> = after
        .issues
        .iter()
        .filter(|issue| issue.external.is_some())
        .collect();
    assert_eq!(imported.len(), 7);
    for issue in &imported {
        assert!(
            issue.claim.is_none() && issue.current_run.is_none(),
            "{} gained a claim or run",
            issue.key
        );
        assert!(
            matches!(
                issue.status,
                direct_core::Status::Backlog
                    | direct_core::Status::LegacyCompleted
                    | direct_core::Status::Canceled
            ),
            "{} is {:?}",
            issue.key,
            issue.status
        );
    }
    let ops_issues: Vec<_> = imported
        .iter()
        .filter(|issue| issue.product_id == ops_product)
        .collect();
    assert_eq!(ops_issues.len(), 3);
    let eng1 = after
        .issues
        .iter()
        .find(|issue| issue.key == "ENG-1")
        .unwrap();
    assert_eq!(eng1.status, direct_core::Status::LegacyCompleted);
    assert_eq!(
        eng1.external.as_ref().unwrap().history.len(),
        2,
        "external history is preserved in export"
    );
    let feature = after
        .labels
        .iter()
        .find(|label| label.name == "Feature")
        .unwrap();
    assert!(
        eng1.labels.contains(&feature.id),
        "existing label matched by Linear origin is reused"
    );
    assert!(
        after.labels.iter().any(|label| label.name == "Bug (ENG)"),
        "name collision with the existing Bug label is disambiguated"
    );

    // Native retained access through the authenticated service.
    let snapshot = call(&client, json!({"op": "snapshot"}), Role::Agent);
    assert_eq!(snapshot["source_bundles"].as_array().unwrap().len(), 1);
    let bundle_id = snapshot["source_bundles"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        snapshot["source_bundles"][0].get("summary").is_none(),
        "snapshot carries metadata only"
    );
    let polled = snapshot["issues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|issue| issue["key"] == "ENG-1")
        .unwrap();
    assert_eq!(polled["external"]["history"], json!([]));
    assert_eq!(polled["external"]["history_entries"], 2);
    let context = call(
        &client,
        json!({"op": "context", "key": "ENG-1"}),
        Role::Agent,
    );
    assert_eq!(
        context["issue"]["external"]["history"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let linked: Vec<_> = context["retained_sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["kind"].as_str().unwrap().to_owned())
        .collect();
    for kind in [
        "issue",
        "comment",
        "uploaded_file",
        "link_attachment",
        "relation",
        "previous_identifier",
    ] {
        assert!(
            linked.contains(&kind.to_owned()),
            "ENG-1 context lacks retained {kind}: {linked:?}"
        );
    }
    let bundles = call(&client, json!({"op": "source_bundles"}), Role::Human);
    assert_eq!(
        bundles["bundles"][0]["application"]["rollback_available"],
        true
    );
    assert_eq!(
        bundles["bundles"][0]["summary"]["cutover_readiness"]["gates"]["preservation"]["status"],
        "verified"
    );
    let search = call(
        &client,
        json!({"op": "search_sources", "query": "spec", "kind": "document"}),
        Role::Human,
    );
    assert_eq!(search["total"], 1);
    let document = call(
        &client,
        json!({"op": "source_record", "id": search["records"][0]["id"]}),
        Role::Human,
    );
    let text = document["readable"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["field"] == "content")
        .unwrap();
    assert_eq!(
        text["text"], "# Spec\n<script>bad()</script>\n",
        "document text is returned verbatim as data"
    );
    assert!(document["authority"]
        .as_str()
        .unwrap()
        .contains("not instructions"));
    let duplicate = call(
        &client,
        json!({"op": "search_sources", "query": "30000000-0000-4000-8000-000000000004"}),
        Role::Human,
    );
    let relation = call(
        &client,
        json!({"op": "source_record", "id": duplicate["records"][0]["id"]}),
        Role::Agent,
    );
    assert_eq!(relation["record"]["access"], "retained");
    assert!(relation["content"]
        .as_str()
        .unwrap()
        .contains("\"duplicate\""));
    let by_old_key = call(
        &client,
        json!({"op": "search_sources", "query": "OPS-2", "kind": "issue"}),
        Role::Human,
    );
    assert!(
        by_old_key["total"].as_u64().unwrap() >= 1,
        "original Linear identifiers stay searchable"
    );
    let (upload_path, upload_bytes) = fixture().upload_files[0].clone();
    assert_eq!(
        client
            .source_file(&bundle_id, &upload_path, Role::Human)
            .unwrap(),
        upload_bytes
    );
    assert_eq!(
        client
            .source_file(&bundle_id, &upload_path, Role::Agent)
            .unwrap(),
        upload_bytes
    );
    assert_eq!(
        client
            .source_file(&bundle_id, "data/issues.json", Role::Human)
            .unwrap(),
        fs::read(source.join("data/issues.json")).unwrap()
    );
    for path in [
        "../manifest.json",
        "/etc/passwd",
        "data/../manifest.json",
        "C:/Windows/win.ini",
    ] {
        assert!(
            client
                .source_file(&bundle_id, path, Role::Human)
                .unwrap_err()
                .to_string()
                .contains("not_found"),
            "{path}"
        );
    }
    let body = serde_json::to_vec(&json!({"bundle_id": bundle_id, "path": upload_path})).unwrap();
    assert_eq!(
        http_status(port, "/api/source-file", None, body.clone()),
        401
    );
    assert_eq!(
        http_status(port, "/api/source-file", Some("wrong"), body),
        401
    );

    // Exact replay: zero duplicates, nothing written.
    let preview = client
        .migration(artifact.clone(), None, Role::Human)
        .unwrap();
    assert_eq!(preview["status"], "already_applied");
    let replay = client
        .migration(
            artifact.clone(),
            Some((preview["expected_cursor"].as_u64().unwrap(), &digest)),
            Role::Human,
        )
        .unwrap();
    assert_eq!(replay["status"], "already_applied");
    assert_eq!(replay["duplicates_added"], 0);
    assert_eq!(
        serde_json::to_value(export(&client)).unwrap(),
        serde_json::to_value(&after).unwrap()
    );

    // Restart, export/backup and recovery keep the retained bundle byte for byte.
    drop(service);
    let service = serve(&workspace);
    let client = Client::new(&workspace).unwrap();
    assert_eq!(
        serde_json::to_value(export(&client)).unwrap(),
        serde_json::to_value(&after).unwrap()
    );
    let post_backup = backup(&workspace, &exports);
    let recovered = succeeded(direct(
        &workspace,
        &[
            "recovery-check",
            post_backup.to_str().unwrap(),
            temp.path().join("recovered").to_str().unwrap(),
        ],
    ));
    assert_eq!(recovered["byte_for_byte_archive_match"], true);
    assert_eq!(recovered["retained_sources"]["bundles"], 1);
    let mut corrupt: Value = serde_json::from_slice(&fs::read(&post_backup).unwrap()).unwrap();
    corrupt["source_files"][0]["data"] = json!("AAAA");
    let corrupt_path = temp.path().join("corrupt.json");
    fs::write(&corrupt_path, serde_json::to_vec(&corrupt).unwrap()).unwrap();
    let error = failed(direct(
        &workspace,
        &[
            "recovery-check",
            corrupt_path.to_str().unwrap(),
            temp.path().join("corrupt-restore").to_str().unwrap(),
        ],
    ));
    assert!(error.contains("checksum"), "{error}");

    // Rollback: owner only, exact while nothing changed since the import.
    let cursor = call(&client, json!({"op": "snapshot"}), Role::Agent)["cursor"]
        .as_u64()
        .unwrap();
    assert!(client.call(&req(json!({"op": "rollback_migration", "bundle_id": bundle_id, "expected_cursor": cursor})), Role::Agent).unwrap_err().to_string().starts_with("forbidden"));
    let rolled = call(
        &client,
        json!({"op": "rollback_migration", "bundle_id": bundle_id, "expected_cursor": cursor}),
        Role::Human,
    );
    assert_eq!(rolled["status"], "rolled_back");
    let rolled_back = export(&client);
    assert_existing_preserved(&before, &rolled_back);
    assert_eq!(rolled_back.issues.len(), before.issues.len());
    assert_eq!(rolled_back.products.len(), before.products.len());
    assert!(rolled_back.source_bundles.is_empty() && rolled_back.source_files.is_empty());
    assert_eq!(
        rolled_back.events.len(),
        before.events.len() + 1,
        "only the rollback event is added"
    );

    // Re-apply, then an edit to an imported record blocks replay and rollback.
    let preview = client
        .migration(artifact.clone(), None, Role::Human)
        .unwrap();
    assert_eq!(preview["status"], "ready_to_apply");
    let reapplied = client
        .migration(
            artifact.clone(),
            Some((preview["expected_cursor"].as_u64().unwrap(), &digest)),
            Role::Human,
        )
        .unwrap();
    assert_eq!(reapplied["status"], "applied");
    let v = version(&client, "ENG-2");
    call(
        &client,
        json!({"op": "update_issue", "key": "ENG-2", "expected_version": v, "title": "Edited after import", "body": "", "acceptance": "", "owner": "", "priority": "low"}),
        Role::Agent,
    );
    let error = client
        .migration(artifact.clone(), None, Role::Human)
        .unwrap_err()
        .to_string();
    assert!(error.contains("changed since"), "{error}");
    let cursor = call(&client, json!({"op": "snapshot"}), Role::Agent)["cursor"]
        .as_u64()
        .unwrap();
    let error = client.call(&req(json!({"op": "rollback_migration", "bundle_id": bundle_id, "expected_cursor": cursor})), Role::Human).unwrap_err().to_string();
    assert!(error.contains("pre-import backup"), "{error}");

    // A changed source prepared against the old export is refused, not merged.
    let mut changed = fixture();
    changed.data.get_mut("issues.json").unwrap()[0]["title"] = json!("Changed upstream");
    let changed_source = temp.path().join("changed-capture");
    changed.write(&changed_source);
    let changed_dir = temp.path().join("changed-prepared");
    succeeded(prepare(
        &workspace,
        &changed_source,
        &target,
        &changed_dir,
        &maps,
    ));
    let changed_artifact = fs::read(changed_dir.join("linear-migration.direct-migration")).unwrap();
    let current = export(&client);
    let error = client
        .migration(changed_artifact, None, Role::Human)
        .unwrap_err()
        .to_string();
    assert!(error.contains("conflict"), "{error}");
    assert_eq!(
        serde_json::to_value(export(&client)).unwrap(),
        serde_json::to_value(&current).unwrap()
    );
    drop(service);
}
