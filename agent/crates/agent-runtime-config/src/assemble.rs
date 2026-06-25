//! The single place a `RuntimeConfig` + per-frontend pieces become an `AgentLoop`.
//! Used by both the CLI (`agent-cli`) and the server (`agent-server`) so loop
//! assembly cannot diverge between front-ends.
use crate::{build_registry, build_sandbox, build_skills, pick_protocol, RuntimeConfig};
use agent_core::{AgentLoop, EventSink, LoopConfig, Retriever};
use agent_model::ModelClient;
use agent_policy::{ApprovalChannel, RulePolicy};
use agent_skills::compose_system_prompt;
use agent_tools::Tool;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// The per-frontend pieces: which sink/approval/model to use, the injected tools,
/// and the runtime knobs that are not part of the persisted `RuntimeConfig`.
pub struct LoopParts {
    pub model: Arc<dyn ModelClient>,
    pub sink: Arc<dyn EventSink>,
    pub approval: Arc<dyn ApprovalChannel>,
    pub workspace: PathBuf,
    pub mcp_tools: Vec<Arc<dyn Tool>>,
    pub memory_tools: Vec<Arc<dyn Tool>>,
    pub memory_retriever: Option<Arc<dyn Retriever>>,
    pub stream_idle_timeout: Duration,
    pub base_system_prompt: String,
    /// Offload table the `context_recall` tool reads. The caller owns it and
    /// shares the same handle with its `CuratedContext`. On a loop rebuild (server
    /// settings change), pass the SAME handle so the conversation's table survives.
    pub offload_store: Arc<dyn agent_core::OffloadStore>,
    /// Flag the `context_compact` tool sets; the caller's `CuratedContext` reads it.
    pub compact_flag: Arc<std::sync::atomic::AtomicBool>,
}

/// Result of assembling a loop: the loop itself, the composed system prompt, and
/// any `active_skills` that did not resolve (callers decide strictness).
pub struct BuiltLoop {
    pub loop_: Arc<AgentLoop>,
    pub system_prompt: String,
    pub unknown_presets: Vec<String>,
    /// Tool names registered at build time — retained so tests can assert injection.
    #[cfg(test)]
    pub registered_names: Vec<String>,
}

/// The single RuntimeConfig → LoopConfig mapping. Constants that are identical on
/// both front-ends today stay literals here; `stream_idle_timeout` is frontend-supplied.
pub fn loop_config_from(
    cfg: &RuntimeConfig,
    workspace: PathBuf,
    stream_idle_timeout: Duration,
) -> LoopConfig {
    LoopConfig {
        model_limit: cfg.context_limit,
        max_turns: cfg.max_turns,
        max_retries: 3,
        temperature: cfg.temperature,
        max_tokens: Some(cfg.max_tokens),
        workspace,
        tool_timeout: Duration::from_secs(120),
        stream_idle_timeout,
        top_p: cfg.top_p,
        top_k: cfg.top_k,
        min_p: cfg.min_p,
        presence_penalty: cfg.presence_penalty,
        repeat_penalty: cfg.repeat_penalty,
        enable_thinking: cfg.enable_thinking,
        preserve_thinking: cfg.preserve_thinking,
        sandbox: Some(build_sandbox(cfg)),
        max_parallel_tools: 8,
    }
}

