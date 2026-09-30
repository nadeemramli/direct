//! Retained external sources: verified original files kept in 1 MiB chunks,
//! plus a bounded record index whose entries are exact byte spans of those
//! files. Reads are lazy and scoped: a record view touches only the chunks of
//! its span and a download only the chunks of one file. Snapshot and issue
//! context carry metadata only.
//!
//! Source content is untrusted data. It is returned as text or bytes and is
//! never interpreted, executed or treated as guidance.

use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};

pub const SOURCE_CHUNK_BYTES: usize = 1 << 20;
/// Largest single retained file (the largest known capture file is ~74 MB).
pub const MAX_SOURCE_FILE_BYTES: u64 = 256 << 20;
/// Largest retained bundle, including uploaded files.
pub const MAX_SOURCE_BUNDLE_BYTES: u64 = 768 << 20;
/// Largest migration artifact the owner upload endpoint accepts.
pub const MAX_MIGRATION_ARTIFACT_BYTES: usize = 1 << 30;
/// Largest record span returned inline; larger spans are truncated with a flag.
pub const MAX_SOURCE_RECORD_VIEW_BYTES: u64 = 8 << 20;
const MAX_SEARCH_TEXT: usize = 4_096;
const MAX_SUMMARY_BYTES: usize = 512 << 10;
const MAX_DIRECT_BYTES: usize = 16 << 10;
const MAX_CONTEXT_SOURCES: usize = 200;
pub(crate) const CLASSIFICATIONS: &[&str] = &["native", "transformed", "preserved", "unresolved"];
pub(crate) const ACCESS: &[&str] = &["native", "retained", "missing"];

/// Flat relative paths only: `name` or `data/<name>` / `attachments/<name>`.
pub fn safe_source_path(path: &str) -> bool {
    if path.is_empty()
        || path.len() > 300
        || path.contains(['\\', ':', '\0'])
        || path.chars().any(char::is_control)
    {
        return false;
    }
    let parts: Vec<&str> = path.split('/').collect();
    (1..=2).contains(&parts.len())
        && parts
            .iter()
            .all(|part| !part.is_empty() && !part.starts_with('.') && part.trim() == *part)
        && (parts.len() == 1 || matches!(parts[0], "data" | "attachments"))
}

fn hex64(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}

fn bounded(value: &str, field: &str, max: usize) -> Result<()> {
    if value.len() > max {
        return Err(err(
            "invalid",
            format!("Retained source {field} is too long"),
        ));
    }
    Ok(())
}

pub(crate) fn file_id(bundle_id: &str, path: &str) -> String {
    format!("{bundle_id}/{path}")
}

