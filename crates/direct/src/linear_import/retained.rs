//! Build the retained-source bundle for Direct: every verified capture file
//! (exact bytes) and an index entry per accounted source record that points at
//! the record's exact byte span in its original file. Nothing is rewritten, so
//! Direct can show the original values and export them byte for byte.

use super::{
    deterministic_uuid, nested_string, optional_string,
    package::{reread, VerifiedPackage, UPLOAD_MANIFEST},
    workspace::{Class, Entry, BUNDLE_DIR},
};
use anyhow::{bail, Context, Result};
use direct_core::{
    SourceBundle, SourceFile, SourceRecord, MAX_SOURCE_BUNDLE_BYTES, MAX_SOURCE_FILE_BYTES,
};
use serde::Deserialize;
use serde_json::{value::RawValue, Value};
use std::collections::{BTreeSet, HashMap, HashSet};

pub(super) struct Retained {
    pub bundle: SourceBundle,
    pub files: Vec<(SourceFile, Vec<u8>)>,
    pub records: Vec<SourceRecord>,
}

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    records: Vec<&'a RawValue>,
}

fn spans_of(bytes: &[u8], items: &[&RawValue]) -> Vec<(u64, u64)> {
    let base = bytes.as_ptr() as usize;
    items
        .iter()
        .map(|item| {
            let start = item.get().as_ptr() as usize - base;
            (start as u64, (start + item.get().len()) as u64)
        })
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

fn content_type(path: &str, upload: Option<&Value>) -> String {
    if path.ends_with(".json") {
        return "application/json".into();
    }
    if path.ends_with(".sha256") {
        return "text/plain".into();
    }
    upload
        .and_then(|upload| upload.get("content_type"))
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 100
                && value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"!#$&^_.+-/;= ".contains(&c))
        })
        .unwrap_or("application/octet-stream")
        .into()
}

const SEARCH_FIELDS: &[&str] = &[
    "identifier",
    "title",
    "name",
    "key",
    "url",
    "description",
    "body",
    "content",
    "original_name",
    "canonical_url",
];

fn search_text(entry: &Entry, value: Option<&Value>) -> String {
    let mut text = String::new();
    for part in [entry.source_label.as_deref(), entry.source_title.as_deref()]
        .into_iter()
        .flatten()
    {
        text.push_str(part);
        text.push('\n');
    }
    if let Some(value) = value {
        for field in SEARCH_FIELDS {
            if let Some(part) = value.get(*field).and_then(Value::as_str) {
                text.push_str(part);
                text.push('\n');
            }
        }
        for previous in value
            .get("previousIdentifiers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            text.push_str(previous);
            text.push('\n');
        }
    }
    truncate(&text, 4_000)
}

/// `/records/12/labels/nodes/0` → 12; `/3` → 3.
fn record_index(pointer: &str) -> Option<usize> {
    let mut parts = pointer.trim_start_matches('/').split('/');
    let first = parts.next()?;
    if first == "records" {
        parts.next()?.parse().ok()
    } else {
        first.parse().ok()
    }
}

