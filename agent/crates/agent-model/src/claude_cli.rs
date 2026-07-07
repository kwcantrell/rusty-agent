//! Claude Code CLI as a pure text-generation backend (`ModelClient`).
use crate::{Chunk, Message, ModelError, Role, StopReason};
use crate::{CompletionRequest, ModelClient};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// Replaces the Claude Code harness system prompt so it behaves as a plain
/// generator. The actual task instructions and tool-call protocol live in the
/// transcript piped on stdin (see `render_transcript` + the Prompted protocol).
const BARE_SYSTEM_PROMPT: &str =
    "You are a text generator. Follow the instructions in the message exactly.";

/// Behavior knobs for the claude-cli backend. `Default` reproduces the
/// pre-optimization behavior exactly (stateless, no knob flags).
#[derive(Debug, Clone, Default)]
pub struct ClaudeCliOptions {
    /// Resume the CLI session across calls when the transcript extends
    /// append-only (delta resume). Off = stateless full send every call.
    pub session_reuse: bool,
    /// `--effort <level>`; validated upstream against the CLI's accepted set.
    pub effort: Option<String>,
    /// `--fallback-model <model>` when the primary is unavailable.
    pub fallback_model: Option<String>,
}

/// Upper bound on pooled session states. One-shot callers (compaction, evals
/// through a reuse-enabled client) commit entries that never match again; the
/// cap bounds that growth. Eviction is oldest-first by insertion order.
pub(crate) const MAX_POOLED_SESSIONS: usize = 8;

/// Session state carried across calls for delta resume.
#[derive(Debug, Clone)]
struct SessionState {
    /// Captured from the init event of a persisted spawn; needed to resume.
    session_id: Option<String>,
    /// Whether the CLI wrote this session to disk (resumable).
    persisted: bool,
    /// One hash per transcript message this session has covered.
    fingerprints: Vec<u64>,
}

/// How to spawn the CLI for a given transcript.
enum SpawnPlan {
    /// Full transcript, `--no-session-persistence` (pre-optimization behavior).
    FreshEphemeral,
    /// Full transcript, persistence on; the init event's session_id is recorded.
    /// Costs one extra full send per session so one-shot callers (compaction,
    /// evals) never write session files to disk.
    FreshPersisted,
    /// `--resume <id>`; pipe only `messages[suffix_start..]`, assistant turns
    /// skipped (the CLI session already holds its own replies).
    Resume {
        session_id: String,
        suffix_start: usize,
    },
}

/// Hash a single message by role discriminant, name, content, and reasoning.
fn fingerprint(m: &Message) -> u64 {
    let mut h = DefaultHasher::new();
    std::mem::discriminant(&m.role).hash(&mut h);
    m.name.hash(&mut h);
    m.content.hash(&mut h);
    m.reasoning.hash(&mut h);
    h.finish()
}

/// Returns true iff `new` strictly extends `old`: longer, and byte-identical
/// on the shared prefix.
fn is_strict_extension(old: &[u64], new: &[u64]) -> bool {
    new.len() > old.len() && new[..old.len()] == *old
}

/// Drives the Claude Code CLI as a pure text generator.
pub struct ClaudeCliClient {
    binary: String,
    model: String,
    opts: ClaudeCliOptions,
    /// Pool of resumable session states, one per transcript family this client
    /// has served (parent loop, sibling subagents, compaction one-shots). An
    /// entry is CHECKED OUT (removed) while a call built on it is in flight and
    /// re-inserted only on success — see `plan_spawn`.
    sessions: Arc<Mutex<Vec<SessionState>>>,
}

impl ClaudeCliClient {
    pub fn new(binary: impl Into<String>, model: impl Into<String>) -> Self {
        Self::with_options(binary, model, ClaudeCliOptions::default())
    }

