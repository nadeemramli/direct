//! Integrity verification for a captured Linear source package.
//!
//! Every file the capture manifest lists is checked, not only the files an
//! importer reads. Paths must be flat, relative, and free of symlinks; the
//! capture must not report errors other than recorded upload download
//! failures, and no nested connection may be truncated.

use super::sha256;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

pub(super) const MANIFEST: &str = "manifest.json";
pub(super) const MANIFEST_CHECKSUM: &str = "manifest.sha256";
pub(super) const UPLOAD_MANIFEST: &str = "attachment-manifest.json";

#[derive(Debug, Deserialize)]
pub(super) struct Manifest {
    pub format: u32,
    pub captured_at: String,
    #[serde(default)]
    pub errors: Vec<Value>,
    #[serde(default)]
    pub missing_coverage: Vec<String>,
    #[serde(default)]
    pub counts: BTreeMap<String, u64>,
    #[serde(default)]
    pub query_coverage: Vec<Value>,
    #[serde(default)]
    pub attachments: Option<Value>,
    pub integrity: ManifestIntegrity,
}

#[derive(Debug, Deserialize)]
pub(super) struct ManifestIntegrity {
    pub data_files: Vec<FileChecksum>,
    #[serde(default)]
    pub attachment_files: Vec<FileChecksum>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(super) struct FileChecksum {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// A package whose manifest, data files, upload manifest and upload bytes all
/// matched their recorded checksums when it was read.
pub(super) struct VerifiedPackage {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub manifest_sha256: String,
    /// Every file that belongs in a retained bundle, keyed by its relative path.
    pub files: BTreeMap<String, FileChecksum>,
    /// Parsed data files keyed by file name (for example `issues.json`).
    pub data: BTreeMap<String, Value>,
    /// Entries of `attachment-manifest.json` in their original order.
    pub uploads: Vec<Value>,
    /// Root entries that are not part of the capture format and are not bundled.
    pub unbundled_root_entries: Vec<String>,
}

impl VerifiedPackage {
    pub fn records(&self, file: &str) -> &[Value] {
        self.data
            .get(file)
            .and_then(|value| value.get("records"))
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice)
    }
}

/// Verify a package and require the named data files to be present.
pub(super) fn verify_package(source: &Path, required: &[&str]) -> Result<VerifiedPackage> {
    let root = source
        .canonicalize()
        .context("Linear source package does not exist")?;
    if !fs::symlink_metadata(&root)?.is_dir() {
        bail!("Linear source package must be a directory");
    }
    let mut missing = Vec::new();
    for file in [MANIFEST, MANIFEST_CHECKSUM, UPLOAD_MANIFEST] {
        if !root.join(file).is_file() {
            missing.push(file.to_owned());
        }
    }
    for file in required {
        if !root.join("data").join(file).is_file() {
            missing.push(format!("data/{file}"));
        }
    }
    if !missing.is_empty() {
        bail!(
            "Linear source package is missing required files: {}",
            missing.join(", ")
        );
    }

    let manifest_bytes = read_regular(&root, MANIFEST)?;
    let checksum = String::from_utf8(read_regular(&root, MANIFEST_CHECKSUM)?)
        .context("manifest.sha256 is not UTF-8")?;
    let expected = checksum
        .split_whitespace()
        .next()
        .context("manifest.sha256 is empty")?;
    let manifest_sha256 = sha256(&manifest_bytes);
    if !expected.eq_ignore_ascii_case(&manifest_sha256) {
        bail!("Linear source manifest checksum does not match");
    }
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).context("Linear source manifest is malformed")?;
    if manifest.format != 1 {
        bail!(
            "Unsupported Linear source package format {}",
            manifest.format
        );
    }

