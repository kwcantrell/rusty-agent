use agent_tools::{ToolCall, ToolSchema};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Preserved chain-of-thought for this turn, kept as data rather than baked
    /// into `content`. Each model backend decides how (or whether) to render it
    /// back into the prompt — see `render_transcript` (claude_cli) and
    /// `messages_to_json` (openai). `None` unless `preserve_thinking` is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    /// The run aborted on an unrecoverable model error (fatal or retries
    /// exhausted). Emitted alongside the returned `AgentError`.
    Error,
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
    /// Tool calls the protocol could not parse (native per-call isolation); the
    /// loop answers each with a per-call error result instead of discarding the
    /// whole turn. Always empty from the prompted protocol.
    pub invalid: Vec<InvalidToolCall>,
}

/// A tool call the protocol could not parse; the loop answers it with a
/// per-call error result instead of discarding the turn (spec 2026-07-02).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InvalidToolCall {
    pub id: String,
    pub name: String,
    pub error: String,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("protocol error: {0}")]
pub struct ProtocolError(pub String);

#[derive(Debug, Clone, thiserror::Error)]
pub enum ModelError {
    #[error("http error: {0}")]
    Http(String),
    #[error("status {code}: {body}")]
    Status {
        code: u16,
        body: String,
        /// Seconds parsed from a `Retry-After` response header (integer form
        /// only; HTTP-date form is ignored). `None` when absent. Advisory: the
        /// retry loop honors it, capped, alongside jittered backoff.
        retry_after: Option<u64>,
    },
    #[error("decode error: {0}")]
    Decode(String),
    #[error("stream error: {0}")]
    Stream(String),
    #[error("process error: {0}")]
    Process(String),
    #[error("stream idle timeout after {0:?}")]
    Timeout(std::time::Duration),
    /// The caller's cancel token fired mid-call. Never retried; the loop's
    /// token check is authoritative, this variant exists so cancellation is
    /// not spoofable as a plain stream-error string.
    #[error("cancelled")]
    Cancelled,
}

/// How the agent loop should react to a model error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Transient: transport, stream, timeout, 5xx, 408/429. Retry with backoff.
    Retryable,
    /// Permanent request problem: other 4xx, decode. Abort on first sight.
    Fatal,
    /// The request exceeds the model's context. Retrying verbatim cannot
    /// succeed; the caller should shrink the context and rebuild once.
    ContextOverflow,
}

/// Case-insensitive overflow signature check. Conservative by design: a miss
/// degrades to the code's plain class, never to a wrong retry storm.
fn body_is_overflow(body: &str) -> bool {
    let b = body.to_ascii_lowercase();
    [
        "context length",
        "context window",
        "context size",
        "too many tokens",
        "prompt is too long",
    ]
    .iter()
    .any(|sig| b.contains(sig))
}

impl ModelError {
    /// Classify for the retry loop. Overflow is checked before the 4xx-fatal
    /// rule (overflow usually arrives as a 400); `Status{400|413|422}`,
    /// `Stream`, and `Process` bodies are all signature-checked — the
    /// claude-cli backend surfaces overflow as `Process` stderr text.
    pub fn class(&self) -> ErrorClass {
        match self {
            ModelError::Status {
                code: 400 | 413 | 422,
                body,
                ..
            } if body_is_overflow(body) => ErrorClass::ContextOverflow,
            ModelError::Stream(body) if body_is_overflow(body) => ErrorClass::ContextOverflow,
            ModelError::Process(body) if body_is_overflow(body) => ErrorClass::ContextOverflow,
            ModelError::Status {
                code: 408 | 429 | 500..=599,
                ..
            } => ErrorClass::Retryable,
            ModelError::Status { .. } | ModelError::Decode(_) | ModelError::Cancelled => {
                ErrorClass::Fatal
            }
            ModelError::Http(_)
            | ModelError::Stream(_)
            | ModelError::Process(_)
            | ModelError::Timeout(_) => ErrorClass::Retryable,
        }
    }
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

