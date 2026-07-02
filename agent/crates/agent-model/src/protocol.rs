use crate::{AssistantTurn, CompletionRequest, InvalidToolCall, ParsedTurn, ProtocolError};
use agent_tools::ToolCall;

pub trait ToolCallProtocol: Send + Sync {
    /// Adjust the outbound request (e.g. inject tool schemas into the prompt).
    fn prepare(&self, req: &mut CompletionRequest);
    /// Convert a finished assistant turn into clean text + structured tool calls.
    fn parse(&self, raw: &AssistantTurn) -> Result<ParsedTurn, ProtocolError>;
}

/// Uses the server's native OpenAI-style `tool_calls`.
pub struct NativeProtocol;

impl ToolCallProtocol for NativeProtocol {
    fn prepare(&self, _req: &mut CompletionRequest) {
        // No-op: the client serializes `req.tools` into the `tools` field directly.
    }
    fn parse(&self, raw: &AssistantTurn) -> Result<ParsedTurn, ProtocolError> {
        let mut tool_calls = Vec::new();
        let mut invalid = Vec::new();
        for (i, rc) in raw.raw_tool_calls.iter().enumerate() {
            let id = rc.id.clone().unwrap_or_else(|| format!("call_{i}"));
            let Some(name) = rc.name.clone() else {
                invalid.push(InvalidToolCall {
                    id,
                    name: "unknown".into(),
                    error: format!("tool call {i} missing name"),
                });
                continue;
            };
            let args: serde_json::Value = if rc.args_fragment.trim().is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_str(&rc.args_fragment) {
                    Ok(v) => v,
                    Err(e) => {
                        invalid.push(InvalidToolCall {
                            id,
                            name,
                            error: format!("tool call {i} bad args: {e}"),
                        });
                        continue;
                    }
                }
            };
            tool_calls.push(ToolCall { id, name, args });
        }
        Ok(ParsedTurn {
            text: raw.text.clone(),
            tool_calls,
            invalid,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    #[test]
    fn native_parses_raw_tool_calls_into_structured() {
        let turn = AssistantTurn {
            text: "ok".into(),
            raw_tool_calls: vec![RawToolCall {
                index: None,
                id: Some("c1".into()),
                name: Some("read_file".into()),
                args_fragment: r#"{"path":"a.txt"}"#.into(),
            }],
            stop: StopReason::ToolCalls,
            reasoning: String::new(),
            ..Default::default()
        };
        let parsed = NativeProtocol.parse(&turn).unwrap();
        assert_eq!(parsed.text, "ok");
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "read_file");
        assert_eq!(parsed.tool_calls[0].args["path"], "a.txt");
    }

    #[test]
    fn native_isolates_malformed_args_per_call() {
        let turn = AssistantTurn {
            text: "".into(),
            raw_tool_calls: vec![
                RawToolCall {
                    index: None,
                    id: Some("c1".into()),
                    name: Some("good".into()),
                    args_fragment: r#"{"a":1}"#.into(),
                },
                RawToolCall {
                    index: None,
                    id: Some("c2".into()),
                    name: Some("bad".into()),
                    args_fragment: "{not json".into(),
                },
            ],
            stop: StopReason::ToolCalls,
            reasoning: String::new(),
            ..Default::default()
        };
        let parsed = NativeProtocol.parse(&turn).unwrap();
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "good");
        assert_eq!(parsed.invalid.len(), 1);
        assert_eq!(parsed.invalid[0].id, "c2");
        assert_eq!(parsed.invalid[0].name, "bad");
        assert!(parsed.invalid[0].error.contains("bad args"));
    }

    #[test]
    fn native_isolates_missing_name_per_call() {
        let turn = AssistantTurn {
            text: "".into(),
            raw_tool_calls: vec![RawToolCall {
                index: None,
                id: None,
                name: None,
                args_fragment: "{}".into(),
            }],
            stop: StopReason::ToolCalls,
            reasoning: String::new(),
            ..Default::default()
        };
        let parsed = NativeProtocol.parse(&turn).unwrap();
        assert!(parsed.tool_calls.is_empty());
        assert_eq!(parsed.invalid.len(), 1);
        assert_eq!(parsed.invalid[0].id, "call_0");
        assert_eq!(parsed.invalid[0].name, "unknown");
        assert!(parsed.invalid[0].error.contains("missing name"));
    }

    #[test]
    fn native_prepare_keeps_tools_field() {
        let mut req = CompletionRequest {
            messages: vec![],
            tools: vec![agent_tools::ToolSchema {
                name: "t".into(),
                description: "d".into(),
                parameters: serde_json::json!({}),
            }],
            ..Default::default()
        };
        NativeProtocol.prepare(&mut req);
        assert_eq!(req.tools.len(), 1); // native leaves tools for the client to send
    }
}