    let upload_manifest_bytes = read_regular(&root, UPLOAD_MANIFEST)?;
    let uploads: Vec<Value> = serde_json::from_slice(&upload_manifest_bytes)
        .context("attachment-manifest.json must be a JSON array")?;
    let failed_upload_scopes: BTreeSet<String> = uploads
        .iter()
        .filter(|upload| upload.get("status").and_then(Value::as_str) != Some("downloaded"))
        .filter_map(|upload| upload.get("canonical_url").and_then(Value::as_str))
        .map(|url| format!("attachment {url}"))
        .collect();
    let blocking_errors: Vec<String> = manifest
        .errors
        .iter()
        .map(|error| {
            error
                .get("scope")
                .and_then(Value::as_str)
                .unwrap_or("unscoped capture error")
                .to_owned()
        })
        .filter(|scope| !failed_upload_scopes.contains(scope))
        .collect();
    if !blocking_errors.is_empty() {
        bail!(
            "Linear capture recorded errors, so the package is incomplete: {}",
            blocking_errors.join("; ")
        );
    }
    for coverage in &manifest.query_coverage {
        let root_name = coverage
            .get("root")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        for field in ["failed_nested_connections", "truncated_nested_connections"] {
            if coverage
                .get(field)
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty())
            {
                bail!("Linear capture of {root_name} has {field}; recapture before importing");
            }
        }
    }

    let mut files = BTreeMap::new();
    for (relative, checksum) in [
        (MANIFEST, manifest_sha256.clone()),
        (UPLOAD_MANIFEST, sha256(&upload_manifest_bytes)),
    ] {
        let bytes = if relative == MANIFEST {
            manifest_bytes.len()
        } else {
            upload_manifest_bytes.len()
        };
        files.insert(
            relative.to_owned(),
            FileChecksum {
                path: relative.to_owned(),
                sha256: checksum,
                bytes: bytes as u64,
            },
        );
    }
    let checksum_bytes = checksum.as_bytes();
    files.insert(
        MANIFEST_CHECKSUM.to_owned(),
        FileChecksum {
            path: MANIFEST_CHECKSUM.to_owned(),
            sha256: sha256(checksum_bytes),
            bytes: checksum_bytes.len() as u64,
        },
    );

    let mut data = BTreeMap::new();
    for entry in &manifest.integrity.data_files {
        let name = safe_relative(&entry.path, "data")?;
        if data.contains_key(&name) {
            bail!("Manifest lists {} more than once", entry.path);
        }
        let bytes = verified_bytes(&root, entry)?;
        let value: Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("Captured data file {} is not valid JSON", entry.path))?;
        check_records(&name, &value)?;
        files.insert(entry.path.clone(), entry.clone());
        data.insert(name, value);
    }
    for file in required {
        if !data.contains_key(*file) {
            bail!("Manifest has no checksum for data/{file}");
        }
    }
    reject_unlisted(&root, "data", &files)?;

    for (root_name, count) in &manifest.counts {
        let file = format!("{}.json", kebab(root_name));
        let value = data
            .get(&file)
            .with_context(|| format!("Manifest counts {root_name} but data/{file} is absent"))?;
        let captured = value
            .get("records")
            .and_then(Value::as_array)
            .map_or(0, Vec::len) as u64;
        if captured != *count {
            bail!("Manifest counts {count} {root_name} but data/{file} holds {captured}");
        }
    }

    let mut uploads_by_path: BTreeMap<String, FileChecksum> = BTreeMap::new();
    for entry in &manifest.integrity.attachment_files {
        safe_relative(&entry.path, "attachments")?;
        if let Some(previous) = uploads_by_path.get(&entry.path) {
            if previous != entry {
                bail!("Manifest lists {} with conflicting checksums", entry.path);
            }
            continue;
        }
        verified_bytes(&root, entry)?;
        uploads_by_path.insert(entry.path.clone(), entry.clone());
        files.insert(entry.path.clone(), entry.clone());
    }
    if root.join("attachments").exists() || !uploads_by_path.is_empty() {
        reject_unlisted(&root, "attachments", &files)?;
    }

    let mut downloaded = Vec::new();
    for (index, upload) in uploads.iter().enumerate() {
        if upload.get("status").and_then(Value::as_str) != Some("downloaded") {
            continue;
        }
        let path = upload
            .get("path")
            .and_then(Value::as_str)
            .with_context(|| format!("Downloaded upload {index} has no path"))?;
        let recorded = FileChecksum {
            path: path.to_owned(),
            sha256: upload
                .get("sha256")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            bytes: upload
                .get("bytes")
                .and_then(Value::as_u64)
                .unwrap_or(u64::MAX),
        };
        if uploads_by_path.get(path) != Some(&recorded) {
            bail!("Downloaded upload {index} ({path}) does not match the manifest integrity list");
        }
        downloaded.push(recorded);
    }
    let mut listed = manifest.integrity.attachment_files.clone();
    listed.sort_by(|left, right| left.path.cmp(&right.path));
    downloaded.sort_by(|left, right| left.path.cmp(&right.path));
    if listed != downloaded {
        bail!("attachment-manifest.json and the manifest integrity list disagree");
    }
    if let Some(summary) = &manifest.attachments {
        let expect = |field: &str, actual: usize| -> Result<()> {
            if let Some(recorded) = summary.get(field).and_then(Value::as_u64) {
                if recorded != actual as u64 {
                    bail!("Manifest records {recorded} {field} but attachment-manifest.json has {actual}");
                }
            }
            Ok(())
        };
        expect("discovered_upload_urls", uploads.len())?;
        expect("downloaded_and_checksummed", downloaded.len())?;
        expect("inaccessible_or_failed", uploads.len() - downloaded.len())?;
    }

    let known_root: BTreeSet<&str> = [
        MANIFEST,
        MANIFEST_CHECKSUM,
        UPLOAD_MANIFEST,
        "data",
        "attachments",
    ]
    .into();
    let mut unbundled_root_entries = Vec::new();
    for entry in fs::read_dir(&root)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if !known_root.contains(name.as_str()) {
            unbundled_root_entries.push(name);
        }
    }
    unbundled_root_entries.sort();

    Ok(VerifiedPackage {
        root,
        manifest,
        manifest_sha256,
        files,
        data,
        uploads,
        unbundled_root_entries,
    })
}

