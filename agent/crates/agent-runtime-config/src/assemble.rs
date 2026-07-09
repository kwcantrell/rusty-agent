//! The single place a `RuntimeConfig` + per-frontend pieces become an `AgentLoop`.
//! Used by both the CLI (`agent-cli`) and the server (`agent-server`) so loop
//! assembly cannot diverge between front-ends.
use crate::{build_registry, build_skills, pick_protocol, ModelRef, RuntimeConfig};
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
    /// Artifact stores the curator writes offloaded content into. The caller
    /// owns them and shares the SAME handle with its `CuratedContext`. On a loop
    /// rebuild (server settings change), pass the SAME handle so the
    /// conversation's offloaded artifacts survive (spec §5.3).
    pub artifacts: Arc<agent_core::SessionArtifacts>,
    /// Flag the `context_compact` tool sets; the caller's `CuratedContext` reads it.
    pub compact_flag: Arc<std::sync::atomic::AtomicBool>,
    /// The frontend's single sandbox instance — one probe + one availability
    /// cache per frontend (audit 3.5). Callers that also connect MCP must
    /// pass the SAME Arc they gave `connect_mcp`.
    pub sandbox: Arc<dyn agent_tools::SandboxStrategy>,
    /// Session-stable stats handle; caller-owned (survives server loop rebuilds).
    pub stats: Arc<std::sync::RwLock<agent_core::SessionStats>>,
    /// Session-stable trace writer; None = tracing disabled. Caller-owned: create
    /// ONCE per frontend lifetime (`TraceWriter::create` mints a `{epoch}-{pid}`
    /// session id, so per-assemble writers would interleave into one file).
    pub trace: Option<Arc<crate::trace::TraceWriter>>,
    /// Inputs for constructing ROUTED model clients (spec G3). The primary
    /// model stays caller-built; both values are frontend-held today.
    pub api_key: Option<String>,
    pub claude_binary: String,
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
    /// Did the subagent model route to a distinct client from the primary?
    /// `None` when subagents are disabled (mirrors `dispatch_base_names`).
    #[cfg(test)]
    pub subagent_model_routed: Option<bool>,
    /// Did a dedicated compaction model get built and applied?
    #[cfg(test)]
    pub compaction_model_routed: bool,
    /// (child model_limit, child max_tokens) captured at DispatchDeps build;
    /// None when subagents are disabled. Pins ModelRef limit inheritance.
    #[cfg(test)]
    pub child_loop_knobs: Option<(usize, Option<u32>)>,
}

/// Resolve the tool-call protocol name for a routed child loop (spec G5).
/// Precedence: an explicit `ModelRef::protocol` wins; otherwise, if the ModelRef
/// SWITCHED the child backend to `claude-cli` from a non-claude-cli session
/// default, force `"prompted"` (claude-cli is text-only — a native-protocol child
/// would silently break); otherwise inherit `cfg.protocol`.
pub(crate) fn child_protocol_name(cfg: &RuntimeConfig, r: Option<&ModelRef>) -> String {
    if let Some(p) = r.and_then(|r| r.protocol.as_deref()) {
        return p.to_string();
    }
    let child_backend = r.and_then(|r| r.backend.as_deref()).unwrap_or(&cfg.backend);
    if child_backend == "claude-cli" && cfg.backend != "claude-cli" {
        return "prompted".to_string();
    }
    cfg.protocol.clone()
}

/// True when a composed static system prompt is large relative to the model's
/// context window — over a quarter of it leaves little room for the conversation.
/// Advisory only (the caller warns); pure so it can be unit-tested log-free.
pub(crate) fn prompt_over_budget(est: usize, limit: usize) -> bool {
    est > limit / 4
}

/// True when the tool-result ingestion cap's estimated tokens (bytes/4) exceed
/// a quarter of the context window — a cap that big re-opens the
/// single-oversized-result overflow path the ingestion cap exists to close.
/// Advisory only (the caller warns); pure so it can be unit-tested log-free.
pub(crate) fn result_cap_over_budget(cap_bytes: usize, limit: usize) -> bool {
    cap_bytes / 4 > limit / 4
}

