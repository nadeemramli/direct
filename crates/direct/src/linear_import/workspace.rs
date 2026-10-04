//! Whole-workspace Linear migration rehearsal.
//!
//! Every team, project, initiative, milestone and issue (including unprojected
//! and archived records) is reconciled into ONE isolated Direct workspace. The
//! verified source package is retained byte for byte beside the generated
//! archive, every source record receives an accounting entry, and cutover
//! readiness stays blocked while any information lacks native Direct access.

use super::{
    deterministic_uuid, ensure_private_output, index, nested_string, optional_string,
    package::{reread, verify_package, VerifiedPackage},
    pretty_bytes, required_string, resolved_path, retained, sha256, string, timestamp, write_new,
    SOURCE,
};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use direct_core::{
    validate_archive, Archive, Comment, ExternalIssueRecord, ExternalIssueState, Goal, GoalStatus,
    Issue, IssueLink, IssueLinkKind, Label, LabelProductRule, LinearLabelOrigin, Milestone,
    PlanningScope, Product, Project, ProjectStatus, Status, Store,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    path::Path,
};
use uuid::Uuid;

const TEAMS: &str = "teams.json";
const STATES: &str = "workflow-states.json";
const LABELS: &str = "issue-labels.json";
const PROJECTS: &str = "projects.json";
const MILESTONES: &str = "project-milestones.json";
const INITIATIVES: &str = "initiatives.json";
const ISSUES: &str = "issues.json";
const COMMENTS: &str = "comments.json";
const RELATIONS: &str = "issue-relations.json";
const DOCUMENTS: &str = "documents.json";
const USERS: &str = "users.json";
const LINK_ATTACHMENTS: &str = "attachments.json";
const PROJECT_UPDATES: &str = "project-updates.json";
const INITIATIVE_UPDATES: &str = "initiative-updates.json";
const SCHEMA: &str = "graphql-schema.json";

pub(super) const WHOLE_REQUIRED: &[&str] = &[
    TEAMS,
    STATES,
    LABELS,
    PROJECTS,
    MILESTONES,
    INITIATIVES,
    ISSUES,
    COMMENTS,
    RELATIONS,
    DOCUMENTS,
    USERS,
    LINK_ATTACHMENTS,
];
const EXPECTED_OPTIONAL: &[&str] = &[PROJECT_UPDATES, INITIATIVE_UPDATES, SCHEMA];

pub(super) const BUNDLE_DIR: &str = "source-bundle";
const ARCHIVE_FILE: &str = "direct-import.json";
const ARCHIVE_CHECKSUM: &str = "direct-import.sha256";
const REPORT_FILE: &str = "reconciliation.json";
const ACCOUNTING_FILE: &str = "accounting.json";
const INDEX_FILE: &str = "index.html";
const BUNDLE_SUMS: &str = "source-bundle.sha256";
const MARKER_FILE: &str = "import-complete.json";

const SECONDS: &str =
    "Stored as whole seconds; the exact RFC 3339 string is retained in the source bundle";
const PRESERVED: &str = "No native Direct field; the exact value is retained in the source bundle";
const NATIVE_ACCESS: &str = "Retained byte for byte and readable in Direct's Imported sources view (original record, search, issue links and file download); it is not converted into Direct planning records.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Class {
    Native,
    Transformed,
    Preserved,
    Unresolved,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Class::Native => "native",
            Class::Transformed => "transformed",
            Class::Preserved => "preserved",
            Class::Unresolved => "unresolved",
        }
    }
}

/// One accounting line. `record` entries cover each source record exactly once;
/// `component` entries cover parts of a record (assignments, parents, previous
/// identifiers, uploads); `file` entries cover each bundled data file.
#[derive(Clone, Debug, Serialize)]
pub(super) struct Entry {
    pub level: &'static str,
    pub kind: String,
    pub file: String,
    pub pointer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
    pub classification: Class,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub direct: Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub preserved_fields: Vec<String>,
}

pub(super) struct Plan {
    pub archive: Archive,
    pub entries: Vec<Entry>,
    pub reconciliation: Value,
    /// Linear issue ID → Direct issue key.
    pub issue_keys: HashMap<String, String>,
    pub reused_products: Vec<String>,
    pub reused_labels: Vec<String>,
    /// Linear label ID → existing Direct label ID reused for it.
    pub reused_label_origins: BTreeMap<String, String>,
}

/// Bounded summary stored with a retained bundle in Direct.
pub(super) fn bundle_summary(report: &Value) -> Value {
    let mut summary = serde_json::Map::new();
    for field in [
        "mode",
        "source",
        "source_counts",
        "imported_counts",
        "accounting",
        "classification_totals",
        "cutover_readiness",
        "native_access",
        "policies",
        "unsupported_fields",
        "label_disambiguations",
        "name_disambiguations",
        "cross_product_mappings",
        "previous_identifiers",
        "unknown_state_types",
        "source_limitations",
        "relations",
    ] {
        if let Some(value) = report.get(field) {
            summary.insert(field.into(), value.clone());
        }
    }
    // Keep the summary inside Direct's bound; long lists stay in reconciliation.json.
    for field in [
        "relations",
        "cross_product_mappings",
        "previous_identifiers",
        "label_disambiguations",
        "name_disambiguations",
        "unsupported_fields",
    ] {
        if serde_json::to_vec(&summary).map_or(0, |bytes| bytes.len()) <= 400_000 {
            break;
        }
        summary.insert(field.into(), json!({"omitted": "see reconciliation.json"}));
    }
    Value::Object(summary)
}

pub fn whole_workspace(
    source: &Path,
    output: &Path,
    owner_data_dir: Option<&Path>,
) -> Result<Value> {
    ensure_private_output(output, owner_data_dir)?;
    let source_root = source
        .canonicalize()
        .context("Linear source package does not exist")?;
    let target = resolved_path(output);
    if target.starts_with(&source_root) || source_root.starts_with(&target) {
        bail!("The import output must not overlap the Linear source package");
    }
    let package = verify_package(&source_root, WHOLE_REQUIRED)?;
    let mut plan = build(&package, &Target::default())?;
    let retained = retained::build(
        &package,
        &plan.entries,
        &plan.issue_keys,
        bundle_summary(&plan.reconciliation),
        "linear-import-dry-run",
        chrono::DateTime::parse_from_rfc3339(&package.manifest.captured_at)
            .map(|time| time.timestamp())
            .unwrap_or(0),
    )?;
    plan.archive.source_bundles = vec![retained.bundle];
    plan.archive.source_files = retained
        .files
        .into_iter()
        .map(|(mut file, bytes)| {
            file.data = Some(STANDARD.encode(bytes));
            file
        })
        .collect();
    plan.archive.source_records = retained.records;
    plan.archive.source_records.sort_by(|a, b| a.id.cmp(&b.id));
    validate_archive(&plan.archive).context("Generated Direct archive is invalid")?;
    let archive_bytes = pretty_bytes(&plan.archive)?;
    let generated = generated_files(&package, &plan, &archive_bytes)?;

    let replay = fs::symlink_metadata(output).is_ok();
    if !replay {
        publish(&package, &generated, &plan.archive, output)?;
    }
    let restore = verify_output(&package, &generated, &archive_bytes, output)?;
    let report = &plan.reconciliation;
    Ok(json!({
        "mode": "whole_workspace",
        "idempotent_replay": replay,
        "duplicates_added": 0,
        "output": output,
        "archive": output.join(ARCHIVE_FILE),
        "report": output.join(REPORT_FILE),
        "accounting": output.join(ACCOUNTING_FILE),
        "index": output.join(INDEX_FILE),
        "source_bundle": output.join(BUNDLE_DIR),
        "workspace": output.join("workspace"),
        "archive_sha256": sha256(&archive_bytes),
        "rehearsal": {
            "status": "complete",
            "scope": "Offline preparation only: source integrity, deterministic archive, isolated restore, retained bundle and accounting. Not a cutover.",
            "checks": {
                "source_manifest_checksum": true,
                "data_files_verified": package.files.keys().filter(|path| path.starts_with("data/")).count(),
                "uploaded_files_verified": package.files.keys().filter(|path| path.starts_with("attachments/")).count(),
                "capture_errors_other_than_recorded_upload_failures": 0,
                "truncated_or_failed_nested_connections": 0,
                "archive_valid": true,
                "records_accounted": report["accounting"]["records_accounted"],
                "records_in_source": report["accounting"]["records_in_source"],
                "restore_semantic_match": restore.semantic,
                "restore_byte_match": restore.bytes,
                "bundle_files_verified": restore.bundle_files,
            },
        },
        "cutover_readiness": report["cutover_readiness"],
        "classification_totals": report["classification_totals"],
        "source_counts": report["source_counts"],
        "imported_counts": report["imported_counts"],
    }))
}

struct RestoreCheck {
    semantic: bool,
    bytes: bool,
    bundle_files: usize,
}

fn generated_files(
    package: &VerifiedPackage,
    plan: &Plan,
    archive_bytes: &[u8],
) -> Result<BTreeMap<&'static str, Vec<u8>>> {
    let archive_sha256 = sha256(archive_bytes);
    let accounting = json!({
        "format": 1,
        "classes": {
            "native": "Represented by a Direct record or field.",
            "transformed": "Represented in Direct after a reported change (mapping, rename, split, merge or normalization).",
            "preserved": "Retained only in the source bundle at the given pointer; no native Direct access.",
            "unresolved": "Could not be represented or is missing; needs a decision before cutover.",
        },
        "entries": plan.entries,
    });
    let mut sums = String::new();
    for (path, file) in &package.files {
        sums.push_str(&format!("{}  {path}\n", file.sha256.to_ascii_lowercase()));
    }
    let mut files = BTreeMap::new();
    files.insert(ARCHIVE_FILE, archive_bytes.to_vec());
    files.insert(
        ARCHIVE_CHECKSUM,
        format!("{archive_sha256}  {ARCHIVE_FILE}\n").into_bytes(),
    );
    files.insert(REPORT_FILE, pretty_bytes(&plan.reconciliation)?);
    files.insert(ACCOUNTING_FILE, pretty_bytes(&accounting)?);
    files.insert(
        INDEX_FILE,
        index::render(package, &plan.entries, &plan.reconciliation).into_bytes(),
    );
    files.insert(BUNDLE_SUMS, sums.into_bytes());
    let digests: BTreeMap<_, _> = files
        .iter()
        .map(|(name, bytes)| (*name, sha256(bytes)))
        .collect();
    let marker = json!({
        "format": 1,
        "mode": "linear_whole_workspace",
        "source_manifest_sha256": package.manifest_sha256,
        "archive_sha256": archive_sha256,
        "generated_files": digests,
        "source_bundle_files": package.files.len(),
    });
    files.insert(MARKER_FILE, pretty_bytes(&marker)?);
    Ok(files)
}

/// Stage everything beside the output and publish it with one rename, so an
/// output directory is either complete or absent.
fn publish(
    package: &VerifiedPackage,
    generated: &BTreeMap<&'static str, Vec<u8>>,
    archive: &Archive,
    output: &Path,
) -> Result<()> {
    let parent = output
        .parent()
        .context("Import output needs a parent directory")?;
    if !parent.is_dir() {
        bail!("Import output parent does not exist: {}", parent.display());
    }
    let name = output
        .file_name()
        .context("Import output needs a directory name")?
        .to_string_lossy();
    let staging = parent.join(format!(".{name}.partial-{}", Uuid::new_v4().simple()));
    fs::create_dir(&staging).context("Create the staging directory for the import output")?;
    let result = (|| -> Result<()> {
        direct::protect_dir(&staging)?;
        for (file, bytes) in generated {
            if *file != MARKER_FILE {
                write_new(&staging.join(file), bytes)?;
            }
        }
        let bundle = staging.join(BUNDLE_DIR);
        fs::create_dir(&bundle)?;
        for (relative, checksum) in &package.files {
            let bytes = reread(&package.root, checksum)?;
            let destination = bundle.join(relative);
            if let Some(directory) = destination.parent() {
                fs::create_dir_all(directory)?;
            }
            write_new(&destination, &bytes)?;
        }
        let workspace = staging.join("workspace");
        fs::create_dir(&workspace).context("Create isolated Direct workspace")?;
        direct::protect_dir(&workspace)?;
        {
            let mut store = Store::open(&workspace.join("direct.db"))?;
            store.restore(archive.clone())?;
            let restored = pretty_bytes(&store.export()?)?;
            if restored != generated[ARCHIVE_FILE] {
                bail!("Imported workspace did not reproduce the generated archive byte for byte");
            }
        }
        write_new(&staging.join(MARKER_FILE), &generated[MARKER_FILE])?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) = fs::rename(&staging, output) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error).context("Publish the completed import output");
    }
    Ok(())
}