/// Read one bundled file again and confirm it still matches its checksum.
pub(super) fn reread(root: &Path, entry: &FileChecksum) -> Result<Vec<u8>> {
    verified_bytes(root, entry)
}

/// Accept only `<top>/<name>` with a plain file name, returning the name.
pub(super) fn safe_relative(path: &str, top: &str) -> Result<String> {
    let unsafe_path = || anyhow::anyhow!("Unsafe path in Linear manifest: {path:?}");
    if path.contains(['\\', ':', '\0']) || path.chars().any(char::is_control) {
        return Err(unsafe_path());
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() != 2 || parts[0] != top || parts[1].is_empty() || parts[1].starts_with('.') {
        return Err(unsafe_path());
    }
    if !Path::new(path)
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(unsafe_path());
    }
    Ok(parts[1].to_owned())
}

fn read_regular(root: &Path, relative: &str) -> Result<Vec<u8>> {
    let path = root.join(relative);
    let mut current = root.to_path_buf();
    for part in relative.split('/') {
        current.push(part);
        let metadata = fs::symlink_metadata(&current)
            .with_context(|| format!("Linear source file {relative} is missing"))?;
        if metadata.file_type().is_symlink() {
            bail!("Linear source package contains a symlink at {relative}");
        }
    }
    if !fs::symlink_metadata(&path)?.is_file() {
        bail!("Linear source entry {relative} is not a regular file");
    }
    if !path.canonicalize()?.starts_with(root) {
        bail!("Linear source file {relative} resolves outside the package");
    }
    fs::read(&path).with_context(|| format!("Read Linear source file {relative}"))
}

