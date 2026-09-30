#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[tauri::command]
async fn direct_command(request: direct_core::Request) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = direct::data_dir().map_err(|e| e.to_string())?;
        direct::ensure_service(&dir).map_err(|e| e.to_string())?;
        direct::Client::new(&dir)
            .and_then(|c| c.call(&request, direct_core::Role::Human))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
/// Retained source file bytes, returned as a raw IPC payload (not JSON).
#[tauri::command]
async fn direct_source_file(
    bundle_id: String,
    path: String,
) -> Result<tauri::ipc::Response, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = direct::data_dir().map_err(|e| e.to_string())?;
        direct::ensure_service(&dir).map_err(|e| e.to_string())?;
        direct::Client::new(&dir)
            .and_then(|c| c.source_file(&bundle_id, &path, direct_core::Role::Human))
            .map(tauri::ipc::Response::new)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
/// Owner migration preview/apply. The artifact arrives as the raw request body;
/// `expected-cursor` and `artifact-sha256` headers select apply.
#[tauri::command]
async fn direct_migration(request: tauri::ipc::Request<'_>) -> Result<serde_json::Value, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("Migration artifact must be sent as raw bytes".into());
    };
    let bytes = bytes.clone();
    let header = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let expected = match (header("expected-cursor"), header("artifact-sha256")) {
        (Some(cursor), Some(sha)) => Some((cursor.parse::<u64>().map_err(|e| e.to_string())?, sha)),
        _ => None,
    };
    tauri::async_runtime::spawn_blocking(move || {
        let dir = direct::data_dir().map_err(|e| e.to_string())?;
        direct::ensure_service(&dir).map_err(|e| e.to_string())?;
        direct::Client::new(&dir)
            .and_then(|c| {
                c.migration(
                    bytes,
                    expected
                        .as_ref()
                        .map(|(cursor, sha)| (*cursor, sha.as_str())),
                    direct_core::Role::Human,
                )
            })
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            direct_command,
            direct_source_file,
            direct_migration
        ])
        .run(tauri::generate_context!())
        .expect("Unable to run Direct desktop");
}