/// The one place a RuntimeConfig + per-frontend `LoopParts` become an `AgentLoop`.
/// Never panics: a `compose_system_prompt` failure falls back to the base prompt.
pub fn assemble_loop(cfg: &RuntimeConfig, parts: LoopParts) -> BuiltLoop {
    let mut registry = build_registry(&cfg.http_allow_hosts);
    for t in &parts.mcp_tools {
        registry.register(t.clone());
    }
    if cfg.memory {
        for t in &parts.memory_tools {
            registry.register(t.clone());
        }
    }
    let (skill_registry, skill_tools) = build_skills(&cfg.skills_dirs, &parts.workspace);
    for t in skill_tools {
        registry.register(t);
    }

    // Context-management tools share the caller-owned offload store + compact flag
    // with the frontend's CuratedContext (passed in via LoopParts).
    for t in agent_core::context_tools(parts.offload_store.clone(), parts.compact_flag.clone()) {
        registry.register(t);
    }

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
    let system_prompt = match compose_system_prompt(&parts.base_system_prompt, &skill_registry, &presets) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "compose_system_prompt failed unexpectedly; using base prompt");
            parts.base_system_prompt.clone()
        }
    };

    #[cfg(test)]
    let registered_names: Vec<String> = registry.schemas().into_iter().map(|s| s.name).collect();

    let policy = Arc::new(RulePolicy {
        workspace: parts.workspace.clone(),
        command_allowlist: cfg.command_allowlist.clone(),
        command_denylist: cfg.effective_denylist(),
    });

    let agent = AgentLoop::new(
        parts.model,
        pick_protocol(&cfg.protocol),
        Arc::new(registry),
        policy,
        parts.approval,
        parts.sink,
        loop_config_from(cfg, parts.workspace.clone(), parts.stream_idle_timeout),
    );
    let agent = match (cfg.memory, &parts.memory_retriever) {
        (true, Some(r)) => agent.with_retriever(r.clone()),
        _ => agent,
    };

    BuiltLoop {
        loop_: Arc::new(agent),
        system_prompt,
        unknown_presets,
        #[cfg(test)]
        registered_names,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::AgentEvent;
    use agent_model::OpenAiCompatClient;
    use agent_policy::{ApprovalRequest, ApprovalResponse};

    struct NoSink;
    impl EventSink for NoSink {
        fn emit(&self, _e: AgentEvent) {}
    }
    struct NoApproval;
    #[async_trait::async_trait]
    impl ApprovalChannel for NoApproval {
        async fn request(&self, _r: ApprovalRequest) -> ApprovalResponse { ApprovalResponse::Approve }
    }

    fn fake_mem(name: &'static str) -> Arc<dyn Tool> {
        use agent_tools::{Access, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
        struct M(&'static str);
        #[async_trait::async_trait]
        impl Tool for M {
            fn name(&self) -> &str { self.0 }
            fn description(&self) -> &str { "fake" }
            fn schema(&self) -> ToolSchema {
                ToolSchema { name: self.0.into(), description: "fake".into(),
                    parameters: serde_json::json!({"type":"object"}) }
            }
            fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
                Ok(ToolIntent { tool: self.0.into(), access: Access::Read, paths: vec![],
                    command: None, summary: "x".into() })
            }
            async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx)
                -> Result<ToolOutput, ToolError> {
                Ok(ToolOutput { content: "ok".into(), display: None })
            }
        }
        Arc::new(M(name))
    }

    // A never-connected client is fine: assemble_loop only constructs the loop.
    fn parts(workspace: PathBuf, mem: Vec<Arc<dyn Tool>>) -> LoopParts {
        LoopParts {
            model: Arc::new(OpenAiCompatClient::new("http://127.0.0.1:0".into(), "m".into(), None)),
            sink: Arc::new(NoSink),
            approval: Arc::new(NoApproval),
            workspace,
            mcp_tools: vec![],
            memory_tools: mem,
            memory_retriever: None,
            stream_idle_timeout: Duration::from_secs(99),
            base_system_prompt: "BASE".into(),
            offload_store: Arc::new(agent_core::InMemoryOffloadStore::new()),
            compact_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn cfg() -> RuntimeConfig {
        RuntimeConfig::from_launch("openai".into(), "http://x".into(), "m".into(), "native".into(), 8192)
    }

    #[test]
    fn registers_memory_tools_when_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg(); c.memory = true;
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![fake_mem("remember")]));
        assert!(built.registered_names.iter().any(|n| n == "remember"));
    }

    #[test]
    fn registers_context_management_tools() {
        let dir = tempfile::tempdir().unwrap();
        let built = assemble_loop(&cfg(), parts(dir.path().to_path_buf(), vec![]));
        assert!(built.registered_names.iter().any(|n| n == "context_recall"));
        assert!(built.registered_names.iter().any(|n| n == "context_compact"));
    }

    #[test]
    fn skips_memory_tools_when_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg(); c.memory = false;
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![fake_mem("remember")]));
        assert!(!built.registered_names.iter().any(|n| n == "remember"));
    }

    #[test]
    fn unknown_active_skill_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg(); c.active_skills = vec!["definitely-not-a-real-skill".into()];
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        assert!(built.unknown_presets.iter().any(|n| n == "definitely-not-a-real-skill"));
    }

    #[test]
    fn loop_config_maps_runtime_config() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.temperature = 0.7; c.max_turns = 9; c.max_tokens = 1234; c.context_limit = 5000;
        c.top_p = Some(0.5); c.enable_thinking = false; c.preserve_thinking = true;
        let lc = loop_config_from(&c, dir.path().to_path_buf(), Duration::from_secs(77));
        assert_eq!(lc.model_limit, 5000);
        assert_eq!(lc.max_turns, 9);
        assert_eq!(lc.max_retries, 3);
        assert_eq!(lc.max_tokens, Some(1234));
        assert_eq!(lc.tool_timeout, Duration::from_secs(120));
        assert_eq!(lc.stream_idle_timeout, Duration::from_secs(77));
        assert_eq!(lc.max_parallel_tools, 8);
        assert_eq!(lc.top_p, Some(0.5));
        assert!(!lc.enable_thinking);
        assert!(lc.preserve_thinking);
        assert!((lc.temperature - 0.7).abs() < 1e-6);
        assert!(lc.sandbox.is_some());
    }
}
