use agent_model::StopReason;
use agent_policy::ApprovalRequest;
use agent_tools::ToolOutput;

/// Telemetry for context-window curation (offload / compaction / eviction).
#[derive(Debug, Clone)]
pub enum ContextEvent {
    Offloaded {
        path: String,
        bytes: usize,
        tool: String,
    },
    Compacted {
        turns_replaced: usize,
        tokens_before: usize,
        tokens_after: usize,
    },
    CompactionFailed {
        reason: String,
    },
    /// Plain window eviction omitted history messages from the built request
    /// (distinct from offload/compaction, which transform rather than drop).
    /// `est_tokens` uses the same estimate the window evicts against.
    Evicted {
        messages: usize,
        est_tokens: usize,
    },
    /// The model reported context overflow; the loop forced compaction and
    /// rebuilt the request. Emitted BEFORE maintenance runs, so it fires even
    /// when compaction no-ops (`Compacted`/`CompactionFailed` then narrate the
    /// maintenance outcome).
    OverflowRecovery,
}

/// Terminal status of one tool call — carried on every ToolResult so
/// observers/evals can compute error and denial rates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Ok,
    Denied,
    Error,
    Timeout,
    Panic,
}

impl ToolStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Denied => "denied",
            Self::Error => "error",
            Self::Timeout => "timeout",
            Self::Panic => "panic",
        }
    }
}

/// Typed per-delegation sub-agent stream (spec 2026-07-09 3B-2). Every case
/// carries the delegation id = the dispatching call's on-wire id — the exact
/// string forwarded child rows carry as `parent_id` — so frontends join the
/// typed stream to the existing `dispatch_agent` tool row, at any depth.
#[derive(Debug, Clone)]
pub enum SubagentEvent {
    /// One dispatch began. Emitted after all dispatch validation (a rejected
    /// dispatch never emits Start) and after the loop's ToolStart for the
    /// dispatch_agent call, so frontends always see the host row first.
    Start {
        id: String,
        /// Registry name, or "general-purpose" for the ad-hoc path.
        subagent_type: String,
        /// Per-call role arg (general-purpose only; None for named types).
        role: Option<String>,
    },
    /// Child assistant text delta.
    Text { id: String, text: String },
    /// Child reasoning delta. NOTE: a genuinely NEW egress path — child
    /// reasoning was trace-file-only before 3B-2. Streaming it to the local
    /// UI was gate-approved (spec G5): same trust boundary as the parent's
    /// own reasoning stream.
    Reasoning { id: String, text: String },
    /// Child mid-stream retry retracted in-flight deltas: frontends trim the
    /// tail of THIS delegation's transcript (mirrors top-level StreamRetry).
    StreamRetry {
        id: String,
        discarded_text_chars: usize,
        discarded_reasoning_chars: usize,
    },
    /// The delegation finished, on any path (drop-guard guaranteed: exactly
    /// one End per Start).
    End {
        id: String,
        outcome: SubagentOutcome,
        /// The child's own stop reason from the capture; None when the child
        /// never emitted Done (e.g. timeout before its first turn completed).
        stop: Option<StopReason>,
        /// Human-readable failure/timeout detail; None on Completed.
        detail: Option<String>,
        turns: usize,
        tool_calls: u64,
        duration_ms: u64,
    },
}

/// How a delegation terminated. Deliberately NOT derivable from `stop` +
/// tool-result status (Timeout and Failed both surface non-ok; Cancelled
/// returns Err) — dispatch's execute() is the only place that knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentOutcome {
    Completed,
    Timeout,
    Failed,
    Cancelled,
}

impl SubagentOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

pub enum AgentEvent {
    Token(String),
    Reasoning(String),
    Usage {
        prompt_tokens: usize,
        context_limit: usize,
        turn: usize,
        max_turns: usize,
    },
    /// Server-reported token usage for a completed turn (the faithful metric;
    /// `Usage.prompt_tokens` above is the pre-request local estimate).
    ServerUsage {
        prompt_tokens: u32,
        completion_tokens: u32,
        reasoning_tokens: Option<u32>,
        cached_tokens: Option<u32>,
        cost_usd: Option<f64>,
        turn_duration_ms: u64,
        turn: usize,
        /// Set when this event belongs to a sub-agent: the dispatching
        /// `dispatch_agent` call's id (spec 2026-07-02 E1/E2).
        parent_id: Option<String>,
    },
    ToolStart {
        id: String,
        name: String,
        args: serde_json::Value,
        parent_id: Option<String>,
    },
    /// duration_ms is 0 for gate-rejected calls that never executed.
    ToolResult {
        id: String,
        name: String,
        status: ToolStatus,
        output: ToolOutput,
        duration_ms: u64,
        parent_id: Option<String>,
    },
    Approval(ApprovalRequest),
    Error(String),
    Done(StopReason),
    Context(ContextEvent),
    /// Emitted once at run start when the configured sandbox is degraded
    /// (e.g. Docker unavailable in `auto` mode). Exec-capable tools are
    /// refused while degraded; `auto` recovers automatically once the
    /// mechanism becomes available again.
    SandboxDegraded {
        mechanism: &'static str,
        reason: String,
    },
    /// A model stream died mid-answer and another attempt follows: the
    /// in-flight partial text/reasoning of this turn is abandoned — frontends
    /// should discard those trailing chars before the retry re-streams. Emitted
    /// only when the failed attempt had already streamed something and a fresh
    /// attempt follows (retryable-with-budget or the once-per-turn overflow
    /// rebuild); never on a fatal/cancelled/second-overflow terminal.
    StreamRetry {
        discarded_text_chars: usize,
        discarded_reasoning_chars: usize,
    },
    /// Emitted once at run start with the run's inputs, so a failed top-level
    /// turn is replayable from the trace alone and traces can be harvested into
    /// eval datasets (audit 6.1). `system` is the composed system prompt as the
    /// context manager holds it at run start (None for managers without one).
    /// Wire: never forwarded to frontends (server_event_from maps it to None).
    RunStart {
        input: String,
        system: Option<String>,
    },
    /// Typed per-delegation sub-agent stream (3B-2). Emitted ONLY by
    /// dispatch.rs (execute() lifecycle + SubagentSink delta forwarding).
    Subagent(SubagentEvent),
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: AgentEvent);
}
