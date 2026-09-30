//! Merge a prepared migration artifact into an EXISTING workspace.
//!
//! The artifact is binary: a magic line, a fixed-width header length, a JSON
//! header (entities, retained bundle, file table and record index) and the raw
//! retained file bytes in file-table order. Nothing existing is overwritten:
//! every ID, key, reserved key, name and label taxonomy term is checked for
//! collisions, the merged workspace must pass full archive validation, and the
//! import runs in one transaction guarded by the owner-confirmed event cursor.
//! Exact replay is a no-op; rollback is exact while nothing has changed since.

use super::sources::{
    self, MAX_MIGRATION_ARTIFACT_BYTES, MAX_SOURCE_BUNDLE_BYTES, MAX_SOURCE_FILE_BYTES,
};
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MIGRATION_MAGIC: &[u8] = b"DIRECT-MIGRATION-ARTIFACT/1\n";
const LENGTH_DIGITS: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactFile {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub content_type: String,
    pub role: String,
    #[serde(default)]
    pub original_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationArtifact {
    pub format: u32,
    pub source: String,
    pub target_workspace_id: String,
    /// Event cursor of the target export the artifact was prepared against.
    pub baseline_cursor: u64,
    #[serde(default)]
    pub products: Vec<Product>,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub goals: Vec<Goal>,
    #[serde(default)]
    pub milestones: Vec<Milestone>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub issues: Vec<Issue>,
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub issue_links: Vec<IssueLink>,
    /// Existing products that receive imported work (explicit team mapping).
    #[serde(default)]
    pub reused_products: Vec<String>,
    /// Existing labels whose Linear origin matched an imported assignment.
    #[serde(default)]
    pub reused_labels: Vec<String>,
    pub bundle: SourceBundle,
    pub files: Vec<ArtifactFile>,
    pub records: Vec<SourceRecord>,
}

pub fn encode_migration_artifact(artifact: &MigrationArtifact, payload: &[u8]) -> Result<Vec<u8>> {
    let header = serde_json::to_vec(artifact)?;
    let mut bytes = Vec::with_capacity(
        MIGRATION_MAGIC.len() + LENGTH_DIGITS + 1 + header.len() + payload.len(),
    );
    bytes.extend_from_slice(MIGRATION_MAGIC);
    bytes
        .extend_from_slice(format!("{:0width$}\n", header.len(), width = LENGTH_DIGITS).as_bytes());
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

pub fn parse_migration_artifact(bytes: &[u8]) -> Result<(MigrationArtifact, &[u8])> {
    let invalid = |message: &str| err("invalid", format!("Migration artifact: {message}"));
    if bytes.len() > MAX_MIGRATION_ARTIFACT_BYTES {
        return Err(invalid("exceeds the upload limit"));
    }
    let rest = bytes
        .strip_prefix(MIGRATION_MAGIC)
        .ok_or_else(|| invalid("not a Direct migration artifact"))?;
    if rest.len() < LENGTH_DIGITS + 1 || rest[LENGTH_DIGITS] != b'\n' {
        return Err(invalid("missing header length"));
    }
    let length: usize = std::str::from_utf8(&rest[..LENGTH_DIGITS])
        .ok()
        .filter(|digits| digits.bytes().all(|c| c.is_ascii_digit()))
        .and_then(|digits| digits.parse().ok())
        .ok_or_else(|| invalid("invalid header length"))?;
    let rest = &rest[LENGTH_DIGITS + 1..];
    if length > rest.len() {
        return Err(invalid("header is truncated"));
    }
    let artifact: MigrationArtifact = serde_json::from_slice(&rest[..length])
        .map_err(|e| invalid(&format!("malformed header ({e})")))?;
    if artifact.format != 1 {
        return Err(invalid("unsupported format"));
    }
    Ok((artifact, &rest[length..]))
}

pub fn artifact_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

enum Plan {
    New,
    AlreadyApplied,
}

struct Checked<'a> {
    artifact: MigrationArtifact,
    files: Vec<(SourceFile, &'a [u8])>,
    sha: String,
    plan: Plan,
}

fn entities(artifact: &MigrationArtifact) -> MigrationEntities {
    MigrationEntities {
        products: artifact.products.iter().map(|p| p.id.clone()).collect(),
        projects: artifact.projects.iter().map(|p| p.id.clone()).collect(),
        goals: artifact.goals.iter().map(|g| g.id.clone()).collect(),
        milestones: artifact.milestones.iter().map(|m| m.id.clone()).collect(),
        labels: artifact.labels.iter().map(|l| l.id.clone()).collect(),
        issues: artifact.issues.iter().map(|i| i.id.clone()).collect(),
        comments: artifact.comments.iter().map(|c| c.id.clone()).collect(),
        issue_links: artifact.issue_links.iter().map(|l| l.id.clone()).collect(),
    }
}

fn same<T: Serialize>(left: &T, right: &T) -> Result<bool> {
    Ok(serde_json::to_value(left)? == serde_json::to_value(right)?)
}

/// Compare every imported record with what the workspace holds now.
fn changed_since_apply(current: &Archive, artifact: &MigrationArtifact) -> Result<Vec<String>> {
    let mut changed = Vec::new();
    macro_rules! compare {
        ($field:ident, $label:literal, $id:ident) => {
            for incoming in &artifact.$field {
                match current
                    .$field
                    .iter()
                    .find(|existing| existing.$id == incoming.$id)
                {
                    Some(existing) if same(existing, incoming)? => {}
                    Some(_) => changed.push(format!(
                        "{} {} was edited after import",
                        $label, incoming.$id
                    )),
                    None => changed.push(format!(
                        "{} {} was removed after import",
                        $label, incoming.$id
                    )),
                }
            }
        };
    }
    compare!(products, "product", id);
    compare!(projects, "project", id);
    compare!(goals, "goal", id);
    compare!(milestones, "milestone", id);
    compare!(labels, "label", id);
    compare!(issues, "issue", id);
    compare!(comments, "comment", id);
    compare!(issue_links, "issue link", id);
    Ok(changed)
}

fn check<'a>(conn: &Connection, bytes: &'a [u8]) -> Result<Checked<'a>> {
    let sha = artifact_sha256(bytes);
    let (artifact, payload) = parse_migration_artifact(bytes)?;
    let workspace: String =
        conn.query_row("SELECT value FROM meta WHERE key='workspace_id'", [], |r| {
            r.get(0)
        })?;
    if artifact.target_workspace_id != workspace {
        return Err(err(
            "conflict",
            format!(
                "Artifact was prepared for workspace {}, but this workspace is {workspace}. Prepare it from this workspace's export.",
                artifact.target_workspace_id
            ),
        ));
    }
    // Payload: file table order, exact total, bounded sizes.
    let mut offset = 0usize;
    let mut files = Vec::with_capacity(artifact.files.len());
    let mut total = 0u64;
    for entry in &artifact.files {
        if entry.bytes > MAX_SOURCE_FILE_BYTES {
            return Err(err(
                "invalid",
                format!("Retained file {} exceeds the per-file limit", entry.path),
            ));
        }
        total += entry.bytes;
        let end = offset
            .checked_add(entry.bytes as usize)
            .filter(|end| *end <= payload.len())
            .ok_or_else(|| err("invalid", "Migration artifact payload is truncated"))?;
        let file = SourceFile {
            bundle_id: artifact.bundle.id.clone(),
            path: entry.path.clone(),
            sha256: entry.sha256.clone(),
            bytes: entry.bytes,
            content_type: entry.content_type.clone(),
            role: entry.role.clone(),
            original_name: entry.original_name.clone(),
            data: None,
        };
        files.push((file, &payload[offset..end]));
        offset = end;
    }
    if offset != payload.len() {
        return Err(err(
            "invalid",
            "Migration artifact has unexpected trailing data",
        ));
    }
    if total > MAX_SOURCE_BUNDLE_BYTES {
        return Err(err("invalid", "Retained bundle exceeds the bundle limit"));
    }
    if artifact
        .records
        .iter()
        .any(|record| record.bundle_id != artifact.bundle.id)
    {
        return Err(err(
            "invalid",
            "Retained records must belong to the artifact bundle",
        ));
    }
    sources::validate_source_payload(
        files.iter().map(|(file, bytes)| (file, *bytes)),
        &artifact.records,
    )?;

    let mut current = Store::export_from(conn, false)?;
    if let Some(existing) = current
        .source_bundles
        .iter()
        .find(|b| b.id == artifact.bundle.id)
    {
        let same_artifact = existing
            .application
            .as_ref()
            .is_some_and(|application| application.artifact_sha256 == sha);
        if !same_artifact {
            return Err(err(
                "conflict",
                "This source bundle is already in the workspace from a different artifact or restore. A changed source is not merged: roll back or restore the pre-import backup first.",
            ));
        }
        let changed = changed_since_apply(&current, &artifact)?;
        if !changed.is_empty() {
            return Err(err(
                "conflict",
                format!(
                    "This artifact was applied, but {} imported record(s) changed since, so it is not re-applied: {}",
                    changed.len(),
                    changed.iter().take(20).cloned().collect::<Vec<_>>().join("; ")
                ),
            ));
        }
        return Ok(Checked {
            artifact,
            files,
            sha,
            plan: Plan::AlreadyApplied,
        });
    }
    if current.source_bundles.iter().any(|bundle| {
        bundle
            .manifest_sha256
            .eq_ignore_ascii_case(&artifact.bundle.manifest_sha256)
    }) {
        return Err(err(
            "conflict",
            "Another bundle already retains this source manifest",
        ));
    }

    let mut conflicts = Vec::new();
    let reserved: HashSet<String> = events(conn, 0)?
        .into_iter()
        .map(|event| event.entity)
        .collect();
    let new_products: HashSet<&str> = artifact.products.iter().map(|p| p.id.as_str()).collect();
    for product in &artifact.products {
        if current.products.iter().any(|p| p.id == product.id) {
            conflicts.push(format!("product id {} already exists", product.id));
        }
        if current
            .products
            .iter()
            .any(|p| p.key.eq_ignore_ascii_case(&product.key))
        {
            conflicts.push(format!(
                "product key {} already exists; map the Linear team to that product explicitly or rename it",
                product.key
            ));
        }
    }
    for id in &artifact.reused_products {
        if new_products.contains(id.as_str()) || !current.products.iter().any(|p| p.id == *id) {
            conflicts.push(format!("mapped product {id} is not an existing product"));
        }
    }
    for project in &artifact.projects {
        if current.projects.iter().any(|p| p.id == project.id) {
            conflicts.push(format!("project id {} already exists", project.id));
        }
        if current.projects.iter().any(|p| {
            p.product_id == project.product_id
                && p.name.trim().to_lowercase() == project.name.trim().to_lowercase()
        }) {
            conflicts.push(format!(
                "project name {:?} already exists in its product",
                project.name
            ));
        }
    }
    for goal in &artifact.goals {
        if current.goals.iter().any(|g| g.id == goal.id) {
            conflicts.push(format!("goal id {} already exists", goal.id));
        }
        if current.goals.iter().any(|g| {
            g.product_id == goal.product_id
                && g.name.trim().to_lowercase() == goal.name.trim().to_lowercase()
        }) {
            conflicts.push(format!(
                "goal name {:?} already exists in its product",
                goal.name
            ));
        }
    }
    for milestone in &artifact.milestones {
        if current.milestones.iter().any(|m| m.id == milestone.id) {
            conflicts.push(format!("milestone id {} already exists", milestone.id));
        }
    }
    for label in &artifact.labels {
        if current.labels.iter().any(|l| l.id == label.id) {
            conflicts.push(format!("label id {} already exists", label.id));
        } else if let Err(error) = label_taxonomy_conflicts(
            &current.labels,
            &label.name,
            &label.aliases,
            &label.linear_origins,
            None,
        ) {
            conflicts.push(format!("label {:?}: {}", label.name, error.message));
        }
    }
    for id in &artifact.reused_labels {
        if !current.labels.iter().any(|l| l.id == *id)
            || artifact.labels.iter().any(|l| l.id == *id)
        {
            conflicts.push(format!("reused label {id} is not an existing label"));
        }
    }
    let imported_keys: HashSet<&str> = artifact.issues.iter().map(|i| i.key.as_str()).collect();
    for issue in &artifact.issues {
        if current.issues.iter().any(|i| i.id == issue.id) {
            conflicts.push(format!("issue id {} already exists", issue.id));
        }
        if current.issues.iter().any(|i| i.key == issue.key) {
            conflicts.push(format!("issue key {} already exists", issue.key));
        } else if reserved.contains(&issue.key) {
            conflicts.push(format!(
                "issue key {} is reserved by activity history",
                issue.key
            ));
        }
        if !new_products.contains(issue.product_id.as_str())
            && !artifact.reused_products.contains(&issue.product_id)
        {
            conflicts.push(format!(
                "issue {} targets a product outside the artifact mapping",
                issue.key
            ));
        }
        if issue.claim.is_some()
            || issue.current_run.is_some()
            || issue.verification_key.is_some()
            || issue.parent.is_some()
            || issue.needs_fix
            || matches!(
                issue.status,
                Status::Ready | Status::Doing | Status::Verify | Status::Done
            )
            || issue.external.is_none()
        {
            conflicts.push(format!(
                "issue {} carries readiness, claims, verification or acceptance, or lacks source provenance; imports cannot create those",
                issue.key
            ));
        }
    }
    for comment in &artifact.comments {
        if current.comments.iter().any(|c| c.id == comment.id) {
            conflicts.push(format!("comment id {} already exists", comment.id));
        }
        if !imported_keys.contains(comment.issue_key.as_str()) || comment.external_source.is_none()
        {
            conflicts.push(format!(
                "comment {} must belong to an imported issue with source provenance",
                comment.id
            ));
        }
    }
    for link in &artifact.issue_links {
        if current.issue_links.iter().any(|l| l.id == link.id) {
            conflicts.push(format!("issue link id {} already exists", link.id));
        }
        if !imported_keys.contains(link.source_key.as_str())
            || !imported_keys.contains(link.target_key.as_str())
        {
            conflicts.push(format!("issue link {} must join imported issues", link.id));
        }
    }
    for record in &artifact.records {
        if record
            .issue_keys
            .iter()
            .any(|key| !imported_keys.contains(key.as_str()))
        {
            conflicts.push(format!(
                "retained record {} links an issue outside the import",
                record.id
            ));
        }
    }
    if !conflicts.is_empty() {
        let total = conflicts.len();
        conflicts.truncate(25);
        return Err(err(
            "conflict",
            format!(
                "Migration refused: {total} collision(s) with existing records; nothing was changed. {}",
                conflicts.join("; ")
            ),
        ));
    }

    // The merged workspace must satisfy every archive invariant.
    current.format = 12;
    current.products.extend(artifact.products.iter().cloned());
    current.projects.extend(artifact.projects.iter().cloned());
    current.goals.extend(artifact.goals.iter().cloned());
    current
        .milestones
        .extend(artifact.milestones.iter().cloned());
    current.labels.extend(artifact.labels.iter().cloned());
    current.issues.extend(artifact.issues.iter().cloned());
    current.comments.extend(artifact.comments.iter().cloned());
    current
        .issue_links
        .extend(artifact.issue_links.iter().cloned());
    let mut bundle = artifact.bundle.clone();
    bundle.imported_by = "migration".into();
    bundle.application = None;
    if bundle.file_count != files.len() as u64
        || bundle.total_bytes != total
        || bundle.record_count != artifact.records.len() as u64
    {
        return Err(err(
            "invalid",
            "Artifact bundle totals disagree with its files or records",
        ));
    }
    current.source_bundles.push(bundle);
    current
        .source_files
        .extend(files.iter().map(|(file, _)| file.clone()));
    current
        .source_records
        .extend(artifact.records.iter().cloned());
    validate_archive_structure(&current).map_err(|error| {
        err(
            "invalid",
            format!("Merged workspace would be invalid: {}", error.message),
        )
    })?;
    Ok(Checked {
        artifact,
        files,
        sha,
        plan: Plan::New,
    })
}

