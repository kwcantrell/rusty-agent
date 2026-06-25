pub mod bridge;
pub mod llama;
pub mod workspace;

use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

struct AppState {
    bridge: Arc<bridge::Bridge>,
    config_path: PathBuf, // app.json (persisted workspace)
}

#[tauri::command]
fn get_local_ws_url(state: tauri::State<'_, AppState>) -> String {
    state.bridge.ws_url()
}

#[tauri::command]
async fn get_workspace(state: tauri::State<'_, AppState>) -> Result<Option<String>, String> {
    // Return the EFFECTIVE workspace (defaults to $HOME on first launch) so the
    // TopBar always shows it + the Change… picker, even before one is persisted.
    Ok(Some(
        state
            .bridge
            .current_workspace()
            .await
            .to_string_lossy()
            .into_owned(),
    ))
}

#[tauri::command]
async fn pick_workspace(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    let folder = app.dialog().file().blocking_pick_folder();
    let Some(path) = folder else { return Ok(None) };
    let dir = path.into_path().map_err(|e| e.to_string())?;
    // Persist, then reconnect the runtime to the new dir.
    let cfg = workspace::AppConfig { workspace: Some(dir.clone()) };
    cfg.save(&state.config_path).map_err(|e| e.to_string())?;
    state.bridge.set_workspace(dir.clone()).await;
    Ok(Some(dir.to_string_lossy().into_owned()))
}

#[tauri::command]
async fn llama_health() -> llama::LlamaHealth {
    llama::check_health("http://localhost:8080").await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_config_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| PathBuf::from("."));
            let config_path = app_config_dir.join("app.json");
            let runtime_config_path = app_config_dir.join("agent-runtime.json");

            // Restore the last workspace, or default to $HOME.
            let workspace = workspace::AppConfig::load(&config_path)
                .workspace
                .or_else(dirs_home)
                .unwrap_or_else(|| PathBuf::from("."));

            let bridge = tauri::async_runtime::block_on(bridge::start(
                workspace,
                runtime_config_path,
                "http://localhost:8080".into(),
                "qwen3.6-35b-a3b".into(),
            ))?;
            app.manage(AppState { bridge, config_path });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_local_ws_url,
            get_workspace,
            pick_workspace,
            llama_health
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
