//! Sub-agent dispatch: sub-agents-as-tools (spec 2026-07-01-subagent-dispatch-core).
use crate::{
    AgentEvent, AgentLoop, ContextCurationMiddleware, CuratedContext, EventSink, LoopConfig,
    Middleware, OffloadConfig, SessionArtifacts, StuckDetectionMiddleware,
};
use agent_model::{Message, ModelClient, StopReason, ToolCallProtocol};
use agent_policy::{ApprovalChannel, PolicyEngine};
use agent_tools::{
    Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolRegistry, ToolSchema,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Appended to the parent's composed system prompt for every child.
pub const SUBAGENT_PREAMBLE: &str = "You are a sub-agent dispatched by a parent \
agent to complete one self-contained task. Work autonomously: no one can answer \
questions. Your final message is returned verbatim to the parent as the task \
result, so end with a complete, standalone answer.";

/// Upper bound on the `role` arg (system-prompt injection; spec G6).
pub const MAX_ROLE_CHARS: usize = 2000;

static DISPATCH_ORDINAL: AtomicU64 = AtomicU64::new(1);

/// Process-wide dispatch ordinal: keeps forwarded child event ids unique across
/// parallel siblings and across the parent's own tool-call ids (spec D9).
pub fn next_dispatch_n() -> u64 {
    DISPATCH_ORDINAL.fetch_add(1, Ordering::Relaxed)
}

#[derive(Default)]
struct Capture {
    /// Token text split into segments at ToolResult boundaries; the last
    /// segment is the child's final-turn text (spec D10).
    segments: Vec<String>,
    tool_calls: u64,
    turns: usize,
    stop: Option<StopReason>,
}

pub struct CaptureSummary {
    pub final_text: String,
    pub tool_calls: u64,
    pub turns: usize,
    pub stop: Option<StopReason>,
}

/// Sink-shaped hook for tracing the child's non-forwarded transcript
/// (implemented over TraceWriter in agent-runtime-config — dep direction).
pub trait SubagentTrace: Send + Sync {
    /// Record one non-forwarded child event, attributed to dispatch ordinal `n`
    /// and the dispatching model call's id `parent_id` (lineage join: a child
    /// that makes zero tool calls still ties to its dispatch row via this id).
    fn record(&self, n: u64, parent_id: &str, event: &AgentEvent);
}

/// The child loop's sink: captures the transcript for the tool result and
/// forwards ONLY ToolStart/ToolResult (ids `sub{n}:{id}`, names `sub:{name}`)
/// plus ServerUsage (real cost) to the parent sink — all existing wire frames,
/// so no wire/web/CLI changes (spec D9). Forwards carry the dispatching call's
/// id as `parent_id` (lineage). Child Token/Done/Error/Context stay off the
/// parent's streamed transcript, but tee to the optional child-trace tap so a
/// failed child turn is replayable (spec E4).
pub struct SubagentSink {
    parent: Arc<dyn EventSink>,
    n: u64,
    /// The dispatching model call's id, stamped as `parent_id` on every forward.
    parent_call_id: String,
    /// Tap for the child's non-forwarded events; None = tracing off.
    child_trace: Option<Arc<dyn SubagentTrace>>,
    cap: Mutex<Capture>,
}

impl SubagentSink {
    pub fn new(
        parent: Arc<dyn EventSink>,
        n: u64,
        parent_call_id: String,
        child_trace: Option<Arc<dyn SubagentTrace>>,
    ) -> Self {
        Self {
            parent,
            n,
            parent_call_id,
            child_trace,
            cap: Mutex::new(Capture {
                segments: vec![String::new()],
                ..Capture::default()
            }),
        }
    }

    pub fn summary(&self) -> CaptureSummary {
        let cap = self.cap.lock().unwrap();
        let tail = cap.segments.last().cloned().unwrap_or_default();
        let final_text = if tail.trim().is_empty() {
            cap.segments.concat().trim().to_string()
        } else {
            tail.trim().to_string()
        };
        CaptureSummary {
            final_text,
            tool_calls: cap.tool_calls,
            turns: cap.turns,
            stop: cap.stop,
        }
    }
}

impl EventSink for SubagentSink {
    fn emit(&self, event: AgentEvent) {
        match event {
            AgentEvent::ToolStart { id, name, args, .. } => {
                self.cap.lock().unwrap().tool_calls += 1;
                self.parent.emit(AgentEvent::ToolStart {
                    id: format!("sub{}:{}", self.n, id),
                    name: format!("sub:{name}"),
                    args,
                    parent_id: Some(self.parent_call_id.clone()),
                });
            }
            AgentEvent::ToolResult {
                id,
                name,
                status,
                output,
                duration_ms,
                ..
            } => {
                self.cap.lock().unwrap().segments.push(String::new());
                self.parent.emit(AgentEvent::ToolResult {
                    id: format!("sub{}:{}", self.n, id),
                    name: format!("sub:{name}"),
                    status,
                    output,
                    duration_ms,
                    parent_id: Some(self.parent_call_id.clone()),
                });
            }
            AgentEvent::ServerUsage {
                prompt_tokens,
                completion_tokens,
                reasoning_tokens,
                cached_tokens,
                cost_usd,
                turn_duration_ms,
                turn,
                ..
            } => {
                self.parent.emit(AgentEvent::ServerUsage {
                    prompt_tokens,
                    completion_tokens,
                    reasoning_tokens,
                    cached_tokens,
                    cost_usd,
                    turn_duration_ms,
                    turn,
                    parent_id: Some(self.parent_call_id.clone()),
                });
            }
            // Everything else stays off the frontends (spec D9/E9) but goes to
            // the child-trace tap so a failed child turn is replayable (E4).
            other => {
                if let Some(t) = &self.child_trace {
                    t.record(self.n, &self.parent_call_id, &other);
                }
                let mut cap = self.cap.lock().unwrap();
                match other {
                    AgentEvent::Token(t) => {
                        cap.segments
                            .last_mut()
                            .expect("segments never empty")
                            .push_str(&t);
                    }
                    AgentEvent::Usage { turn, .. } => cap.turns = cap.turns.max(turn),
                    AgentEvent::Done(reason) => cap.stop = Some(reason),
                    // A child stream died mid-answer and retries: retract the
                    // abandoned trailing text from the current segment so the
                    // captured result the parent model reads holds only the
                    // re-streamed text. Reasoning isn't captured, so only the
                    // text count matters; trim char-boundary-safe (count chars,
                    // not bytes). If the segment empties, leave it empty — don't
                    // pop, or the ToolResult-boundary segment invariant breaks.
                    AgentEvent::StreamRetry {
                        discarded_text_chars,
                        ..
                    } => {
                        let seg = cap.segments.last_mut().expect("segments never empty");
                        let keep = seg.chars().count().saturating_sub(discarded_text_chars);
                        *seg = seg.chars().take(keep).collect();
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Build the tool result for a child that died (wall-clock timeout or fatal
/// model error) from whatever the sink captured — partial results reach the
/// coordinator instead of being discarded (finding 4.4; mirrors the
/// budget-exhaustion posture). `stop_fallback` keeps the footer honest when
/// the child never emitted Done: "timeout" / "failed", never a clean Stop.
fn failure_output(sink: &SubagentSink, what: String, stop_fallback: &str) -> ToolOutput {
    let s = sink.summary();
    let stop_str = match s.stop {
        Some(r) => format!("{r:?}"),
        None => stop_fallback.to_string(),
    };
    let footer = format!(
        "[sub-agent: {} turns, {} tool calls, stop: {stop_str}]",
        s.turns, s.tool_calls
    );
    let content = if s.final_text.is_empty() {
        format!("[{what} — no partial transcript captured]\n{footer}")
    } else {
        format!(
            "[{what} — partial transcript follows]\n{}\n\n{footer}",
            s.final_text
        )
    };
    ToolOutput {
        content,
        display: None,
    }
}

/// Everything `DispatchAgentTool` needs to spawn a child `AgentLoop`.
#[derive(Clone)]
pub struct DispatchDeps {
    pub model: Arc<dyn ModelClient>,
    pub protocol: Arc<dyn ToolCallProtocol>,
    pub policy: Arc<dyn PolicyEngine>,
    pub approval: Arc<dyn ApprovalChannel>,
    /// The parent's (Observed) sink — forwarded child events reach stats/trace/UI.
    pub sink: Arc<dyn EventSink>,
    /// Trace tap for the child's non-forwarded events; None = tracing off.
    pub child_trace: Option<Arc<dyn SubagentTrace>>,
    /// Snapshot of the parent's tools taken BEFORE dispatch_agent and the
    /// parent's context tools were registered (spec D4: structural depth-1).
    pub base_tools: Vec<Arc<dyn Tool>>,
    pub child_system_prompt: String,
    /// Parent LoopConfig clone with max_turns = subagent_max_turns (shares the
    /// parent's sandbox Arc — spec Invariant).
    pub loop_config: LoopConfig,
    pub max_result_bytes: usize,
    pub subagent_timeout: std::time::Duration,
    /// Dedicated compaction model routed into child loops too; None = use `model`.
    pub compaction_model: Option<Arc<dyn ModelClient>>,
    /// This tool's depth; top-level = 1 (spec G7).
    pub depth: usize,
    /// From `cfg.subagent_max_depth` (assembly clamps >= 1); a child may dispatch
    /// only while `depth < max_depth` (spec G7).
    pub max_depth: usize,
    /// "" at top level; "sub{n}:" for a nested tool so a grandchild's parent_id
    /// is the child row's on-wire visible id (spec G8).
    pub id_prefix: String,
    /// Parent-configured tool-description overrides, applied to the child
    /// registry too so the tool vocabulary stays uniform across depths
    /// (finding 4.1; seam spec 2026-07-02-tool-description-override-seam).
    pub description_overrides: std::collections::HashMap<String, String>,
}

pub struct DispatchAgentTool {
    deps: DispatchDeps,
    /// Computed at construction from depth/max_depth: the "minus dispatch_agent"
    /// claim is only true at the depth floor (findings 2.3/4.5).
    description: String,
}

impl DispatchAgentTool {
    pub fn new(deps: DispatchDeps) -> Self {
        // Matches the child-registry rule in execute(): a child gets a nested
        // dispatch_agent by default whenever depth < max_depth.
        let caps = if deps.depth < deps.max_depth {
            "(including dispatch_agent while nesting depth allows, so it can \
             dispatch its own sub-agents; the tools allowlist restricts this \
             transitively)"
        } else {
            "(minus dispatch_agent itself)"
        };
        let description = format!(
            "Delegate an independent, multi-step subtask to an isolated sub-agent with \
             its own fresh context window. The sub-agent has the same permissions and \
             tools as you {caps}, works autonomously on the \
             prompt you give it, and its final answer is returned as this tool's \
             result. Make the prompt self-contained: the sub-agent cannot see this \
             conversation. You may dispatch several sub-agents in one message by \
             issuing multiple dispatch_agent calls — they run concurrently."
        );
        Self { deps, description }
    }
}

#[async_trait::async_trait]
impl Tool for DispatchAgentTool {
    fn name(&self) -> &str {
        "dispatch_agent"
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn when_not_to_call(&self) -> Option<&str> {
        Some(
            "Do NOT use for a single operation another tool does directly — call \
             that tool. Do not use when the answer depends on this conversation's \
             context (the sub-agent cannot see it), and do not expect it to ask \
             you questions — it runs unattended.",
        )
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "dispatch_agent".into(),
            description: self.description().into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "The complete, self-contained task for the sub-agent: goal, relevant paths/facts, and what to return."
                    },
                    "tools": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Optional allowlist restricting which tools the sub-agent may use (default: all). For focus, not safety — permissions are inherited either way. The child's context tool (context_compact) is always available. Include dispatch_agent to let the sub-agent dispatch its own (only meaningful when nesting depth allows); the restriction applies transitively to its children."
                    },
                    "role": {
                        "type": "string",
                        "description": "Optional persona/role instructions injected into the sub-agent's system prompt (stronger steering than putting them in the prompt). Max 2000 characters."
                    }
                },
                "required": ["prompt"]
            }),
        }
    }
    fn timeout_override(&self) -> Option<std::time::Duration> {
        Some(self.deps.subagent_timeout)
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
        // Sanitize to a single line so the approval/trace summary can't be
        // corrupted by embedded newlines before truncating to 80 chars.
        let head: String = prompt
            .chars()
            .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
            .take(80)
            .collect();
        // Read: spawning computation is not an effect — every effectful child
        // action is gated by the same policy + approval as the parent (spec D3).
        Ok(ToolIntent {
            tool: "dispatch_agent".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: format!("dispatch sub-agent: {head}"),
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolCtx,
    ) -> Result<ToolOutput, ToolError> {
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .filter(|p| !p.trim().is_empty())
            .ok_or_else(|| ToolError::InvalidArgs("prompt (non-empty string) is required".into()))?
            .to_string();
        let role: Option<String> = match args.get("role") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(s)) => {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    None
                } else if s.chars().count() > MAX_ROLE_CHARS {
                    return Err(ToolError::InvalidArgs(format!(
                        "role must be at most {MAX_ROLE_CHARS} characters"
                    )));
                } else {
                    Some(trimmed.to_string())
                }
            }
            Some(_) => return Err(ToolError::InvalidArgs("role must be a string".into())),
        };
        let allow: Option<Vec<String>> = match args.get("tools") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::Array(a)) => Some(
                a.iter()
                    .map(|v| {
                        v.as_str().map(str::to_string).ok_or_else(|| {
                            ToolError::InvalidArgs("tools must be an array of strings".into())
                        })
                    })
                    .collect::<Result<_, _>>()?,
            ),
            Some(_) => {
                return Err(ToolError::InvalidArgs(
                    "tools must be an array of strings".into(),
                ))
            }
        };
        const IMPLICIT_CHILD_TOOLS: [&str; 1] = ["context_compact"];
        // "dispatch_agent" is a valid allowlist name ONLY while the child could
        // itself dispatch (depth < max_depth). At the depth floor it is unknown
        // — naming it there is an error, keeping the allowlist contract coherent
        // and transitive (spec G7, I-1 resolution).
        let nested_allowed = self.deps.depth < self.deps.max_depth;
        if let Some(names) = &allow {
            let available: Vec<&str> = self.deps.base_tools.iter().map(|t| t.name()).collect();
            for n in names {
                let is_nested = n == "dispatch_agent" && nested_allowed;
                if !available.contains(&n.as_str())
                    && !IMPLICIT_CHILD_TOOLS.contains(&n.as_str())
                    && !is_nested
                {
                    return Err(ToolError::InvalidArgs(format!(
                        "unknown tool '{n}'; available: {}, plus always-available: {}",
                        available.join(", "),
                        IMPLICIT_CHILD_TOOLS.join(", ")
                    )));
                }
            }
        }

        // Mint the dispatch ordinal FIRST: it both stamps forwarded child event
        // ids (`sub{n}:`) and, when depth allows, names the nested tool's prefix
        // so a grandchild's parent_id is this call's visible row id (spec G8).
        let n = next_dispatch_n();

        // Child registry: (filtered) base snapshot + child-bound context tools.
        // dispatch_agent is structurally absent (spec D4: no recursion).
        let mut reg = ToolRegistry::new();
        // The FILTERED base set: exactly the tools the child sees (minus the
        // per-level-fresh context tools and dispatch_agent). Reused below as a
        // nested tool's base so a grandchild cannot exceed the child's scope
        // when an allowlist is in force (spec G7, transitive focus).
        let mut filtered_base: Vec<Arc<dyn Tool>> = Vec::new();
        for t in &self.deps.base_tools {
            // Defense in depth for D4: dispatch_agent is never child-visible,
            // even if a caller leaks it into the base snapshot. No recursion.
            if t.name() == "dispatch_agent" {
                continue;
            }
            if allow
                .as_ref()
                .is_none_or(|names| names.iter().any(|n| n == t.name()))
            {
                filtered_base.push(t.clone());
            }
        }
        for t in &filtered_base {
            reg.register(t.clone());
        }
        // Depth budget (spec G7/G8): a child may dispatch only while under the
        // configured depth AND (no allowlist, or the allowlist names it). The
        // nested tool's id_prefix is THIS call's visible prefix so a grandchild's
        // parent_id is the child row's on-wire id.
        let nested_named = allow
            .as_ref()
            .is_none_or(|names| names.iter().any(|n| n == "dispatch_agent"));
        if nested_allowed && nested_named {
            let mut nested = self.deps.clone();
            nested.depth = self.deps.depth + 1;
            nested.id_prefix = format!("sub{n}:");
            // Transitive scope: when an allowlist filtered the base, the nested
            // tool sees only that filtered set (grandchild ⊆ child). Without an
            // allowlist, the full snapshot passes through unchanged.
            if allow.is_some() {
                nested.base_tools = filtered_base.clone();
            }
            reg.register(Arc::new(DispatchAgentTool::new(nested)));
        }
        let artifacts = Arc::new(SessionArtifacts::new());
        let flag = Arc::new(AtomicBool::new(false));
        // Each dispatch child gets its own ContextCurationMiddleware instance,
        // bound to THIS child's flag (never the parent's) — spec §5.6.
        let curation = Arc::new(ContextCurationMiddleware::new(flag.clone()));
        for c in curation.tools() {
            reg.register(c.tool.clone());
        }
        // Finding 4.1: apply the parent's description overrides to the child
        // registry (registry-level, matching assemble.rs's parent application).
        // Names not in THIS child's registry (e.g. allowlist-filtered tools)
        // just warn — same posture as the parent path.
        reg.set_description_overrides(self.deps.description_overrides.clone());
        let system = match &role {
            Some(r) => format!("{}\n\nRole: {r}", self.deps.child_system_prompt),
            None => self.deps.child_system_prompt.clone(),
        };
        let mut child_ctx = CuratedContext::new(Message::system(system), artifacts.clone(), flag)
            .with_offload_config(OffloadConfig {
                max_result_bytes: self.deps.max_result_bytes,
                ..OffloadConfig::default()
            })
            .with_artifact_prefix(format!("sub{n}-"));

        // Visible parent id: at top level this is the raw call id; nested, the
        // prefix makes it the child row's on-wire id (spec G8 attribution).
        let parent_id = format!("{}{}", self.deps.id_prefix, ctx.call_id);
        let sink = Arc::new(SubagentSink::new(
            self.deps.sink.clone(),
            n,
            parent_id,
            self.deps.child_trace.clone(),
        ));
        let child = AgentLoop::new(
            self.deps.model.clone(),
            self.deps.protocol.clone(),
            Arc::new(reg),
            self.deps.policy.clone(),
            self.deps.approval.clone(),
            sink.clone(),
            self.deps.loop_config.clone(),
        );
        // Own middleware instance per child (spec §5.6): scheduled curation
        // against THIS child's store/flag, not the parent's. StuckDetection is
        // stateless, so a fresh instance per child (rather than sharing the
        // parent's) is just as correct and keeps ownership uniform.
        // Child tools see the guarded composite: the two artifact mounts
        // (read-only) over a HostBackend at the parent workspace root (spec §5.6).
        let child_backend: Arc<dyn agent_tools::backend::Backend> =
            Arc::new(agent_tools::backend::CompositeBackend::new(
                vec![
                    (
                        "large_tool_results/".into(),
                        Arc::new(agent_tools::backend::ReadOnlyToTools(
                            artifacts.results.clone(),
                        )) as Arc<dyn agent_tools::backend::Backend>,
                    ),
                    (
                        "conversation_history/".into(),
                        Arc::new(agent_tools::backend::ReadOnlyToTools(
                            artifacts.history.clone(),
                        )) as Arc<dyn agent_tools::backend::Backend>,
                    ),
                ],
                Arc::new(agent_tools::backend::HostBackend::new(
                    self.deps.loop_config.workspace.clone(),
                )),
            ));
        let child = child
            .with_middleware(vec![curation, Arc::new(StuckDetectionMiddleware)])
            .with_backend(child_backend);
        // Route child-loop compaction through the dedicated model when set.
        let child = match &self.deps.compaction_model {
            Some(m) => child.with_compaction_model(m.clone()),
            None => child,
        };

        // Parent cancel propagates down; child self-cancel never travels up (D8).
        let child_cancel = ctx.cancel.child_token();
        let run = child.run_with_cancel(&mut child_ctx, prompt, child_cancel.clone());
        match tokio::time::timeout(ctx.timeout, run).await {
            Err(_elapsed) => {
                child_cancel.cancel();
                return Ok(failure_output(
                    &sink,
                    format!("sub-agent timed out after {}s", ctx.timeout.as_secs()),
                    "timeout",
                ));
            }
            Ok(Err(e)) => {
                return Ok(failure_output(
                    &sink,
                    format!("sub-agent failed: {e}"),
                    "failed",
                ));
            }
            Ok(Ok(())) => {}
        }
        if ctx.cancel.is_cancelled() {
            return Err(ToolError::Failed {
                message: "sub-agent cancelled".into(),
                stderr: None,
            });
        }

        let s = sink.summary();
        let stop = s.stop.unwrap_or(StopReason::Stop);
        let footer = format!(
            "[sub-agent: {} turns, {} tool calls, stop: {stop:?}]",
            s.turns, s.tool_calls
        );
        let budget_note = matches!(s.stop, Some(StopReason::BudgetExhausted))
            .then_some("[sub-agent hit its turn budget before finishing]");
        // Apply the empty-check to the CHILD text first so a text-less child never
        // emits a stray blank-line run: footer alone, or (budget) note + footer
        // joined by a single newline. With text present: note prefix, the text, a
        // blank line, then the footer.
        let content = if s.final_text.is_empty() {
            match budget_note {
                Some(note) => format!("{note}\n{footer}"),
                None => footer,
            }
        } else {
            match budget_note {
                Some(note) => format!("{note}\n{}\n\n{footer}", s.final_text),
                None => format!("{}\n\n{footer}", s.final_text),
            }
        };
        Ok(ToolOutput {
            content,
            display: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentEvent, ContextEvent, EventSink, ToolStatus};
    use agent_model::StopReason;
    use agent_tools::ToolOutput;
    use std::sync::{Arc, Mutex};

    /// Captures full (kind, id, name, parent) quads — testkit's CollectingSink
    /// drops ids and the parent_id lineage field.
    #[derive(Default)]
    struct FullSink {
        events: Mutex<Vec<(String, String, String, String)>>,
    }
    impl EventSink for FullSink {
        fn emit(&self, event: AgentEvent) {
            let quad = match event {
                AgentEvent::ToolStart {
                    id,
                    name,
                    parent_id,
                    ..
                } => ("tool_start".into(), id, name, parent_id.unwrap_or_default()),
                AgentEvent::ToolResult {
                    id,
                    name,
                    status,
                    parent_id,
                    ..
                } => (
                    format!("tool_result:{}", status.as_str()),
                    id,
                    name,
                    parent_id.unwrap_or_default(),
                ),
                AgentEvent::ServerUsage {
                    prompt_tokens,
                    parent_id,
                    ..
                } => (
                    "server_usage".into(),
                    prompt_tokens.to_string(),
                    String::new(),
                    parent_id.unwrap_or_default(),
                ),
                // Anything else reaching the parent is a forwarding-table bug —
                // record it so the exact-equality assertion below catches the leak.
                _ => (
                    "unexpected".to_string(),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
            };
            self.events.lock().unwrap().push(quad);
        }
    }

    /// Records (ordinal, parent_id, kind-name) for every tapped event.
    #[derive(Default)]
    struct TapSpy {
        seen: Mutex<Vec<(u64, String, &'static str)>>,
    }
    impl SubagentTrace for TapSpy {
        fn record(&self, n: u64, parent_id: &str, event: &AgentEvent) {
            let kind = match event {
                AgentEvent::Token(_) => "token",
                AgentEvent::Reasoning(_) => "reasoning",
                AgentEvent::Usage { .. } => "usage",
                AgentEvent::Done(_) => "done",
                AgentEvent::Error(_) => "error",
                AgentEvent::Context(_) => "context",
                AgentEvent::Approval(_) => "approval",
                AgentEvent::SandboxDegraded { .. } => "sandbox_degraded",
                AgentEvent::StreamRetry { .. } => "stream_retry",
                AgentEvent::RunStart { .. } => "run_start",
                AgentEvent::ToolStart { .. }
                | AgentEvent::ToolResult { .. }
                | AgentEvent::ServerUsage { .. } => "FORWARDED-KIND-MUST-NOT-BE-TAPPED",
            };
            self.seen
                .lock()
                .unwrap()
                .push((n, parent_id.to_string(), kind));
        }
    }

    fn tool_result(id: &str, name: &str) -> AgentEvent {
        AgentEvent::ToolResult {
            id: id.into(),
            name: name.into(),
            status: ToolStatus::Ok,
            output: ToolOutput {
                content: "r".into(),
                display: None,
            },
            duration_ms: 1,
            parent_id: None,
        }
    }

    #[test]
    fn forwards_tool_events_rewritten_and_suppresses_the_rest() {
        let parent = Arc::new(FullSink::default());
        let sink = SubagentSink::new(parent.clone(), 7, "d1".into(), None);
        sink.emit(AgentEvent::Token("hi".into()));
        sink.emit(AgentEvent::Reasoning("r".into()));
        sink.emit(AgentEvent::Usage {
            prompt_tokens: 1,
            context_limit: 10,
            turn: 1,
            max_turns: 5,
        });
        sink.emit(AgentEvent::ToolStart {
            id: "c1".into(),
            name: "echo".into(),
            args: serde_json::json!({}),
            parent_id: None,
        });
        sink.emit(tool_result("c1", "echo"));
        sink.emit(AgentEvent::ServerUsage {
            prompt_tokens: 42,
            completion_tokens: 1,
            reasoning_tokens: None,
            cached_tokens: None,
            cost_usd: None,
            turn_duration_ms: 1,
            turn: 1,
            parent_id: None,
        });
        sink.emit(AgentEvent::Error("boom".into()));
        sink.emit(AgentEvent::Context(ContextEvent::OverflowRecovery));
        sink.emit(AgentEvent::Done(StopReason::Stop));

        let got = parent.events.lock().unwrap().clone();
        // ONLY ToolStart/ToolResult (rewritten) + ServerUsage forwarded, each
        // stamped with the dispatching call's id ("d1") as parent_id.
        assert_eq!(
            got,
            vec![
                (
                    "tool_start".to_string(),
                    "sub7:c1".to_string(),
                    "sub:echo".to_string(),
                    "d1".to_string()
                ),
                (
                    "tool_result:ok".to_string(),
                    "sub7:c1".to_string(),
                    "sub:echo".to_string(),
                    "d1".to_string()
                ),
                (
                    "server_usage".to_string(),
                    "42".to_string(),
                    String::new(),
                    "d1".to_string()
                ),
            ]
        );
    }

    #[test]
    fn forwards_carry_parent_id_and_tap_gets_exactly_the_suppressed_kinds() {
        let parent = Arc::new(FullSink::default());
        let tap = Arc::new(TapSpy::default());
        let sink = SubagentSink::new(parent.clone(), 7, "d1".into(), Some(tap.clone()));
        sink.emit(AgentEvent::Token("hi".into()));
        sink.emit(AgentEvent::ToolStart {
            id: "c1".into(),
            name: "echo".into(),
            args: serde_json::json!({}),
            parent_id: None,
        });
        sink.emit(tool_result("c1", "echo"));
        sink.emit(AgentEvent::ServerUsage {
            prompt_tokens: 42,
            completion_tokens: 1,
            reasoning_tokens: None,
            cached_tokens: None,
            cost_usd: None,
            turn_duration_ms: 1,
            turn: 1,
            parent_id: None,
        });
        sink.emit(AgentEvent::Error("boom".into()));
        sink.emit(AgentEvent::Done(StopReason::Stop));

        // Forwards stamped with the dispatch call id (even though the child emitted None):
        let got = parent.events.lock().unwrap().clone();
        assert_eq!(
            got[0],
            (
                "tool_start".to_string(),
                "sub7:c1".to_string(),
                "sub:echo".to_string(),
                "d1".to_string()
            )
        );
        assert_eq!(got[1].3, "d1");
        assert_eq!(got[2].3, "d1"); // server_usage
                                    // Tap saw exactly the non-forwarded kinds, attributed to ordinal 7 and
                                    // stamped with the dispatch call id "d1" (the zero-tool-call join key):
        assert_eq!(
            tap.seen.lock().unwrap().clone(),
            vec![
                (7, "d1".to_string(), "token"),
                (7, "d1".to_string(), "error"),
                (7, "d1".to_string(), "done"),
            ]
        );
    }

    #[test]
    fn no_tap_means_no_panic_and_capture_still_works() {
        let sink = SubagentSink::new(Arc::new(FullSink::default()), 1, "d1".into(), None);
        sink.emit(AgentEvent::Token("t".into()));
        sink.emit(AgentEvent::Done(StopReason::Stop));
        assert_eq!(sink.summary().final_text, "t");
    }

    #[test]
    fn summary_final_text_is_tail_after_last_tool_result() {
        let sink = SubagentSink::new(Arc::new(FullSink::default()), 1, "d1".into(), None);
        sink.emit(AgentEvent::Token("thinking...".into()));
        sink.emit(tool_result("c1", "echo"));
        sink.emit(AgentEvent::Token("final ".into()));
        sink.emit(AgentEvent::Token("answer".into()));
        sink.emit(AgentEvent::Done(StopReason::Stop));
        let s = sink.summary();
        assert_eq!(s.final_text, "final answer");
        assert_eq!(s.tool_calls, 0); // no ToolStart was emitted
        assert_eq!(s.stop, Some(StopReason::Stop));
    }

    #[test]
    fn stream_retry_retracts_abandoned_partial_text_from_capture() {
        // A child stream dies mid-answer (StreamRetry retracts the partial),
        // then re-streams: the captured result must hold only the post-retry
        // text — the abandoned partial must not leak to the parent model.
        let sink = SubagentSink::new(Arc::new(FullSink::default()), 1, "d1".into(), None);
        sink.emit(AgentEvent::Token("partial ".into()));
        sink.emit(AgentEvent::Token("junk".into())); // 12 chars streamed this attempt
        sink.emit(AgentEvent::StreamRetry {
            discarded_text_chars: 12,
            discarded_reasoning_chars: 0,
        });
        sink.emit(AgentEvent::Token("real answer".into()));
        sink.emit(AgentEvent::Done(StopReason::Stop));
        assert_eq!(sink.summary().final_text, "real answer");
    }

    #[test]
    fn summary_falls_back_to_all_text_when_tail_is_blank() {
        let sink = SubagentSink::new(Arc::new(FullSink::default()), 1, "d1".into(), None);
        sink.emit(AgentEvent::Token("early words".into()));
        sink.emit(tool_result("c1", "echo"));
        // no tokens after the last tool result
        let s = sink.summary();
        assert_eq!(s.final_text, "early words");
    }

    #[test]
    fn summary_counts_tool_calls_and_turns() {
        let sink = SubagentSink::new(Arc::new(FullSink::default()), 1, "d1".into(), None);
        sink.emit(AgentEvent::Usage {
            prompt_tokens: 1,
            context_limit: 10,
            turn: 1,
            max_turns: 5,
        });
        sink.emit(AgentEvent::ToolStart {
            id: "c1".into(),
            name: "a".into(),
            args: serde_json::json!({}),
            parent_id: None,
        });
        sink.emit(AgentEvent::ToolStart {
            id: "c2".into(),
            name: "b".into(),
            args: serde_json::json!({}),
            parent_id: None,
        });
        sink.emit(AgentEvent::Usage {
            prompt_tokens: 2,
            context_limit: 10,
            turn: 2,
            max_turns: 5,
        });
        let s = sink.summary();
        assert_eq!(s.tool_calls, 2);
        assert_eq!(s.turns, 2);
    }

    #[test]
    fn dispatch_ordinals_are_unique() {
        let a = next_dispatch_n();
        let b = next_dispatch_n();
        assert_ne!(a, b);
    }

    // --- Footer formatting pins (empty-text / budget-exhausted) -------------
    use crate::testkit::{AlwaysApprove, PassthroughProtocol, Scripted, ScriptedModel};
    use crate::LoopConfig;
    use agent_tools::ToolCtx;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn exec_deps(model: ScriptedModel, max_turns: usize) -> DispatchDeps {
        let ws = std::env::temp_dir();
        DispatchDeps {
            model: Arc::new(model),
            protocol: Arc::new(PassthroughProtocol),
            policy: Arc::new(agent_policy::RulePolicy {
                workspace: ws.clone(),
                command_allowlist: vec![],
                command_denylist: vec![],
            }),
            approval: Arc::new(AlwaysApprove),
            sink: Arc::new(FullSink::default()),
            child_trace: None,
            base_tools: vec![],
            child_system_prompt: "SYS".into(),
            loop_config: LoopConfig {
                model_limit: 16384,
                max_turns,
                max_retries: 1,
                tool_timeout: Duration::from_secs(5),
                stream_idle_timeout: Duration::from_secs(3600),
                workspace: ws,
                ..LoopConfig::default()
            },
            max_result_bytes: 16 * 1024,
            subagent_timeout: Duration::from_secs(600),
            compaction_model: None,
            depth: 1,
            max_depth: 1,
            id_prefix: String::new(),
            description_overrides: Default::default(),
        }
    }

    fn exec_ctx() -> ToolCtx {
        ToolCtx {
            workspace: std::env::temp_dir(),
            timeout: Duration::from_secs(600),
            cancel: CancellationToken::new(),
            sandbox: Arc::new(agent_tools::HostExecutor),
            backend: Arc::new(agent_tools::backend::HostBackend::new(std::env::temp_dir())),
            call_id: "d1".into(),
        }
    }

    #[tokio::test]
    async fn budget_exhausted_with_no_text_has_no_blank_line_run() {
        // Child burns its 1-turn budget on a (denied) tool call and emits zero
        // Token text. The budget note + footer must join without a stray blank
        // line (no "\n\n\n"); the old prepend-then-append path produced one.
        let tool = DispatchAgentTool::new(exec_deps(
            ScriptedModel::new(vec![
                Scripted::Call("c1".into(), "nope".into(), "{}".into()),
                Scripted::Call("c2".into(), "nope".into(), "{}".into()),
            ]),
            1,
        ));
        let out = tool
            .execute(serde_json::json!({"prompt": "p"}), &exec_ctx())
            .await
            .unwrap();
        assert!(out.content.contains("turn budget"), "{:?}", out.content);
        assert!(out.content.contains("[sub-agent:"), "{:?}", out.content);
        assert!(!out.content.contains("\n\n\n"), "{:?}", out.content);
    }

    #[tokio::test]
    async fn empty_text_child_returns_footer_alone_without_leading_whitespace() {
        // Non-budget empty child: footer alone, no leading blank line/whitespace.
        let tool = DispatchAgentTool::new(exec_deps(
            ScriptedModel::new(vec![Scripted::Text(String::new())]),
            5,
        ));
        let out = tool
            .execute(serde_json::json!({"prompt": "p"}), &exec_ctx())
            .await
            .unwrap();
        assert!(out.content.starts_with("[sub-agent:"), "{:?}", out.content);
        assert!(out.content.contains("stop: Stop"), "{:?}", out.content);
        assert!(
            !out.content.starts_with(char::is_whitespace),
            "{:?}",
            out.content
        );
        assert!(!out.content.contains("\n\n\n"), "{:?}", out.content);
    }

    /// Finding 4.1: registry-level description overrides must reach the CHILD
    /// registry too (the seam spec's uniformity claim). context_compact is
    /// always-registered for children, so overriding it needs no base tool.
    #[tokio::test]
    async fn description_overrides_reach_child_registry() {
        struct SchemaCapturingModel {
            inner: ScriptedModel,
            seen: std::sync::Mutex<Vec<(String, String)>>,
        }
        #[async_trait::async_trait]
        impl agent_model::ModelClient for SchemaCapturingModel {
            async fn stream(
                &self,
                req: agent_model::CompletionRequest,
            ) -> Result<
                futures::stream::BoxStream<
                    'static,
                    Result<agent_model::Chunk, agent_model::ModelError>,
                >,
                agent_model::ModelError,
            > {
                self.seen.lock().unwrap().extend(
                    req.tools
                        .iter()
                        .map(|t| (t.name.clone(), t.description.clone())),
                );
                self.inner.stream(req).await
            }
        }
        let model = Arc::new(SchemaCapturingModel {
            inner: ScriptedModel::new(vec![Scripted::Text("x".into())]),
            seen: Default::default(),
        });
        let mut d = exec_deps(ScriptedModel::new(vec![]), 5);
        d.model = model.clone();
        d.description_overrides =
            [("context_compact".to_string(), "OVERRIDDEN".to_string())].into();
        // Clone-propagation pin: nested deps are self.deps.clone() in execute().
        assert_eq!(
            d.clone().description_overrides.get("context_compact"),
            Some(&"OVERRIDDEN".to_string())
        );
        let tool = DispatchAgentTool::new(d);
        tool.execute(serde_json::json!({"prompt": "p"}), &exec_ctx())
            .await
            .unwrap();
        let seen = model.seen.lock().unwrap();
        assert!(
            seen.iter()
                .any(|(n, desc)| n == "context_compact" && desc.starts_with("OVERRIDDEN")),
            "child request schemas must carry the override: {seen:?}"
        );
    }

    // --- Task 7: child-stack invariant pin -----------------------------------

    /// A trivial memory-shaped tool: exercises "memory tools present in
    /// base_tools" without any real agent-memory coupling. Named `remember`
    /// so it reads unambiguously as a memory tool in the captured schema list.
    struct RememberStub;
    #[async_trait::async_trait]
    impl Tool for RememberStub {
        fn name(&self) -> &str {
            "remember"
        }
        fn description(&self) -> &str {
            "remember a fact (test stub)"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: "remember".into(),
                description: "".into(),
                parameters: serde_json::json!({"type":"object"}),
            }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent {
                tool: "remember".into(),
                access: Access::Write,
                paths: vec![],
                command: None,
                summary: "remember".into(),
            })
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput {
                content: "remembered".into(),
                display: None,
            })
        }
    }

    /// The normative claim (task-7 brief): children run `[context-curation,
    /// stuck-detection]`, never `memory-recall`. Behavioral evidence over new
    /// `#[cfg(test)]` surface (per the brief's stated preference), reusing the
    /// `SchemaCapturingModel` shape from `description_overrides_reach_child_registry`:
    ///
    /// (a) tool-surface evidence: a memory tool placed in `base_tools` (the
    ///     spec D4/§5.6 channel real memory tools use — `ToolContribution {
    ///     child_visible: true }` in the assembled parent's base snapshot)
    ///     reaches the child's registered tool schemas alongside the
    ///     context-curation tool (`context_compact`), proving
    ///     `filtered_base` + `curation.tools()` both land in the child registry.
    /// (b) stuck-detection is LIVE: three identical calls trip the spec §5.5
    ///     nudge inside the child's own turn loop (only reachable if
    ///     `StuckDetectionMiddleware` is actually in the child's stack).
    /// (c) memory-recall is ABSENT: even with a memory tool visible, nothing
    ///     injects a "Relevant memories" recall block into the child's own
    ///     completion requests (`MemoryRecallMiddleware::on_run_start` is the
    ///     only source of that block, and it is never constructed in
    ///     `DispatchAgentTool::execute`) — the strongest observable proxy for
    ///     "never memory-recall" without a #[cfg(test)] stack accessor.
    #[tokio::test]
    async fn child_stack_is_exactly_curation_and_stuck_detection_never_memory_recall() {
        struct SchemaCapturingModel {
            inner: ScriptedModel,
            seen: std::sync::Mutex<Vec<(String, String)>>,
            /// Every request's full system+user text, to prove no recall block
            /// ever appears anywhere in the child's own model traffic.
            request_texts: std::sync::Mutex<Vec<String>>,
        }
        #[async_trait::async_trait]
        impl agent_model::ModelClient for SchemaCapturingModel {
            async fn stream(
                &self,
                req: agent_model::CompletionRequest,
            ) -> Result<
                futures::stream::BoxStream<
                    'static,
                    Result<agent_model::Chunk, agent_model::ModelError>,
                >,
                agent_model::ModelError,
            > {
                self.seen.lock().unwrap().extend(
                    req.tools
                        .iter()
                        .map(|t| (t.name.clone(), t.description.clone())),
                );
                self.request_texts.lock().unwrap().push(
                    req.messages
                        .iter()
                        .map(|m| m.content.clone())
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
                self.inner.stream(req).await
            }
        }
        // Three identical write_stub-style calls (via `remember`) to trip the
        // child's own stuck-detection nudge on the 3rd turn, then a text exit.
        let one = || Scripted::Call("c1".into(), "remember".into(), r#"{"k":"a"}"#.into());
        let model = Arc::new(SchemaCapturingModel {
            inner: ScriptedModel::new(vec![one(), one(), one(), Scripted::Text("done".into())]),
            seen: Default::default(),
            request_texts: Default::default(),
        });
        let mut d = exec_deps(ScriptedModel::new(vec![]), 10);
        d.model = model.clone();
        d.base_tools = vec![Arc::new(RememberStub)];
        let tool = DispatchAgentTool::new(d);
        tool.execute(serde_json::json!({"prompt": "p"}), &exec_ctx())
            .await
            .unwrap();

        // (a) tool-surface: the memory tool AND the context-curation tool
        // reached the child's registered schemas.
        let seen = model.seen.lock().unwrap();
        let seen_names: std::collections::HashSet<&str> =
            seen.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            seen_names.contains("remember"),
            "memory tool from base_tools must reach the child registry: {seen_names:?}"
        );
        assert!(
            seen_names.contains("context_compact"),
            "context-curation's tool must reach the child registry: {seen_names:?}"
        );
        assert!(
            !seen_names.contains("dispatch_agent"),
            "dispatch_agent must never be child-visible (spec D4): {seen_names:?}"
        );

        // (b) stuck-detection is live in the child's own loop: 4 requests sent
        // (3 identical + 1 differing after the nudge), matching the top-level
        // "nudge on 3rd, no abort within budget" shape.
        assert_eq!(
            model.request_texts.lock().unwrap().len(),
            4,
            "child model must be consulted once per turn (3 calls + 1 text exit)"
        );
        let last_request = model.request_texts.lock().unwrap().last().unwrap().clone();
        assert!(
            last_request.contains("identical tool call"),
            "the child's own stuck-detection nudge must appear in its own request \
             history, proving StuckDetectionMiddleware ran inside the child: {last_request}"
        );

        // (c) memory-recall is absent: no request ever carries a recall block,
        // even though a memory tool was visible and callable.
        for (i, text) in model.request_texts.lock().unwrap().iter().enumerate() {
            assert!(
                !text.contains("Relevant memories from past sessions"),
                "request {i} must carry no memory-recall block — MemoryRecallMiddleware \
                 is never installed on a dispatch child: {text}"
            );
        }
    }
}
