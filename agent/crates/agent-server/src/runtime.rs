use crate::approval::IpcApprovalChannel;
use crate::sink::ChannelEventSink;
use crate::wire::{
    redact_base_url, ArchitectureSnapshot, ContextInfo, DiscoveredSkill, LoopInfo, ModelInfo,
    PolicyInfo, PromptInfo, SandboxInfo, SettingsState, ToolEntry,
};
use agent_core::{estimate_tokens, AgentLoop, SessionArtifacts, DEFAULT_STREAM_IDLE_TIMEOUT};
use agent_runtime_config::{
    assemble_loop, build_model, build_sandbox, claude_cli_opts, BuiltLoop, LoopParts,
    RuntimeConfig, HARD_FLOOR_DENYLIST,
};
use agent_skills::SkillRegistry;
use agent_tools::Tool;
use std::collections::HashSet;
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
    base_system_prompt: String,
    system_prompt: Mutex<String>,
    /// Conversation-stable context-management handles. Reused across loop rebuilds
    /// so a settings change never orphans the artifact stores from their tools.
    artifacts: Arc<SessionArtifacts>,
    compact_flag: Arc<AtomicBool>,
    /// Conversation-stable plan list (shared with the session's `CuratedContext`,
    /// the `compact_flag` shape). Reused across loop rebuilds so a settings
    /// change never orphans the plan from `write_todos` (spec §5.4/§5.6).
    todos: agent_core::TodoHandle,
    /// Session-stable observability handles. Created ONCE here and reused across
    /// every loop rebuild — a per-rebuild TraceWriter would interleave two writers into one file.
    stats: Arc<std::sync::RwLock<agent_core::SessionStats>>,
    trace: Option<Arc<agent_runtime_config::TraceWriter>>,
    /// Durable session identity (4B-0): minted once here, shared with the
    /// TraceWriter, recorded in sessions/<id>/descriptor.json. The
    /// descriptor — not the trace — is what a restarted daemon indexes.
    session_id: String,
    /// The session's root park-point checkpointer (4B-1). Built once with
    /// the durable id + daemon-local secret; None when HOME/secret is
    /// unavailable (checkpointing degrades to live-only approvals).
    checkpointer: Option<Arc<agent_core::Checkpointer>>,
    /// On-disk config content as of our last read/write. `apply` refuses to
    /// clobber a file some other process (CLI, editor) changed since.
    persisted_file: Mutex<Option<String>>,
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
        base_system_prompt: String,
    ) -> Self {
        let config = config.normalized();
        // Startup stays lenient (no validate() — an unknown preset is warned + dropped
        // in build_loop so the daemon always boots), but surface soft warnings so a
        // no-op-inducing config (e.g. a bash allowlist entry) is visible in the log.
        for w in config.warnings() {
            tracing::warn!(target: "config", "{w}");
        }
        let artifacts = Arc::new(SessionArtifacts::new());
        let compact_flag = Arc::new(AtomicBool::new(false));
        let todos: agent_core::TodoHandle = Arc::new(Mutex::new(Vec::new()));
        let stats: Arc<std::sync::RwLock<agent_core::SessionStats>> = Arc::default();
        let session_id = agent_runtime_config::mint_session_id();
        if let Some(root) = agent_runtime_config::sessions_root(&config) {
            let d = agent_runtime_config::SessionDescriptor {
                schema: agent_runtime_config::DESCRIPTOR_SCHEMA,
                session_id: session_id.clone(),
                workspace: workspace.clone(),
                created_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                config_path: Some(config_path.clone()),
            };
            if let Err(e) = agent_runtime_config::write_descriptor(&root, &d) {
                tracing::warn!(target: "session", error = %e,
                    "cannot write session descriptor; run will not be resumable");
            }
            agent_runtime_config::prune_session_dirs(&root, 50);
        }
        let trace = agent_runtime_config::build_trace(&config, &session_id);
        let checkpointer = agent_runtime_config::sessions_root(&config).and_then(|root| {
            let meta = agent_runtime_config::metadata_root()?;
            match agent_runtime_config::load_or_create_secret(&meta) {
                Ok(key) => Some(agent_core::Checkpointer::new(
                    agent_runtime_config::session_dir(&root, &session_id).join("checkpoint"),
                    key,
                    session_id.clone(),
                )),
                Err(e) => {
                    tracing::warn!(target: "session", error = %e,
                        "no daemon secret; approvals will not be durable");
                    None
                }
            }
        });
        let persisted_file = Mutex::new(std::fs::read_to_string(&config_path).ok());
        let built = build_loop(
            &config,
            &sink,
            &approval,
            &workspace,
            &api_key,
            &claude_binary,
            &mcp_tools,
            &base_system_prompt,
            &artifacts,
            &compact_flag,
            &todos,
            &stats,
            &trace,
            &checkpointer,
        );
        // Startup is lenient: an unknown persisted preset is already warned + dropped
        // inside build_loop, so the daemon always boots.
        Self {
            loop_cell: Mutex::new(built.loop_),
            config: Mutex::new(config),
            system_prompt: Mutex::new(built.system_prompt),
            sink,
            approval,
            workspace,
            api_key,
            claude_binary,
            config_path,
            mcp_tools,
            base_system_prompt,
            artifacts,
            compact_flag,
            todos,
            stats,
            trace,
            session_id,
            checkpointer,
            persisted_file,
        }
    }

    /// Session-stable stats handle (folded by the loop's ObservedSink; read by RPCs).
    pub fn stats(&self) -> Arc<std::sync::RwLock<agent_core::SessionStats>> {
        self.stats.clone()
    }

    /// Durable session identity (4B-0). Stable for the daemon's lifetime;
    /// a restarted daemon re-learns it from descriptor.json, not from us.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The session's root checkpointer (None when identity/secret
    /// unavailable). Shared by build_loop and the resume coordinator.
    pub fn checkpointer(&self) -> Option<Arc<agent_core::Checkpointer>> {
        self.checkpointer.clone()
    }

    /// Assemble a loop bound to ANOTHER session's workspace + checkpointer
    /// (attach-to-resume, spec §2.4 step 2): current config, fresh
    /// artifacts/todos/flag, shared sink/approval/stats/mcp tools.
    pub fn build_resume_loop(
        &self,
        workspace: &Path,
        checkpoint: Arc<agent_core::Checkpointer>,
        artifacts: &Arc<SessionArtifacts>,
        todos: &agent_core::TodoHandle,
        compact_flag: &Arc<AtomicBool>,
    ) -> BuiltLoop {
        let cfg = self.config.lock().unwrap().clone();
        build_loop(
            &cfg,
            &self.sink,
            &self.approval,
            workspace,
            &self.api_key,
            &self.claude_binary,
            &self.mcp_tools,
            &self.base_system_prompt,
            artifacts,
            compact_flag,
            todos,
            &self.stats,
            &self.trace,
            &Some(checkpoint),
        )
    }

    /// Rewrite descriptor.json with a new workspace (workspace switch).
    /// Resume must bind to the CURRENT workspace (spec §2.1/§3.3).
    pub fn rewrite_descriptor_workspace(&self, workspace: &std::path::Path) {
        let config = self.config.lock().unwrap().clone();
        let Some(root) = agent_runtime_config::sessions_root(&config) else {
            return;
        };
        let dir = agent_runtime_config::session_dir(&root, &self.session_id);
        let Some(mut d) = agent_runtime_config::load_descriptor(&dir) else {
            return; // construction never wrote one (warned there already)
        };
        d.workspace = workspace.to_path_buf();
        if let Err(e) = agent_runtime_config::write_descriptor(&root, &d) {
            tracing::warn!(target: "session", error = %e,
                "cannot rewrite session descriptor on workspace switch");
        }
    }

    /// Conversation-stable artifact stores (shared with the session's `CuratedContext`).
    pub fn artifacts(&self) -> Arc<SessionArtifacts> {
        self.artifacts.clone()
    }

    /// Conversation-stable compaction-request flag (shared with the session's context).
    pub fn compact_flag(&self) -> Arc<AtomicBool> {
        self.compact_flag.clone()
    }

    /// Conversation-stable plan list (shared with the session's context).
    pub fn todos(&self) -> agent_core::TodoHandle {
        self.todos.clone()
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
        {
            // Guard against external edits since our last read/write. A TOCTOU window exists:
            // an external edit landing between this check and the save below is clobbered (single-daemon semantics, accepted).
            let seen = self.persisted_file.lock().unwrap();
            let on_disk = std::fs::read_to_string(&self.config_path).ok();
            if on_disk != *seen {
                return Err(
                    "config file changed externally — restart the daemon to pick it up".into(),
                );
            }
        }
        let built = build_loop(
            &cfg,
            &self.sink,
            &self.approval,
            &self.workspace,
            &self.api_key,
            &self.claude_binary,
            &self.mcp_tools,
            &self.base_system_prompt,
            &self.artifacts,
            &self.compact_flag,
            &self.todos,
            &self.stats,
            &self.trace,
            &self.checkpointer,
        );
        if !built.unknown_presets.is_empty() {
            // Strict on the wire: a typo'd / missing active skill is a hard error.
            return Err(format!(
                "unknown active skill(s): {}",
                built.unknown_presets.join(", ")
            ));
        }
        cfg.save(&self.config_path)
            .map_err(|e| format!("persist failed: {e}"))?;
        // Warn only now that nothing can fail: a config rejected as validate-bad,
        // externally-changed, unknown-preset, or persist-failed never logs its soft
        // warnings (it was never accepted).
        for w in cfg.warnings() {
            tracing::warn!(target: "config", "{w}");
        }
        // Record what save() just wrote. If the re-read transiently fails, fall back
        // to the serialization we know it wrote — a None here would make the next
        // apply() spuriously refuse as "changed externally".
        *self.persisted_file.lock().unwrap() = std::fs::read_to_string(&self.config_path)
            .ok()
            .or_else(|| serde_json::to_string_pretty(&cfg).ok());
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
            .map(|s| DiscoveredSkill {
                name: s.name,
                description: s.description,
            })
            .collect();
        let sandbox_degraded =
            crate::wire::sandbox_degraded_from(self.current_loop().sandbox_descriptor());
        SettingsState {
            settings: cfg,
            workspace: self.workspace.display().to_string(),
            api_key_set: self.api_key.is_some(),
            hard_floor: HARD_FLOOR_DENYLIST.iter().map(|s| s.to_string()).collect(),
            discovered_skills: discovered,
            sandbox_degraded,
        }
    }

    /// Read-only self-portrait of the LIVE loop (post-apply). Assembles from state
    /// this struct already holds; never mutates, never exposes the prompt text.
    // Accepted cfg-vs-loop generation tear: config is read under a separate lock from
    // the loop/prompt, so a concurrent apply() can produce old-cfg + new-loop values in
    // one snapshot — read-only, self-heals on refresh (same accepted pattern as settings_state()).
    pub fn architecture(&self, memory_index_budget: usize) -> ArchitectureSnapshot {
        const CONTEXT_TOOLS: [&str; 1] = ["context_compact"];
        const SKILL_TOOLS: [&str; 4] = [
            "list_skills",
            "use_skill",
            "create_skill",
            "read_skill_file",
        ];
        let cfg = self.config.lock().unwrap().clone();
        let loop_ = self.current_loop();
        let mcp: HashSet<String> = self
            .mcp_tools
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        let tools = loop_
            .tool_schemas()
            .into_iter()
            .map(|s| {
                let kind = if mcp.contains(&s.name) {
                    "mcp"
                } else if CONTEXT_TOOLS.contains(&s.name.as_str()) {
                    "context"
                } else if SKILL_TOOLS.contains(&s.name.as_str()) {
                    "skills"
                } else {
                    "builtin"
                };
                ToolEntry {
                    summary: s
                        .description
                        .split('.')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string(),
                    name: s.name,
                    kind: kind.to_string(),
                }
            })
            .collect();
        let d = loop_.sandbox_descriptor();
        let prompt = self.current_system_prompt();
        ArchitectureSnapshot {
            model: ModelInfo {
                backend: cfg.backend.clone(),
                base_url_host: redact_base_url(&cfg.base_url),
                model: cfg.model.clone(),
                protocol: cfg.protocol.clone(),
                temperature: cfg.temperature,
                top_p: cfg.top_p,
                top_k: cfg.top_k,
                enable_thinking: cfg.enable_thinking,
                preserve_thinking: cfg.preserve_thinking,
            },
            tools,
            policy: PolicyInfo {
                allowlist: cfg.command_allowlist.clone(),
                denylist: cfg.effective_denylist(),
                hard_floor: HARD_FLOOR_DENYLIST.iter().map(|s| s.to_string()).collect(),
                http_allow_hosts: cfg.http_allow_hosts.clone(),
            },
            sandbox: SandboxInfo {
                mode: match d.mode {
                    agent_tools::Mode::Off => "off",
                    agent_tools::Mode::Auto => "auto",
                    agent_tools::Mode::Enforce => "enforce",
                }
                .to_string(),
                mechanism: d.mechanism.to_string(),
                image: d.image,
                network: d.network,
                degraded: d.degraded,
            },
            context: ContextInfo {
                context_limit: cfg.context_limit,
                max_tool_result_bytes: cfg.max_tool_result_bytes,
                memory_enabled: cfg.memory,
                memory_index_budget,
                compaction_model: cfg.compaction_model.as_ref().and_then(|m| m.model.clone()),
            },
            loop_info: LoopInfo {
                max_turns: cfg.max_turns,
                max_parallel_tools: cfg.max_parallel_tools,
                subagents_enabled: cfg.subagents,
                subagent_max_depth: cfg.subagent_max_depth,
                subagent_model: cfg.subagent_model.as_ref().and_then(|m| m.model.clone()),
                stream_idle_timeout_secs: DEFAULT_STREAM_IDLE_TIMEOUT.as_secs(),
            },
            prompt: PromptInfo {
                est_tokens: estimate_tokens(&prompt),
                override_active: cfg.system_prompt_override.is_some(),
                override_chars: cfg
                    .system_prompt_override
                    .as_ref()
                    .map(|s| s.chars().count()),
            },
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
    base_system_prompt: &str,
    artifacts: &Arc<SessionArtifacts>,
    compact_flag: &Arc<AtomicBool>,
    todos: &agent_core::TodoHandle,
    stats: &Arc<std::sync::RwLock<agent_core::SessionStats>>,
    trace: &Option<Arc<agent_runtime_config::TraceWriter>>,
    checkpoint: &Option<Arc<agent_core::Checkpointer>>,
) -> BuiltLoop {
    let model = build_model(
        &cfg.backend,
        &cfg.base_url,
        &cfg.model,
        claude_binary,
        api_key.clone(),
        claude_cli_opts(cfg),
    );
    assemble_loop(
        cfg,
        LoopParts {
            model,
            sink: sink.clone(),
            approval: approval.clone(),
            workspace: workspace.to_path_buf(),
            mcp_tools: mcp_tools.to_vec(),
            stream_idle_timeout: DEFAULT_STREAM_IDLE_TIMEOUT,
            base_system_prompt: base_system_prompt.to_string(),
            artifacts: artifacts.clone(),
            compact_flag: compact_flag.clone(),
            todos: todos.clone(),
            sandbox: build_sandbox(cfg),
            stats: stats.clone(),
            trace: trace.clone(),
            api_key: api_key.clone(),
            claude_binary: claude_binary.to_string(),
            checkpoint: checkpoint.clone(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_config::RuntimeConfig;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn slot() -> crate::sink::EventSlot {
        Arc::new(Mutex::new(None))
    }

    fn parts() -> (Arc<ChannelEventSink>, Arc<IpcApprovalChannel>) {
        let s = slot();
        let sink = Arc::new(ChannelEventSink::new(s.clone()));
        let approval = Arc::new(IpcApprovalChannel::new(s, Some(Duration::from_secs(1))));
        (sink, approval)
    }

    /// Minimal stub `Tool` for injecting named tools in architecture tests.
    struct NamedTool(String);

    #[async_trait::async_trait]
    impl agent_tools::Tool for NamedTool {
        fn name(&self) -> &str {
            &self.0
        }
        fn description(&self) -> &str {
            "stub tool. Does stub things."
        }
        fn schema(&self) -> agent_tools::ToolSchema {
            agent_tools::ToolSchema {
                name: self.0.clone(),
                description: "stub tool. Does stub things.".into(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            }
        }
        fn intent(
            &self,
            _a: &serde_json::Value,
        ) -> Result<agent_tools::ToolIntent, agent_tools::ToolError> {
            Ok(agent_tools::ToolIntent {
                tool: self.0.clone(),
                access: agent_tools::Access::Read,
                paths: vec![],
                command: None,
                summary: "stub".into(),
            })
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &agent_tools::ToolCtx,
        ) -> Result<agent_tools::ToolOutput, agent_tools::ToolError> {
            Ok(agent_tools::ToolOutput {
                content: "ok".into(),
                display: None,
            })
        }
    }

    /// Like `make()` but injects one MCP tool (`mcp_x`).
    fn make_with_tools() -> (RuntimeState, tempfile::TempDir) {
        let (sink, approval) = parts();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.json");
        let cfg = RuntimeConfig::from_launch(
            "openai".into(),
            "http://localhost:8080".into(),
            "m1".into(),
            "native".into(),
            8192,
        );
        let mcp_tool: Arc<dyn Tool> = Arc::new(NamedTool("mcp_x".into()));
        let rs = RuntimeState::new(
            cfg,
            sink,
            approval,
            dir.path().to_path_buf(),
            None,
            "claude".into(),
            path,
            Arc::from(vec![mcp_tool]),
            crate::daemon::SYSTEM_PROMPT.to_string(),
        );
        (rs, dir)
    }

    /// Like `make()` but points `trace_dir` at a tempdir so the descriptor
    /// (and trace) land there instead of the real `$HOME` — do NOT reuse
    /// `make_with_tools`/`make`, whose config leaves `trace_dir: None`.
    fn make_with_trace_dir() -> (RuntimeState, tempfile::TempDir, tempfile::TempDir) {
        let (sink, approval) = parts();
        let ws = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let path = ws.path().join("rt.json");
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(),
            "http://localhost:8080".into(),
            "m1".into(),
            "native".into(),
            8192,
        );
        cfg.trace_dir = Some(sessions.path().to_string_lossy().into_owned());
        let rs = RuntimeState::new(
            cfg,
            sink,
            approval,
            ws.path().to_path_buf(),
            None,
            "claude".into(),
            path,
            Arc::from(Vec::<Arc<dyn Tool>>::new()),
            crate::daemon::SYSTEM_PROMPT.to_string(),
        );
        (rs, ws, sessions)
    }

    #[test]
    fn runtime_writes_descriptor_at_construction_and_exposes_id() {
        let (rs, ws, sessions) = make_with_trace_dir();
        let id = rs.session_id().to_string();
        assert!(!id.is_empty());
        let dir = agent_runtime_config::session_dir(sessions.path(), &id);
        let d = agent_runtime_config::load_descriptor(&dir).expect("descriptor written");
        assert_eq!(d.session_id, id);
        assert_eq!(d.workspace, ws.path());
        assert_eq!(d.schema, agent_runtime_config::DESCRIPTOR_SCHEMA);
        assert!(d.config_path.is_some());
        // trace (enabled by from_launch) shares the SAME id
        assert!(sessions.path().join(format!("{id}.jsonl")).exists());
    }

    #[test]
    fn rewrite_descriptor_workspace_updates_only_workspace() {
        let (rs, _ws, sessions) = make_with_trace_dir();
        let id = rs.session_id().to_string();
        let dir = agent_runtime_config::session_dir(sessions.path(), &id);
        let before = agent_runtime_config::load_descriptor(&dir).unwrap();
        rs.rewrite_descriptor_workspace(std::path::Path::new("/elsewhere"));
        let after = agent_runtime_config::load_descriptor(&dir).unwrap();
        assert_eq!(after.workspace, std::path::Path::new("/elsewhere"));
        assert_eq!(after.session_id, before.session_id);
        assert_eq!(after.created_ms, before.created_ms);
    }

    #[test]
    fn architecture_lists_registered_tools_with_provenance() {
        let (rs, _dir) = make_with_tools();
        let snap = rs.architecture(512);
        let names: Vec<&str> = snap.tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"mcp_x"));
        let kind_of = |n: &str| {
            snap.tools
                .iter()
                .find(|t| t.name == n)
                .unwrap()
                .kind
                .clone()
        };
        assert_eq!(kind_of("mcp_x"), "mcp");
        // Only context_compact is classified "context" now that context_recall
        // is retired (offload recovery moved to the ordinary file tools).
        assert_eq!(kind_of("context_compact"), "context");
        assert!(
            !snap.tools.iter().any(|t| t.name == "context_recall"),
            "context_recall must not be registered post-retirement"
        );
        // grep lands in the same bucket as read_file (both plain file tools).
        assert_eq!(kind_of("grep"), kind_of("read_file"));
    }

    #[test]
    fn architecture_policy_carries_hard_floor_and_redacted_url() {
        let (rs, _dir) = make();
        let snap = rs.architecture(0);
        for f in agent_runtime_config::HARD_FLOOR_DENYLIST {
            assert!(snap.policy.hard_floor.contains(&f.to_string()));
            assert!(
                snap.policy.denylist.contains(&f.to_string()),
                "effective denylist includes floor"
            );
        }
        assert!(
            !snap.model.base_url_host.contains("/v1"),
            "path must be redacted: {}",
            snap.model.base_url_host
        );
    }

    #[test]
    fn architecture_prompt_flags_track_override() {
        let (rs, _dir) = make();
        assert!(!rs.architecture(0).prompt.override_active);
        let mut cfg = rs.settings_state().settings;
        cfg.system_prompt_override = Some("OVERRIDE".into());
        rs.apply(cfg).unwrap();
        let p = rs.architecture(0).prompt;
        assert!(p.override_active);
        assert_eq!(p.override_chars, Some(8));
        assert!(p.est_tokens > 0);
    }

    #[test]
    fn architecture_reflects_sandbox_and_memory_index_budget() {
        let (rs, _dir) = make();
        let snap = rs.architecture(1234);
        assert_eq!(snap.context.memory_index_budget, 1234);
        assert!(!snap.sandbox.mechanism.is_empty());
        // Verify sandbox mode is a lowercase string matching one of the valid modes
        assert!(
            matches!(snap.sandbox.mode.as_str(), "off" | "auto" | "enforce"),
            "sandbox mode must be lowercase (off/auto/enforce), got: {}",
            snap.sandbox.mode
        );
        // Verify it matches the fixture's configured mode (default is "auto")
        assert_eq!(snap.sandbox.mode, "auto");
        // DEFAULT_STREAM_IDLE_TIMEOUT is a build-time constant; it must be non-zero.
        assert!(
            snap.loop_info.stream_idle_timeout_secs > 0,
            "stream_idle_timeout_secs must be positive, got {}",
            snap.loop_info.stream_idle_timeout_secs
        );
    }

    fn make() -> (RuntimeState, tempfile::TempDir) {
        let (sink, approval) = parts();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.json");
        let cfg = RuntimeConfig::from_launch(
            "openai".into(),
            "http://localhost:8080".into(),
            "m1".into(),
            "native".into(),
            8192,
        );
        let rs = RuntimeState::new(
            cfg,
            sink,
            approval,
            dir.path().to_path_buf(),
            None,
            "claude".into(),
            path,
            Arc::from(Vec::<Arc<dyn Tool>>::new()),
            crate::daemon::SYSTEM_PROMPT.to_string(),
        );
        (rs, dir)
    }

    #[test]
    fn apply_swaps_the_loop_and_persists() {
        let (rs, dir) = make();
        let before = rs.current_loop();
        let mut next = RuntimeConfig::from_launch(
            "openai".into(),
            "http://localhost:8080".into(),
            "m2".into(),
            "native".into(),
            8192,
        );
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
            "openai".into(),
            "   ".into(),
            "m".into(),
            "native".into(),
            8192,
        ); // empty base_url
        bad.base_url = "  ".into();
        let err = rs.apply(bad).unwrap_err();
        assert!(err.contains("base_url"));
        assert!(
            Arc::ptr_eq(&before, &rs.current_loop()),
            "loop unchanged on rejection"
        );
    }

    #[test]
    fn settings_state_reports_floor_and_no_api_key() {
        let (rs, _dir) = make();
        let st = rs.settings_state();
        assert!(!st.api_key_set);
        assert!(st.hard_floor.iter().any(|d| d == "rm -rf /"));
    }

    #[test]
    fn apply_with_valid_active_skill_updates_system_prompt() {
        use std::fs;
        let (rs, dir) = make();
        // Author a skill under the workspace default writable root.
        let sdir = dir
            .path()
            .join(".rusty-agent")
            .join("skills")
            .join("greeter");
        fs::create_dir_all(&sdir).unwrap();
        fs::write(
            sdir.join("SKILL.md"),
            "---\nname: greeter\ndescription: d\n---\nSay hi politely.",
        )
        .unwrap();

        let before = rs.current_loop();
        let mut next = RuntimeConfig::from_launch(
            "openai".into(),
            "http://localhost:8080".into(),
            "m1".into(),
            "native".into(),
            8192,
        );
        next.active_skills = vec!["greeter".into()];
        rs.apply(next).unwrap();

        assert!(!Arc::ptr_eq(&before, &rs.current_loop()), "loop swapped");
        assert!(
            rs.current_system_prompt().contains("Say hi politely."),
            "preset body folded into the live system prompt"
        );
    }

    #[test]
    fn apply_rejects_unknown_active_skill_without_swapping() {
        let (rs, _dir) = make();
        let before = rs.current_loop();
        let mut bad = RuntimeConfig::from_launch(
            "openai".into(),
            "http://localhost:8080".into(),
            "m1".into(),
            "native".into(),
            8192,
        );
        bad.active_skills = vec!["does-not-exist".into()];
        let err = rs.apply(bad).unwrap_err();
        assert!(
            err.contains("does-not-exist"),
            "error names the missing skill: {err}"
        );
        assert!(
            Arc::ptr_eq(&before, &rs.current_loop()),
            "loop unchanged on rejection"
        );
    }

    #[test]
    fn startup_drops_unknown_persisted_preset_without_panicking() {
        // A persisted config naming a non-existent preset must still boot (lenient).
        let (sink, approval) = parts();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.json");
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(),
            "http://localhost:8080".into(),
            "m1".into(),
            "native".into(),
            8192,
        );
        cfg.active_skills = vec!["ghost".into()];
        let rs = RuntimeState::new(
            cfg,
            sink,
            approval,
            dir.path().to_path_buf(),
            None,
            "claude".into(),
            path,
            Arc::from(Vec::<Arc<dyn Tool>>::new()),
            crate::daemon::SYSTEM_PROMPT.to_string(),
        );
        // Booted: base prompt present, the ghost preset silently dropped.
        assert!(rs.current_system_prompt().contains("local coding agent"));
        assert!(!rs.current_system_prompt().contains("ghost"));
    }

    #[test]
    fn settings_state_includes_discovered_skills() {
        use std::fs;
        let (sink, approval) = parts();
        let dir = tempfile::tempdir().unwrap();
        // Put a skill in <workspace>/.rusty-agent/skills/greeter (the default writable root).
        let sdir = dir
            .path()
            .join(".rusty-agent")
            .join("skills")
            .join("greeter");
        fs::create_dir_all(&sdir).unwrap();
        fs::write(
            sdir.join("SKILL.md"),
            "---\nname: greeter\ndescription: says hi\n---\nbody",
        )
        .unwrap();
        let path = dir.path().join("rt.json");
        let cfg = RuntimeConfig::from_launch(
            "openai".into(),
            "http://localhost:8080".into(),
            "m1".into(),
            "native".into(),
            8192,
        );
        let rs = RuntimeState::new(
            cfg,
            sink,
            approval,
            dir.path().to_path_buf(),
            None,
            "claude".into(),
            path,
            Arc::from(Vec::<Arc<dyn Tool>>::new()),
            crate::daemon::SYSTEM_PROMPT.to_string(),
        );
        let st = rs.settings_state();
        assert!(st
            .discovered_skills
            .iter()
            .any(|s| s.name == "greeter" && s.description == "says hi"));
    }

    #[test]
    fn build_loop_with_sandbox_mode_off_succeeds() {
        // sandbox_mode = "off" resolves to HostExecutor (no Docker probe needed).
        let (sink, approval) = parts();
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(),
            "http://localhost:8080".into(),
            "m1".into(),
            "native".into(),
            8192,
        );
        cfg.sandbox_mode = "off".into();
        // build_loop must succeed — off → HostExecutor, no Docker required.
        let artifacts = Arc::new(SessionArtifacts::new());
        let flag = Arc::new(AtomicBool::new(false));
        let todos = Arc::new(Mutex::new(Vec::new()));
        let result = build_loop(
            &cfg,
            &sink,
            &approval,
            dir.path(),
            &None,
            "claude",
            &[],
            crate::daemon::SYSTEM_PROMPT,
            &artifacts,
            &flag,
            &todos,
            &Arc::default(),
            &None,
            &None,
        );
        // If we get here without panic/error the strategy was constructed OK.
        let _loop = result.loop_; // just confirm it's built
    }

    #[test]
    fn apply_refuses_when_config_file_changed_externally() {
        let (rs, dir) = make();
        let next = rs.settings_state().settings;
        rs.apply(next.clone()).unwrap(); // first apply persists and records content

        // simulate a CLI edit behind the daemon's back
        std::fs::write(dir.path().join("rt.json"), "{\"model\":\"other\"}").unwrap();

        let err = rs.apply(next).unwrap_err();
        assert!(err.contains("changed externally"), "got: {err}");
    }

    #[test]
    fn apply_twice_without_external_edits_is_fine() {
        let (rs, _dir) = make();
        let next = rs.settings_state().settings;
        rs.apply(next.clone()).unwrap();
        rs.apply(next).unwrap();
    }

    /// Scopes `$HOME` to a tempdir for the duration of the closure, then
    /// restores it — `RuntimeState::new` sources the daemon secret via
    /// `load_or_create_secret(metadata_root())`, i.e. the REAL `$HOME`; tests
    /// that reach that path must never write `~/.rusty-agent/secret` for
    /// real. Paired with `#[serial]` on every caller (env vars are process-global).
    fn with_scoped_home<T>(f: impl FnOnce() -> T) -> T {
        let home_dir = tempfile::tempdir().unwrap();
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", home_dir.path());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        match prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match result {
            Ok(v) => v,
            Err(p) => std::panic::resume_unwind(p),
        }
    }

    #[test]
    #[serial_test::serial]
    fn runtime_owns_a_checkpointer_rooted_in_the_session_dir() {
        with_scoped_home(|| {
            let (rs, _ws, sessions) = make_with_trace_dir();
            let ck = rs.checkpointer().expect("checkpointer built");
            let expect = agent_runtime_config::session_dir(sessions.path(), rs.session_id())
                .join("checkpoint");
            assert_eq!(ck.dir(), expect.as_path());
            // E1: construction creates NOTHING on disk
            assert!(!expect.exists());
        });
    }

    #[test]
    #[serial_test::serial]
    fn build_resume_loop_binds_descriptor_workspace_and_checkpointer() {
        with_scoped_home(|| {
            let (rs, _ws, sessions) = make_with_trace_dir();
            let other_ws = tempfile::tempdir().unwrap();
            let ck = agent_core::Checkpointer::new(
                sessions.path().join("old-1").join("checkpoint"),
                [1u8; 32],
                "old-1".into(),
            );
            let artifacts = Arc::new(SessionArtifacts::new());
            let todos: agent_core::TodoHandle = Arc::new(Mutex::new(Vec::new()));
            let flag = Arc::new(AtomicBool::new(false));
            let built = rs.build_resume_loop(other_ws.path(), ck, &artifacts, &todos, &flag);
            // system prompt composed from CURRENT config (live truth):
            assert_eq!(built.system_prompt, rs.current_system_prompt());
        });
    }

    #[test]
    fn build_loop_with_sandbox_mode_auto_constructs_loop() {
        // "auto" probes Docker but still returns a valid loop regardless of Docker availability.
        let (sink, approval) = parts();
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(),
            "http://localhost:8080".into(),
            "m1".into(),
            "native".into(),
            8192,
        );
        cfg.sandbox_mode = "auto".into();
        let artifacts = Arc::new(SessionArtifacts::new());
        let flag = Arc::new(AtomicBool::new(false));
        let todos = Arc::new(Mutex::new(Vec::new()));
        let result = build_loop(
            &cfg,
            &sink,
            &approval,
            dir.path(),
            &None,
            "claude",
            &[],
            crate::daemon::SYSTEM_PROMPT,
            &artifacts,
            &flag,
            &todos,
            &Arc::default(),
            &None,
            &None,
        );
        let _loop = result.loop_;
    }
}
