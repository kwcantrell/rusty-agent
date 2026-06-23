# Claude CLI Inference Backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `ClaudeCliClient` that implements the existing `ModelClient` trait by driving the authenticated Claude Code CLI as a pure text generator, so the Rust agent loop can run against Claude-quality reasoning via a Claude subscription instead of SGLang.

**Architecture:** A new module in the `agent-model` crate spawns `claude -p --output-format stream-json` per turn with its own tools disabled, pipes the linearized transcript on stdin, and translates the CLI's stdout JSON event stream into the existing `Chunk` stream. Tool calls are produced/parsed by the existing `Prompted` protocol — nothing above the `ModelClient` trait changes. Backend selection is a new `--backend {openai|claude-cli}` flag in `agent-cli` and `agent-server`.

**Tech Stack:** Rust (edition 2021), `tokio` process + async I/O, `async-stream`, `serde_json`, `futures`. Spec: `docs/superpowers/specs/2026-06-23-claude-cli-inference-backend-design.md`.

## Global Constraints

- Edition `2021`, license `MIT` (workspace defaults — do not add per-crate overrides).
- Cargo is **not** on `PATH`. Every cargo command must be prefixed: `source ~/.cargo/env && cargo …`.
- Run all cargo commands from the `agent/` directory (the workspace root): `cd /home/kalen/rust-agent-runtime/agent`.
- The `ModelClient` trait, `Chunk`, `Message`, `CompletionRequest`, and the `Prompted` protocol are FIXED. Do not modify them. `ClaudeCliClient` must satisfy the existing trait exactly:
  `async fn stream(&self, req: CompletionRequest) -> Result<BoxStream<'static, Result<Chunk, ModelError>>, ModelError>`.
- When `--backend claude-cli` is selected, `--protocol` is forced to `prompted` (native OpenAI `tool_calls` are unavailable from a disabled-tools CLI). Default backend is `openai`, preserving all current behavior.
- Follow existing crate conventions: pure parsing functions are unit-tested in a `#[cfg(test)] mod tests` block in the same file (see `openai.rs::parse_sse_line` tests); model clients are `Send + Sync`.

## File Structure

- `agent/crates/agent-model/src/claude_cli.rs` — **new.** The whole backend: `parse_event_line` (stdout JSON → `Chunk`), `render_transcript` (`&[Message]` → prompt string), and `ClaudeCliClient` (`ModelClient` impl spawning the subprocess). One file, one responsibility (the Claude-CLI backend), mirroring how `openai.rs` holds the entire OpenAI backend.
- `agent/crates/agent-model/src/lib.rs` — **modify.** Add `mod claude_cli; pub use claude_cli::*;`.
- `agent/crates/agent-model/src/types.rs` — **modify.** Add one `ModelError::Process(String)` variant.
- `agent/crates/agent-model/Cargo.toml` — **modify.** Promote `tokio` from dev-dependency to a normal dependency.
- `agent/crates/agent-runtime-config/src/lib.rs` — **modify.** Add `build_model(...) -> Arc<dyn ModelClient>` + `backend_name_is_valid`.
- `agent/crates/agent-runtime-config/Cargo.toml` — **modify.** Confirm/add `agent-model` dep (already present) — no change expected.
- `agent/crates/agent-cli/src/main.rs` — **modify.** Add `--backend` / `--claude-binary` flags; build client via `build_model`; force prompted protocol for claude-cli.
- `agent/crates/agent-server/src/main.rs` — **modify.** Same flag + wiring in the `Run` subcommand.
- `docs/superpowers/context/claude-cli-inference.md` — **new** (Task 0). Records the spike's real sample event lines + go/no-go decision; its captured lines become the fixtures used in Task 1.

---

## Task 0: Phase 0 Spike — validate CLI behaves as a pure generator (GATING, no Rust)