/// Verify an existing output against a fresh deterministic generation. Used
/// after publishing (from a reopened store, i.e. a restart) and on replay.
fn verify_output(
    package: &VerifiedPackage,
    generated: &BTreeMap<&'static str, Vec<u8>>,
    archive_bytes: &[u8],
    output: &Path,
) -> Result<RestoreCheck> {
    let metadata = fs::symlink_metadata(output)?;
    if super::redirected(&metadata) || !metadata.is_dir() {
        bail!("Existing import output is a symlink, reparse point or non-directory; choose a new output directory");
    }
    let marker = output.join(MARKER_FILE);
    if !marker.is_file() {
        bail!("Existing import output is incomplete (no completion marker); choose a new output directory");
    }
    if fs::read(&marker)? != generated[MARKER_FILE] {
        bail!("Existing import output was generated from a different source package or importer version; choose a new output directory");
    }
    for (file, bytes) in generated {
        let existing = fs::read(output.join(file))
            .with_context(|| format!("Existing import output is missing {file}"))?;
        if existing != *bytes {
            bail!("Existing import output file {file} differs from this source; choose a new output directory");
        }
    }
    let bundle = output.join(BUNDLE_DIR);
    let mut found = BTreeSet::new();
    collect_files(&bundle, "", &mut found)?;
    let expected: BTreeSet<_> = package.files.keys().cloned().collect();
    if found != expected {
        let extra: Vec<_> = found.difference(&expected).cloned().collect();
        let missing: Vec<_> = expected.difference(&found).cloned().collect();
        bail!(
            "Retained source bundle does not match the source package (missing: {}; unexpected: {})",
            missing.join(", "),
            extra.join(", ")
        );
    }
    for (relative, checksum) in &package.files {
        let bytes = fs::read(bundle.join(relative))?;
        if bytes.len() as u64 != checksum.bytes
            || !checksum.sha256.eq_ignore_ascii_case(&sha256(&bytes))
        {
            bail!("Retained source bundle file {relative} failed its checksum");
        }
    }
    let database = output.join("workspace").join("direct.db");
    if !database.is_file() {
        bail!("Existing import output has no isolated workspace database");
    }
    super::validate_replay_workspace(output)?;
    let store = Store::open(&database)?;
    let exported = store.export()?;
    let archive: Archive = serde_json::from_slice(archive_bytes)?;
    let semantic = serde_json::to_value(&exported)? == serde_json::to_value(&archive)?;
    let bytes = pretty_bytes(&exported)? == archive_bytes;
    if !semantic || !bytes {
        bail!("Existing isolated workspace differs from its deterministic import archive; choose a new output directory");
    }
    Ok(RestoreCheck {
        semantic,
        bytes,
        bundle_files: found.len(),
    })
}

