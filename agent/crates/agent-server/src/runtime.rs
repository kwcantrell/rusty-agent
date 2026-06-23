use crate::approval::WsApprovalChannel;
use crate::sink::WsEventSink;
use crate::wire::{WireBody, WireEnvelope, PROTOCOL_VERSION};
use agent_core::{AgentLoop, LoopConfig, DEFAULT_STREAM_IDLE_TIMEOUT};
use agent_policy::RulePolicy;
use agent_runtime_config::{build_model, build_registry, pick_protocol, RuntimeConfig, HARD_FLOOR_DENYLIST};
use agent_tools::Tool;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// Holds the live `AgentLoop` plus everything needed to rebuild it on a settings
/// change. The loop is swapped atomically; an in-flight run keeps the `Arc` it
/// already cloned, so it finishes on its old config (next-turn apply, no interrupt).
pub struct RuntimeState {
    loop_cell: Mutex<Arc<AgentLoop>>,
    config: Mutex<RuntimeConfig>,
    sink: Arc<WsEventSink>,
    approval: Arc<WsApprovalChannel>,
    workspace: PathBuf,
    api_key: Option<String>,
    claude_binary: String,
    config_path: PathBuf,
    session: Arc<Mutex<String>>,
    tx: mpsc::UnboundedSender<WireEnvelope>,
    mcp_tools: Arc<[Arc<dyn Tool>]>,
}