This task writes **no Rust**. It is a manual investigation that must pass before any other task starts. Its job is to retire the single biggest risk (the CLI's baked-in agent system prompt fighting the `Prompted` fence format) and to capture the **real** stdout-event shapes that Task 1's parser and tests depend on.

**Files:**
- Create: `docs/superpowers/context/claude-cli-inference.md`

- [ ] **Step 1: Confirm the CLI is installed and authenticated**

Run:
```bash
which claude && claude --version
```
Expected: a path and a version string. If missing, stop — the user must install/authenticate Claude Code first.

- [ ] **Step 2: Capture raw stream-json output for a plain prompt**

Run:
```bash
printf 'Say exactly: hello world' | claude -p --output-format stream-json --verbose --allowedTools "" --model sonnet
```
Expected: newline-delimited JSON objects. Save the full output. Identify:
- the event `type` that carries assistant text and the JSON path to that text (anticipated: `type":"assistant"` with `message.content[].type=="text"` → `.text`),
- the terminal event (anticipated: `type":"result"` with `subtype` / `is_error`),
- any field that signals truncation / max-tokens (for `StopReason::Length`).

- [ ] **Step 3: Confirm tools are genuinely disabled**

Inspect the captured events from Step 2 for any `tool_use` / tool-execution events. Expected: none — with `--allowedTools ""` the CLI should not invoke Read/Bash/etc. Note the result in the doc.

- [ ] **Step 4: Confirm the CLI respects the `Prompted` fence format**

Run (this mimics what `PromptedJsonProtocol::prepare` injects):
```bash
printf 'You can call tools. To call one, emit a fenced block exactly like:\n```tool_call\n{"name":"<tool>","arguments":{...}}\n```\nEmit at most one tool_call block per reply. Available tools:\n- read_file: read a file | schema: {"type":"object","properties":{"path":{"type":"string"}}}\n\n## User\nRead the file a.txt' | claude -p --output-format stream-json --verbose --allowedTools "" --model sonnet
```
Expected: the assistant text contains a ` ```tool_call ` block with `{"name":"read_file","arguments":{"path":"a.txt"}}` (or very close). Judge: does it reliably emit the fence, or does it editorialize / refuse / ignore the format?

- [ ] **Step 5: Record the go/no-go decision**

Write `docs/superpowers/context/claude-cli-inference.md` containing:
1. The exact `claude` flags that worked.
2. **Two real captured event lines** verbatim — one assistant-text event and one terminal result event (these become Task 1's test fixtures).
3. The truncation-signal field name, if any.
4. The decision:
   - **PASS** (emits clean text + reliably respects the fence + tools disabled) → proceed to Task 1.
   - **FAIL** → stop and report to the user. Fallback options to discuss: use `--append-system-prompt` and accept harness-prompt coexistence, or build an Agent SDK sidecar. **Do not proceed to Task 1 on FAIL.**

- [ ] **Step 6: Commit the spike notes**

```bash
cd /home/kalen/rust-agent-runtime
git add docs/superpowers/context/claude-cli-inference.md
git commit -m "docs(spike): Claude CLI stream-json shapes + go/no-go for inference backend"
```

> **Gate:** Tasks 1–4 assume Step 5 returned PASS. If the real captured events in the spike doc differ from the anticipated shapes used in the code below, adjust the JSON paths in `parse_event_line` (Task 1) and the fixtures accordingly — the captured lines are the source of truth.

---

## Task 1: stdout JSON event → `Chunk` parser

**Files:**
- Modify: `agent/crates/agent-model/Cargo.toml` (promote `tokio` to a normal dependency)
- Create: `agent/crates/agent-model/src/claude_cli.rs`
- Modify: `agent/crates/agent-model/src/lib.rs` (register the module)
- Modify: `agent/crates/agent-model/src/types.rs` (add `ModelError::Process`)

**Interfaces:**
- Consumes: `Chunk` and `ModelError` from `crate::types`; `StopReason`.
- Produces: `pub(crate) fn parse_event_line(line: &str) -> Result<Vec<Chunk>, ModelError>` — parses ONE line of CLI stdout. Returns `Ok(vec![])` for irrelevant/empty lines (system/init, user echoes), `Ok` chunks for assistant-text and terminal events, `Err(ModelError::Decode)` for non-JSON lines.

- [ ] **Step 1: Promote `tokio` to a normal dependency**

In `agent/crates/agent-model/Cargo.toml`, move `tokio` out of `[dev-dependencies]` into `[dependencies]` (it stays in dev too implicitly via the normal dep). Resulting `[dependencies]` gains:
```toml
tokio = { workspace = true }
```
And remove the `tokio = { workspace = true }` line from `[dev-dependencies]` (the normal dep covers tests). The workspace `tokio` already has `features = ["full"]`, which includes `process`, `io-util`, and `macros`.

- [ ] **Step 2: Add the `Process` error variant**

In `agent/crates/agent-model/src/types.rs`, add to the `ModelError` enum (after `Stream`):
```rust
    #[error("process error: {0}")]
    Process(String),
```
(`ModelError` is only ever stringified — no exhaustive match sites exist — so this is safe.)

- [ ] **Step 3: Register the module**

In `agent/crates/agent-model/src/lib.rs`, after the `openai` lines, add:
```rust
mod claude_cli;
pub use claude_cli::*;
```

- [ ] **Step 4: Write the failing parser tests**

Create `agent/crates/agent-model/src/claude_cli.rs` with ONLY the test module and a stub, using the **real captured lines from Task 0** where noted:
```rust
//! Claude Code CLI as a pure text-generation backend (`ModelClient`).
use crate::{Chunk, ModelError, StopReason};

pub(crate) fn parse_event_line(_line: &str) -> Result<Vec<Chunk>, ModelError> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
```

- [ ] **Step 5: Run the tests to verify they fail**

Run:
```bash
cd /home/kalen/rust-agent-runtime/agent && source ~/.cargo/env && cargo test -p agent-model claude_cli 2>&1 | tail -20
```
Expected: compiles, tests FAIL/panic at `unimplemented!()`.

- [ ] **Step 6: Implement `parse_event_line`**

Replace the stub in `claude_cli.rs`:
```rust
use serde_json::Value;

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
```
(Keep `use crate::{Chunk, ModelError, StopReason};` at the top; add the `serde_json::Value` import.)

- [ ] **Step 7: Run the tests to verify they pass**

Run:
```bash
cd /home/kalen/rust-agent-runtime/agent && source ~/.cargo/env && cargo test -p agent-model claude_cli 2>&1 | tail -20
```
Expected: all 5 tests PASS.

- [ ] **Step 8: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-model/src/claude_cli.rs agent/crates/agent-model/src/lib.rs agent/crates/agent-model/src/types.rs agent/crates/agent-model/Cargo.toml
git commit -m "feat(model): parse Claude CLI stream-json events into Chunks"
```

---

## Task 2: Transcript → prompt renderer

**Files:**
- Modify: `agent/crates/agent-model/src/claude_cli.rs`

**Interfaces:**
- Consumes: `crate::{Message, Role}`.
- Produces: `pub(crate) fn render_transcript(messages: &[Message]) -> String` — linearizes an (already `Prompted::prepare`-processed) message list into a single role-delimited prompt string for `claude -p` stdin.

- [ ] **Step 1: Write the failing renderer tests**

Add to the `tests` module in `claude_cli.rs`:
```rust
    use crate::Message;

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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run:
```bash
cd /home/kalen/rust-agent-runtime/agent && source ~/.cargo/env && cargo test -p agent-model claude_cli::tests 2>&1 | tail -20
```
Expected: compile error (`render_transcript` not found) or FAIL.

- [ ] **Step 3: Implement `render_transcript`**

Add to `claude_cli.rs` (extend the `use crate::{...}` line to include `Message, Role`):
```rust
use crate::{Message, Role};

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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run:
```bash
cd /home/kalen/rust-agent-runtime/agent && source ~/.cargo/env && cargo test -p agent-model claude_cli::tests 2>&1 | tail -20
```
Expected: all tests (Task 1 + Task 2) PASS.

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-model/src/claude_cli.rs
git commit -m "feat(model): render transcript into role-delimited Claude CLI prompt"
```

---

## Task 3: `ClaudeCliClient` — subprocess `ModelClient` impl

**Files:**
- Modify: `agent/crates/agent-model/src/claude_cli.rs`

**Interfaces:**
- Consumes: `parse_event_line`, `render_transcript` (Task 1/2); `crate::{CompletionRequest, Chunk, ModelClient, ModelError}`.
- Produces: `pub struct ClaudeCliClient` with `pub fn new(binary: impl Into<String>, model: impl Into<String>) -> Self` and an `impl ModelClient`. `binary` is the path/name of the `claude` executable (configurable so tests can substitute a fake and users can point at a non-`PATH` binary).

- [ ] **Step 1: Write the failing integration tests (fake `claude` script)**

Add a second test module at the bottom of `claude_cli.rs` (separate from the unit `tests` mod because these are async and write temp files):
```rust
#[cfg(test)]
mod proc_tests {
    use super::*;
    use crate::{CompletionRequest, Message, ModelClient};
    use futures::StreamExt;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    /// Write an executable shell stub to a temp path and return it.
    fn write_fake(script: &str) -> tempfile::TempPath {
        let mut f = tempfile::Builder::new().prefix("fake-claude-").tempfile().unwrap();
        write!(f, "{script}").unwrap();
        let path = f.into_temp_path();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    fn req() -> CompletionRequest {
        CompletionRequest {
            messages: vec![Message::user("hi")],
            tools: vec![],
            temperature: 0.0,
            max_tokens: None,
        }
    }

    #[tokio::test]
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
    async fn missing_binary_is_process_error() {
        let client = ClaudeCliClient::new("/nonexistent/claude-binary-xyz", "sonnet");
        let res = client.stream(req()).await;
        assert!(matches!(res, Err(ModelError::Process(_))));
    }
}
```
Add `tempfile` to `[dev-dependencies]` in `agent/crates/agent-model/Cargo.toml`:
```toml
tempfile.workspace = true
```

- [ ] **Step 2: Run the tests to verify they fail**

Run:
```bash
cd /home/kalen/rust-agent-runtime/agent && source ~/.cargo/env && cargo test -p agent-model proc_tests 2>&1 | tail -20
```
Expected: compile error (`ClaudeCliClient` not found).

- [ ] **Step 3: Implement `ClaudeCliClient`**

Add to `claude_cli.rs` (extend imports as shown). Full implementation:
```rust
use crate::{CompletionRequest, ModelClient};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

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
        let stderr = child.stderr.take().expect("stderr piped");

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
                    let mut buf = String::new();
                    let _ = BufReader::new(stderr).read_to_string(&mut buf).await;
                    yield Err(ModelError::Process(
                        format!("claude exited ({status}): {}", buf.trim())));
                }
                Err(e) => yield Err(ModelError::Process(e.to_string())),
            }
        };
        Ok(stream.boxed())
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run:
```bash
cd /home/kalen/rust-agent-runtime/agent && source ~/.cargo/env && cargo test -p agent-model 2>&1 | tail -25
```
Expected: all `agent-model` tests pass, including the three `proc_tests`.

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-model/src/claude_cli.rs agent/crates/agent-model/Cargo.toml
git commit -m "feat(model): ClaudeCliClient drives claude -p as a streaming ModelClient"
```

---

## Task 4: Wire the backend into runtime-config, CLI, and daemon

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/lib.rs`
- Modify: `agent/crates/agent-cli/src/main.rs`
- Modify: `agent/crates/agent-server/src/main.rs`