    pub fn with_options(
        binary: impl Into<String>,
        model: impl Into<String>,
        opts: ClaudeCliOptions,
    ) -> Self {
        Self {
            binary: binary.into(),
            model: model.into(),
            opts,
            sessions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn base_command(&self) -> Command {
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-p")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            // Token-level deltas instead of whole assistant messages.
            .arg("--include-partial-messages")
            .arg("--allowedTools")
            .arg("")
            .arg("--model")
            .arg(&self.model)
            // `--system-prompt` REPLACES the "you are Claude Code" harness prompt
            // (so it can't compete with the Prompted tool preamble on stdin).
            .arg("--system-prompt")
            .arg(BARE_SYSTEM_PROMPT)
            // Don't load the user's settings — that's where SessionStart hooks live.
            .arg("--setting-sources")
            .arg("project")
            .arg("--strict-mcp-config");
        if let Some(e) = &self.opts.effort {
            cmd.arg("--effort").arg(e);
        }
        if let Some(f) = &self.opts.fallback_model {
            cmd.arg("--fallback-model").arg(f);
        }
        // The CLI authenticates via its own subscription/OAuth — it must not
        // inherit the runtime's model API key.
        // NOTE: do NOT use `--bare` — it forces API-key auth and never reads
        // OAuth/keychain, defeating the subscription piggyback.
        cmd.env_remove("AGENT_API_KEY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        cmd
    }

    /// Decide how to spawn for this transcript and produce the pending state to
    /// commit on success.
    ///
    /// Checkout semantics: the longest strict-prefix entry is REMOVED from the
    /// pool while this call is in flight. A concurrent caller with the same
    /// prefix then matches nothing and degrades to a fresh send — two callers
    /// can never resume the same session simultaneously. Success re-inserts the
    /// updated state (in `stream()`); failure simply never re-inserts, so the
    /// loop's retry lands on a fresh full send.
    fn plan_spawn(&self, messages: &[Message]) -> (SpawnPlan, SessionState) {
        let fps: Vec<u64> = messages.iter().map(fingerprint).collect();
        let fresh = |persisted: bool| SessionState {
            session_id: None,
            persisted,
            fingerprints: fps.clone(),
        };
        if !self.opts.session_reuse {
            return (SpawnPlan::FreshEphemeral, fresh(false));
        }
        let checked_out = {
            let mut pool = self.sessions.lock().expect("session pool lock");
            let best = pool
                .iter()
                .enumerate()
                .filter(|(_, s)| is_strict_extension(&s.fingerprints, &fps))
                .max_by_key(|(_, s)| s.fingerprints.len())
                .map(|(i, _)| i);
            best.map(|i| pool.remove(i))
        };
        match checked_out {
            Some(s) => {
                if !s.persisted {
                    // First extension: pay one full send to make the session resumable.
                    return (SpawnPlan::FreshPersisted, fresh(true));
                }
                let suffix_start = s.fingerprints.len();
                let suffix_has_content = messages[suffix_start..]
                    .iter()
                    .any(|m| m.role != Role::Assistant);
                match (s.session_id, suffix_has_content) {
                    (Some(id), true) => (
                        SpawnPlan::Resume {
                            session_id: id.clone(),
                            suffix_start,
                        },
                        SessionState {
                            session_id: Some(id),
                            persisted: true,
                            fingerprints: fps,
                        },
                    ),
                    // No id captured or assistant-only suffix: degrade safely.
                    _ => (SpawnPlan::FreshEphemeral, fresh(false)),
                }
            }
            // First call of a family, or history was rewritten: start over.
            None => (SpawnPlan::FreshEphemeral, fresh(false)),
        }
    }
}

#[async_trait]
impl ModelClient for ClaudeCliClient {
    async fn stream(
        &self,
        req: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<Chunk, ModelError>>, ModelError> {
        let (plan, mut pending) = self.plan_spawn(&req.messages);

        let prompt = match &plan {
            SpawnPlan::Resume { suffix_start, .. } => {
                // The CLI session already holds its own assistant turns.
                let suffix: Vec<Message> = req.messages[*suffix_start..]
                    .iter()
                    .filter(|m| m.role != Role::Assistant)
                    .cloned()
                    .collect();
                render_transcript(&suffix)
            }
            _ => render_transcript(&req.messages),
        };

        let mut cmd = self.base_command();
        match &plan {
            SpawnPlan::FreshEphemeral => {
                cmd.arg("--no-session-persistence");
            }
            SpawnPlan::FreshPersisted => {}
            SpawnPlan::Resume { session_id, .. } => {
                cmd.arg("--resume").arg(session_id);
            }
        }

        let mut child = cmd
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

        let sessions = Arc::clone(&self.sessions);
        let track_state = self.opts.session_reuse;
        let stream = async_stream::stream! {
            let mut parser = EventParser::new();
            let mut lines = BufReader::new(stdout).lines();
            let mut failed = false;
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => match parser.parse_line(&line) {
                        Ok(chunks) => {
                            for c in chunks {
                                yield Ok(c);
                            }
                        }
                        Err(e) => {
                            failed = true;
                            // No state reset needed on failure: plan_spawn already
                            // checked the matched pool entry out, and we only
                            // re-insert on success.
                            yield Err(e);
                            break;
                        }
                    },
                    Ok(None) => break, // stdout EOF
                    Err(e) => {
                        failed = true;
                        yield Err(ModelError::Stream(e.to_string()));
                        break;
                    }
                }
            }

            if !failed {
                // stdout drained; confirm a clean exit, else surface stderr.
                match child.wait().await {
                    Ok(status) if status.success() => {
                        if track_state {
                            // Prefer the id the CLI just reported (a resume may
                            // continue under the same id or, with future CLIs, a
                            // forked one — the init event is authoritative).
                            pending.session_id =
                                parser.session_id.take().or(pending.session_id.take());
                            let mut pool = sessions.lock().expect("session pool lock");
                            pool.push(pending);
                            if pool.len() > MAX_POOLED_SESSIONS {
                                pool.remove(0); // evict oldest
                            }
                        }
                        return;
                    }
                    Ok(status) => {
                        let buf = stderr_task.await.unwrap_or_default();
                        yield Err(ModelError::Process(
                            format!("claude exited ({status}): {}", buf.trim())));
                    }
                    Err(e) => {
                        yield Err(ModelError::Process(e.to_string()));
                    }
                }
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
        // claude_cli is a bare text generator with no separate reasoning channel,
        // so preserved chain-of-thought rides inline as a <think> block ahead of
        // the answer (mirrors how the CLI emits and re-consumes its own thinking).
        if let Some(reasoning) = &m.reasoning {
            out.push_str("<think>");
            out.push_str(reasoning);
            out.push_str("</think>\n");
        }
        out.push_str(&m.content);
        out.push_str("\n\n");
    }
    out
}

/// Stateful stream-json line parser: one instance per CLI spawn. Tracks the
/// init event's session_id (for resume) and whether stream_event deltas were
/// seen (whole assistant messages then duplicate the deltas and are skipped).
pub(crate) struct EventParser {
    pub(crate) session_id: Option<String>,
    saw_stream_deltas: bool,
}

impl EventParser {
    pub(crate) fn new() -> Self {
        Self {
            session_id: None,
            saw_stream_deltas: false,
        }
    }

    pub(crate) fn parse_line(&mut self, line: &str) -> Result<Vec<Chunk>, ModelError> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(vec![]);
        }
        let v: Value = serde_json::from_str(line).map_err(|e| ModelError::Decode(e.to_string()))?;
        let mut out = Vec::new();
        match v["type"].as_str() {
            Some("system") => {
                if v["subtype"] == "init" {
                    if let Some(id) = v["session_id"].as_str() {
                        self.session_id = Some(id.to_string());
                    }
                }
            }
            Some("stream_event") => {
                let ev = &v["event"];
                if ev["type"] == "content_block_delta" {
                    match ev["delta"]["type"].as_str() {
                        Some("text_delta") => {
                            if let Some(t) = ev["delta"]["text"].as_str() {
                                if !t.is_empty() {
                                    self.saw_stream_deltas = true;
                                    out.push(Chunk::Text(t.to_string()));
                                }
                            }
                        }
                        Some("thinking_delta") => {
                            if let Some(t) = ev["delta"]["thinking"].as_str() {
                                if !t.is_empty() {
                                    self.saw_stream_deltas = true;
                                    out.push(Chunk::Reasoning(t.to_string()));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("assistant") => {
                // With --include-partial-messages the whole message repeats what
                // the deltas already streamed — emit only if no deltas were seen
                // (back-compat with a CLI that ignores the flag).
                if !self.saw_stream_deltas {
                    if let Some(blocks) = v["message"]["content"].as_array() {
                        for b in blocks {
                            match b["type"].as_str() {
                                Some("text") => {
                                    if let Some(t) = b["text"].as_str() {
                                        if !t.is_empty() {
                                            out.push(Chunk::Text(t.to_string()));
                                        }
                                    }
                                }
                                Some("thinking") => {
                                    if let Some(t) = b["thinking"].as_str() {
                                        if !t.is_empty() {
                                            out.push(Chunk::Reasoning(t.to_string()));
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            Some("result") => {
                if let Some(u) = v.get("usage").and_then(Value::as_object) {
                    let field = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
                    let cache_read = field("cache_read_input_tokens");
                    // Fold cache tokens into prompt_tokens so it reflects the
                    // effective context size; cached_tokens still surfaces the
                    // cache-read portion separately.
                    out.push(Chunk::Usage {
                        prompt_tokens: (field("input_tokens")
                            + cache_read
                            + field("cache_creation_input_tokens"))
                            as u32,
                        completion_tokens: field("output_tokens") as u32,
                        reasoning_tokens: None,
                        cached_tokens: if cache_read > 0 {
                            Some(cache_read as u32)
                        } else {
                            None
                        },
                        cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
                    });
                }
                let truncated = v["subtype"].as_str() == Some("error_max_turns")
                    || v["stop_reason"].as_str() == Some("max_tokens");
                out.push(Chunk::Done(if truncated {
                    StopReason::Length
                } else {
                    StopReason::Stop
                }));
            }
            _ => {} // user echoes etc. — nothing to emit.
        }
        Ok(out)
    }
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
        let mut f = tempfile::Builder::new()
            .prefix("fake-claude-")
            .tempfile()
            .unwrap();
        write!(f, "{script}").unwrap();
        f.flush().unwrap();
        f.as_file().sync_all().unwrap(); // settle the executable before exec() to avoid ETXTBSY under parallel test runs
        let path = f.into_temp_path();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    /// Fake CLI that records argv/stdin per call into `dir` and emits a canned
    /// stream with session id "sess-<n>". `fail_call` (0 = never) exits 1 on that call.
    fn write_recording_fake(dir: &std::path::Path, fail_call: u32) -> tempfile::TempPath {
        let d = dir.display();
        let script = format!(
            "#!/usr/bin/env bash\n\
             n=$(cat {d}/count 2>/dev/null || echo 0); n=$((n+1)); echo $n > {d}/count\n\
             printf '%s\\n' \"$*\" > {d}/argv.$n\n\
             cat > {d}/stdin.$n\n\
             if [ \"$n\" -eq \"{fail_call}\" ]; then echo boom >&2; exit 1; fi\n\
             echo '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"sess-'$n'\"}}'\n\
             echo '{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"ok'$n'\"}}]}},\"session_id\":\"sess-'$n'\"}}'\n\
             echo '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}'\n"
        );
        write_fake(&script)
    }

    async fn drain(client: &ClaudeCliClient, messages: Vec<Message>) -> Result<(), ModelError> {
        let mut stream = client
            .stream(CompletionRequest {
                messages,
                ..Default::default()
            })
            .await?;
        while let Some(item) = stream.next().await {
            item?;
        }
        Ok(())
    }

    fn read(dir: &std::path::Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap_or_default()
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
                Chunk::Reasoning(_) => {}
                Chunk::Usage { .. } => {}
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
            Some(ModelError::Process(msg)) => {
                assert!(msg.contains("not authenticated"), "got: {msg}")
            }
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
        // Fails unless every load-bearing bare-generator flag is present.
        let script = "#!/usr/bin/env bash\ncat >/dev/null\n\
            for f in --system-prompt --setting-sources --strict-mcp-config --no-session-persistence --allowedTools --include-partial-messages; do\n\
              case \" $* \" in *\" $f \"*) ;; *) echo \"missing $f\" >&2; exit 3;; esac\n\
            done\n\
            echo '{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]},\"session_id\":\"t\"}'\n\
            echo '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"session_id\":\"t\"}'\n";
        let fake = write_fake(script);
        let client = ClaudeCliClient::new(fake.to_str().unwrap(), "sonnet");
        let mut stream = client.stream(req()).await.unwrap();
        let mut text = String::new();
        while let Some(item) = stream.next().await {
            if let Chunk::Text(t) = item.unwrap() {
                text.push_str(&t);
            }
        }
        assert_eq!(text, "ok");
    }

    #[tokio::test]
    #[serial]
    async fn forwards_effort_and_fallback_model_flags() {
        let script = "#!/usr/bin/env bash\ncat >/dev/null\n\
            case \" $* \" in *\" --effort high \"*) ;; *) echo 'missing --effort' >&2; exit 3;; esac\n\
            case \" $* \" in *\" --fallback-model sonnet \"*) ;; *) echo 'missing --fallback-model' >&2; exit 3;; esac\n\
            echo '{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]},\"session_id\":\"t\"}'\n\
            echo '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"session_id\":\"t\"}'\n";
        let fake = write_fake(script);
        let client = ClaudeCliClient::with_options(
            fake.to_str().unwrap(),
            "opus",
            ClaudeCliOptions {
                effort: Some("high".into()),
                fallback_model: Some("sonnet".into()),
                ..Default::default()
            },
        );
        let mut stream = client.stream(req()).await.unwrap();
        let mut text = String::new();
        while let Some(item) = stream.next().await {
            if let Chunk::Text(t) = item.unwrap() {
                text.push_str(&t);
            }
        }
        assert_eq!(text, "ok");
    }

    #[tokio::test]
    #[serial]
    async fn default_options_omit_knob_flags() {
        let script = "#!/usr/bin/env bash\ncat >/dev/null\n\
            case \" $* \" in *\" --effort \"*|*\" --fallback-model \"*) echo 'unexpected knob flag' >&2; exit 3;; esac\n\
            echo '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false}'\n";
        let fake = write_fake(script);
        let client = ClaudeCliClient::new(fake.to_str().unwrap(), "sonnet");
        let mut stream = client.stream(req()).await.unwrap();
        let mut done = None;
        while let Some(item) = stream.next().await {
            if let Chunk::Done(r) = item.unwrap() {
                done = Some(r);
            }
        }
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
                if let Err(e) = item {
                    err = Some(e);
                }
            }
            err
        };
        let err = tokio::time::timeout(std::time::Duration::from_secs(10), collect)
            .await
            .expect("stream must not deadlock on large stderr");
        assert!(
            matches!(err, Some(ModelError::Process(_))),
            "expected Process error, got {err:?}"
        );
    }

    // ── Task 5: session state machine proc tests ──────────────────────────────

    #[tokio::test]
    #[serial]
    async fn session_reuse_walks_ephemeral_persisted_resume() {
        let dir = tempfile::tempdir().unwrap();
        let fake = write_recording_fake(dir.path(), 0);
        let client = ClaudeCliClient::with_options(
            fake.to_str().unwrap(),
            "sonnet",
            ClaudeCliOptions {
                session_reuse: true,
                ..Default::default()
            },
        );

        let base = vec![Message::system("sys"), Message::user("u1")];
        drain(&client, base.clone()).await.unwrap();

        let mut ext1 = base.clone();
        ext1.push(Message::assistant("ok1", None));
        ext1.push(Message::tool("call_0", "read_file", "t1"));
        drain(&client, ext1.clone()).await.unwrap();

        let mut ext2 = ext1.clone();
        ext2.push(Message::assistant("ok2", None));
        ext2.push(Message::user("u2"));
        drain(&client, ext2).await.unwrap();

        // Call 1: ephemeral full send.
        let argv1 = read(dir.path(), "argv.1");
        assert!(argv1.contains("--no-session-persistence"), "argv1: {argv1}");
        assert!(read(dir.path(), "stdin.1").contains("u1"));

        // Call 2: first extension → persisted full send (no resume yet).
        let argv2 = read(dir.path(), "argv.2");
        assert!(
            !argv2.contains("--no-session-persistence"),
            "argv2: {argv2}"
        );
        assert!(!argv2.contains("--resume"), "argv2: {argv2}");
        let stdin2 = read(dir.path(), "stdin.2");
        assert!(
            stdin2.contains("u1") && stdin2.contains("t1"),
            "stdin2: {stdin2}"
        );

        // Call 3: resume with suffix only; assistant turns skipped.
        let argv3 = read(dir.path(), "argv.3");
        assert!(argv3.contains("--resume sess-2"), "argv3: {argv3}");
        let stdin3 = read(dir.path(), "stdin.3");
        assert!(stdin3.contains("u2"), "stdin3: {stdin3}");
        assert!(!stdin3.contains("u1"), "stdin3 resent prefix: {stdin3}");
        assert!(!stdin3.contains("t1"), "stdin3 resent prefix: {stdin3}");
        assert!(!stdin3.contains("ok2"), "stdin3 resent assistant: {stdin3}");
    }

    #[tokio::test]
    #[serial]
    async fn history_rewrite_resets_to_ephemeral() {
        let dir = tempfile::tempdir().unwrap();
        let fake = write_recording_fake(dir.path(), 0);
        let client = ClaudeCliClient::with_options(
            fake.to_str().unwrap(),
            "sonnet",
            ClaudeCliOptions {
                session_reuse: true,
                ..Default::default()
            },
        );
        drain(&client, vec![Message::system("sys"), Message::user("u1")])
            .await
            .unwrap();
        // Not an extension: same length, different content (curation rewrote history).
        drain(
            &client,
            vec![Message::system("sys"), Message::user("rewritten")],
        )
        .await
        .unwrap();
        let argv2 = read(dir.path(), "argv.2");
        assert!(argv2.contains("--no-session-persistence"), "argv2: {argv2}");
        assert!(read(dir.path(), "stdin.2").contains("rewritten"));
    }

    #[tokio::test]
    #[serial]
    async fn stream_error_resets_session_state() {
        let dir = tempfile::tempdir().unwrap();
        let fake = write_recording_fake(dir.path(), 2); // call 2 fails
        let client = ClaudeCliClient::with_options(
            fake.to_str().unwrap(),
            "sonnet",
            ClaudeCliOptions {
                session_reuse: true,
                ..Default::default()
            },
        );
        let base = vec![Message::system("sys"), Message::user("u1")];
        drain(&client, base.clone()).await.unwrap();
        let mut ext = base.clone();
        ext.push(Message::assistant("ok1", None));
        ext.push(Message::user("u2"));
        assert!(drain(&client, ext.clone()).await.is_err()); // call 2: persisted attempt fails
                                                             // Retry (as the loop would): state was reset → ephemeral full send again.
        drain(&client, ext).await.unwrap();
        let argv3 = read(dir.path(), "argv.3");
        assert!(argv3.contains("--no-session-persistence"), "argv3: {argv3}");
        assert!(
            read(dir.path(), "stdin.3").contains("u1"),
            "full resend expected"
        );
    }

    #[tokio::test]
    #[serial]
    async fn reuse_off_is_always_ephemeral_full_send() {
        let dir = tempfile::tempdir().unwrap();
        let fake = write_recording_fake(dir.path(), 0);
        let client = ClaudeCliClient::new(fake.to_str().unwrap(), "sonnet"); // Default: reuse off
        let base = vec![Message::system("sys"), Message::user("u1")];
        drain(&client, base.clone()).await.unwrap();
        let mut ext = base.clone();
        ext.push(Message::assistant("ok1", None));
        ext.push(Message::user("u2"));
        drain(&client, ext).await.unwrap();
        let argv2 = read(dir.path(), "argv.2");
        assert!(argv2.contains("--no-session-persistence"), "argv2: {argv2}");
        assert!(
            read(dir.path(), "stdin.2").contains("u1"),
            "full resend expected"
        );
    }

    // ── Task 1: session pool with checkout semantics ──────────────────────────

    #[tokio::test]
    #[serial]
    async fn interleaved_transcript_families_each_reach_resume() {
        // The sibling-subagent pattern: two independent transcript families
        // through ONE client. The old single state slot made them clobber each
        // other (every call re-planned fresh); the pool must give each family
        // its own resume track.
        let dir = tempfile::tempdir().unwrap();
        let fake = write_recording_fake(dir.path(), 0);
        let client = ClaudeCliClient::with_options(
            fake.to_str().unwrap(),
            "sonnet",
            ClaudeCliOptions {
                session_reuse: true,
                ..Default::default()
            },
        );

        let a1 = vec![Message::system("sys"), Message::user("task-a")];
        let b1 = vec![Message::system("sys"), Message::user("task-b")];
        drain(&client, a1.clone()).await.unwrap(); // call 1: A fresh ephemeral
        drain(&client, b1.clone()).await.unwrap(); // call 2: B fresh ephemeral

        let mut a2 = a1.clone();
        a2.push(Message::assistant("ok1", None));
        a2.push(Message::user("a-next"));
        let mut b2 = b1.clone();
        b2.push(Message::assistant("ok2", None));
        b2.push(Message::user("b-next"));
        drain(&client, a2.clone()).await.unwrap(); // call 3: A first extension -> persisted (sess-3)
        drain(&client, b2.clone()).await.unwrap(); // call 4: B first extension -> persisted (sess-4)

        let mut a3 = a2.clone();
        a3.push(Message::assistant("ok3", None));
        a3.push(Message::user("a-more"));
        let mut b3 = b2.clone();
        b3.push(Message::assistant("ok4", None));
        b3.push(Message::user("b-more"));
        drain(&client, a3).await.unwrap(); // call 5: A resumes sess-3
        drain(&client, b3).await.unwrap(); // call 6: B resumes sess-4

        let argv5 = read(dir.path(), "argv.5");
        assert!(argv5.contains("--resume sess-3"), "argv5: {argv5}");
        let stdin5 = read(dir.path(), "stdin.5");
        assert!(stdin5.contains("a-more"), "stdin5: {stdin5}");
        assert!(!stdin5.contains("task-a"), "stdin5 resent prefix: {stdin5}");

        let argv6 = read(dir.path(), "argv.6");
        assert!(argv6.contains("--resume sess-4"), "argv6: {argv6}");
        let stdin6 = read(dir.path(), "stdin.6");
        assert!(stdin6.contains("b-more"), "stdin6: {stdin6}");
        assert!(!stdin6.contains("task-b"), "stdin6 resent prefix: {stdin6}");
    }

    #[tokio::test]
    #[serial]
    async fn session_pool_is_bounded_and_evicts_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let fake = write_recording_fake(dir.path(), 0);
        let client = ClaudeCliClient::with_options(
            fake.to_str().unwrap(),
            "sonnet",
            ClaudeCliOptions {
                session_reuse: true,
                ..Default::default()
            },
        );
        // MAX + 2 unrelated one-shot transcripts each commit one pool entry.
        let first = vec![Message::system("sys"), Message::user("task-0")];
        let first_fps: Vec<u64> = first.iter().map(fingerprint).collect();
        for i in 0..(MAX_POOLED_SESSIONS + 2) {
            let msgs = vec![Message::system("sys"), Message::user(format!("task-{i}"))];
            drain(&client, msgs).await.unwrap();
        }
        let pool = client.sessions.lock().unwrap();
        assert_eq!(pool.len(), MAX_POOLED_SESSIONS);
        // The oldest entry (task-0) was evicted.
        assert!(
            !pool.iter().any(|s| s.fingerprints == first_fps),
            "oldest entry should have been evicted"
        );
    }

    #[test]
    fn checkout_prevents_concurrent_resume_of_same_session() {
        // Two same-prefix planners racing: the first checks the entry out, the
        // second must find nothing and degrade to a fresh send — never a second
        // Resume against the same session id.
        let client = ClaudeCliClient::with_options(
            "claude",
            "sonnet",
            ClaudeCliOptions {
                session_reuse: true,
                ..Default::default()
            },
        );
        let base = vec![Message::system("sys"), Message::user("u1")];
        let base_fps: Vec<u64> = base.iter().map(fingerprint).collect();
        client.sessions.lock().unwrap().push(SessionState {
            session_id: Some("sess-x".into()),
            persisted: true,
            fingerprints: base_fps,
        });
        let mut ext = base.clone();
        ext.push(Message::assistant("ok", None));
        ext.push(Message::user("u2"));

        let (plan1, _) = client.plan_spawn(&ext);
        assert!(matches!(plan1, SpawnPlan::Resume { .. }));
        let (plan2, _) = client.plan_spawn(&ext);
        assert!(matches!(plan2, SpawnPlan::FreshEphemeral));
    }

    #[test]
    fn checkout_picks_longest_matching_prefix() {
        let client = ClaudeCliClient::with_options(
            "claude",
            "sonnet",
            ClaudeCliOptions {
                session_reuse: true,
                ..Default::default()
            },
        );
        let base = vec![Message::system("sys"), Message::user("u1")];
        let mut longer = base.clone();
        longer.push(Message::assistant("ok", None));
        let base_fps: Vec<u64> = base.iter().map(fingerprint).collect();
        let longer_fps: Vec<u64> = longer.iter().map(fingerprint).collect();
        {
            let mut pool = client.sessions.lock().unwrap();
            pool.push(SessionState {
                session_id: Some("sess-short".into()),
                persisted: true,
                fingerprints: base_fps,
            });
            pool.push(SessionState {
                session_id: Some("sess-long".into()),
                persisted: true,
                fingerprints: longer_fps,
            });
        }
        let mut ext = longer.clone();
        ext.push(Message::user("u2"));
        let (plan, _) = client.plan_spawn(&ext);
        match plan {
            SpawnPlan::Resume {
                session_id,
                suffix_start,
            } => {
                assert_eq!(session_id, "sess-long");
                assert_eq!(suffix_start, 3);
            }
            _ => panic!("expected Resume from the longest matching entry"),
        }
        // The shorter entry is still in the pool (only the match is checked out).
        assert_eq!(client.sessions.lock().unwrap().len(), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Message;

    // Fixture lines: verbatim captures from
    // docs/okf/claude-cli-headless/sources/probe-stream-json-2-1-195.md (claude 2.1.195).
    const INIT_LINE: &str = r#"{"type":"system","subtype":"init","cwd":"/home/kalen/rust-agent-runtime","session_id":"c33cee68-7fa0-4926-b32e-c1ad457673ea","tools":["Task","Bash","CronCreate","CronDelete","CronList","DesignSync","Edit","EnterWorktree","ExitWorktree","Monitor","NotebookEdit","PushNotification","Read","RemoteTrigger","ScheduleWakeup","SendMessage","Skill","TaskCreate","TaskGet","TaskList","TaskOutput","TaskStop","TaskUpdate","ToolSearch","WebFetch","WebSearch","Workflow","Write"],"mcp_servers":[],"model":"claude-sonnet-4-6","permissionMode":"default","slash_commands":["deep-research","design-sync","update-config","verify","debug","code-review","simplify","batch","fewer-permission-prompts","loop","schedule","claude-api","run","run-skill-generator","clear","compact","config","context","heapdump","init","reload-skills","review","security-review","usage-credits","extra-usage","usage","insights","goal","team-onboarding"],"apiKeySource":"none","claude_code_version":"2.1.195","output_style":"default","agents":["claude","Explore","general-purpose","Plan","statusline-setup"],"skills":["deep-research","design-sync","update-config","verify","debug","code-review","simplify","batch","fewer-permission-prompts","loop","schedule","claude-api","run","run-skill-generator"],"plugins":[],"analytics_disabled":false,"product_feedback_disabled":false,"uuid":"9f605e3a-53d6-431f-a75b-27e396f377be","memory_paths":{"auto":"/home/kalen/.claude/projects/-home-kalen-rust-agent-runtime/memory/"},"fast_mode_state":"off"}"#;
    const TEXT_DELTA_LINE: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}},"session_id":"c33cee68-7fa0-4926-b32e-c1ad457673ea","parent_tool_use_id":null,"uuid":"f7612ac4-3352-4d69-bdd4-deccecc0ee2b"}"#;
    // Shape from Anthropic SSE documentation — no thinking was emitted by claude 2.1.195
    // during the probe run (output_tokens_details.thinking_tokens was 0; see probe file).
    const THINKING_DELTA_LINE: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}},"session_id":"sess-abc"}"#;
    const ASSISTANT_LINE: &str = r#"{"type":"assistant","message":{"model":"claude-sonnet-4-6","id":"msg_011CcoMohKeVVbHLgCVVjcDy","type":"message","role":"assistant","content":[{"type":"text","text":"hello"}],"stop_reason":null,"stop_sequence":null,"stop_details":null,"usage":{"input_tokens":2,"cache_creation_input_tokens":20333,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":20333},"output_tokens":1,"service_tier":"standard","inference_geo":"not_available"},"diagnostics":null,"context_management":null},"parent_tool_use_id":null,"session_id":"c33cee68-7fa0-4926-b32e-c1ad457673ea","uuid":"4a79959d-3a36-4656-8d9c-01dd3645f578","request_id":"req_011CcoMogMrNdDwSc8fZCS1c"}"#;
    // Shape from Anthropic SSE documentation — no thinking was emitted by claude 2.1.195
    // during the probe run (see probe file).
    const ASSISTANT_THINKING_LINE: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"a plan"},{"type":"text","text":"hello world"}]},"session_id":"sess-abc"}"#;
    const RESULT_LINE: &str = r#"{"type":"result","subtype":"success","is_error":false,"api_error_status":null,"duration_ms":2118,"duration_api_ms":3091,"ttft_ms":2085,"ttft_stream_ms":2000,"time_to_request_ms":22,"num_turns":1,"result":"hello","stop_reason":"end_turn","session_id":"c33cee68-7fa0-4926-b32e-c1ad457673ea","total_cost_usd":0.122623,"usage":{"input_tokens":2,"cache_creation_input_tokens":20333,"cache_read_input_tokens":0,"output_tokens":4,"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":20333,"ephemeral_5m_input_tokens":0},"inference_geo":"not_available","iterations":[{"input_tokens":2,"output_tokens":4,"cache_read_input_tokens":0,"cache_creation_input_tokens":20333,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":20333},"type":"message"}],"speed":"standard"},"modelUsage":{"claude-haiku-4-5-20251001":{"inputTokens":504,"outputTokens":11,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"webSearchRequests":0,"costUSD":0.000559,"contextWindow":200000,"maxOutputTokens":32000},"claude-sonnet-4-6":{"inputTokens":2,"outputTokens":4,"cacheReadInputTokens":0,"cacheCreationInputTokens":20333,"webSearchRequests":0,"costUSD":0.12206399999999999,"contextWindow":200000,"maxOutputTokens":32000}},"permission_denials":[],"terminal_reason":"completed","fast_mode_state":"off","uuid":"cf354c70-9f45-4d4c-9192-c87b5149b7bd"}"#;

    #[test]
    fn init_line_captures_session_id_and_emits_nothing() {
        let mut p = EventParser::new();
        assert!(p.parse_line(INIT_LINE).unwrap().is_empty());
        assert_eq!(
            p.session_id.as_deref(),
            Some("c33cee68-7fa0-4926-b32e-c1ad457673ea")
        );
    }

    #[test]
    fn text_delta_emits_text_chunk() {
        let mut p = EventParser::new();
        let chunks = p.parse_line(TEXT_DELTA_LINE).unwrap();
        assert!(matches!(chunks.as_slice(), [Chunk::Text(t)] if t == "hello"));
    }

    #[test]
    fn thinking_delta_emits_reasoning_chunk() {
        let mut p = EventParser::new();
        let chunks = p.parse_line(THINKING_DELTA_LINE).unwrap();
        assert!(matches!(chunks.as_slice(), [Chunk::Reasoning(t)] if t == "hmm"));
    }

    #[test]
    fn whole_assistant_message_is_skipped_after_deltas() {
        let mut p = EventParser::new();
        p.parse_line(TEXT_DELTA_LINE).unwrap();
        assert!(p.parse_line(ASSISTANT_LINE).unwrap().is_empty());
    }

    #[test]
    fn whole_assistant_message_emits_when_no_deltas_seen() {
        // Back-compat: a CLI that ignores --include-partial-messages still streams.
        let mut p = EventParser::new();
        let chunks = p.parse_line(ASSISTANT_LINE).unwrap();
        assert!(matches!(chunks.as_slice(), [Chunk::Text(t)] if t == "hello"));
    }

    #[test]
    fn whole_assistant_thinking_block_emits_reasoning_when_no_deltas() {
        let mut p = EventParser::new();
        let chunks = p.parse_line(ASSISTANT_THINKING_LINE).unwrap();
        assert!(matches!(&chunks[0], Chunk::Reasoning(t) if t == "a plan"));
        assert!(matches!(&chunks[1], Chunk::Text(t) if t == "hello world"));
    }

    #[test]
    fn result_event_emits_done_stop() {
        // The real RESULT_LINE carries a usage object, so a Chunk::Usage precedes
        // the Done. Assert only the last chunk.
        let mut p = EventParser::new();
        let chunks = p.parse_line(RESULT_LINE).unwrap();
        assert!(matches!(chunks.last(), Some(Chunk::Done(StopReason::Stop))));
    }

    #[test]
    fn result_event_carries_usage_and_cost() {
        let line = r#"{"type":"result","subtype":"success","total_cost_usd":0.0421,"usage":{"input_tokens":1200,"output_tokens":345}}"#;
        let chunks = EventParser::new().parse_line(line).unwrap();
        assert!(chunks.iter().any(|c| matches!(c,
            Chunk::Usage { prompt_tokens: 1200, completion_tokens: 345,
                           cost_usd: Some(c), .. } if (*c - 0.0421).abs() < 1e-9)));
        assert!(matches!(chunks.last(), Some(Chunk::Done(StopReason::Stop))));
    }

    #[test]
    fn result_event_folds_cache_tokens_into_prompt() {
        let line = r#"{"type":"result","subtype":"success","usage":{"input_tokens":1000,"cache_read_input_tokens":4000,"cache_creation_input_tokens":500,"output_tokens":42}}"#;
        let chunks = EventParser::new().parse_line(line).unwrap();
        assert!(chunks.iter().any(|c| matches!(
            c,
            Chunk::Usage {
                prompt_tokens: 5500,
                completion_tokens: 42,
                cached_tokens: Some(4000),
                ..
            }
        )));
    }

    #[test]
    fn max_turns_result_maps_to_length() {
        let line = r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#;
        let chunks = EventParser::new().parse_line(line).unwrap();
        assert!(matches!(
            chunks.last(),
            Some(Chunk::Done(StopReason::Length))
        ));
    }

    #[test]
    fn blank_line_yields_nothing() {
        assert!(EventParser::new().parse_line("  ").unwrap().is_empty());
    }

    #[test]
    fn non_json_line_is_decode_error() {
        assert!(matches!(
            EventParser::new().parse_line("not json"),
            Err(ModelError::Decode(_))
        ));
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
        assert!(
            p.contains("## Tool (read_file)\nfile contents here"),
            "got: {p}"
        );
    }

    #[test]
    fn assistant_message_rendered() {
        let msgs = vec![Message::assistant("on it", None)];
        let p = render_transcript(&msgs);
        assert!(p.contains("## Assistant\non it"));
    }

    #[test]
    fn preserved_reasoning_renders_as_think_block_before_content() {
        let msgs = vec![Message::assistant("final answer", None).with_reasoning("secret plan")];
        let p = render_transcript(&msgs);
        assert!(
            p.contains("## Assistant\n<think>secret plan</think>\nfinal answer"),
            "got: {p}"
        );
    }

    #[test]
    fn no_reasoning_renders_content_only() {
        let msgs = vec![Message::assistant("final answer", None)];
        let p = render_transcript(&msgs);
        assert!(!p.contains("<think>"));
        assert!(p.contains("## Assistant\nfinal answer"));
    }
}
