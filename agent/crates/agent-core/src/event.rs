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
    ToolStart { name: String, args: serde_json::Value },
    ToolResult { name: String, output: ToolOutput },
    Approval(ApprovalRequest),
    Error(String),
    Done(StopReason),
    Context(ContextEvent),
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: AgentEvent);
}
