//! Performance regression for migrated workspaces: many projects, goals and
//! milestones plus tens of megabytes of imported external history. Ordinary
//! snapshot polling must stay small and fast; context and export must still
//! return every original history value.

use direct_core::{
    Archive, Command, ExternalIssueRecord, ExternalIssueState, Goal, GoalStatus, Issue, Milestone,
    PlanningScope, Product, Project, ProjectStatus, Request, Role, Status, Store,
};
use serde_json::{json, Value};
use std::time::Instant;

const ISSUES: usize = 560;
const HISTORY: usize = 110;

fn history(issue: usize) -> Vec<Value> {
    (0..HISTORY)
        .map(|entry| {
            json!({
                "id": format!("h-{issue}-{entry}"),
                "createdAt": "2026-01-02T03:04:05.678Z",
                "fromState": {"id": "s1", "name": "Backlog", "type": "backlog"},
                "toState": {"id": "s2", "name": "In Progress", "type": "started"},
                "actor": {"id": "u1", "name": "Ada"},
                "note": "x".repeat(420),
            })
        })
        .collect()
}

fn workspace() -> Archive {
    let product = Product {
        id: "00000000-0000-4000-8000-00000000aaaa".into(),
        key: "ENG".into(),
        name: "Engineering".into(),
        repo_windows: String::new(),
        repo_wsl: String::new(),
        vault_windows: String::new(),
        vault_wsl: String::new(),
        sort_order: 0,
        section_id: None,
    };
    let projects: Vec<Project> = (0..30)
        .map(|index| Project {
            id: format!("project-{index:02}"),
            product_id: product.id.clone(),
            name: format!("Project {index}"),
            description: String::new(),
            status: ProjectStatus::Active,
            priority: "medium".into(),
            sort_order: index,
            external_source: None,
            external_id: None,
            external_url: None,
            labels: vec![],
            template: None,
            version: 1,
            created_at: 1,
            updated_at: 1,
        })
        .collect();
    let milestones: Vec<Milestone> = (0..31)
        .map(|index| Milestone {
            id: format!("milestone-{index:02}"),
            project_id: format!("project-{:02}", index % 30),
            name: format!("Milestone {index}"),
            description: String::new(),
            sort_order: index,
            external_source: None,
            external_id: None,
            external_url: None,
            version: 1,
            created_at: 1,
            updated_at: 1,
        })
        .collect();
    let goals: Vec<Goal> = (0..9)
        .map(|index| Goal {
            id: format!("goal-{index}"),
            product_id: product.id.clone(),
            name: format!("Goal {index}"),
            description: String::new(),
            status: GoalStatus::Active,
            priority: "medium".into(),
            project_ids: vec![format!("project-{:02}", index * 3)],
            external_source: None,
            external_id: None,
            external_url: None,
            version: 1,
            created_at: 1,
            updated_at: 1,
        })
        .collect();
    let issues: Vec<Issue> = (0..ISSUES)
        .map(|index| Issue {
            intake: None,
            id: format!("issue-{index:04}"),
            key: format!("ENG-{}", index + 1),
            product_id: product.id.clone(),
            project_id: Some(format!("project-{:02}", index % 30)),
            milestone_id: Some(format!("milestone-{:02}", index % 30)),
            planning_scope: PlanningScope::Project,
            theoria_refs: vec![],
            labels: vec![],
            title: format!("Imported {index}"),
            body: "Body".into(),
            acceptance: String::new(),
            owner: String::new(),
            priority: "low".into(),
            status: Status::Backlog,
            version: 1,
            created_at: 1,
            updated_at: 1,
            claim: None,
            needs_fix: false,
            parent: None,
            verification_key: None,
            current_run: None,
            external: Some(ExternalIssueRecord {
                source: "linear".into(),
                id: format!("linear-{index}"),
                url: format!("https://linear.example/{index}"),
                state: ExternalIssueState {
                    id: "s2".into(),
                    name: "In Progress".into(),
                    kind: "started".into(),
                },
                created_at: "2026-01-02T03:04:05.678Z".into(),
                updated_at: "2026-01-02T03:04:05.678Z".into(),
                started_at: None,
                completed_at: None,
                canceled_at: None,
                archived_at: None,
                history: history(index),
            }),
            template: None,
        })
        .collect();
    Archive {
        format: direct_core::ARCHIVE_FORMAT,
        workspace_id: "00000000-0000-4000-8000-00000000bbbb".into(),
        products: vec![product],
        product_sections: Vec::new(),
        customer_signals: Vec::new(),
        context_links: Vec::new(),
        cloud_handoffs: Vec::new(),
        skill_packages: Vec::new(),
        agent_roles: Vec::new(),
        role_publications: Vec::new(),
        agent_members: Vec::new(),
        issue_assignments: Vec::new(),
        agent_runs: Vec::new(),
        projects,
        goals,
        milestones,
        labels: vec![],
        theoria_documents: vec![],
        method_findings: vec![],
        git_traces: vec![],
        releases: vec![],
        release_evidence: vec![],
        release_workflows: vec![],
        issue_links: vec![],
        templates: vec![],
        template_revisions: vec![],
        issues,
        comments: vec![],
        verifications: vec![],
        events: vec![],
        requests: vec![],
        source_bundles: vec![],
        source_files: vec![],
        source_records: vec![],
    }
}

