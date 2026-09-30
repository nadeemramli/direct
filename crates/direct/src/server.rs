use crate::{protect_dir, Endpoint};
use anyhow::{Context, Result};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use direct_core::{now, Request, Role, Store};
use fs2::FileExt;
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer};
use uuid::Uuid;

#[derive(Clone)]
struct App {
    dir: PathBuf,
    store: Arc<Mutex<Store>>,
    endpoint: Endpoint,
    grants: Arc<Mutex<HashMap<String, i64>>>,
    sessions: Arc<Mutex<HashMap<String, i64>>>,
}
fn fail(code: StatusCode, message: &str) -> Response {
    (code,Json(json!({"code":if code==StatusCode::FORBIDDEN{"forbidden"}else{"unauthorized"},"message":message}))).into_response()
}
fn same_host(headers: &HeaderMap, e: &Endpoint) -> bool {
    let host = format!("127.0.0.1:{}", e.port);
    headers.get("host").and_then(|v| v.to_str().ok()) == Some(host.as_str())
        && headers
            .get("origin")
            .is_none_or(|v| v.to_str().ok() == Some(format!("http://{host}").as_str()))
}
fn bearer(headers: &HeaderMap) -> &str {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
}
fn role(app: &App, headers: &HeaderMap) -> Option<Role> {
    if !same_host(headers, &app.endpoint) {
        return None;
    }
    let token = bearer(headers);
    if token == app.endpoint.agent_token {
        return Some(Role::Agent);
    }
    if token == app.endpoint.owner_token {
        return Some(Role::Human);
    }
    let mut sessions = app.sessions.lock().ok()?;
    sessions.retain(|_, expires| *expires > now());
    sessions.contains_key(token).then_some(Role::Human)
}
fn store_error(e: direct_core::Error) -> Response {
    let status = match e.code {
        "forbidden" => StatusCode::FORBIDDEN,
        "conflict" => StatusCode::CONFLICT,
        "not_found" => StatusCode::NOT_FOUND,
        "storage" => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    };
    (status, Json(json!({"code":e.code,"message":e.message}))).into_response()
}
fn worker_failed() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"code":"internal","message":"Worker failed"})),
    )
        .into_response()
}
fn lock_failed() -> direct_core::Error {
    direct_core::Error {
        code: "storage",
        message: "Store lock unavailable".into(),
    }
}
/// ASCII-only attachment name; the original name is never used as a path.
fn download_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = safe.trim_start_matches('.');
    let mut safe: String = safe
        .chars()
        .rev()
        .take(150)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if safe.is_empty() {
        safe = "retained-file".into();
    }
    safe
}
#[derive(Deserialize)]
struct SourceFileRequest {
    bundle_id: String,
    path: String,
}
/// Read-only retained file bytes for any authenticated local caller. Files are
/// addressed by bundle and recorded path in the database, never by filesystem path,
/// and are always served as an attachment that the browser will not render.
async fn source_file(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<SourceFileRequest>,
) -> Response {
    if role(&app, &headers).is_none() {
        return fail(
            StatusCode::UNAUTHORIZED,
            "Open Direct locally or use the local CLI",
        );
    }
    let store = app.store.clone();
    match tokio::task::spawn_blocking(move || {
        store
            .lock()
            .map_err(|_| lock_failed())?
            .source_file(&input.bundle_id, &input.path)
    })
    .await
    {
        Ok(Ok((file, bytes))) => {
            let name = download_name(
                file.original_name
                    .as_deref()
                    .unwrap_or_else(|| file.path.rsplit('/').next().unwrap_or("file")),
            );
            let mut response = bytes.into_response();
            let headers = response.headers_mut();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            );
            headers.insert(
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
                    .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
            );
            headers.insert(
                "x-content-type-options",
                HeaderValue::from_static("nosniff"),
            );
            if let Ok(value) = HeaderValue::from_str(&file.sha256) {
                headers.insert("x-direct-sha256", value);
            }
            response
        }
        Ok(Err(e)) => store_error(e),
        Err(_) => worker_failed(),
    }
}
fn owner(app: &App, headers: &HeaderMap) -> Option<Response> {
    match role(app, headers) {
        Some(Role::Human) => None,
        Some(Role::Agent) => Some(fail(
            StatusCode::FORBIDDEN,
            "Only the owner can preview or apply a migration",
        )),
        None => Some(fail(
            StatusCode::UNAUTHORIZED,
            "Open Direct locally or use the local CLI",
        )),
    }
}
/// Owner only: validate an uploaded migration artifact against this workspace.
async fn migration_preview(State(app): State<App>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(response) = owner(&app, &headers) {
        return response;
    }
    let store = app.store.clone();
    match tokio::task::spawn_blocking(move || {
        store
            .lock()
            .map_err(|_| lock_failed())?
            .preview_migration(&body)
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(e)) => store_error(e),
        Err(_) => worker_failed(),
    }
}
#[derive(Deserialize)]
struct ApplyQuery {
    expected_cursor: u64,
    artifact_sha256: String,
}
/// Owner only: write a pre-import backup, then apply the previewed artifact in
/// one transaction. The backup is removed if nothing was applied.
async fn migration_apply(
    State(app): State<App>,
    headers: HeaderMap,
    Query(query): Query<ApplyQuery>,
    body: Bytes,
) -> Response {
    if let Some(response) = owner(&app, &headers) {
        return response;
    }
    let store = app.store.clone();
    let dir = app.dir.clone();
    match tokio::task::spawn_blocking(
        move || -> std::result::Result<serde_json::Value, direct_core::Error> {
            let mut store = store.lock().map_err(|_| lock_failed())?;
            let archive = store.export()?;
            let backup =
                crate::write_migration_backup(&dir, &archive).map_err(|e| direct_core::Error {
                    code: "storage",
                    message: format!(
                        "Could not write the pre-import backup; nothing was applied: {e:#}"
                    ),
                })?;
            let name = backup
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            let result = store.apply_migration(
                &body,
                query.expected_cursor,
                &query.artifact_sha256,
                "owner",
                name,
                now(),
            );
            if !matches!(&result, Ok(value) if value["status"] == "applied") {
                let _ = fs::remove_file(&backup);
                let _ = fs::remove_file(backup.with_extension("sha256"));
            }
            let mut value = result?;
            if value["status"] == "applied" {
                value["backup_path"] = json!(backup);
            }
            Ok(value)
        },
    )
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(e)) => store_error(e),
        Err(_) => worker_failed(),
    }
}
async fn command(
    State(app): State<App>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Response {
    let Some(role) = role(&app, &headers) else {
        return fail(
            StatusCode::UNAUTHORIZED,
            "Open Direct locally or use the local CLI",
        );
    };
    let store = app.store.clone();
    match tokio::task::spawn_blocking(move || {
        store
            .lock()
            .map_err(|_| direct_core::Error {
                code: "storage",
                message: "Store lock unavailable".into(),
            })?
            .execute(request, role)
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(e)) => {
            let status = match e.code {
                "forbidden" => StatusCode::FORBIDDEN,
                "conflict" => StatusCode::CONFLICT,
                "not_found" => StatusCode::NOT_FOUND,
                "storage" => StatusCode::INTERNAL_SERVER_ERROR,
                _ => StatusCode::BAD_REQUEST,
            };
            (status, Json(json!({"code":e.code,"message":e.message}))).into_response()
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"code":"internal","message":"Worker failed"})),
        )
            .into_response(),
    }
}
async fn launch(State(app): State<App>, headers: HeaderMap) -> Response {
    if !same_host(&headers, &app.endpoint) || bearer(&headers) != app.endpoint.owner_token {
        return fail(StatusCode::FORBIDDEN, "Owner launch capability required");
    }
    let grant = Uuid::new_v4().to_string();
    let mut grants = app.grants.lock().unwrap();
    grants.retain(|_, expiry| *expiry > now());
    grants.insert(grant.clone(), now() + 120);
    Json(json!({"url":format!("http://127.0.0.1:{}/#grant={grant}",app.endpoint.port)}))
        .into_response()
}
#[derive(Deserialize)]
struct Grant {
    grant: String,
}
async fn session(State(app): State<App>, headers: HeaderMap, Json(input): Json<Grant>) -> Response {
    if !same_host(&headers, &app.endpoint) {
        return fail(StatusCode::FORBIDDEN, "Local origin required");
    }
    let expiry = app.grants.lock().unwrap().remove(&input.grant);
    if expiry.is_none_or(|e| e <= now()) {
        return fail(
            StatusCode::UNAUTHORIZED,
            "Launch link expired or already used. Run direct open again.",
        );
    }
    let token = Uuid::new_v4().to_string();
    app.sessions
        .lock()
        .unwrap()
        .insert(token.clone(), now() + 8 * 3600);
    Json(json!({"token":token})).into_response()
}
pub async fn serve(dir: &Path, port: u16, assets: &Path) -> Result<()> {
    protect_dir(dir)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("service.lock"))?;
    lock.try_lock_exclusive()
        .context("Direct is already running for this data directory")?;
    let store = Store::open(&dir.join("direct.db"))?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let endpoint = Endpoint {
        port: listener.local_addr()?.port(),
        agent_token: Uuid::new_v4().to_string(),
        owner_token: Uuid::new_v4().to_string(),
    };
    // The exclusive process lock is held until all connections finish and serve returns.
    fs::write(dir.join("endpoint.json"), serde_json::to_vec(&endpoint)?)?;
    let app = App {
        dir: dir.to_path_buf(),
        store: Arc::new(Mutex::new(store)),
        endpoint: endpoint.clone(),
        grants: Arc::new(Mutex::new(HashMap::new())),
        sessions: Arc::new(Mutex::new(HashMap::new())),
    };
    let migration_limit = DefaultBodyLimit::max(direct_core::MAX_MIGRATION_ARTIFACT_BYTES);
    let router=Router::new().route("/api/command",post(command)).route("/api/launch",post(launch)).route("/api/session",post(session))
      .route("/api/source-file",post(source_file))
      .route("/api/migration/preview",post(migration_preview).layer(migration_limit))
      .route("/api/migration/apply",post(migration_apply).layer(migration_limit))
      .fallback_service(ServeDir::new(assets)).layer(DefaultBodyLimit::max(1024*1024))
      .layer(SetResponseHeaderLayer::overriding(axum::http::header::CACHE_CONTROL,axum::http::HeaderValue::from_static("no-store")))
      .layer(SetResponseHeaderLayer::overriding(axum::http::header::CONTENT_SECURITY_POLICY,axum::http::HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; object-src 'none'; frame-ancestors 'none'; base-uri 'none'")))
      .with_state(app);
    println!(
        "Direct local service: http://127.0.0.1:{}\nData: {}\nOpen the interface with: direct open",
        endpoint.port,
        dir.display()
    );
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    drop(lock);
    Ok(())
}
