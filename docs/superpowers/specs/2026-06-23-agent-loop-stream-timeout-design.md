# Per-turn idle timeout for model-stream consumption (P1) — design

**Status:** approved, ready for implementation plan
**Date:** 2026-06-23
**Scope:** one follow-up (P1) from `docs/superpowers/context/claude-cli-inference.md`

## Problem

`AgentLoop::one_completion` (`agent/crates/agent-core/src/loop_.rs:53-66`) consumes the
model stream with a bare loop and no deadline:

```rust
let mut stream = self.model.stream(req).await?;
...
while let Some(item) = stream.next().await { ... }
```

Neither the initial `self.model.stream(req).await` (stream-open / first-byte) nor the
per-chunk `stream.next().await` is bounded. A backend that stalls — before the first
byte or mid-stream — blocks the turn **forever**. `tool_timeout` (line ~167) wraps only
tool *execution*, not model streaming.

This is **pre-existing** and **backend-agnostic**: it affects the `OpenAiCompatClient`
(SGLang) path and the `ClaudeCliClient` path identically. The fix belongs in the core
loop, which is why it gets a brainstorm→spec cycle rather than an ad-hoc patch. The
"keep the core untouched" discipline (held through #5 and #6) yields here deliberately:
this is a correctness bug *in* the core loop, and P1 explicitly calls for a spec'd loop
change.

## Decisions (locked during brainstorming)

1. **Timeout semantics:** idle / inter-chunk deadline (reset on every chunk), not a
   total wall-clock cap. A healthy but long generation is never killed; only a genuine
   lack of progress trips it.
2. **On-timeout behavior:** surface a new retryable `ModelError::Timeout`, reusing the
   existing `completion_with_retry` backoff path. A transient stall recovers; a
   persistent hang exhausts `max_retries` and fails the turn cleanly and bounded.
3. **Config surface:** required `Duration` on `LoopConfig` (always-on, can't be disabled)
   + a CLI flag, defaulted at 120s.

## Mechanism — idle (inter-chunk) deadline

In `one_completion`, wrap **both** awaits in `tokio::time::timeout(idle, …)`:

- the initial `self.model.stream(req).await` — covers a stream-open / first-byte hang, and
- each `stream.next().await` — covers inter-token stalls.

The clock resets on every chunk received, so a healthy long generation never trips —
only a lack of progress for `idle` seconds does.

```rust
async fn one_completion(&self, req: CompletionRequest) -> Result<AssistantTurn, ModelError> {
    let idle = self.config.stream_idle_timeout;
    let mut stream = match tokio::time::timeout(idle, self.model.stream(req)).await {
        Err(_) => return Err(ModelError::Timeout(idle)),
        Ok(r) => r?,
    };
    let mut text = String::new();
    let mut raw_tool_calls: Vec<RawToolCall> = Vec::new();
    let mut stop = StopReason::Stop;
    loop {
        match tokio::time::timeout(idle, stream.next()).await {
            Err(_) => return Err(ModelError::Timeout(idle)), // stream dropped here
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

On timeout we return early; the `stream` local is dropped. For `ClaudeCliClient` that
drop triggers `kill_on_drop(true)` and reaps the subprocess; for `OpenAiCompatClient` it
tears down the reqwest connection. **No new cancellation plumbing is required** — the
existing inert `CancellationToken` in `run_tool` is unrelated and stays as-is.

## Error flow — retryable

Add a variant to `ModelError` (`agent/crates/agent-model/src/types.rs:67-79`):

```rust
#[error("stream idle timeout after {0:?}")]
Timeout(std::time::Duration),
```

`one_completion` already returns `Result<_, ModelError>`, and `completion_with_retry`
(`loop_.rs:69-88`) already retries **any** `ModelError` with backoff. So a timeout flows
through the existing path with **zero new control flow**: retried up to `max_retries`,
then surfaced as `AgentError::Model` with `AgentEvent::Error` emitted on final failure
(exactly as transport errors are handled today).

## Config & CLI surface

- Add `stream_idle_timeout: Duration` to `LoopConfig` (`loop_.rs:18-26`). Always-on;
  **not** an `Option` — the default-off footgun is the bug being fixed.
- Add `pub const DEFAULT_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);` to
  `agent-core`. 120s is generous enough for claude-cli cold-start + `thinking` blocks
  before the first token (spike notes: ~3.5s trivial, longer with thinking).
- **CLI** (`agent/crates/agent-cli/src/main.rs`): add
  `--stream-timeout-secs` (`#[arg(long, default_value_t = 120)]`), wired into the
  `LoopConfig` constructed at ~line 74.
- **Daemon** (`agent/crates/agent-server/src/daemon.rs:52`): set
  `stream_idle_timeout: DEFAULT_STREAM_IDLE_TIMEOUT` (no remote knob now; a future
  Settings channel can carry it).

## Testing (TDD)

- Extend `ScriptedModel` (`agent/crates/agent-core/src/testkit.rs:12-44`) with:
  - `Scripted::Hang` — `stream()` succeeds but the returned stream's `next()` never
    resolves (`futures::stream::pending().boxed()`), covering the inter-chunk await.
  - `Scripted::HangOpen` — the `stream()` call itself never resolves
    (`std::future::pending().await`), covering the stream-open await.
- Drive tests with the Tokio test clock: `#[tokio::test(start_paused = true)]` (or
  explicit `tokio::time::advance`) so they are instant and deterministic — no real waits.
- Cases:
  1. **Idle hang → fail:** `Scripted::Hang` with small `max_retries` → loop retries,
     exhausts retries, ends with `AgentEvent::Error` (last event is `error`/`done` per
     the existing event-name convention). Asserts the timeout is classified as a
     `ModelError` (retried), not a panic/hang.
  2. **Stream-open hang → fail:** `Scripted::HangOpen` → same terminal behavior, proving
     the initial await is bounded too.
  3. **Hang-then-success → recover:** `[Scripted::Hang, Scripted::Text("recovered")]`
     with `max_retries >= 1` → reaches `done` (mirrors the existing
     `transport_error_then_success_via_retry` test).
  4. **Slow but progressing → no trip:** a stream that yields a chunk every `idle/2`
     completes normally without a timeout (guards against killing healthy long
     generations).
- All existing 71 tests stay green; `cargo clippy --all-targets -- -D warnings` clean.

## Out of scope (YAGNI)

- Total wall-clock per-turn cap (idle timeout is the right stall detector).
- Separate first-chunk vs idle knobs (one knob, generously defaulted).
- Wiring the inert `CancellationToken` to Ctrl-C/SIGINT (separate concern).
- Remote / daemon-side configurability of the timeout (deferred to a Settings channel).

## Touched files (summary)

- `agent/crates/agent-model/src/types.rs` — `ModelError::Timeout(Duration)` variant.
- `agent/crates/agent-core/src/loop_.rs` — idle-timeout wrapping in `one_completion`;
  `stream_idle_timeout` field on `LoopConfig`; `DEFAULT_STREAM_IDLE_TIMEOUT` const.
- `agent/crates/agent-core/src/testkit.rs` — `Scripted::Hang` / `Scripted::HangOpen`.
- `agent/crates/agent-cli/src/main.rs` — `--stream-timeout-secs` flag + wiring.
- `agent/crates/agent-server/src/daemon.rs` — wire the default const.
- Update the P1 box in `docs/superpowers/context/claude-cli-inference.md` to Resolved on
  completion (out of this spec; a plan step).
