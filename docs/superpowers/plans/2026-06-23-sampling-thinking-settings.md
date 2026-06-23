# Sampling & Thinking Settings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add seven inference controls — `top_p`, `top_k`, `min_p`, `presence_penalty`, `repeat_penalty`, `enable_thinking`, `preserve_thinking` — to the agent's editable settings and plumb them end-to-end: config → request body → reasoning capture → loop/history → wire → CLI + web.

**Architecture:** The five sampling params are `Option`s carried `RuntimeConfig → LoopConfig → CompletionRequest`, serialized into the OpenAI body only when set. `enable_thinking` rides as `chat_template_kwargs.enable_thinking`. Reasoning is captured into a distinct channel (a `reasoning_content` SSE field plus a streaming `<think>` splitter), surfaced as a new `AgentEvent::Reasoning`/`WireEvent::Reasoning` for live display, and either re-wrapped into history (`preserve_thinking=true`) or dropped (default).

**Tech Stack:** Rust (tokio, serde, serde_json, async-trait, futures, wiremock); React + TypeScript + Vite + Vitest.

## Global Constraints

- Source cargo first every shell session: `source "$HOME/.cargo/env"`. Build/test from `agent/`.
- Gate every Rust task on: `cargo test --workspace` green AND `cargo clippy --all-targets -- -D warnings` clean.
- Gate every web task on: `cd web && npm test` green.
- Spec defaults (verbatim): `enable_thinking` default **true**; `preserve_thinking` default **false**; the five sampling params default **unset** (`None`, omitted from the request body).
- Validation bounds (checked only when `Some`): `top_p` ∈ `0.0..=1.0`; `min_p` ∈ `0.0..=1.0`; `presence_penalty` ∈ `-2.0..=2.0`; `repeat_penalty` > `0.0`; `top_k` any `u32`.
- The `claude-cli` backend ignores all seven knobs (no forwarding, no `reasoning_content`); `validate()` must NOT reject sampling values under `claude-cli`.
- This slice intentionally modifies core crates (`agent-model`, `agent-core`); keep changes additive (new optional fields / new enum variants).
- `PartialRuntimeConfig` uses the spec §3 simplification: the sampling-param mirrors are plain `Option<T>` (absent and explicit `null` both mean "keep base").
- Reference spec: `docs/superpowers/specs/2026-06-23-sampling-thinking-settings-design.md`.

---

## File Structure

- `agent/crates/agent-runtime-config/src/runtime_config.rs` — `RuntimeConfig` fields, validation, serde defaults, partial merge (Task 1).
- `agent/crates/agent-model/src/types.rs` — `CompletionRequest`/`AssistantTurn`/`Chunk`/`StopReason` field & variant additions (Tasks 2, 3).
- `agent/crates/agent-model/src/openai.rs` — `body()` serialization, `ThinkingSplitter`, `parse_sse_line`, stream loop (Tasks 2, 3).
- `agent/crates/agent-core/src/loop_.rs` — `LoopConfig` fields, request build, reasoning accumulation, history strip/preserve (Tasks 2, 3, 5).
- `agent/crates/agent-core/src/event.rs` — `AgentEvent::Reasoning` (Task 4).
- `agent/crates/agent-core/src/testkit.rs` — match arms, `Scripted::Reasoning` (Tasks 4, 5).
- `agent/crates/agent-server/src/{runtime.rs,wire.rs}` — `build_loop` wiring, `WireEvent::Reasoning`, `wire_event_from` (Tasks 2, 4).
- `agent/crates/agent-cli/src/{main.rs,render.rs}` — CLI flags, reasoning render (Tasks 4, 6).
- `web/src/{wire.ts,state.ts}` + `web/src/components/{SettingsPanel.tsx,MessageList.tsx,ReasoningMessage.tsx}` — TS types, reducer, UI (Tasks 7, 8).
- `agent/docs/RUNNING.md`, `docs/superpowers/context/follow-ups.md` — docs (Tasks 6, 9).

---

## Task 1: RuntimeConfig fields, validation & serde defaults

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs`
- Test: same file (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `RuntimeConfig` gains pub fields `top_p: Option<f32>`, `top_k: Option<u32>`, `min_p: Option<f32>`, `presence_penalty: Option<f32>`, `repeat_penalty: Option<f32>`, `enable_thinking: bool`, `preserve_thinking: bool`.

- [ ] **Step 1: Write failing validation + default tests**

Add these tests inside the existing `mod tests` in `runtime_config.rs`:

```rust
#[test]
fn from_launch_seeds_thinking_and_sampling_defaults() {
    let c = base();
    assert!(c.enable_thinking);
    assert!(!c.preserve_thinking);
    assert!(c.top_p.is_none() && c.top_k.is_none() && c.min_p.is_none());
    assert!(c.presence_penalty.is_none() && c.repeat_penalty.is_none());
}

#[test]
fn validate_enforces_sampling_bounds_only_when_set() {
    let mut c = base();
    c.top_p = Some(1.5);
    assert!(c.validate().is_err());
    let mut c = base();
    c.min_p = Some(-0.1);
    assert!(c.validate().is_err());
    let mut c = base();
    c.presence_penalty = Some(3.0);
    assert!(c.validate().is_err());
    let mut c = base();
    c.repeat_penalty = Some(0.0);
    assert!(c.validate().is_err());
    // None and in-range Some both pass; top_k has no bound.
    let mut c = base();
    c.top_p = Some(0.9);
    c.top_k = Some(40);
    c.repeat_penalty = Some(1.1);
    assert!(c.validate().is_ok());
}

