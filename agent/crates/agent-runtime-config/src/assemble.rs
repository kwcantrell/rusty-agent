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
    /// Session-stable stats handle; caller-owned (survives server loop rebuilds).
    pub stats: Arc<std::sync::RwLock<agent_core::SessionStats>>,
    /// Session-stable trace writer; None = tracing disabled. Caller-owned: create
    /// ONCE per frontend lifetime (`TraceWriter::create` mints a `{epoch}-{pid}`
    /// session id, so per-assemble writers would interleave into one file).
    pub trace: Option<Arc<crate::trace::TraceWriter>>,
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
    /// Assembled, folded tool schemas — retained so tests can assert the tool contract.
    #[cfg(test)]
    pub schemas: Vec<agent_tools::ToolSchema>,
    /// Child-base snapshot names when subagents are enabled — pins the "snapshot
    /// excludes context tools + dispatch itself" invariant.
    #[cfg(test)]
    pub dispatch_base_names: Option<Vec<String>>,
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
        sandbox: build_sandbox(cfg),
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

    // Snapshot for sub-agent children BEFORE context tools (child gets its own,
    // bound to a per-dispatch store/flag) and before dispatch itself (spec D4:
    // structural no-recursion). The POSITION of this line is the invariant.
    let child_base = cfg.subagents.then(|| registry.all());

    // Context-management tools share the caller-owned offload store + compact flag
    // with the frontend's CuratedContext (passed in via LoopParts).
    for t in agent_core::context_tools(
        parts.offload_store.clone(),
        parts.compact_flag.clone(),
        cfg.max_tool_result_bytes,
    ) {
        registry.register(t);
    }

    let available: HashSet<String> = skill_registry.scan().into_iter().map(|s| s.name).collect();
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
    let system_prompt = match compose_system_prompt(
        &parts.base_system_prompt,
        &skill_registry,
        &presets,
    ) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "compose_system_prompt failed unexpectedly; using base prompt");
            parts.base_system_prompt.clone()
        }
    };

    let policy = Arc::new(RulePolicy {
        workspace: parts.workspace.clone(),
        command_allowlist: cfg.command_allowlist.clone(),
        command_denylist: cfg.effective_denylist(),
    });

    // Every event the loop emits flows through the observability wrapper:
    // fold stats, write the trace, then forward to the frontend sink.
    let sink: Arc<dyn EventSink> = Arc::new(crate::trace::ObservedSink {
        inner: parts.sink.clone(),
        stats: parts.stats.clone(),
        trace: parts.trace.clone(),
    });

    let loop_config = loop_config_from(cfg, parts.workspace.clone(), parts.stream_idle_timeout);

    // Sub-agent dispatch: capture the child-base names before the snapshot is moved
    // into the tool, then register `dispatch_agent` iff subagents are enabled.
    #[cfg(test)]
    let dispatch_base_names: Option<Vec<String>> = child_base
        .as_ref()
        .map(|b| b.iter().map(|t| t.name().to_string()).collect());
    if let Some(child_base) = child_base {
        let mut child_config = loop_config.clone();
        child_config.max_turns = cfg.subagent_max_turns;
        registry.register(Arc::new(agent_core::DispatchAgentTool::new(
            agent_core::DispatchDeps {
                model: parts.model.clone(),
                protocol: pick_protocol(&cfg.protocol),
                policy: policy.clone(),
                approval: parts.approval.clone(),
                sink: sink.clone(),
                base_tools: child_base,
                child_system_prompt: format!(
                    "{system_prompt}\n\n{}",
                    agent_core::SUBAGENT_PREAMBLE
                ),
                loop_config: child_config,
                max_result_bytes: cfg.max_tool_result_bytes,
                subagent_timeout: Duration::from_secs(cfg.subagent_timeout_secs),
            },
        )));
    }

    #[cfg(test)]
    let schemas = registry.schemas();
    #[cfg(test)]
    let registered_names: Vec<String> = schemas.iter().map(|s| s.name.clone()).collect();

    let agent = AgentLoop::new(
        parts.model,
        pick_protocol(&cfg.protocol),
        Arc::new(registry),
        policy,
        parts.approval,
        sink,
        loop_config,
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
        #[cfg(test)]
        schemas,
        #[cfg(test)]
        dispatch_base_names,
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
        async fn request(&self, _r: ApprovalRequest) -> ApprovalResponse {
            ApprovalResponse::Approve
        }
    }

    fn fake_mem(name: &'static str) -> Arc<dyn Tool> {
        use agent_tools::{Access, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
        struct M(&'static str);
        #[async_trait::async_trait]
        impl Tool for M {
            fn name(&self) -> &str {
                self.0
            }
            fn description(&self) -> &str {
                "fake"
            }
            fn schema(&self) -> ToolSchema {
                ToolSchema {
                    name: self.0.into(),
                    description: "fake".into(),
                    parameters: serde_json::json!({"type":"object"}),
                }
            }
            fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
                Ok(ToolIntent {
                    tool: self.0.into(),
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
                Ok(ToolOutput {
                    content: "ok".into(),
                    display: None,
                })
            }
        }
        Arc::new(M(name))
    }

    // A never-connected client is fine: assemble_loop only constructs the loop.
    fn parts(workspace: PathBuf, mem: Vec<Arc<dyn Tool>>) -> LoopParts {
        LoopParts {
            model: Arc::new(OpenAiCompatClient::new(
                "http://127.0.0.1:0".into(),
                "m".into(),
                None,
            )),
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
            stats: Arc::new(std::sync::RwLock::new(agent_core::SessionStats::default())),
            trace: None,
        }
    }

    fn cfg() -> RuntimeConfig {
        RuntimeConfig::from_launch(
            "openai".into(),
            "http://x".into(),
            "m".into(),
            "native".into(),
            8192,
        )
    }

    #[test]
    fn assemble_wires_stats_through_observed_sink() {
        // The loop's installed sink is not directly reachable, so assert at the
        // unit level: LoopParts carries the caller-owned handles, and an
        // ObservedSink built over them folds stats AND forwards to the inner sink.
        let dir = tempfile::tempdir().unwrap();
        let p = parts(dir.path().to_path_buf(), vec![]);
        let stats = p.stats.clone();
        let inner = Arc::new(agent_core::testkit::CollectingSink::default());
        let sink = crate::trace::ObservedSink {
            inner: inner.clone(),
            stats: stats.clone(),
            trace: p.trace.clone(),
        };
        sink.emit(AgentEvent::Error("x".into()));
        assert_eq!(stats.read().unwrap().errors, 1); // folded
        assert_eq!(inner.events.lock().unwrap().len(), 1); // forwarded
                                                           // And the loop still assembles with the new fields present.
        let _ = assemble_loop(&cfg(), p);
    }

    #[test]
    fn registers_memory_tools_when_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.memory = true;
        let built = assemble_loop(
            &c,
            parts(dir.path().to_path_buf(), vec![fake_mem("remember")]),
        );
        assert!(built.registered_names.iter().any(|n| n == "remember"));
    }

    #[test]
    fn registers_context_management_tools() {
        let dir = tempfile::tempdir().unwrap();
        let built = assemble_loop(&cfg(), parts(dir.path().to_path_buf(), vec![]));
        assert!(built.registered_names.iter().any(|n| n == "context_recall"));
        assert!(built
            .registered_names
            .iter()
            .any(|n| n == "context_compact"));
    }

    #[test]
    fn skips_memory_tools_when_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.memory = false;
        let built = assemble_loop(
            &c,
            parts(dir.path().to_path_buf(), vec![fake_mem("remember")]),
        );
        assert!(!built.registered_names.iter().any(|n| n == "remember"));
    }

    #[test]
    fn unknown_active_skill_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.active_skills = vec!["definitely-not-a-real-skill".into()];
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        assert!(built
            .unknown_presets
            .iter()
            .any(|n| n == "definitely-not-a-real-skill"));
    }

    #[test]
    fn loop_config_maps_runtime_config() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.temperature = 0.7;
        c.max_turns = 9;
        c.max_tokens = 1234;
        c.context_limit = 5000;
        c.top_p = Some(0.5);
        c.enable_thinking = false;
        c.preserve_thinking = true;
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
        assert!(!lc.sandbox.describe().mechanism.is_empty());
    }

    #[test]
    fn registers_dispatch_agent_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let built = assemble_loop(&cfg(), parts(dir.path().to_path_buf(), vec![]));
        assert!(built.registered_names.iter().any(|n| n == "dispatch_agent"));
    }

    #[test]
    fn omits_dispatch_agent_when_subagents_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.subagents = false;
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        assert!(!built.registered_names.iter().any(|n| n == "dispatch_agent"));
        assert!(built.dispatch_base_names.is_none());
    }

    #[test]
    fn child_base_snapshot_excludes_context_tools_and_dispatch_itself() {
        let dir = tempfile::tempdir().unwrap();
        let built = assemble_loop(&cfg(), parts(dir.path().to_path_buf(), vec![]));
        let base = built.dispatch_base_names.expect("subagents on by default");
        assert!(!base.iter().any(|n| n == "dispatch_agent"), "{base:?}");
        assert!(
            !base
                .iter()
                .any(|n| n == "context_recall" || n == "context_compact"),
            "{base:?}"
        );
        // Sanity: real tools are in the snapshot.
        assert!(base.iter().any(|n| n == "read_file"), "{base:?}");
    }

    #[test]
    fn every_required_param_is_described_in_the_assembled_registry() {
        let dir = tempfile::tempdir().unwrap();
        // Default config (memory off): base + context + skill tools are real; the
        // runtime-injected `recall` is intentionally absent (enforced in agent-memory).
        let built = assemble_loop(&cfg(), parts(dir.path().to_path_buf(), vec![]));
        for s in &built.schemas {
            let missing = agent_tools::required_params_missing_description(s);
            assert!(
                missing.is_empty(),
                "{} has undescribed required params: {missing:?}",
                s.name
            );
        }
    }

    #[test]
    fn confusable_tools_carry_disambiguation_in_the_assembled_registry() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let built = assemble_loop(&cfg(), parts(dir.path().to_path_buf(), vec![]));
        let present: HashSet<&str> = built.schemas.iter().map(|s| s.name.as_str()).collect();

        // Every confusable tool that IS assembled here must carry the folded marker.
        for name in agent_tools::CONFUSABLE_TOOLS {
            if let Some(s) = built.schemas.iter().find(|s| s.name == *name) {
                assert!(
                    s.description.contains(agent_tools::WHEN_NOT_TO_CALL_MARKER),
                    "{name} is missing '{}' in its description: {}",
                    agent_tools::WHEN_NOT_TO_CALL_MARKER,
                    s.description
                );
            }
        }

        // Coverage ratchet: the ONLY confusable tool absent from this assembly is
        // `recall` (runtime-injected, enforced in agent-memory). If a future
        // confusable tool becomes invisible here without separate coverage, this
        // fails and forces a decision.
        let absent: HashSet<&str> = agent_tools::CONFUSABLE_TOOLS
            .iter()
            .copied()
            .filter(|n| !present.contains(n))
            .collect();
        assert_eq!(
            absent,
            HashSet::from(["recall"]),
            "unexpected confusable tools missing from the assembled registry: {absent:?}"
        );
    }
}
