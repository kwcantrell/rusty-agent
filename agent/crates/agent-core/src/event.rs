use agent_model::StopReason;
use agent_policy::ApprovalRequest;
use agent_tools::ToolOutput;

/// Telemetry for context-window curation (offload / compaction).
#[derive(Debug, Clone)]
pub enum ContextEvent {
    Offloaded { id: u64, bytes: usize, tool: String },
    Compacted { turns_replaced: usize, tokens_before: usize, tokens_after: usize },
    CompactionFailed { reason: String },
}

pub enum AgentEvent {
    Token(String),
    Reasoning(String),
    Usage { prompt_tokens: usize, context_limit: usize, turn: usize, max_turns: usize },
    /// Server-reported token usage for a completed turn (the faithful metric;
    /// `Usage.prompt_tokens` above is the pre-request local estimate).
    ServerUsage { prompt_tokens: u32, completion_tokens: u32, turn: usize },
    ToolStart { name: String, args: serde_json::Value },
    ToolResult { name: String, output: ToolOutput },
    Approval(ApprovalRequest),
    Error(String),
    Done(StopReason),
    Context(ContextEvent),
    /// Emitted once at run start when the configured sandbox has silently
    /// degraded to unsandboxed host execution (e.g. Docker unavailable in
    /// `auto` mode). The run is NOT isolated despite being configured to be;
    /// surfaces that "we thought we were sandboxed" hole loudly to every
    /// observer instead of leaving it in a single `tracing::warn!` line.
    SandboxDegraded { mechanism: &'static str, reason: String },
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: AgentEvent);
}
