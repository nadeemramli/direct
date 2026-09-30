//! End-to-end rehearsal of the whole-workspace Linear import through the real
//! CLI: package verification → isolated database → export → service restart →
//! recovery check → replay. All data is synthetic.

use direct::Client;
use direct_core::{Command, Request, Role};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command as Process, Output, Stdio},
    thread,
    time::Duration,
};

const CREATED: &str = "2026-01-02T03:04:05.678Z";

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn pretty(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).unwrap();
    bytes.push(b'\n');
    bytes
}

/// A synthetic capture package laid out exactly like `capture-linear.mjs` output.
#[derive(Clone)]
struct Package {
    data: BTreeMap<&'static str, Value>,
    uploads: Vec<Value>,
    upload_files: Vec<(String, Vec<u8>)>,
    errors: Vec<Value>,
}

fn team_key(id: &str) -> &'static str {
    match id.chars().last() {
        Some('1') => "ENG",
        Some('2') => "OPS",
        _ => "WEB",
    }
}

fn team(id: &str, key: &str, name: &str) -> Value {
    json!({"id": id, "key": key, "name": name})
}

fn issue(id: &str, identifier: &str, team_id: &str, team_key: &str, state: (&str, &str)) -> Value {
    json!({
        "id": id,
        "identifier": identifier,
        "number": identifier.split('-').nth(1).unwrap().parse::<u64>().unwrap(),
        "title": format!("Issue {identifier}"),
        "description": format!("Body of {identifier}"),
        "priority": 3,
        "createdAt": CREATED,
        "updatedAt": "2026-01-03T00:00:00Z",
        "startedAt": null,
        "completedAt": null,
        "canceledAt": null,
        "archivedAt": null,
        "dueDate": null,
        "estimate": null,
        "previousIdentifiers": [],
        "url": format!("https://linear.app/example/issue/{identifier}"),
        "team": {"id": team_id, "key": team_key, "name": team_key},
        "state": {"id": state.0, "name": state.0, "type": state.1},
        "project": null,
        "projectMilestone": null,
        "parent": null,
        "assignee": null,
        "labels": {"nodes": [], "pageInfo": {"hasNextPage": false, "endCursor": null}},
        "history": {"nodes": [], "pageInfo": {"hasNextPage": false, "endCursor": null}},
    })
}

fn labels(ids: &[&str]) -> Value {
    json!({"nodes": ids.iter().map(|id| json!({"id": id, "name": id})).collect::<Vec<_>>(), "pageInfo": {"hasNextPage": false, "endCursor": null}})
}

fn relation(id: &str, kind: &str, from: &str, to: &str, created: &str) -> Value {
    json!({"id": id, "type": kind, "issue": {"id": from}, "relatedIssue": {"id": to}, "createdAt": created, "updatedAt": created, "archivedAt": null})
}

const UPLOAD_URL: &str = "https://uploads.linear.app/org/file/diagram.png";
const FAILED_URL: &str = "https://uploads.linear.app/org/file/missing.pdf";

