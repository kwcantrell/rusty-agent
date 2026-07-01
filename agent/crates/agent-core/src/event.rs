use agent_model::StopReason;
use agent_policy::ApprovalRequest;
use agent_tools::ToolOutput;

/// Telemetry for context-window curation (offload / compaction).
#[derive(Debug, Clone)]
pub enum ContextEvent {
    Offloaded {
        id: u64,
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
    },
    ToolStart {
        id: String,
        name: String,
        args: serde_json::Value,
    },
    /// duration_ms is 0 for gate-rejected calls that never executed.
    ToolResult {
        id: String,
        name: String,
        status: ToolStatus,
        output: ToolOutput,
        duration_ms: u64,
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
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: AgentEvent);
}