/// Structural validation of source metadata (no file bytes needed).
pub(crate) fn validate_source_metadata(a: &Archive) -> Result<()> {
    if a.format < 12
        && (!a.source_bundles.is_empty()
            || !a.source_files.is_empty()
            || !a.source_records.is_empty())
    {
        return Err(err("invalid", "Retained sources require archive format 12"));
    }
    let issue_keys: HashSet<&str> = a.issues.iter().map(|issue| issue.key.as_str()).collect();
    let mut bundles: HashMap<&str, &SourceBundle> = HashMap::new();
    let mut manifests = HashSet::new();
    for bundle in &a.source_bundles {
        if Uuid::parse_str(&bundle.id).is_err()
            || bundles.insert(&bundle.id, bundle).is_some()
            || !manifests.insert(bundle.manifest_sha256.to_ascii_lowercase())
            || !hex64(&bundle.manifest_sha256)
            || bundle.source.trim().is_empty()
            || bundle.imported_by.trim().is_empty()
        {
            return Err(err(
                "invalid",
                "Invalid or duplicate retained source bundle",
            ));
        }
        bounded(&bundle.source, "source", 60)?;
        bounded(&bundle.label, "label", 200)?;
        bounded(&bundle.captured_at, "capture time", 60)?;
        bounded(&bundle.imported_by, "importer", 120)?;
        if serde_json::to_vec(&bundle.summary)?.len() > MAX_SUMMARY_BYTES {
            return Err(err("invalid", "Retained source summary is too large"));
        }
        if let Some(application) = &bundle.application {
            if !hex64(&application.artifact_sha256)
                || application.applied_cursor < application.baseline_cursor
            {
                return Err(err("invalid", "Invalid migration application record"));
            }
            if let Some(backup) = &application.backup {
                bounded(backup, "backup name", 200)?;
            }
        }
    }
    let mut files: HashMap<(&str, &str), &SourceFile> = HashMap::new();
    let mut totals: HashMap<&str, (u64, u64)> = HashMap::new();
    for file in &a.source_files {
        if !bundles.contains_key(file.bundle_id.as_str())
            || !safe_source_path(&file.path)
            || !hex64(&file.sha256)
            || file.bytes > MAX_SOURCE_FILE_BYTES
            || file.content_type.trim().is_empty()
            || file.content_type.len() > 100
            || !matches!(file.role.as_str(), "manifest" | "data" | "upload")
            || file
                .original_name
                .as_ref()
                .is_some_and(|name| name.len() > 300)
            || files
                .insert((file.bundle_id.as_str(), file.path.as_str()), file)
                .is_some()
        {
            return Err(err(
                "invalid",
                format!("Invalid or duplicate retained file {:?}", file.path),
            ));
        }
        let total = totals.entry(&file.bundle_id).or_default();
        total.0 += 1;
        total.1 += file.bytes;
    }
    let mut record_counts: HashMap<&str, u64> = HashMap::new();
    let mut record_ids = HashSet::new();
    for record in &a.source_records {
        let Some(file) = files.get(&(record.bundle_id.as_str(), record.file.as_str())) else {
            return Err(err("invalid", "Retained record references an unknown file"));
        };
        if Uuid::parse_str(&record.id).is_err()
            || !record_ids.insert(record.id.as_str())
            || record.start >= record.end
            || record.end > file.bytes
            || !matches!(record.level.as_str(), "record" | "component" | "file")
            || !CLASSIFICATIONS.contains(&record.classification.as_str())
            || !ACCESS.contains(&record.access.as_str())
            || record.kind.trim().is_empty()
            || record.issue_keys.len() > 100
            || record
                .issue_keys
                .iter()
                .any(|key| !issue_keys.contains(key.as_str()))
            || record.reasons.len() > 50
            || record.preserved_fields.len() > 500
            || record.download_path.as_ref().is_some_and(|path| {
                !files.contains_key(&(record.bundle_id.as_str(), path.as_str()))
            })
        {
            return Err(err("invalid", "Invalid retained source record"));
        }
        bounded(&record.kind, "record kind", 80)?;
        bounded(record.source_id.as_deref().unwrap_or(""), "record id", 300)?;
        bounded(record.label.as_deref().unwrap_or(""), "record label", 1_000)?;
        bounded(record.title.as_deref().unwrap_or(""), "record title", 2_000)?;
        bounded(&record.pointer, "record pointer", 500)?;
        bounded(&record.search_text, "search text", MAX_SEARCH_TEXT)?;
        for reason in &record.reasons {
            bounded(reason, "record reason", 2_000)?;
        }
        for field in &record.preserved_fields {
            bounded(field, "record field", 200)?;
        }
        if serde_json::to_vec(&record.direct)?.len() > MAX_DIRECT_BYTES {
            return Err(err("invalid", "Retained record reference is too large"));
        }
        *record_counts.entry(&record.bundle_id).or_default() += 1;
    }
    for bundle in &a.source_bundles {
        let (count, bytes) = totals.get(bundle.id.as_str()).copied().unwrap_or_default();
        if count != bundle.file_count
            || bytes != bundle.total_bytes
            || bytes > MAX_SOURCE_BUNDLE_BYTES
            || record_counts.get(bundle.id.as_str()).copied().unwrap_or(0) != bundle.record_count
        {
            return Err(err(
                "invalid",
                "Retained source bundle totals disagree with its files or records",
            ));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct IdOnly {
    #[serde(default)]
    id: Option<String>,
}

/// Verify file bytes and that every record span is well-formed JSON from its file.
pub(crate) fn validate_source_payload<'a>(
    files: impl IntoIterator<Item = (&'a SourceFile, &'a [u8])>,
    records: &[SourceRecord],
) -> Result<()> {
    let mut bytes_by_file: HashMap<(&str, &str), &[u8]> = HashMap::new();
    for (file, bytes) in files {
        if bytes.len() as u64 != file.bytes
            || !file
                .sha256
                .eq_ignore_ascii_case(&format!("{:x}", Sha256::digest(bytes)))
        {
            return Err(err(
                "invalid",
                format!("Retained file {} failed its checksum", file.path),
            ));
        }
        bytes_by_file.insert((file.bundle_id.as_str(), file.path.as_str()), bytes);
    }
    for record in records {
        let bytes = bytes_by_file
            .get(&(record.bundle_id.as_str(), record.file.as_str()))
            .ok_or_else(|| err("invalid", "Retained record file bytes are missing"))?;
        let span = bytes
            .get(record.start as usize..record.end as usize)
            .ok_or_else(|| err("invalid", "Retained record span is outside its file"))?;
        if record.level == "file" {
            continue;
        }
        if record.level == "record" && record.source_id.is_some() {
            let parsed: IdOnly = serde_json::from_slice(span)
                .map_err(|_| err("invalid", "Retained record span is not a JSON object"))?;
            if parsed.id.is_some() && parsed.id != record.source_id {
                return Err(err(
                    "invalid",
                    "Retained record span does not match its source ID",
                ));
            }
        } else {
            serde_json::from_slice::<serde::de::IgnoredAny>(span)
                .map_err(|_| err("invalid", "Retained record span is not valid JSON"))?;
        }
    }
    Ok(())
}

/// Archive-level byte validation: every file must carry base64 data.
pub(crate) fn validate_source_archive_bytes(a: &Archive) -> Result<()> {
    let decoded = a
        .source_files
        .iter()
        .map(|file| decode_file(file).map(|bytes| (file, bytes)))
        .collect::<Result<Vec<_>>>()?;
    validate_source_payload(
        decoded
            .iter()
            .map(|(file, bytes)| (*file, bytes.as_slice())),
        &a.source_records,
    )
}

pub(crate) fn decode_file(file: &SourceFile) -> Result<Vec<u8>> {
    let data = file.data.as_deref().ok_or_else(|| {
        err(
            "invalid",
            format!("Retained file {} has no data", file.path),
        )
    })?;
    STANDARD.decode(data).map_err(|_| {
        err(
            "invalid",
            format!("Retained file {} data is not base64", file.path),
        )
    })
}

pub(crate) fn put_bundle(conn: &Connection, bundle: &SourceBundle) -> Result<()> {
    conn.execute(
        "INSERT INTO source_bundles VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![bundle.id, serde_json::to_string(bundle)?],
    )?;
    Ok(())
}

pub(crate) fn put_file(conn: &Connection, file: &SourceFile, bytes: &[u8]) -> Result<()> {
    let id = file_id(&file.bundle_id, &file.path);
    let mut meta = file.clone();
    meta.data = None;
    conn.execute(
        "INSERT INTO source_files VALUES (?1,?2,?3)",
        params![id, file.bundle_id, serde_json::to_string(&meta)?],
    )?;
    let mut statement = conn.prepare("INSERT INTO source_chunks VALUES (?1,?2,?3)")?;
    for (seq, chunk) in bytes.chunks(SOURCE_CHUNK_BYTES).enumerate() {
        statement.execute(params![id, seq as i64, chunk])?;
    }
    Ok(())
}

pub(crate) fn put_record(conn: &Connection, record: &SourceRecord) -> Result<()> {
    let mut search = String::new();
    for part in [
        record.label.as_deref().unwrap_or(""),
        record.title.as_deref().unwrap_or(""),
        record.source_id.as_deref().unwrap_or(""),
        &record.kind,
        &record.search_text,
    ] {
        search.push_str(&part.to_lowercase());
        search.push('\n');
    }
    for key in &record.issue_keys {
        search.push_str(&key.to_lowercase());
        search.push('\n');
    }
    conn.execute(
        "INSERT INTO source_records VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            record.id,
            record.bundle_id,
            record.kind,
            record.classification,
            search,
            serde_json::to_string(record)?
        ],
    )?;
    for key in &record.issue_keys {
        conn.execute(
            "INSERT OR IGNORE INTO source_record_issues VALUES (?1,?2)",
            params![key, record.id],
        )?;
    }
    Ok(())
}

pub(crate) fn delete_bundle(conn: &Connection, bundle_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM source_record_issues WHERE record_id IN (SELECT id FROM source_records WHERE bundle_id=?1)",
        [bundle_id],
    )?;
    conn.execute("DELETE FROM source_records WHERE bundle_id=?1", [bundle_id])?;
    conn.execute(
        "DELETE FROM source_chunks WHERE file_id IN (SELECT id FROM source_files WHERE bundle_id=?1)",
        [bundle_id],
    )?;
    conn.execute("DELETE FROM source_files WHERE bundle_id=?1", [bundle_id])?;
    conn.execute("DELETE FROM source_bundles WHERE id=?1", [bundle_id])?;
    Ok(())
}

