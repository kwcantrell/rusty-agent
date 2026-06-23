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
}
