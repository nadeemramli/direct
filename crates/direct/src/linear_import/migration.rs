//! Offline preparation of a migration artifact for an EXISTING workspace.
//!
//! The artifact binds to a fresh owner export (workspace identity and event
//! cursor). Preparation rehearses the real core transaction on a restored copy
//! of that export: apply, exact replay, rollback and re-apply, comparing every
//! pre-existing record each time. It never opens the owner's database.

use super::{
    ensure_private_output,
    package::verify_package,
    pretty_bytes, resolved_path, retained, sha256,
    workspace::{self, bundle_summary, Target},
    write_new,
};
use anyhow::{bail, Context, Result};
use direct_core::{
    artifact_sha256, encode_migration_artifact, validate_archive, Archive, ArtifactFile, Command,
    MigrationArtifact, Request, Role, Status, Store,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::Path,
};
use uuid::Uuid;

const ARTIFACT_FILE: &str = "linear-migration.direct-migration";
const ARTIFACT_CHECKSUM: &str = "linear-migration.sha256";
const PREVIEW_FILE: &str = "preview.json";
const REPORT_FILE: &str = "reconciliation.json";
const ACCOUNTING_FILE: &str = "accounting.json";
const REHEARSAL_FILE: &str = "rehearsal-report.json";
const MARKER_FILE: &str = "prepare-complete.json";
const REHEARSAL_ACTOR: &str = "linear-migration-rehearsal";

pub fn prepare(
    source: &Path,
    target_export: &Path,
    output: &Path,
    team_maps: &[String],
    owner_data_dir: Option<&Path>,
) -> Result<Value> {
    ensure_private_output(output, owner_data_dir)?;
    let source_root = source
        .canonicalize()
        .context("Linear source package does not exist")?;
    let resolved = resolved_path(output);
    if resolved.starts_with(&source_root) || source_root.starts_with(&resolved) {
        bail!("The migration output must not overlap the Linear source package");
    }
    let target_bytes = fs::read(target_export).context("Read the target workspace export")?;
    let target_archive: Archive =
        serde_json::from_slice(&target_bytes).context("Target export is not a Direct archive")?;
    validate_archive(&target_archive).context("Target export is not a valid Direct archive")?;
    let package = verify_package(&source_root, workspace::WHOLE_REQUIRED)?;
    if target_archive.source_bundles.iter().any(|bundle| {
        bundle
            .manifest_sha256
            .eq_ignore_ascii_case(&package.manifest_sha256)
    }) {
        bail!("The target workspace already retains this Linear capture; there is nothing to prepare. Re-applying the original artifact is an exact no-op.");
    }
    let target = target_from(&target_archive, team_maps)?;
    let plan = workspace::build(&package, &target)?;
    let archive = &plan.archive;
    let mut collisions = Vec::new();
    let ids = archive
        .products
        .iter()
        .map(|p| &p.id)
        .chain(archive.projects.iter().map(|p| &p.id))
        .chain(archive.goals.iter().map(|g| &g.id))
        .chain(archive.milestones.iter().map(|m| &m.id))
        .chain(archive.labels.iter().map(|l| &l.id))
        .chain(archive.issues.iter().map(|i| &i.id))
        .chain(archive.comments.iter().map(|c| &c.id))
        .chain(archive.issue_links.iter().map(|l| &l.id));
    for id in ids {
        if target.ids.contains(id) {
            collisions.push(id.clone());
        }
    }
    if !collisions.is_empty() {
        collisions.truncate(20);
        bail!(
            "Imported record IDs already exist in the target workspace; refusing to overwrite: {}",
            collisions.join(", ")
        );
    }

    let mut report = plan.reconciliation.clone();
    report["mode"] = json!("migration_artifact");
    report["target"] = json!({
        "workspace_id": target_archive.workspace_id,
        "baseline_cursor": target.baseline_cursor,
        "export_sha256": sha256(&target_bytes),
        "existing_products": target_archive.products.len(),
        "existing_issues": target_archive.issues.len(),
        "team_mappings": plan.reused_products,
        "reused_labels": plan.reused_labels,
    });
    report["cutover_readiness"]["gates"]["application"] = json!({
        "status": "prepared",
        "detail": "Apply this artifact in the owner's Migration panel. The service re-checks every collision against the live workspace, writes a pre-import backup and applies in one transaction.",
    });
    let retained = retained::build(
        &package,
        &plan.entries,
        &plan.issue_keys,
        bundle_summary(&report),
        "migration",
        0,
    )?;
    let mut records = retained.records;
    records.sort_by(|a, b| a.id.cmp(&b.id));
    let mut payload = Vec::with_capacity(retained.bundle.total_bytes as usize);
    let files = retained
        .files
        .iter()
        .map(|(file, bytes)| {
            payload.extend_from_slice(bytes);
            ArtifactFile {
                path: file.path.clone(),
                sha256: file.sha256.clone(),
                bytes: file.bytes,
                content_type: file.content_type.clone(),
                role: file.role.clone(),
                original_name: file.original_name.clone(),
            }
        })
        .collect();
    let artifact = MigrationArtifact {
        format: 1,
        source: "linear".into(),
        target_workspace_id: target_archive.workspace_id.clone(),
        baseline_cursor: target.baseline_cursor,
        products: archive.products.clone(),
        projects: archive.projects.clone(),
        goals: archive.goals.clone(),
        milestones: archive.milestones.clone(),
        labels: archive.labels.clone(),
        issues: archive.issues.clone(),
        comments: archive.comments.clone(),
        issue_links: archive.issue_links.clone(),
        reused_products: plan.reused_products.clone(),
        reused_labels: plan.reused_labels.clone(),
        reused_label_origins: plan.reused_label_origins.clone(),
        bundle: retained.bundle,
        files,
        records,
    };
    let bytes = encode_migration_artifact(&artifact, &payload)?;
    let digest = artifact_sha256(&bytes);
    let at = chrono::DateTime::parse_from_rfc3339(&package.manifest.captured_at)
        .map(|time| time.timestamp())
        .unwrap_or(1);

    // Deterministic generated files (the rehearsal database is verified, not compared).
    let accounting = json!({"format": 1, "entries": plan.entries});
    let mut generated: BTreeMap<&'static str, Vec<u8>> = BTreeMap::new();
    generated.insert(ARTIFACT_FILE, bytes.clone());
    generated.insert(
        ARTIFACT_CHECKSUM,
        format!("{digest}  {ARTIFACT_FILE}\n").into_bytes(),
    );
    generated.insert(REPORT_FILE, pretty_bytes(&report)?);
    generated.insert(ACCOUNTING_FILE, pretty_bytes(&accounting)?);

    let replay = fs::symlink_metadata(output).is_ok();
    let rehearsal = if replay {
        None
    } else {
        Some(publish(
            output,
            &target_archive,
            &bytes,
            &digest,
            at,
            &mut generated,
        )?)
    };
    let (preview, rehearsal_report) = verify(output, &generated)?;
    Ok(json!({
        "mode": "migration_prepare",
        "idempotent_replay": replay,
        "output": output,
        "artifact": output.join(ARTIFACT_FILE),
        "artifact_sha256": digest,
        "artifact_bytes": bytes.len(),
        "target_workspace_id": target_archive.workspace_id,
        "baseline_cursor": target.baseline_cursor,
        "preview": preview,
        "rehearsal": rehearsal.unwrap_or(rehearsal_report),
        "cutover_readiness": report["cutover_readiness"],
        "source_counts": report["source_counts"],
        "imported_counts": report["imported_counts"],
    }))
}

