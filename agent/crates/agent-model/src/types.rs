use agent_tools::{ToolCall, ToolSchema};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
    /// Preserved chain-of-thought for this turn, kept as data rather than baked
    /// into `content`. Each model backend decides how (or whether) to render it
    /// back into the prompt — see `render_transcript` (claude_cli) and
    /// `messages_to_json` (openai). `None` unless `preserve_thinking` is on.
    pub reasoning: Option<String>,
}

impl Message {
    pub fn system(c: impl Into<String>) -> Self {
        Self::plain(Role::System, c)
    }
    pub fn user(c: impl Into<String>) -> Self {
        Self::plain(Role::User, c)
    }
    pub fn assistant(c: impl Into<String>, calls: Option<Vec<ToolCall>>) -> Self {
        Self {
            role: Role::Assistant,
            content: c.into(),
            tool_calls: calls,
            tool_call_id: None,
            name: None,
            reasoning: None,
        }
    }
    pub fn tool(call_id: impl Into<String>, name: impl Into<String>, c: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: c.into(),
            tool_calls: None,
            tool_call_id: Some(call_id.into()),
            name: Some(name.into()),
            reasoning: None,
        }
    }
    fn plain(role: Role, c: impl Into<String>) -> Self {
        Self {
            role,
            content: c.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning: None,
        }
    }
    /// Attach preserved reasoning, returning the message for chaining.
    pub fn with_reasoning(mut self, reasoning: impl Into<String>) -> Self {
        self.reasoning = Some(reasoning.into());
        self
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
    /// Ask the backend to retain prior-turn reasoning in the rendered prompt
    /// (Qwen3.6 `chat_template_kwargs.preserve_thinking`). Paired with assistant
    /// `Message.reasoning`, which the OpenAI adapter sends as `reasoning_content`.
    pub preserve_thinking: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StopReason {
    #[default]
    Stop,
    ToolCalls,
    Length,
    BudgetExhausted,
    Cancelled,
}

#[derive(Debug, Clone, Default)]
pub struct RawToolCall {
    /// Streaming correlation index (OpenAI/llama.cpp send it on every fragment of
    /// a call). Used to reassemble parallel calls even if fragments interleave.
    pub index: Option<usize>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub args_fragment: String,
}

#[derive(Debug, Clone)]
pub enum Chunk {
    Text(String),
    Reasoning(String),
    ToolCallDelta(RawToolCall),
    Done(StopReason),
    Usage {
        prompt_tokens: u32,
        completion_tokens: u32,
        reasoning_tokens: Option<u32>,
        cached_tokens: Option<u32>,
        cost_usd: Option<f64>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct AssistantTurn {
    pub text: String,
    pub raw_tool_calls: Vec<RawToolCall>,
    pub stop: StopReason,
    pub reasoning: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub reasoning_tokens: Option<u32>,
    pub cached_tokens: Option<u32>,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ParsedTurn {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("protocol error: {0}")]
pub struct ProtocolError(pub String);

#[derive(Debug, Clone, thiserror::Error)]
pub enum ModelError {
    #[error("http error: {0}")]
    Http(String),
    #[error("status {code}: {body}")]
    Status { code: u16, body: String },
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
    fn completion_request_defaults_preserve_thinking_off() {
        let req = CompletionRequest::default();
        assert!(!req.preserve_thinking);
    }

    #[test]
    fn assistant_carries_no_reasoning_by_default_and_builder_attaches_it() {
        let plain = Message::assistant("answer", None);
        assert_eq!(plain.reasoning, None);
        let with = Message::assistant("answer", None).with_reasoning("secret plan");
        assert_eq!(with.reasoning.as_deref(), Some("secret plan"));
        assert_eq!(with.content, "answer"); // reasoning is separate data, not baked into content
    }

    #[test]
    fn timeout_error_displays_duration() {
        let e = ModelError::Timeout(std::time::Duration::from_secs(120));
        assert_eq!(e.to_string(), "stream idle timeout after 120s");
    }
}
