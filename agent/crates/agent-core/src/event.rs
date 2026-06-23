use agent_model::StopReason;
use agent_policy::ApprovalRequest;
use agent_tools::ToolOutput;

pub enum AgentEvent {
    Token(String),
    ToolStart { name: String, args: serde_json::Value },
    ToolResult { name: String, output: ToolOutput },
    Approval(ApprovalRequest),
    Error(String),
    Done(StopReason),
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: AgentEvent);
}