fn target_from(archive: &Archive, team_maps: &[String]) -> Result<Target> {
    let mut team_map = HashMap::new();
    for mapping in team_maps {
        let (team, product) = mapping.split_once('=').with_context(|| {
            format!("--map-team expects LINEAR_TEAM_ID=DIRECT_PRODUCT_ID, got {mapping:?}")
        })?;
        if !archive.products.iter().any(|p| p.id == product) {
            bail!("--map-team names unknown Direct product {product}");
        }
        if team_map
            .insert(team.to_owned(), product.to_owned())
            .is_some()
        {
            bail!("--map-team names Linear team {team} twice");
        }
    }
    let normalized = |value: &str| value.trim().to_lowercase();
    let mut ids = HashSet::new();
    ids.extend(archive.products.iter().map(|p| p.id.clone()));
    ids.extend(archive.projects.iter().map(|p| p.id.clone()));
    ids.extend(archive.goals.iter().map(|g| g.id.clone()));
    ids.extend(archive.milestones.iter().map(|m| m.id.clone()));
    ids.extend(archive.labels.iter().map(|l| l.id.clone()));
    ids.extend(archive.issues.iter().map(|i| i.id.clone()));
    ids.extend(archive.comments.iter().map(|c| c.id.clone()));
    ids.extend(archive.issue_links.iter().map(|l| l.id.clone()));
    let mut keys: HashSet<String> = archive.issues.iter().map(|i| i.key.clone()).collect();
    keys.extend(archive.events.iter().map(|event| event.entity.clone()));
    Ok(Target {
        baseline_cursor: archive
            .events
            .iter()
            .map(|event| event.seq)
            .max()
            .unwrap_or(0),
        products: archive.products.clone(),
        team_map,
        labels: archive.labels.clone(),
        keys,
        project_names: archive
            .projects
            .iter()
            .map(|p| (p.product_id.clone(), normalized(&p.name)))
            .collect(),
        goal_names: archive
            .goals
            .iter()
            .map(|g| (g.product_id.clone(), normalized(&g.name)))
            .collect(),
        ids,
    })
}

