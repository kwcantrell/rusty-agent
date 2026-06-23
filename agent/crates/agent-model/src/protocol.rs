use crate::{AssistantTurn, CompletionRequest, ParsedTurn, ProtocolError};
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
        for (i, rc) in raw.raw_tool_calls.iter().enumerate() {
            let name = rc.name.clone()
                .ok_or_else(|| ProtocolError(format!("tool call {i} missing name")))?;
            let args: serde_json::Value = if rc.args_fragment.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&rc.args_fragment)
                    .map_err(|e| ProtocolError(format!("tool call {i} bad args: {e}")))?
            };
            let id = rc.id.clone().unwrap_or_else(|| format!("call_{i}"));
            tool_calls.push(ToolCall { id, name, args });
        }
        Ok(ParsedTurn { text: raw.text.clone(), tool_calls })
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
                id: Some("c1".into()), name: Some("read_file".into()),
                args_fragment: r#"{"path":"a.txt"}"#.into() }],
            stop: StopReason::ToolCalls,
        };
        let parsed = NativeProtocol.parse(&turn).unwrap();
        assert_eq!(parsed.text, "ok");
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "read_file");
        assert_eq!(parsed.tool_calls[0].args["path"], "a.txt");
    }

    #[test]
    fn native_rejects_malformed_args() {
        let turn = AssistantTurn { text: "".into(),
            raw_tool_calls: vec![RawToolCall { id: Some("c1".into()),
                name: Some("x".into()), args_fragment: "{not json".into() }],
            stop: StopReason::ToolCalls };
        assert!(NativeProtocol.parse(&turn).is_err());
    }

    #[test]
    fn native_prepare_keeps_tools_field() {
        let mut req = CompletionRequest { messages: vec![], tools: vec![
            agent_tools::ToolSchema { name: "t".into(), description: "d".into(),
                parameters: serde_json::json!({}) }],
            temperature: 0.0, max_tokens: None };
        NativeProtocol.prepare(&mut req);
        assert_eq!(req.tools.len(), 1); // native leaves tools for the client to send
    }
}
