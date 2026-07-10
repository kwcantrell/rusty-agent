pub mod bridge;
mod channel_out;
pub mod devserver;
pub mod llama;
pub mod workspace;

use agent_runtime_config::RuntimeConfig;
use agent_server::session::{SendOutcome, Session};
use agent_server::wire::{ArchitectureSnapshot, ContextSnapshot, Decision, ServerEvent, SettingsState};
use channel_out::ChannelOut;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

struct AppState {
    bridge: Arc<bridge::Bridge>,
    config_path: PathBuf, // app.json (persisted workspace)
    dev: devserver::DevServerManager,
}

fn session(state: &tauri::State<'_, AppState>) -> Arc<Session> {
    state.bridge.session()
}

/// Register/replace the outbound event channel for this webview.
///
/// MUST be `async`: `Session::set_event_out` calls `tokio::spawn` (parked-run
/// re-emit), which panics ("there is no reactor running") when invoked from a
/// SYNC command — sync commands run on the WebKitGTK/glib main thread with no
/// Tokio runtime entered, and that panic aborts across the C-FFI boundary
/// ("non-unwinding panic"). An `async` command runs on Tauri's managed Tokio
/// runtime, so the inner `tokio::spawn` has a reactor.
#[tauri::command]
async fn subscribe(state: tauri::State<'_, AppState>, channel: Channel<ServerEvent>) -> Result<(), String> {
    session(&state).set_event_out(Arc::new(ChannelOut(channel)));
    Ok(())
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
fn architecture_get(state: tauri::State<'_, AppState>) -> ArchitectureSnapshot {
    session(&state).architecture()
}

#[tauri::command]
async fn dev_scripts_detect(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<devserver::DevScriptCandidate>, String> {
    let ws = state.bridge.current_workspace().await;
    Ok(devserver::detect(&ws))
}

#[tauri::command]
async fn dev_server_start(
    state: tauri::State<'_, AppState>,
    candidate: devserver::DevScriptCandidate,
) -> Result<devserver::DevServerStatus, String> {
    let ws = state.bridge.current_workspace().await;
    state.dev.start(candidate, &ws).await
}

#[tauri::command]
fn dev_server_stop(state: tauri::State<'_, AppState>) {
    state.dev.stop();
}

#[tauri::command]
fn dev_server_status(state: tauri::State<'_, AppState>) -> Option<devserver::DevServerStatus> {
    state.dev.status()
}

#[tauri::command]
fn session_stats(state: tauri::State<'_, AppState>) -> agent_core::SessionStats {
    session(&state).session_stats()
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
async fn pick_workspace<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    let folder = app.dialog().file().blocking_pick_folder();
    let Some(path) = folder else { return Ok(None) };
    let dir = path.into_path().map_err(|e| e.to_string())?;
    // Persist, then reconnect the runtime to the new dir.
    let cfg = workspace::AppConfig { workspace: Some(dir.clone()) };
    cfg.save(&state.config_path).map_err(|e| e.to_string())?;
    state.dev.stop(); // old server pointed at the previous workspace
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

#[tauri::command]
async fn skill_get(state: tauri::State<'_, AppState>, name: String)
    -> Result<agent_server::session::SkillDto, String> {
    session(&state).skill_get(name).await
}

#[tauri::command]
async fn skill_save(state: tauri::State<'_, AppState>, name: String, body: String)
    -> Result<(), String> {
    session(&state).skill_save(name, body).await
}

/// Single source of truth for the Tauri command surface. Used by both the
/// production builder and the `#[cfg(test)]` mock app so the two lists cannot drift.
macro_rules! all_handlers {
    () => {
        tauri::generate_handler![
            subscribe,
            send_input,
            approve,
            cancel,
            settings_get,
            architecture_get,
            dev_scripts_detect,
            dev_server_start,
            dev_server_stop,
            dev_server_status,
            session_stats,
            settings_update,
            context_get,
            get_workspace,
            pick_workspace,
            llama_health,
            skill_get,
            skill_save
        ]
    };
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
            app.manage(AppState { bridge, config_path, dev: devserver::DevServerManager::new() });
            Ok(())
        })
        .invoke_handler(all_handlers!())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                window.state::<AppState>().dev.stop();
            }
        })
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
            .manage(AppState { bridge, config_path: PathBuf::from("/tmp/app.json"), dev: devserver::DevServerManager::new() })
            .invoke_handler(all_handlers!())
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

    /// Smoke test: `architecture_get` resolves over the mock IPC and the payload
    /// contains the seven architecture block keys: model, tools, policy, sandbox,
    /// context, loop, prompt.
    #[test]
    fn architecture_get_returns_snapshot_over_ipc() {
        let app = app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "arch", Default::default())
            .build()
            .unwrap();
        let res = tauri::test::get_ipc_response(
            &webview,
            tauri::webview::InvokeRequest {
                cmd: "architecture_get".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::default(),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_string(),
            },
        );
        assert!(res.is_ok(), "architecture_get should resolve: {res:?}");
        let v: serde_json::Value = res.unwrap().deserialize().unwrap();
        for key in ["model", "tools", "policy", "sandbox", "context", "loop", "prompt"] {
            assert!(v.get(key).is_some(), "missing block {key}: {v}");
        }
    }

    /// Smoke test: `dev_scripts_detect` resolves over the mock IPC to a JSON array
    /// (empty for the temp workspace, which has no package.json).
    #[test]
    fn dev_scripts_detect_returns_array_over_ipc() {
        let app = app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "dev", Default::default())
            .build()
            .unwrap();
        let res = tauri::test::get_ipc_response(
            &webview,
            tauri::webview::InvokeRequest {
                cmd: "dev_scripts_detect".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::default(),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_string(),
            },
        );
        assert!(res.is_ok(), "dev_scripts_detect should resolve: {res:?}");
        let v: serde_json::Value = res.unwrap().deserialize().unwrap();
        assert!(v.is_array(), "expected an array, got {v}");
    }

    /// The live-preview iframe (UrlArtifact, http://localhost:*) must be allowed
    /// by an explicit frame-src — with none, frames fall back to
    /// default-src 'self' and bundled builds block the Design canvas.
    #[test]
    fn csp_declares_frame_src_for_localhost_preview() {
        let conf = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json"))
            .expect("read tauri.conf.json");
        let v: serde_json::Value = serde_json::from_str(&conf).expect("parse tauri.conf.json");
        let csp = v["app"]["security"]["csp"].as_str().expect("csp string");
        let frame = csp.split(';').map(str::trim)
            .find(|d| d.starts_with("frame-src"))
            .expect("csp must declare an explicit frame-src");
        for src in ["'self'", "http://localhost:*", "http://127.0.0.1:*"] {
            assert!(frame.contains(src), "frame-src must allow {src}: {frame}");
        }
    }
}