fn fixture() -> Package {
    let eng = "10000000-0000-4000-8000-000000000001";
    let ops = "10000000-0000-4000-8000-000000000002";
    let web = "10000000-0000-4000-8000-000000000003";
    let mut archived_team = team(web, "WEB", "Web");
    archived_team["archivedAt"] = json!("2026-01-05T00:00:00Z");
    let teams = json!([
        team(eng, "ENG", "Engineering"),
        team(ops, "OPS", "Operations"),
        archived_team
    ]);
    let states = json!([
        {"id": "s-eng-backlog", "name": "Backlog", "type": "backlog", "team": {"id": eng}},
        {"id": "s-eng-started", "name": "In Progress", "type": "started", "team": {"id": eng}},
        {"id": "s-eng-done", "name": "Done", "type": "completed", "team": {"id": eng}},
        {"id": "s-eng-canceled", "name": "Canceled", "type": "canceled", "team": {"id": eng}},
        {"id": "s-ops-triage", "name": "Triage", "type": "triage", "team": {"id": ops}},
        {"id": "s-ops-done", "name": "Done", "type": "completed", "team": {"id": ops}},
    ]);
    let label = |id: &str, name: &str, team: Option<&str>, archived: bool| json!({"id": id, "name": name, "description": format!("{name} label"), "color": "#aabbcc", "team": team.map(|team| json!({"id": team, "key": team_key(team), "name": team_key(team)})), "createdAt": CREATED, "updatedAt": CREATED, "archivedAt": if archived { json!(CREATED) } else { Value::Null }, "isGroup": false});
    let issue_labels = json!([
        label(
            "20000000-0000-4000-8000-000000000001",
            "Bug",
            Some(eng),
            false
        ),
        label(
            "20000000-0000-4000-8000-000000000002",
            "bug",
            Some(ops),
            false
        ),
        label("20000000-0000-4000-8000-000000000003", "Bug ", None, false),
        label(
            "20000000-0000-4000-8000-000000000004",
            "Feature <b>",
            None,
            false
        ),
        label(
            "20000000-0000-4000-8000-000000000005",
            "Old",
            Some(eng),
            true
        ),
    ]);
    let project = |id: &str,
                   name: &str,
                   teams: &[&str],
                   status: &str,
                   sort: f64,
                   initiatives: &[&str]| {
        json!({"id": id, "name": name, "description": format!("{name} description"), "content": format!("Long-form {name} overview"), "priority": 2, "sortOrder": sort, "createdAt": CREATED, "updatedAt": CREATED, "archivedAt": null, "url": format!("https://linear.app/example/project/{id}"), "status": {"id": format!("ps-{status}"), "name": status, "type": status}, "state": status, "targetDate": null,
            "teams": {"nodes": teams.iter().map(|team| json!({"id": team, "key": team_key(team), "name": team_key(team)})).collect::<Vec<_>>(), "pageInfo": {"hasNextPage": false}},
            "initiatives": {"nodes": initiatives.iter().map(|id| json!({"id": id})).collect::<Vec<_>>(), "pageInfo": {"hasNextPage": false}}})
    };
    let mut beta = project(
        "40000000-0000-4000-8000-00000000000b",
        "Beta",
        &[eng],
        "completed",
        -10.0,
        &[],
    );
    beta["archivedAt"] = json!("2026-01-06T00:00:00Z");
    let projects = json!([
        project(
            "40000000-0000-4000-8000-00000000000a",
            "Alpha",
            &[eng],
            "started",
            2.5,
            &["i-growth"]
        ),
        beta,
        project(
            "40000000-0000-4000-8000-00000000000c",
            "Shared",
            &[eng, ops],
            "planned",
            1.0,
            &[]
        ),
        project(
            "40000000-0000-4000-8000-00000000000d",
            "Ops Runbook",
            &[ops],
            "planned",
            0.0,
            &[]
        ),
    ]);
    let milestone = |id: &str, name: &str, project: &str| json!({"id": id, "name": name, "description": "", "sortOrder": 1.5, "targetDate": "2026-03-01", "project": {"id": project}, "createdAt": CREATED, "updatedAt": CREATED, "archivedAt": null});
    let milestones = json!([
        milestone("m-alpha-1", "M1", "40000000-0000-4000-8000-00000000000a"),
        milestone("m-alpha-2", "m1", "40000000-0000-4000-8000-00000000000a"),
        milestone("m-beta-1", "Launch", "40000000-0000-4000-8000-00000000000b"),
    ]);
    let initiatives = json!([
        {"id": "i-growth", "name": "Growth", "description": "Grow", "status": "Active", "createdAt": CREATED, "updatedAt": CREATED, "url": "https://linear.app/example/initiative/growth", "projects": {"nodes": [{"id": "40000000-0000-4000-8000-00000000000d"}], "pageInfo": {"hasNextPage": false}}},
        {"id": "i-orphan", "name": "Orphan", "description": "", "status": "Planned", "createdAt": CREATED, "updatedAt": CREATED, "projects": {"nodes": [], "pageInfo": {"hasNextPage": false}}},
    ]);

    let mut eng1 = issue(
        "iss-eng-1",
        "ENG-1",
        eng,
        "ENG",
        ("s-eng-done", "completed"),
    );
    eng1["title"] = json!("<script>alert(1)</script> Parent work");
    eng1["project"] = json!({"id": "40000000-0000-4000-8000-00000000000a"});
    eng1["projectMilestone"] = json!({"id": "m-alpha-1"});
    eng1["completedAt"] = json!("2026-01-04T00:00:00Z");
    eng1["labels"] = labels(&[
        "20000000-0000-4000-8000-000000000001",
        "20000000-0000-4000-8000-000000000004",
    ]);
    eng1["history"] = json!({"nodes": [{"id": "h1", "createdAt": CREATED, "fromState": null}, {"id": "h2", "createdAt": CREATED, "toState": {"id": "s-eng-done"}}], "pageInfo": {"hasNextPage": false}});
    eng1["assignee"] = json!({"id": "u-ada", "name": "Ada"});
    eng1["priority"] = json!(1);
    eng1["previousIdentifiers"] = json!(["OPS-9"]);
    eng1["description"] = json!(format!("See ![diagram]({UPLOAD_URL})"));

    let mut eng2 = issue(
        "iss-eng-2",
        "ENG-2",
        eng,
        "ENG",
        ("s-eng-started", "started"),
    );
    eng2["project"] = json!({"id": "40000000-0000-4000-8000-00000000000b"});
    eng2["projectMilestone"] = json!({"id": "m-beta-1"});
    eng2["parent"] = json!({"id": "iss-eng-1"});
    eng2["priority"] = json!(0);
    eng2["labels"] = labels(&["20000000-0000-4000-8000-000000000003"]);
    eng2["history"] =
        json!({"nodes": [{"id": "h3", "createdAt": CREATED}], "pageInfo": {"hasNextPage": false}});

    let mut eng3 = issue(
        "iss-eng-3",
        "ENG-3",
        eng,
        "ENG",
        ("s-eng-backlog", "backlog"),
    );
    eng3["archivedAt"] = json!("2026-01-07T00:00:00Z");
    eng3["dueDate"] = json!("2026-02-01");
    eng3["estimate"] = json!(3);

    let mut eng4 = issue(
        "iss-eng-4",
        "ENG-4",
        eng,
        "ENG",
        ("s-eng-canceled", "canceled"),
    );
    eng4["project"] = json!({"id": "40000000-0000-4000-8000-00000000000c"});
    eng4["canceledAt"] = json!("2026-01-04T00:00:00Z");
    eng4["labels"] = labels(&["20000000-0000-4000-8000-000000000002"]);

    let mut ops1 = issue("iss-ops-1", "OPS-1", ops, "OPS", ("s-ops-triage", "triage"));
    ops1["project"] = json!({"id": "40000000-0000-4000-8000-00000000000c"});
    ops1["labels"] = labels(&["20000000-0000-4000-8000-000000000002"]);
    let mut ops2 = issue(
        "iss-ops-2",
        "OPS-2",
        ops,
        "OPS",
        ("s-ops-done", "completed"),
    );
    ops2["project"] = json!({"id": "40000000-0000-4000-8000-00000000000c"});
    ops2["completedAt"] = json!("2026-01-04T00:00:00Z");
    let mut ops3 = issue("iss-ops-3", "OPS-3", ops, "OPS", ("s-ops-triage", "triage"));
    ops3["project"] = json!({"id": "40000000-0000-4000-8000-00000000000d"});
    let issues = json!([eng1, eng2, eng3, eng4, ops1, ops2, ops3]);

    let mut archived_relation = relation(
        "30000000-0000-4000-8000-000000000008",
        "related",
        "iss-eng-2",
        "iss-eng-3",
        "2026-01-02T00:00:08Z",
    );
    archived_relation["archivedAt"] = json!("2026-01-03T00:00:00Z");
    let relations = json!([
        relation(
            "30000000-0000-4000-8000-000000000001",
            "blocks",
            "iss-eng-1",
            "iss-eng-2",
            "2026-01-02T00:00:01Z"
        ),
        relation(
            "30000000-0000-4000-8000-000000000002",
            "related",
            "iss-eng-1",
            "iss-eng-3",
            "2026-01-02T00:00:02Z"
        ),
        relation(
            "30000000-0000-4000-8000-000000000003",
            "related",
            "iss-eng-3",
            "iss-eng-1",
            "2026-01-02T00:00:03Z"
        ),
        relation(
            "30000000-0000-4000-8000-000000000004",
            "duplicate",
            "iss-eng-3",
            "iss-eng-2",
            "2026-01-02T00:00:04Z"
        ),
        relation(
            "30000000-0000-4000-8000-000000000005",
            "related",
            "iss-eng-1",
            "iss-ops-1",
            "2026-01-02T00:00:05Z"
        ),
        relation(
            "30000000-0000-4000-8000-000000000006",
            "blocks",
            "iss-eng-2",
            "iss-eng-1",
            "2026-01-02T00:00:06Z"
        ),
        relation(
            "rel-dup-blocks",
            "blocks",
            "iss-eng-1",
            "iss-eng-2",
            "2026-01-02T00:00:07Z"
        ),
        archived_relation,
    ]);
    let comments = json!([
        {"id": "c-1", "body": format!("hello <img src=x onerror=alert(1)> {UPLOAD_URL}"), "createdAt": CREATED, "updatedAt": CREATED, "url": "https://linear.app/example/comment/c-1", "issue": {"id": "iss-eng-1"}, "user": {"id": "u-ada", "name": "Ada"}, "parent": null, "archivedAt": null},
        {"id": "c-2", "body": "reply", "createdAt": CREATED, "updatedAt": CREATED, "url": "https://linear.app/example/comment/c-2", "issue": {"id": "iss-eng-1"}, "user": {"id": "u-bo", "name": "Bo"}, "parent": {"id": "c-1"}, "archivedAt": null},
        {"id": "c-3", "body": "on a document", "createdAt": CREATED, "updatedAt": CREATED, "issue": null, "documentContent": {"id": "dc-1"}, "user": {"id": "u-bo", "name": "Bo"}, "archivedAt": null},
        {"id": "c-4", "body": "removed", "createdAt": CREATED, "updatedAt": CREATED, "issue": {"id": "iss-eng-2"}, "user": {"id": "u-bo", "name": "Bo"}, "archivedAt": "2026-01-03T00:00:00Z"},
    ]);
    let documents = json!([
        {"id": "d-1", "title": "Spec <draft>", "content": "# Spec\n<script>bad()</script>\n", "project": {"id": "40000000-0000-4000-8000-00000000000a"}, "createdAt": CREATED, "updatedAt": CREATED},
    ]);
    let users = json!([
        {"id": "u-ada", "name": "Ada", "email": "ada@example.invalid", "active": true},
        {"id": "u-bo", "name": "Bo", "email": "bo@example.invalid", "active": false},
    ]);
    let link_attachments = json!([
        {"id": "a-1", "title": "PR", "url": "https://github.com/example/repo/pull/1", "issue": {"id": "iss-eng-1"}, "createdAt": CREATED},
    ]);
    let project_updates = json!([{"id": "pu-1", "body": "On track", "project": {"id": "40000000-0000-4000-8000-00000000000a"}, "createdAt": CREATED}]);
    let initiative_updates = json!([{"id": "iu-1", "body": "Growing", "initiative": {"id": "i-growth"}, "createdAt": CREATED}]);

    let data = BTreeMap::from([
        ("teams.json", teams),
        ("workflow-states.json", states),
        ("issue-labels.json", issue_labels),
        ("projects.json", projects),
        ("project-milestones.json", milestones),
        ("initiatives.json", initiatives),
        ("issues.json", issues),
        ("comments.json", comments),
        ("issue-relations.json", relations),
        ("documents.json", documents),
        ("users.json", users),
        ("attachments.json", link_attachments),
        ("project-updates.json", project_updates),
        ("initiative-updates.json", initiative_updates),
        (
            "graphql-schema.json",
            json!({"query_type": "Query", "types": []}),
        ),
    ]);
    let bytes = b"\x89PNG synthetic image bytes".to_vec();
    let path = format!("attachments/{}-diagram.png", sha(&bytes));
    let downloaded = |source: &str| json!({"canonical_url": UPLOAD_URL, "source_paths": [source], "status": "downloaded", "path": path, "original_name": "diagram.png", "content_type": "image/png", "bytes": bytes.len(), "sha256": sha(&bytes)});
    Package {
        data,
        uploads: vec![
            downloaded("issues.0.description"),
            downloaded("comments.0.body"),
            json!({"canonical_url": FAILED_URL, "source_paths": ["comments.1.body"], "status": "failed", "error": "attachment download returned HTTP 404"}),
        ],
        upload_files: vec![(path.clone(), bytes)],
        errors: vec![
            json!({"scope": format!("attachment {FAILED_URL}"), "message": "attachment download returned HTTP 404"}),
        ],
    }
}

