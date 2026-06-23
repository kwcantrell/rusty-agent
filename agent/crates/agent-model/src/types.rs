use agent_tools::{ToolCall, ToolSchema};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role { System, User, Assistant, Tool }

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
}

impl Message {
    pub fn system(c: impl Into<String>) -> Self { Self::plain(Role::System, c) }
    pub fn user(c: impl Into<String>) -> Self { Self::plain(Role::User, c) }
    pub fn assistant(c: impl Into<String>, calls: Option<Vec<ToolCall>>) -> Self {
        Self { role: Role::Assistant, content: c.into(), tool_calls: calls,
               tool_call_id: None, name: None }
    }
    pub fn tool(call_id: impl Into<String>, name: impl Into<String>, c: impl Into<String>) -> Self {
        Self { role: Role::Tool, content: c.into(), tool_calls: None,
               tool_call_id: Some(call_id.into()), name: Some(name.into()) }
    }
    fn plain(role: Role, c: impl Into<String>) -> Self {
        Self { role, content: c.into(), tool_calls: None, tool_call_id: None, name: None }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompletionRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSchema>,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub min_p: Option<f32>,
    pub presence_penalty: Option<f32>,
    pub repeat_penalty: Option<f32>,
    pub enable_thinking: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StopReason { #[default] Stop, ToolCalls, Length, BudgetExhausted }

#[derive(Debug, Clone, Default)]
pub struct RawToolCall {
    pub id: Option<String>,
    pub name: Option<String>,
    pub args_fragment: String,
}

#[derive(Debug, Clone)]
pub enum Chunk { Text(String), Reasoning(String), ToolCallDelta(RawToolCall), Done(StopReason) }

#[derive(Debug, Clone, Default)]
pub struct AssistantTurn {
    pub text: String,
    pub raw_tool_calls: Vec<RawToolCall>,
    pub stop: StopReason,
    pub reasoning: String,
}

#[derive(Debug, Clone)]
pub struct ParsedTurn { pub text: String, pub tool_calls: Vec<ToolCall> }

#[derive(Debug, Clone, thiserror::Error)]
#[error("protocol error: {0}")]
pub struct ProtocolError(pub String);

#[derive(Debug, Clone, thiserror::Error)]
pub enum ModelError {
    #[error("http error: {0}")]
    Http(String),
    #[error("status {0}")]
    Status(u16),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("stream error: {0}")]
    Stream(String),
    #[error("process error: {0}")]
    Process(String),
    #[error("stream idle timeout after {0:?}")]
    Timeout(std::time::Duration),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_constructors_set_role() {
        assert!(matches!(Message::system("s").role, Role::System));
        assert!(matches!(Message::user("u").role, Role::User));
        let t = Message::tool("call-1", "read_file", "contents");
        assert!(matches!(t.role, Role::Tool));
        assert_eq!(t.tool_call_id.as_deref(), Some("call-1"));
        assert_eq!(t.name.as_deref(), Some("read_file"));
    }

    #[test]
    fn timeout_error_displays_duration() {
        let e = ModelError::Timeout(std::time::Duration::from_secs(120));
        assert_eq!(e.to_string(), "stream idle timeout after 120s");
    }
}