/// The single RuntimeConfig → LoopConfig mapping. Constants that are identical on
/// both front-ends today stay literals here; `stream_idle_timeout` is frontend-supplied.
pub fn loop_config_from(
    cfg: &RuntimeConfig,
    workspace: PathBuf,
    stream_idle_timeout: Duration,
    sandbox: Arc<dyn agent_tools::SandboxStrategy>,
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
        sandbox,
        max_parallel_tools: cfg.max_parallel_tools,
        post_tool_validators: cfg.post_tool_validators.clone(),
        compaction_model_limit: cfg.compaction_model.as_ref().and_then(|m| m.context_limit),
    }
}

/// Fresh claude-cli client with the parent's exact construction parameters.
/// Distinct instances keep each loop's session pool private (belt-and-
/// suspenders: the pool in ClaudeCliClient makes Arc-sharing safe, but
/// separate instances also keep the parent's pool unpolluted by child and
/// compaction entries). See docs/superpowers/specs/2026-07-07-claude-cli-followups-design.md.
fn fresh_claude_cli_client(
    cfg: &RuntimeConfig,
    claude_binary: &str,
    api_key: Option<String>,
) -> Arc<dyn ModelClient> {
    crate::build_model(
        &cfg.backend,
        &cfg.base_url,
        &cfg.model,
        claude_binary,
        api_key,
        crate::claude_cli_opts(cfg),
    )
}

