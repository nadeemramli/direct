//! Synthetic multi-team Linear capture package shared by the importer tests.
#![allow(dead_code)]

use direct::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    process::{Child, Command as Process, Output, Stdio},
    thread,
    time::Duration,
};

pub const CREATED: &str = "2026-01-02T03:04:05.678Z";

pub fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn pretty(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).unwrap();
    bytes.push(b'\n');
    bytes
}

/// A synthetic capture package laid out exactly like `capture-linear.mjs` output.
#[derive(Clone)]
pub struct Package {
    pub data: BTreeMap<&'static str, Value>,
    pub uploads: Vec<Value>,
    pub upload_files: Vec<(String, Vec<u8>)>,
    errors: Vec<Value>,
}

pub fn team_key(id: &str) -> &'static str {
    match id.chars().last() {
        Some('1') => "ENG",
        Some('2') => "OPS",
        _ => "WEB",
    }
}

pub fn team(id: &str, key: &str, name: &str) -> Value {
    json!({"id": id, "key": key, "name": name})
}

pub fn issue(
    id: &str,
    identifier: &str,
    team_id: &str,
    team_key: &str,
    state: (&str, &str),
) -> Value {
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

pub fn labels(ids: &[&str]) -> Value {
    json!({"nodes": ids.iter().map(|id| json!({"id": id, "name": id})).collect::<Vec<_>>(), "pageInfo": {"hasNextPage": false, "endCursor": null}})
}

pub fn relation(id: &str, kind: &str, from: &str, to: &str, created: &str) -> Value {
    json!({"id": id, "type": kind, "issue": {"id": from}, "relatedIssue": {"id": to}, "createdAt": created, "updatedAt": created, "archivedAt": null})
}

pub const UPLOAD_URL: &str = "https://uploads.linear.app/org/file/diagram.png";
pub const FAILED_URL: &str = "https://uploads.linear.app/org/file/missing.pdf";

pub fn fixture() -> Package {
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
    pub fn write(&self, root: &Path) {
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

pub fn write_manifest(root: &Path, manifest: &Value) {
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

pub fn edit_manifest(root: &Path, edit: impl FnOnce(&mut Value)) {
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    edit(&mut manifest);
    write_manifest(root, &manifest);
}

pub fn camel(value: &str) -> String {
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

pub fn direct(data_dir: &Path, args: &[&str]) -> Output {
    Process::new(env!("CARGO_BIN_EXE_direct"))
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .output()
        .unwrap()
}

pub fn import(data_dir: &Path, source: &Path, output: &Path) -> Output {
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

pub fn succeeded(output: Output) -> Value {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

pub fn failed(output: Output) -> String {
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

pub struct Service(pub Child);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn serve(dir: &Path) -> Service {
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

pub fn files_under(root: &Path) -> BTreeMap<String, Vec<u8>> {
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

pub fn issue_by_key<'a>(archive: &'a Value, key: &str) -> &'a Value {
    archive["issues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|issue| issue["key"] == key)
        .unwrap()
}

pub fn entry<'a>(accounting: &'a Value, file: &str, id: &str) -> &'a Value {
    accounting["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            entry["level"] == "record" && entry["file"] == file && entry["source_id"] == id
        })
        .unwrap_or_else(|| panic!("no accounting entry for {file} {id}"))
}
