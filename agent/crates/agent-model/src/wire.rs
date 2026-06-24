use crate::{Message, Role};
use serde_json::{json, Value};

/// Serialize our `Message` list into OpenAI chat-completions JSON.
pub fn messages_to_json(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| {
            let role = match m.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            };
            let mut obj = json!({ "role": role, "content": m.content });
            // Preserved chain-of-thought goes in `reasoning_content` — the field
            // Qwen3.6's chat template reads first and re-includes when
            // chat_template_kwargs.preserve_thinking is set (see body()).
            if let Some(reasoning) = &m.reasoning {
                obj["reasoning_content"] = json!(reasoning);
            }
            if let Some(id) = &m.tool_call_id {
                obj["tool_call_id"] = json!(id);
            }
            if let Some(calls) = &m.tool_calls {
                obj["tool_calls"] = json!(calls
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.id,
                            "type": "function",
                            "function": { "name": c.name, "arguments": c.args.to_string() }
                        })
                    })
                    .collect::<Vec<_>>());
            }
            obj
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_tool_result_message() {
        let m = Message::tool("c1", "read_file", "data");
        let v = &messages_to_json(&[m])[0];
        assert_eq!(v["role"], "tool");
        assert_eq!(v["tool_call_id"], "c1");
        assert_eq!(v["content"], "data");
    }

    #[test]
    fn emits_reasoning_content_for_qwen_round_trip() {
        // Qwen3.6's chat template reads `message.reasoning_content` first, then
        // includes it when chat_template_kwargs.preserve_thinking is true. We send
        // preserved reasoning there and keep `content` as just the answer.
        let m = Message::assistant("final answer", None).with_reasoning("secret plan");
        let v = &messages_to_json(&[m])[0];
        assert_eq!(v["content"], "final answer");
        assert_eq!(v["reasoning_content"], "secret plan");
        assert!(!v["content"].as_str().unwrap().contains("<think>"));
    }

    #[test]
    fn omits_reasoning_content_when_message_has_none() {
        let m = Message::assistant("final answer", None);
        let v = &messages_to_json(&[m])[0];
        assert!(v.get("reasoning_content").is_none());
    }
}