fn read(store: &mut Store, command: Command) -> Value {
    store
        .execute(
            Request {
                actor: "reader".into(),
                request_id: String::new(),
                command,
            },
            Role::Agent,
        )
        .unwrap()
}

#[test]
fn snapshot_stays_small_and_fast_with_large_imported_history() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("direct.db");
    let archive = workspace();
    let history_bytes: usize = archive
        .issues
        .iter()
        .map(|issue| {
            serde_json::to_vec(&issue.external.as_ref().unwrap().history)
                .unwrap()
                .len()
        })
        .sum();
    assert!(
        history_bytes > 30_000_000,
        "fixture carries {history_bytes} bytes of history"
    );
    let mut store = Store::open(&path).unwrap();
    store.restore(archive.clone()).unwrap();

    // Issue rows never embed history, so every ordinary read stays light.
    let conn = rusqlite::Connection::open(&path).unwrap();
    let largest: i64 = conn
        .query_row("SELECT MAX(length(data)) FROM issues", [], |r| r.get(0))
        .unwrap();
    assert!(largest < 4_096, "issue rows carry {largest} bytes");

    let started = Instant::now();
    let snapshot = read(&mut store, Command::Snapshot);
    let elapsed = started.elapsed();
    let bytes = serde_json::to_vec(&snapshot).unwrap().len();
    eprintln!("snapshot of {ISSUES} issues with {history_bytes} history bytes: {elapsed:?}, {bytes} response bytes");
    assert!(bytes < 1_500_000, "snapshot is {bytes} bytes");
    assert!(
        elapsed.as_secs_f64() < 3.0,
        "snapshot took {elapsed:?} for {ISSUES} issues, 30 projects, 31 milestones and 9 goals"
    );
    assert_eq!(snapshot["project_progress"].as_array().unwrap().len(), 30);
    assert_eq!(snapshot["milestone_progress"].as_array().unwrap().len(), 31);
    assert_eq!(snapshot["goal_progress"].as_array().unwrap().len(), 9);
    assert_eq!(
        snapshot["issues"][0]["external"]["history_entries"],
        HISTORY
    );
    assert_eq!(snapshot["issues"][0]["external"]["history"], json!([]));

    // Context and export keep every original history value.
    let context = read(
        &mut store,
        Command::Context {
            key: "ENG-7".into(),
        },
    );
    assert_eq!(context["issue"]["external"]["history"], json!(history(6)));
    let exported = store.export().unwrap();
    assert_eq!(
        serde_json::to_value(&exported).unwrap(),
        serde_json::to_value(&archive).unwrap(),
        "export reproduces the restored archive exactly"
    );

    // A mutation on an imported issue keeps its stored history.
    store
        .execute(
            Request {
                actor: "owner".into(),
                request_id: "edit-1".into(),
                command: Command::UpdateIssue {
                    intake: None,
                    key: "ENG-7".into(),
                    expected_version: 1,
                    title: "Edited".into(),
                    body: "Body".into(),
                    acceptance: String::new(),
                    owner: String::new(),
                    priority: "low".into(),
                    planning_scope: None,
                },
            },
            Role::Human,
        )
        .unwrap();
    let context = read(
        &mut store,
        Command::Context {
            key: "ENG-7".into(),
        },
    );
    assert_eq!(context["issue"]["title"], "Edited");
    assert_eq!(context["issue"]["external"]["history"], json!(history(6)));
}

#[test]
fn databases_with_embedded_history_are_split_once_on_open() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("direct.db");
    let mut archive = workspace();
    archive.issues.truncate(3);
    let mut store = Store::open(&path).unwrap();
    store.restore(archive.clone()).unwrap();
    drop(store);
    // Simulate a database written before history moved out of issue rows.
    let conn = rusqlite::Connection::open(&path).unwrap();
    for issue in &archive.issues {
        conn.execute(
            "UPDATE issues SET data=?1 WHERE id=?2",
            (serde_json::to_string(issue).unwrap(), &issue.id),
        )
        .unwrap();
    }
    conn.execute("DELETE FROM issue_histories", []).unwrap();
    conn.execute("DELETE FROM meta WHERE key='issue_history_split'", [])
        .unwrap();
    drop(conn);

    let mut reopened = Store::open(&path).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    let largest: i64 = conn
        .query_row("SELECT MAX(length(data)) FROM issues", [], |r| r.get(0))
        .unwrap();
    assert!(
        largest < 4_096,
        "embedded history was not moved out ({largest} bytes)"
    );
    let context = read(
        &mut reopened,
        Command::Context {
            key: "ENG-2".into(),
        },
    );
    assert_eq!(context["issue"]["external"]["history"], json!(history(1)));
    assert_eq!(
        serde_json::to_value(reopened.export().unwrap()).unwrap(),
        serde_json::to_value(&archive).unwrap()
    );
}
