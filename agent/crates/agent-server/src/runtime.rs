use crate::approval::IpcApprovalChannel;
use crate::sink::ChannelEventSink;
use crate::wire::{DiscoveredSkill, SettingsState};
use agent_core::{AgentLoop, OffloadStore, Retriever, DEFAULT_STREAM_IDLE_TIMEOUT};
use agent_runtime_config::{assemble_loop, build_model, BuiltLoop, LoopParts, RuntimeConfig, HARD_FLOOR_DENYLIST};
use agent_skills::SkillRegistry;
use agent_tools::Tool;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// Holds the live `AgentLoop` plus everything needed to rebuild it on a settings
/// change. The loop is swapped atomically; an in-flight run keeps the `Arc` it
/// already cloned, so it finishes on its old config (next-turn apply, no interrupt).
pub struct RuntimeState {
    loop_cell: Mutex<Arc<AgentLoop>>,
    config: Mutex<RuntimeConfig>,
    sink: Arc<ChannelEventSink>,
    approval: Arc<IpcApprovalChannel>,
    workspace: PathBuf,
    api_key: Option<String>,
    claude_binary: String,
    config_path: PathBuf,
    mcp_tools: Arc<[Arc<dyn Tool>]>,
    memory_tools: Arc<[Arc<dyn Tool>]>,
    memory_retriever: Option<Arc<dyn Retriever>>,
    base_system_prompt: String,
    system_prompt: Mutex<String>,
    /// Conversation-stable context-management handles. Reused across loop rebuilds
    /// so a settings change never orphans the offload table from its tools.
    offload_store: Arc<dyn OffloadStore>,
    compact_flag: Arc<AtomicBool>,
}

impl RuntimeState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: RuntimeConfig,
        sink: Arc<ChannelEventSink>,
        approval: Arc<IpcApprovalChannel>,
        workspace: PathBuf,
        api_key: Option<String>,
        claude_binary: String,
        config_path: PathBuf,
        mcp_tools: Arc<[Arc<dyn Tool>]>,
        memory_tools: Arc<[Arc<dyn Tool>]>,
        memory_retriever: Option<Arc<dyn Retriever>>,
        base_system_prompt: String,
    ) -> Self {
        let config = config.normalized();
        let offload_store: Arc<dyn OffloadStore> =
            Arc::new(agent_core::InMemoryOffloadStore::new());
        let compact_flag = Arc::new(AtomicBool::new(false));
        let built = build_loop(
            &config, &sink, &approval, &workspace, &api_key, &claude_binary, &mcp_tools,
            &memory_tools, memory_retriever.as_ref(), &base_system_prompt,
            &offload_store, &compact_flag);
        // Startup is lenient: an unknown persisted preset is already warned + dropped
        // inside build_loop, so the daemon always boots.
        Self {
            loop_cell: Mutex::new(built.loop_),
            config: Mutex::new(config),
            system_prompt: Mutex::new(built.system_prompt),
            sink, approval, workspace, api_key, claude_binary, config_path,
            mcp_tools, memory_tools, memory_retriever, base_system_prompt,
            offload_store, compact_flag,
        }
    }

    /// Conversation-stable offload table (shared with the session's `CuratedContext`).
    pub fn offload_store(&self) -> Arc<dyn OffloadStore> {
        self.offload_store.clone()
    }

    /// Conversation-stable compaction-request flag (shared with the session's context).
    pub fn compact_flag(&self) -> Arc<AtomicBool> {
        self.compact_flag.clone()
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
            &self.claude_binary, &self.mcp_tools, &self.memory_tools,
            self.memory_retriever.as_ref(), &self.base_system_prompt,
            &self.offload_store, &self.compact_flag);
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

    pub fn settings_state(&self) -> SettingsState {
        let cfg = self.config.lock().unwrap().clone();
        let discovered = SkillRegistry::from_config(&cfg.skills_dirs, &self.workspace)
            .scan()
            .into_iter()
            .map(|s| DiscoveredSkill { name: s.name, description: s.description })
            .collect();
        let sandbox_degraded = crate::wire::sandbox_degraded_from(
            self.current_loop().sandbox_descriptor());
        SettingsState {
            settings: cfg,
            workspace: self.workspace.display().to_string(),
            api_key_set: self.api_key.is_some(),
            hard_floor: HARD_FLOOR_DENYLIST.iter().map(|s| s.to_string()).collect(),
            discovered_skills: discovered,
            sandbox_degraded,
        }
    }
}