**Interfaces:**
- Consumes: `agent_model::{ClaudeCliClient, OpenAiCompatClient, ModelClient}`.
- Produces:
  - `pub fn backend_name_is_valid(name: &str) -> bool`
  - `pub fn build_model(backend: &str, base_url: &str, model: &str, claude_binary: &str, api_key: Option<String>) -> Arc<dyn ModelClient>` — returns a `ClaudeCliClient` for `"claude-cli"`, else an `OpenAiCompatClient`.

- [ ] **Step 1: Write the failing runtime-config test**

In `agent/crates/agent-runtime-config/src/lib.rs`, add to the `tests` module:
```rust
    #[test]
    fn backend_validation() {
        assert!(backend_name_is_valid("openai"));
        assert!(backend_name_is_valid("claude-cli"));
        assert!(!backend_name_is_valid("bogus"));
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run:
```bash
cd /home/kalen/rust-agent-runtime/agent && source ~/.cargo/env && cargo test -p agent-runtime-config 2>&1 | tail -15
```
Expected: compile error (`backend_name_is_valid` not found).

- [ ] **Step 3: Implement the helpers**

In `agent/crates/agent-runtime-config/src/lib.rs`, extend the top `use` line to include the clients and `ModelClient`:
```rust
use agent_model::{ClaudeCliClient, ModelClient, NativeProtocol, OpenAiCompatClient,
                  PromptedJsonProtocol, ToolCallProtocol};
