pub mod bridge;

use std::sync::Arc;
use tauri::Manager;

struct AppState {
    bridge: Arc<bridge::Bridge>,
}

#[tauri::command]
fn get_local_ws_url(state: tauri::State<'_, AppState>) -> String {
    state.bridge.ws_url()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Workspace defaults to home for now; Task 5 adds the picker + persistence.
            let workspace = dirs_home().unwrap_or_else(|| std::path::PathBuf::from("."));
            let config_path = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join("agent-runtime.json");
            let bridge = tauri::async_runtime::block_on(bridge::start(
                workspace,
                config_path,
                "http://localhost:8080".into(),
                "qwen3.6-35b-a3b".into(),
            ))?;
            app.manage(AppState { bridge });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_local_ws_url])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}
