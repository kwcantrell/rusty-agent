# Per-turn idle timeout for model-stream consumption (P1) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bound model-stream consumption in `AgentLoop` with an idle (inter-chunk) timeout so a stalled backend can no longer block a turn forever.

**Architecture:** Wrap both the initial `model.stream(req)` await and each `stream.next()` await in `tokio::time::timeout(idle, …)`. On elapse, drop the stream (firing `ClaudeCliClient`'s `kill_on_drop` / tearing down the reqwest connection) and return a new retryable `ModelError::Timeout`, which flows through the existing `completion_with_retry` backoff path. Configured by a required `stream_idle_timeout: Duration` on `LoopConfig`, defaulted to 120s and exposed via a CLI flag.

**Tech Stack:** Rust, Tokio (`time::timeout`, `test-util` time pausing via `#[tokio::test(start_paused = true)]`), `futures::StreamExt`, `thiserror`, `clap`.

## Global Constraints

- Run `source "$HOME/.cargo/env"` before any cargo command (cargo is not on PATH).
- Build/test from the `agent/` directory.
- `cargo clippy --all-targets -- -D warnings` must stay clean.
- All existing tests stay green (71 Rust tests pass today).
- Keep changes additive and follow existing patterns; do not restructure unrelated code.
- The spec is `docs/superpowers/specs/2026-06-23-agent-loop-stream-timeout-design.md`.

---

### Task 1: `ModelError::Timeout` variant

**Files:**
- Modify: `agent/crates/agent-model/src/types.rs:67-79` (the `ModelError` enum)

**Interfaces:**
- Consumes: nothing.
- Produces: `ModelError::Timeout(std::time::Duration)` — a new enum variant the loop will
  return on stall. `Display` renders as `stream idle timeout after <Duration:?>`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module at the bottom of `agent/crates/agent-model/src/types.rs`:

```rust
    #[test]
    fn timeout_error_displays_duration() {
        let e = ModelError::Timeout(std::time::Duration::from_secs(120));
        assert_eq!(e.to_string(), "stream idle timeout after 120s");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-model timeout_error_displays_duration`
Expected: FAIL to compile — `no variant named Timeout found for enum ModelError`.

- [ ] **Step 3: Add the variant**

In the `ModelError` enum in `agent/crates/agent-model/src/types.rs`, add as the last variant
(after `Process`):

```rust
    #[error("stream idle timeout after {0:?}")]
    Timeout(std::time::Duration),
```

- [ ] **Step 4: Run test to verify it passes**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-model timeout_error_displays_duration`
Expected: PASS.

Note: `{0:?}` on `Duration::from_secs(120)` renders `120s`, so the expected string matches.

- [ ] **Step 5: Commit**

```bash
cd agent && git add crates/agent-model/src/types.rs
git commit -m "feat(model): add retryable ModelError::Timeout variant"
```

---

### Task 2: Plumb `stream_idle_timeout` config + testkit hang doubles (no behavior change)

This task only adds the config field, the default constant, the test doubles, and updates
every `LoopConfig` construction site so the workspace keeps compiling. The loop's behavior
is unchanged until Task 3.

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs:18-26` (`LoopConfig` struct) and add a `const`
- Modify: `agent/crates/agent-core/src/testkit.rs:12-44` (`Scripted` enum + `ScriptedModel::stream`)
- Modify: `agent/crates/agent-core/src/loop_.rs` tests (4 `LoopConfig` literals at ~215, ~249, ~270, ~290)
- Modify: `agent/crates/agent-core/tests/e2e_sglang.rs:40-41`
- Modify: `agent/crates/agent-cli/src/main.rs:74-77`
- Modify: `agent/crates/agent-server/src/daemon.rs:52-56`

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces:
  - `LoopConfig.stream_idle_timeout: std::time::Duration` — required field.
  - `agent_core::DEFAULT_STREAM_IDLE_TIMEOUT: std::time::Duration` (= 120s), re-exported via
    `pub use loop_::*`.
  - `agent_core::testkit::Scripted::Hang` — `stream()` returns Ok, but the stream's `next()`
    never resolves.
  - `agent_core::testkit::Scripted::HangOpen` — the `stream()` call itself never resolves.

- [ ] **Step 1: Add the field and the default constant**

In `agent/crates/agent-core/src/loop_.rs`, add the field to `LoopConfig` (after `tool_timeout`):

```rust
pub struct LoopConfig {
    pub model_limit: usize,
    pub max_turns: usize,
    pub max_retries: usize,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    pub workspace: PathBuf,
    pub tool_timeout: Duration,
    /// Max time with no stream progress (stream-open or inter-chunk) before a turn
    /// is treated as a stalled-backend `ModelError::Timeout`.
    pub stream_idle_timeout: Duration,
}
```

Immediately above the `LoopConfig` struct, add the default constant:

```rust
/// Default idle timeout for model-stream consumption. Generous enough to cover
/// claude-cli cold-start + `thinking` blocks before the first token.
pub const DEFAULT_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
```

- [ ] **Step 2: Add the test-double variants**

In `agent/crates/agent-core/src/testkit.rs`, add two variants to the `Scripted` enum
(after `Error`):

```rust
    /// `stream()` succeeds but the returned stream never yields (inter-chunk stall).
    Hang,
    /// The `stream()` call itself never resolves (stream-open stall).
    HangOpen,
```

Then handle them in `ScriptedModel::stream`. The `match next { … }` is exhaustive, so add
two arms. Place them before the closing brace of the match:

```rust
            Scripted::Hang => Ok(stream::pending().boxed()),
            Scripted::HangOpen => {
                std::future::pending::<()>().await;
                unreachable!("HangOpen never resolves")
            }
```

`stream::pending()` is already reachable via the `use futures::stream::{self, …}` import at
the top of the file. `std::future::pending` needs no import.

- [ ] **Step 3: Update every `LoopConfig` construction site**

The new required field breaks all literals. Add `stream_idle_timeout: …` to each.

In `agent/crates/agent-core/src/loop_.rs` test module — all four test `LoopConfig` literals
get the same addition. They currently end with `tool_timeout: std::time::Duration::from_secs(5) }`
(or `from_secs(5)`); change each to also set the field, e.g.:

```rust
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60) });
```

Apply to all four (`runs_tool_then_finishes`, `denied_tool_feeds_error_back_and_continues`,
`transport_error_then_success_via_retry`, `budget_exhaustion_stops_the_loop`). Match each
literal's existing closing punctuation.

In `agent/crates/agent-core/tests/e2e_sglang.rs:40-41`, change:

```rust
        LoopConfig { model_limit: 8192, max_turns: 8, max_retries: 2, temperature: 0.0,
            max_tokens: Some(512), workspace: ws, tool_timeout: Duration::from_secs(60),
            stream_idle_timeout: Duration::from_secs(120) });
```

In `agent/crates/agent-cli/src/main.rs:74-77`, change the `LoopConfig` to:

```rust
        sink, LoopConfig {
            model_limit: cli.context_limit, max_turns: 25, max_retries: 3, temperature: 0.2,
            max_tokens: Some(2048), workspace, tool_timeout: Duration::from_secs(120),
            stream_idle_timeout: agent_core::DEFAULT_STREAM_IDLE_TIMEOUT,
        });
```

(The CLI flag arrives in Task 4; for now it uses the default constant.)

In `agent/crates/agent-server/src/daemon.rs:52-56`, change the `LoopConfig` to:

```rust
        LoopConfig {
            model_limit: params.context_limit, max_turns: 25, max_retries: 3,
            temperature: 0.2, max_tokens: Some(2048), workspace: params.workspace.clone(),
            tool_timeout: Duration::from_secs(120),
            stream_idle_timeout: agent_core::DEFAULT_STREAM_IDLE_TIMEOUT,
        },
```

If `daemon.rs` does not already import `agent_core` such that `agent_core::DEFAULT_STREAM_IDLE_TIMEOUT`
resolves, it does — line 5 is `use agent_core::{AgentLoop, LoopConfig, WindowContext};`, and the
crate path `agent_core::` is always available. Same for `main.rs`.

- [ ] **Step 4: Build and run the whole suite to verify green (no behavior change)**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test --workspace`
Expected: PASS — same tests as before, all green. The new field and variants compile; no
loop behavior has changed yet.

Run: `source "$HOME/.cargo/env" && cd agent && cargo clippy --all-targets -- -D warnings`
Expected: clean. (If clippy warns that `Scripted::Hang`/`HangOpen` are never constructed,
that is expected at this point — they are used in Task 3. If it errors under `-D warnings`,
add `#[allow(dead_code)]` to those two variants now and remove it in Task 3. Verify by
running the clippy command; only add the allow if it actually fails.)