fn counts(artifact: &MigrationArtifact, files: &[(SourceFile, &[u8])]) -> Value {
    let mut access: BTreeMap<&str, u64> = BTreeMap::new();
    for record in &artifact.records {
        *access.entry(record.access.as_str()).or_default() += 1;
    }
    json!({
        "products": artifact.products.len(),
        "reused_products": artifact.reused_products.len(),
        "projects": artifact.projects.len(),
        "goals": artifact.goals.len(),
        "milestones": artifact.milestones.len(),
        "labels": artifact.labels.len(),
        "reused_labels": artifact.reused_labels.len(),
        "issues": artifact.issues.len(),
        "legacy_completed": artifact.issues.iter().filter(|i| i.status == Status::LegacyCompleted).count(),
        "comments": artifact.comments.len(),
        "issue_links": artifact.issue_links.len(),
        "retained_files": files.len(),
        "retained_bytes": files.iter().map(|(file, _)| file.bytes).sum::<u64>(),
        "retained_records": artifact.records.len(),
        "retained_records_by_access": access,
    })
}

impl Store {
    /// Validate an artifact against this workspace without changing anything.
    pub fn preview_migration(&self, bytes: &[u8]) -> Result<Value> {
        let checked = check(&self.conn, bytes)?;
        Ok(json!({
            "status": match checked.plan { Plan::New => "ready_to_apply", Plan::AlreadyApplied => "already_applied" },
            "artifact_sha256": checked.sha,
            "workspace_id": checked.artifact.target_workspace_id,
            "prepared_baseline_cursor": checked.artifact.baseline_cursor,
            "expected_cursor": cursor(&self.conn)?,
            "workspace_changed_since_preparation": cursor(&self.conn)? != checked.artifact.baseline_cursor,
            "bundle": {
                "id": checked.artifact.bundle.id,
                "source": checked.artifact.bundle.source,
                "captured_at": checked.artifact.bundle.captured_at,
                "manifest_sha256": checked.artifact.bundle.manifest_sha256,
            },
            "counts": counts(&checked.artifact, &checked.files),
            "summary": checked.artifact.bundle.summary,
        }))
    }