#[test]
fn sampling_round_trips_and_partial_file_keeps_base() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rt.json");
    let mut c = base();
    c.top_k = Some(20);
    c.enable_thinking = false;
    c.preserve_thinking = true;
    c.save(&path).unwrap();
    let loaded = RuntimeConfig::load_over(base(), &path);
    assert_eq!(loaded.top_k, Some(20));
    assert!(!loaded.enable_thinking);
    assert!(loaded.preserve_thinking);

    // A file missing the new keys leaves the base values intact.
    std::fs::write(&path, r#"{"model":"only-model"}"#).unwrap();
    let loaded = RuntimeConfig::load_over(base(), &path);
    assert_eq!(loaded.model, "only-model");
    assert!(loaded.enable_thinking); // base default preserved
    assert!(loaded.top_k.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-runtime-config 2>&1 | tail -20`
Expected: compile errors — `RuntimeConfig` has no field `top_p`/`enable_thinking`/etc.

- [ ] **Step 3: Add the fields to `RuntimeConfig`**

In the `pub struct RuntimeConfig` definition, after `pub context_limit: usize,` add:

```rust
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub top_k: Option<u32>,
    #[serde(default)]
    pub min_p: Option<f32>,
    #[serde(default)]
    pub presence_penalty: Option<f32>,
    #[serde(default)]
    pub repeat_penalty: Option<f32>,
    #[serde(default = "default_true")]
    pub enable_thinking: bool,
    #[serde(default)]
    pub preserve_thinking: bool,
```

Add this helper just above the `impl RuntimeConfig` block:

```rust
fn default_true() -> bool { true }
```

- [ ] **Step 4: Seed defaults in `from_launch`**

In `from_launch`, inside the returned `Self { … }`, after `context_limit,` add:

```rust
            top_p: None,
            top_k: None,
            min_p: None,
            presence_penalty: None,
            repeat_penalty: None,
            enable_thinking: true,
            preserve_thinking: false,
```

- [ ] **Step 5: Add bounds to `validate()`**

In `validate()`, just before `Ok(())`, add:

```rust
        if let Some(v) = self.top_p { if !(0.0..=1.0).contains(&v) {
            return Err("top_p must be between 0.0 and 1.0".into()); } }
        if let Some(v) = self.min_p { if !(0.0..=1.0).contains(&v) {
            return Err("min_p must be between 0.0 and 1.0".into()); } }
        if let Some(v) = self.presence_penalty { if !(-2.0..=2.0).contains(&v) {
            return Err("presence_penalty must be between -2.0 and 2.0".into()); } }
        if let Some(v) = self.repeat_penalty { if v <= 0.0 {
            return Err("repeat_penalty must be > 0.0".into()); } }
```

- [ ] **Step 6: Extend `PartialRuntimeConfig` and `merge`**

In `struct PartialRuntimeConfig`, after `context_limit: Option<usize>,` add:

```rust
    top_p: Option<f32>,
    top_k: Option<u32>,
    min_p: Option<f32>,
    presence_penalty: Option<f32>,
    repeat_penalty: Option<f32>,
    enable_thinking: Option<bool>,
    preserve_thinking: Option<bool>,
```

In `fn merge`, before `self` is returned, add:

```rust
        if let Some(v) = p.top_p { self.top_p = Some(v); }
        if let Some(v) = p.top_k { self.top_k = Some(v); }
        if let Some(v) = p.min_p { self.min_p = Some(v); }
        if let Some(v) = p.presence_penalty { self.presence_penalty = Some(v); }
        if let Some(v) = p.repeat_penalty { self.repeat_penalty = Some(v); }
        if let Some(v) = p.enable_thinking { self.enable_thinking = v; }
        if let Some(v) = p.preserve_thinking { self.preserve_thinking = v; }
```

- [ ] **Step 7: Run tests + clippy to verify green**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-runtime-config && cargo clippy -p agent-runtime-config --all-targets -- -D warnings`
Expected: all tests pass; clippy clean. (The existing `save_then_load_over_round_trips_all_fields` and partial-file tests still pass because the new fields have serde defaults.)

- [ ] **Step 8: Commit**

```bash
git add agent/crates/agent-runtime-config/src/runtime_config.rs
git commit -m "feat(config): add sampling + thinking fields to RuntimeConfig with validation"
```

---

## Task 2: Sampling params through the request body

**Files:**
- Modify: `agent/crates/agent-model/src/types.rs` (`CompletionRequest` + `Default`)
- Modify: `agent/crates/agent-model/src/openai.rs` (`body()` + fix test literals)
- Modify: `agent/crates/agent-model/src/{prompted.rs,protocol.rs,claude_cli.rs}` (fix test literals)
- Modify: `agent/crates/agent-core/src/loop_.rs` (`LoopConfig` + request build + fix test literals)
- Modify: `agent/crates/agent-server/src/runtime.rs` (`build_loop` wiring)
- Modify: `agent/crates/agent-cli/src/main.rs` (`LoopConfig` literal)
- Test: `agent/crates/agent-model/src/openai.rs`

**Interfaces:**
- Consumes: `RuntimeConfig` sampling/thinking fields (Task 1).
- Produces: `CompletionRequest` gains `top_p: Option<f32>`, `top_k: Option<u32>`, `min_p: Option<f32>`, `presence_penalty: Option<f32>`, `repeat_penalty: Option<f32>`, `enable_thinking: bool`, and derives `Default`. `LoopConfig` gains the same five sampling `Option`s plus `enable_thinking: bool` and `preserve_thinking: bool`, and derives `Default`.

- [ ] **Step 1: Write failing body() test**

Add to `mod tests` in `openai.rs`:

```rust
#[test]
fn body_serializes_sampling_and_thinking() {
    let client = OpenAiCompatClient::new("http://x".into(), "m".into(), None);
    let req = CompletionRequest {
        messages: vec![Message::user("hi")],
        top_p: Some(0.8),
        top_k: Some(30),
        enable_thinking: false,
        ..Default::default()
    };
    let b = client.body(&req);
    assert_eq!(b["top_p"], serde_json::json!(0.8));
    assert_eq!(b["top_k"], serde_json::json!(30));
    assert_eq!(b["chat_template_kwargs"]["enable_thinking"], serde_json::json!(false));
    // Unset params are omitted entirely.
    assert!(b.get("min_p").is_none());
    assert!(b.get("presence_penalty").is_none());
    assert!(b.get("repeat_penalty").is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-model body_serializes 2>&1 | tail -20`
Expected: compile error — `CompletionRequest` has no field `top_p`; no `Default` impl.

- [ ] **Step 3: Add fields + Default to `CompletionRequest`**

In `types.rs`, change the derive and body of `CompletionRequest`:

```rust
#[derive(Debug, Clone, Default)]
pub struct CompletionRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSchema>,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub min_p: Option<f32>,
    pub presence_penalty: Option<f32>,
    pub repeat_penalty: Option<f32>,
    pub enable_thinking: bool,
}
```

- [ ] **Step 4: Serialize them in `body()`**

In `openai.rs::body`, change the initial `json!` to include `chat_template_kwargs`, and add the omit-when-`None` inserts after the `max_tokens` block:

```rust
        let mut b = json!({
            "model": self.model,
            "messages": messages_to_json(&req.messages),
            "stream": true,
            "temperature": req.temperature,
            "chat_template_kwargs": { "enable_thinking": req.enable_thinking },
        });
        if let Some(mt) = req.max_tokens {
            b["max_tokens"] = json!(mt);
        }
        if let Some(v) = req.top_p { b["top_p"] = json!(v); }
        if let Some(v) = req.top_k { b["top_k"] = json!(v); }
        if let Some(v) = req.min_p { b["min_p"] = json!(v); }
        if let Some(v) = req.presence_penalty { b["presence_penalty"] = json!(v); }
        if let Some(v) = req.repeat_penalty { b["repeat_penalty"] = json!(v); }
```

- [ ] **Step 5: Fix the existing `CompletionRequest` test literals**

These now-incomplete literals must spread defaults. Edit each:
- `openai.rs` ~line 193 (`streams_text_chunks_then_done`) and ~line 223 (`surfaces_http_error_status`): replace the trailing `temperature: 0.0,\n            max_tokens: None,\n        };` with `..Default::default()\n        };`.
- `prompted.rs` ~line 66 and `protocol.rs` ~line 68: replace the trailing `temperature: 0.0, max_tokens: None };` with `..Default::default() };`.
- `claude_cli.rs` ~line 202 (`fn req()`): replace the body with:

```rust
        CompletionRequest {
            messages: vec![Message::user("hi")],
            ..Default::default()
        }
```

- [ ] **Step 6: Add the fields + Default to `LoopConfig` and build the request from them**

In `loop_.rs`, change `LoopConfig`:

```rust
#[derive(Default)]
pub struct LoopConfig {
    pub model_limit: usize,
    pub max_turns: usize,
    pub max_retries: usize,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    pub workspace: PathBuf,
    pub tool_timeout: Duration,
    pub stream_idle_timeout: Duration,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub min_p: Option<f32>,
    pub presence_penalty: Option<f32>,
    pub repeat_penalty: Option<f32>,
    pub enable_thinking: bool,
    pub preserve_thinking: bool,
}
```

In `run()`, replace the `let base = CompletionRequest { … };` block with:

```rust
            let base = CompletionRequest {
                messages: ctx.build(self.config.model_limit),
                tools: self.tools.schemas(),
                temperature: self.config.temperature,
                max_tokens: self.config.max_tokens,
                top_p: self.config.top_p,
                top_k: self.config.top_k,
                min_p: self.config.min_p,
                presence_penalty: self.config.presence_penalty,
                repeat_penalty: self.config.repeat_penalty,
                enable_thinking: self.config.enable_thinking,
            };
```

- [ ] **Step 7: Fix the 8 `LoopConfig` test literals in `loop_.rs`**

Each test literal currently ends with `stream_idle_timeout: <expr> });`. For every one (lines ~234, 268, 290, 311, 330, 354, 378, 415), insert `..Default::default()` before the closing brace, e.g.:

```rust
                stream_idle_timeout: std::time::Duration::from_secs(60), ..Default::default() });
```
and for the four using the short form:
```rust
                stream_idle_timeout: Duration::from_secs(10), ..Default::default() });
```

- [ ] **Step 8: Wire `build_loop` (agent-server) from the config**

In `runtime.rs::build_loop`, inside the `LoopConfig { … }` literal, after `stream_idle_timeout: DEFAULT_STREAM_IDLE_TIMEOUT,` add:

```rust
            top_p: cfg.top_p,
            top_k: cfg.top_k,
            min_p: cfg.min_p,
            presence_penalty: cfg.presence_penalty,
            repeat_penalty: cfg.repeat_penalty,
            enable_thinking: cfg.enable_thinking,
            preserve_thinking: cfg.preserve_thinking,
```

- [ ] **Step 9: Fix the CLI `LoopConfig` literal (defaults for now; flags come in Task 6)**

In `agent-cli/src/main.rs`, inside the `LoopConfig { … }` literal, after `stream_idle_timeout: Duration::from_secs(cli.stream_timeout_secs),` add:

```rust
            top_p: None, top_k: None, min_p: None,
            presence_penalty: None, repeat_penalty: None,
            enable_thinking: true, preserve_thinking: false,
```

- [ ] **Step 10: Run workspace tests + clippy**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test --workspace && cargo clippy --all-targets -- -D warnings`
Expected: all green; the new `body_serializes_sampling_and_thinking` passes.

- [ ] **Step 11: Commit**

```bash
git add agent/crates/agent-model agent/crates/agent-core agent/crates/agent-server agent/crates/agent-cli
git commit -m "feat(model): plumb sampling params + enable_thinking into the request body"
```

---

## Task 3: Reasoning capture — ThinkingSplitter + reasoning_content

**Files:**
- Modify: `agent/crates/agent-model/src/types.rs` (`Chunk::Reasoning`, `AssistantTurn.reasoning`, `StopReason`/`AssistantTurn` `Default`)
- Modify: `agent/crates/agent-model/src/openai.rs` (`ThinkingSplitter`, `parse_sse_line`, stream loop)
- Modify: `agent/crates/agent-model/src/protocol.rs`, `prompted.rs` (fix `AssistantTurn` literals)
- Modify: `agent/crates/agent-core/src/loop_.rs` (`one_completion` accumulation)
- Test: `agent/crates/agent-model/src/openai.rs`

**Interfaces:**
- Produces: `Chunk::Reasoning(String)` variant; `AssistantTurn` gains `reasoning: String`; `ThinkingSplitter` with `fn push(&mut self, &str) -> Vec<Chunk>` and `fn flush(&mut self) -> Vec<Chunk>`. `parse_sse_line(line: &str, splitter: &mut ThinkingSplitter)`.

- [ ] **Step 1: Write failing ThinkingSplitter unit tests**

Add to `mod tests` in `openai.rs`:

```rust
fn collect(s: &mut ThinkingSplitter, parts: &[&str]) -> (String, String) {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut chunks: Vec<Chunk> = Vec::new();
    for p in parts { chunks.extend(s.push(p)); }
    chunks.extend(s.flush());
    for c in chunks {
        match c {
            Chunk::Text(t) => text.push_str(&t),
            Chunk::Reasoning(r) => reasoning.push_str(&r),
            _ => {}
        }
    }
    (text, reasoning)
}

#[test]
fn splitter_routes_think_block() {
    let mut s = ThinkingSplitter::default();
    let (text, reasoning) = collect(&mut s, &["<think>plan</think>answer"]);
    assert_eq!(reasoning, "plan");
    assert_eq!(text, "answer");
}

#[test]
fn splitter_handles_tag_split_across_chunks() {
    let mut s = ThinkingSplitter::default();
    let (text, reasoning) = collect(&mut s, &["<thi", "nk>deep", " thought</thi", "nk>done"]);
    assert_eq!(reasoning, "deep thought");
    assert_eq!(text, "done");
}

#[test]
fn splitter_passes_through_plain_text() {
    let mut s = ThinkingSplitter::default();
    let (text, reasoning) = collect(&mut s, &["hello ", "world"]);
    assert_eq!(text, "hello world");
    assert!(reasoning.is_empty());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-model splitter 2>&1 | tail -20`
Expected: compile error — no `ThinkingSplitter`, no `Chunk::Reasoning`.

- [ ] **Step 3: Add `Chunk::Reasoning` and `AssistantTurn.reasoning`**

In `types.rs`:
- Add `Reasoning(String)` to `enum Chunk`:

```rust
pub enum Chunk { Text(String), Reasoning(String), ToolCallDelta(RawToolCall), Done(StopReason) }
```
- Make `StopReason` default to `Stop`, and give `AssistantTurn` a `reasoning` field + `Default`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StopReason { #[default] Stop, ToolCalls, Length, BudgetExhausted }

#[derive(Debug, Clone, Default)]
pub struct AssistantTurn {
    pub text: String,
    pub raw_tool_calls: Vec<RawToolCall>,
    pub stop: StopReason,
    pub reasoning: String,
}
```

- [ ] **Step 4: Implement `ThinkingSplitter` in `openai.rs`**

Add near the top of `openai.rs` (after the imports):

```rust
/// Splits a streamed `content` channel into answer text and `<think>…</think>`
/// reasoning, buffering a partial tag that straddles a chunk boundary.
#[derive(Default)]
pub(crate) struct ThinkingSplitter {
    in_think: bool,
    buf: String,
}

impl ThinkingSplitter {
    fn emit(out: &mut Vec<Chunk>, in_think: bool, s: &str) {
        if s.is_empty() { return; }
        out.push(if in_think { Chunk::Reasoning(s.to_string()) } else { Chunk::Text(s.to_string()) });
    }

    pub(crate) fn push(&mut self, content: &str) -> Vec<Chunk> {
        let mut out = Vec::new();
        self.buf.push_str(content);
        loop {
            let tag: &str = if self.in_think { "</think>" } else { "<think>" };
            if let Some(idx) = self.buf.find(tag) {
                let before = self.buf[..idx].to_string();
                Self::emit(&mut out, self.in_think, &before);
                self.buf.drain(..idx + tag.len());
                self.in_think = !self.in_think;
                continue;
            }
            let keep = partial_prefix_len(tag, &self.buf);
            let flush_to = self.buf.len() - keep;
            let flush = self.buf[..flush_to].to_string();
            Self::emit(&mut out, self.in_think, &flush);
            self.buf.drain(..flush_to);
            break;
        }
        out
    }

    pub(crate) fn flush(&mut self) -> Vec<Chunk> {
        let mut out = Vec::new();
        let rest = std::mem::take(&mut self.buf);
        Self::emit(&mut out, self.in_think, &rest);
        out
    }
}

/// Length of the longest suffix of `buf` that is a proper prefix of `tag`.
fn partial_prefix_len(tag: &str, buf: &str) -> usize {
    let max = tag.len().saturating_sub(1).min(buf.len());
    for k in (1..=max).rev() {
        let start = buf.len() - k;
        if buf.is_char_boundary(start) && buf[start..] == tag[..k] {
            return k;
        }
    }
    0
}
```

- [ ] **Step 5: Run splitter tests to verify pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-model splitter 2>&1 | tail -20`
Expected: the three splitter tests pass. (Other crates may not build yet — that's covered next.)

- [ ] **Step 6: Route content + reasoning_content through `parse_sse_line`**

Change the signature and content handling in `parse_sse_line`:

```rust
fn parse_sse_line(line: &str, splitter: &mut ThinkingSplitter) -> Option<Result<Vec<Chunk>, ModelError>> {
```
After `let mut out = Vec::new();`, add reasoning capture before the existing content block:

```rust
    if let Some(reasoning) = choice["delta"]["reasoning_content"].as_str() {
        if !reasoning.is_empty() {
            out.push(Chunk::Reasoning(reasoning.to_string()));
        }
    }
```
Replace the existing content block:

```rust
    if let Some(content) = choice["delta"]["content"].as_str() {
        if !content.is_empty() {
            out.extend(splitter.push(content));
        }
    }
```

- [ ] **Step 7: Own the splitter in the stream loop and flush at the end**

In `OpenAiCompatClient::stream`, inside the `async_stream::stream! {` block, add `let mut splitter = ThinkingSplitter::default();` right after `let mut buf = String::new();`. Change the parse call to `parse_sse_line(&line, &mut splitter)`. In the `Some(Ok(chunks))` arm, when `is_done` is true, flush before returning:

```rust
                        Some(Ok(chunks)) => {
                            let is_done = line.contains("[DONE]");
                            for chunk in chunks {
                                yield Ok(chunk);
                            }
                            if is_done {
                                for chunk in splitter.flush() { yield Ok(chunk); }
                                return;
                            }
                            continue;
                        }
```
And in the byte-stream `None => return,` arm, flush first:

```rust
                    None => {
                        for chunk in splitter.flush() { yield Ok(chunk); }
                        return;
                    }
```

- [ ] **Step 8: Accumulate reasoning in `one_completion` (agent-core)**

In `loop_.rs::one_completion`, add `let mut reasoning = String::new();` next to `let mut text = String::new();`. In the chunk match, add an arm and extend the returned turn:

```rust
                    Chunk::Text(t) => { self.sink.emit(AgentEvent::Token(t.clone())); text.push_str(&t); }
                    Chunk::Reasoning(r) => { reasoning.push_str(&r); }
                    Chunk::ToolCallDelta(rc) => merge_tool_call(&mut raw_tool_calls, rc),
                    Chunk::Done(r) => stop = r,
```
Change the return to:

```rust
        Ok(AssistantTurn { text, raw_tool_calls, stop, reasoning })
```

- [ ] **Step 9: Fix `AssistantTurn` test literals + the openai stream-test match**

- `openai.rs` `streams_text_chunks_then_done`: add an arm to its `match item.unwrap()`:

```rust
                Chunk::Reasoning(_) => {}
```
- `protocol.rs` (~lines 43, 59) and `prompted.rs` (~lines 81, 93, 102): each `AssistantTurn { … }` literal sets `text`, `raw_tool_calls`, `stop`. Append `, reasoning: String::new()` before the closing brace of each (or `..Default::default()` if `stop` is the default `Stop`). Example for `protocol.rs:43`:

```rust
        let turn = AssistantTurn {
            text: "hi".into(),
            raw_tool_calls: vec![/* unchanged */],
            stop: StopReason::ToolCalls,
            reasoning: String::new(),
        };
```

- [ ] **Step 10: Run workspace tests + clippy**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test --workspace && cargo clippy --all-targets -- -D warnings`
Expected: all green. Reasoning is now captured into `AssistantTurn.reasoning`; not yet emitted/stripped.

- [ ] **Step 11: Commit**

```bash
git add agent/crates/agent-model agent/crates/agent-core
git commit -m "feat(model): capture reasoning via reasoning_content + streaming <think> splitter"
```

---

## Task 4: Reasoning event end-to-end (Rust)

**Files:**
- Modify: `agent/crates/agent-core/src/event.rs` (`AgentEvent::Reasoning`)
- Modify: `agent/crates/agent-core/src/loop_.rs` (emit in `one_completion`)
- Modify: `agent/crates/agent-core/src/testkit.rs` (`CollectingSink` arm)
- Modify: `agent/crates/agent-cli/src/render.rs` (dim render arm)
- Modify: `agent/crates/agent-server/src/wire.rs` (`WireEvent::Reasoning` + `wire_event_from`)
- Test: `agent/crates/agent-server/src/wire.rs`

**Interfaces:**
- Consumes: `AssistantTurn.reasoning`, `Chunk::Reasoning` (Task 3).
- Produces: `AgentEvent::Reasoning(String)`; `WireEvent::Reasoning { text: String }` (serde-tagged `"reasoning"`).

- [ ] **Step 1: Write failing wire-mapping test**

Add to `mod tests` in `wire.rs`:

```rust
#[test]
fn reasoning_event_maps_to_wire() {
    let payload = wire_event_from(AgentEvent::Reasoning("thinking".into())).unwrap();
    let json = serde_json::to_string(&payload).unwrap();
    assert!(json.contains("\"type\":\"reasoning\""));
    assert!(json.contains("thinking"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-server reasoning_event 2>&1 | tail -20`
Expected: compile error — no `AgentEvent::Reasoning`.

- [ ] **Step 3: Add the `AgentEvent` variant**

In `event.rs`, add to `enum AgentEvent` after `Token(String),`:

```rust
    Reasoning(String),
```

- [ ] **Step 4: Emit it in `one_completion`**

In `loop_.rs`, change the `Chunk::Reasoning` arm added in Task 3 to also emit:

```rust
                    Chunk::Reasoning(r) => { self.sink.emit(AgentEvent::Reasoning(r.clone())); reasoning.push_str(&r); }
```

- [ ] **Step 5: Add match arms in the exhaustive `AgentEvent` consumers**

- `testkit.rs` `CollectingSink::emit`, after the `Token` arm:

```rust
            AgentEvent::Reasoning(r) => format!("reasoning:{r}"),
```
- `render.rs` (`TerminalSink::emit`), after the `Token` arm — render reasoning dimmed:

```rust
            AgentEvent::Reasoning(r) => {
                let _ = write!(out, "\x1b[2m{r}\x1b[0m");
                let _ = out.flush();
            }
```

- [ ] **Step 6: Add `WireEvent::Reasoning` + map it**

In `wire.rs`, add to `enum WireEvent` after `Token { text: String },`:

```rust
    Reasoning { text: String },
```
In `wire_event_from`, add after the `Token` arm:

```rust
        AgentEvent::Reasoning(t) => WireEvent::Reasoning { text: t },
```

- [ ] **Step 7: Run workspace tests + clippy**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test --workspace && cargo clippy --all-targets -- -D warnings`
Expected: green, including `reasoning_event_maps_to_wire`.

- [ ] **Step 8: Commit**

```bash
git add agent/crates/agent-core agent/crates/agent-cli agent/crates/agent-server
git commit -m "feat(core): surface reasoning as AgentEvent/WireEvent::Reasoning + dim CLI render"
```

---

## Task 5: History strip / preserve in the loop

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` (`run()` assistant append)
- Modify: `agent/crates/agent-core/src/testkit.rs` (`Scripted::Reasoning`)
- Test: `agent/crates/agent-core/src/loop_.rs`

**Interfaces:**
- Consumes: `LoopConfig.preserve_thinking` (Task 2), `AssistantTurn.reasoning` (Task 3).
- Produces: `Scripted::Reasoning(reasoning, answer)` test variant.

- [ ] **Step 1: Add the `Scripted::Reasoning` test double**

In `testkit.rs`, add a variant to `enum Scripted`:

```rust
    /// Emits a reasoning chunk then a final answer (no tool calls): (reasoning, answer).
    Reasoning(String, String),
```
And handle it in `ScriptedModel::stream`'s match:

```rust
            Scripted::Reasoning(reasoning, answer) => Ok(stream::iter(vec![
                Ok(Chunk::Reasoning(reasoning)), Ok(Chunk::Text(answer)),
                Ok(Chunk::Done(StopReason::Stop))]).boxed()),
```

- [ ] **Step 2: Write failing strip/preserve tests**

Add to `mod tests` in `loop_.rs`:

```rust
async fn run_reasoning(preserve: bool) -> Vec<Message> {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();
    let model = Arc::new(ScriptedModel::new(vec![
        Scripted::Reasoning("secret plan".into(), "final answer".into()),
    ]));
    let sink = Arc::new(CollectingSink::default());
    let agent = AgentLoop::new(
        model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
        Arc::new(AlwaysApprove), sink,
        LoopConfig { model_limit: 100_000, max_turns: 5, max_retries: 1, temperature: 0.0,
            max_tokens: None, workspace: ws,
            tool_timeout: std::time::Duration::from_secs(5),
            stream_idle_timeout: std::time::Duration::from_secs(60),
            preserve_thinking: preserve, ..Default::default() });
    let mut ctx = WindowContext::new(Message::system("sys"));
    agent.run(&mut ctx, "go".into()).await.unwrap();
    ctx.build(100_000)
}

#[tokio::test]
async fn preserve_thinking_keeps_reasoning_in_history() {
    let msgs = run_reasoning(true).await;
    let a = msgs.iter().find(|m| matches!(m.role, agent_model::Role::Assistant)).unwrap();
    assert!(a.content.contains("<think>secret plan</think>"));
    assert!(a.content.contains("final answer"));
}

#[tokio::test]
async fn default_strips_reasoning_from_history() {
    let msgs = run_reasoning(false).await;
    let a = msgs.iter().find(|m| matches!(m.role, agent_model::Role::Assistant)).unwrap();
    assert!(!a.content.contains("secret plan"));
    assert_eq!(a.content, "final answer");
}
```

- [ ] **Step 3: Run to verify failure**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-core thinking_keeps 2>&1 | tail -20`
Expected: FAIL — assistant content is `"final answer"` in both cases (no preserve logic yet).

- [ ] **Step 4: Apply the strip/preserve logic in `run()`**

In `loop_.rs::run`, replace the assistant append (currently `ctx.append(Message::assistant(parsed.text.clone(), …));`) with:

```rust
            let stored = if self.config.preserve_thinking && !assistant.reasoning.is_empty() {
                format!("<think>{}</think>\n{}", assistant.reasoning, parsed.text)
            } else {
                parsed.text.clone()
            };
            ctx.append(Message::assistant(stored,
                if parsed.tool_calls.is_empty() { None } else { Some(parsed.tool_calls.clone()) }));
```

- [ ] **Step 5: Run tests + clippy**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test --workspace && cargo clippy --all-targets -- -D warnings`
Expected: both new tests pass; workspace green.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-core
git commit -m "feat(core): strip or preserve reasoning in history per preserve_thinking"
```

---

## Task 6: CLI flags

**Files:**
- Modify: `agent/crates/agent-cli/src/main.rs` (clap args + `LoopConfig` wiring)
- Modify: `agent/docs/RUNNING.md` (document the flags)

**Interfaces:**
- Consumes: `LoopConfig` sampling/thinking fields (Task 2).

- [ ] **Step 1: Add the clap flags to `struct Cli`**

In `main.rs`, after the `skill: Vec<String>,` field, add:

```rust
    /// Nucleus sampling (0.0–1.0); unset = server default
    #[arg(long)]
    top_p: Option<f32>,
    /// Top-k sampling; unset = server default
    #[arg(long)]
    top_k: Option<u32>,
    /// Min-p sampling (0.0–1.0); unset = server default
    #[arg(long)]
    min_p: Option<f32>,
    /// Presence penalty (-2.0–2.0); unset = server default
    #[arg(long)]
    presence_penalty: Option<f32>,
    /// Repetition penalty (>0.0); unset = server default
    #[arg(long)]
    repeat_penalty: Option<f32>,
    /// Disable model reasoning (chat_template_kwargs.enable_thinking=false)
    #[arg(long = "no-thinking", default_value_t = false)]
    no_thinking: bool,
    /// Keep prior <think> reasoning in conversation history
    #[arg(long, default_value_t = false)]
    preserve_thinking: bool,
```

- [ ] **Step 2: Wire the flags into the `LoopConfig` literal**

Replace the placeholder block added in Task 2 Step 9 with:

```rust
            top_p: cli.top_p, top_k: cli.top_k, min_p: cli.min_p,
            presence_penalty: cli.presence_penalty, repeat_penalty: cli.repeat_penalty,
            enable_thinking: !cli.no_thinking, preserve_thinking: cli.preserve_thinking,
```

- [ ] **Step 3: Verify it builds and the flags parse**

Run: `source "$HOME/.cargo/env" && cd agent && cargo build -p agent-cli && cargo run -p agent-cli -- --help 2>&1 | grep -E "top-p|no-thinking|preserve-thinking"`
Expected: build succeeds; `--top-p`, `--no-thinking`, `--preserve-thinking` appear in help.

- [ ] **Step 4: Document the flags in RUNNING.md**

In `agent/docs/RUNNING.md`, in the CLI flags section, add a short subsection:

```markdown
### Sampling & thinking flags

- `--top-p`, `--top-k`, `--min-p`, `--presence-penalty`, `--repeat-penalty` — optional
  sampler overrides; omitted from the request when unset (server default applies).
  `top-k`/`min-p`/`repeat-penalty` are llama.cpp/SGLang extensions, ignored by stock OpenAI.
- `--no-thinking` — turn off model reasoning (sends `chat_template_kwargs.enable_thinking=false`).
  Reasoning is on by default and shown dimmed in the terminal.
- `--preserve-thinking` — keep prior `<think>` reasoning in history across turns
  (default: stripped, per Qwen3 multi-turn guidance). Ignored by the `claude-cli` backend.
```

- [ ] **Step 5: Run workspace tests + clippy**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test --workspace && cargo clippy --all-targets -- -D warnings`
Expected: green.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-cli agent/docs/RUNNING.md
git commit -m "feat(cli): add sampling + thinking flags and document them"
```

---

## Task 7: Web — settings types & SettingsPanel inputs

**Files:**
- Modify: `web/src/wire.ts` (`RuntimeSettings` fields)
- Modify: `web/src/components/SettingsPanel.tsx` (inputs)
- Test: `web/src/components/SettingsPanel.test.tsx` (create)

**Interfaces:**
- Produces: `RuntimeSettings` gains `top_p`, `top_k`, `min_p`, `presence_penalty`, `repeat_penalty` typed `number | null`, and `enable_thinking`, `preserve_thinking` typed `boolean`.

- [ ] **Step 1: Extend the `RuntimeSettings` type**

In `wire.ts`, add to `interface RuntimeSettings` after `context_limit: number;`:

```typescript
  top_p: number | null;
  top_k: number | null;
  min_p: number | null;
  presence_penalty: number | null;
  repeat_penalty: number | null;
  enable_thinking: boolean;
  preserve_thinking: boolean;
```

- [ ] **Step 2: Write a failing SettingsPanel test**

Create `web/src/components/SettingsPanel.test.tsx`:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { SettingsPanel } from "./SettingsPanel";
import type { RuntimeSettings } from "../wire";

const base: RuntimeSettings = {
  backend: "openai", base_url: "http://x", model: "m", protocol: "native",
  command_allowlist: [], command_denylist: [], temperature: 0.2, max_tokens: 2048,
  max_turns: 25, context_limit: 8192,
  top_p: null, top_k: null, min_p: null, presence_penalty: null, repeat_penalty: null,
  enable_thinking: true, preserve_thinking: false,
};

describe("SettingsPanel sampling inputs", () => {
  it("maps empty top_p to null and a typed value to a number on save", () => {
    const onSave = vi.fn();
    render(<SettingsPanel settings={base} meta={null} error={null} disabled={false}
      onSave={onSave} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("Top-p"), { target: { value: "0.9" } });
    fireEvent.click(screen.getByText("Save"));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ top_p: 0.9, top_k: null }));
  });

  it("toggles enable_thinking", () => {
    const onSave = vi.fn();
    render(<SettingsPanel settings={base} meta={null} error={null} disabled={false}
      onSave={onSave} onClose={() => {}} />);
    fireEvent.click(screen.getByLabelText("Enable thinking"));
    fireEvent.click(screen.getByText("Save"));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ enable_thinking: false }));
  });
});
```

- [ ] **Step 3: Run to verify failure**

Run: `cd web && npm test -- SettingsPanel 2>&1 | tail -20`
Expected: FAIL — no "Top-p"/"Enable thinking" labels.

- [ ] **Step 4: Add a "Sampling & thinking" section to SettingsPanel**

In `SettingsPanel.tsx`, add a helper above the `return` (after the `save` definition):

```tsx
  const num = (k: keyof RuntimeSettings) => (e: React.ChangeEvent<HTMLInputElement>) =>
    set(k, (e.target.value === "" ? null : Number(e.target.value)) as RuntimeSettings[typeof k]);
  const numVal = (v: number | null) => (v === null ? "" : v);
```
Then insert a new `<section>` just before the closing `meta && (` block:

```tsx
        <section className="mb-4 space-y-3">
          <h3 className="text-sm font-semibold text-zinc-300">Sampling &amp; thinking</h3>
          <div>
            <label className={label} htmlFor="top_p">Top-p</label>
            <input id="top_p" type="number" step="0.05" className={field}
              value={numVal(form.top_p)} onChange={num("top_p")} />
          </div>
          <div>
            <label className={label} htmlFor="top_k">Top-k</label>
            <input id="top_k" type="number" className={field}
              value={numVal(form.top_k)} onChange={num("top_k")} />
          </div>
          <div>
            <label className={label} htmlFor="min_p">Min-p</label>
            <input id="min_p" type="number" step="0.01" className={field}
              value={numVal(form.min_p)} onChange={num("min_p")} />
          </div>
          <div>
            <label className={label} htmlFor="presence_penalty">Presence penalty</label>
            <input id="presence_penalty" type="number" step="0.1" className={field}
              value={numVal(form.presence_penalty)} onChange={num("presence_penalty")} />
          </div>
          <div>
            <label className={label} htmlFor="repeat_penalty">Repeat penalty</label>
            <input id="repeat_penalty" type="number" step="0.05" className={field}
              value={numVal(form.repeat_penalty)} onChange={num("repeat_penalty")} />
          </div>
          <label className="flex items-center gap-2 text-sm">
            <input id="enable_thinking" type="checkbox" checked={form.enable_thinking}
              onChange={(e) => set("enable_thinking", e.target.checked)} />
            Enable thinking
          </label>
          <label className="flex items-center gap-2 text-sm">
            <input id="preserve_thinking" type="checkbox" checked={form.preserve_thinking}
              onChange={(e) => set("preserve_thinking", e.target.checked)} />
            Preserve thinking in history
          </label>
        </section>
```
Note: the checkbox `<label>` wraps its `<input>`, so `getByLabelText("Enable thinking")` resolves it.

- [ ] **Step 5: Run web tests to verify pass**

Run: `cd web && npm test -- SettingsPanel 2>&1 | tail -20`
Expected: both tests pass.

- [ ] **Step 6: Typecheck/build + commit**

Run: `cd web && npm run build 2>&1 | tail -5`
Expected: build succeeds.

```bash
git add web/src/wire.ts web/src/components/SettingsPanel.tsx web/src/components/SettingsPanel.test.tsx
git commit -m "feat(web): sampling + thinking inputs in SettingsPanel"
```

---

## Task 8: Web — reasoning display

**Files:**
- Modify: `web/src/wire.ts` (`WireEvent` reasoning)
- Modify: `web/src/state.ts` (`Item` + reducer case)
- Modify: `web/src/components/MessageList.tsx` (case)
- Create: `web/src/components/ReasoningMessage.tsx`
- Test: `web/src/state.test.ts` (add cases) or create `web/src/reasoning.test.ts`

**Interfaces:**
- Consumes: `WireEvent::Reasoning { text }` from the daemon (Task 4).
- Produces: `Item` gains `{ kind: "reasoning"; text: string }`.

- [ ] **Step 1: Add the reasoning `WireEvent` variant**

In `wire.ts`, add to the `WireEvent` union after the `token` line:

```typescript
  | { type: "reasoning"; text: string }
```

- [ ] **Step 2: Write a failing reducer test**

Create `web/src/reasoning.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { reduce, initialState } from "./state";
import type { Inbound } from "./wire";

const ev = (payload: unknown): Inbound =>
  ({ v: 1, session_id: "s", kind: "event", payload } as Inbound);

describe("reasoning events", () => {
  it("accumulates reasoning into a reasoning item, separate from the answer", () => {
    let s = initialState([]);
    s = reduce(s, { type: "user_send", text: "hi" });
    s = reduce(s, { type: "frame", frame: ev({ type: "reasoning", text: "plan " }) });
    s = reduce(s, { type: "frame", frame: ev({ type: "reasoning", text: "more" }) });
    s = reduce(s, { type: "frame", frame: ev({ type: "token", text: "answer" }) });
    const reasoning = s.items.find((i) => i.kind === "reasoning");
    const assistant = s.items.find((i) => i.kind === "assistant");
    expect(reasoning).toMatchObject({ kind: "reasoning", text: "plan more" });
    expect(assistant).toMatchObject({ kind: "assistant", text: "answer" });
  });
});
```

- [ ] **Step 3: Run to verify failure**

Run: `cd web && npm test -- reasoning 2>&1 | tail -20`
Expected: FAIL — no reasoning item produced.

- [ ] **Step 4: Add the `Item` kind and reducer case**

In `state.ts`, add to the `Item` union:

```typescript
  | { kind: "reasoning"; text: string }
```
In `reduceFrame`'s `switch (p.type)`, add a case (mirrors the token accumulator but for reasoning items):

```typescript
    case "reasoning": {
      const items = [...s.items];
      const last = items[items.length - 1];
      if (last && last.kind === "reasoning") {
        items[items.length - 1] = { ...last, text: last.text + p.text };
      } else {
        items.push({ kind: "reasoning", text: p.text });
      }
      return { ...s, items };
    }
```

- [ ] **Step 5: Run reducer test to verify pass**

Run: `cd web && npm test -- reasoning 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Create the collapsible `ReasoningMessage` component**

Create `web/src/components/ReasoningMessage.tsx`:

```tsx
import { useState } from "react";

export function ReasoningMessage({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="my-2 max-w-[80%] rounded border border-zinc-700 bg-zinc-900/60 px-3 py-2 text-xs text-zinc-400">
      <button onClick={() => setOpen((o) => !o)} className="font-medium text-zinc-300">
        {open ? "▾" : "▸"} Thinking
      </button>
      {open && <pre className="mt-1 whitespace-pre-wrap break-words">{text}</pre>}
    </div>
  );
}
```

- [ ] **Step 7: Render reasoning items in `MessageList`**

In `MessageList.tsx`, import the component and add a case in the `switch (it.kind)`:

```tsx
import { ReasoningMessage } from "./ReasoningMessage";
```
```tsx
          case "reasoning":
            return <ReasoningMessage key={i} text={it.text} />;
```

- [ ] **Step 8: Run all web tests + build**

Run: `cd web && npm test 2>&1 | tail -15 && npm run build 2>&1 | tail -5`
Expected: all tests pass; build succeeds (the `MessageList` switch is now exhaustive over `Item` kinds).

- [ ] **Step 9: Commit**

```bash
git add web/src/wire.ts web/src/state.ts web/src/components/MessageList.tsx web/src/components/ReasoningMessage.tsx web/src/reasoning.test.ts
git commit -m "feat(web): collapsible Thinking display for reasoning events"
```

---

## Task 9: Live validation, follow-ups ledger & graph refresh

**Files:**
- Modify: `docs/superpowers/context/follow-ups.md` (create if absent)

**Interfaces:** none (closeout task).

- [ ] **Step 1: Live smoke test against Qwen3**

Ensure the model server is up (`docker start llama-agent` if needed). Then:

Run:
```bash
source "$HOME/.cargo/env" && cd agent
cargo run -p agent-cli -- --base-url http://localhost:8080 --model qwen3.6-35b-a3b \
  --workspace /tmp --context-limit 32768 --top-k 20
```
At the prompt, enter a small task (e.g. "say hello and stop"). Expected: reasoning renders dimmed, the answer renders normally, no panics. Then re-run with `--no-thinking` and confirm no `<think>`/dim reasoning appears. (Manual; not a gated automated test.)

- [ ] **Step 2: Record review follow-ups**

After the whole-branch review, add a dated section to `docs/superpowers/context/follow-ups.md` (create the file with a top-level `# Review Follow-ups` heading if it does not exist):

```markdown
## 2026-06-23 sampling-thinking-settings

- <title> — <file:line> — Open|Accepted|Resolved (<sha if resolved>) — <one-line reason>
```
Populate with every Minor finding (plus Accepted won't-fix items) from the final review. If the review surfaced none, record: `- No Critical/Important findings; Minors (if any) resolved in-cycle.`

- [ ] **Step 3: Commit the ledger**

```bash
git add docs/superpowers/context/follow-ups.md
git commit -m "docs(follow-ups): record sampling-thinking-settings review findings"
```

- [ ] **Step 4: Refresh the knowledge graph (once, after the subsystem is complete)**

Run: `/graphify . --update`
Expected: only changed files re-extracted; the graph reflects the new fields/variants. (Per the project primer, run this ONCE after the whole subsystem is merged-ready, not per task.)

---

## Self-Review

**Spec coverage:**
- §3 RuntimeConfig fields/validation/serde defaults → Task 1. ✓
- §4 CompletionRequest fields + body() omit-when-None + chat_template_kwargs → Task 2. ✓
- §4 Chunk::Reasoning + reasoning_content + ThinkingSplitter + AssistantTurn.reasoning → Task 3. ✓
- §5 LoopConfig fields + request build → Task 2; reasoning accumulation → Task 3; AgentEvent::Reasoning emit → Task 4; history strip/preserve → Task 5. ✓
- §6 claude-cli ignores knobs → no forwarding added to ClaudeCliClient (it consumes only messages); validate() adds no claude-cli sampling rejection (Task 1). ✓
- §7 WireEvent::Reasoning + wire_event_from; Worker unchanged → Task 4 (no cloud edits). ✓
- §8 CLI flags + terminal render → Tasks 4 (render), 6 (flags); web RuntimeSettings + SettingsPanel + collapsible Thinking → Tasks 7, 8. ✓
- §10 testing (validation, round-trip, body, splitter, history, wire, web) → Tasks 1–8. ✓
- §11 follow-ups durability → Task 9. ✓

**Placeholder scan:** No "TBD/TODO"; every code step shows complete code; Task 2 Step 9's CLI defaults are real values replaced by flags in Task 6 (not a placeholder, an intermediate default).

**Type consistency:** `ThinkingSplitter::push/flush`, `Chunk::Reasoning(String)`, `AssistantTurn.reasoning: String`, `AgentEvent::Reasoning(String)`, `WireEvent::Reasoning { text }`, `RuntimeSettings.top_p: number | null`, `Item { kind: "reasoning"; text }` are used consistently across tasks. `enable_thinking`/`preserve_thinking` are `bool` in `RuntimeConfig`/`LoopConfig`, `boolean` in TS, and on the wire the daemon receives a full `RuntimeConfig` (so `null` sampling values deserialize to `None`).
