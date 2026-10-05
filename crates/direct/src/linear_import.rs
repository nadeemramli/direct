mod index;
mod migration;
mod package;
mod retained;
mod workspace;

pub use migration::prepare as prepare_migration;
pub use workspace::whole_workspace;

use anyhow::{bail, Context, Result};
use chrono::DateTime;
use direct_core::{
    validate_archive, Archive, Comment, ExternalIssueRecord, ExternalIssueState, Goal, GoalStatus,
    Issue, IssueLink, IssueLinkKind, Milestone, PlanningScope, Product, Project, ProjectStatus,
    Status, Store,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const SOURCE: &str = "linear";
const REQUIRED_DATA: &[&str] = &[
    "teams.json",
    "projects.json",
    "project-milestones.json",
    "initiatives.json",
    "issues.json",
    "comments.json",
    "issue-relations.json",
    "workflow-states.json",
];

#[derive(Debug, Deserialize)]
struct CaptureFile {
    records: Vec<Value>,
}

pub fn dry_run(
    source: &Path,
    project_id: &str,
    output: &Path,
    owner_data_dir: Option<&Path>,
) -> Result<Value> {
    let source = source
        .canonicalize()
        .context("Linear source package does not exist")?;
    ensure_private_output(output, owner_data_dir)?;
    let manifest = package::verify_package(&source, REQUIRED_DATA)?.manifest;
    let data = source.join("data");
    let projects = records(&data, "projects.json")?;
    let source_project = projects
        .iter()
        .find(|project| string(project, "id") == project_id)
        .with_context(|| format!("Linear project {project_id} was not found"))?;
    let issues = records(&data, "issues.json")?;
    let selected_issues: Vec<_> = issues
        .into_iter()
        .filter(|issue| nested_string(issue, &["project", "id"]).as_deref() == Some(project_id))
        .collect();
    if selected_issues.is_empty() {
        bail!("Selected Linear project contains no captured issues");
    }

    let issue_ids: HashSet<_> = selected_issues
        .iter()
        .map(|issue| string(issue, "id").to_owned())
        .collect();
    let team = source_project
        .pointer("/teams/nodes/0")
        .or_else(|| selected_issues.first().and_then(|issue| issue.get("team")))
        .context("Selected project has no captured team")?;
    let product = Product {
        id: string(team, "id").to_owned(),
        key: string(team, "key").to_owned(),
        name: string(team, "name").to_owned(),
        repo_windows: String::new(),
        repo_wsl: String::new(),
        vault_windows: String::new(),
        vault_wsl: String::new(),
        sort_order: 0,
        section_id: None,
    };
    if product.id.is_empty() || product.key.is_empty() || product.name.is_empty() {
        bail!("Selected project team is missing its id, key, or name");
    }

    let mut unknown_issue_states = BTreeSet::new();
    let mut direct_issues = selected_issues
        .iter()
        .map(|issue| import_issue(issue, &product, project_id, &mut unknown_issue_states))
        .collect::<Result<Vec<_>>>()?;
    direct_issues.sort_by(|left, right| left.id.cmp(&right.id));
    let issue_keys: HashMap<_, _> = direct_issues
        .iter()
        .filter_map(|issue| {
            issue
                .external
                .as_ref()
                .map(|external| (external.id.clone(), issue.key.clone()))
        })
        .collect();

    let all_comments = records(&data, "comments.json")?;
    let mut comments = all_comments
        .iter()
        .filter(|comment| {
            nested_string(comment, &["issue", "id"])
                .or_else(|| optional_string(comment, "issueId"))
                .is_some_and(|id| issue_ids.contains(&id))
        })
        .map(|comment| import_comment(comment, &issue_keys))
        .collect::<Result<Vec<_>>>()?;
    comments.sort_by(|left, right| left.id.cmp(&right.id));

    let all_milestones = records(&data, "project-milestones.json")?;
    let mut milestones = all_milestones
        .iter()
        .filter(|milestone| {
            nested_string(milestone, &["project", "id"]).as_deref() == Some(project_id)
        })
        .map(|milestone| import_milestone(milestone, project_id))
        .collect::<Result<Vec<_>>>()?;
    milestones.sort_by(|left, right| left.id.cmp(&right.id));

    let all_initiatives = records(&data, "initiatives.json")?;
    let mut unknown_goal_states = BTreeSet::new();
    let mut goals = all_initiatives
        .iter()
        .filter(|initiative| {
            initiative
                .pointer("/projects/nodes")
                .and_then(Value::as_array)
                .is_some_and(|projects| {
                    projects
                        .iter()
                        .any(|project| string(project, "id") == project_id)
                })
        })
        .map(|initiative| {
            import_goal(
                initiative,
                &product.id,
                project_id,
                &mut unknown_goal_states,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    goals.sort_by(|left, right| left.id.cmp(&right.id));

    let all_relations = records(&data, "issue-relations.json")?;
    let touching_relations: Vec<_> = all_relations
        .iter()
        .filter(|relation| {
            nested_string(relation, &["issue", "id"]).is_some_and(|id| issue_ids.contains(&id))
                || nested_string(relation, &["relatedIssue", "id"])
                    .is_some_and(|id| issue_ids.contains(&id))
        })
        .collect();
    let internal_relations: Vec<_> = touching_relations
        .iter()
        .filter(|relation| {
            nested_string(relation, &["issue", "id"]).is_some_and(|id| issue_ids.contains(&id))
                && nested_string(relation, &["relatedIssue", "id"])
                    .is_some_and(|id| issue_ids.contains(&id))
        })
        .copied()
        .collect();
    let mut unknown_relation_types = BTreeSet::new();
    let mut issue_links = internal_relations
        .iter()
        .filter_map(|relation| match import_relation(relation, &issue_keys) {
            Ok(Some(link)) => Some(Ok(link)),
            Ok(None) => {
                unknown_relation_types.insert(string(relation, "type").to_owned());
                None
            }
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>>>()?;

    let mut external_parent_links = 0_u64;
    for issue in &selected_issues {
        let Some(parent) = issue.get("parent").filter(|value| !value.is_null()) else {
            continue;
        };
        let child_id = string(issue, "id");
        let parent_id = string(parent, "id");
        if let (Some(child_key), Some(parent_key)) =
            (issue_keys.get(child_id), issue_keys.get(parent_id))
        {
            issue_links.push(IssueLink {
                id: deterministic_uuid("linear-parent", &format!("{child_id}:{parent_id}")),
                source_key: child_key.clone(),
                target_key: parent_key.clone(),
                kind: IssueLinkKind::Parent,
                external_source: Some(SOURCE.into()),
                external_id: Some(format!("parent:{child_id}:{parent_id}")),
                created_by: "linear-import".into(),
                created_at: timestamp(issue, "createdAt")?,
            });
        } else {
            external_parent_links += 1;
        }
    }
    issue_links.sort_by(|left, right| left.id.cmp(&right.id));

    let project = import_project(source_project, &product.id, &mut unknown_goal_states)?;
    let history_entries: usize = direct_issues
        .iter()
        .map(|issue| {
            issue
                .external
                .as_ref()
                .map_or(0, |external| external.history.len())
        })
        .sum();
    let archive = Archive {
        format: direct_core::ARCHIVE_FORMAT,
        workspace_id: project_id.to_owned(),
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
        planner_findings: Vec::new(),
        routines: Vec::new(),
        routine_occurrences: Vec::new(),
        routine_notices: Vec::new(),
        delivery_facts: Vec::new(),
        projects: vec![project],
        goals,
        milestones,
        labels: vec![],
        theoria_documents: vec![],
        method_findings: vec![],
        git_traces: vec![],
        releases: vec![],
        release_evidence: vec![],
        release_workflows: vec![],
        issue_links,
        templates: vec![],
        template_revisions: vec![],
        issues: direct_issues,
        comments,
        verifications: vec![],
        events: vec![],
        requests: vec![],
        source_bundles: vec![],
        source_files: vec![],
        source_records: vec![],
    };
    validate_archive(&archive).context("Generated Direct archive is invalid")?;

    let archive_bytes = pretty_bytes(&archive)?;
    let archive_sha256 = sha256(&archive_bytes);
    let source_manifest_sha256 = sha256(&fs::read(source.join("manifest.json"))?);
    let reconciliation = json!({
        "format": 1,
        "source": {
            "service": "Linear",
            "capture_format": manifest.format,
            "captured_at": manifest.captured_at,
            "manifest_sha256": source_manifest_sha256,
            "project_id": project_id,
            "project_name": string(source_project, "name"),
            "package_errors": manifest.errors,
            "missing_coverage": manifest.missing_coverage,
        },
        "source_counts": {
            "projects": 1,
            "goals": archive.goals.len(),
            "milestones": archive.milestones.len(),
            "releases": 0,
            "issues": archive.issues.len(),
            "comments": archive.comments.len(),
            "history_entries": history_entries,
            "relations_touching_project": touching_relations.len() + external_parent_links as usize,
            "relations_internal": internal_relations.len() + archive.issue_links.iter().filter(|link| link.kind == IssueLinkKind::Parent).count(),
            "relations_cross_boundary": touching_relations.len() - internal_relations.len() + external_parent_links as usize,
        },
        "imported_counts": {
            "products": archive.products.len(),
            "projects": archive.projects.len(),
            "goals": archive.goals.len(),
            "milestones": archive.milestones.len(),
            "issues": archive.issues.len(),
            "comments": archive.comments.len(),
            "history_entries": history_entries,
            "issue_links": archive.issue_links.len(),
            "verification_runs": archive.verifications.len(),
            "releases": archive.releases.len(),
            "release_evidence": archive.release_evidence.len(),
            "release_workflows": archive.release_workflows.len(),
            "legacy_completed": archive.issues.iter().filter(|issue| issue.status == Status::LegacyCompleted).count(),
        },
        "omitted_cross_boundary_relations": touching_relations.len() - internal_relations.len() + external_parent_links as usize,
        "unknown_issue_state_types": unknown_issue_states,
        "unknown_planning_state_types": unknown_goal_states,
        "unknown_relation_types": unknown_relation_types,
        "legacy_completion_policy": "Linear completed issues use legacy_completed with source state, timestamps, and history. No Direct verification run or passed result is created.",
        "release_policy": "The captured Linear project package has no release/deployment record type, so no Direct release or release evidence is invented. Releases can be added after import with explicit provenance and delivery evidence.",
        "archive": {
            "format": archive.format,
            "sha256": archive_sha256,
        }
    });
    let reconciliation_bytes = pretty_bytes(&reconciliation)?;

    let archive_path = output.join("direct-import.json");
    let report_path = output.join("reconciliation.json");
    let checksum_path = output.join("direct-import.sha256");
    let workspace = output.join("workspace");
    let idempotent_replay = if output.exists() {
        validate_replay_workspace(output)?;
        if !archive_path.is_file()
            || !report_path.is_file()
            || !workspace.join("direct.db").is_file()
        {
            bail!("Existing dry-run output is incomplete; choose a new output directory");
        }
        if fs::read(&archive_path)? != archive_bytes {
            bail!(
                "Existing dry-run archive differs from this source; choose a new output directory"
            );
        }
        let store = Store::open(&workspace.join("direct.db"))?;
        if serde_json::to_value(store.export()?)? != serde_json::to_value(&archive)? {
            bail!("Existing dry-run workspace differs from its deterministic import archive");
        }
        true
    } else {
        let parent = output
            .parent()
            .context("Dry-run output needs a parent directory")?;
        if !parent.exists() {
            bail!("Dry-run output parent does not exist: {}", parent.display());
        }
        fs::create_dir(output).context("Create isolated dry-run output")?;
        write_new(&archive_path, &archive_bytes)?;
        write_new(&report_path, &reconciliation_bytes)?;
        write_new(
            &checksum_path,
            format!("{archive_sha256}  direct-import.json\n").as_bytes(),
        )?;
        fs::create_dir(&workspace).context("Create isolated Direct workspace")?;
        direct::protect_dir(&workspace)?;
        let mut store = Store::open(&workspace.join("direct.db"))?;
        store.restore(archive.clone())?;
        if serde_json::to_value(store.export()?)? != serde_json::to_value(&archive)? {
            bail!("Imported workspace did not reproduce the generated archive");
        }
        false
    };

    Ok(json!({
        "verified": true,
        "verified_scope": "Source package integrity and a deterministic isolated restore of one project. Not whole-workspace accounting or cutover readiness.",
        "cutover_ready": false,
        "idempotent_replay": idempotent_replay,
        "duplicates_added": 0,
        "output": output,
        "archive": archive_path,
        "report": report_path,
        "workspace": workspace,
        "archive_sha256": archive_sha256,
        "reconciliation": reconciliation,
    }))
}

fn ensure_private_output(output: &Path, owner_data_dir: Option<&Path>) -> Result<()> {
    if !output.is_absolute() {
        bail!("Linear dry-run output must be an absolute path");
    }
    let repository = std::env::current_dir()?.canonicalize()?;
    let normalized = output
        .parent()
        .unwrap_or(output)
        .canonicalize()
        .unwrap_or_else(|_| output.to_path_buf());
    if normalized.starts_with(&repository) {
        bail!("Refusing to place private Linear import data inside the repository");
    }
    if let Some(data) = owner_data_dir {
        let data = resolved_path(data);
        let target = resolved_path(output);
        if target.starts_with(&data) || data.starts_with(&target) {
            bail!(
                "Refusing to place Linear import output in or over the Direct data directory {}; the owner's workspace is never replaced",
                data.display()
            );
        }
    }
    Ok(())
}

/// SQLite opens may upgrade a database and write sidecars. Reject redirected
/// replay paths before opening anything, including Windows directory junctions.
fn validate_replay_workspace(output: &Path) -> Result<()> {
    let workspace = output.join("workspace");
    for directory in [output, workspace.as_path()] {
        let metadata = fs::symlink_metadata(directory).with_context(|| {
            format!(
                "Existing import workspace is missing {}",
                directory.display()
            )
        })?;
        if redirected(&metadata) || !metadata.is_dir() {
            bail!("Existing import workspace contains a symlink, reparse point or non-directory at {}", directory.display());
        }
    }
    for name in [
        "direct.db",
        "direct.db-wal",
        "direct.db-shm",
        "direct.db-journal",
    ] {
        let path = workspace.join(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && name != "direct.db" => {
                continue
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Read existing import workspace path {}", path.display())
                })
            }
        };
        if redirected(&metadata) || !metadata.is_file() {
            bail!(
                "Existing import workspace contains a symlink, reparse point or non-file at {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn redirected(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

/// Resolve a path that may not exist yet through its nearest existing parent.
fn resolved_path(path: &Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => resolved_path(parent).join(name),
        _ => path.to_path_buf(),
    }
}

fn records(data: &Path, name: &str) -> Result<Vec<Value>> {
    let path = data.join(name);
    let file: CaptureFile = serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("Read captured Linear data {}", path.display()))?;
    Ok(file.records)
}

fn import_project(
    value: &Value,
    product_id: &str,
    unknown: &mut BTreeSet<String>,
) -> Result<Project> {
    let state = optional_string(value, "state").unwrap_or_else(|| "planned".into());
    let status = match state.as_str() {
        "planned" => ProjectStatus::Planned,
        "started" | "active" => ProjectStatus::Active,
        "paused" => ProjectStatus::Paused,
        "completed" | "done" => ProjectStatus::Completed,
        "canceled" | "cancelled" => ProjectStatus::Canceled,
        other => {
            unknown.insert(format!("project:{other}"));
            ProjectStatus::Planned
        }
    };
    Ok(Project {
        labels: vec![],
        template: None,
        id: required_string(value, "id")?,
        product_id: product_id.into(),
        name: required_string(value, "name")?,
        description: optional_string(value, "description").unwrap_or_default(),
        status,
        priority: priority(value),
        sort_order: (number(value, "sortOrder").unwrap_or(0.0) as i64).max(0),
        external_source: Some(SOURCE.into()),
        external_id: Some(required_string(value, "id")?),
        external_url: optional_string(value, "url"),
        version: 1,
        created_at: timestamp(value, "createdAt")?,
        updated_at: timestamp(value, "updatedAt")?,
    })
}

fn import_goal(
    value: &Value,
    product_id: &str,
    project_id: &str,
    unknown: &mut BTreeSet<String>,
) -> Result<Goal> {
    let raw = value
        .get("status")
        .and_then(|state| {
            state
                .as_str()
                .or_else(|| state.get("type").and_then(Value::as_str))
        })
        .unwrap_or("planned")
        .to_ascii_lowercase();
    let status = match raw.as_str() {
        "planned" => GoalStatus::Planned,
        "started" | "active" => GoalStatus::Active,
        "paused" => GoalStatus::Paused,
        "completed" | "done" => GoalStatus::Completed,
        "canceled" | "cancelled" => GoalStatus::Canceled,
        other => {
            unknown.insert(format!("goal:{other}"));
            GoalStatus::Planned
        }
    };
    Ok(Goal {
        id: required_string(value, "id")?,
        product_id: product_id.into(),
        name: required_string(value, "name")?,
        description: optional_string(value, "description").unwrap_or_default(),
        status,
        priority: priority(value),
        project_ids: vec![project_id.into()],
        external_source: Some(SOURCE.into()),
        external_id: Some(required_string(value, "id")?),
        external_url: optional_string(value, "url"),
        version: 1,
        created_at: timestamp(value, "createdAt")?,
        updated_at: timestamp(value, "updatedAt")?,
    })
}

fn import_milestone(value: &Value, project_id: &str) -> Result<Milestone> {
    Ok(Milestone {
        id: required_string(value, "id")?,
        project_id: project_id.into(),
        name: required_string(value, "name")?,
        description: optional_string(value, "description").unwrap_or_default(),
        sort_order: (number(value, "sortOrder").unwrap_or(0.0) as i64).max(0),
        external_source: Some(SOURCE.into()),
        external_id: Some(required_string(value, "id")?),
        external_url: None,
        version: 1,
        created_at: timestamp(value, "createdAt")?,
        updated_at: timestamp(value, "updatedAt")?,
    })
}

fn import_issue(
    value: &Value,
    product: &Product,
    project_id: &str,
    unknown: &mut BTreeSet<String>,
) -> Result<Issue> {
    let external_id = required_string(value, "id")?;
    let state = value.get("state").context("Linear issue has no state")?;
    let state_kind = required_string(state, "type")?;
    let status = match state_kind.as_str() {
        "completed" => Status::LegacyCompleted,
        "canceled" | "cancelled" | "duplicate" => Status::Canceled,
        "triage" | "backlog" | "unstarted" | "started" => Status::Backlog,
        other => {
            unknown.insert(other.to_owned());
            Status::Backlog
        }
    };
    let milestone_id = nested_string(value, &["projectMilestone", "id"]);
    let history = value
        .pointer("/history/nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let external = ExternalIssueRecord {
        source: SOURCE.into(),
        id: external_id,
        url: required_string(value, "url")?,
        state: ExternalIssueState {
            id: required_string(state, "id")?,
            name: required_string(state, "name")?,
            kind: state_kind,
        },
        created_at: required_string(value, "createdAt")?,
        updated_at: required_string(value, "updatedAt")?,
        started_at: optional_string(value, "startedAt"),
        completed_at: optional_string(value, "completedAt"),
        canceled_at: optional_string(value, "canceledAt"),
        archived_at: optional_string(value, "archivedAt"),
        history,
    };
    Ok(Issue {
        intake: None,
        labels: vec![],
        template: None,
        id: required_string(value, "id")?,
        key: required_string(value, "identifier")?,
        product_id: product.id.clone(),
        project_id: Some(project_id.into()),
        milestone_id,
        planning_scope: PlanningScope::Project,
        theoria_refs: vec![],
        title: required_string(value, "title")?,
        body: optional_string(value, "description").unwrap_or_default(),
        acceptance: String::new(),
        owner: nested_string(value, &["assignee", "name"]).unwrap_or_default(),
        priority: priority(value),
        status,
        version: 1,
        created_at: timestamp(value, "createdAt")?,
        updated_at: timestamp(value, "updatedAt")?,
        claim: None,
        needs_fix: false,
        parent: None,
        verification_key: None,
        current_run: None,
        external: Some(external),
    })
}

fn import_comment(value: &Value, issue_keys: &HashMap<String, String>) -> Result<Comment> {
    let issue_id = nested_string(value, &["issue", "id"])
        .or_else(|| optional_string(value, "issueId"))
        .context("Linear comment has no issue")?;
    Ok(Comment {
        id: required_string(value, "id")?,
        issue_key: issue_keys
            .get(&issue_id)
            .cloned()
            .context("Linear comment references an unselected issue")?,
        actor: nested_string(value, &["user", "name"]).unwrap_or_else(|| "Linear".into()),
        body: optional_string(value, "body").unwrap_or_default(),
        at: timestamp(value, "createdAt")?,
        external_source: Some(SOURCE.into()),
        external_id: Some(required_string(value, "id")?),
        external_url: optional_string(value, "url"),
    })
}

fn import_relation(
    value: &Value,
    issue_keys: &HashMap<String, String>,
) -> Result<Option<IssueLink>> {
    let issue_id = nested_string(value, &["issue", "id"]).context("Relation has no issue")?;
    let related_id =
        nested_string(value, &["relatedIssue", "id"]).context("Relation has no related issue")?;
    let relation_type = required_string(value, "type")?;
    let (source_id, target_id, kind) = match relation_type.as_str() {
        "blocks" => (&related_id, &issue_id, IssueLinkKind::BlockedBy),
        "related" => (&issue_id, &related_id, IssueLinkKind::Related),
        _ => return Ok(None),
    };
    Ok(Some(IssueLink {
        id: required_string(value, "id")?,
        source_key: issue_keys
            .get(source_id)
            .cloned()
            .context("Relation source is outside selection")?,
        target_key: issue_keys
            .get(target_id)
            .cloned()
            .context("Relation target is outside selection")?,
        kind,
        external_source: Some(SOURCE.into()),
        external_id: Some(required_string(value, "id")?),
        created_by: "linear-import".into(),
        created_at: timestamp(value, "createdAt")?,
    }))
}

fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn required_string(value: &Value, key: &str) -> Result<String> {
    let value = string(value, key);
    if value.trim().is_empty() {
        bail!("Linear record is missing {key}");
    }
    Ok(value.to_owned())
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn nested_string(value: &Value, path: &[&str]) -> Option<String> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

fn priority(value: &Value) -> String {
    match value.get("priority").and_then(Value::as_i64).unwrap_or(0) {
        1 => "urgent",
        2 => "high",
        3 => "medium",
        4 => "low",
        _ => "low",
    }
    .into()
}

fn timestamp(value: &Value, key: &str) -> Result<i64> {
    let raw = required_string(value, key)?;
    Ok(DateTime::parse_from_rfc3339(&raw)
        .with_context(|| format!("Linear {key} is not an RFC 3339 timestamp"))?
        .timestamp())
}

fn deterministic_uuid(namespace: &str, value: &str) -> String {
    let digest = Sha256::digest(format!("{namespace}:{value}").as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn pretty_bytes(value: &impl serde::Serialize) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, fs};
    use tempfile::tempdir;

    fn write_json(path: &Path, value: &Value) {
        fs::write(
            path,
            format!("{}\n", serde_json::to_string_pretty(value).unwrap()),
        )
        .unwrap();
    }

    fn fixture(root: &Path, issue_state: &str, include_comments: bool) {
        fs::create_dir_all(root.join("data")).unwrap();
        let created = "2026-01-01T00:00:00Z";
        let project_id = "11111111-1111-4111-8111-111111111111";
        let issue_id = "22222222-2222-4222-8222-222222222222";
        let files = BTreeMap::from([
            (
                "teams.json",
                json!([{"id":"33333333-3333-4333-8333-333333333333","key":"TST","name":"Test"}]),
            ),
            (
                "projects.json",
                json!([{"id":project_id,"name":"Pilot","description":"Bounded","state":"planned","priority":2,"sortOrder":1,"createdAt":created,"updatedAt":created,"url":"https://linear.example/project","teams":{"nodes":[{"id":"33333333-3333-4333-8333-333333333333","key":"TST","name":"Test"}]}}]),
            ),
            ("project-milestones.json", json!([])),
            ("initiatives.json", json!([])),
            (
                "issues.json",
                json!([{"id":issue_id,"identifier":"TST-1","title":"Imported","description":"Body","priority":2,"createdAt":created,"updatedAt":created,"completedAt":if issue_state=="completed" {Some(created)} else {None},"canceledAt":null,"archivedAt":null,"startedAt":null,"url":"https://linear.example/issue","project":{"id":project_id},"team":{"id":"33333333-3333-4333-8333-333333333333","key":"TST","name":"Test"},"state":{"id":"44444444-4444-4444-8444-444444444444","name":"Source state","type":issue_state},"history":{"nodes":[{"id":"h1","createdAt":created}]}}]),
            ),
            (
                "comments.json",
                if include_comments {
                    json!([{"id":"55555555-5555-4555-8555-555555555555","body":"History","createdAt":created,"url":"https://linear.example/comment","issue":{"id":issue_id},"user":{"name":"Owner"}}])
                } else {
                    json!([])
                },
            ),
            ("issue-relations.json", json!([])),
            ("workflow-states.json", json!([])),
        ]);
        let mut checksums = Vec::new();
        for (name, records) in files {
            let value = json!({"captured_at":created,"count":records.as_array().unwrap().len(),"records":records});
            let path = root.join("data").join(name);
            write_json(&path, &value);
            let bytes = fs::read(&path).unwrap();
            checksums.push(
                json!({"path":format!("data/{name}"),"bytes":bytes.len(),"sha256":sha256(&bytes)}),
            );
        }
        fs::write(root.join("attachment-manifest.json"), "[]\n").unwrap();
        let manifest = json!({"format":1,"captured_at":created,"errors":[],"missing_coverage":["deleted records unavailable"],"integrity":{"data_files":checksums}});
        write_json(&root.join("manifest.json"), &manifest);
        let bytes = fs::read(root.join("manifest.json")).unwrap();
        fs::write(
            root.join("manifest.sha256"),
            format!("{}  manifest.json\n", sha256(&bytes)),
        )
        .unwrap();
    }

    #[test]
    fn dry_run_is_idempotent_and_preserves_legacy_completion_without_verification() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        fixture(&source, "completed", true);
        let output = temp.path().join("output");
        let project = "11111111-1111-4111-8111-111111111111";
        let first = dry_run(&source, project, &output, None).unwrap();
        assert_eq!(first["idempotent_replay"], false);
        let second = dry_run(&source, project, &output, None).unwrap();
        assert_eq!(second["idempotent_replay"], true);
        assert_eq!(second["duplicates_added"], 0);
        let archive: Archive =
            serde_json::from_slice(&fs::read(output.join("direct-import.json")).unwrap()).unwrap();
        assert_eq!(archive.issues.len(), 1);
        assert_eq!(archive.comments.len(), 1);
        assert_eq!(archive.issues[0].status, Status::LegacyCompleted);
        assert_eq!(
            archive.issues[0].external.as_ref().unwrap().history.len(),
            1
        );
        assert!(archive.verifications.is_empty());
        let mut store = Store::open(&output.join("workspace/direct.db")).unwrap();
        let snapshot = store
            .execute(
                direct_core::Request {
                    actor: "test".into(),
                    request_id: String::new(),
                    command: direct_core::Command::Snapshot,
                },
                direct_core::Role::Agent,
            )
            .unwrap();
        assert_eq!(snapshot["project_progress"][0]["completed"], 0);
        assert_eq!(snapshot["project_progress"][0]["legacy_completed"], 1);
        assert_eq!(snapshot["project_progress"][0]["completion_percent"], 0);
    }

    #[test]
    fn unknown_states_are_reported_and_missing_files_fail_clearly() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        fixture(&source, "custom_state", false);
        let output = temp.path().join("output");
        let result = dry_run(
            &source,
            "11111111-1111-4111-8111-111111111111",
            &output,
            None,
        )
        .unwrap();
        assert_eq!(
            result["reconciliation"]["unknown_issue_state_types"][0],
            "custom_state"
        );

        let broken = temp.path().join("broken");
        fixture(&broken, "completed", false);
        fs::remove_file(broken.join("data/comments.json")).unwrap();
        let error = dry_run(
            &broken,
            "11111111-1111-4111-8111-111111111111",
            &temp.path().join("broken-output"),
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("data/comments.json"));
    }
}