pub(super) fn build(
    package: &VerifiedPackage,
    entries: &[Entry],
    issue_keys: &HashMap<String, String>,
    summary: Value,
    imported_by: &str,
    imported_at: i64,
) -> Result<Retained> {
    let bundle_id = deterministic_uuid("linear-source-bundle", &package.manifest_sha256);
    let upload_by_path: HashMap<&str, &Value> = package
        .uploads
        .iter()
        .filter_map(|upload| Some((upload.get("path")?.as_str()?, upload)))
        .collect();

    let mut files = Vec::new();
    let mut spans: HashMap<String, Vec<(u64, u64)>> = HashMap::new();
    let mut total = 0u64;
    for (path, checksum) in &package.files {
        if checksum.bytes > MAX_SOURCE_FILE_BYTES {
            bail!("Captured file {path} exceeds Direct's retained file limit of {MAX_SOURCE_FILE_BYTES} bytes");
        }
        total += checksum.bytes;
        let bytes = reread(&package.root, checksum)?;
        if path.starts_with("data/") {
            if let Ok(envelope) = serde_json::from_slice::<Envelope>(&bytes) {
                let records = spans_of(&bytes, &envelope.records);
                spans.insert(path.clone(), records);
            }
        } else if path == UPLOAD_MANIFEST {
            let items: Vec<&RawValue> = serde_json::from_slice(&bytes)
                .context("attachment-manifest.json must be a JSON array")?;
            spans.insert(path.clone(), spans_of(&bytes, &items));
        }
        let upload = upload_by_path.get(path.as_str()).copied();
        let role = if path.starts_with("data/") {
            "data"
        } else if path.starts_with("attachments/") {
            "upload"
        } else {
            "manifest"
        };
        files.push((
            SourceFile {
                bundle_id: bundle_id.clone(),
                path: path.clone(),
                sha256: checksum.sha256.to_ascii_lowercase(),
                bytes: checksum.bytes,
                content_type: content_type(path, upload),
                role: role.into(),
                original_name: upload
                    .and_then(|upload| optional_string(upload, "original_name"))
                    .map(|name| truncate(&name, 300)),
                data: None,
            },
            bytes,
        ));
    }
    if total > MAX_SOURCE_BUNDLE_BYTES {
        bail!("Captured package ({total} bytes) exceeds Direct's retained bundle limit of {MAX_SOURCE_BUNDLE_BYTES} bytes");
    }
    let file_bytes: HashMap<&str, u64> = files
        .iter()
        .map(|(file, _)| (file.path.as_str(), file.bytes))
        .collect();

    let keys: HashSet<&str> = issue_keys.values().map(String::as_str).collect();
    let key_of = |id: Option<String>| id.and_then(|id| issue_keys.get(&id).cloned());
    let issue_key_at: Vec<Option<String>> = package
        .records("issues.json")
        .iter()
        .map(|issue| key_of(optional_string(issue, "id")))
        .collect();
    let comment_key_at: Vec<Option<String>> = package
        .records("comments.json")
        .iter()
        .map(|comment| {
            key_of(
                nested_string(comment, &["issue", "id"])
                    .or_else(|| optional_string(comment, "issueId")),
            )
        })
        .collect();
    let attachment_key_at: Vec<Option<String>> = package
        .records("attachments.json")
        .iter()
        .map(|attachment| key_of(nested_string(attachment, &["issue", "id"])))
        .collect();

    let mut records = Vec::with_capacity(entries.len());
    let mut ids = HashSet::new();
    for entry in entries {
        let file = entry.file.clone();
        let Some(file_size) = file_bytes.get(file.as_str()).copied() else {
            bail!("Accounting entry points at {file}, which is not a retained file");
        };
        let (start, end, index) = if entry.level == "file" {
            (0, file_size, None)
        } else {
            let index = record_index(&entry.pointer).with_context(|| {
                format!("Accounting pointer {} has no record index", entry.pointer)
            })?;
            let (start, end) = spans
                .get(&file)
                .and_then(|spans| spans.get(index))
                .copied()
                .with_context(|| format!("No byte span for {file} {}", entry.pointer))?;
            (start, end, Some(index))
        };
        if start >= end {
            // An empty file cannot be addressed as a span; it stays downloadable.
            continue;
        }
        let value: Option<&Value> = match (file.as_str(), index) {
            (UPLOAD_MANIFEST, Some(index)) => package.uploads.get(index),
            (path, Some(index)) => path
                .strip_prefix("data/")
                .and_then(|name| package.records(name).get(index)),
            _ => None,
        };
        let mut linked = BTreeSet::new();
        if let Some(index) = index {
            match file.as_str() {
                "data/issues.json" => linked.extend(issue_key_at.get(index).cloned().flatten()),
                "data/comments.json" => linked.extend(comment_key_at.get(index).cloned().flatten()),
                "data/attachments.json" => {
                    linked.extend(attachment_key_at.get(index).cloned().flatten())
                }
                "data/issue-relations.json" => {
                    if let Some(value) = value {
                        linked.extend(key_of(nested_string(value, &["issue", "id"])));
                        linked.extend(key_of(nested_string(value, &["relatedIssue", "id"])));
                    }
                }
                UPLOAD_MANIFEST => {
                    for path in value
                        .and_then(|value| value.get("source_paths"))
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                    {
                        let mut parts = path.split('.');
                        let (Some(group), Some(position)) = (parts.next(), parts.next()) else {
                            continue;
                        };
                        let Ok(position) = position.parse::<usize>() else {
                            continue;
                        };
                        let key = match group {
                            "issues" => issue_key_at.get(position).cloned().flatten(),
                            "comments" => comment_key_at.get(position).cloned().flatten(),
                            "attachments" => attachment_key_at.get(position).cloned().flatten(),
                            _ => None,
                        };
                        linked.extend(key);
                    }
                }
                _ => {}
            }
        }
        for field in ["source", "target", "issue", "key"] {
            if let Some(key) = entry.direct.get(field).and_then(Value::as_str) {
                if keys.contains(key) {
                    linked.insert(key.to_owned());
                }
            }
        }
        let missing = entry.kind == "uploaded_file" && entry.classification == Class::Unresolved;
        let access = if missing {
            "missing"
        } else if matches!(entry.classification, Class::Native | Class::Transformed) {
            "native"
        } else {
            "retained"
        };
        let download_path = if entry.level == "file" {
            Some(file.clone())
        } else {
            entry
                .direct
                .get("bundle_path")
                .and_then(Value::as_str)
                .and_then(|path| path.strip_prefix(&format!("{BUNDLE_DIR}/")))
                .filter(|path| file_bytes.contains_key(path))
                .map(str::to_owned)
        };
        let id = deterministic_uuid(
            "linear-source-record",
            &format!("{bundle_id}|{file}|{}|{}", entry.pointer, entry.kind),
        );
        if !ids.insert(id.clone()) {
            continue;
        }
        let direct = if serde_json::to_vec(&entry.direct)?.len() > 16_000 {
            Value::Null
        } else {
            entry.direct.clone()
        };
        records.push(SourceRecord {
            id,
            bundle_id: bundle_id.clone(),
            kind: truncate(&entry.kind, 80),
            level: entry.level.into(),
            source_id: entry.source_id.as_deref().map(|id| truncate(id, 300)),
            label: entry
                .source_label
                .as_deref()
                .map(|label| truncate(label, 1_000)),
            title: entry
                .source_title
                .as_deref()
                .map(|title| truncate(title, 2_000)),
            classification: entry.classification.as_str().into(),
            access: access.into(),
            file,
            start,
            end,
            pointer: truncate(&entry.pointer, 500),
            direct,
            reasons: entry
                .reasons
                .iter()
                .take(50)
                .map(|reason| truncate(reason, 2_000))
                .collect(),
            preserved_fields: entry
                .preserved_fields
                .iter()
                .take(500)
                .map(|field| truncate(field, 200))
                .collect(),
            issue_keys: linked.into_iter().take(100).collect(),
            download_path,
            search_text: search_text(entry, value),
        });
    }

    let bundle = SourceBundle {
        id: bundle_id,
        source: "linear".into(),
        label: "Linear workspace capture".into(),
        captured_at: package.manifest.captured_at.clone(),
        manifest_sha256: package.manifest_sha256.to_ascii_lowercase(),
        file_count: files.len() as u64,
        total_bytes: total,
        record_count: records.len() as u64,
        summary,
        application: None,
        imported_by: imported_by.into(),
        imported_at,
    };
    Ok(Retained {
        bundle,
        files,
        records,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_value_spans_are_exact_original_bytes() {
        let bytes = br#"{"captured_at":"x","records":[ {"id":"a","n":1} ,{"id":"b"}]}"#;
        let envelope: Envelope = serde_json::from_slice(bytes).unwrap();
        let spans = spans_of(bytes, &envelope.records);
        assert_eq!(
            &bytes[spans[0].0 as usize..spans[0].1 as usize],
            br#"{"id":"a","n":1}"#
        );
        assert_eq!(
            &bytes[spans[1].0 as usize..spans[1].1 as usize],
            br#"{"id":"b"}"#
        );
        assert_eq!(record_index("/records/12/labels/nodes/0"), Some(12));
        assert_eq!(record_index("/3"), Some(3));
    }
}