- [ ] **Step 5: Commit**

```bash
cd agent && git add crates/agent-core/src/loop_.rs crates/agent-core/src/testkit.rs \
  crates/agent-core/tests/e2e_sglang.rs crates/agent-cli/src/main.rs \
  crates/agent-server/src/daemon.rs
git commit -m "feat(core): plumb stream_idle_timeout config + hang test doubles"
```

---

### Task 3: Enforce the idle timeout in `one_completion`

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs:52-66` (`one_completion`)
- Test: `agent/crates/agent-core/src/loop_.rs` (its existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `ModelError::Timeout` (Task 1); `LoopConfig.stream_idle_timeout`,
  `Scripted::Hang`, `Scripted::HangOpen` (Task 2). `AgentError::Model(_)` already exists
  (`loop_.rs:12-16`).
- Produces: no new public surface — `one_completion` keeps its signature
  `async fn one_completion(&self, req: CompletionRequest) -> Result<AssistantTurn, ModelError>`.

**Why the tests use a guard:** without the timeout, a stalled stream makes `agent.run`
hang forever, which is an infinite test, not a clean failure. Each red/green test wraps
`agent.run(...)` in a test-level `tokio::time::timeout(600s, …)` *guard* that is much longer
than the loop's 10s idle timeout. Red (no loop timeout): `run` hangs, the 600s guard fires,
`.expect(...)` panics → FAIL. Green (loop timeout present): the loop's 10s timeout returns an
error long before the guard → guard does not fire → assertions run. Under
`#[tokio::test(start_paused = true)]` the Tokio clock auto-advances, so both timers fire
instantly and the tests are sub-millisecond.