fn collect_files(directory: &Path, prefix: &str, found: &mut BTreeSet<String>) -> Result<()> {
    let metadata = fs::symlink_metadata(directory)
        .context("Existing import output has no retained source bundle")?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("Retained source bundle contains a non-directory or symlink at {prefix:?}");
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let kind = fs::symlink_metadata(entry.path())?.file_type();
        if kind.is_symlink() {
            bail!("Retained source bundle contains a symlink at {relative}");
        }
        if kind.is_dir() {
            collect_files(&entry.path(), &relative, found)?;
        } else {
            found.insert(relative);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Mapping

#[derive(Default)]
struct Reports {
    label_disambiguations: Vec<Value>,
    name_disambiguations: Vec<Value>,
    cross_product: Vec<Value>,
    relations_deduplicated: Vec<Value>,
    relations_unsupported: Vec<Value>,
    relations_unresolved: Vec<Value>,
    relations_archived: Vec<Value>,
    previous_identifiers: Vec<Value>,
    unknown_states: BTreeSet<String>,
}

/// The existing workspace an import will merge into. Empty for an isolated rehearsal.
#[derive(Default)]
pub(super) struct Target {
    pub baseline_cursor: u64,
    pub products: Vec<Product>,
    /// Explicit Linear team ID → existing Direct product ID mapping.
    pub team_map: HashMap<String, String>,
    pub labels: Vec<Label>,
    /// Existing issue keys plus keys reserved by activity history.
    pub keys: HashSet<String>,
    pub project_names: HashSet<(String, String)>,
    pub goal_names: HashSet<(String, String)>,
    pub ids: HashSet<String>,
}

impl Target {
    fn highest_number(&self, product_key: &str) -> u64 {
        let prefix = format!("{product_key}-");
        self.keys
            .iter()
            .filter_map(|key| key.strip_prefix(&prefix)?.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
    }
}

#[derive(Clone)]
enum LabelScope {
    Workspace,
    Team(String),
    /// Reused existing label: Direct product IDs it applies to (empty = all).
    Products(Vec<String>),
}

struct Builder<'a> {
    package: &'a VerifiedPackage,
    target: &'a Target,
    /// Linear team ID → Direct product ID (the team ID unless mapped).
    product_ids: HashMap<String, String>,
    reused_products: BTreeSet<String>,
    reused_labels: BTreeSet<String>,
    /// Linear issue ID → original identifier, for issues given a new Direct key.
    renumbered: HashMap<String, String>,
    entries: Vec<Entry>,
    reports: Reports,
    products: Vec<Product>,
    team_keys: HashMap<String, String>,
    projects: Vec<Project>,
    project_products: HashMap<String, String>,
    goals: Vec<Goal>,
    initiative_goals: HashMap<String, Vec<String>>,
    milestones: Vec<Milestone>,
    milestone_projects: HashMap<String, String>,
    labels: Vec<Label>,
    label_map: HashMap<String, (String, LabelScope)>,
    issues: Vec<Issue>,
    issue_keys: HashMap<String, String>,
    issue_products: HashMap<String, String>,
    comments: Vec<Comment>,
    links: Vec<IssueLink>,
    /// Source relation IDs (or parent pairs) represented by each Direct link.
    link_sources: HashMap<String, String>,
}

pub(super) fn build(package: &VerifiedPackage, target: &Target) -> Result<Plan> {
    let mut builder = Builder {
        package,
        target,
        product_ids: HashMap::new(),
        reused_products: BTreeSet::new(),
        reused_labels: BTreeSet::new(),
        renumbered: HashMap::new(),
        entries: Vec::new(),
        reports: Reports::default(),
        products: Vec::new(),
        team_keys: HashMap::new(),
        projects: Vec::new(),
        project_products: HashMap::new(),
        goals: Vec::new(),
        initiative_goals: HashMap::new(),
        milestones: Vec::new(),
        milestone_projects: HashMap::new(),
        labels: Vec::new(),
        label_map: HashMap::new(),
        issues: Vec::new(),
        issue_keys: HashMap::new(),
        issue_products: HashMap::new(),
        comments: Vec::new(),
        links: Vec::new(),
        link_sources: HashMap::new(),
    };
    builder.teams()?;
    builder.workflow_states();
    builder.labels()?;
    builder.projects()?;
    builder.milestones()?;
    builder.initiatives()?;
    builder.issues()?;
    builder.comments()?;
    builder.parents()?;
    builder.relations()?;
    builder.preserved_only();
    builder.uploads();
    builder.data_files();
    builder.finish()
}

impl<'a> Builder<'a> {
    /// Existing names in mapped products, keyed by the Linear team scope used for naming.
    fn reserved_names(&self, names: &HashSet<(String, String)>) -> HashSet<(String, String)> {
        let mut reserved = HashSet::new();
        for (team, product) in &self.product_ids {
            for (product_id, name) in names {
                if product_id == product {
                    reserved.insert((team.clone(), name.clone()));
                }
            }
        }
        reserved
    }

    fn record(
        &mut self,
        file: &str,
        index: usize,
        value: &Value,
        classification: Class,
        direct: Value,
        reasons: Vec<String>,
    ) {
        let kind = kind_for(file);
        let preserved = preserved_fields(&kind, value);
        let mut classification = classification;
        let mut reasons = reasons;
        if classification == Class::Native && natively_imported(&kind) && !preserved.is_empty() {
            classification = Class::Transformed;
            reasons.push(format!(
                "Populated source fields without a native Direct field are retained only in the source bundle: {}",
                preserved.join(", ")
            ));
        }
        self.entries.push(Entry {
            level: "record",
            kind: kind.clone(),
            file: format!("data/{file}"),
            pointer: format!("/records/{index}"),
            source_id: optional_string(value, "id"),
            source_label: source_label(value),
            source_title: optional_string(value, "title")
                .filter(|title| Some(title) != source_label(value).as_ref()),
            classification,
            direct,
            reasons,
            preserved_fields: preserved,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn component(
        &mut self,
        kind: &str,
        file: &str,
        pointer: String,
        source_id: Option<String>,
        source_label: Option<String>,
        classification: Class,
        direct: Value,
        reasons: Vec<String>,
    ) {
        self.entries.push(Entry {
            level: "component",
            kind: kind.into(),
            file: format!("data/{file}"),
            pointer,
            source_id,
            source_label,
            source_title: None,
            classification,
            direct,
            reasons,
            preserved_fields: Vec::new(),
        });
    }

    fn teams(&mut self) -> Result<()> {
        let package = self.package;
        let mut keys = HashSet::new();
        for (index, team) in package.records(TEAMS).iter().enumerate() {
            let id = required_string(team, "id")?;
            let key = required_string(team, "key")?;
            let name = required_string(team, "name")?;
            if !keys.insert(key.clone()) || self.team_keys.contains_key(&id) {
                bail!("Linear teams repeat the key or id of team {key}");
            }
            let mut class = Class::Native;
            let mut reasons = Vec::new();
            if has_value(team.get("archivedAt")) {
                class = Class::Transformed;
                reasons.push("Archived in Linear; Direct products have no archived state, so archivedAt is retained in the source bundle".into());
            }
            if let Some(product_id) = self.target.team_map.get(&id) {
                let product = self
                    .target
                    .products
                    .iter()
                    .find(|product| product.id == *product_id)
                    .with_context(|| {
                        format!("Team mapping for {key} names unknown product {product_id}")
                    })?;
                self.team_keys.insert(id.clone(), product.key.clone());
                self.product_ids.insert(id.clone(), product.id.clone());
                self.reused_products.insert(product.id.clone());
                reasons.push(format!(
                    "Explicitly mapped to existing Direct product {} ({}); that product record is not changed",
                    product.key, product.name
                ));
                self.record(
                    TEAMS,
                    index,
                    team,
                    Class::Transformed,
                    json!({"entity": "product", "id": product.id, "key": product.key, "existing": true}),
                    reasons,
                );
                continue;
            }
            if let Some(existing) = self
                .target
                .products
                .iter()
                .find(|product| product.key.eq_ignore_ascii_case(&key) || product.id == id)
            {
                bail!(
                    "Linear team {key} collides with existing Direct product {} ({}). Import it into that product explicitly with --map-team {id}={}, or rename one of them first",
                    existing.key, existing.name, existing.id
                );
            }
            self.team_keys.insert(id.clone(), key.clone());
            self.product_ids.insert(id.clone(), id.clone());
            self.products.push(Product {
                id: id.clone(),
                key: key.clone(),
                name,
                repo_windows: String::new(),
                repo_wsl: String::new(),
                vault_windows: String::new(),
                vault_wsl: String::new(),
                sort_order: 0,
                section_id: None,
            });
            self.record(
                TEAMS,
                index,
                team,
                class,
                json!({"entity": "product", "id": id, "key": key}),
                reasons,
            );
        }
        if self.team_keys.is_empty() {
            bail!("Linear source package contains no teams");
        }
        for team in self.target.team_map.keys() {
            if !self.team_keys.contains_key(team) {
                bail!("--map-team names Linear team {team}, which is not in this capture");
            }
        }
        Ok(())
    }

    fn workflow_states(&mut self) {
        let package = self.package;
        let mut usage: HashMap<String, usize> = HashMap::new();
        for issue in package.records(ISSUES) {
            if let Some(id) = nested_string(issue, &["state", "id"]) {
                *usage.entry(id).or_default() += 1;
            }
        }
        for (index, state) in package.records(STATES).iter().enumerate() {
            let kind = string(state, "type");
            let mapped = map_state(kind, true);
            let mut reasons = vec![format!(
                "Linear {kind:?} state maps to Direct {}; {} issue(s) keep this state's id, name and type in external provenance",
                status_name(&mapped.status),
                usage.get(string(state, "id")).copied().unwrap_or(0)
            )];
            let class = if mapped.unresolved {
                self.reports.unknown_states.insert(kind.to_owned());
                reasons.push("Unknown Linear state type; issues in this state are imported as backlog pending a mapping decision".into());
                Class::Unresolved
            } else {
                Class::Transformed
            };
            self.record(
                STATES,
                index,
                state,
                class,
                json!({"entity": "status_mapping", "status": status_name(&mapped.status)}),
                reasons,
            );
        }
    }

    fn labels(&mut self) -> Result<()> {
        let package = self.package;
        struct Candidate {
            index: usize,
            id: String,
            original: String,
            team: Option<(String, String)>,
        }
        let mut candidates = Vec::new();
        for (index, label) in package.records(LABELS).iter().enumerate() {
            let id = required_string(label, "id")?;
            if let Some(existing) = self.target.labels.iter().find(|existing| {
                existing
                    .linear_origins
                    .iter()
                    .any(|origin| origin.id == id.trim())
            }) {
                self.reused_labels.insert(existing.id.clone());
                self.label_map.insert(
                    id.clone(),
                    (
                        existing.id.clone(),
                        LabelScope::Products(
                            existing
                                .products
                                .iter()
                                .map(|rule| rule.product_id.clone())
                                .collect(),
                        ),
                    ),
                );
                self.record(LABELS, index, label, Class::Transformed, json!({"entity": "label", "id": existing.id, "name": existing.name, "existing": true}), vec![format!("Existing Direct label {:?} already records this Linear label as its origin; assignments use it and the label is not changed", existing.name)]);
                continue;
            }
            let team_id = nested_string(label, &["team", "id"]);
            let team = match team_id {
                Some(team_id) => match self.team_keys.get(&team_id) {
                    Some(key) => Some((team_id, key.clone())),
                    None => {
                        self.record(LABELS, index, label, Class::Unresolved, Value::Null, vec![format!("Label belongs to team {team_id}, which is absent from teams.json; not imported")]);
                        continue;
                    }
                },
                None => None,
            };
            if id.trim().len() > 120 {
                self.record(
                    LABELS,
                    index,
                    label,
                    Class::Unresolved,
                    Value::Null,
                    vec!["Linear label id exceeds Direct's 120-byte origin limit".into()],
                );
                continue;
            }
            candidates.push(Candidate {
                index,
                id,
                original: string(label, "name").trim().to_owned(),
                team,
            });
        }
        let mut groups: HashMap<String, usize> = HashMap::new();
        for candidate in &candidates {
            *groups.entry(normalized(&candidate.original)).or_default() += 1;
        }
        let order = |candidate: &Candidate| {
            (
                normalized(&candidate.original),
                candidate
                    .team
                    .as_ref()
                    .map(|team| team.1.clone())
                    .unwrap_or_default(),
                candidate.id.clone(),
            )
        };
        let (mut singles, mut collided): (Vec<_>, Vec<_>) = candidates
            .into_iter()
            .partition(|candidate| groups[&normalized(&candidate.original)] == 1);
        singles.sort_by_key(order);
        collided.sort_by_key(order);
        let mut taken: HashSet<String> = self
            .target
            .labels
            .iter()
            .flat_map(|label| std::iter::once(&label.name).chain(label.aliases.iter()))
            .map(|name| normalized(name))
            .collect();
        let mut named = Vec::new();
        for candidate in singles.into_iter().chain(collided) {
            let short = short_id(&candidate.id);
            let base = if candidate.original.is_empty() {
                format!("Linear label {short}")
            } else {
                candidate.original.clone()
            };
            let scope = candidate
                .team
                .as_ref()
                .map_or_else(|| "workspace".to_owned(), |team| team.1.clone());
            let collision = groups[&normalized(&candidate.original)] > 1
                || taken.contains(&normalized(&candidate.original));
            let mut attempts = Vec::new();
            if !collision {
                attempts.push(fit(&base, "", 80));
            }
            attempts.push(fit(&base, &format!(" ({scope})"), 80));
            attempts.push(fit(&base, &format!(" ({scope} {short})"), 80));
            attempts.push(fit(&base, &format!(" ({})", candidate.id.trim()), 80));
            let name = attempts
                .into_iter()
                .find(|name| !taken.contains(&normalized(name)))
                .context("Could not assign a unique Direct label name")?;
            taken.insert(normalized(&name));
            named.push((candidate, name, collision));
        }
        named.sort_by_key(|(candidate, _, _)| candidate.index);
        for (candidate, name, collision) in named {
            let label = &package.records(LABELS)[candidate.index];
            let mut reasons = Vec::new();
            let mut class = Class::Native;
            if name != candidate.original {
                class = Class::Transformed;
                let reason = if collision {
                    format!("Name {:?} collides case-insensitively with another Linear or existing Direct label; kept distinct as {name:?} (original name retained in linear_origins)", candidate.original)
                } else {
                    format!("Name adjusted to {name:?} to fit Direct's label name rules (original retained in linear_origins)")
                };
                reasons.push(reason);
                self.reports.label_disambiguations.push(json!({
                    "linear_id": candidate.id,
                    "original_name": candidate.original,
                    "team": candidate.team.as_ref().map(|team| team.1.clone()),
                    "direct_name": name,
                    "collision": collision,
                }));
            }
            let raw_description = optional_string(label, "description").unwrap_or_default();
            let description = truncate(&raw_description, 1_000);
            if description != raw_description {
                class = Class::Transformed;
                reasons.push("Description exceeds Direct's 1000-byte label limit; truncated (full text retained in the source bundle)".into());
            }
            let raw_color = optional_string(label, "color").unwrap_or_default();
            let color = if valid_color(&raw_color) {
                raw_color.clone()
            } else {
                class = Class::Transformed;
                reasons.push(format!(
                    "Color {raw_color:?} is not #rrggbb; stored without a color"
                ));
                String::new()
            };
            if has_value(label.get("archivedAt")) {
                class = Class::Transformed;
                reasons.push("Archived in Linear; imported as a label definition so historical assignments stay visible".into());
            }
            if label.get("isGroup").and_then(Value::as_bool) == Some(true) {
                reasons.push("Linear label group; group membership is not captured and Direct labels are flat".into());
            }
            let direct_id = uuid_or_derived(&candidate.id, "linear-label");
            let products = candidate
                .team
                .as_ref()
                .map(|team| {
                    vec![LabelProductRule {
                        product_id: self.product_ids[&team.0].clone(),
                        default_for_new_issues: false,
                    }]
                })
                .unwrap_or_default();
            self.labels.push(Label {
                id: direct_id.clone(),
                name: name.clone(),
                description,
                color,
                aliases: Vec::new(),
                products,
                linear_origins: vec![LinearLabelOrigin {
                    id: candidate.id.trim().to_owned(),
                    name: truncate(&candidate.original, 200).trim_end().to_owned(),
                }],
                version: 1,
                created_at: timestamp(label, "createdAt")?,
                updated_at: timestamp(label, "updatedAt")?,
            });
            self.label_map.insert(
                candidate.id.clone(),
                (
                    direct_id.clone(),
                    candidate
                        .team
                        .as_ref()
                        .map_or(LabelScope::Workspace, |team| {
                            LabelScope::Team(team.0.clone())
                        }),
                ),
            );
            self.record(
                LABELS,
                candidate.index,
                label,
                class,
                json!({"entity": "label", "id": direct_id, "name": name, "scope": candidate.team.as_ref().map_or("workspace".to_owned(), |team| team.1.clone())}),
                reasons,
            );
        }
        Ok(())
    }

    fn projects(&mut self) -> Result<()> {
        let package = self.package;
        let mut issue_teams: HashMap<String, BTreeMap<String, usize>> = HashMap::new();
        for issue in package.records(ISSUES) {
            if let (Some(project), Some(team)) = (
                nested_string(issue, &["project", "id"]),
                nested_string(issue, &["team", "id"]),
            ) {
                *issue_teams
                    .entry(project)
                    .or_default()
                    .entry(team)
                    .or_default() += 1;
            }
        }
        struct Pending {
            index: usize,
            product: String,
            reasons: Vec<String>,
            class: Class,
        }
        let mut pending = Vec::new();
        for (index, project) in package.records(PROJECTS).iter().enumerate() {
            let id = required_string(project, "id")?;
            let listed = node_ids(project, "teams");
            let mut reasons = Vec::new();
            let mut class = Class::Native;
            let unknown: Vec<_> = listed
                .iter()
                .filter(|team| !self.team_keys.contains_key(*team))
                .cloned()
                .collect();
            if !unknown.is_empty() {
                class = Class::Transformed;
                reasons.push(format!(
                    "References team(s) absent from teams.json: {}",
                    unknown.join(", ")
                ));
            }
            let mut teams: Vec<String> = listed
                .into_iter()
                .filter(|team| self.team_keys.contains_key(team))
                .collect();
            let counts = issue_teams.get(&id).cloned().unwrap_or_default();
            if teams.is_empty() {
                teams = counts
                    .keys()
                    .filter(|team| self.team_keys.contains_key(*team))
                    .cloned()
                    .collect();
                if !teams.is_empty() {
                    class = Class::Transformed;
                    reasons.push(
                        "Project lists no captured team; product derived from its issues".into(),
                    );
                }
            }
            let Some(primary) = teams.iter().max_by(|left, right| {
                let score = |team: &String| counts.get(team).copied().unwrap_or(0);
                score(left)
                    .cmp(&score(right))
                    .then_with(|| self.team_keys[*right].cmp(&self.team_keys[*left]))
            }) else {
                self.record(PROJECTS, index, project, Class::Unresolved, Value::Null, vec!["Direct projects belong to exactly one product, and this project has no captured team or issue to derive one from".into()]);
                continue;
            };
            if teams.len() > 1 {
                class = Class::Transformed;
                let keys: Vec<_> = teams
                    .iter()
                    .map(|team| self.team_keys[team].clone())
                    .collect();
                reasons.push(format!(
                    "Linear project spans teams {}; Direct projects belong to one product, so it is assigned to {} (most issues, then team key). Issues of other teams keep their own product and are reported as cross-product memberships",
                    keys.join(", "),
                    self.team_keys[primary]
                ));
                self.reports.cross_product.push(json!({
                    "kind": "multi_team_project",
                    "project_id": id,
                    "project": string(project, "name"),
                    "teams": keys,
                    "assigned_product": self.team_keys[primary],
                }));
            }
            pending.push(Pending {
                index,
                product: primary.clone(),
                reasons,
                class,
            });
        }
        let records = package.records(PROJECTS);
        let names = unique_names(
            pending.iter().map(|item| {
                let project = &records[item.index];
                (
                    item.product.clone(),
                    string(project, "name").to_owned(),
                    string(project, "id").to_owned(),
                )
            }),
            160,
            "Linear project",
            &self.reserved_names(&self.target.project_names),
        );
        let ranks = ranks(pending.iter().map(|item| {
            let project = &records[item.index];
            (
                item.product.clone(),
                number(project, "sortOrder"),
                normalized(string(project, "name")),
                string(project, "id").to_owned(),
            )
        }));
        for item in pending {
            let project = &records[item.index];
            let id = required_string(project, "id")?;
            let mut reasons = item.reasons;
            let mut class = item.class;
            let name = names[&id].clone();
            if name != string(project, "name") {
                class = Class::Transformed;
                reasons.push(format!("Name {:?} is empty, too long, or duplicates another project name in the product; stored as {name:?}", string(project, "name")));
                self.reports.name_disambiguations.push(json!({"entity": "project", "id": id, "original": string(project, "name"), "direct_name": name}));
            }
            let raw_status = project
                .pointer("/status/type")
                .and_then(Value::as_str)
                .or_else(|| project.get("state").and_then(Value::as_str))
                .unwrap_or("planned")
                .to_ascii_lowercase();
            let (status, status_reason) = map_project_status(&raw_status);
            if let Some(reason) = status_reason {
                class = if reason.starts_with("Unknown") {
                    self.reports
                        .unknown_states
                        .insert(format!("project:{raw_status}"));
                    Class::Unresolved
                } else {
                    class.max(Class::Transformed)
                };
                reasons.push(reason);
            }
            let (priority, priority_reason) = map_priority(project);
            if let Some(reason) = priority_reason {
                class = class.max(Class::Transformed);
                reasons.push(reason);
            }
            if has_value(project.get("archivedAt")) {
                class = class.max(Class::Transformed);
                reasons.push("Archived in Linear; Direct projects have no archived flag, so archivedAt is retained in the source bundle".into());
            }
            self.project_products
                .insert(id.clone(), item.product.clone());
            self.projects.push(Project {
                id: id.clone(),
                product_id: self.product_ids[&item.product].clone(),
                name: name.clone(),
                description: optional_string(project, "description").unwrap_or_default(),
                status,
                priority,
                sort_order: ranks[&id],
                external_source: Some(SOURCE.into()),
                external_id: Some(id.clone()),
                external_url: non_empty(project, "url"),
                labels: Vec::new(),
                template: None,
                version: 1,
                created_at: timestamp(project, "createdAt")?,
                updated_at: timestamp(project, "updatedAt")?,
            });
            self.record(
                PROJECTS,
                item.index,
                project,
                class,
                json!({"entity": "project", "id": id, "product": self.team_keys[&item.product]}),
                reasons,
            );
        }
        Ok(())
    }

    fn milestones(&mut self) -> Result<()> {
        let package = self.package;
        let records = package.records(MILESTONES);
        let mut pending = Vec::new();
        for (index, milestone) in records.iter().enumerate() {
            let project = nested_string(milestone, &["project", "id"]);
            match project.filter(|project| self.project_products.contains_key(project)) {
                Some(project) => pending.push((index, project)),
                None => self.record(
                    MILESTONES,
                    index,
                    milestone,
                    Class::Unresolved,
                    Value::Null,
                    vec![
                        "Milestone's project was not imported, so Direct cannot hold the milestone"
                            .into(),
                    ],
                ),
            }
        }
        let names = unique_names(
            pending.iter().map(|(index, project)| {
                (
                    project.clone(),
                    string(&records[*index], "name").to_owned(),
                    string(&records[*index], "id").to_owned(),
                )
            }),
            160,
            "Linear milestone",
            &HashSet::new(),
        );
        let ranks = ranks(pending.iter().map(|(index, project)| {
            let milestone = &records[*index];
            (
                project.clone(),
                number(milestone, "sortOrder"),
                normalized(string(milestone, "name")),
                string(milestone, "id").to_owned(),
            )
        }));
        for (index, project) in pending {
            let milestone = &records[index];
            let id = required_string(milestone, "id")?;
            let mut class = Class::Native;
            let mut reasons = Vec::new();
            let name = names[&id].clone();
            if name != string(milestone, "name") {
                class = Class::Transformed;
                reasons.push(format!("Name {:?} is empty, too long, or duplicates another milestone in the project; stored as {name:?}", string(milestone, "name")));
                self.reports.name_disambiguations.push(json!({"entity": "milestone", "id": id, "original": string(milestone, "name"), "direct_name": name}));
            }
            if has_value(milestone.get("archivedAt")) {
                class = Class::Transformed;
                reasons.push("Archived in Linear; Direct milestones have no archived flag, so archivedAt is retained in the source bundle".into());
            }
            self.milestone_projects.insert(id.clone(), project.clone());
            self.milestones.push(Milestone {
                id: id.clone(),
                project_id: project.clone(),
                name,
                description: optional_string(milestone, "description").unwrap_or_default(),
                sort_order: ranks[&id],
                external_source: Some(SOURCE.into()),
                external_id: Some(id.clone()),
                external_url: None,
                version: 1,
                created_at: timestamp(milestone, "createdAt")?,
                updated_at: timestamp(milestone, "updatedAt")?,
            });
            self.record(
                MILESTONES,
                index,
                milestone,
                class,
                json!({"entity": "milestone", "id": id, "project": project}),
                reasons,
            );
        }
        Ok(())
    }

    fn initiatives(&mut self) -> Result<()> {
        let package = self.package;
        let mut reverse: HashMap<String, BTreeSet<String>> = HashMap::new();
        for project in package.records(PROJECTS) {
            for initiative in node_ids(project, "initiatives") {
                reverse
                    .entry(initiative)
                    .or_default()
                    .insert(string(project, "id").to_owned());
            }
        }
        struct Part {
            index: usize,
            id: String,
            external_id: String,
            product: String,
            projects: Vec<String>,
            split: bool,
        }
        let mut parts = Vec::new();
        for (index, initiative) in package.records(INITIATIVES).iter().enumerate() {
            let id = required_string(initiative, "id")?;
            let mut linked: BTreeSet<String> =
                node_ids(initiative, "projects").into_iter().collect();
            linked.extend(reverse.get(&id).cloned().unwrap_or_default());
            let mut by_product: BTreeMap<String, Vec<String>> = BTreeMap::new();
            let mut unimported = Vec::new();
            for project in linked {
                match self.project_products.get(&project) {
                    Some(product) => by_product.entry(product.clone()).or_default().push(project),
                    None => unimported.push(project),
                }
            }
            for team in node_ids(initiative, "teams") {
                if self.team_keys.contains_key(&team) {
                    by_product.entry(team).or_default();
                }
            }
            if by_product.is_empty() {
                self.record(INITIATIVES, index, initiative, Class::Unresolved, Value::Null, vec!["Direct goals belong to one product, and this initiative has no imported project or captured team to derive one from".into()]);
                continue;
            }
            let split = by_product.len() > 1;
            if split {
                let keys: Vec<_> = by_product
                    .keys()
                    .map(|team| self.team_keys[team].clone())
                    .collect();
                self.reports.cross_product.push(json!({
                    "kind": "multi_product_initiative",
                    "initiative_id": id,
                    "initiative": string(initiative, "name"),
                    "products": keys,
                    "resolution": "one Direct goal per product, each linking only that product's projects",
                }));
            }
            if !unimported.is_empty() {
                self.reports.cross_product.push(json!({
                    "kind": "initiative_project_not_imported",
                    "initiative_id": id,
                    "projects": unimported,
                }));
            }
            for (product, projects) in by_product {
                let (goal_id, external_id) = if split {
                    (
                        deterministic_uuid("linear-initiative-part", &format!("{id}:{product}")),
                        format!("{id}:{}", self.team_keys[&product]),
                    )
                } else {
                    (id.clone(), id.clone())
                };
                parts.push(Part {
                    index,
                    id: goal_id,
                    external_id,
                    product,
                    projects,
                    split,
                });
            }
        }
        let records = package.records(INITIATIVES);
        let names = unique_names(
            parts.iter().map(|part| {
                (
                    part.product.clone(),
                    string(&records[part.index], "name").to_owned(),
                    part.id.clone(),
                )
            }),
            160,
            "Linear initiative",
            &self.reserved_names(&self.target.goal_names),
        );
        let mut by_index: BTreeMap<usize, Vec<Part>> = BTreeMap::new();
        for part in parts {
            by_index.entry(part.index).or_default().push(part);
        }
        for (index, parts) in by_index {
            let initiative = &records[index];
            let source_id = required_string(initiative, "id")?;
            let mut class = Class::Native;
            let mut reasons = Vec::new();
            let raw_status = initiative
                .get("status")
                .and_then(|state| {
                    state
                        .as_str()
                        .or_else(|| state.get("type").and_then(Value::as_str))
                })
                .unwrap_or("planned")
                .to_ascii_lowercase();
            let (status, status_reason) = map_goal_status(&raw_status);
            if let Some(reason) = status_reason {
                class = if reason.starts_with("Unknown") {
                    self.reports
                        .unknown_states
                        .insert(format!("goal:{raw_status}"));
                    Class::Unresolved
                } else {
                    Class::Transformed
                };
                reasons.push(reason);
            }
            let (priority, priority_reason) = map_priority(initiative);
            if let Some(reason) = priority_reason {
                class = class.max(Class::Transformed);
                reasons.push(reason);
            }
            if has_value(initiative.get("archivedAt")) {
                class = class.max(Class::Transformed);
                reasons.push("Archived in Linear; Direct goals have no archived flag, so archivedAt is retained in the source bundle".into());
            }
            if parts[0].split {
                class = class.max(Class::Transformed);
                reasons.push(format!("Initiative spans {} products; split into one Direct goal per product with external IDs <initiative id>:<team key>", parts.len()));
            }
            let mut goal_refs = Vec::new();
            for part in &parts {
                let name = names[&part.id].clone();
                if name != string(initiative, "name") {
                    class = class.max(Class::Transformed);
                    reasons.push(format!("Name {:?} is empty, too long, or duplicates another goal in the product; stored as {name:?}", string(initiative, "name")));
                    self.reports.name_disambiguations.push(json!({"entity": "goal", "id": part.id, "original": string(initiative, "name"), "direct_name": name}));
                }
                self.goals.push(Goal {
                    id: part.id.clone(),
                    product_id: self.product_ids[&part.product].clone(),
                    name,
                    description: optional_string(initiative, "description").unwrap_or_default(),
                    status: status.clone(),
                    priority: priority.clone(),
                    project_ids: part.projects.clone(),
                    external_source: Some(SOURCE.into()),
                    external_id: Some(part.external_id.clone()),
                    external_url: non_empty(initiative, "url"),
                    version: 1,
                    created_at: timestamp(initiative, "createdAt")?,
                    updated_at: timestamp(initiative, "updatedAt")?,
                });
                self.initiative_goals
                    .entry(source_id.clone())
                    .or_default()
                    .push(part.id.clone());
                goal_refs.push(json!({"id": part.id, "product": self.team_keys[&part.product], "projects": part.projects}));
            }
            self.record(
                INITIATIVES,
                index,
                initiative,
                class,
                json!({"entity": "goal", "goals": goal_refs}),
                reasons,
            );
        }
        Ok(())
    }

    fn issues(&mut self) -> Result<()> {
        let package = self.package;
        let mut seen_ids = HashSet::new();
        let mut seen_identifiers = HashSet::new();
        let mut by_team: BTreeMap<String, Vec<(u64, String, String)>> = BTreeMap::new();
        for issue in package.records(ISSUES) {
            let id = required_string(issue, "id")?;
            let identifier = required_string(issue, "identifier")?;
            if !seen_ids.insert(id.clone()) || !seen_identifiers.insert(identifier.clone()) {
                bail!("Linear issues repeat id or identifier {identifier}");
            }
            let team = nested_string(issue, &["team", "id"])
                .with_context(|| format!("Linear issue {identifier} has no team"))?;
            if !self.team_keys.contains_key(&team) {
                bail!("Linear issue {identifier} references team {team}, which is absent from teams.json; the capture is incomplete");
            }
            let number = identifier
                .rsplit('-')
                .next()
                .and_then(|number| number.parse::<u64>().ok())
                .filter(|number| *number > 0)
                .with_context(|| {
                    format!("Linear issue identifier {identifier} has no issue number")
                })?;
            by_team
                .entry(team)
                .or_default()
                .push((number, identifier, id));
        }
        for (team, mut issues) in by_team {
            let product_key = self.team_keys[&team].clone();
            let mapped = self.target.team_map.contains_key(&team);
            let keep = issues.iter().all(|(_, identifier, _)| {
                identifier
                    .strip_prefix(&format!("{product_key}-"))
                    .is_some_and(|number| number.parse::<u64>().is_ok_and(|value| value > 0))
                    && !self.target.keys.contains(identifier)
            });
            if keep {
                for (_, identifier, id) in issues {
                    self.issue_keys.insert(id, identifier.clone());
                    self.issue_products.insert(identifier, team.clone());
                }
                continue;
            }
            if !mapped {
                bail!("Linear issue identifiers of team {product_key} do not match its key or collide with existing or reserved Direct keys; map the team explicitly with --map-team to renumber them");
            }
            // Explicitly mapped into an existing product whose key space is taken:
            // allocate new keys after every used or reserved number, in Linear order.
            issues.sort();
            let mut next = self.target.highest_number(&product_key);
            for (_, identifier, id) in issues {
                next += 1;
                let key = format!("{product_key}-{next}");
                self.reports.previous_identifiers.push(json!({
                    "previous": identifier,
                    "direct_key": key,
                    "reason": "renumbered into an explicitly mapped existing product",
                }));
                self.renumbered.insert(id.clone(), identifier);
                self.issue_keys.insert(id, key.clone());
                self.issue_products.insert(key, team.clone());
            }
        }
        for (index, issue) in package.records(ISSUES).iter().enumerate() {
            let id = required_string(issue, "id")?;
            let key = self.issue_keys[&id].clone();
            let product = self.issue_products[&key].clone();
            let pointer = format!("/records/{index}");
            let mut class = Class::Native;
            let mut reasons = Vec::new();
            let state = issue
                .get("state")
                .with_context(|| format!("Linear issue {key} has no state"))?;
            let state_kind = required_string(state, "type")?;
            let completed_at = optional_string(issue, "completedAt");
            let mapped = map_state(&state_kind, completed_at.is_some());
            if let Some(reason) = mapped.reason {
                reasons.push(reason.into());
            }
            if mapped.unresolved {
                self.reports.unknown_states.insert(state_kind.clone());
                class = Class::Unresolved;
            } else if mapped.transformed {
                class = class.max(Class::Transformed);
            }

            let source_project = nested_string(issue, &["project", "id"]);
            let project_id = match &source_project {
                Some(project) => match self.project_products.get(project) {
                    Some(project_product) if *project_product == product => Some(project.clone()),
                    Some(project_product) => {
                        class = class.max(Class::Unresolved);
                        reasons.push(format!("Linear project {project} was assigned to product {}; Direct cannot attach this {} issue across products, so project membership is retained only in the source bundle", self.team_keys[project_product], self.team_keys[&product]));
                        self.reports.cross_product.push(json!({"kind": "issue_project_membership", "issue": key, "project_id": project, "issue_product": self.team_keys[&product], "project_product": self.team_keys[project_product]}));
                        None
                    }
                    None => {
                        class = class.max(Class::Unresolved);
                        reasons.push(format!("Linear project {project} was not imported; membership retained only in the source bundle"));
                        None
                    }
                },
                None => None,
            };
            let milestone_id = match nested_string(issue, &["projectMilestone", "id"]) {
                Some(milestone)
                    if project_id.is_some()
                        && self.milestone_projects.get(&milestone) == project_id.as_ref() =>
                {
                    Some(milestone)
                }
                Some(milestone) => {
                    class = class.max(Class::Unresolved);
                    reasons.push(format!("Milestone {milestone} is not an imported milestone of this issue's Direct project; retained only in the source bundle"));
                    None
                }
                None => None,
            };
            let (priority, priority_reason) = map_priority(issue);
            if let Some(reason) = priority_reason {
                class = class.max(Class::Transformed);
                reasons.push(reason);
            }
            if let Some(identifier) = self.renumbered.get(&id) {
                class = class.max(Class::Transformed);
                reasons.push(format!("Linear identifier {identifier} is already used or reserved in the mapped product, so this issue is {key}; the original identifier stays searchable in its retained source record"));
            }
            if has_value(issue.get("archivedAt")) {
                reasons
                    .push("Archived in Linear; archivedAt is kept in external provenance".into());
            }
            if issue.get("trashed").and_then(Value::as_bool) == Some(true) {
                class = class.max(Class::Transformed);
                reasons.push("Linear marks this issue as trashed; imported with its archived provenance for review before cutover".into());
            }

            let assignments: Vec<(String, String)> =
                match issue.pointer("/labels/nodes").and_then(Value::as_array) {
                    Some(nodes) => nodes
                        .iter()
                        .enumerate()
                        .map(|(position, node)| {
                            (
                                format!("{pointer}/labels/nodes/{position}"),
                                string(node, "id").to_owned(),
                            )
                        })
                        .collect(),
                    None => string_list(issue, "labelIds")
                        .into_iter()
                        .enumerate()
                        .map(|(position, label)| (format!("{pointer}/labelIds/{position}"), label))
                        .collect(),
                };
            let mut labels = BTreeSet::new();
            for (assignment_pointer, label) in assignments {
                let (assignment_class, direct, reason) = match self.label_map.get(&label) {
                    Some((direct_id, scope))
                        if match scope {
                            LabelScope::Workspace => true,
                            LabelScope::Team(team) => *team == product,
                            LabelScope::Products(products) => {
                                products.is_empty()
                                    || products.contains(&self.product_ids[&product])
                            }
                        } =>
                    {
                        if labels.insert(direct_id.clone()) {
                            (
                                Class::Native,
                                json!({"issue": key, "label": direct_id}),
                                None,
                            )
                        } else {
                            (
                                Class::Transformed,
                                json!({"issue": key, "label": direct_id}),
                                Some(
                                    "Repeated assignment of the same label; attached once"
                                        .to_owned(),
                                ),
                            )
                        }
                    }
                    Some((_, scope)) => {
                        let scope = match scope {
                            LabelScope::Team(team) => self
                                .team_keys
                                .get(team)
                                .cloned()
                                .unwrap_or_else(|| team.clone()),
                            _ => "other products".to_owned(),
                        };
                        self.reports.cross_product.push(json!({"kind": "label_assignment", "issue": key, "label_id": label, "label_team": scope}));
                        (Class::Unresolved, Value::Null, Some(format!("Label belongs to {scope}; Direct label rules do not apply it to this issue's product")))
                    }
                    None => (
                        Class::Unresolved,
                        Value::Null,
                        Some(
                            "Label definition is absent from issue-labels.json or was not imported"
                                .to_owned(),
                        ),
                    ),
                };
                if assignment_class == Class::Unresolved {
                    class = class.max(Class::Unresolved);
                }
                self.component(
                    "label_assignment",
                    ISSUES,
                    assignment_pointer,
                    Some(label),
                    Some(key.clone()),
                    assignment_class,
                    direct,
                    reason.into_iter().collect(),
                );
            }

            for (position, previous) in string_list(issue, "previousIdentifiers")
                .into_iter()
                .enumerate()
            {
                let mut reasons = vec!["Previous Linear identifier; Direct keeps the current key, and this identifier is mapped in the index and reconciliation report".to_owned()];
                if self.issue_products.contains_key(&previous) {
                    reasons.push(format!(
                        "{previous} is now the key of a different current issue"
                    ));
                }
                self.reports
                    .previous_identifiers
                    .push(json!({"previous": previous, "direct_key": key}));
                self.component(
                    "previous_identifier",
                    ISSUES,
                    format!("{pointer}/previousIdentifiers/{position}"),
                    Some(id.clone()),
                    Some(previous),
                    Class::Preserved,
                    json!({"issue": key}),
                    reasons,
                );
            }

            let history: Vec<Value> = issue
                .pointer("/history/nodes")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if !history.is_empty() {
                self.component(
                    "issue_history",
                    ISSUES,
                    format!("{pointer}/history/nodes"),
                    Some(id.clone()),
                    Some(format!("{} entries", history.len())),
                    Class::Native,
                    json!({"issue": key, "field": "external.history", "entries": history.len()}),
                    Vec::new(),
                );
            }
            let external = ExternalIssueRecord {
                source: SOURCE.into(),
                id: id.clone(),
                url: required_string(issue, "url")?,
                state: ExternalIssueState {
                    id: required_string(state, "id")?,
                    name: required_string(state, "name")?,
                    kind: state_kind,
                },
                created_at: required_string(issue, "createdAt")?,
                updated_at: required_string(issue, "updatedAt")?,
                started_at: optional_string(issue, "startedAt"),
                completed_at,
                canceled_at: optional_string(issue, "canceledAt"),
                archived_at: optional_string(issue, "archivedAt"),
                history,
            };
            let status = mapped.status;
            self.issues.push(Issue {
                intake: None,
                labels: labels.into_iter().collect(),
                id: id.clone(),
                key: key.clone(),
                product_id: self.product_ids[&product].clone(),
                project_id: project_id.clone(),
                milestone_id,
                planning_scope: if source_project.is_some() {
                    PlanningScope::Project
                } else {
                    PlanningScope::Inbox
                },
                theoria_refs: Vec::new(),
                title: required_string(issue, "title")?,
                body: optional_string(issue, "description").unwrap_or_default(),
                acceptance: String::new(),
                owner: nested_string(issue, &["assignee", "name"]).unwrap_or_default(),
                priority,
                status: status.clone(),
                version: 1,
                created_at: timestamp(issue, "createdAt")?,
                updated_at: timestamp(issue, "updatedAt")?,
                claim: None,
                needs_fix: false,
                parent: None,
                verification_key: None,
                current_run: None,
                external: Some(external),
                template: None,
            });
            self.record(
                ISSUES,
                index,
                issue,
                class,
                json!({"entity": "issue", "key": key, "status": status_name(&status), "project": project_id}),
                reasons,
            );
        }
        Ok(())
    }

    fn comments(&mut self) -> Result<()> {
        let package = self.package;
        for (index, comment) in package.records(COMMENTS).iter().enumerate() {
            let id = required_string(comment, "id")?;
            let issue = nested_string(comment, &["issue", "id"])
                .or_else(|| optional_string(comment, "issueId"));
            let Some(issue) = issue else {
                let owner = [
                    "documentContent",
                    "projectUpdate",
                    "initiativeUpdate",
                    "post",
                    "document",
                    "project",
                    "initiative",
                ]
                .into_iter()
                .find(|field| has_value(comment.get(*field)))
                .unwrap_or("an entity that was not captured");
                self.record(COMMENTS, index, comment, Class::Preserved, Value::Null, vec![format!("Comment belongs to {owner}, not an issue; Direct comments attach only to issues. {NATIVE_ACCESS}")]);
                continue;
            };
            let Some(key) = self.issue_keys.get(&issue).cloned() else {
                self.record(
                    COMMENTS,
                    index,
                    comment,
                    Class::Unresolved,
                    Value::Null,
                    vec![format!(
                        "Comment references issue {issue}, which is absent from issues.json"
                    )],
                );
                continue;
            };
            if has_value(comment.get("archivedAt")) {
                self.record(COMMENTS, index, comment, Class::Preserved, json!({"issue": key}), vec![format!("Archived in Linear; retained in the source bundle rather than shown as a live Direct comment. {NATIVE_ACCESS}")]);
                continue;
            }
            let mut class = Class::Native;
            let mut reasons = Vec::new();
            if let Some(parent) = nested_string(comment, &["parent", "id"]) {
                class = Class::Transformed;
                reasons.push(format!("Reply to comment {parent}; Direct comments are flat, so the thread parent is retained in the source bundle"));
            }
            let actor = nested_string(comment, &["user", "name"])
                .or_else(|| nested_string(comment, &["user", "displayName"]))
                .or_else(|| nested_string(comment, &["botActor", "name"]))
                .or_else(|| nested_string(comment, &["externalUser", "name"]));
            let actor = match actor.filter(|actor| !actor.trim().is_empty()) {
                Some(actor) => actor,
                None => {
                    class = class.max(Class::Transformed);
                    reasons.push(
                        "No author in the captured comment; recorded with actor \"Linear\"".into(),
                    );
                    "Linear".into()
                }
            };
            self.comments.push(Comment {
                id: id.clone(),
                issue_key: key.clone(),
                actor,
                body: optional_string(comment, "body").unwrap_or_default(),
                at: timestamp(comment, "createdAt")?,
                external_source: Some(SOURCE.into()),
                external_id: Some(id.clone()),
                external_url: non_empty(comment, "url"),
            });
            self.record(
                COMMENTS,
                index,
                comment,
                class,
                json!({"entity": "comment", "id": id, "issue": key}),
                reasons,
            );
        }
        Ok(())
    }

    /// Try to add a link, returning why it could not be represented.
    fn add_link(
        &mut self,
        link: IssueLink,
        origin: &str,
    ) -> std::result::Result<String, LinkRefusal> {
        if link.source_key == link.target_key {
            return Err(LinkRefusal::Unresolved(
                "A Direct link cannot point an issue at itself".into(),
            ));
        }
        let source_product = &self.issue_products[&link.source_key];
        let target_product = &self.issue_products[&link.target_key];
        if source_product != target_product {
            return Err(LinkRefusal::CrossProduct(format!(
                "Links join {} and {} issues; Direct links stay within one product",
                self.team_keys[source_product], self.team_keys[target_product]
            )));
        }
        if let Some(existing) = self
            .links
            .iter()
            .find(|prior| same_logical_link(prior, &link.source_key, &link.target_key, &link.kind))
        {
            return Err(LinkRefusal::Duplicate(
                existing.id.clone(),
                self.link_sources[&existing.id].clone(),
            ));
        }
        if link.kind == IssueLinkKind::Parent
            && self.links.iter().any(|prior| {
                prior.kind == IssueLinkKind::Parent && prior.source_key == link.source_key
            })
        {
            return Err(LinkRefusal::Unresolved(
                "Issue already has a parent link".into(),
            ));
        }
        if creates_cycle(&self.links, &link.source_key, &link.target_key, &link.kind) {
            return Err(LinkRefusal::Unresolved(format!(
                "Would create a {} cycle, which Direct rejects",
                link_kind_name(&link.kind)
            )));
        }
        let id = link.id.clone();
        self.link_sources.insert(id.clone(), origin.to_owned());
        self.links.push(link);
        Ok(id)
    }

    fn parents(&mut self) -> Result<()> {
        let package = self.package;
        let mut pairs = Vec::new();
        for (index, issue) in package.records(ISSUES).iter().enumerate() {
            if let Some(parent) = nested_string(issue, &["parent", "id"]) {
                pairs.push((string(issue, "id").to_owned(), parent, index));
            }
        }
        pairs.sort();
        for (child, parent, index) in pairs {
            let issue = &package.records(ISSUES)[index];
            let child_key = self.issue_keys[&child].clone();
            let pointer = format!("/records/{index}/parent");
            let Some(parent_key) = self.issue_keys.get(&parent).cloned() else {
                self.reports.relations_unresolved.push(json!({"kind": "parent", "child": child_key, "parent_id": parent, "reason": "parent issue absent from issues.json"}));
                self.component(
                    "parent_link",
                    ISSUES,
                    pointer,
                    Some(parent),
                    Some(child_key),
                    Class::Unresolved,
                    Value::Null,
                    vec!["Parent issue is absent from issues.json".into()],
                );
                continue;
            };
            let link = IssueLink {
                id: deterministic_uuid("linear-parent", &format!("{child}:{parent}")),
                source_key: child_key.clone(),
                target_key: parent_key.clone(),
                kind: IssueLinkKind::Parent,
                external_source: Some(SOURCE.into()),
                external_id: Some(format!("parent:{child}:{parent}")),
                created_by: "linear-import".into(),
                created_at: timestamp(issue, "createdAt")?,
            };
            let label = format!("{child_key} → {parent_key}");
            match self.add_link(link, &format!("parent:{child}:{parent}")) {
                Ok(id) => self.component("parent_link", ISSUES, pointer, Some(parent), Some(label), Class::Native, json!({"link": id, "kind": "parent", "source": child_key, "target": parent_key}), vec!["Child is the link source and the parent is the target".into()]),
                Err(refusal) => {
                    let (class, reason) = refusal.describe();
                    self.note_relation_refusal(&refusal, json!({"kind": "parent", "child": child_key, "parent": parent_key, "reason": reason}));
                    self.component("parent_link", ISSUES, pointer, Some(parent), Some(label), class, Value::Null, vec![reason]);
                }
            }
        }
        Ok(())
    }

    fn note_relation_refusal(&mut self, refusal: &LinkRefusal, item: Value) {
        match refusal {
            LinkRefusal::CrossProduct(_) => self.reports.cross_product.push(item),
            LinkRefusal::Duplicate(_, _) => self.reports.relations_deduplicated.push(item),
            LinkRefusal::Unresolved(_) => self.reports.relations_unresolved.push(item),
        }
    }

    fn relations(&mut self) -> Result<()> {
        let package = self.package;
        let records = package.records(RELATIONS);
        let mut order: Vec<usize> = (0..records.len()).collect();
        let rank = |relation: &Value| match string(relation, "type") {
            "blocks" => 0,
            "related" => 1,
            _ => 2,
        };
        order.sort_by(|left, right| {
            let (a, b) = (&records[*left], &records[*right]);
            rank(a)
                .cmp(&rank(b))
                .then_with(|| string(a, "createdAt").cmp(string(b, "createdAt")))
                .then_with(|| string(a, "id").cmp(string(b, "id")))
        });
        for index in order {
            let relation = &records[index];
            let id = required_string(relation, "id")?;
            let kind = string(relation, "type").to_owned();
            let from = nested_string(relation, &["issue", "id"]).unwrap_or_default();
            let to = nested_string(relation, &["relatedIssue", "id"]).unwrap_or_default();
            let from_key = self.issue_keys.get(&from).cloned();
            let to_key = self.issue_keys.get(&to).cloned();
            let summary = json!({"kind": "relation", "relation_id": id, "type": kind, "issue": from_key.clone().unwrap_or(from.clone()), "related_issue": to_key.clone().unwrap_or(to.clone())});
            let (Some(from_key), Some(to_key)) = (from_key, to_key) else {
                let mut item = summary.clone();
                item["reason"] = json!("an endpoint issue is absent from issues.json");
                self.reports.relations_unresolved.push(item);
                self.record(
                    RELATIONS,
                    index,
                    relation,
                    Class::Unresolved,
                    Value::Null,
                    vec!["An endpoint issue is absent from issues.json".into()],
                );
                continue;
            };
            if has_value(relation.get("archivedAt")) {
                self.reports.relations_archived.push(summary);
                self.record(RELATIONS, index, relation, Class::Preserved, Value::Null, vec![format!("Archived in Linear; retained in the source bundle rather than asserted as a live Direct link. {NATIVE_ACCESS}")]);
                continue;
            }
            let (source_key, target_key, link_kind) = match kind.as_str() {
                "blocks" => (to_key.clone(), from_key.clone(), IssueLinkKind::BlockedBy),
                "related" => (from_key.clone(), to_key.clone(), IssueLinkKind::Related),
                other => {
                    let mut item = summary;
                    item["reason"] = json!(format!(
                        "Direct has no {other:?} link kind; not converted to related or blocked_by"
                    ));
                    self.reports.relations_unsupported.push(item);
                    self.record(RELATIONS, index, relation, Class::Preserved, Value::Null, vec![format!("Linear {other:?} relation has no Direct link kind and is not converted to another kind. {NATIVE_ACCESS}")]);
                    continue;
                }
            };
            let link = IssueLink {
                id: uuid_or_derived(&id, "linear-relation"),
                source_key: source_key.clone(),
                target_key: target_key.clone(),
                kind: link_kind.clone(),
                external_source: Some(SOURCE.into()),
                external_id: Some(id.clone()),
                created_by: "linear-import".into(),
                created_at: timestamp(relation, "createdAt")?,
            };
            match self.add_link(link, &id) {
                Ok(link_id) => {
                    let reasons = if link_kind == IssueLinkKind::BlockedBy {
                        vec![format!("{from_key} blocks {to_key}: stored on {to_key} as blocked_by {from_key}")]
                    } else {
                        Vec::new()
                    };
                    self.record(RELATIONS, index, relation, Class::Native, json!({"link": link_id, "kind": link_kind_name(&link_kind), "source": source_key, "target": target_key}), reasons);
                }
                Err(refusal) => {
                    let (class, reason) = refusal.describe();
                    let mut item = summary;
                    item["reason"] = json!(reason);
                    let direct = match &refusal {
                        LinkRefusal::Duplicate(link, _) => {
                            item["represented_by_link"] = json!(link);
                            json!({"link": link, "kind": link_kind_name(&link_kind)})
                        }
                        _ => Value::Null,
                    };
                    self.note_relation_refusal(&refusal, item);
                    self.record(RELATIONS, index, relation, class, direct, vec![reason]);
                }
            }
        }
        Ok(())
    }

    fn preserved_only(&mut self) {
        let package = self.package;
        let mut references: HashMap<String, (usize, usize)> = HashMap::new();
        for issue in package.records(ISSUES) {
            if let Some(user) = nested_string(issue, &["assignee", "id"]) {
                references.entry(user).or_default().0 += 1;
            }
        }
        for comment in package.records(COMMENTS) {
            if let Some(user) = nested_string(comment, &["user", "id"]) {
                references.entry(user).or_default().1 += 1;
            }
        }
        for (index, user) in package.records(USERS).iter().enumerate() {
            let (owned, authored) = references
                .get(string(user, "id"))
                .copied()
                .unwrap_or_default();
            self.record(USERS, index, user, Class::Preserved, Value::Null, vec![format!("Direct has no user directory. This user's display name is attribution text on {owned} issue owner field(s) and {authored} comment(s); identity details stay in the source bundle")]);
        }
        for (index, document) in package.records(DOCUMENTS).iter().enumerate() {
            let direct = self.planning_refs(document);
            self.record(DOCUMENTS, index, document, Class::Preserved, direct, vec![format!("Direct has no native document record; content is retained byte for byte and is not imported as issue text or Theoria guidance. {NATIVE_ACCESS}")]);
        }
        for (index, attachment) in package.records(LINK_ATTACHMENTS).iter().enumerate() {
            let issue = nested_string(attachment, &["issue", "id"])
                .and_then(|issue| self.issue_keys.get(&issue).cloned());
            let direct = issue
                .as_ref()
                .map_or(Value::Null, |key| json!({"issue": key}));
            self.record(LINK_ATTACHMENTS, index, attachment, Class::Preserved, direct, vec![format!("Linked attachment metadata; Direct has no attachment record and third-party contents were not captured. {NATIVE_ACCESS}")]);
        }
        for file in [PROJECT_UPDATES, INITIATIVE_UPDATES] {
            for (index, update) in package.records(file).iter().enumerate() {
                let direct = self.planning_refs(update);
                self.record(
                    file,
                    index,
                    update,
                    Class::Preserved,
                    direct,
                    vec![format!(
                        "Direct has no planning update record. {NATIVE_ACCESS}"
                    )],
                );
            }
        }
        let known: BTreeSet<&str> = WHOLE_REQUIRED
            .iter()
            .chain(EXPECTED_OPTIONAL)
            .copied()
            .collect();
        for (file, value) in &package.data {
            if known.contains(file.as_str()) {
                continue;
            }
            if let Some(records) = value.get("records").and_then(Value::as_array) {
                for (index, record) in records.iter().enumerate() {
                    self.record(file, index, record, Class::Preserved, Value::Null, vec![format!("Record type from data/{file} is not recognized by this importer. {NATIVE_ACCESS}")]);
                }
            }
        }
    }

    fn planning_refs(&self, value: &Value) -> Value {
        let mut refs = serde_json::Map::new();
        if let Some(project) = nested_string(value, &["project", "id"]) {
            if self.project_products.contains_key(&project) {
                refs.insert("project".into(), json!(project));
            }
        }
        if let Some(initiative) = nested_string(value, &["initiative", "id"]) {
            if let Some(goals) = self.initiative_goals.get(&initiative) {
                refs.insert("goals".into(), json!(goals));
            }
        }
        if let Some(issue) =
            nested_string(value, &["issue", "id"]).and_then(|issue| self.issue_keys.get(&issue))
        {
            refs.insert("issue".into(), json!(issue));
        }
        if refs.is_empty() {
            Value::Null
        } else {
            Value::Object(refs)
        }
    }

    fn uploads(&mut self) {
        let package = self.package;
        for (index, upload) in package.uploads.iter().enumerate() {
            let url = optional_string(upload, "canonical_url");
            let name = optional_string(upload, "original_name").or_else(|| url.clone());
            let (class, direct, reasons) = if string(upload, "status") == "downloaded" {
                (
                    Class::Preserved,
                    json!({"bundle_path": format!("{BUNDLE_DIR}/{}", string(upload, "path"))}),
                    vec![format!("Uploaded file retained with its checksum; Direct issue text still references the original URL. {NATIVE_ACCESS}")],
                )
            } else {
                (
                    Class::Unresolved,
                    Value::Null,
                    vec![format!(
                        "Upload was not downloaded during capture ({}); its bytes are missing",
                        optional_string(upload, "error")
                            .unwrap_or_else(|| "no error recorded".into())
                    )],
                )
            };
            self.entries.push(Entry {
                level: "component",
                kind: "uploaded_file".into(),
                file: super::package::UPLOAD_MANIFEST.into(),
                pointer: format!("/{index}"),
                source_id: url,
                source_label: name,
                source_title: None,
                classification: class,
                direct,
                reasons,
                preserved_fields: Vec::new(),
            });
        }
    }

    fn data_files(&mut self) {
        let package = self.package;
        for (path, checksum) in &package.files {
            let Some(name) = path.strip_prefix("data/") else {
                continue;
            };
            let records = package.data[name]
                .get("records")
                .and_then(Value::as_array)
                .map(Vec::len);
            let reason = match records {
                Some(count) => format!(
                    "{count} record(s); retained byte for byte (sha256 {})",
                    checksum.sha256
                ),
                None => format!(
                    "Capture metadata without records; retained byte for byte (sha256 {})",
                    checksum.sha256
                ),
            };
            self.entries.push(Entry {
                level: "file",
                kind: if name == SCHEMA {
                    "capture_schema".into()
                } else {
                    "data_file".into()
                },
                file: path.clone(),
                pointer: String::new(),
                source_id: None,
                source_label: Some(name.to_owned()),
                source_title: None,
                classification: Class::Preserved,
                direct: json!({"bundle_path": format!("{BUNDLE_DIR}/{path}")}),
                reasons: vec![reason],
                preserved_fields: Vec::new(),
            });
        }
    }

    fn finish(self) -> Result<Plan> {
        let package = self.package;
        // Every record of every data file must have exactly one record-level entry.
        let mut accounted: HashMap<(String, String), usize> = HashMap::new();
        for entry in self.entries.iter().filter(|entry| entry.level == "record") {
            *accounted
                .entry((entry.file.clone(), entry.pointer.clone()))
                .or_default() += 1;
        }
        let mut records_in_source = 0;
        for (file, value) in &package.data {
            let Some(records) = value.get("records").and_then(Value::as_array) else {
                continue;
            };
            for index in 0..records.len() {
                records_in_source += 1;
                match accounted.get(&(format!("data/{file}"), format!("/records/{index}"))) {
                    Some(1) => {}
                    Some(count) => {
                        bail!("data/{file} record {index} has {count} accounting entries")
                    }
                    None => bail!("data/{file} record {index} has no accounting entry"),
                }
            }
        }
        let records_accounted = accounted.values().sum::<usize>();
        if records_accounted != records_in_source {
            bail!("Accounting covers {records_accounted} records but the source holds {records_in_source}");
        }
        let uploads_accounted = self
            .entries
            .iter()
            .filter(|entry| entry.kind == "uploaded_file")
            .count();
        if uploads_accounted != package.uploads.len() {
            bail!(
                "Accounting covers {uploads_accounted} of {} uploaded files",
                package.uploads.len()
            );
        }

        let mut archive = Archive {
            format: direct_core::ARCHIVE_FORMAT,
            workspace_id: deterministic_uuid("linear-workspace", &{
                let mut ids: Vec<_> = self
                    .products
                    .iter()
                    .map(|product| product.id.clone())
                    .collect();
                ids.sort();
                ids.join(",")
            }),
            products: self.products,
            product_sections: Vec::new(),
            projects: self.projects,
            goals: self.goals,
            milestones: self.milestones,
            labels: self.labels,
            theoria_documents: vec![],
            method_findings: vec![],
            git_traces: vec![],
            releases: vec![],
            release_evidence: vec![],
            release_workflows: vec![],
            issue_links: self.links,
            templates: vec![],
            template_revisions: vec![],
            issues: self.issues,
            comments: self.comments,
            verifications: vec![],
            events: vec![],
            requests: vec![],
            source_bundles: vec![],
            source_files: vec![],
            source_records: vec![],
        };
        archive.products.sort_by(|a, b| a.id.cmp(&b.id));
        archive.projects.sort_by(|a, b| a.id.cmp(&b.id));
        archive.goals.sort_by(|a, b| a.id.cmp(&b.id));
        archive.milestones.sort_by(|a, b| a.id.cmp(&b.id));
        archive.labels.sort_by(|a, b| a.id.cmp(&b.id));
        archive.issue_links.sort_by(|a, b| a.id.cmp(&b.id));
        archive.issues.sort_by(|a, b| a.id.cmp(&b.id));
        archive.comments.sort_by(|a, b| a.id.cmp(&b.id));

        let reconciliation = reconciliation(
            package,
            &archive,
            &self.entries,
            &self.reports,
            records_in_source,
            records_accounted,
        );
        Ok(Plan {
            archive,
            entries: self.entries,
            reconciliation,
            issue_keys: self.issue_keys,
            reused_products: self.reused_products.into_iter().collect(),
            reused_labels: self.reused_labels.into_iter().collect(),
            reused_label_origins: self
                .label_map
                .iter()
                .filter(|(_, (_, scope))| matches!(scope, LabelScope::Products(_)))
                .map(|(origin, (id, _))| (origin.clone(), id.clone()))
                .collect(),
        })
    }
}

enum LinkRefusal {
    CrossProduct(String),
    /// Existing Direct link ID and the source relation it came from.
    Duplicate(String, String),
    Unresolved(String),
}

impl LinkRefusal {
    fn describe(&self) -> (Class, String) {
        match self {
            LinkRefusal::CrossProduct(reason) => (
                Class::Unresolved,
                format!("{reason}; retained only in the source bundle"),
            ),
            LinkRefusal::Duplicate(link, origin) => (
                Class::Transformed,
                format!("Semantic duplicate of {origin}; represented by Direct link {link}"),
            ),
            LinkRefusal::Unresolved(reason) => (
                Class::Unresolved,
                format!("{reason}; retained only in the source bundle"),
            ),
        }
    }
}

fn reconciliation(
    package: &VerifiedPackage,
    archive: &Archive,
    entries: &[Entry],
    reports: &Reports,
    records_in_source: usize,
    records_accounted: usize,
) -> Value {
    let records = |file: &str| package.records(file);
    let count_where = |file: &str, predicate: &dyn Fn(&Value) -> bool| {
        records(file)
            .iter()
            .filter(|value| predicate(value))
            .count()
    };
    let mut relation_types: BTreeMap<String, usize> = BTreeMap::new();
    for relation in records(RELATIONS) {
        *relation_types
            .entry(string(relation, "type").to_owned())
            .or_default() += 1;
    }
    let label_assignments: usize = records(ISSUES)
        .iter()
        .map(|issue| {
            issue
                .pointer("/labels/nodes")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or_else(|| string_list(issue, "labelIds").len())
        })
        .sum();
    let history_source: usize = records(ISSUES)
        .iter()
        .map(|issue| {
            issue
                .pointer("/history/nodes")
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        })
        .sum();
    let history_imported: usize = archive
        .issues
        .iter()
        .map(|issue| {
            issue
                .external
                .as_ref()
                .map_or(0, |external| external.history.len())
        })
        .sum();
    let unique_uploads = package
        .files
        .keys()
        .filter(|path| path.starts_with("attachments/"))
        .count();
    let downloaded = package
        .uploads
        .iter()
        .filter(|upload| string(upload, "status") == "downloaded")
        .count();
    let source_counts = json!({
        "teams": records(TEAMS).len(),
        "workflow_states": records(STATES).len(),
        "label_definitions": records(LABELS).len(),
        "label_assignments": label_assignments,
        "issues_with_labels": count_where(ISSUES, &|issue| issue.pointer("/labels/nodes").and_then(Value::as_array).is_some_and(|nodes| !nodes.is_empty()) || !string_list(issue, "labelIds").is_empty()),
        "projects": records(PROJECTS).len(),
        "milestones": records(MILESTONES).len(),
        "initiatives": records(INITIATIVES).len(),
        "issues": records(ISSUES).len(),
        "unprojected_issues": count_where(ISSUES, &|issue| nested_string(issue, &["project", "id"]).is_none()),
        "archived_issues": count_where(ISSUES, &|issue| has_value(issue.get("archivedAt"))),
        "completed_issues": count_where(ISSUES, &|issue| nested_string(issue, &["state", "type"]).as_deref() == Some("completed")),
        "comments": records(COMMENTS).len(),
        "history_entries": history_source,
        "relations": records(RELATIONS).len(),
        "relations_by_type": relation_types,
        "parents": count_where(ISSUES, &|issue| nested_string(issue, &["parent", "id"]).is_some()),
        "documents": records(DOCUMENTS).len(),
        "link_attachments": records(LINK_ATTACHMENTS).len(),
        "uploaded_file_entries": package.uploads.len(),
        "uploaded_files_downloaded": downloaded,
        "uploaded_files_unique": unique_uploads,
        "users": records(USERS).len(),
        "project_updates": records(PROJECT_UPDATES).len(),
        "initiative_updates": records(INITIATIVE_UPDATES).len(),
        "issues_with_previous_identifiers": count_where(ISSUES, &|issue| !string_list(issue, "previousIdentifiers").is_empty()),
        "previous_identifiers": reports.previous_identifiers.len(),
        "data_files": package.files.keys().filter(|path| path.starts_with("data/")).count(),
    });
    let link_count = |kind: IssueLinkKind| {
        archive
            .issue_links
            .iter()
            .filter(|link| link.kind == kind)
            .count()
    };
    let status_count = |status: Status| {
        archive
            .issues
            .iter()
            .filter(|issue| issue.status == status)
            .count()
    };
    let imported_counts = json!({
        "products": archive.products.len(),
        "projects": archive.projects.len(),
        "goals": archive.goals.len(),
        "milestones": archive.milestones.len(),
        "labels": archive.labels.len(),
        "label_assignments": archive.issues.iter().map(|issue| issue.labels.len()).sum::<usize>(),
        "issues": archive.issues.len(),
        "issues_without_project": archive.issues.iter().filter(|issue| issue.project_id.is_none()).count(),
        "legacy_completed": status_count(Status::LegacyCompleted),
        "canceled": status_count(Status::Canceled),
        "backlog": status_count(Status::Backlog),
        "ready": status_count(Status::Ready),
        "owner_verified_done": status_count(Status::Done),
        "claims": archive.issues.iter().filter(|issue| issue.claim.is_some()).count(),
        "comments": archive.comments.len(),
        "history_entries": history_imported,
        "issue_links": {
            "parent": link_count(IssueLinkKind::Parent),
            "blocked_by": link_count(IssueLinkKind::BlockedBy),
            "related": link_count(IssueLinkKind::Related),
        },
        "verification_runs": archive.verifications.len(),
        "releases": archive.releases.len(),
        "events": archive.events.len(),
    });

    let mut totals: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
    let mut by_kind: BTreeMap<String, BTreeMap<&str, usize>> = BTreeMap::new();
    for entry in entries {
        *totals
            .entry(entry.level)
            .or_default()
            .entry(entry.classification.as_str())
            .or_default() += 1;
        *by_kind
            .entry(entry.kind.clone())
            .or_default()
            .entry(entry.classification.as_str())
            .or_default() += 1;
    }
    let inventory = field_inventory(package);
    let unsupported_fields: Vec<Value> = inventory
        .iter()
        .filter(|(kind, _)| natively_imported(kind))
        .flat_map(|(kind, fields)| {
            fields
                .iter()
                .filter(|(_, info)| {
                    info["classification"] == "preserved"
                        && info["records_with_value"].as_u64().unwrap_or(0) > 0
                })
                .map(move |(field, info)| {
                    json!({"kind": kind, "field": field, "records_with_value": info["records_with_value"]})
                })
        })
        .collect();
    let preserved_only: BTreeMap<String, usize> = entries
        .iter()
        .filter(|entry| entry.level != "file" && entry.classification == Class::Preserved)
        .fold(BTreeMap::new(), |mut map, entry| {
            *map.entry(entry.kind.clone()).or_default() += 1;
            map
        });
    let unresolved: BTreeMap<String, usize> = entries
        .iter()
        .filter(|entry| entry.classification == Class::Unresolved)
        .fold(BTreeMap::new(), |mut map, entry| {
            *map.entry(entry.kind.clone()).or_default() += 1;
            map
        });
    let absent_optional: Vec<&str> = EXPECTED_OPTIONAL
        .iter()
        .filter(|file| !package.data.contains_key(**file))
        .copied()
        .collect();
    let mut source_limitations: Vec<Value> = package
        .manifest
        .missing_coverage
        .iter()
        .map(|item| json!({"source": "capture manifest", "detail": item}))
        .collect();
    for file in &absent_optional {
        source_limitations.push(json!({"source": "importer", "detail": format!("data/{file} was not captured, so that record type cannot be accounted")}));
    }
    if !package.unbundled_root_entries.is_empty() {
        source_limitations.push(json!({"source": "importer", "detail": "Package root holds entries outside the capture format; they are not verified or bundled", "entries": package.unbundled_root_entries}));
    }

    let archived_or_trashed = count_where(ISSUES, &|issue| {
        issue.get("trashed").and_then(Value::as_bool) == Some(true)
    });
    // Access: native Direct records, retained-only records readable in Direct, and missing bytes.
    let mut access: BTreeMap<&str, usize> = BTreeMap::new();
    for entry in entries.iter().filter(|entry| entry.level != "file") {
        let key = if entry.kind == "uploaded_file" && entry.classification == Class::Unresolved {
            "missing"
        } else if matches!(entry.classification, Class::Native | Class::Transformed) {
            "native"
        } else {
            "retained"
        };
        *access.entry(key).or_default() += 1;
    }
    let missing = access.get("missing").copied().unwrap_or(0);
    let mut review_items = Vec::new();
    for (code, detail, count) in [
        ("unsupported_fields", "Populated source fields without a native Direct field; readable in each retained record.", unsupported_fields.len()),
        ("unresolved_records", "Records or components not represented as Direct planning records; readable as retained records.", unresolved.values().sum::<usize>()),
        ("unsupported_relation_semantics", "Linear relation types without a Direct link kind; retained unconverted.", reports.relations_unsupported.len()),
        ("cross_product_mapping", "Linear structures spanning teams; see cross_product_mappings.", reports.cross_product.len()),
        ("source_limitations", "Coverage the capture itself cannot prove; see source_limitations.", source_limitations.len()),
        ("trashed_issues", "Linear marks these issues as trashed; decide whether they stay.", archived_or_trashed),
    ] {
        if count > 0 {
            review_items.push(json!({"code": code, "detail": detail, "count": count}));
        }
    }
    let gates = json!({
        "preservation": {
            "status": "verified",
            "detail": "Every manifest-listed file matched its checksum and is retained byte for byte.",
            "files": package.files.len(),
        },
        "access": {
            "status": if missing == 0 { "available_in_direct" } else { "incomplete" },
            "detail": "native = a Direct record exists and the original is readable; retained = readable only as an original source record in Direct; missing = bytes were never captured.",
            "records": access,
        },
        "freshness": {
            "status": "unverified",
            "captured_at": package.manifest.captured_at,
            "detail": "The importer cannot know whether Linear changed after this capture. Freeze Linear intake, take a final capture, and prepare from it.",
        },
        "application": {
            "status": "not_applied",
            "detail": "Apply a prepared artifact to the owner's workspace through the owner Migration panel; this report does not change any workspace.",
        },
        "owner_decision": {"status": "pending"},
    });
    let mut blockers = vec![
        json!({"code": "final_capture_freshness", "detail": "Freshness of the capture against live Linear is not verified."}),
        json!({"code": "live_application_pending", "detail": "Nothing has been applied to the owner's workspace."}),
        json!({"code": "owner_cutover_decision_required", "detail": "Cutover requires an explicit owner decision after reviewing this report."}),
    ];
    if missing > 0 {
        blockers.push(json!({"code": "missing_source_bytes", "detail": "Some uploaded files were never downloaded, so their contents exist only in Linear.", "count": missing}));
    }

    json!({
        "format": 1,
        "mode": "whole_workspace",
        "source": {
            "service": "Linear",
            "capture_format": package.manifest.format,
            "captured_at": package.manifest.captured_at,
            "manifest_sha256": package.manifest_sha256,
            "recorded_upload_failures": package.uploads.len() - downloaded,
            "missing_coverage": package.manifest.missing_coverage,
        },
        "source_counts": source_counts,
        "imported_counts": imported_counts,
        "accounting": {
            "records_in_source": records_in_source,
            "records_accounted": records_accounted,
            "uploaded_files_accounted": package.uploads.len(),
            "complete": records_in_source == records_accounted,
            "detail": ACCOUNTING_FILE,
        },
        "classification_totals": totals,
        "classification_by_kind": by_kind,
        "field_inventory": inventory,
        "unsupported_fields": unsupported_fields,
        "label_disambiguations": reports.label_disambiguations,
        "name_disambiguations": reports.name_disambiguations,
        "relations": {
            "deduplicated": reports.relations_deduplicated,
            "unsupported_semantics": reports.relations_unsupported,
            "unresolved": reports.relations_unresolved,
            "archived_not_imported": reports.relations_archived,
        },
        "cross_product_mappings": reports.cross_product,
        "previous_identifiers": reports.previous_identifiers,
        "unknown_state_types": reports.unknown_states,
        "source_limitations": source_limitations,
        "native_access": {
            "status": if missing == 0 { "available_in_direct" } else { "incomplete" },
            "statement": NATIVE_ACCESS,
            "retained_only": preserved_only,
            "bundle": BUNDLE_DIR,
            "index": INDEX_FILE,
        },
        "policies": {
            "legacy_completion": "Linear completed issues use legacy_completed with source state, timestamps and history. No Direct verification run, readiness, claim or owner acceptance is created, and Legacy done does not count as owner-verified Done.",
            "open_work": "Triage, backlog, unstarted and started Linear issues are imported as Backlog; readiness remains an owner decision.",
            "releases": "The Linear capture has no release or deployment record type, so no Direct release or release evidence is invented.",
            "history": "Linear issue history is kept in each issue's external provenance. No Direct activity events are created from source payloads.",
        },
        "cutover_readiness": {
            "status": "not_complete",
            "ready": false,
            "gates": gates,
            "blockers": blockers,
            "review_items": review_items,
        },
        "archive": {
            "format": archive.format,
            "workspace_id": archive.workspace_id,
        },
    })
}

fn field_inventory(package: &VerifiedPackage) -> BTreeMap<String, BTreeMap<String, Value>> {
    let mut inventory: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    for (file, value) in &package.data {
        let Some(records) = value.get("records").and_then(Value::as_array) else {
            continue;
        };
        let kind = kind_for(file);
        let fields = inventory.entry(kind.clone()).or_default();
        for record in records {
            let Some(map) = record.as_object() else {
                continue;
            };
            for (field, value) in map {
                let (class, reason) = field_rule(&kind, field);
                let info = fields.entry(field.clone()).or_insert_with(|| {
                    json!({"classification": class.as_str(), "reason": reason, "records_with_value": 0})
                });
                if has_value(Some(value)) {
                    info["records_with_value"] =
                        json!(info["records_with_value"].as_u64().unwrap_or(0) + 1);
                }
            }
        }
    }
    inventory
}

// ---------------------------------------------------------------------------
// Field rules and helpers

fn kind_for(file: &str) -> String {
    match file {
        TEAMS => "team",
        STATES => "workflow_state",
        LABELS => "label",
        PROJECTS => "project",
        MILESTONES => "milestone",
        INITIATIVES => "initiative",
        ISSUES => "issue",
        COMMENTS => "comment",
        RELATIONS => "relation",
        DOCUMENTS => "document",
        USERS => "user",
        LINK_ATTACHMENTS => "link_attachment",
        PROJECT_UPDATES => "project_update",
        INITIATIVE_UPDATES => "initiative_update",
        other => return format!("unrecognized:{}", other.trim_end_matches(".json")),
    }
    .into()
}

fn field_rule(kind: &str, field: &str) -> (Class, &'static str) {
    use Class::{Native as N, Transformed as T};
    let rule: Option<(Class, &'static str)> = match (kind, field) {
        (_, "id") if !matches!(kind, "user" | "document" | "link_attachment" | "project_update" | "initiative_update") && !kind.starts_with("unrecognized") => Some((N, "Direct record id or external provenance id")),
        ("team", "key") => Some((N, "Product key and issue-key prefix")),
        ("team", "name") => Some((N, "Product name")),
        ("workflow_state", "name" | "type") => Some((T, "Mapped to a Direct status; each issue keeps its state id, name and type in external provenance")),
        ("label", "name") => Some((N, "Canonical label name (disambiguated on collision); original kept in linear_origins")),
        ("label", "description") => Some((N, "Label description (truncated at 1000 bytes if longer)")),
        ("label", "color") => Some((N, "Label color when it is #rrggbb")),
        ("label", "team") => Some((N, "Label product rule; workspace labels apply to every product")),
        ("project", "name") | ("milestone", "name") | ("initiative", "name") => Some((N, "Record name (disambiguated when a Direct uniqueness rule requires it)")),
        ("project" | "milestone" | "initiative", "description") => Some((N, "Record description")),
        ("project", "state" | "status") | ("initiative", "status") => Some((N, "Planning status (unknown values reported)")),
        ("project" | "initiative", "priority") => Some((N, "Shared priority scale (absent or 0 is reported as a transformation)")),
        ("project" | "milestone", "sortOrder") => Some((T, "Normalized to a non-negative rank that keeps the source order")),
        ("project" | "initiative", "url") => Some((N, "External URL")),
        ("project", "teams") => Some((T, "Direct projects belong to one product; multi-team projects use their primary team (reported)")),
        ("project", "initiatives") | ("initiative", "projects") => Some((N, "Goal project links (split per product when projects span products)")),
        ("initiative", "teams") => Some((N, "Goal product")),
        ("project", "milestones" | "projectMilestones") => {
            Some((N, "Imported as milestone records from project-milestones.json"))
        }
        ("milestone", "project") => Some((N, "Milestone project")),
        ("project" | "milestone" | "initiative" | "label", "createdAt" | "updatedAt")
        | ("comment" | "relation", "createdAt") => Some((T, SECONDS)),
        ("issue", "identifier") => Some((N, "Direct issue key")),
        ("issue", "number") => Some((T, "Numeric suffix of the Direct issue key")),
        ("issue", "title") => Some((N, "Issue title")),
        ("issue", "description") => Some((N, "Issue body")),
        ("issue", "priority") => Some((T, "Priority scale; Linear 0 (No priority) is stored as low and reported")),
        ("issue", "state") => Some((N, "Direct status plus external state id, name and type")),
        ("issue", "team") => Some((N, "Issue product")),
        ("issue", "project") => Some((N, "Issue project when the project is in the same product (otherwise reported)")),
        ("issue", "projectMilestone") => Some((N, "Issue milestone when it belongs to the issue's Direct project")),
        ("issue", "parent") => Some((N, "Parent link from child to parent")),
        ("issue", "labels" | "labelIds") => Some((N, "Label assignments")),
        ("issue", "assignee") => Some((T, "Assignee display name stored as owner text; user identity retained in the source bundle")),
        ("issue", "history") => Some((N, "external.history")),
        ("issue", "url" | "createdAt" | "updatedAt" | "startedAt" | "completedAt" | "canceledAt" | "archivedAt") => Some((N, "External issue provenance")),
        ("issue", "previousIdentifiers") => Some((Class::Preserved, "Mapped to the current Direct key in the index and reconciliation report; not a Direct field")),
        ("comment", "body") => Some((N, "Comment body")),
        ("comment", "url") => Some((N, "Comment external URL")),
        ("comment", "issue" | "issueId") => Some((N, "Comment issue")),
        ("comment", "user") => Some((T, "Author display name stored as the comment actor")),
        ("comment", "parent") => Some((T, "Reply thread flattened; parent retained in the source bundle")),
        ("relation", "type") => Some((N, "blocks → blocked_by (reversed onto the blocked issue), related → related; other types retained unconverted")),
        ("relation", "issue" | "relatedIssue") => Some((N, "Link endpoints")),
        _ => None,
    };
    rule.unwrap_or((Class::Preserved, PRESERVED))
}

/// Record kinds that become Direct records; other kinds are preserved whole.
fn natively_imported(kind: &str) -> bool {
    matches!(
        kind,
        "team"
            | "workflow_state"
            | "label"
            | "project"
            | "milestone"
            | "initiative"
            | "issue"
            | "comment"
            | "relation"
    )
}

fn preserved_fields(kind: &str, value: &Value) -> Vec<String> {
    let Some(map) = value.as_object() else {
        return Vec::new();
    };
    map.iter()
        .filter(|(field, value)| {
            field_rule(kind, field).0 == Class::Preserved && has_value(Some(value))
        })
        .map(|(field, _)| field.clone())
        .collect()
}

fn source_label(value: &Value) -> Option<String> {
    ["identifier", "key", "name", "title"]
        .into_iter()
        .find_map(|field| optional_string(value, field).filter(|text| !text.is_empty()))
}

/// Whether a source value carries information. Null, false, "", [], {} and
/// empty connections are defaults and carry none.
pub(super) fn has_value(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => {
            !(map.is_empty()
                || (map
                    .get("nodes")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
                    && map.keys().all(|key| key == "nodes" || key == "pageInfo")))
        }
        Some(_) => true,
    }
}

struct MappedState {
    status: Status,
    reason: Option<&'static str>,
    transformed: bool,
    unresolved: bool,
}

fn map_state(kind: &str, has_completed_at: bool) -> MappedState {
    let mapped = |status, reason, transformed, unresolved| MappedState {
        status,
        reason,
        transformed,
        unresolved,
    };
    match kind {
        "completed" if has_completed_at => mapped(Status::LegacyCompleted, Some("Linear completion stored as Legacy done; no Direct verification run or owner acceptance is created"), false, false),
        "completed" => mapped(Status::Backlog, Some("Completed state without completedAt cannot be Legacy done; imported as backlog pending review"), false, true),
        "canceled" | "cancelled" | "duplicate" => mapped(Status::Canceled, None, false, false),
        "backlog" => mapped(Status::Backlog, None, false, false),
        "triage" | "unstarted" | "started" => mapped(Status::Backlog, Some("Open Linear work is imported as Backlog; readiness and claims are owner and agent decisions, not inferred"), true, false),
        _ => mapped(Status::Backlog, Some("Unknown Linear state type; imported as backlog pending a mapping decision"), false, true),
    }
}

fn status_name(status: &Status) -> &'static str {
    match status {
        Status::Backlog => "backlog",
        Status::Ready => "ready",
        Status::Doing => "doing",
        Status::Verify => "verify",
        Status::Done => "done",
        Status::LegacyCompleted => "legacy_completed",
        Status::Canceled => "canceled",
    }
}

fn link_kind_name(kind: &IssueLinkKind) -> &'static str {
    match kind {
        IssueLinkKind::Parent => "parent",
        IssueLinkKind::BlockedBy => "blocked_by",
        IssueLinkKind::Related => "related",
        IssueLinkKind::LegacyVerification => "legacy_verification",
    }
}

fn map_project_status(raw: &str) -> (ProjectStatus, Option<String>) {
    match raw {
        "planned" => (ProjectStatus::Planned, None),
        "started" | "active" => (ProjectStatus::Active, None),
        "paused" => (ProjectStatus::Paused, None),
        "completed" | "done" => (ProjectStatus::Completed, None),
        "canceled" | "cancelled" => (ProjectStatus::Canceled, None),
        "backlog" => (ProjectStatus::Planned, Some("Linear backlog project status stored as planned".into())),
        other => (ProjectStatus::Planned, Some(format!("Unknown Linear project status {other:?}; stored as planned pending a mapping decision"))),
    }
}

fn map_goal_status(raw: &str) -> (GoalStatus, Option<String>) {
    match raw {
        "planned" => (GoalStatus::Planned, None),
        "started" | "active" => (GoalStatus::Active, None),
        "paused" => (GoalStatus::Paused, None),
        "completed" | "done" => (GoalStatus::Completed, None),
        "canceled" | "cancelled" => (GoalStatus::Canceled, None),
        "backlog" => (GoalStatus::Planned, Some("Linear backlog initiative status stored as planned".into())),
        other => (GoalStatus::Planned, Some(format!("Unknown Linear initiative status {other:?}; stored as planned pending a mapping decision"))),
    }
}

fn map_priority(value: &Value) -> (String, Option<String>) {
    match value.get("priority") {
        None | Some(Value::Null) => ("medium".into(), Some("No Linear priority; stored with Direct's default medium".into())),
        Some(priority) => match priority
            .as_f64()
            .filter(|value| value.fract() == 0.0)
            .map(|value| value as i64)
        {
            Some(1) => ("urgent".into(), None),
            Some(2) => ("high".into(), None),
            Some(3) => ("medium".into(), None),
            Some(4) => ("low".into(), None),
            Some(0) => ("low".into(), Some("Linear priority 0 (No priority) has no Direct equivalent; stored as low (original value retained in the source bundle)".into())),
            _ => ("low".into(), Some(format!("Unrecognized Linear priority {priority}; stored as low"))),
        },
    }
}

fn same_logical_link(link: &IssueLink, source: &str, target: &str, kind: &IssueLinkKind) -> bool {
    link.kind == *kind
        && ((link.source_key == source && link.target_key == target)
            || (*kind == IssueLinkKind::Related
                && link.source_key == target
                && link.target_key == source))
}

fn creates_cycle(links: &[IssueLink], source: &str, target: &str, kind: &IssueLinkKind) -> bool {
    if !matches!(kind, IssueLinkKind::Parent | IssueLinkKind::BlockedBy) {
        return false;
    }
    let mut stack = vec![target];
    let mut visited = HashSet::new();
    while let Some(current) = stack.pop() {
        if current == source {
            return true;
        }
        if visited.insert(current) {
            stack.extend(
                links
                    .iter()
                    .filter(|link| link.kind == *kind && link.source_key == current)
                    .map(|link| link.target_key.as_str()),
            );
        }
    }
    false
}

fn node_ids(value: &Value, field: &str) -> Vec<String> {
    value
        .get(field)
        .and_then(|connection| connection.get("nodes"))
        .and_then(Value::as_array)
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|node| optional_string(node, "id"))
                .collect()
        })
        .unwrap_or_default()
}

