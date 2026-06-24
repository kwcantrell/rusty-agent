use agent_model::StopReason;
use agent_policy::ApprovalRequest;
use agent_tools::ToolOutput;

pub enum AgentEvent {
    Token(String),
    Reasoning(String),
    Usage { prompt_tokens: usize, context_limit: usize, turn: usize, max_turns: usize },
    ToolStart { name: String, args: serde_json::Value },
    ToolResult { name: String, output: ToolOutput },
    Approval(ApprovalRequest),
    Error(String),
    Done(StopReason),
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: AgentEvent);
}
