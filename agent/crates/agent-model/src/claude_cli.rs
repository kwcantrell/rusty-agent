//! Claude Code CLI as a pure text-generation backend (`ModelClient`).
use crate::{Chunk, Message, ModelError, Role, StopReason};
use serde_json::Value;
use crate::{CompletionRequest, ModelClient};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// Replaces the Claude Code harness system prompt so it behaves as a plain
/// generator. The actual task instructions and tool-call protocol live in the
/// transcript piped on stdin (see `render_transcript` + the Prompted protocol).
const BARE_SYSTEM_PROMPT: &str =
    "You are a text generator. Follow the instructions in the message exactly.";

/// Drives the Claude Code CLI as a pure text generator.
pub struct ClaudeCliClient {
    binary: String,
    model: String,
}

impl ClaudeCliClient {
    pub fn new(binary: impl Into<String>, model: impl Into<String>) -> Self {
        Self { binary: binary.into(), model: model.into() }
    }
}

#[async_trait]
impl ModelClient for ClaudeCliClient {
    async fn stream(
        &self,
        req: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<Chunk, ModelError>>, ModelError> {
        let prompt = render_transcript(&req.messages);

        let mut child = Command::new(&self.binary)
            .arg("-p")
            .arg("--output-format").arg("stream-json")
            .arg("--verbose")
            .arg("--allowedTools").arg("")
            .arg("--model").arg(&self.model)
            // Run the CLI as a pure generator without losing subscription auth.
            // `--system-prompt` REPLACES the "you are Claude Code" harness prompt
            // (so it can't compete with the Prompted tool preamble on stdin); the
            // transcript still carries our own instructions in the piped prompt.
            .arg("--system-prompt").arg(BARE_SYSTEM_PROMPT)
            // Don't load the user's settings — that's where SessionStart hooks
            // live, which otherwise inject the whole skill harness into context.
            .arg("--setting-sources").arg("project")
            // Skip MCP discovery and session-to-disk writes (we're stateless).
            .arg("--strict-mcp-config")
            .arg("--no-session-persistence")
            // NOTE: do NOT use `--bare` — it forces ANTHROPIC_API_KEY/apiKeyHelper
            // auth and never reads OAuth/keychain, which defeats the whole point
            // of driving the CLI (piggybacking the Claude subscription).
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true) // kill the CLI if the stream is dropped/cancelled
            .spawn()
            .map_err(|e| ModelError::Process(format!("spawn {}: {e}", self.binary)))?;

        // Feed the prompt on a separate task so a large prompt can't deadlock
        // against the child filling its stdout pipe.
        let mut stdin = child.stdin.take().expect("stdin piped");
        tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            // stdin dropped here -> EOF for the child.
        });

        let stdout = child.stdout.take().expect("stdout piped");
        let mut stderr = child.stderr.take().expect("stderr piped");

        // Drain stderr concurrently so a child that writes more than one pipe
        // buffer (~64 KiB) to stderr after closing stdout cannot deadlock
        // child.wait().
        let stderr_task = tokio::spawn(async move {
            let mut buf = String::new();
            let _ = stderr.read_to_string(&mut buf).await;
            buf
        });

        let stream = async_stream::stream! {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => match parse_event_line(&line) {
                        Ok(chunks) => {
                            for c in chunks {
                                yield Ok(c);
                            }
                        }
                        Err(e) => {
                            yield Err(e);
                            return;
                        }
                    },
                    Ok(None) => break, // stdout EOF
                    Err(e) => {
                        yield Err(ModelError::Stream(e.to_string()));
                        return;
                    }
                }
            }

            // stdout drained; confirm a clean exit, else surface stderr.
            match child.wait().await {
                Ok(status) if status.success() => {}
                Ok(status) => {
                    let buf = stderr_task.await.unwrap_or_default();
                    yield Err(ModelError::Process(
                        format!("claude exited ({status}): {}", buf.trim())));
                }
                Err(e) => yield Err(ModelError::Process(e.to_string())),
            }
        };
        Ok(stream.boxed())
    }
}

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
mod proc_tests {
    use super::*;
    use crate::{CompletionRequest, Message, ModelClient};
    use futures::StreamExt;
    use serial_test::serial;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    /// Write an executable shell stub to a temp path and return it.
    fn write_fake(script: &str) -> tempfile::TempPath {
        let mut f = tempfile::Builder::new().prefix("fake-claude-").tempfile().unwrap();
        write!(f, "{script}").unwrap();
        f.flush().unwrap();
        f.as_file().sync_all().unwrap(); // settle the executable before exec() to avoid ETXTBSY under parallel test runs
        let path = f.into_temp_path();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    fn req() -> CompletionRequest {
        CompletionRequest {
            messages: vec![Message::user("hi")],
            ..Default::default()
        }
    }

    #[tokio::test]
    #[serial]
    async fn streams_text_then_done_from_fake_cli() {
        let script = "#!/usr/bin/env bash\ncat >/dev/null\n\
            echo '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"t\"}'\n\
            echo '{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"hello from fake\"}]},\"session_id\":\"t\"}'\n\
            echo '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"hello from fake\",\"session_id\":\"t\"}'\n";
        let fake = write_fake(script);
        let client = ClaudeCliClient::new(fake.to_str().unwrap(), "sonnet");
        let mut stream = client.stream(req()).await.unwrap();
        let mut text = String::new();
        let mut done = None;
        while let Some(item) = stream.next().await {
            match item.unwrap() {
                Chunk::Text(t) => text.push_str(&t),
                Chunk::Done(r) => done = Some(r),
                Chunk::ToolCallDelta(_) => {}
            }
        }
        assert_eq!(text, "hello from fake");
        assert_eq!(done, Some(StopReason::Stop));
    }

    #[tokio::test]
    #[serial]
    async fn nonzero_exit_surfaces_process_error() {
        let script = "#!/usr/bin/env bash\ncat >/dev/null\n\
            echo 'not authenticated' >&2\nexit 1\n";
        let fake = write_fake(script);
        let client = ClaudeCliClient::new(fake.to_str().unwrap(), "sonnet");
        let mut stream = client.stream(req()).await.unwrap();
        let mut err = None;
        while let Some(item) = stream.next().await {
            if let Err(e) = item {
                err = Some(e);
            }
        }
        match err {
            Some(ModelError::Process(msg)) => assert!(msg.contains("not authenticated"), "got: {msg}"),
            other => panic!("expected Process error, got {other:?}"),
        }
    }

    #[tokio::test]
    #[serial]
    async fn missing_binary_is_process_error() {
        let client = ClaudeCliClient::new("/nonexistent/claude-binary-xyz", "sonnet");
        let res = client.stream(req()).await;
        assert!(matches!(res, Err(ModelError::Process(_))));
    }

    #[tokio::test]
    #[serial]
    async fn forwards_bare_generator_flags() {
        // The fake CLI exits non-zero (→ Process error) unless every flag that
        // makes it behave as a subscription-auth bare generator is present, so a
        // future edit that drops one is caught by the clean-stream assertion.
        let script = "#!/usr/bin/env bash\ncat >/dev/null\n\
            for f in --system-prompt --setting-sources --strict-mcp-config --no-session-persistence --allowedTools; do\n\
              case \" $* \" in *\" $f \"*) ;; *) echo \"missing $f\" >&2; exit 3;; esac\n\
            done\n\
            echo '{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]},\"session_id\":\"t\"}'\n\
            echo '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"session_id\":\"t\"}'\n";
        let fake = write_fake(script);
        let client = ClaudeCliClient::new(fake.to_str().unwrap(), "sonnet");
        let mut stream = client.stream(req()).await.unwrap();
        let mut text = String::new();
        let mut done = None;
        while let Some(item) = stream.next().await {
            match item.unwrap() {
                Chunk::Text(t) => text.push_str(&t),
                Chunk::Done(r) => done = Some(r),
                Chunk::ToolCallDelta(_) => {}
            }
        }
        assert_eq!(text, "ok");
        assert_eq!(done, Some(StopReason::Stop));
    }

    #[tokio::test]
    #[serial]
    async fn large_stderr_on_failure_does_not_deadlock() {
        // Emit ~256 KiB to stderr (far past the ~64 KiB pipe buffer) after
        // closing stdout, then fail. Must not hang; must surface Process error.
        let script = "#!/usr/bin/env bash\ncat >/dev/null\n\
            yes 'errline' | head -c 262144 >&2\nexit 1\n";
        let fake = write_fake(script);
        let client = ClaudeCliClient::new(fake.to_str().unwrap(), "sonnet");
        let mut stream = client.stream(req()).await.unwrap();
        let collect = async {
            let mut err = None;
            while let Some(item) = stream.next().await {
                if let Err(e) = item { err = Some(e); }
            }
            err
        };
        let err = tokio::time::timeout(std::time::Duration::from_secs(10), collect)
            .await
            .expect("stream must not deadlock on large stderr");
        assert!(matches!(err, Some(ModelError::Process(_))), "expected Process error, got {err:?}");
    }
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
