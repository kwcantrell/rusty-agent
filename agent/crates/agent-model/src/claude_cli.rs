//! Claude Code CLI as a pure text-generation backend (`ModelClient`).
use crate::{Chunk, Message, ModelError, Role, StopReason};
use serde_json::Value;

pub(crate) fn render_transcript(messages: &[Message]) -> String {
    let mut out = String::new();
    for m in messages {
        let header = match m.role {
            Role::System => "## System".to_string(),
            Role::User => "## User".to_string(),
            Role::Assistant => "## Assistant".to_string(),
            Role::Tool => {
                let name = m.name.as_deref().unwrap_or("tool");
                format!("## Tool ({name})")
            }
        };
        out.push_str(&header);
        out.push('\n');
        out.push_str(&m.content);
        out.push_str("\n\n");
    }
    out
}

pub(crate) fn parse_event_line(line: &str) -> Result<Vec<Chunk>, ModelError> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(vec![]);
    }
    let v: Value = serde_json::from_str(line).map_err(|e| ModelError::Decode(e.to_string()))?;
    let mut out = Vec::new();
    match v["type"].as_str() {
        Some("assistant") => {
            if let Some(blocks) = v["message"]["content"].as_array() {
                for b in blocks {
                    if b["type"] == "text" {
                        if let Some(t) = b["text"].as_str() {
                            if !t.is_empty() {
                                out.push(Chunk::Text(t.to_string()));
                            }
                        }
                    }
                }
            }
        }
        Some("result") => {
            // `Length` only when the CLI signals truncation; otherwise a normal stop.
            let truncated = v["subtype"].as_str() == Some("error_max_turns")
                || v["stop_reason"].as_str() == Some("max_tokens");
            out.push(Chunk::Done(if truncated {
                StopReason::Length
            } else {
                StopReason::Stop
            }));
        }
        _ => {} // system/init, user echoes, etc. — nothing to emit.
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Message;

    // NOTE: replace these two literals with the verbatim lines captured in
    // docs/superpowers/context/claude-cli-inference.md (Task 0, Step 5) if the
    // real shapes differ.
    const ASSISTANT_LINE: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hello world"}]},"session_id":"t"}"#;
    const RESULT_LINE: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"hello world","session_id":"t"}"#;

    #[test]
    fn parses_assistant_text_into_text_chunk() {
        let chunks = parse_event_line(ASSISTANT_LINE).unwrap();
        assert_eq!(chunks.len(), 1);
        match &chunks[0] {
            Chunk::Text(t) => assert_eq!(t, "hello world"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn result_event_emits_done_stop() {
        let chunks = parse_event_line(RESULT_LINE).unwrap();
        assert!(matches!(chunks.as_slice(), [Chunk::Done(StopReason::Stop)]));
    }

    #[test]
    fn ignores_system_init_lines() {
        let line = r#"{"type":"system","subtype":"init","session_id":"t"}"#;
        assert!(parse_event_line(line).unwrap().is_empty());
    }

    #[test]
    fn blank_line_yields_nothing() {
        assert!(parse_event_line("  ").unwrap().is_empty());
    }

    #[test]
    fn non_json_line_is_decode_error() {
        assert!(matches!(parse_event_line("not json"), Err(ModelError::Decode(_))));
    }

    #[test]
    fn renders_roles_with_headers() {
        let msgs = vec![
            Message::system("you are a coding agent"),
            Message::user("read a.txt"),
        ];
        let p = render_transcript(&msgs);
        assert!(p.contains("## System\nyou are a coding agent"));
        assert!(p.contains("## User\nread a.txt"));
        // System must come before User.
        assert!(p.find("## System").unwrap() < p.find("## User").unwrap());
    }

    #[test]
    fn tool_message_includes_tool_name_in_header() {
        let msgs = vec![Message::tool("call_0", "read_file", "file contents here")];
        let p = render_transcript(&msgs);
        assert!(p.contains("## Tool (read_file)\nfile contents here"), "got: {p}");
    }

    #[test]
    fn assistant_message_rendered() {
        let msgs = vec![Message::assistant("on it", None)];
        let p = render_transcript(&msgs);
        assert!(p.contains("## Assistant\non it"));
    }
}