/// Build the loop for the current config. Thin wrapper over the shared
/// `agent_runtime_config::assemble_loop`; this crate supplies the per-frontend
/// parts (WebSocket sink/approval, the model, injected tools). The one place a
/// `RuntimeConfig` becomes a loop now lives in `agent-runtime-config`.
#[allow(clippy::too_many_arguments)]
fn build_loop(
    cfg: &RuntimeConfig,
    sink: &Arc<ChannelEventSink>,
    approval: &Arc<IpcApprovalChannel>,
    workspace: &Path,
    api_key: &Option<String>,
    claude_binary: &str,
    mcp_tools: &[Arc<dyn Tool>],
    memory_tools: &[Arc<dyn Tool>],
    memory_retriever: Option<&Arc<dyn Retriever>>,
    base_system_prompt: &str,
    offload_store: &Arc<dyn OffloadStore>,
    compact_flag: &Arc<AtomicBool>,
) -> BuiltLoop {
    let model = build_model(&cfg.backend, &cfg.base_url, &cfg.model, claude_binary, api_key.clone());
    assemble_loop(cfg, LoopParts {
        model,
        sink: sink.clone(),
        approval: approval.clone(),
        workspace: workspace.to_path_buf(),
        mcp_tools: mcp_tools.to_vec(),
        memory_tools: memory_tools.to_vec(),
        memory_retriever: memory_retriever.cloned(),
        stream_idle_timeout: DEFAULT_STREAM_IDLE_TIMEOUT,
        base_system_prompt: base_system_prompt.to_string(),
        offload_store: offload_store.clone(),
        compact_flag: compact_flag.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_config::RuntimeConfig;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn slot() -> crate::sink::EventSlot { Arc::new(Mutex::new(None)) }

    fn parts() -> (Arc<ChannelEventSink>, Arc<IpcApprovalChannel>) {
        let s = slot();
        let sink = Arc::new(ChannelEventSink::new(s.clone()));
        let approval = Arc::new(IpcApprovalChannel::new(s, Duration::from_secs(1)));
        (sink, approval)
    }

    fn make() -> (RuntimeState, tempfile::TempDir) {
        let (sink, approval) = parts();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.json");
        let cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
        let rs = RuntimeState::new(cfg, sink, approval, dir.path().to_path_buf(), None,
            "claude".into(), path, Arc::from(Vec::<Arc<dyn Tool>>::new()),
            Arc::from(Vec::<Arc<dyn Tool>>::new()), None,
            crate::daemon::SYSTEM_PROMPT.to_string());
        (rs, dir)
    }

    #[test]
    fn apply_swaps_the_loop_and_persists() {
        let (rs, dir) = make();
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
        let (rs, _dir) = make();
        let before = rs.current_loop();
        let mut bad = RuntimeConfig::from_launch(
            "openai".into(), "   ".into(), "m".into(), "native".into(), 8192); // empty base_url
        bad.base_url = "  ".into();
        let err = rs.apply(bad).unwrap_err();
        assert!(err.contains("base_url"));
        assert!(Arc::ptr_eq(&before, &rs.current_loop()), "loop unchanged on rejection");
    }

    #[test]
    fn settings_state_reports_floor_and_no_api_key() {
        let (rs, _dir) = make();
        let st = rs.settings_state();
        assert!(!st.api_key_set);
        assert!(st.hard_floor.iter().any(|d| d == "sudo"));
    }

    #[test]
    fn apply_with_valid_active_skill_updates_system_prompt() {
        use std::fs;
        let (rs, dir) = make();
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
        let (rs, _dir) = make();
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
        let (sink, approval) = parts();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.json");
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
        cfg.active_skills = vec!["ghost".into()];
        let rs = RuntimeState::new(cfg, sink, approval, dir.path().to_path_buf(), None,
            "claude".into(), path, Arc::from(Vec::<Arc<dyn Tool>>::new()),
            Arc::from(Vec::<Arc<dyn Tool>>::new()), None,
            crate::daemon::SYSTEM_PROMPT.to_string());
        // Booted: base prompt present, the ghost preset silently dropped.
        assert!(rs.current_system_prompt().contains("local coding agent"));
        assert!(!rs.current_system_prompt().contains("ghost"));
    }

    #[test]
    fn settings_state_includes_discovered_skills() {
        use std::fs;
        let (sink, approval) = parts();
        let dir = tempfile::tempdir().unwrap();
        // Put a skill in <workspace>/.agent/skills/greeter (the default writable root).
        let sdir = dir.path().join(".agent").join("skills").join("greeter");
        fs::create_dir_all(&sdir).unwrap();
        fs::write(sdir.join("SKILL.md"), "---\nname: greeter\ndescription: says hi\n---\nbody").unwrap();
        let path = dir.path().join("rt.json");
        let cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
        let rs = RuntimeState::new(cfg, sink, approval, dir.path().to_path_buf(), None,
            "claude".into(), path, Arc::from(Vec::<Arc<dyn Tool>>::new()),
            Arc::from(Vec::<Arc<dyn Tool>>::new()), None,
            crate::daemon::SYSTEM_PROMPT.to_string());
        let st = rs.settings_state();
        assert!(st.discovered_skills.iter().any(|s| s.name == "greeter" && s.description == "says hi"));
    }

    #[test]
    fn build_loop_with_sandbox_mode_off_succeeds() {
        // sandbox_mode = "off" resolves to HostExecutor (no Docker probe needed).
        let (sink, approval) = parts();
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
        cfg.sandbox_mode = "off".into();
        // build_loop must succeed — off → HostExecutor, no Docker required.
        let store: Arc<dyn OffloadStore> = Arc::new(agent_core::InMemoryOffloadStore::new());
        let flag = Arc::new(AtomicBool::new(false));
        let result = build_loop(
            &cfg, &sink, &approval, dir.path(), &None, "claude",
            &[], &[], None, crate::daemon::SYSTEM_PROMPT, &store, &flag);
        // If we get here without panic/error the strategy was constructed OK.
        let _loop = result.loop_; // just confirm it's built
    }

    #[test]
    fn build_loop_with_sandbox_mode_auto_constructs_loop() {
        // "auto" probes Docker but still returns a valid loop regardless of Docker availability.
        let (sink, approval) = parts();
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
        cfg.sandbox_mode = "auto".into();
        let store: Arc<dyn OffloadStore> = Arc::new(agent_core::InMemoryOffloadStore::new());
        let flag = Arc::new(AtomicBool::new(false));
        let result = build_loop(
            &cfg, &sink, &approval, dir.path(), &None, "claude",
            &[], &[], None, crate::daemon::SYSTEM_PROMPT, &store, &flag);
        let _loop = result.loop_;
    }

}
