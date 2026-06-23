use crate::{AssistantTurn, CompletionRequest, Message, ParsedTurn, ProtocolError, Role, ToolCallProtocol};
use agent_tools::ToolCall;

const FENCE: &str = "```tool_call";

pub struct PromptedJsonProtocol;

impl PromptedJsonProtocol {
    fn system_preamble(req: &CompletionRequest) -> String {
        let mut s = String::from(
            "You can call tools. To call one, emit a fenced block exactly like:\n\
             ```tool_call\n{\"name\":\"<tool>\",\"arguments\":{...}}\n```\n\
             Emit at most one tool_call block per reply. Available tools:\n");
        for t in &req.tools {
            s.push_str(&format!("- {}: {} | schema: {}\n", t.name, t.description, t.parameters));
        }
        s
    }
}

impl ToolCallProtocol for PromptedJsonProtocol {
    fn prepare(&self, req: &mut CompletionRequest) {
        let preamble = Self::system_preamble(req);
        // Merge into an existing leading system message, or insert one.
        if let Some(first) = req.messages.first_mut() {
            if matches!(first.role, Role::System) {
                first.content = format!("{preamble}\n{}", first.content);
                req.tools.clear();
                return;
            }
        }
        req.messages.insert(0, Message::system(preamble));
        req.tools.clear();
    }

    fn parse(&self, raw: &AssistantTurn) -> Result<ParsedTurn, ProtocolError> {
        let text = &raw.text;
        let Some(start) = text.find(FENCE) else {
            return Ok(ParsedTurn { text: text.clone(), tool_calls: vec![] });
        };
        let after = &text[start + FENCE.len()..];
        let Some(end_rel) = after.find("```") else {
            return Err(ProtocolError("unterminated tool_call block".into()));
        };
        let body = after[..end_rel].trim();
        let v: serde_json::Value = serde_json::from_str(body)
            .map_err(|e| ProtocolError(format!("bad tool_call JSON: {e}")))?;
        let name = v.get("name").and_then(|n| n.as_str())
            .ok_or_else(|| ProtocolError("tool_call missing `name`".into()))?;
        let args = v.get("arguments").cloned().unwrap_or_else(|| serde_json::json!({}));
        let visible = format!("{}{}", &text[..start], &after[end_rel + 3..]).trim().to_string();
        Ok(ParsedTurn {
            text: visible,
            tool_calls: vec![ToolCall { id: "call_0".into(), name: name.into(), args }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    #[test]
    fn prepare_moves_schemas_into_system_prompt_and_clears_tools() {
        let mut req = CompletionRequest {
            messages: vec![Message::user("hi")],
            tools: vec![agent_tools::ToolSchema { name: "read_file".into(),
                description: "read".into(), parameters: serde_json::json!({"type":"object"}) }],
            temperature: 0.0, max_tokens: None };
        PromptedJsonProtocol.prepare(&mut req);
        assert!(req.tools.is_empty());
        let sys = req.messages.iter().find(|m| matches!(m.role, Role::System)).unwrap();
        assert!(sys.content.contains("read_file"));
    }

    #[test]
    fn parse_extracts_fenced_tool_call_block() {
        let text = "Let me read it.\n```tool_call\n{\"name\":\"read_file\",\
                    \"arguments\":{\"path\":\"a.txt\"}}\n```";
        let turn = AssistantTurn { text: text.into(), raw_tool_calls: vec![],
            stop: StopReason::Stop };
        let parsed = PromptedJsonProtocol.parse(&turn).unwrap();
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "read_file");
        assert_eq!(parsed.tool_calls[0].args["path"], "a.txt");
        assert!(parsed.text.contains("Let me read it"));
        assert!(!parsed.text.contains("```")); // block stripped from visible text
    }

    #[test]
    fn parse_returns_plain_text_when_no_block() {
        let turn = AssistantTurn { text: "all done".into(), raw_tool_calls: vec![],
            stop: StopReason::Stop };
        let parsed = PromptedJsonProtocol.parse(&turn).unwrap();
        assert!(parsed.tool_calls.is_empty());
        assert_eq!(parsed.text, "all done");
    }

    #[test]
    fn parse_errors_on_malformed_block() {
        let turn = AssistantTurn { text: "```tool_call\n{bad}\n```".into(),
            raw_tool_calls: vec![], stop: StopReason::Stop };
        assert!(PromptedJsonProtocol.parse(&turn).is_err());
    }
}