/// Differences in records that existed before (`before`) compared with `after`.
pub(super) fn changed_existing(before: &Archive, after: &Archive) -> Result<Vec<String>> {
    let mut differences = Vec::new();
    if before.workspace_id != after.workspace_id {
        differences.push("workspace identity changed".into());
    }
    let before = serde_json::to_value(before)?;
    let after = serde_json::to_value(after)?;
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
        let index: HashMap<String, &Value> = after[collection]
            .as_array()
            .into_iter()
            .flatten()
            .map(|item| (item[key].to_string(), item))
            .collect();
        for item in before[collection].as_array().into_iter().flatten() {
            match index.get(&item[key].to_string()) {
                Some(current) if *current == item => {}
                Some(_) => differences.push(format!("{collection} {} changed", item[key])),
                None => differences.push(format!("{collection} {} is missing", item[key])),
            }
        }
    }
    let files = |value: &Value| -> HashMap<String, Value> {
        value["source_files"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|file| {
                (
                    format!("{}/{}", file["bundle_id"], file["path"]),
                    file.clone(),
                )
            })
            .collect()
    };
    let after_files = files(&after);
    for (id, file) in files(&before) {
        if after_files.get(&id) != Some(&file) {
            differences.push(format!("source file {id} changed or is missing"));
        }
    }
    Ok(differences)
}

fn rehearse(
    dir: &Path,
    target: &Archive,
    bytes: &[u8],
    digest: &str,
    at: i64,
) -> Result<(Value, Value)> {
    let mut store = Store::open(&dir.join("direct.db"))?;
    store.restore(target.clone())?;
    let before = store.export()?;
    let preview = store.preview_migration(bytes)?;
    if preview["status"] != "ready_to_apply" {
        bail!("Rehearsal preview did not report ready_to_apply: {preview}");
    }
    let cursor = preview["expected_cursor"].as_u64().unwrap_or(0);
    let applied = store.apply_migration(bytes, cursor, digest, REHEARSAL_ACTOR, None, at)?;
    let after = store.export()?;
    let changed_after_apply = changed_existing(&before, &after)?;
    let (artifact, _) = direct_core::parse_migration_artifact(bytes)?;
    let added_issues = after.issues.len() - before.issues.len();
    let imported: HashSet<&str> = artifact.issues.iter().map(|i| i.key.as_str()).collect();
    let fabricated = after
        .issues
        .iter()
        .filter(|issue| imported.contains(issue.key.as_str()))
        .filter(|issue| {
            issue.claim.is_some()
                || matches!(
                    issue.status,
                    Status::Ready | Status::Doing | Status::Verify | Status::Done
                )
        })
        .count();
    let applied_cursor = applied["applied_cursor"].as_u64().unwrap_or(0);
    let replay = store.apply_migration(bytes, applied_cursor, digest, REHEARSAL_ACTOR, None, at)?;
    let after_replay = store.export()?;
    let replay_identical = serde_json::to_value(&after_replay)? == serde_json::to_value(&after)?;
    let rollback = store.execute_at(
        Request {
            actor: REHEARSAL_ACTOR.into(),
            request_id: format!("rehearsal-rollback-{}", &digest[..16]),
            command: Command::RollbackMigration {
                bundle_id: artifact.bundle.id.clone(),
                expected_cursor: applied_cursor,
            },
        },
        Role::Human,
        at,
    )?;
    let after_rollback = store.export()?;
    let changed_after_rollback = changed_existing(&before, &after_rollback)?;
    let rollback_exact = changed_after_rollback.is_empty()
        && after_rollback.issues.len() == before.issues.len()
        && after_rollback.products.len() == before.products.len()
        && after_rollback.labels.len() == before.labels.len()
        && after_rollback.source_bundles.len() == before.source_bundles.len()
        && after_rollback.events.len() == before.events.len() + 1;
    let preview_again = store.preview_migration(bytes)?;
    let reapplied = store.apply_migration(
        bytes,
        preview_again["expected_cursor"].as_u64().unwrap_or(0),
        digest,
        REHEARSAL_ACTOR,
        None,
        at,
    )?;
    let final_export = store.export()?;
    let changed_final = changed_existing(&before, &final_export)?;
    let passed = changed_after_apply.is_empty()
        && added_issues == artifact.issues.len()
        && fabricated == 0
        && after.verifications.len() == before.verifications.len()
        && replay["status"] == "already_applied"
        && replay_identical
        && rollback["status"] == "rolled_back"
        && rollback_exact
        && reapplied["status"] == "applied"
        && changed_final.is_empty();
    let report = json!({
        "status": if passed { "passed" } else { "failed" },
        "scope": "Real core migration transaction on a restored copy of the target export; the owner's workspace is not opened.",
        "existing_records_unchanged_after_apply": changed_after_apply.is_empty(),
        "existing_record_differences": changed_after_apply.iter().chain(changed_final.iter()).take(50).collect::<Vec<_>>(),
        "issues_added": added_issues,
        "verification_runs_added": after.verifications.len() - before.verifications.len(),
        "fabricated_readiness_or_claims": fabricated,
        "exact_replay": replay["status"],
        "replay_changed_nothing": replay_identical,
        "rollback": rollback["status"],
        "rollback_restored_existing_state": rollback_exact,
        "rollback_differences": changed_after_rollback.into_iter().take(50).collect::<Vec<_>>(),
        "reapply": reapplied["status"],
        "counts": applied["counts"],
    });
    if !passed {
        bail!("Migration rehearsal failed: {report}");
    }
    let mut preview = preview;
    preview["expected_cursor"] = json!(cursor);
    Ok((preview, report))
}

