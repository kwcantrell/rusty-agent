pub mod bridge;
mod channel_out;
pub mod llama;
pub mod workspace;

use agent_runtime_config::RuntimeConfig;
use agent_server::session::{SendOutcome, Session};
use agent_server::wire::{ContextSnapshot, Decision, ServerEvent, SettingsState};
use channel_out::ChannelOut;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

struct AppState {
    bridge: Arc<bridge::Bridge>,
    config_path: PathBuf, // app.json (persisted workspace)
}

fn session(state: &tauri::State<'_, AppState>) -> Arc<Session> {
    state.bridge.session()
}

/// Register/replace the outbound event channel for this webview.
#[tauri::command]
fn subscribe(state: tauri::State<'_, AppState>, channel: Channel<ServerEvent>) {
    session(&state).set_event_out(Arc::new(ChannelOut(channel)));
}

/// Start a run. Rejects with `busy` if one is already in flight (A1 guard).
///
/// MUST be `async`: `Session::send_input` calls `tokio::spawn`, which panics
/// ("there is no reactor running") when invoked from a SYNC command — sync
/// commands run on the WebKitGTK/glib main thread with no Tokio runtime entered,
/// and that panic aborts across the C-FFI boundary ("non-unwinding panic").
/// An `async` command runs on Tauri's managed Tokio runtime, so the inner
/// `tokio::spawn` has a reactor.
#[tauri::command]
async fn send_input(state: tauri::State<'_, AppState>, text: String) -> Result<(), String> {
    match session(&state).send_input(text) {
        SendOutcome::Started => Ok(()),
        SendOutcome::Busy => Err("busy".into()),
    }
}

/// Resolve a pending approval by correlation id.
#[tauri::command]
fn approve(state: tauri::State<'_, AppState>, id: String, decision: Decision) {
    session(&state).approve(&id, decision);
}

/// Trip the active run's cancellation token (B3 interactive cancel).
#[tauri::command]
fn cancel(state: tauri::State<'_, AppState>) {
    session(&state).cancel();
}

#[tauri::command]
fn settings_get(state: tauri::State<'_, AppState>) -> SettingsState {
    session(&state).settings_get()
}

#[tauri::command]
fn settings_update(
    state: tauri::State<'_, AppState>,
    settings: RuntimeConfig,
) -> Result<SettingsState, String> {
    session(&state).settings_update(settings)
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
async fn context_get(
    state: tauri::State<'_, AppState>,
) -> Result<ContextSnapshot, String> {
    Ok(session(&state).context_get().await)
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
            subscribe,
            send_input,
            approve,
            cancel,
            settings_get,
            settings_update,
            context_get,
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

#[cfg(test)]
mod cmd_tests {
    use super::*;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    fn app() -> tauri::App<tauri::test::MockRuntime> {
        let dir = tempfile::tempdir().unwrap();
        let bridge = tauri::async_runtime::block_on(bridge::start(
            dir.path().to_path_buf(),
            dir.path().join("rt.json"),
            "http://localhost:8080".into(),
            "m".into(),
        ))
        .unwrap();
        std::mem::forget(dir); // keep the temp dir alive for the test process
        mock_builder()
            .manage(AppState { bridge, config_path: PathBuf::from("/tmp/app.json") })
            .invoke_handler(tauri::generate_handler![
                subscribe, send_input, approve, cancel, settings_get, settings_update, context_get
            ])
            .build(mock_context(noop_assets()))
            .expect("failed to build mock app")
    }

    /// Smoke test: a registered command returns Ok over the mock IPC and the
    /// payload deserializes back to `SettingsState`. Behavioral coverage (run
    /// guard, cancel, approval, settings) lives in the agent-server Session tests.
    #[test]
    fn settings_get_returns_state_over_ipc() {
        let app = app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let res = tauri::test::get_ipc_response(
            &webview,
            tauri::webview::InvokeRequest {
                cmd: "settings_get".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::default(),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_string(),
            },
        );
        assert!(res.is_ok(), "settings_get should resolve: {res:?}");
        let state = res.unwrap().deserialize::<SettingsState>().unwrap();
        assert!(!state.api_key_set);
    }

    /// Smoke test: `context_get` resolves over the mock IPC and the payload
    /// deserializes to a `ContextSnapshot` with at least a `system` segment and
    /// a non-zero `model_limit` (bridge::start seeds 262_144).
    #[test]
    fn context_get_returns_snapshot_over_ipc() {
        let app = app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "ctx", Default::default())
            .build()
            .unwrap();
        let res = tauri::test::get_ipc_response(
            &webview,
            tauri::webview::InvokeRequest {
                cmd: "context_get".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::default(),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_string(),
            },
        );
        assert!(res.is_ok(), "context_get should resolve: {res:?}");
        let snap = res.unwrap().deserialize::<ContextSnapshot>().unwrap();
        assert!(snap.model_limit > 0, "model_limit should be seeded by bridge::start");
        assert!(
            snap.segments.iter().any(|s| s.category == "system"),
            "snapshot must contain a system segment"
        );
    }
}