fn string_list(value: &Value, field: &str) -> Vec<String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn number(value: &Value, field: &str) -> f64 {
    value
        .get(field)
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .unwrap_or(0.0)
}

fn non_empty(value: &Value, field: &str) -> Option<String> {
    optional_string(value, field).filter(|text| !text.trim().is_empty())
}

fn normalized(value: &str) -> String {
    value.trim().to_lowercase()
}

fn short_id(id: &str) -> String {
    id.chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect()
}

fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_owned();
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

/// Fit `base + suffix` into `max` bytes by shortening the base.
fn fit(base: &str, suffix: &str, max: usize) -> String {
    let base = truncate(base, max.saturating_sub(suffix.len()));
    format!("{}{suffix}", base.trim_end())
}

fn valid_color(value: &str) -> bool {
    value.is_empty()
        || (value.len() == 7
            && value.starts_with('#')
            && value[1..].bytes().all(|c| c.is_ascii_hexdigit()))
}

fn uuid_or_derived(id: &str, namespace: &str) -> String {
    if Uuid::parse_str(id).is_ok() {
        id.to_owned()
    } else {
        deterministic_uuid(namespace, id)
    }
}

/// Assign names that are unique (case-insensitively, trimmed) within each
/// scope and fit `max` bytes. Returns id → name.
fn unique_names(
    items: impl Iterator<Item = (String, String, String)>,
    max: usize,
    fallback: &str,
    reserved: &HashSet<(String, String)>,
) -> HashMap<String, String> {
    let mut items: Vec<_> = items.collect();
    items.sort_by(|left, right| {
        (&left.0, normalized(&left.1), &left.2).cmp(&(&right.0, normalized(&right.1), &right.2))
    });
    let mut counts: HashMap<(String, String), usize> = HashMap::new();
    for (scope, name, _) in &items {
        *counts.entry((scope.clone(), normalized(name))).or_default() += 1;
    }
    let mut taken: HashSet<(String, String)> = reserved.clone();
    // Reserve names that are already unique and valid so disambiguated names never displace them.
    for (scope, name, _) in &items {
        let trimmed = name.trim();
        if !trimmed.is_empty()
            && trimmed.len() <= max
            && counts[&(scope.clone(), normalized(name))] == 1
            && !reserved.contains(&(scope.clone(), normalized(name)))
        {
            taken.insert((scope.clone(), normalized(trimmed)));
        }
    }
    let mut names = HashMap::new();
    for (scope, name, id) in items {
        let trimmed = name.trim();
        let unique = !trimmed.is_empty()
            && trimmed.len() <= max
            && counts[&(scope.clone(), normalized(&name))] == 1
            && !reserved.contains(&(scope.clone(), normalized(&name)));
        let chosen = if unique {
            trimmed.to_owned()
        } else {
            let base = if trimmed.is_empty() {
                format!("{fallback} {}", short_id(&id))
            } else {
                trimmed.to_owned()
            };
            [
                fit(&base, "", max),
                fit(&base, &format!(" ({})", short_id(&id)), max),
                fit(&base, &format!(" ({id})"), max),
            ]
            .into_iter()
            .find(|candidate| !taken.contains(&(scope.clone(), normalized(candidate))))
            .unwrap_or_else(|| fit(&base, &format!(" ({id})"), max))
        };
        taken.insert((scope.clone(), normalized(&chosen)));
        names.insert(id, chosen);
    }
    names
}

