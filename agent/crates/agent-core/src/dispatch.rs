//! Sub-agent dispatch: sub-agents-as-tools (spec 2026-07-01-subagent-dispatch-core).
use crate::{
    AgentEvent, AgentLoop, ContextCurationMiddleware, CuratedContext, EventSink, LoopConfig,
    Middleware, OffloadConfig, RepairMiddleware, RespondTool, ResponseCapture, ResponseHandle,
    SessionArtifacts, StuckDetectionMiddleware, TodoHandle, ToolCallLimit, WriteTodosTool,
    RESPOND_TOOL_NAME,
};
use agent_model::{Message, ModelClient, StopReason, ToolCallProtocol};
use agent_policy::{
    ApprovalChannel, PermissionLists, PolicyEngine, SubAgentPolicy, ToolPermissions,
};
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

/// Appended to a named child's composed system prompt when its spec declares a
/// `response_format` (spec 3B-1b §2.2): the child returns its result by calling
/// the `respond` tool, not in prose.
pub const RESPONSE_FORMAT_CLAUSE: &str = "You MUST finish this task by calling the \
`respond` tool exactly once, passing your final answer as its arguments in the shape \
the tool's schema requires. Do not put your final answer in prose — only the \
`respond` call is returned to the parent. If a `respond` call is rejected as invalid, \
read the error and call `respond` again with corrected arguments.";

/// Upper bound on the `role` arg (system-prompt injection; spec G6).
pub const MAX_ROLE_CHARS: usize = 2000;

static DISPATCH_ORDINAL: AtomicU64 = AtomicU64::new(1);

/// Process-wide dispatch ordinal: keeps forwarded child event ids unique across
/// parallel siblings and across the parent's own tool-call ids (spec D9).
pub fn next_dispatch_n() -> u64 {
    DISPATCH_ORDINAL.fetch_add(1, Ordering::Relaxed)
}

/// A `SubAgentSpec` resolved at assembly into everything the dispatch tool needs
/// to spawn a named child (models are pre-built here because agent-core cannot
/// call `build_routed_model`). Spec §2.1/§2.4/§2.5.
pub struct ResolvedSubAgent {
    pub description: String,
    /// Already includes `SUBAGENT_PREAMBLE` (composed at assembly).
    pub system_prompt: String,
    pub tools: Option<Vec<String>>,
    pub model: Arc<dyn ModelClient>,
    pub protocol: Arc<dyn ToolCallProtocol>,
    pub model_limit: Option<usize>,
    pub max_tokens: Option<u32>,
    pub tool_call_limit: Option<usize>,
    /// The resolved flat `response_format` schema (spec 3B-1b §2.1); `None` ⇒ the
    /// child returns free prose as today.
    pub response_format: Option<serde_json::Value>,
    /// RAW floor lists (3B-1c §2.5) — parsed at dispatch, not at assembly; `None`
    /// ⇒ the child gets the caller's policy Arc untouched. Assembly normalizes
    /// empty blocks to `None`, so `Some` is non-empty by construction.
    pub permissions: Option<PermissionLists>,
}

/// Dispatch-facing named sub-agent registry (spec §2.2). `general-purpose` is
/// implicit (the default ad-hoc path) and never stored here.
#[derive(Default)]
pub struct SubAgentRegistry {
    map: std::collections::HashMap<String, ResolvedSubAgent>,
}