- [ ] **Step 1: Write the failing tests**

Add these four tests inside the existing `mod tests` in `agent/crates/agent-core/src/loop_.rs`
(the module already imports `super::*`, `crate::testkit::*`, `WindowContext`, `Message`,
`RulePolicy`, and has `registry()` / `policy(ws)` helpers). Reuse those helpers.

```rust
    #[tokio::test(start_paused = true)]
    async fn idle_stall_times_out_and_fails_after_retries() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![Scripted::Hang, Scripted::Hang, Scripted::Hang]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 2, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: Duration::from_secs(5),
                stream_idle_timeout: Duration::from_secs(10) });
        let mut ctx = WindowContext::new(Message::system("sys"));
        // Guard >> the loop's 10s idle timeout so the loop terminates first.
        let result = tokio::time::timeout(Duration::from_secs(600), agent.run(&mut ctx, "go".into()))
            .await
            .expect("loop must terminate on a stalled stream, not hang");
        assert!(matches!(result, Err(AgentError::Model(_))));
        let events = sink.events.lock().unwrap().clone();
        assert!(events.iter().any(|e| e.starts_with("error:")));
    }

    #[tokio::test(start_paused = true)]
    async fn stream_open_stall_times_out() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![Scripted::HangOpen, Scripted::HangOpen]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 1, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: Duration::from_secs(5),
                stream_idle_timeout: Duration::from_secs(10) });
        let mut ctx = WindowContext::new(Message::system("sys"));
        let result = tokio::time::timeout(Duration::from_secs(600), agent.run(&mut ctx, "go".into()))
            .await
            .expect("loop must terminate when the stream never opens, not hang");
        assert!(matches!(result, Err(AgentError::Model(_))));
    }

    #[tokio::test(start_paused = true)]
    async fn stall_then_success_recovers_via_retry() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Hang,
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 3, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: Duration::from_secs(5),
                stream_idle_timeout: Duration::from_secs(10) });
        let mut ctx = WindowContext::new(Message::system("sys"));
        let result = tokio::time::timeout(Duration::from_secs(600), agent.run(&mut ctx, "go".into()))
            .await
            .expect("loop must terminate, not hang");
        assert!(result.is_ok());
        assert_eq!(sink.events.lock().unwrap().last().unwrap(), "done");
    }

    struct SlowModel { gap: Duration }
    #[async_trait::async_trait]
    impl agent_model::ModelClient for SlowModel {
        async fn stream(&self, _req: CompletionRequest)
            -> Result<futures::stream::BoxStream<'static, Result<Chunk, ModelError>>, ModelError> {
            let gap = self.gap;
            let chunks = vec![
                Ok(Chunk::Text("hel".into())),
                Ok(Chunk::Text("lo".into())),
                Ok(Chunk::Done(StopReason::Stop)),
            ];
            Ok(futures::stream::iter(chunks)
                .then(move |c| async move { tokio::time::sleep(gap).await; c })
                .boxed())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn slow_but_progressing_stream_does_not_trip() {
        let ws = std::env::temp_dir();
        // gap (5s) < idle timeout (10s): healthy progress must NOT trip the timeout.
        let model = Arc::new(SlowModel { gap: Duration::from_secs(5) });
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 1, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: Duration::from_secs(5),
                stream_idle_timeout: Duration::from_secs(10) });
        let mut ctx = WindowContext::new(Message::system("sys"));
        let result = tokio::time::timeout(Duration::from_secs(600), agent.run(&mut ctx, "go".into()))
            .await
            .expect("loop must terminate, not hang");
        assert!(result.is_ok());
        let events = sink.events.lock().unwrap().clone();
        assert!(!events.iter().any(|e| e.starts_with("error:")));
        assert_eq!(events.last().unwrap(), "done");
    }
```