impl Package {
    fn write(&self, root: &Path) {
        fs::create_dir_all(root.join("data")).unwrap();
        fs::create_dir_all(root.join("attachments")).unwrap();
        let mut data_files = Vec::new();
        let mut counts = serde_json::Map::new();
        for (name, records) in &self.data {
            let value = if *name == "graphql-schema.json" {
                records.clone()
            } else {
                let root_name = camel(name.trim_end_matches(".json"));
                counts.insert(root_name, json!(records.as_array().unwrap().len()));
                json!({"captured_at": CREATED, "count": records.as_array().unwrap().len(), "records": records})
            };
            let bytes = pretty(&value);
            fs::write(root.join("data").join(name), &bytes).unwrap();
            data_files.push(json!({"path": format!("data/{name}"), "bytes": bytes.len(), "sha256": sha(&bytes)}));
        }
        let mut attachment_files = Vec::new();
        for upload in &self.uploads {
            if upload["status"] == "downloaded" {
                attachment_files.push(json!({"path": upload["path"], "sha256": upload["sha256"], "bytes": upload["bytes"]}));
            }
        }
        for (path, bytes) in &self.upload_files {
            fs::write(root.join(path), bytes).unwrap();
        }
        fs::write(
            root.join("attachment-manifest.json"),
            pretty(&json!(self.uploads)),
        )
        .unwrap();
        let downloaded = attachment_files.len();
        let manifest = json!({
            "format": 1,
            "captured_at": CREATED,
            "source": {"service": "Linear", "workspace_slug": "example"},
            "output_directory": "/private/capture",
            "counts": counts,
            "attachments": {"discovered_upload_urls": self.uploads.len(), "downloaded_and_checksummed": downloaded, "inaccessible_or_failed": self.uploads.len() - downloaded, "link_attachment_records": 1},
            "query_coverage": [{"root": "issues", "records": 7, "include_archived_supported": true, "failed_nested_connections": [], "truncated_nested_connections": []}],
            "errors": self.errors,
            "missing_coverage": [
                "1 uploaded attachment(s) could not be downloaded or checksummed; see attachment-manifest.json",
                "Deleted records that are no longer returned by Linear are not recoverable through the public API.",
                "External link attachments are preserved as metadata but their third-party contents are not downloaded.",
            ],
            "integrity": {"manifest_sha256": "SELF (see manifest.sha256)", "data_files": data_files, "attachment_files": attachment_files},
        });
        write_manifest(root, &manifest);
    }
}