    /// Apply an artifact in one transaction. `expected_cursor` and
    /// `expected_sha256` must match what the owner previewed.
    pub fn apply_migration(
        &mut self,
        bytes: &[u8],
        expected_cursor: u64,
        expected_sha256: &str,
        actor: &str,
        backup: Option<String>,
        at: i64,
    ) -> Result<Value> {
        required(actor, "actor")?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current = cursor(&tx)?;
        if current != expected_cursor {
            return Err(err(
                "conflict",
                format!("Workspace changed since preview (cursor {current}, expected {expected_cursor}). Preview again."),
            ));
        }
        let checked = check(&tx, bytes)?;
        if !checked.sha.eq_ignore_ascii_case(expected_sha256) {
            return Err(err(
                "conflict",
                "Artifact differs from the previewed artifact",
            ));
        }
        let artifact = &checked.artifact;
        if let Plan::AlreadyApplied = checked.plan {
            return Ok(json!({
                "status": "already_applied",
                "duplicates_added": 0,
                "bundle_id": artifact.bundle.id,
                "artifact_sha256": checked.sha,
                "cursor": current,
            }));
        }
        for product in &artifact.products {
            tx.execute(
                "INSERT INTO products VALUES (?1,?2,?3)",
                params![product.id, product.key, serde_json::to_string(product)?],
            )?;
        }
        for project in &artifact.projects {
            put_project(&tx, project)?;
        }
        for goal in &artifact.goals {
            put_goal(&tx, goal)?;
        }
        for milestone in &artifact.milestones {
            put_milestone(&tx, milestone)?;
        }
        for label in &artifact.labels {
            put_label(&tx, label)?;
        }
        for issue in &artifact.issues {
            put_issue(&tx, issue)?;
        }
        for comment in &artifact.comments {
            tx.execute(
                "INSERT INTO comments VALUES (?1,?2)",
                params![comment.id, serde_json::to_string(comment)?],
            )?;
        }
        for link in &artifact.issue_links {
            put_issue_link(&tx, link)?;
        }
        for (file, payload) in &checked.files {
            sources::put_file(&tx, file, payload)?;
        }
        for record in &artifact.records {
            sources::put_record(&tx, record)?;
        }
        let mut keys: Vec<&str> = artifact.issues.iter().map(|i| i.key.as_str()).collect();
        keys.sort();
        for key in keys {
            emit(&tx, actor, "migration_imported", key, at)?;
        }
        emit(&tx, actor, "migration_applied", &artifact.bundle.id, at)?;
        let applied_cursor = cursor(&tx)?;
        let mut bundle = artifact.bundle.clone();
        bundle.imported_by = actor.into();
        bundle.imported_at = at;
        bundle.application = Some(MigrationApplication {
            artifact_sha256: checked.sha.clone(),
            baseline_cursor: current,
            applied_cursor,
            backup: backup.clone(),
            entities: entities(artifact),
        });
        sources::put_bundle(&tx, &bundle)?;
        let result = json!({
            "status": "applied",
            "duplicates_added": 0,
            "bundle_id": bundle.id,
            "artifact_sha256": checked.sha,
            "baseline_cursor": current,
            "applied_cursor": applied_cursor,
            "backup": backup,
            "counts": counts(artifact, &checked.files),
        });
        tx.commit()?;
        Ok(result)
    }
}