The test module's existing `use` lines cover `Arc`, `Message`, `RulePolicy`, `WindowContext`,
and (via `super::*`) `Chunk`, `StopReason`, `CompletionRequest`, `ModelError`, `AgentError`.
`futures` and `async_trait` are crate dependencies (referenced fully-qualified above), so no
new `use` is required. If the compiler reports `then`/`boxed` unresolved, add
`use futures::StreamExt;` inside the test module.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-core --lib loop_`
Expected: the three stall/recover tests FAIL — `idle_stall_times_out_and_fails_after_retries`,
`stream_open_stall_times_out`, and `stall_then_success_recovers_via_retry` panic with the
`.expect("…not hang")` message because the un-timeout'd loop never returns. (`slow_but_progressing_stream_does_not_trip`
may already pass — it is a regression guard, not a red/green case.)

- [ ] **Step 3: Implement the idle timeout in `one_completion`**

Replace the body of `one_completion` in `agent/crates/agent-core/src/loop_.rs` (currently
lines 53-66) with:

```rust
    async fn one_completion(&self, req: CompletionRequest) -> Result<AssistantTurn, ModelError> {
        let idle = self.config.stream_idle_timeout;
        let mut stream = match tokio::time::timeout(idle, self.model.stream(req)).await {
            Err(_) => return Err(ModelError::Timeout(idle)),
            Ok(opened) => opened?,
        };
        let mut text = String::new();
        let mut raw_tool_calls: Vec<RawToolCall> = Vec::new();
        let mut stop = StopReason::Stop;
        loop {
            match tokio::time::timeout(idle, stream.next()).await {
                // Stalled: dropping `stream` on return fires kill_on_drop / tears down the connection.
                Err(_) => return Err(ModelError::Timeout(idle)),
                Ok(None) => break,
                Ok(Some(item)) => match item? {
                    Chunk::Text(t) => { self.sink.emit(AgentEvent::Token(t.clone())); text.push_str(&t); }
                    Chunk::ToolCallDelta(rc) => merge_tool_call(&mut raw_tool_calls, rc),
                    Chunk::Done(r) => stop = r,
                },
            }
        }
        Ok(AssistantTurn { text, raw_tool_calls, stop })
    }
```

`tokio::time::timeout` returns `Err(Elapsed)` on the deadline; `Ok(_)` carries the inner
result. The `?` on `opened` and on `item` preserves the existing error propagation.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-core --lib loop_`
Expected: PASS — all four new tests plus the existing loop tests
(`runs_tool_then_finishes`, `denied_tool_feeds_error_back_and_continues`,
`transport_error_then_success_via_retry`, `budget_exhaustion_stops_the_loop`).

- [ ] **Step 5: Full suite + clippy**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test --workspace`
Expected: PASS (all crates).

Run: `source "$HOME/.cargo/env" && cd agent && cargo clippy --all-targets -- -D warnings`
Expected: clean. If you added `#[allow(dead_code)]` to the `Scripted` variants in Task 2,
remove it now (they are constructed by these tests) and re-run clippy.

- [ ] **Step 6: Commit**

```bash
cd agent && git add crates/agent-core/src/loop_.rs
git commit -m "feat(core): enforce idle timeout on model-stream consumption"
```

---

### Task 4: Expose `--stream-timeout-secs` CLI flag

**Files:**
- Modify: `agent/crates/agent-cli/src/main.rs:16-40` (the `Cli` struct) and `:74-77` (`LoopConfig`)

**Interfaces:**
- Consumes: `LoopConfig.stream_idle_timeout` (Task 2).
- Produces: a `--stream-timeout-secs <u64>` CLI flag (default 120) wired into `LoopConfig`.

- [ ] **Step 1: Add the flag to the `Cli` struct**

In `agent/crates/agent-cli/src/main.rs`, add after the `context_limit` field (line ~39):

```rust
    /// Idle timeout (seconds) for model-stream consumption before a stalled turn fails
    #[arg(long, default_value_t = 120)]
    stream_timeout_secs: u64,
```

- [ ] **Step 2: Wire the flag into `LoopConfig`**

Change the `stream_idle_timeout` line in the `LoopConfig` constructed at ~line 74 from the
default constant (set in Task 2) to the flag value:

```rust
            stream_idle_timeout: Duration::from_secs(cli.stream_timeout_secs),
```

- [ ] **Step 3: Verify the binary builds and the flag parses**

Run: `source "$HOME/.cargo/env" && cd agent && cargo build -p agent-cli`
Expected: builds clean.

Run: `source "$HOME/.cargo/env" && cd agent && cargo run -p agent-cli -- --help`
Expected: the help text lists `--stream-timeout-secs <STREAM_TIMEOUT_SECS>` with
`[default: 120]`.

- [ ] **Step 4: Clippy**

Run: `source "$HOME/.cargo/env" && cd agent && cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
cd agent && git add crates/agent-cli/src/main.rs
git commit -m "feat(cli): add --stream-timeout-secs to configure the loop idle timeout"
```

---

### Task 5: Update docs (follow-up tracker + RUNNING)

**Files:**
- Modify: `docs/superpowers/context/claude-cli-inference.md` (the P1 box, ~lines 100-111, and
  the "Resolved" list, ~lines 129-134)
- Modify: `agent/docs/RUNNING.md` (the CLI flags / §1 area where backend flags are documented)

**Interfaces:**
- Consumes: nothing.
- Produces: documentation only.

- [ ] **Step 1: Move P1 from Open to Resolved**

In `docs/superpowers/context/claude-cli-inference.md`, remove the P1 bullet from the
`**P1 — reliability (cross-cutting, needs its own spec)**` block under `### Open` (and remove
that now-empty heading), and add a line to the `### Resolved (kept for context)` list:

```markdown
- [x] Per-turn idle timeout on model-stream consumption (P1) — `agent-core/src/loop_.rs`
  `one_completion` now wraps stream-open + each chunk in `tokio::time::timeout`, surfacing a
  retryable `ModelError::Timeout`. Configurable via `LoopConfig.stream_idle_timeout`
  (default 120s) / CLI `--stream-timeout-secs`. Spec:
  `docs/superpowers/specs/2026-06-23-agent-loop-stream-timeout-design.md`.
```

- [ ] **Step 2: Document the CLI flag in RUNNING.md**

In `agent/docs/RUNNING.md`, find the section that lists the `agent-cli` flags (near the
`--context-limit` / `--backend` documentation) and add:

```markdown
- `--stream-timeout-secs <secs>` (default 120): idle timeout for model streaming. If the
  backend produces no stream progress (no open, no new chunk) for this many seconds, the
  turn fails with a retryable timeout instead of hanging. Covers both the SGLang/OpenAI and
  `claude-cli` backends.
```

(If the exact surrounding wording differs, match the file's existing flag-list style; the
content above is the requirement.)

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/context/claude-cli-inference.md agent/docs/RUNNING.md
git commit -m "docs: mark P1 stream-timeout resolved + document --stream-timeout-secs"
```

---

## Self-Review

**Spec coverage:**
- Mechanism — idle timeout on both stream-open and inter-chunk awaits → Task 3, Step 3. ✓
- `ModelError::Timeout(Duration)` retryable variant → Task 1; flows through existing
  `completion_with_retry` (unchanged) → verified by `idle_stall_…` and `stall_then_success_…`
  tests in Task 3. ✓
- `stream_idle_timeout: Duration` on `LoopConfig` (always-on, not `Option`) +
  `DEFAULT_STREAM_IDLE_TIMEOUT` = 120s → Task 2, Step 1. ✓
- CLI `--stream-timeout-secs` (default 120) → Task 4; daemon uses default const → Task 2,
  Step 3. ✓
- Testkit `Scripted::Hang` / `Scripted::HangOpen` + paused-clock tests, cases (a)-(d) →
  Task 2 Step 2 + Task 3 Step 1. ✓
- All existing 71 tests stay green + clippy clean → checked in Task 2 Step 4 and Task 3 Step 5. ✓
- Docs: P1 → Resolved → Task 5. ✓
- Out-of-scope items (total wall-clock cap, dual knobs, CancellationToken wiring, remote
  config) → not implemented, correct. ✓

**Placeholder scan:** No TBD/TODO/"handle edge cases"; every code step shows the full code and
exact command + expected output. ✓

**Type consistency:** `stream_idle_timeout: Duration` and `DEFAULT_STREAM_IDLE_TIMEOUT`
spelled identically across Tasks 2/3/4; `ModelError::Timeout(Duration)` and
`AgentError::Model(_)` match the existing enums; `Scripted::Hang`/`HangOpen` consistent
between Task 2 (defined) and Task 3 (used); `one_completion` signature unchanged. ✓