fn publish(
    output: &Path,
    target: &Archive,
    bytes: &[u8],
    digest: &str,
    at: i64,
    generated: &mut BTreeMap<&'static str, Vec<u8>>,
) -> Result<Value> {
    let parent = output
        .parent()
        .context("Migration output needs a parent directory")?;
    if !parent.is_dir() {
        bail!(
            "Migration output parent does not exist: {}",
            parent.display()
        );
    }
    let name = output
        .file_name()
        .context("Migration output needs a directory name")?
        .to_string_lossy();
    let staging = parent.join(format!(".{name}.partial-{}", Uuid::new_v4().simple()));
    fs::create_dir(&staging).context("Create the staging directory for the migration output")?;
    let result = (|| -> Result<Value> {
        direct::protect_dir(&staging)?;
        let rehearsal = staging.join("rehearsal");
        fs::create_dir(&rehearsal)?;
        direct::protect_dir(&rehearsal)?;
        let (preview, report) = rehearse(&rehearsal, target, bytes, digest, at)?;
        generated.insert(PREVIEW_FILE, pretty_bytes(&preview)?);
        generated.insert(REHEARSAL_FILE, pretty_bytes(&report)?);
        let digests: BTreeMap<_, _> = generated
            .iter()
            .map(|(file, bytes)| (*file, sha256(bytes)))
            .collect();
        generated.insert(
            MARKER_FILE,
            pretty_bytes(&json!({"format": 1, "mode": "linear_migration_prepare", "generated_files": digests}))?,
        );
        for (file, bytes) in generated.iter() {
            write_new(&staging.join(file), bytes)?;
        }
        Ok(report)
    })();
    match result {
        Ok(report) => {
            if let Err(error) = fs::rename(&staging, output) {
                let _ = fs::remove_dir_all(&staging);
                return Err(error).context("Publish the prepared migration output");
            }
            Ok(report)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            Err(error)
        }
    }
}

/// Existing output must match a fresh deterministic generation exactly.
fn verify(output: &Path, generated: &BTreeMap<&'static str, Vec<u8>>) -> Result<(Value, Value)> {
    let metadata = fs::symlink_metadata(output)?;
    if super::redirected(&metadata) || !metadata.is_dir() {
        bail!("Existing migration output is a symlink, reparse point or non-directory; choose a new output directory");
    }
    let marker: Value = serde_json::from_slice(
        &fs::read(output.join(MARKER_FILE))
            .context("Existing migration output is incomplete (no completion marker); choose a new output directory")?,
    )?;
    for (file, bytes) in generated {
        let existing = fs::read(output.join(file))
            .with_context(|| format!("Existing migration output is missing {file}"))?;
        if existing != *bytes {
            bail!("Existing migration output file {file} differs from this source and target; choose a new output directory");
        }
    }
    for file in [PREVIEW_FILE, REHEARSAL_FILE] {
        let bytes = fs::read(output.join(file))?;
        if marker["generated_files"][file] != json!(sha256(&bytes)) {
            bail!("Existing migration output file {file} was modified");
        }
    }
    Ok((
        serde_json::from_slice(&fs::read(output.join(PREVIEW_FILE))?)?,
        serde_json::from_slice(&fs::read(output.join(REHEARSAL_FILE))?)?,
    ))
}
