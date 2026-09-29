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
fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![direct_command])
        .run(tauri::generate_context!())
        .expect("Unable to run Direct desktop");
}