    #[test]
    fn class_table() {
        use ErrorClass::*;
        let cases: Vec<(ModelError, ErrorClass)> = vec![
            (ModelError::Http("connect refused".into()), Retryable),
            (ModelError::Stream("byte stream cut".into()), Retryable),
            (ModelError::Process("claude exited (1)".into()), Retryable),
            (
                ModelError::Timeout(std::time::Duration::from_secs(120)),
                Retryable,
            ),
            (
                ModelError::Status {
                    code: 500,
                    body: "oops".into(),
                    retry_after: None,
                },
                Retryable,
            ),
            (
                ModelError::Status {
                    code: 503,
                    body: "busy".into(),
                    retry_after: None,
                },
                Retryable,
            ),
            (
                ModelError::Status {
                    code: 408,
                    body: "timeout".into(),
                    retry_after: None,
                },
                Retryable,
            ),
            (
                ModelError::Status {
                    code: 429,
                    body: "rate limited".into(),
                    retry_after: None,
                },
                Retryable,
            ),
            (
                ModelError::Status {
                    code: 400,
                    body: "invalid request".into(),
                    retry_after: None,
                },
                Fatal,
            ),
            (
                ModelError::Status {
                    code: 401,
                    body: "bad key".into(),
                    retry_after: None,
                },
                Fatal,
            ),
            (
                ModelError::Status {
                    code: 403,
                    body: "forbidden".into(),
                    retry_after: None,
                },
                Fatal,
            ),
            (
                ModelError::Status {
                    code: 404,
                    body: "no such model".into(),
                    retry_after: None,
                },
                Fatal,
            ),
            (ModelError::Decode("not json".into()), Fatal),
            (ModelError::Cancelled, Fatal), // defensive: must never reach a retry arm
        ];
        for (e, want) in cases {
            assert_eq!(e.class(), want, "wrong class for {e}");
        }
    }

    #[test]
    fn overflow_is_detected_on_status_and_stream_bodies() {
        use ErrorClass::*;
        let overflowing = [
            "This model's maximum CONTEXT LENGTH is 8192 tokens",
            "the request exceeds the available context size",
            "context window exceeded",
            "too many tokens in prompt",
            "your prompt is too long",
        ];
        for body in overflowing {
            for code in [400u16, 413, 422] {
                let e = ModelError::Status {
                    code,
                    body: body.into(),
                    retry_after: None,
                };
                assert_eq!(e.class(), ContextOverflow, "code {code}, body {body}");
            }
            let e = ModelError::Stream(format!("server error in stream: {body}"));
            assert_eq!(e.class(), ContextOverflow, "stream body {body}");
        }
    }

    #[test]
    fn overflow_signatures_are_conservative() {
        use ErrorClass::*;
        // Near-misses must NOT match: degrade to the plain class.
        let e = ModelError::Status {
            code: 400,
            body: "context deadline exceeded".into(),
            retry_after: None,
        };
        assert_eq!(e.class(), Fatal);
        let e = ModelError::Stream("context deadline exceeded".into());
        assert_eq!(e.class(), Retryable);
        // Overflow bodies on non-overflow codes keep their code's class.
        let e = ModelError::Status {
            code: 500,
            body: "context length exceeded".into(),
            retry_after: None,
        };
        assert_eq!(e.class(), Retryable);
        let e = ModelError::Status {
            code: 404,
            body: "context length exceeded".into(),
            retry_after: None,
        };
        assert_eq!(e.class(), Fatal);
    }

    #[test]
    fn overflow_is_detected_on_process_bodies() {
        // claude-cli surfaces model errors as Process("claude exited (1): <stderr>").
        for body in [
            "claude exited (1): This model's maximum CONTEXT LENGTH is 8192 tokens",
            "claude exited (1): your prompt is too long",
        ] {
            assert_eq!(
                ModelError::Process(body.into()).class(),
                ErrorClass::ContextOverflow,
                "expected overflow for {body:?}"
            );
        }
        // Conservative: near-miss stays Retryable.
        assert_eq!(
            ModelError::Process("claude exited (1): context deadline exceeded".into()).class(),
            ErrorClass::Retryable
        );
        // Spawn-style bodies without signatures stay Retryable.
        assert_eq!(
            ModelError::Process("spawn claude: No such file or directory".into()).class(),
            ErrorClass::Retryable
        );
    }

    #[test]
    fn message_serde_round_trips_all_fields() {
        let mut m = Message::assistant(
            "text".to_string(),
            Some(vec![agent_tools::ToolCall {
                id: "c1".into(),
                name: "t".into(),
                args: serde_json::json!({"k": 1}),
            }]),
        );
        m.reasoning = Some("thought".into());
        let json = serde_json::to_string(&m).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back.role, m.role);
        assert_eq!(back.content, m.content);
        assert_eq!(back.reasoning, m.reasoning);
        assert_eq!(back.tool_calls.as_ref().unwrap()[0].id, "c1");
        // Lenient decode: absent optionals default (forward compat).
        let sparse: Message =
            serde_json::from_str(r#"{"role":"User","content":"hi"}"#).unwrap();
        assert_eq!(sparse.content, "hi");
        assert!(sparse.tool_calls.is_none());
    }
}