/// Rank records within each scope by (source order, name, id). Returns id → rank.
fn ranks(items: impl Iterator<Item = (String, f64, String, String)>) -> HashMap<String, i64> {
    let mut items: Vec<_> = items.collect();
    items.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.total_cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.3.cmp(&right.3))
    });
    let mut ranks = HashMap::new();
    let mut previous_scope = None;
    let mut rank = 0;
    for (scope, _, _, id) in items {
        if previous_scope.as_ref() != Some(&scope) {
            rank = 0;
            previous_scope = Some(scope);
        }
        ranks.insert(id, rank);
        rank += 1;
    }
    ranks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_names_keep_valid_names_and_disambiguate_the_rest() {
        let names = unique_names(
            vec![
                ("p".to_owned(), "Launch".to_owned(), "b-2".to_owned()),
                ("p".to_owned(), "launch".to_owned(), "a-1".to_owned()),
                ("p".to_owned(), "Other".to_owned(), "c-3".to_owned()),
                ("q".to_owned(), "Launch".to_owned(), "d-4".to_owned()),
                ("p".to_owned(), "  ".to_owned(), "e-5".to_owned()),
            ]
            .into_iter(),
            160,
            "Linear project",
            &HashSet::new(),
        );
        assert_eq!(names["c-3"], "Other");
        assert_eq!(names["d-4"], "Launch");
        let first = names["a-1"].to_lowercase();
        let second = names["b-2"].to_lowercase();
        assert_ne!(first, second);
        assert!(first.starts_with("launch") && second.starts_with("launch"));
        assert_eq!(names["e-5"], "Linear project e5");
    }

    #[test]
    fn fit_respects_byte_limits_on_char_boundaries() {
        let name = fit(&"é".repeat(60), " (ENG)", 80);
        assert!(name.len() <= 80);
        assert!(name.ends_with(" (ENG)"));
    }

    #[test]
    fn empty_connections_carry_no_value() {
        assert!(!has_value(Some(
            &json!({"nodes": [], "pageInfo": {"hasNextPage": false}})
        )));
        assert!(has_value(Some(&json!({"nodes": [{"id": "x"}]}))));
        assert!(has_value(Some(&json!(0))));
        assert!(has_value(Some(&json!(true))));
        assert!(!has_value(Some(&json!(false))));
        assert!(!has_value(Some(&json!(""))));
    }
}