impl SubAgentRegistry {
    pub fn from_map(map: std::collections::HashMap<String, ResolvedSubAgent>) -> Self {
        Self { map }
    }
    pub fn get(&self, name: &str) -> Option<&ResolvedSubAgent> {
        self.map.get(name)
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    pub fn names(&self) -> Vec<&str> {
        self.map.keys().map(String::as_str).collect()
    }
    /// `(name, description)` pairs for the `subagent_type` enum docs.
    pub fn schema_hints(&self) -> Vec<(String, String)> {
        self.map
            .iter()
            .map(|(n, r)| (n.clone(), r.description.clone()))
            .collect()
    }
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
    /// Named sub-agent registry (spec §2.2). Empty ⇒ only `general-purpose`
    /// exists and the tool schema is byte-identical to 3A.
    pub subagents: Arc<SubAgentRegistry>,
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

    /// 3B-1c §2.5/§2.6: the child's effective policy. Named child with floors →
    /// SubAgentPolicy over the CALLER'S effective policy (monotone down chains);
    /// everything else → the same Arc (byte-identical). Parse failure = the
    /// named child is undispatchable (fail-closed; the only dialect gate on the
    /// lenient-boot path, where validate() never ran).
    fn child_policy(
        &self,
        subagent_type: &str,
        resolved: Option<&ResolvedSubAgent>,
    ) -> Result<Arc<dyn PolicyEngine>, ToolError> {
        match resolved.and_then(|r| r.permissions.as_ref()) {
            Some(raw) => {
                let rules =
                    ToolPermissions::parse(subagent_type, &raw.deny, &raw.ask).map_err(|e| {
                        ToolError::InvalidArgs(format!(
                            "named sub-agent '{subagent_type}': invalid permissions: {e}"
                        ))
                    })?;
                Ok(Arc::new(SubAgentPolicy::new(
                    self.deps.policy.clone(),
                    rules,
                )))
            }
            None => Ok(self.deps.policy.clone()),
        }
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
        let mut properties = serde_json::json!({
            "prompt": {
                "type": "string",
                "description": "The complete, self-contained task for the sub-agent: goal, relevant paths/facts, and what to return."
            },
            "tools": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Optional allowlist restricting which tools the sub-agent may use (default: all). For focus, not safety — permissions are inherited either way. The child's context tool (context_compact) is always available. Include dispatch_agent to let the sub-agent dispatch its own (only meaningful when nesting depth allows); the restriction applies transitively to its children. Ignored when subagent_type names a registered sub-agent."
            },
            "role": {
                "type": "string",
                "description": "Optional persona/role instructions injected into the sub-agent's system prompt (stronger steering than putting them in the prompt). Max 2000 characters. Ignored when subagent_type names a registered sub-agent."
            }
        });
        // Fix M1: expose registered sub-agents as a typed enum so the model can
        // ROUTE to the right one. Present ONLY when the registry is non-empty, so
        // an empty registry keeps the schema byte-identical to 3A.
        if !self.deps.subagents.is_empty() {
            let mut hints = self.deps.subagents.schema_hints();
            hints.sort_by(|a, b| a.0.cmp(&b.0));
            let mut variants: Vec<String> = vec!["general-purpose".into()];
            let mut doc = String::from(
                "Which sub-agent to dispatch. 'general-purpose' inherits your tools/model and honors `role`/`tools`. Registered sub-agents (use their own prompt/tools/model, and IGNORE `role`/`tools`): ",
            );
            for (i, (name, desc)) in hints.iter().enumerate() {
                variants.push(name.clone());
                if i > 0 {
                    doc.push_str("; ");
                }
                doc.push_str(&format!("{name} — {desc}"));
            }
            properties["subagent_type"] = serde_json::json!({
                "type": "string",
                "enum": variants,
                "default": "general-purpose",
                "description": doc,
            });
        }
        ToolSchema {
            name: "dispatch_agent".into(),
            description: self.description().into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": properties,
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
        let subagent_type = args
            .get("subagent_type")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| "general-purpose".to_string());
        let resolved: Option<&ResolvedSubAgent> = if subagent_type == "general-purpose" {
            None
        } else {
            match self.deps.subagents.get(&subagent_type) {
                Some(r) => Some(r),
                None => {
                    let mut names: Vec<&str> = self.deps.subagents.names();
                    names.sort_unstable();
                    return Err(ToolError::InvalidArgs(format!(
                        "unknown subagent_type '{subagent_type}'; registered: general-purpose, {}",
                        names.join(", ")
                    )));
                }
            }
        };
        // Child policy BEFORE nested-deps cloning (spec §2.5 ordering requirement:
        // grandchildren must gate against THIS child's effective policy, or a
        // denied child could delegate around its floor).
        let child_policy = self.child_policy(&subagent_type, resolved)?;
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
        // Named type: its allowlist REPLACES per-call tools (deepagents replace
        // semantics). general-purpose keeps the per-call `allow` computed above.
        let allow = match resolved {
            Some(r) => r.tools.clone(),
            None => allow,
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
            nested.policy = child_policy.clone();
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
        // Per-child todos handle (deepagents contract, spec §5.6): fresh, never
        // the parent's. If write_todos is in the child's filtered base snapshot
        // (a parent-bound instance reached via child_visible = true), rebind it
        // to the child's OWN handle (last-wins registry) so the child plans for
        // itself and its plan is never merged back to the parent. Guardrails and
        // TodoListMiddleware stay OUT of the child stack — only the tool rebinds.
        let todos: TodoHandle = Arc::new(Mutex::new(Vec::new()));
        if filtered_base.iter().any(|t| t.name() == "write_todos") {
            reg.register(Arc::new(WriteTodosTool::new(todos.clone())));
        }
        // Structured response (3B-1b §2.2): a named child with a response_format gets
        // a synthetic `respond` tool, registered DIRECTLY here (exempt from the `tools`
        // allowlist, like the context tools) so it is always callable. The handle is
        // read back at the handoff and by ResponseCapture.
        let response_handle: ResponseHandle = Arc::new(Mutex::new(None));
        let response_schema: Option<serde_json::Value> =
            resolved.and_then(|r| r.response_format.clone());
        if let Some(schema) = response_schema.clone() {
            // Resolved-registry collision guard (spec §2.2): no base/injected tool may
            // already own the reserved name. Benign by construction today (register is
            // last-wins, so `respond` would win anyway, and no host/context tool is
            // named `respond`), but assert it so a future collision fails loudly in
            // tests rather than silently shadowing a real tool.
            debug_assert!(
                reg.get(RESPOND_TOOL_NAME).is_none(),
                "reserved tool name `{RESPOND_TOOL_NAME}` collides with an existing child tool",
            );
            reg.register(Arc::new(RespondTool::new(schema, response_handle.clone())));
        }
        // Finding 4.1: apply the parent's description overrides to the child
        // registry (registry-level, matching assemble.rs's parent application).
        // Names not in THIS child's registry (e.g. allowlist-filtered tools)
        // just warn — same posture as the parent path.
        reg.set_description_overrides(self.deps.description_overrides.clone());
        // Named type replaces the parent-derived prompt (preamble already baked
        // into resolved.system_prompt) and ignores `role`. general-purpose keeps
        // the parent child_system_prompt + optional role (byte-identical to 3A).
        let system = match resolved {
            Some(r) => r.system_prompt.clone(),
            None => match &role {
                Some(rl) => format!("{}\n\nRole: {rl}", self.deps.child_system_prompt),
                None => self.deps.child_system_prompt.clone(),
            },
        };
        let mut child_ctx = CuratedContext::new(Message::system(system), artifacts.clone(), flag)
            .with_offload_config(OffloadConfig {
                max_result_bytes: self.deps.max_result_bytes,
                ..OffloadConfig::default()
            })
            .with_artifact_prefix(format!("sub{n}-"))
            .with_todos(todos);

        // Visible parent id: at top level this is the raw call id; nested, the
        // prefix makes it the child row's on-wire id (spec G8 attribution).
        let parent_id = format!("{}{}", self.deps.id_prefix, ctx.call_id);
        let sink = Arc::new(SubagentSink::new(
            self.deps.sink.clone(),
            n,
            parent_id,
            self.deps.child_trace.clone(),
        ));
        // Named type may route its own model/protocol/window; general-purpose
        // uses the parent-configured child defaults (byte-identical to 3A).
        let (child_model, child_protocol, child_loop_config) = match resolved {
            Some(r) => {
                let mut lc = self.deps.loop_config.clone();
                if let Some(ml) = r.model_limit {
                    lc.model_limit = ml;
                }
                if r.max_tokens.is_some() {
                    lc.max_tokens = r.max_tokens;
                }
                (r.model.clone(), r.protocol.clone(), lc)
            }
            None => (
                self.deps.model.clone(),
                self.deps.protocol.clone(),
                self.deps.loop_config.clone(),
            ),
        };
        let child = AgentLoop::new(
            child_model,
            child_protocol,
            Arc::new(reg),
            child_policy.clone(),
            self.deps.approval.clone(),
            sink.clone(),
            child_loop_config,
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
        // 3A default child stack; a named type with tool_call_limit appends the
        // 3A ToolCallLimit guardrail (E4). Aborts via EndRun(StopReason::Error).
        let mut child_mw: Vec<Arc<dyn Middleware>> = vec![
            curation,
            Arc::new(StuckDetectionMiddleware),
            Arc::new(RepairMiddleware),
        ];
        if let Some(cap) = resolved.and_then(|r| r.tool_call_limit) {
            child_mw.push(Arc::new(ToolCallLimit::with_cap(cap)));
        }
        // LAST in the vec ⇒ FIRST under fire_after_tools' reverse iteration ⇒ a
        // captured response wins a same-turn ToolCallLimit trip (reports Stop). §2.3.
        if response_schema.is_some() {
            child_mw.push(Arc::new(ResponseCapture::new(response_handle.clone())));
        }
        let child = child.with_middleware(child_mw).with_backend(child_backend);
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
        let content = if let Some(payload) = response_handle.lock().unwrap().take() {
            // §2.4 Some: single-line JSON payload (line 1), footer on later lines;
            // the child's pre-`respond` prose (final_text) is SEVERED. budget_note is
            // intentionally omitted: a captured payload means ResponseCapture ended the
            // run with Stop, so s.stop is never BudgetExhausted on this branch.
            let body = serde_json::to_string(&payload).unwrap_or_else(|_| "null".into());
            format!("{body}\n\n{footer}")
        } else if response_schema.is_some() {
            // §2.4 None + response_format set: marked, distinguishable free-text fallback.
            let marker = "[response_format: UNSATISFIED — free-text fallback]";
            match (s.final_text.is_empty(), budget_note) {
                (true, Some(note)) => format!("{note}\n{marker}\n{footer}"),
                (true, None) => format!("{marker}\n{footer}"),
                (false, Some(note)) => format!("{note}\n{}\n\n{marker}\n{footer}", s.final_text),
                (false, None) => format!("{}\n\n{marker}\n{footer}", s.final_text),
            }
        } else {
            // No response_format → byte-identical to 3B-1.
            if s.final_text.is_empty() {
                match budget_note {
                    Some(note) => format!("{note}\n{footer}"),
                    None => footer,
                }
            } else {
                match budget_note {
                    Some(note) => format!("{note}\n{}\n\n{footer}", s.final_text),
                    None => format!("{}\n\n{footer}", s.final_text),
                }
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
            subagents: Arc::new(SubAgentRegistry::default()),
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

    fn rf_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object", "additionalProperties": false,
            "required": ["summary"],
            "properties": {"summary": {"type": "string"}}
        })
    }

    fn resolved_with(
        rf: Option<serde_json::Value>,
        child: ScriptedModel,
        tcl: Option<usize>,
    ) -> std::collections::HashMap<String, ResolvedSubAgent> {
        let mut m = std::collections::HashMap::new();
        m.insert(
            "triage".to_string(),
            ResolvedSubAgent {
                description: "Triage".into(),
                system_prompt: "You triage.".into(),
                tools: None,
                model: Arc::new(child),
                protocol: Arc::new(PassthroughProtocol),
                model_limit: None,
                max_tokens: None,
                tool_call_limit: tcl,
                response_format: rf,
                permissions: None,
            },
        );
        m
    }

    #[tokio::test]
    async fn named_child_response_format_returns_severed_payload() {
        let child = ScriptedModel::new(vec![Scripted::Call(
            "c1".into(),
            "respond".into(),
            r#"{"summary":"done"}"#.into(),
        )]);
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 3);
        deps.subagents = Arc::new(SubAgentRegistry::from_map(resolved_with(
            Some(rf_schema()),
            child,
            None,
        )));
        let tool = DispatchAgentTool::new(deps);
        let out = tool
            .execute(
                serde_json::json!({"prompt":"go","subagent_type":"triage"}),
                &exec_ctx(),
            )
            .await
            .unwrap();
        let line1 = out.content.lines().next().unwrap();
        let v: serde_json::Value = serde_json::from_str(line1).expect("line 1 is JSON");
        assert_eq!(v["summary"], "done");
        assert!(
            !out.content.contains("You triage"),
            "child prose must be severed"
        );
    }

    #[tokio::test]
    async fn named_child_response_format_severs_pre_respond_prose() {
        // Child reasons aloud (assistant text) in the SAME turn it calls `respond`
        // (a real tool-calling model can emit text alongside tool_calls in one
        // turn — Scripted::Text/Call would instead stop the loop after the text
        // turn via StopReason::Stop, never reaching the call). The prose must
        // NOT appear in the handoff — only the JSON payload + footer (sever).
        use agent_model::{Chunk, RawToolCall};
        let child = ScriptedModel::new(vec![Scripted::Chunks(vec![
            Chunk::Text("Let me think... the severity is clearly low.".into()),
            Chunk::ToolCallDelta(RawToolCall {
                index: None,
                id: Some("c1".into()),
                name: Some("respond".into()),
                args_fragment: r#"{"summary":"done"}"#.into(),
            }),
            Chunk::Done(StopReason::ToolCalls),
        ])]);
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 4);
        deps.subagents = Arc::new(SubAgentRegistry::from_map(resolved_with(
            Some(rf_schema()),
            child,
            None,
        )));
        let out = DispatchAgentTool::new(deps)
            .execute(
                serde_json::json!({"prompt":"go","subagent_type":"triage"}),
                &exec_ctx(),
            )
            .await
            .unwrap();
        // Line 1 is the JSON payload.
        let v: serde_json::Value =
            serde_json::from_str(out.content.lines().next().unwrap()).unwrap();
        assert_eq!(v["summary"], "done");
        // The child's pre-respond prose is severed — absent from the ENTIRE handoff.
        assert!(
            !out.content.contains("Let me think"),
            "pre-respond prose leaked: {}",
            out.content
        );
        assert!(
            !out.content.contains("severity is clearly low"),
            "pre-respond prose leaked: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn invalid_respond_retries_then_succeeds() {
        let child = ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "respond".into(), r#"{"wrong":1}"#.into()),
            Scripted::Call("c2".into(), "respond".into(), r#"{"summary":"ok"}"#.into()),
        ]);
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 4);
        deps.subagents = Arc::new(SubAgentRegistry::from_map(resolved_with(
            Some(rf_schema()),
            child,
            None,
        )));
        let out = DispatchAgentTool::new(deps)
            .execute(
                serde_json::json!({"prompt":"go","subagent_type":"triage"}),
                &exec_ctx(),
            )
            .await
            .unwrap();
        let v: serde_json::Value =
            serde_json::from_str(out.content.lines().next().unwrap()).unwrap();
        assert_eq!(v["summary"], "ok");
    }

    #[tokio::test]
    async fn no_valid_respond_yields_marked_fallback() {
        let child = ScriptedModel::new(vec![Scripted::Text("prose answer, no tool".into())]);
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 2);
        deps.subagents = Arc::new(SubAgentRegistry::from_map(resolved_with(
            Some(rf_schema()),
            child,
            None,
        )));
        let out = DispatchAgentTool::new(deps)
            .execute(
                serde_json::json!({"prompt":"go","subagent_type":"triage"}),
                &exec_ctx(),
            )
            .await
            .unwrap();
        assert!(out.content.contains("[response_format: UNSATISFIED"));
        assert!(
            serde_json::from_str::<serde_json::Value>(out.content.lines().next().unwrap()).is_err()
        );
    }

    #[tokio::test]
    async fn respond_reachable_under_empty_tools_allowlist() {
        let child = ScriptedModel::new(vec![Scripted::Call(
            "c1".into(),
            "respond".into(),
            r#"{"summary":"x"}"#.into(),
        )]);
        let mut m = resolved_with(Some(rf_schema()), child, None);
        m.get_mut("triage").unwrap().tools = Some(vec![]); // allowlist omits respond
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 3);
        deps.subagents = Arc::new(SubAgentRegistry::from_map(m));
        let out = DispatchAgentTool::new(deps)
            .execute(
                serde_json::json!({"prompt":"go","subagent_type":"triage"}),
                &exec_ctx(),
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(out.content.lines().next().unwrap()).unwrap()
                ["summary"],
            "x"
        );
    }

    #[tokio::test]
    async fn capture_wins_same_turn_tool_call_limit() {
        // tool_call_limit = 1: the respond call is the 1st (and cap-tripping) call.
        // ResponseCapture (pushed last) must win → footer reports Stop, not Error.
        let child = ScriptedModel::new(vec![Scripted::Call(
            "c1".into(),
            "respond".into(),
            r#"{"summary":"done"}"#.into(),
        )]);
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 3);
        deps.subagents = Arc::new(SubAgentRegistry::from_map(resolved_with(
            Some(rf_schema()),
            child,
            Some(1),
        )));
        let out = DispatchAgentTool::new(deps)
            .execute(
                serde_json::json!({"prompt":"go","subagent_type":"triage"}),
                &exec_ctx(),
            )
            .await
            .unwrap();
        assert!(
            out.content.contains("stop: Stop"),
            "capture must report Stop: {}",
            out.content
        );
        assert!(!out.content.contains("stop: Error"));
        // Precedence winning must also sever the payload on the same turn, not just
        // fix the stop-reason: line 1 is the JSON, not prose.
        let v: serde_json::Value =
            serde_json::from_str(out.content.lines().next().unwrap()).unwrap();
        assert_eq!(v["summary"], "done");
    }

    #[tokio::test]
    async fn named_child_without_response_format_is_byte_identical() {
        let child = ScriptedModel::new(vec![Scripted::Text("plain answer".into())]);
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 3);
        deps.subagents = Arc::new(SubAgentRegistry::from_map(resolved_with(None, child, None)));
        let out = DispatchAgentTool::new(deps)
            .execute(
                serde_json::json!({"prompt":"go","subagent_type":"triage"}),
                &exec_ctx(),
            )
            .await
            .unwrap();
        assert_eq!(
            out.content,
            "plain answer\n\n[sub-agent: 1 turns, 0 tool calls, stop: Stop]"
        );
    }

    #[test]
    fn empty_registry_omits_subagent_type_from_schema() {
        // A DispatchAgentTool built with an empty registry has a schema byte-identical
        // to 3A (no `subagent_type` property).
        let tool = DispatchAgentTool::new(exec_deps(ScriptedModel::new(vec![]), 1));
        let schema = tool.schema();
        let props = schema.parameters.get("properties").unwrap();
        assert!(
            props.get("subagent_type").is_none(),
            "empty registry must not add subagent_type"
        );
        assert!(props.get("prompt").is_some());
    }

    // --- Task B3: subagent_type schema enum + parse/resolve ------------------

    fn one_agent_registry() -> Arc<SubAgentRegistry> {
        let mut m = std::collections::HashMap::new();
        m.insert(
            "reviewer".to_string(),
            ResolvedSubAgent {
                description: "reviews code".into(),
                system_prompt: format!("You review.\n\n{}", SUBAGENT_PREAMBLE),
                tools: None,
                model: Arc::new(ScriptedModel::new(vec![])),
                protocol: Arc::new(PassthroughProtocol),
                model_limit: None,
                max_tokens: None,
                tool_call_limit: Some(3),
                response_format: None,
                permissions: None,
            },
        );
        Arc::new(SubAgentRegistry::from_map(m))
    }

    #[test]
    fn nonempty_registry_adds_subagent_type_enum_with_descriptions() {
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 1);
        deps.subagents = one_agent_registry();
        let tool = DispatchAgentTool::new(deps);
        let schema = tool.schema();
        let st = schema.parameters["properties"]["subagent_type"].clone();
        let variants: Vec<String> = serde_json::from_value(st["enum"].clone()).unwrap();
        assert!(variants.contains(&"general-purpose".to_string()));
        assert!(variants.contains(&"reviewer".to_string()));
        // description mentions the registered agent's purpose (M1 discovery)
        assert!(st["description"].as_str().unwrap().contains("reviews code"));
    }

    #[tokio::test]
    async fn unknown_subagent_type_is_invalid_args() {
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 1);
        deps.subagents = one_agent_registry();
        let tool = DispatchAgentTool::new(deps);
        let err = tool
            .execute(
                serde_json::json!({"prompt":"hi","subagent_type":"nope"}),
                &exec_ctx(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    // --- Task B4: named-type resolution applied to child construction -------

    /// `DispatchDeps` wired so that whichever model actually runs the child —
    /// `deps.model` (general-purpose path) OR a named type's own
    /// `resolved.model` (named path) — records the child's OWN system message
    /// (`Role::System`) into a shared `Arc<Mutex<Option<String>>>`. Rebuilds
    /// `registry` with each entry's model wrapped by the same capturing
    /// client, so callers can pass `one_agent_registry()` unmodified and this
    /// harness stays agnostic to which path a given test exercises. Reuses the
    /// `SchemaCapturingModel`/`RequestTextCapturingModel` idiom already used
    /// above, specialized to pull out exactly the system message content so
    /// prompt/preamble assertions don't have to parse the joined transcript.
    fn deps_with_capturing_child(
        registry: Arc<SubAgentRegistry>,
    ) -> (DispatchDeps, Arc<Mutex<Option<String>>>) {
        struct SystemCapturingModel {
            inner: Arc<dyn ModelClient>,
            captured_system: Arc<Mutex<Option<String>>>,
        }
        #[async_trait::async_trait]
        impl agent_model::ModelClient for SystemCapturingModel {
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
                {
                    let mut guard = self.captured_system.lock().unwrap();
                    if guard.is_none() {
                        if let Some(m) = req
                            .messages
                            .iter()
                            .find(|m| m.role == agent_model::Role::System)
                        {
                            *guard = Some(m.content.clone());
                        }
                    }
                }
                self.inner.stream(req).await
            }
        }
        let captured_system: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let wrap = |inner: Arc<dyn ModelClient>| -> Arc<dyn ModelClient> {
            Arc::new(SystemCapturingModel {
                inner,
                captured_system: captured_system.clone(),
            })
        };
        let model: Arc<dyn ModelClient> = wrap(Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "done".into(),
        )])));
        let mut wrapped_map = std::collections::HashMap::new();
        for name in registry.names() {
            let r = registry.get(name).unwrap();
            wrapped_map.insert(
                name.to_string(),
                ResolvedSubAgent {
                    description: r.description.clone(),
                    system_prompt: r.system_prompt.clone(),
                    tools: r.tools.clone(),
                    model: wrap(r.model.clone()),
                    protocol: r.protocol.clone(),
                    model_limit: r.model_limit,
                    max_tokens: r.max_tokens,
                    tool_call_limit: r.tool_call_limit,
                    response_format: r.response_format.clone(),
                    permissions: r.permissions.clone(),
                },
            );
        }
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 5);
        deps.model = model;
        deps.subagents = Arc::new(SubAgentRegistry::from_map(wrapped_map));
        (deps, captured_system)
    }

    #[tokio::test]
    async fn named_type_uses_spec_prompt_and_preamble_ignoring_role_tools() {
        // A capturing model records the child's system prompt; assert it is the
        // spec prompt + preamble, NOT the parent child_system_prompt or the role.
        let (deps, captured_system) = deps_with_capturing_child(one_agent_registry());
        let tool = DispatchAgentTool::new(deps);
        let _ = tool
            .execute(
                serde_json::json!({
                    "prompt":"do it","subagent_type":"reviewer",
                    "role":"IGNORED ROLE","tools":["IGNORED"]
                }),
                &exec_ctx(),
            )
            .await;
        let sys = captured_system.lock().unwrap().clone().expect("child ran");
        assert!(sys.starts_with("You review."));
        assert!(sys.contains(SUBAGENT_PREAMBLE));
        assert!(!sys.contains("IGNORED ROLE"));
    }

    // Pin m-3 / architecture item (b): with a NON-EMPTY registry, selecting
    // general-purpose still yields the 3A child (parent prompt + role appended) —
    // the headline byte-identical invariant (spec §3 invariant 1) on the `None` arm.
    #[tokio::test]
    async fn general_purpose_under_nonempty_registry_is_unchanged() {
        let (deps, captured_system) = deps_with_capturing_child(one_agent_registry());
        let parent_prompt = deps.child_system_prompt.clone();
        let tool = DispatchAgentTool::new(deps);
        let _ = tool
            .execute(
                serde_json::json!({
                    "prompt":"do it","subagent_type":"general-purpose","role":"R"
                }),
                &exec_ctx(),
            )
            .await;
        let sys = captured_system.lock().unwrap().clone().expect("child ran");
        assert!(sys.starts_with(&parent_prompt)); // parent-derived prompt, not a spec's
        assert!(sys.contains("Role: R")); // role honored on the general-purpose path
    }

    // Unknown-tool refs surface at DISPATCH time (Task B2 design note), not assembly.
    #[tokio::test]
    async fn named_type_unknown_tool_errors_at_dispatch() {
        let mut m = std::collections::HashMap::new();
        m.insert(
            "bad".to_string(),
            ResolvedSubAgent {
                description: "d".into(),
                system_prompt: format!("p\n\n{}", SUBAGENT_PREAMBLE),
                tools: Some(vec!["no_such_tool".into()]), // not in the (empty) base snapshot
                model: Arc::new(ScriptedModel::new(vec![])),
                protocol: Arc::new(PassthroughProtocol),
                model_limit: None,
                max_tokens: None,
                tool_call_limit: None,
                response_format: None,
                permissions: None,
            },
        );
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 1);
        deps.subagents = Arc::new(SubAgentRegistry::from_map(m));
        let tool = DispatchAgentTool::new(deps);
        let err = tool
            .execute(
                serde_json::json!({"prompt":"hi","subagent_type":"bad"}),
                &exec_ctx(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
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

    // --- Task B4: per-child write_todos isolation ----------------------------

    /// A child that calls `write_todos` updates ITS OWN handle/pinned block; the
    /// parent's todos handle is never touched (deepagents contract, spec §5.6).
    /// `base_tools` carries a `write_todos` instance bound to the PARENT's
    /// handle (the same channel real middleware-contributed tools use); dispatch
    /// must rebind it to a fresh per-child handle (last-wins registry) so the
    /// child's own plan never reaches the parent's handle.
    #[tokio::test]
    async fn child_write_todos_is_isolated_from_the_parent() {
        struct RequestTextCapturingModel {
            inner: ScriptedModel,
            request_texts: std::sync::Mutex<Vec<String>>,
        }
        #[async_trait::async_trait]
        impl agent_model::ModelClient for RequestTextCapturingModel {
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
        let parent_handle: crate::TodoHandle = Arc::new(Mutex::new(Vec::new()));
        let model = Arc::new(RequestTextCapturingModel {
            inner: ScriptedModel::new(vec![
                Scripted::Call(
                    "c1".into(),
                    "write_todos".into(),
                    r#"{"todos":[{"content":"child task","status":"in_progress"}]}"#.into(),
                ),
                Scripted::Text("done".into()),
            ]),
            request_texts: Default::default(),
        });
        let mut d = exec_deps(ScriptedModel::new(vec![]), 5);
        d.model = model.clone();
        // Parent-bound instance in base_tools — the channel real
        // TodoListMiddleware-contributed tools use (child_visible = true lands
        // it in the assembled parent's base snapshot, spec §5.6).
        d.base_tools = vec![Arc::new(crate::WriteTodosTool::new(parent_handle.clone()))];
        let tool = DispatchAgentTool::new(d);
        tool.execute(serde_json::json!({"prompt": "p"}), &exec_ctx())
            .await
            .unwrap();

        // The parent's handle must stay empty — the child's plan is never
        // merged back (deepagents contract, spec §5.6).
        assert!(
            parent_handle.lock().unwrap().is_empty(),
            "parent todos handle must never be touched by a child's write_todos call"
        );

        // The child's OWN plan renders in the child's own context: the pinned
        // block appears in a later request the child's own model receives.
        let texts = model.request_texts.lock().unwrap().clone();
        assert!(
            texts
                .iter()
                .any(|t| t.contains("child task") && t.contains("in_progress")),
            "the child's own plan must render as a pinned block in its own \
             request history: {texts:?}"
        );
    }

    /// Child stack is `[curation, stuck, repair]` (Task A3): a malformed child
    /// turn re-asks exactly once with the byte-identical message, then resolves
    /// (spec §5.6, global-constraints byte-identical repair message).
    #[tokio::test]
    async fn dispatched_child_repairs_a_malformed_turn_once() {
        struct RequestTextCapturingModel {
            inner: ScriptedModel,
            request_texts: std::sync::Mutex<Vec<String>>,
        }
        #[async_trait::async_trait]
        impl agent_model::ModelClient for RequestTextCapturingModel {
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
        let model = Arc::new(RequestTextCapturingModel {
            inner: ScriptedModel::new(vec![
                // Malformed JSON args on a registered tool name (no `Malformed`
                // variant in testkit — the same shape loop_.rs's malformed-turn
                // tests use).
                Scripted::Call("c1".into(), "remember".into(), r#"{"k": "#.into()),
                Scripted::Text("recovered".into()),
            ]),
            request_texts: Default::default(),
        });
        let mut d = exec_deps(ScriptedModel::new(vec![]), 5);
        d.model = model.clone();
        d.base_tools = vec![Arc::new(RememberStub)];
        let tool = DispatchAgentTool::new(d);
        let out = tool
            .execute(serde_json::json!({"prompt": "p"}), &exec_ctx())
            .await
            .unwrap();
        assert!(
            out.content.contains("recovered"),
            "the child must resolve past the malformed turn: {}",
            out.content
        );

        // Exactly one re-ask with the byte-identical repair message, then the
        // recovered text turn: 2 requests total (malformed turn + re-ask turn).
        let texts = model.request_texts.lock().unwrap().clone();
        assert_eq!(
            texts.len(),
            2,
            "malformed turn + one re-ask turn, no more: {texts:?}"
        );
        assert!(
            texts[1].contains("Your tool call could not be parsed: ")
                && texts[1].contains("Re-emit it correctly."),
            "re-ask message must be byte-identical to the loop-resident repair \
             wording: {}",
            texts[1]
        );
    }

    // --- 3B-1c C2: dispatch wiring — parse-at-dispatch, nested threading -----

    /// Executable probe: flips a flag when it actually runs. Access::Read with no
    /// paths ⇒ base RulePolicy says Allow — so ONLY a floor can stop it.
    struct ProbeTool {
        name: &'static str,
        executed: Arc<std::sync::atomic::AtomicBool>,
    }
    #[async_trait::async_trait]
    impl Tool for ProbeTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "probe"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: self.name.into(),
                description: "".into(),
                parameters: serde_json::json!({"type":"object"}),
            }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent {
                tool: self.name.into(),
                access: Access::Read,
                paths: vec![],
                command: None,
                summary: "probe".into(),
            })
        }
        async fn execute(
            &self,
            _a: serde_json::Value,
            _c: &ToolCtx,
        ) -> Result<ToolOutput, ToolError> {
            self.executed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(ToolOutput {
                content: "ok".into(),
                display: None,
            })
        }
    }

    struct CountingApproval {
        count: Arc<std::sync::atomic::AtomicUsize>,
        resp: agent_policy::ApprovalResponse,
    }
    #[async_trait::async_trait]
    impl agent_policy::ApprovalChannel for CountingApproval {
        async fn request(
            &self,
            _r: agent_policy::ApprovalRequest,
        ) -> agent_policy::ApprovalResponse {
            self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.resp
        }
    }

    /// deps + registry entry "triage" with the given floors, probe in base_tools,
    /// child scripted to call the probe then finish.
    fn floored_deps(probe: Arc<ProbeTool>, perms: Option<PermissionLists>) -> DispatchDeps {
        let child = ScriptedModel::new(vec![
            Scripted::Call("c1".into(), probe.name.to_string(), "{}".into()),
            Scripted::Text("child done".into()),
        ]);
        let mut m = resolved_with(None, child, None);
        m.get_mut("triage").unwrap().permissions = perms;
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 4);
        deps.base_tools = vec![probe];
        deps.subagents = Arc::new(SubAgentRegistry::from_map(m));
        deps
    }

    #[tokio::test]
    async fn deny_floor_blocks_child_tool_with_reason() {
        let executed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let probe = Arc::new(ProbeTool {
            name: "probe",
            executed: executed.clone(),
        });
        let deps = floored_deps(
            probe,
            Some(PermissionLists {
                deny: vec!["probe".into()],
                ask: vec![],
            }),
        );
        let tool = DispatchAgentTool::new(deps);
        let out = tool
            .execute(
                serde_json::json!({"prompt":"go","subagent_type":"triage"}),
                &exec_ctx(),
            )
            .await
            .unwrap();
        assert!(
            !executed.load(std::sync::atomic::Ordering::SeqCst),
            "floored tool must not run"
        );
        assert!(
            out.content.contains("child done"),
            "child run continues after the denial"
        );
    }

    /// Implementer note (deny reason observation — plan-review corrected): the
    /// ONLY viable mechanism in this harness to observe the denial REASON is
    /// via the child's NEXT completion request (the denied tool result becomes
    /// a message in it). `FullSink` drops content; `ScriptedModel` ignores
    /// incoming requests entirely — neither can see the reason string. Install
    /// a request-text-capturing model AS THE TRIAGE CHILD'S model.
    #[tokio::test]
    async fn deny_floor_reason_reaches_child_next_request() {
        struct RequestTextCapturingModel {
            inner: ScriptedModel,
            request_texts: std::sync::Mutex<Vec<String>>,
        }
        #[async_trait::async_trait]
        impl agent_model::ModelClient for RequestTextCapturingModel {
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
        let executed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let probe = Arc::new(ProbeTool {
            name: "probe",
            executed: executed.clone(),
        });
        let capturing = Arc::new(RequestTextCapturingModel {
            inner: ScriptedModel::new(vec![
                Scripted::Call("c1".into(), "probe".into(), "{}".into()),
                Scripted::Text("done".into()),
            ]),
            request_texts: Default::default(),
        });
        let mut m = resolved_with(None, ScriptedModel::new(vec![]), None);
        m.get_mut("triage").unwrap().permissions = Some(PermissionLists {
            deny: vec!["probe".into()],
            ask: vec![],
        });
        // The named child runs `resolved.model`, NOT `deps.model` — install the
        // capturing model there (kept alive via `capturing.clone()`) so its own
        // next request is observed after `execute()` returns.
        m.get_mut("triage").unwrap().model = capturing.clone();
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 4);
        deps.base_tools = vec![probe];
        deps.subagents = Arc::new(SubAgentRegistry::from_map(m));
        let tool = DispatchAgentTool::new(deps);
        tool.execute(
            serde_json::json!({"prompt":"go","subagent_type":"triage"}),
            &exec_ctx(),
        )
        .await
        .unwrap();
        assert!(!executed.load(std::sync::atomic::Ordering::SeqCst));

        let texts = capturing.request_texts.lock().unwrap().clone();
        assert!(
            texts
                .iter()
                .any(|t| t.contains("denied by sub-agent 'triage' permissions")),
            "the denial reason must reach the child's next completion request: {texts:?}"
        );
    }

    /// §3 invariant 1 pins: rule-less named, and general-purpose, get the SAME
    /// policy Arc — no wrapper. (Empty-block → None is pinned at assembly, C1.)
    #[tokio::test]
    async fn ruleless_and_general_purpose_share_the_policy_arc() {
        let deps = exec_deps(ScriptedModel::new(vec![]), 2);
        let base = deps.policy.clone();
        let m = resolved_with(None, ScriptedModel::new(vec![]), None); // permissions: None
        let mut deps = deps;
        deps.subagents = Arc::new(SubAgentRegistry::from_map(m));
        let tool = DispatchAgentTool::new(deps);
        let gp = tool.child_policy("general-purpose", None).unwrap();
        assert!(Arc::ptr_eq(&gp, &base));
        let named = tool
            .child_policy("triage", tool.deps.subagents.get("triage"))
            .unwrap();
        assert!(Arc::ptr_eq(&named, &base));
    }

    #[tokio::test]
    async fn mcp_shaped_prefix_floor_blocks_child_tool() {
        let executed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let probe = Arc::new(ProbeTool {
            name: "github__create_issue",
            executed: executed.clone(),
        });
        let deps = floored_deps(
            probe,
            Some(PermissionLists {
                deny: vec!["github__*".into()],
                ask: vec![],
            }),
        );
        let tool = DispatchAgentTool::new(deps);
        tool.execute(
            serde_json::json!({"prompt":"go","subagent_type":"triage"}),
            &exec_ctx(),
        )
        .await
        .unwrap();
        assert!(!executed.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn ask_floor_routes_through_approval_channel() {
        for (resp, should_run) in [
            (agent_policy::ApprovalResponse::Approve, true),
            (agent_policy::ApprovalResponse::Deny, false),
        ] {
            let executed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let probe = Arc::new(ProbeTool {
                name: "probe",
                executed: executed.clone(),
            });
            let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let mut deps = floored_deps(
                probe,
                Some(PermissionLists {
                    deny: vec![],
                    ask: vec!["probe".into()],
                }),
            );
            deps.approval = Arc::new(CountingApproval {
                count: count.clone(),
                resp,
            });
            let tool = DispatchAgentTool::new(deps);
            tool.execute(
                serde_json::json!({"prompt":"go","subagent_type":"triage"}),
                &exec_ctx(),
            )
            .await
            .unwrap();
            assert_eq!(
                count.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "exactly one prompt"
            );
            assert_eq!(
                executed.load(std::sync::atomic::Ordering::SeqCst),
                should_run
            );
        }
    }

    /// §2.6 lenient-boot fail-closed: a dialect-invalid block (validate() never
    /// ran) makes the named child undispatchable — NEVER unfloored.
    #[tokio::test]
    async fn invalid_permissions_fail_dispatch_closed() {
        let executed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let probe = Arc::new(ProbeTool {
            name: "probe",
            executed: executed.clone(),
        });
        let deps = floored_deps(
            probe,
            Some(PermissionLists {
                deny: vec!["a*b".into()],
                ask: vec![],
            }),
        );
        let tool = DispatchAgentTool::new(deps);
        let err = tool
            .execute(
                serde_json::json!({"prompt":"go","subagent_type":"triage"}),
                &exec_ctx(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid permissions"), "{err}");
        assert!(!executed.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// §3 invariant 2(b) — the delegation-escape guard: a floored child's
    /// general-purpose GRANDCHILD is still floored (nested.policy threading).
    #[tokio::test]
    async fn transitivity_floor_reaches_grandchild() {
        let executed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let probe = Arc::new(ProbeTool {
            name: "probe",
            executed: executed.clone(),
        });
        // Grandchild (general-purpose, from nested deps' default child model):
        // tries the floored probe, then finishes.
        let grandchild = ScriptedModel::new(vec![
            Scripted::Call("g1".into(), "probe".into(), "{}".into()),
            Scripted::Text("grandchild done".into()),
        ]);
        // Named child: dispatches the grandchild, then finishes.
        let child = ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "dispatch_agent".into(),
                r#"{"prompt":"delegate"}"#.into(),
            ),
            Scripted::Text("child done".into()),
        ]);
        let mut m = resolved_with(None, child, None);
        m.get_mut("triage").unwrap().permissions = Some(PermissionLists {
            deny: vec!["probe".into()],
            ask: vec![],
        });
        let mut deps = exec_deps(grandchild, 4);
        deps.base_tools = vec![probe];
        deps.max_depth = 2; // allow one nested dispatch level
        deps.subagents = Arc::new(SubAgentRegistry::from_map(m));
        let tool = DispatchAgentTool::new(deps);
        let out = tool
            .execute(
                serde_json::json!({"prompt":"go","subagent_type":"triage"}),
                &exec_ctx(),
            )
            .await
            .unwrap();
        assert!(
            !executed.load(std::sync::atomic::Ordering::SeqCst),
            "grandchild must inherit the child's floor — delegation must not escape it"
        );
        assert!(out.content.contains("child done"));
    }
}