/// Owner-only rollback, exact while nothing has changed since the import.
pub(crate) fn rollback(
    tx: &Transaction,
    bundle_id: &str,
    expected_cursor: u64,
    actor: &str,
    at: i64,
) -> Result<Value> {
    let bundle = sources::bundles(tx)?
        .into_iter()
        .find(|bundle| bundle.id == bundle_id)
        .ok_or_else(|| err("not_found", "Unknown retained source bundle"))?;
    let application = bundle.application.clone().ok_or_else(|| {
        err(
            "conflict",
            "This bundle was not applied as a migration and cannot be rolled back",
        )
    })?;
    let current = cursor(tx)?;
    if current != expected_cursor {
        return Err(err(
            "conflict",
            format!(
                "Workspace cursor is {current}, not {expected_cursor}. Reload before rolling back."
            ),
        ));
    }
    if current != application.applied_cursor {
        return Err(err(
            "conflict",
            "The workspace changed after this import, so an exact rollback is not possible. Restore the pre-import backup into a new data directory instead (see docs/linear-import.md).",
        ));
    }
    let entities = &application.entities;
    for id in &entities.issue_links {
        tx.execute("DELETE FROM issue_links WHERE id=?1", [id])?;
    }
    for id in &entities.comments {
        tx.execute("DELETE FROM comments WHERE id=?1", [id])?;
    }
    for id in &entities.issues {
        tx.execute("DELETE FROM issues WHERE id=?1", [id])?;
    }
    for id in &entities.labels {
        tx.execute("DELETE FROM labels WHERE id=?1", [id])?;
    }
    for id in &entities.milestones {
        tx.execute("DELETE FROM milestones WHERE id=?1", [id])?;
    }
    for id in &entities.goals {
        tx.execute("DELETE FROM goals WHERE id=?1", [id])?;
    }
    for id in &entities.projects {
        tx.execute("DELETE FROM projects WHERE id=?1", [id])?;
    }
    for id in &entities.products {
        tx.execute("DELETE FROM products WHERE id=?1", [id])?;
    }
    sources::delete_bundle(tx, bundle_id)?;
    tx.execute(
        "DELETE FROM events WHERE seq > ?1",
        [application.baseline_cursor as i64],
    )?;
    emit(tx, actor, "migration_rolled_back", bundle_id, at)?;
    Ok(json!({
        "status": "rolled_back",
        "bundle_id": bundle_id,
        "restored_cursor": application.baseline_cursor,
        "removed": {
            "products": entities.products.len(),
            "projects": entities.projects.len(),
            "goals": entities.goals.len(),
            "milestones": entities.milestones.len(),
            "labels": entities.labels.len(),
            "issues": entities.issues.len(),
            "comments": entities.comments.len(),
            "issue_links": entities.issue_links.len(),
        },
    }))
}