fn write_manifest(root: &Path, manifest: &Value) {
    let bytes = serde_json::to_vec_pretty(manifest).unwrap();
    fs::write(
        root.join("manifest.json"),
        format!("{}\n", String::from_utf8(bytes).unwrap()),
    )
    .unwrap();
    let bytes = fs::read(root.join("manifest.json")).unwrap();
    fs::write(
        root.join("manifest.sha256"),
        format!("{}  manifest.json\n", sha(&bytes)),
    )
    .unwrap();
}

fn edit_manifest(root: &Path, edit: impl FnOnce(&mut Value)) {
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    edit(&mut manifest);
    write_manifest(root, &manifest);
}

fn camel(value: &str) -> String {
    let mut output = String::new();
    let mut upper = false;
    for character in value.chars() {
        if character == '-' {
            upper = true;
        } else if upper {
            output.push(character.to_ascii_uppercase());
            upper = false;
        } else {
            output.push(character);
        }
    }
    output
}

fn direct(data_dir: &Path, args: &[&str]) -> Output {
    Process::new(env!("CARGO_BIN_EXE_direct"))
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .output()
        .unwrap()
}

fn import(data_dir: &Path, source: &Path, output: &Path) -> Output {
    direct(
        data_dir,
        &[
            "linear-import-dry-run",
            "--whole-workspace",
            "--source",
            source.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ],
    )
}