```
Then add (near `pick_protocol`):
```rust
pub fn backend_name_is_valid(name: &str) -> bool {
    matches!(name, "openai" | "claude-cli")
}

/// Build the model client for the selected backend.
/// `claude-cli` ignores `base_url`/`api_key`; `openai` ignores `claude_binary`.
pub fn build_model(
    backend: &str,
    base_url: &str,
    model: &str,
    claude_binary: &str,
    api_key: Option<String>,
) -> Arc<dyn ModelClient> {
    match backend {
        "claude-cli" => Arc::new(ClaudeCliClient::new(claude_binary, model)),
        _ => Arc::new(OpenAiCompatClient::new(base_url.to_string(), model.to_string(), api_key)),
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run:
```bash
cd /home/kalen/rust-agent-runtime/agent && source ~/.cargo/env && cargo test -p agent-runtime-config 2>&1 | tail -15
```
Expected: PASS.

- [ ] **Step 5: Wire `agent-cli`**

In `agent/crates/agent-cli/src/main.rs`:

(a) Replace the `use agent_model::{Message, OpenAiCompatClient};` line with:
```rust
use agent_model::Message;
```
(b) Extend the `use agent_runtime_config::{...}` line to add `backend_name_is_valid, build_model`:
```rust
use agent_runtime_config::{backend_name_is_valid, build_registry, build_model,
    default_allowlist, default_denylist, pick_protocol};
```
(c) Add two fields to the `Cli` struct (after `base_url`):
```rust
    /// Inference backend: openai | claude-cli
    #[arg(long, default_value = "openai")]
    backend: String,
    /// Path/name of the Claude Code CLI binary (claude-cli backend only)
    #[arg(long, default_value = "claude")]
    claude_binary: String,
```
(d) Replace the model-construction + protocol lines:
```rust
    let api_key = std::env::var("AGENT_API_KEY").ok();
    let model = Arc::new(OpenAiCompatClient::new(cli.base_url.clone(), cli.model.clone(), api_key));
    let protocol = pick_protocol(&cli.protocol);
```
with:
```rust
    if !backend_name_is_valid(&cli.backend) {
        eprintln!("unknown --backend '{}': use openai | claude-cli", cli.backend);
        std::process::exit(2);
    }
    let api_key = std::env::var("AGENT_API_KEY").ok();
    let model = build_model(&cli.backend, &cli.base_url, &cli.model, &cli.claude_binary, api_key);
    // claude-cli is a pure text generator; tool calls must come via the prompted protocol.
    let protocol_name = if cli.backend == "claude-cli" {
        if cli.protocol != "prompted" {
            eprintln!("note: forcing --protocol prompted for claude-cli backend");
        }
        "prompted"
    } else {
        cli.protocol.as_str()
    };
    let protocol = pick_protocol(protocol_name);
```
(`build_model` already returns `Arc<dyn ModelClient>`, so drop the now-unused `Arc::new` wrapper around the client — `model` is passed directly to `AgentLoop::new`.)

- [ ] **Step 6: Wire `agent-server`**

In `agent/crates/agent-server/src/main.rs`:

(a) Replace `use agent_model::OpenAiCompatClient;` with:
```rust
use agent_runtime_config::{backend_name_is_valid, build_model};
```
(b) In the `Run` subcommand variant, add fields after `base_url`:
```rust
        #[arg(long, default_value = "openai")]
        backend: String,
        #[arg(long, default_value = "claude")]
        claude_binary: String,
```
(c) Update the `Cmd::Run { .. }` destructuring pattern to include `backend, claude_binary`.
(d) Replace:
```rust
            let api_key = std::env::var("AGENT_API_KEY").ok();
            let client = Arc::new(OpenAiCompatClient::new(base_url, model, api_key));
```
with:
```rust
            if !backend_name_is_valid(&backend) {
                eprintln!("unknown --backend '{backend}': use openai | claude-cli");
                std::process::exit(2);
            }
            let api_key = std::env::var("AGENT_API_KEY").ok();
            let client = build_model(&backend, &base_url, &model, &claude_binary, api_key);
```
(e) Force the prompted protocol for claude-cli. The `protocol` variable is moved into `DaemonParams`; just before constructing `params`, add:
```rust
            let protocol = if backend == "claude-cli" {
                if protocol != "prompted" {
                    eprintln!("note: forcing --protocol prompted for claude-cli backend");
                }
                "prompted".to_string()
            } else {
                protocol
            };
```
(Place this after the `protocol` binding from the `Run { protocol, .. }` destructure and before `DaemonParams { .. protocol, .. }`.)

- [ ] **Step 7: Build the whole workspace**

Run:
```bash
cd /home/kalen/rust-agent-runtime/agent && source ~/.cargo/env && cargo build 2>&1 | tail -25
```
Expected: clean build, no warnings about unused `OpenAiCompatClient` import.

- [ ] **Step 8: Run the full test suite**

Run:
```bash
cd /home/kalen/rust-agent-runtime/agent && source ~/.cargo/env && cargo test 2>&1 | tail -30
```
Expected: all tests pass (existing + new). Note: the existing `e2e_sglang.rs` test may require a running SGLang server — if it was already gated/ignored, leave it as-is; do not "fix" it as part of this work.

- [ ] **Step 9: Manual smoke test (requires real authenticated CLI)**

Run from the repo root against a real workspace:
```bash
cd /home/kalen/rust-agent-runtime/agent && source ~/.cargo/env && \
  cargo run -p agent-cli -- --backend claude-cli --model sonnet --workspace .
```
Then type a simple task (e.g. `list the files in the current directory`). Expected: the agent emits a `tool_call` for `list_directory`, your loop executes it, and Claude summarizes — confirming the end-to-end path. (This requires Task 0 to have PASSED and an authenticated CLI; skip if running in CI.)

- [ ] **Step 10: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-runtime-config/src/lib.rs agent/crates/agent-cli/src/main.rs agent/crates/agent-server/src/main.rs
git commit -m "feat(cli,server): add --backend claude-cli to run against the Claude CLI"
```

---

## Self-Review

**Spec coverage:**
- Motivation / subscription-auth path → Task 0 validates the real CLI; Task 4 exposes `--backend claude-cli` (no API key path). ✓
- Pure-generator + Prompted protocol → Task 4 forces `prompted`; tools disabled via `--allowedTools ""` in Task 3. ✓
- New `ModelClient` impl, not an HTTP shim → Task 3. ✓
- Stateless per-turn subprocess → Task 3 spawns per `stream()` call; no `--resume`. ✓
- Backend selection via `--backend`, `--base-url` ignored for claude-cli → Task 4 (`build_model`). ✓
- Transcript flatten-to-text rendering → Task 2. ✓
- Streaming translation (text deltas + Done, Length on truncation) → Task 1. ✓
- Error mappings (binary-not-found, non-zero exit w/ stderr, auth/rate-limit surfaced via stderr) → Task 3 (`ModelError::Process`) + `missing_binary`/`nonzero_exit` tests. ✓
- Cancellation kills the child → `kill_on_drop(true)` in Task 3. ✓
- Testing: unit JSON parser tests + fake-`claude` hermetic integration test → Tasks 1 & 3. ✓
- Phase 0 spike gating → Task 0. ✓

**Placeholder scan:** No TBD/TODO/"handle errors appropriately" — every code step shows full code. The only intentional deferral is replacing Task 1 fixtures with Task 0's captured lines, which is explicit and conditional. ✓

**Type consistency:** `parse_event_line(&str) -> Result<Vec<Chunk>, ModelError>`, `render_transcript(&[Message]) -> String`, `ClaudeCliClient::new(impl Into<String>, impl Into<String>)`, `build_model(&str, &str, &str, &str, Option<String>) -> Arc<dyn ModelClient>` — names/signatures match across Tasks 1→4. `ModelError::Process(String)` defined in Task 1, used in Tasks 3 & 4. ✓