fn verified_bytes(root: &Path, entry: &FileChecksum) -> Result<Vec<u8>> {
    let bytes = read_regular(root, &entry.path)?;
    if bytes.len() as u64 != entry.bytes || !entry.sha256.eq_ignore_ascii_case(&sha256(&bytes)) {
        bail!("Linear source integrity check failed for {}", entry.path);
    }
    Ok(bytes)
}

fn reject_unlisted(root: &Path, top: &str, files: &BTreeMap<String, FileChecksum>) -> Result<()> {
    let directory = root.join(top);
    let metadata = fs::symlink_metadata(&directory)
        .with_context(|| format!("Linear source directory {top}/ is missing"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("Linear source entry {top}/ must be a real directory");
    }
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let relative = format!("{top}/{}", entry.file_name().to_string_lossy());
        if !files.contains_key(&relative) {
            bail!("Linear source package contains an unlisted file {relative}");
        }
    }
    Ok(())
}

fn check_records(name: &str, value: &Value) -> Result<()> {
    // The schema introspection payload is metadata, not an entity envelope.
    if name == "graphql-schema.json" && value.get("records").is_none() {
        if !value.is_object() {
            bail!("data/{name} metadata must be an object");
        }
        return Ok(());
    }
    let Some(records) = value.get("records") else {
        bail!("data/{name} must contain a records array");
    };
    let records = records
        .as_array()
        .with_context(|| format!("data/{name} records must be an array"))?;
    if let Some(count) = value.get("count") {
        let count = count
            .as_u64()
            .with_context(|| format!("data/{name} count must be a non-negative integer"))?;
        if count != records.len() as u64 {
            bail!(
                "data/{name} declares {count} records but holds {}",
                records.len()
            );
        }
    }
    for (index, record) in records.iter().enumerate() {
        if !record.is_object() {
            bail!("data/{name} record {index} must be an object");
        }
        if let Some(path) = truncated_connection(record, String::new()) {
            bail!("data/{name} record {index} has a truncated nested connection at {path}; recapture before importing");
        }
    }
    Ok(())
}

fn truncated_connection(value: &Value, path: String) -> Option<String> {
    match value {
        Value::Object(map) => {
            if map
                .get("pageInfo")
                .and_then(|info| info.get("hasNextPage"))
                .and_then(Value::as_bool)
                == Some(true)
            {
                return Some(if path.is_empty() { "/".into() } else { path });
            }
            map.iter()
                .find_map(|(key, child)| truncated_connection(child, format!("{path}/{key}")))
        }
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(index, child)| truncated_connection(child, format!("{path}/{index}"))),
        _ => None,
    }
}

/// Mirror the capture script's root-to-file naming (`workflowStates` → `workflow-states`).
pub(super) fn kebab(value: &str) -> String {
    let mut output = String::new();
    let mut previous_lower_or_digit = false;
    for character in value.chars() {
        if character.is_ascii_uppercase() && previous_lower_or_digit {
            output.push('-');
        }
        previous_lower_or_digit = character.is_ascii_lowercase() || character.is_ascii_digit();
        output.push(character.to_ascii_lowercase());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_paths_must_be_flat_and_relative() {
        assert_eq!(
            safe_relative("data/issues.json", "data").unwrap(),
            "issues.json"
        );
        for path in [
            "data/../secret.json",
            "../data/issues.json",
            "/data/issues.json",
            "data\\issues.json",
            "data/sub/issues.json",
            "data/.hidden",
            "C:/data/issues.json",
            "attachments/file.png",
            "data/",
        ] {
            assert!(safe_relative(path, "data").is_err(), "{path} was accepted");
        }
    }

    #[test]
    fn kebab_matches_capture_file_names() {
        assert_eq!(kebab("workflowStates"), "workflow-states");
        assert_eq!(kebab("issueRelations"), "issue-relations");
        assert_eq!(kebab("teams"), "teams");
    }
}