fn succeeded(output: Output) -> Value {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn failed(output: Output) -> String {
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

struct Service(Child);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn serve(dir: &Path) -> Service {
    let child = direct::hidden(
        Process::new(env!("CARGO_BIN_EXE_direct"))
            .arg("--data-dir")
            .arg(dir)
            .arg("serve"),
    )
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let service = Service(child);
    for _ in 0..100 {
        if Client::new(dir).is_ok_and(|client| client.healthy()) {
            return service;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("Service failed to start on the isolated workspace");
}

fn files_under(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, directory: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, files);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(relative, fs::read(&path).unwrap());
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut files);
    files
}

fn issue_by_key<'a>(archive: &'a Value, key: &str) -> &'a Value {
    archive["issues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|issue| issue["key"] == key)
        .unwrap()
}

fn entry<'a>(accounting: &'a Value, file: &str, id: &str) -> &'a Value {
    accounting["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            entry["level"] == "record" && entry["file"] == file && entry["source_id"] == id
        })
        .unwrap_or_else(|| panic!("no accounting entry for {file} {id}"))
}

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
    assert_eq!(first["cutover_readiness"]["status"], "blocked");
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
    let blockers: Vec<_> = report["cutover_readiness"]["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["code"].as_str().unwrap().to_owned())
        .collect();
    for code in [
        "no_live_apply",
        "native_access_missing",
        "unresolved_records",
        "unsupported_relation_semantics",
        "cross_product_mapping",
        "source_limitations",
        "unsupported_fields",
    ] {
        assert!(
            blockers.contains(&code.to_owned()),
            "missing blocker {code}"
        );
    }
    assert_eq!(report["native_access"]["status"], "missing");

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
                        product: "ENG".into(),
                        title: "Added after import".into(),
                        body: String::new(),
                        acceptance: String::new(),
                        owner: String::new(),
                        priority: "medium".into(),
                        planning_scope: direct_core::PlanningScope::Inbox,
                        project_id: None,
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