/// The one place a RuntimeConfig + per-frontend `LoopParts` become an `AgentLoop`.
/// Never panics: a `compose_system_prompt` failure falls back to the base prompt.
pub fn assemble_loop(cfg: &RuntimeConfig, parts: LoopParts) -> BuiltLoop {
    let mut registry = build_registry(&cfg.http_allow_hosts, cfg.max_tool_result_bytes);
    for t in &parts.mcp_tools {
        registry.register(t.clone());
    }
    let (skill_registry, skill_tools) = build_skills(&cfg.skills_dirs, &parts.workspace);
    for t in skill_tools {
        registry.register(t);
    }

    // The middleware stack. Built here so its tool contributions can register
    // before the child_base snapshot below (spec §5.5/§5.6).
    let mut stack: Vec<Arc<dyn agent_core::Middleware>> = Vec::new();
    if cfg.memory {
        stack.push(Arc::new(agent_core::MemoryRecallMiddleware::new(
            parts.memory_tools.clone(),
            parts.memory_retriever.clone(),
        )));
    }
    // Scheduled context curation (spec §5.5): loop-bottom + text-exit maintain,
    // plus the context-management tools (child-invisible; children get their
    // own per-dispatch instance bound to a fresh store/flag below).
    stack.push(Arc::new(agent_core::ContextCurationMiddleware::new(
        parts.compact_flag.clone(),
    )));
    // Repeated-identical-call detection (spec §5.5): stateless, so a single
    // shared instance is fine on both the parent and every dispatch child.
    stack.push(Arc::new(agent_core::StuckDetectionMiddleware));
    // Register child-visible contributions BEFORE the child_base snapshot;
    // the rest after (spec §5.6). debug_assert: no name collisions.
    for c in stack.iter().flat_map(|m| m.tools()) {
        if c.child_visible {
            debug_assert!(
                registry.get(c.tool.name()).is_none(),
                "middleware tool contribution shadows an existing tool"
            );
            registry.register(c.tool.clone());
        }
    }

    // Snapshot for sub-agent children BEFORE context tools (child gets its own,
    // bound to a per-dispatch store/flag) and before dispatch itself (spec D4:
    // structural no-recursion). The POSITION of this line is the invariant.
    let child_base = cfg.subagents.then(|| registry.all());

    // Non-child-visible middleware tool contributions register after the
    // snapshot — context-curation tools land here (child-invisible: children
    // get their own instance in dispatch.rs, spec §5.6).
    for c in stack.iter().flat_map(|m| m.tools()) {
        if !c.child_visible {
            registry.register(c.tool.clone());
        }
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
    let base: &str = cfg
        .system_prompt_override
        .as_deref()
        .unwrap_or(&parts.base_system_prompt);
    let system_prompt = match compose_system_prompt(base, &skill_registry, &presets) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "compose_system_prompt failed unexpectedly; using base prompt");
            base.to_string()
        }
    };

    // Advisory: a static prompt over a quarter of the window crowds out the
    // conversation. No behavior change — surface it so preset bloat is visible.
    let prompt_est = agent_core::estimate_tokens(&system_prompt);
    if prompt_over_budget(prompt_est, cfg.context_limit) {
        tracing::warn!(
            estimate = prompt_est,
            context_limit = cfg.context_limit,
            presets = ?presets,
            "composed system prompt exceeds a quarter of the context window"
        );
    }

    // Advisory (audit 7.2): a tool-result cap comparable to the window re-opens
    // the single-oversized-result overflow path. No behavior change.
    if result_cap_over_budget(cfg.max_tool_result_bytes, cfg.context_limit) {
        tracing::warn!(
            max_tool_result_bytes = cfg.max_tool_result_bytes,
            context_limit = cfg.context_limit,
            "max_tool_result_bytes estimated tokens exceed a quarter of the context window"
        );
    }

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

    let loop_config = loop_config_from(
        cfg,
        parts.workspace.clone(),
        parts.stream_idle_timeout,
        parts.sandbox.clone(),
    );

    // Dedicated compaction model (spec G3): routed into both the parent loop and
    // child loops. For the openai backend, None inherits the primary model at every
    // read-site (stateless client — sharing is harmless). For claude-cli, a distinct
    // instance keeps the parent's session pool private (the pool itself makes sharing
    // safe since the checkout-keyed rework — this is belt-and-suspenders isolation).
    // Build a fresh instance when no explicit override is set.
    let compaction_model: Option<Arc<dyn ModelClient>> = cfg
        .compaction_model
        .as_ref()
        .map(|r| crate::build_routed_model(cfg, r, &parts.claude_binary, parts.api_key.clone()))
        .or_else(|| {
            if cfg.backend == "claude-cli" {
                Some(fresh_claude_cli_client(
                    cfg,
                    &parts.claude_binary,
                    parts.api_key.clone(),
                ))
            } else {
                None
            }
        });
    #[cfg(test)]
    let compaction_model_routed = compaction_model.is_some();

    // Sub-agent dispatch: capture the child-base names before the snapshot is moved
    // into the tool, then register `dispatch_agent` iff subagents are enabled.
    #[cfg(test)]
    let dispatch_base_names: Option<Vec<String>> = child_base
        .as_ref()
        .map(|b| b.iter().map(|t| t.name().to_string()).collect());
    #[cfg(test)]
    let mut subagent_model_routed: Option<bool> = None;
    #[cfg(test)]
    let mut child_loop_knobs: Option<(usize, Option<u32>)> = None;
    if let Some(child_base) = child_base {
        // Child protocol resolves through child_protocol_name (spec G5/M-1): a
        // ModelRef that switches the child backend to claude-cli defaults to
        // "prompted" unless it names a protocol explicitly.
        let child_protocol = pick_protocol(&child_protocol_name(cfg, cfg.subagent_model.as_ref()));
        let child_model = match &cfg.subagent_model {
            Some(r) => {
                crate::build_routed_model(cfg, r, &parts.claude_binary, parts.api_key.clone())
            }
            // For the openai backend, cloning the Arc is harmless: the client is
            // stateless. For claude-cli, a distinct instance keeps the parent's
            // session pool private (the pool itself makes sharing safe since the
            // checkout-keyed rework — this is belt-and-suspenders isolation).
            None if cfg.backend == "claude-cli" => {
                fresh_claude_cli_client(cfg, &parts.claude_binary, parts.api_key.clone())
            }
            None => parts.model.clone(),
        };
        #[cfg(test)]
        {
            subagent_model_routed = Some(!Arc::ptr_eq(&child_model, &parts.model));
        }
        let mut child_config = loop_config.clone();
        child_config.max_turns = cfg.subagent_max_turns;
        // Finding 4.2: a routed subagent model's declared limits travel with it;
        // None inherits the primary knobs already in the clone.
        if let Some(r) = &cfg.subagent_model {
            if let Some(cl) = r.context_limit {
                child_config.model_limit = cl;
            }
            if let Some(mt) = r.max_tokens {
                child_config.max_tokens = Some(mt);
            }
        }
        #[cfg(test)]
        {
            child_loop_knobs = Some((child_config.model_limit, child_config.max_tokens));
        }
        registry.register(Arc::new(agent_core::DispatchAgentTool::new(
            agent_core::DispatchDeps {
                model: child_model,
                protocol: child_protocol,
                policy: policy.clone(),
                approval: parts.approval.clone(),
                sink: sink.clone(),
                child_trace: parts.trace.clone().map(|t| {
                    Arc::new(crate::trace::ChildTraceTap(t)) as Arc<dyn agent_core::SubagentTrace>
                }),
                base_tools: child_base,
                child_system_prompt: format!(
                    "{system_prompt}\n\n{}",
                    agent_core::SUBAGENT_PREAMBLE
                ),
                loop_config: child_config,
                max_result_bytes: cfg.max_tool_result_bytes,
                subagent_timeout: Duration::from_secs(cfg.subagent_timeout_secs),
                compaction_model: compaction_model.clone(),
                depth: 1,
                max_depth: cfg.subagent_max_depth.max(1),
                id_prefix: String::new(),
                description_overrides: cfg.tool_description_overrides.clone(),
            },
        )));
    }

    registry.set_description_overrides(cfg.tool_description_overrides.clone());

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
    let agent = match &compaction_model {
        Some(m) => agent.with_compaction_model(m.clone()),
        None => agent,
    };
    let agent = agent.with_middleware(stack);

    // The guarded composite tools see: two read-only artifact mounts over the
    // caller's SessionArtifacts, backed by a HostBackend at the workspace root
    // (spec §5.2/§5.6). Curation writes go through the UNWRAPPED handles.
    use agent_tools::backend::{Backend, CompositeBackend, HostBackend, ReadOnlyToTools};
    for name in ["large_tool_results", "conversation_history"] {
        if parts.workspace.join(name).exists() {
            tracing::warn!(
                dir = name,
                "workspace entry is shadowed by a reserved artifact mount (spec §5.2)"
            );
        }
    }
    let composite: Arc<dyn Backend> = Arc::new(CompositeBackend::new(
        vec![
            (
                "large_tool_results/".into(),
                Arc::new(ReadOnlyToTools(parts.artifacts.results.clone())) as Arc<dyn Backend>,
            ),
            (
                "conversation_history/".into(),
                Arc::new(ReadOnlyToTools(parts.artifacts.history.clone())) as Arc<dyn Backend>,
            ),
        ],
        Arc::new(HostBackend::new(parts.workspace.clone())),
    ));
    let agent = agent.with_backend(composite);

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
        #[cfg(test)]
        subagent_model_routed,
        #[cfg(test)]
        compaction_model_routed,
        #[cfg(test)]
        child_loop_knobs,
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
            artifacts: Arc::new(agent_core::SessionArtifacts::new()),
            compact_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            sandbox: crate::build_sandbox(&cfg()),
            stats: Arc::new(std::sync::RwLock::new(agent_core::SessionStats::default())),
            trace: None,
            api_key: None,
            claude_binary: "claude".into(),
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
    fn loop_config_maps_compaction_model_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        assert_eq!(
            loop_config_from(
                &c,
                dir.path().to_path_buf(),
                Duration::from_secs(77),
                crate::build_sandbox(&c)
            )
            .compaction_model_limit,
            None
        );
        c.compaction_model = Some(crate::ModelRef {
            context_limit: Some(4096),
            ..Default::default()
        });
        assert_eq!(
            loop_config_from(
                &c,
                dir.path().to_path_buf(),
                Duration::from_secs(77),
                crate::build_sandbox(&c)
            )
            .compaction_model_limit,
            Some(4096)
        );
    }

    #[test]
    fn routed_subagent_window_reaches_child_config() {
        let dir = tempfile::tempdir().unwrap();
        // Unset: child inherits the primary knobs.
        let mut c = cfg();
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        let (ml, mt) = built.child_loop_knobs.expect("subagents on by default");
        assert_eq!(ml, c.context_limit);
        assert_eq!(mt, Some(c.max_tokens));
        // Set: the ModelRef limits override the child clone.
        c.subagent_model = Some(crate::ModelRef {
            context_limit: Some(2048),
            max_tokens: Some(256),
            ..Default::default()
        });
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        assert_eq!(built.child_loop_knobs, Some((2048, Some(256))));
    }

    #[test]
    fn prompt_over_budget_trips_above_a_quarter_of_the_window() {
        assert!(!prompt_over_budget(0, 8192));
        assert!(!prompt_over_budget(2048, 8192)); // exactly a quarter is not over
        assert!(prompt_over_budget(2049, 8192)); // one past the quarter trips
    }

    #[test]
    fn result_cap_over_budget_trips_above_a_quarter_of_the_window() {
        // est tokens = bytes/4; quarter of window = limit/4. Integer division:
        // the first value that trips is one whole token past, not one byte.
        assert!(!result_cap_over_budget(8192, 8192)); // exactly at: not over
        assert!(!result_cap_over_budget(8195, 8192)); // same token bucket (8195/4 == 2048)
        assert!(result_cap_over_budget(8196, 8192)); // one token past trips (2049 > 2048)
        assert!(!result_cap_over_budget(16 * 1024, 262_144)); // default cap vs big window: quiet
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
    fn assemble_wires_child_trace_only_when_tracing_is_on() {
        // No trace → assembles fine (child_trace None path).
        let dir = tempfile::tempdir().unwrap();
        let _ = assemble_loop(&cfg(), parts(dir.path().to_path_buf(), vec![]));
        // With a trace writer → also assembles fine (tap constructed).
        let mut p = parts(dir.path().to_path_buf(), vec![]);
        let tdir = tempfile::tempdir().unwrap();
        p.trace = Some(crate::trace::TraceWriter::create(tdir.path(), 1024 * 1024).unwrap());
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
        let lc = loop_config_from(
            &c,
            dir.path().to_path_buf(),
            Duration::from_secs(77),
            crate::build_sandbox(&c),
        );
        assert_eq!(lc.model_limit, 5000);
        assert_eq!(lc.max_turns, 9);
        assert_eq!(lc.max_retries, 3);
        assert_eq!(lc.max_tokens, Some(1234));
        assert_eq!(lc.tool_timeout, Duration::from_secs(120));
        assert_eq!(lc.stream_idle_timeout, Duration::from_secs(77));
        assert_eq!(lc.max_parallel_tools, 8); // default passthrough
        assert_eq!(lc.top_p, Some(0.5));
        assert!(!lc.enable_thinking);
        assert!(lc.preserve_thinking);
        assert!((lc.temperature - 0.7).abs() < 1e-6);
        assert!(!lc.sandbox.describe().mechanism.is_empty());

        // A non-default value passes straight through (no longer a hard-coded literal).
        let mut cfg2 = c.clone();
        cfg2.max_parallel_tools = 2;
        assert_eq!(
            loop_config_from(
                &cfg2,
                dir.path().to_path_buf(),
                Duration::from_secs(77),
                crate::build_sandbox(&cfg2)
            )
            .max_parallel_tools,
            2
        );

        let mut cfg3 = c.clone();
        cfg3.post_tool_validators = vec!["cargo check".into()];
        assert_eq!(
            loop_config_from(
                &cfg3,
                dir.path().to_path_buf(),
                Duration::from_secs(77),
                crate::build_sandbox(&cfg3)
            )
            .post_tool_validators,
            vec!["cargo check".to_string()]
        );
    }

    #[test]
    fn routed_models_default_to_the_primary() {
        let dir = tempfile::tempdir().unwrap();
        let built = assemble_loop(&cfg(), parts(dir.path().to_path_buf(), vec![]));
        assert_eq!(built.subagent_model_routed, Some(false));
        assert!(!built.compaction_model_routed);
    }

    #[test]
    fn routed_models_are_distinct_clients_when_configured() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.subagent_model = Some(crate::ModelRef {
            model: Some("mini".into()),
            ..Default::default()
        });
        c.compaction_model = Some(crate::ModelRef {
            model: Some("tiny".into()),
            ..Default::default()
        });
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        assert_eq!(built.subagent_model_routed, Some(true));
        assert!(built.compaction_model_routed);
    }

    #[test]
    fn routed_models_mix_subagent_set_compaction_none() {
        // M-2: subagent routed, compaction inherits primary.
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.subagent_model = Some(crate::ModelRef {
            model: Some("mini".into()),
            ..Default::default()
        });
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        assert_eq!(built.subagent_model_routed, Some(true));
        assert!(!built.compaction_model_routed);
    }

    #[test]
    fn routed_models_mix_compaction_set_subagent_none() {
        // M-2 (reverse): compaction routed, subagent inherits the primary client.
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.compaction_model = Some(crate::ModelRef {
            model: Some("tiny".into()),
            ..Default::default()
        });
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        assert_eq!(built.subagent_model_routed, Some(false));
        assert!(built.compaction_model_routed);
    }

    #[test]
    fn child_protocol_defaults_and_overrides() {
        // M-1: child protocol precedence — explicit wins, claude-cli backend
        // switch defaults to prompted, claude-cli primary inherits unchanged.
        let mut c = cfg(); // backend "openai", protocol "native"
        assert_eq!(child_protocol_name(&c, None), "native"); // default passthrough
                                                             // Explicit protocol wins even against a backend switch.
        let r = crate::ModelRef {
            backend: Some("claude-cli".into()),
            protocol: Some("native".into()),
            ..Default::default()
        };
        assert_eq!(child_protocol_name(&c, Some(&r)), "native");
        // claude-cli backend switch (no explicit protocol) → prompted.
        let r = crate::ModelRef {
            backend: Some("claude-cli".into()),
            ..Default::default()
        };
        assert_eq!(child_protocol_name(&c, Some(&r)), "prompted");
        // claude-cli PRIMARY: no switch, inherit cfg.protocol unchanged (even
        // when a ModelRef restates the same claude-cli backend).
        c.backend = "claude-cli".into();
        assert_eq!(child_protocol_name(&c, None), "native");
        assert_eq!(child_protocol_name(&c, Some(&r)), "native");
    }

    #[test]
    fn claude_cli_child_and_compaction_get_distinct_clients_when_none_configured() {
        // Finding 1 regression pin: for the claude-cli backend with no explicit
        // subagent_model or compaction_model, the assembled child and compaction
        // clients must NOT be the same Arc as the parent — each ClaudeCliClient
        // owns its own session state, and sharing the parent instance causes the
        // child/compaction call's session fingerprints to overwrite the parent's,
        // silently defeating session reuse.
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.backend = "claude-cli".into();
        c.protocol = "prompted".into();
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        // subagent_model_routed = Some(true) ↔ child client is a distinct Arc
        assert_eq!(built.subagent_model_routed, Some(true));
        // compaction client is also a distinct (fresh) instance
        assert!(built.compaction_model_routed);
    }

    #[test]
    fn openai_child_still_shares_parent_arc_when_none_configured() {
        // For the openai backend, the stateless client is safe to share; the
        // clone behavior must remain unchanged so we don't churn a working path.
        let dir = tempfile::tempdir().unwrap();
        let built = assemble_loop(&cfg(), parts(dir.path().to_path_buf(), vec![]));
        // cfg() uses backend "openai" — child should share the parent Arc
        assert_eq!(built.subagent_model_routed, Some(false));
        assert!(!built.compaction_model_routed);
    }

    #[test]
    fn depth_zero_is_clamped_to_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.subagent_max_depth = 0;
        // Assembles fine; the clamp is a read-site rule (no panic, tool registered).
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        assert!(built.registered_names.iter().any(|n| n == "dispatch_agent"));
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
    fn child_base_snapshot_includes_memory_tools_when_enabled() {
        // Inclusion half of the snapshot invariant: the snapshot is taken AFTER
        // memory tools register (and before context tools + dispatch), so an
        // enabled memory tool is child-visible.
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.memory = true;
        let built = assemble_loop(
            &c,
            parts(dir.path().to_path_buf(), vec![fake_mem("remember")]),
        );
        let base = built.dispatch_base_names.expect("subagents on by default");
        assert!(base.iter().any(|n| n == "remember"), "{base:?}");
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
    fn system_prompt_override_replaces_the_base() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.system_prompt_override = Some("OVERRIDE PROMPT".into());
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        assert!(built.system_prompt.starts_with("OVERRIDE PROMPT"));
        assert!(!built.system_prompt.contains("BASE"));
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

    #[test]
    fn assemble_uses_the_injected_sandbox_not_a_fresh_build() {
        // Audit 3.5: one isolation boundary, one authoritative instance. If
        // assemble rebuilt from cfg, enforce-mode would yield mechanism
        // "docker"; seeing "host" proves the caller's Arc is used.
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.sandbox_mode = "enforce".into();
        let mut p = parts(dir.path().to_path_buf(), vec![]);
        p.sandbox = Arc::new(agent_tools::HostExecutor);
        let built = assemble_loop(&c, p);
        assert_eq!(built.loop_.sandbox_descriptor().mechanism, "host");
    }
}