pub(crate) fn bundles(conn: &Connection) -> Result<Vec<SourceBundle>> {
    all::<SourceBundle>(conn, "source_bundles")
}

pub(crate) fn file_meta(conn: &Connection, bundle_id: &str, path: &str) -> Result<SourceFile> {
    let data: Option<String> = conn
        .query_row(
            "SELECT data FROM source_files WHERE id=?1",
            [file_id(bundle_id, path)],
            |row| row.get(0),
        )
        .optional()?;
    Ok(serde_json::from_str(&data.ok_or_else(|| {
        err("not_found", "Unknown retained file")
    })?)?)
}

fn read_range(conn: &Connection, file: &SourceFile, start: u64, end: u64) -> Result<Vec<u8>> {
    let id = file_id(&file.bundle_id, &file.path);
    let chunk = SOURCE_CHUNK_BYTES as u64;
    let (first, last) = (start / chunk, (end.max(start + 1) - 1) / chunk);
    let mut statement = conn.prepare(
        "SELECT seq, data FROM source_chunks WHERE file_id=?1 AND seq BETWEEN ?2 AND ?3 ORDER BY seq",
    )?;
    let mut bytes = Vec::with_capacity((end - start) as usize);
    let mut expected = first;
    let rows = statement.query_map(params![id, first as i64, last as i64], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;
    for row in rows {
        let (seq, data) = row?;
        if seq as u64 != expected {
            return Err(err("storage", "Retained file chunks are incomplete"));
        }
        expected += 1;
        let chunk_start = seq as u64 * chunk;
        let from = start.saturating_sub(chunk_start) as usize;
        let to = ((end - chunk_start) as usize).min(data.len());
        if from > to {
            return Err(err("storage", "Retained file chunks are incomplete"));
        }
        bytes.extend_from_slice(&data[from..to]);
    }
    if bytes.len() as u64 != end - start {
        return Err(err("storage", "Retained file chunks are incomplete"));
    }
    Ok(bytes)
}

/// Whole retained file, verified against its recorded checksum.
pub(crate) fn file_bytes(conn: &Connection, file: &SourceFile) -> Result<Vec<u8>> {
    let bytes = if file.bytes == 0 {
        Vec::new()
    } else {
        read_range(conn, file, 0, file.bytes)?
    };
    if !file
        .sha256
        .eq_ignore_ascii_case(&format!("{:x}", Sha256::digest(&bytes)))
    {
        return Err(err(
            "storage",
            format!("Retained file {} failed its checksum", file.path),
        ));
    }
    Ok(bytes)
}

pub(crate) fn export_sources(
    conn: &Connection,
    include_bytes: bool,
) -> Result<(Vec<SourceBundle>, Vec<SourceFile>, Vec<SourceRecord>)> {
    let bundles = bundles(conn)?;
    let mut files = all::<SourceFile>(conn, "source_files")?;
    if include_bytes {
        for file in &mut files {
            file.data = Some(STANDARD.encode(file_bytes(conn, file)?));
        }
    }
    let records = all::<SourceRecord>(conn, "source_records")?;
    Ok((bundles, files, records))
}

pub(crate) fn restore_sources(
    tx: &Transaction,
    bundles: &[SourceBundle],
    files: &[SourceFile],
    records: &[SourceRecord],
) -> Result<()> {
    for bundle in bundles {
        put_bundle(tx, bundle)?;
    }
    for file in files {
        put_file(tx, file, &decode_file(file)?)?;
    }
    for record in records {
        put_record(tx, record)?;
    }
    Ok(())
}

/// Small bundle overview for snapshot polling (no summary or entity lists).
pub(crate) fn overview(conn: &Connection) -> Result<Vec<Value>> {
    Ok(bundles(conn)?
        .into_iter()
        .map(|bundle| {
            json!({
                "id": bundle.id,
                "source": bundle.source,
                "label": bundle.label,
                "captured_at": bundle.captured_at,
                "manifest_sha256": bundle.manifest_sha256,
                "file_count": bundle.file_count,
                "total_bytes": bundle.total_bytes,
                "record_count": bundle.record_count,
                "imported_by": bundle.imported_by,
                "imported_at": bundle.imported_at,
                "applied_cursor": bundle.application.as_ref().map(|application| application.applied_cursor),
            })
        })
        .collect())
}

/// Bundle list with summaries and entity counts (not entity lists).
pub(crate) fn describe(conn: &Connection) -> Result<Value> {
    let current = cursor(conn)?;
    let mut items = Vec::new();
    for bundle in bundles(conn)? {
        let mut access: BTreeMap<String, u64> = BTreeMap::new();
        let mut statement = conn.prepare(
            "SELECT json_extract(data,'$.access'), COUNT(*) FROM source_records WHERE bundle_id=?1 GROUP BY 1",
        )?;
        for row in statement.query_map([&bundle.id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })? {
            let (key, count) = row?;
            access.insert(key, count as u64);
        }
        let mut kinds: BTreeMap<String, u64> = BTreeMap::new();
        let mut statement = conn.prepare(
            "SELECT kind, COUNT(*) FROM source_records WHERE bundle_id=?1 GROUP BY kind ORDER BY kind",
        )?;
        for row in statement.query_map([&bundle.id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })? {
            let (key, count) = row?;
            kinds.insert(key, count as u64);
        }
        let files: Vec<SourceFile> = {
            let mut statement =
                conn.prepare("SELECT data FROM source_files WHERE bundle_id=?1 ORDER BY id")?;
            let rows = statement.query_map([&bundle.id], |row| row.get::<_, String>(0))?;
            rows.map(|row| Ok(serde_json::from_str(&row?)?))
                .collect::<Result<Vec<_>>>()?
        };
        let application = bundle.application.as_ref().map(|application| {
            json!({
                "artifact_sha256": application.artifact_sha256,
                "baseline_cursor": application.baseline_cursor,
                "applied_cursor": application.applied_cursor,
                "backup": application.backup,
                "rollback_available": application.applied_cursor == current,
                "entities": {
                    "products": application.entities.products.len(),
                    "projects": application.entities.projects.len(),
                    "goals": application.entities.goals.len(),
                    "milestones": application.entities.milestones.len(),
                    "labels": application.entities.labels.len(),
                    "issues": application.entities.issues.len(),
                    "comments": application.entities.comments.len(),
                    "issue_links": application.entities.issue_links.len(),
                },
            })
        });
        items.push(json!({
            "id": bundle.id,
            "source": bundle.source,
            "label": bundle.label,
            "captured_at": bundle.captured_at,
            "manifest_sha256": bundle.manifest_sha256,
            "file_count": bundle.file_count,
            "total_bytes": bundle.total_bytes,
            "record_count": bundle.record_count,
            "imported_by": bundle.imported_by,
            "imported_at": bundle.imported_at,
            "summary": bundle.summary,
            "application": application,
            "records_by_access": access,
            "records_by_kind": kinds,
            "files": files,
        }));
    }
    Ok(json!({"bundles": items, "cursor": current}))
}

fn summary(record: &SourceRecord) -> Value {
    json!({
        "id": record.id,
        "bundle_id": record.bundle_id,
        "kind": record.kind,
        "level": record.level,
        "source_id": record.source_id,
        "label": record.label,
        "title": record.title,
        "classification": record.classification,
        "access": record.access,
        "issue_keys": record.issue_keys,
        "download_path": record.download_path,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn search(
    conn: &Connection,
    query: &str,
    bundle_id: Option<&str>,
    kind: Option<&str>,
    classification: Option<&str>,
    issue_key: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<Value> {
    bounded(query, "query", 200)?;
    let limit = limit.clamp(1, 200);
    let pattern = format!(
        "%{}%",
        query
            .trim()
            .to_lowercase()
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let filter = "(?1 IS NULL OR bundle_id=?1) AND (?2 IS NULL OR kind=?2) AND (?3 IS NULL OR classification=?3) \
        AND search LIKE ?4 ESCAPE '\\' \
        AND (?5 IS NULL OR id IN (SELECT record_id FROM source_record_issues WHERE issue_key=?5))";
    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM source_records WHERE {filter}"),
        params![bundle_id, kind, classification, pattern, issue_key],
        |row| row.get(0),
    )?;
    let mut statement = conn.prepare(&format!(
        "SELECT data FROM source_records WHERE {filter} ORDER BY kind, id LIMIT ?6 OFFSET ?7"
    ))?;
    let rows = statement.query_map(
        params![
            bundle_id,
            kind,
            classification,
            pattern,
            issue_key,
            limit,
            offset
        ],
        |row| row.get::<_, String>(0),
    )?;
    let mut records = Vec::new();
    for row in rows {
        let record: SourceRecord = serde_json::from_str(&row?)?;
        records.push(summary(&record));
    }
    Ok(json!({"total": total, "offset": offset, "limit": limit, "records": records}))
}

pub(crate) fn for_issue(conn: &Connection, key: &str) -> Result<Vec<Value>> {
    let mut statement = conn.prepare(
        "SELECT r.data FROM source_records r JOIN source_record_issues i ON i.record_id=r.id \
         WHERE i.issue_key=?1 ORDER BY r.kind, r.id LIMIT ?2",
    )?;
    let rows = statement.query_map(params![key, MAX_CONTEXT_SOURCES as i64], |row| {
        row.get::<_, String>(0)
    })?;
    rows.map(|row| {
        let record: SourceRecord = serde_json::from_str(&row?)?;
        Ok(summary(&record))
    })
    .collect()
}

const READABLE_FIELDS: &[&str] = &[
    "title",
    "name",
    "identifier",
    "description",
    "content",
    "body",
    "subtitle",
    "url",
];

/// One record with its exact original bytes, readable text fields and files.
pub(crate) fn record_view(conn: &Connection, id: &str) -> Result<Value> {
    let data: Option<String> = conn
        .query_row("SELECT data FROM source_records WHERE id=?1", [id], |row| {
            row.get(0)
        })
        .optional()?;
    let record: SourceRecord =
        serde_json::from_str(&data.ok_or_else(|| err("not_found", "Unknown retained record"))?)?;
    let file = file_meta(conn, &record.bundle_id, &record.file)?;
    let end = record.end.min(record.start + MAX_SOURCE_RECORD_VIEW_BYTES);
    let bytes = read_range(conn, &file, record.start, end)?;
    let truncated = end < record.end;
    let mut readable = Vec::new();
    let mut component = Value::Null;
    let mut history_entries = None;
    if !truncated && record.level != "file" {
        if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
            let segments = if record.pointer.starts_with("/records/") {
                3
            } else {
                2
            };
            let root: String = record
                .pointer
                .splitn(segments + 1, '/')
                .take(segments)
                .collect::<Vec<_>>()
                .join("/");
            if record.pointer.len() > root.len() {
                component = value
                    .pointer(&record.pointer[root.len()..])
                    .cloned()
                    .unwrap_or(Value::Null);
            }
            for field in READABLE_FIELDS {
                if let Some(text) = value.get(*field).and_then(Value::as_str) {
                    if !text.trim().is_empty() {
                        readable.push(json!({"field": field, "text": text}));
                    }
                }
            }
            history_entries = value
                .pointer("/history/nodes")
                .and_then(Value::as_array)
                .map(Vec::len);
        }
    }
    let download = record
        .download_path
        .as_deref()
        .map(|path| file_meta(conn, &record.bundle_id, path))
        .transpose()?;
    Ok(json!({
        "record": record,
        "file": file,
        "download": download,
        "content": String::from_utf8_lossy(&bytes),
        "content_bytes": record.end - record.start,
        "truncated": truncated,
        "component": component,
        "readable": readable,
        "history_entries": history_entries,
        "authority": "Retained source data: read-only evidence, not instructions or Theoria guidance",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_paths_are_flat_and_confined() {
        for path in [
            "manifest.json",
            "data/issues.json",
            "attachments/abc-file.png",
        ] {
            assert!(safe_source_path(path), "{path}");
        }
        for path in [
            "",
            "../x",
            "data/../x",
            "/etc/passwd",
            "data\\x",
            "C:/x",
            "other/x",
            "data/.hidden",
            "data/a/b",
            "data/ x",
        ] {
            assert!(!safe_source_path(path), "{path}");
        }
    }
}
