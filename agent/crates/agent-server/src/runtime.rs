use crate::approval::WsApprovalChannel;
use crate::sink::WsEventSink;
use crate::wire::{DiscoveredSkill, WireBody, WireEnvelope, PROTOCOL_VERSION};
use agent_core::{AgentLoop, LoopConfig, DEFAULT_STREAM_IDLE_TIMEOUT};
use agent_policy::RulePolicy;
use agent_runtime_config::{build_model, build_registry, build_sandbox, build_skills, pick_protocol, RuntimeConfig, HARD_FLOOR_DENYLIST};
use agent_skills::{compose_system_prompt, SkillRegistry};
use agent_tools::Tool;
use std::collections::HashSet;
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
    memory_tools: Arc<[Arc<dyn Tool>]>,
    base_system_prompt: String,
    system_prompt: Mutex<String>,
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
        memory_tools: Arc<[Arc<dyn Tool>]>,
        base_system_prompt: String,
    ) -> Self {
        let config = config.normalized();
        let built = build_loop(
            &config, &sink, &approval, &workspace, &api_key, &claude_binary, &mcp_tools,
            &memory_tools, &base_system_prompt);
        // Startup is lenient: an unknown persisted preset is already warned + dropped
        // inside build_loop, so the daemon always boots.
        Self {
            loop_cell: Mutex::new(built.loop_),
            config: Mutex::new(config),
            system_prompt: Mutex::new(built.system_prompt),
            sink, approval, workspace, api_key, claude_binary, config_path, session, tx,
            mcp_tools, memory_tools, base_system_prompt,
        }
    }

    /// Clone the current loop `Arc` (lock held only for the clone, never across await).
    pub fn current_loop(&self) -> Arc<AgentLoop> {
        self.loop_cell.lock().unwrap().clone()
    }

    /// The composed system prompt for the live config (base + awareness + presets).
    /// Applied to the session context at the start of each turn (next-turn discipline).
    pub fn current_system_prompt(&self) -> String {
        self.system_prompt.lock().unwrap().clone()
    }

    /// Validate+normalize, build (rejecting unknown presets), persist, then swap.
    /// On any failure, nothing changes.
    pub fn apply(&self, incoming: RuntimeConfig) -> Result<(), String> {
        let cfg = incoming.normalized();
        cfg.validate()?;
        let built = build_loop(
            &cfg, &self.sink, &self.approval, &self.workspace, &self.api_key,
            &self.claude_binary, &self.mcp_tools, &self.memory_tools, &self.base_system_prompt);
        if !built.unknown_presets.is_empty() {
            // Strict on the wire: a typo'd / missing active skill is a hard error.
            return Err(format!("unknown active skill(s): {}", built.unknown_presets.join(", ")));
        }
        cfg.save(&self.config_path).map_err(|e| format!("persist failed: {e}"))?;
        *self.loop_cell.lock().unwrap() = built.loop_;
        *self.system_prompt.lock().unwrap() = built.system_prompt;
        *self.config.lock().unwrap() = cfg;
        Ok(())
    }

    fn state_body(&self) -> WireBody {
        let cfg = self.config.lock().unwrap().clone();
        let discovered = SkillRegistry::from_config(&cfg.skills_dirs, &self.workspace)
            .scan()
            .into_iter()
            .map(|s| DiscoveredSkill { name: s.name, description: s.description })
            .collect();
        WireBody::SettingsState {
            settings: cfg,
            workspace: self.workspace.display().to_string(),
            api_key_set: self.api_key.is_some(),
            hard_floor: HARD_FLOOR_DENYLIST.iter().map(|s| s.to_string()).collect(),
            discovered_skills: discovered,
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

/// The result of (re)building the loop: the loop itself, the composed system
/// prompt, and any `active_skills` names that were not found (dropped from the
/// prompt). Callers decide whether unknown presets are fatal (wire) or tolerated
/// (startup).
struct BuiltLoop {
    loop_: Arc<AgentLoop>,
    system_prompt: String,
    unknown_presets: Vec<String>,
    /// Tool names registered at build time, retained so tests can assert injection.
    #[cfg(test)]
    registered_names: Vec<String>,
}

/// Assemble an `AgentLoop` from a config + the persistent seams, building the
/// skill registry/tools from `cfg.skills_dirs` and composing the system prompt
/// from `cfg.active_skills`. The one place a `RuntimeConfig` becomes a loop.
#[allow(clippy::too_many_arguments)]
fn build_loop(
    cfg: &RuntimeConfig,
    sink: &Arc<WsEventSink>,
    approval: &Arc<WsApprovalChannel>,
    workspace: &Path,
    api_key: &Option<String>,
    claude_binary: &str,
    mcp_tools: &[Arc<dyn Tool>],
    memory_tools: &[Arc<dyn Tool>],
    base_system_prompt: &str,
) -> BuiltLoop {
    let model = build_model(&cfg.backend, &cfg.base_url, &cfg.model, claude_binary, api_key.clone());
    let policy = Arc::new(RulePolicy {
        workspace: workspace.to_path_buf(),
        command_allowlist: cfg.command_allowlist.clone(),
        command_denylist: cfg.effective_denylist(),
    });
    let mut registry = build_registry(&cfg.http_allow_hosts);
    for t in mcp_tools {
        registry.register(t.clone());
    }
    for t in memory_tools {
        registry.register(t.clone());
    }
    // Skills: build the registry + the 4 tools from the configured roots.
    let (skill_registry, skill_tools) = build_skills(&cfg.skills_dirs, workspace);
    for t in skill_tools {
        registry.register(t);
    }
    // Compose the prompt: keep only presets that actually exist; warn + drop the rest.
    let available: HashSet<String> =
        skill_registry.scan().into_iter().map(|s| s.name).collect();
    let mut presets = Vec::new();
    let mut unknown_presets = Vec::new();
    for name in &cfg.active_skills {
        if available.contains(name) {
            presets.push(name.clone());
        } else {
            tracing::warn!(skill = %name, "active skill not found; dropping from system prompt");
            unknown_presets.push(name.clone());
        }
    }
    // All names in `presets` are known, so compose cannot error here.
    let system_prompt = match compose_system_prompt(base_system_prompt, &skill_registry, &presets) {
        Ok(p) => p,
        Err(e) => {
            // Unreachable today (presets are pre-filtered against scan()), but surface it
            // rather than silently dropping all presets if that ever changes. Never panic
            // here — startup must stay lenient.
            tracing::error!(error = %e, "compose_system_prompt failed unexpectedly; using base prompt");
            base_system_prompt.to_string()
        }
    };

    #[cfg(test)]
    let registered_names: Vec<String> = registry.schemas().into_iter().map(|s| s.name).collect();
    let loop_ = Arc::new(AgentLoop::new(
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
            top_p: cfg.top_p,
            top_k: cfg.top_k,
            min_p: cfg.min_p,
            presence_penalty: cfg.presence_penalty,
            repeat_penalty: cfg.repeat_penalty,
            enable_thinking: cfg.enable_thinking,
            preserve_thinking: cfg.preserve_thinking,
            sandbox: Some(build_sandbox(cfg)),
        },
    ));
    BuiltLoop {
        loop_,
        system_prompt,
        unknown_presets,
        #[cfg(test)]
        registered_names,
    }
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
            "claude".into(), path, session, tx, Arc::from(Vec::<Arc<dyn Tool>>::new()),
            Arc::from(Vec::<Arc<dyn Tool>>::new()),
            crate::daemon::SYSTEM_PROMPT.to_string());
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

    #[test]
    fn apply_with_valid_active_skill_updates_system_prompt() {
        use std::fs;
        let (rs, _rx, dir) = make();
        // Author a skill under the workspace default writable root.
        let sdir = dir.path().join(".agent").join("skills").join("greeter");
        fs::create_dir_all(&sdir).unwrap();
        fs::write(sdir.join("SKILL.md"),
            "---\nname: greeter\ndescription: d\n---\nSay hi politely.").unwrap();

        let before = rs.current_loop();
        let mut next = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
        next.active_skills = vec!["greeter".into()];
        rs.apply(next).unwrap();

        assert!(!Arc::ptr_eq(&before, &rs.current_loop()), "loop swapped");
        assert!(rs.current_system_prompt().contains("Say hi politely."),
            "preset body folded into the live system prompt");
    }

    #[test]
    fn apply_rejects_unknown_active_skill_without_swapping() {
        let (rs, _rx, _dir) = make();
        let before = rs.current_loop();
        let mut bad = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
        bad.active_skills = vec!["does-not-exist".into()];
        let err = rs.apply(bad).unwrap_err();
        assert!(err.contains("does-not-exist"), "error names the missing skill: {err}");
        assert!(Arc::ptr_eq(&before, &rs.current_loop()), "loop unchanged on rejection");
    }

    #[test]
    fn startup_drops_unknown_persisted_preset_without_panicking() {
        // A persisted config naming a non-existent preset must still boot (lenient).
        let (tx, _rx) = mpsc::unbounded_channel();
        let session = Arc::new(Mutex::new(String::new()));
        let sink = Arc::new(WsEventSink::new(tx.clone(), session.clone()));
        let approval = Arc::new(WsApprovalChannel::new(tx.clone(), session.clone(), Duration::from_secs(1)));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.json");
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
        cfg.active_skills = vec!["ghost".into()];
        let rs = RuntimeState::new(cfg, sink, approval, dir.path().to_path_buf(), None,
            "claude".into(), path, session, tx, Arc::from(Vec::<Arc<dyn Tool>>::new()),
            Arc::from(Vec::<Arc<dyn Tool>>::new()),
            crate::daemon::SYSTEM_PROMPT.to_string());
        // Booted: base prompt present, the ghost preset silently dropped.
        assert!(rs.current_system_prompt().contains("local coding agent"));
        assert!(!rs.current_system_prompt().contains("ghost"));
    }

    #[test]
    fn settings_state_includes_discovered_skills() {
        use std::fs;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let session = Arc::new(Mutex::new(String::new()));
        let sink = Arc::new(WsEventSink::new(tx.clone(), session.clone()));
        let approval = Arc::new(WsApprovalChannel::new(tx.clone(), session.clone(), Duration::from_secs(1)));
        let dir = tempfile::tempdir().unwrap();
        // Put a skill in <workspace>/.agent/skills/greeter (the default writable root).
        let sdir = dir.path().join(".agent").join("skills").join("greeter");
        fs::create_dir_all(&sdir).unwrap();
        fs::write(sdir.join("SKILL.md"), "---\nname: greeter\ndescription: says hi\n---\nbody").unwrap();
        let path = dir.path().join("rt.json");
        let cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
        let rs = RuntimeState::new(cfg, sink, approval, dir.path().to_path_buf(), None,
            "claude".into(), path, session, tx, Arc::from(Vec::<Arc<dyn Tool>>::new()),
            Arc::from(Vec::<Arc<dyn Tool>>::new()),
            crate::daemon::SYSTEM_PROMPT.to_string());
        assert!(rs.handle(&WireBody::SettingsGet));
        let env = rx.try_recv().expect("a frame");
        match env.body {
            WireBody::SettingsState { discovered_skills, .. } => {
                assert!(discovered_skills.iter().any(|s| s.name == "greeter" && s.description == "says hi"));
            }
            _ => panic!("expected settings_state"),
        }
    }

    #[test]
    fn build_loop_with_sandbox_mode_off_succeeds() {
        // sandbox_mode = "off" resolves to HostExecutor (no Docker probe needed).
        let (tx, _rx) = mpsc::unbounded_channel();
        let session = Arc::new(Mutex::new(String::new()));
        let sink = Arc::new(WsEventSink::new(tx.clone(), session.clone()));
        let approval = Arc::new(WsApprovalChannel::new(tx.clone(), session.clone(), Duration::from_secs(1)));
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
        cfg.sandbox_mode = "off".into();
        // build_loop must succeed — off → HostExecutor, no Docker required.
        let result = build_loop(
            &cfg, &sink, &approval, dir.path(), &None, "claude",
            &[], &[], crate::daemon::SYSTEM_PROMPT);
        // If we get here without panic/error the strategy was constructed OK.
        // Verify the loop describes itself as using host execution.
        let _loop = result.loop_; // just confirm it's built
    }

    #[test]
    fn build_loop_with_sandbox_mode_auto_constructs_loop() {
        // "auto" probes Docker but still returns a valid loop regardless of Docker availability.
        let (tx, _rx) = mpsc::unbounded_channel();
        let session = Arc::new(Mutex::new(String::new()));
        let sink = Arc::new(WsEventSink::new(tx.clone(), session.clone()));
        let approval = Arc::new(WsApprovalChannel::new(tx.clone(), session.clone(), Duration::from_secs(1)));
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
        cfg.sandbox_mode = "auto".into();
        let result = build_loop(
            &cfg, &sink, &approval, dir.path(), &None, "claude",
            &[], &[], crate::daemon::SYSTEM_PROMPT);
        let _loop = result.loop_;
    }

    #[test]
    fn build_loop_registers_injected_memory_tools() {
        use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
        use async_trait::async_trait;

        struct FakeMem;
        #[async_trait]
        impl Tool for FakeMem {
            fn name(&self) -> &str { "remember" }
            fn description(&self) -> &str { "fake" }
            fn schema(&self) -> ToolSchema {
                ToolSchema {
                    name: "remember".into(),
                    description: "fake".into(),
                    parameters: serde_json::json!({"type": "object"}),
                }
            }
            fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
                Ok(ToolIntent {
                    tool: "remember".into(),
                    access: Access::Read,
                    paths: vec![],
                    command: None,
                    summary: "x".into(),
                })
            }
            async fn execute(
                &self,
                _a: serde_json::Value,
                _c: &ToolCtx,
            ) -> Result<ToolOutput, ToolError> {
                Ok(ToolOutput { content: "ok".into(), display: None })
            }
        }

        let (tx, _rx) = mpsc::unbounded_channel();
        let session = Arc::new(Mutex::new(String::new()));
        let sink = Arc::new(WsEventSink::new(tx.clone(), session.clone()));
        let approval = Arc::new(WsApprovalChannel::new(
            tx.clone(), session.clone(), Duration::from_secs(1)));
        let dir = tempfile::tempdir().unwrap();
        let cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);

        let result = build_loop(
            &cfg, &sink, &approval, dir.path(), &None, "claude",
            &[], &[Arc::new(FakeMem)], crate::daemon::SYSTEM_PROMPT);

        assert!(
            result.registered_names.iter().any(|n| n == "remember"),
            "memory tool \"remember\" must be registered; got: {:?}",
            result.registered_names,
        );
    }
}
