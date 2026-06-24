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
    fn drops_preserved_reasoning_for_openai_backend() {
        // OpenAI-compat reasoning backends don't accept prior chain-of-thought
        // back (DeepSeek 400s on reasoning_content in input; Qwen templates strip
        // historical <think>). So reasoning is preserved as data but NOT re-sent.
        let m = Message::assistant("final answer", None).with_reasoning("secret plan");
        let v = &messages_to_json(&[m])[0];
        assert_eq!(v["content"], "final answer");
        assert!(v.get("reasoning_content").is_none());
        assert!(!v["content"].as_str().unwrap().contains("<think>"));
    }
}
