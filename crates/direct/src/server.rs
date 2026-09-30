use crate::{protect_dir, Endpoint};
use anyhow::{Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
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
    path::Path,
    sync::{Arc, Mutex},
};
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer};
use uuid::Uuid;

#[derive(Clone)]
struct App {
    store: Arc<Mutex<Store>>,
    endpoint: Endpoint,
    grants: Arc<Mutex<HashMap<String, i64>>>,
    sessions: Arc<Mutex<HashMap<String, i64>>>,
    shutdown: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
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
async fn shutdown(State(app): State<App>, headers: HeaderMap) -> Response {
    if !same_host(&headers, &app.endpoint) || bearer(&headers) != app.endpoint.owner_token {
        return fail(StatusCode::FORBIDDEN, "Owner shutdown capability required");
    }
    if let Some(sender) = app.shutdown.lock().unwrap().take() {
        let _ = sender.send(());
    }
    Json(json!({"stopping":true})).into_response()
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
    if !expiry.is_some_and(|e| e > now()) {
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
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let app = App {
        store: Arc::new(Mutex::new(store)),
        endpoint: endpoint.clone(),
        grants: Arc::new(Mutex::new(HashMap::new())),
        sessions: Arc::new(Mutex::new(HashMap::new())),
        shutdown: Arc::new(Mutex::new(Some(shutdown_tx))),
    };
    let router=Router::new().route("/api/command",post(command)).route("/api/launch",post(launch)).route("/api/session",post(session)).route("/api/shutdown",post(shutdown))
      .fallback_service(ServeDir::new(assets)).layer(DefaultBodyLimit::max(1024*1024))
      .layer(SetResponseHeaderLayer::overriding(axum::http::header::CACHE_CONTROL,axum::http::HeaderValue::from_static("no-store")))
      .layer(SetResponseHeaderLayer::overriding(axum::http::header::CONTENT_SECURITY_POLICY,axum::http::HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; object-src 'none'; frame-ancestors 'none'; base-uri 'none'")))
      .with_state(app);
    println!(
        "Direct local service: http://127.0.0.1:{}\nData: {}\nOpen the interface with: direct open",
        endpoint.port,
        dir.display()
    );
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(async {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = shutdown_rx => {},
            }
        })
        .await;
    let _ = fs::remove_file(dir.join("endpoint.json"));
    drop(lock);
    result?;
    Ok(())
}