impl RuntimeState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: RuntimeConfig,
        sink: Arc<WsEventSink>,
        approval: Arc<WsApprovalChannel>,
        workspace: PathBuf,
        api_key: Option<String>,
        claude_binary: String,
        config_path: PathBuf,
        session: Arc<Mutex<String>>,
        tx: mpsc::UnboundedSender<WireEnvelope>,
        mcp_tools: Arc<[Arc<dyn Tool>]>,
    ) -> Self {
        let config = config.normalized();
        let initial = build_loop(&config, &sink, &approval, &workspace, &api_key, &claude_binary, &mcp_tools);
        Self {
            loop_cell: Mutex::new(initial),
            config: Mutex::new(config),
            sink, approval, workspace, api_key, claude_binary, config_path, session, tx, mcp_tools,
        }
    }

    /// Clone the current loop `Arc` (lock held only for the clone, never across await).
    pub fn current_loop(&self) -> Arc<AgentLoop> {
        self.loop_cell.lock().unwrap().clone()
    }

    /// Validate+normalize, persist, then swap. On any failure, nothing changes.
    pub fn apply(&self, incoming: RuntimeConfig) -> Result<(), String> {
        let cfg = incoming.normalized();
        cfg.validate()?;
        cfg.save(&self.config_path).map_err(|e| format!("persist failed: {e}"))?;
        let new_loop = build_loop(
            &cfg, &self.sink, &self.approval, &self.workspace, &self.api_key, &self.claude_binary,
            &self.mcp_tools);
        *self.loop_cell.lock().unwrap() = new_loop;
        *self.config.lock().unwrap() = cfg;
        Ok(())
    }

    fn state_body(&self) -> WireBody {
        WireBody::SettingsState {
            settings: self.config.lock().unwrap().clone(),
            workspace: self.workspace.display().to_string(),
            api_key_set: self.api_key.is_some(),
            hard_floor: HARD_FLOOR_DENYLIST.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn send(&self, body: WireBody) {
        let env = WireEnvelope {
            v: PROTOCOL_VERSION,
            session_id: self.session.lock().unwrap().clone(),
            id: None,
            body,
        };
        let _ = self.tx.send(env);
    }

    /// Dispatch a settings_* frame. Returns true if it was handled (a settings frame).
    pub fn handle(&self, body: &WireBody) -> bool {
        match body {
            WireBody::SettingsGet => {
                let s = self.state_body();
                self.send(s);
                true
            }
            WireBody::SettingsUpdate { settings } => {
                match self.apply(settings.clone()) {
                    Ok(()) => {
                        let s = self.state_body();
                        self.send(s);
                    }
                    Err(message) => self.send(WireBody::SettingsError { message }),
                }
                true
            }
            _ => false,
        }
    }
}

/// Assemble an `AgentLoop` from a config + the persistent seams. The one place that
/// turns a `RuntimeConfig` into a loop (initial build + every reconfigure).
fn build_loop(
    cfg: &RuntimeConfig,
    sink: &Arc<WsEventSink>,
    approval: &Arc<WsApprovalChannel>,
    workspace: &Path,
    api_key: &Option<String>,
    claude_binary: &str,
    mcp_tools: &[Arc<dyn Tool>],
) -> Arc<AgentLoop> {
    let model = build_model(&cfg.backend, &cfg.base_url, &cfg.model, claude_binary, api_key.clone());
    let policy = Arc::new(RulePolicy {
        workspace: workspace.to_path_buf(),
        command_allowlist: cfg.command_allowlist.clone(),
        command_denylist: cfg.effective_denylist(),
    });
    let mut registry = build_registry();
    for t in mcp_tools {
        registry.register(t.clone());
    }
    Arc::new(AgentLoop::new(
        model,
        pick_protocol(&cfg.protocol),
        Arc::new(registry),
        policy,
        approval.clone(),
        sink.clone(),
        LoopConfig {
            model_limit: cfg.context_limit,
            max_turns: cfg.max_turns,
            max_retries: 3,
            temperature: cfg.temperature,
            max_tokens: Some(cfg.max_tokens),
            workspace: workspace.to_path_buf(),
            tool_timeout: Duration::from_secs(120),
            stream_idle_timeout: DEFAULT_STREAM_IDLE_TIMEOUT,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::WsApprovalChannel;
    use crate::sink::WsEventSink;
    use crate::wire::WireBody;
    use agent_runtime_config::RuntimeConfig;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::sync::mpsc;

    fn make() -> (RuntimeState, mpsc::UnboundedReceiver<crate::wire::WireEnvelope>, tempfile::TempDir) {
        let (tx, rx) = mpsc::unbounded_channel();
        let session = Arc::new(Mutex::new(String::new()));
        let sink = Arc::new(WsEventSink::new(tx.clone(), session.clone()));
        let approval = Arc::new(WsApprovalChannel::new(tx.clone(), session.clone(), Duration::from_secs(1)));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.json");
        let cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
        let rs = RuntimeState::new(cfg, sink, approval, dir.path().to_path_buf(), None,
            "claude".into(), path, session, tx, Arc::from(Vec::<Arc<dyn Tool>>::new()));
        (rs, rx, dir)
    }

    #[test]
    fn apply_swaps_the_loop_and_persists() {
        let (rs, _rx, dir) = make();
        let before = rs.current_loop();
        let mut next = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m2".into(), "native".into(), 8192);
        next.temperature = 0.9;
        rs.apply(next).unwrap();
        let after = rs.current_loop();
        assert!(!Arc::ptr_eq(&before, &after), "loop should be a new Arc");
        assert!(dir.path().join("rt.json").exists(), "config persisted");
    }

    #[test]
    fn apply_rejects_invalid_without_swapping() {
        let (rs, _rx, _dir) = make();
        let before = rs.current_loop();
        let mut bad = RuntimeConfig::from_launch(
            "openai".into(), "   ".into(), "m".into(), "native".into(), 8192); // empty base_url
        bad.base_url = "  ".into();
        let err = rs.apply(bad).unwrap_err();
        assert!(err.contains("base_url"));
        assert!(Arc::ptr_eq(&before, &rs.current_loop()), "loop unchanged on rejection");
    }

    #[test]
    fn handle_settings_get_emits_state() {
        let (rs, mut rx, _dir) = make();
        assert!(rs.handle(&WireBody::SettingsGet));
        let env = rx.try_recv().expect("a frame");
        match env.body {
            WireBody::SettingsState { api_key_set, hard_floor, .. } => {
                assert!(!api_key_set);
                assert!(hard_floor.iter().any(|d| d == "sudo"));
            }
            _ => panic!("expected settings_state"),
        }
    }

    #[test]
    fn handle_invalid_update_emits_error() {
        let (rs, mut rx, _dir) = make();
        let mut bad = RuntimeConfig::from_launch(
            "openai".into(), "".into(), "m".into(), "native".into(), 8192);
        bad.base_url = "".into();
        assert!(rs.handle(&WireBody::SettingsUpdate { settings: bad }));
        let env = rx.try_recv().expect("a frame");
        assert!(matches!(env.body, WireBody::SettingsError { .. }));
    }

    #[test]
    fn handle_ignores_non_settings_frames() {
        let (rs, _rx, _dir) = make();
        assert!(!rs.handle(&WireBody::UserInput { text: "hi".into() }));
    }
}
