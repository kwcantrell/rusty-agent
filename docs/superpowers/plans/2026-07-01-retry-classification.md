# Retry Classification + Overflow Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Model errors are retried only when retrying can plausibly succeed; context overflow gets one forced compaction-and-rebuild per turn; every terminal loop path emits `Done(StopReason)`.

**Architecture:** Classification lives on `ModelError` (`class() -> ErrorClass` in agent-model, beside the enum). The loop's `completion_with_retry` consumes the class: Fatal aborts on first sight, Retryable retries with exponential backoff, ContextOverflow returns to the turn loop which forces `ctx.request_compaction()` + `maintain()` and rebuilds the request once per turn. `StopReason::Error` closes the three Done-less terminal paths.

**Tech Stack:** Rust (agent/ Cargo workspace), tokio (paused-clock tests), thiserror.

**Spec:** `docs/superpowers/specs/2026-07-01-retry-classification-design.md` — binding; read it first.

## Global Constraints

- Two Cargo workspaces; everything here is in `agent/` — run cargo from `/home/kalen/rust-agent-runtime/agent` (`source ~/.cargo/env` if cargo is missing).
- Conventional commits `type(scope): summary`.
- Clippy is `-D warnings` in CI; inline format args (`{e}` not `{}`).
- Classification table (verbatim from spec): Retryable = `Http | Stream | Process | Timeout | Status{500..=599 | 408 | 429}`; Fatal = `Status{other}` (all remaining codes) `| Decode | Cancelled (defensive)`; ContextOverflow = `Status{400 | 413 | 422}` or `Stream` whose body matches an overflow signature — overflow checked BEFORE the 4xx-fatal rule.
- Overflow signatures (case-insensitive substrings, exactly these five): `"context length"`, `"context window"`, `"context size"`, `"too many tokens"`, `"prompt is too long"`.
- Backoff: `100ms · 2^(attempt-1)` capped at 5 s. No jitter, no Retry-After.
- Run-abort semantics unchanged: fatal/exhausted still returns `Err(AgentError::Model(..))` — the only addition is the `Done` emission.
- Behavior that must NOT change: cancel checks stay first and win; existing recovery tests (`transport_error_then_success_via_retry`, `stall_then_success_recovers_via_retry`) pass unchanged; `Scripted::Error` still yields `Http("scripted error")`.
- Final gate: `bash scripts/ci.sh` from the repo root.

---

### Task 1: Error taxonomy (`agent-model/src/types.rs`)

**Files:**
- Modify: `agent/crates/agent-model/src/types.rs` (ModelError at ~146-160; add tests in the file's test module, or create one if absent)

**Interfaces:**
- Consumes: existing `ModelError` enum.
- Produces (used by Tasks 2-3):
  - `ModelError::Cancelled` variant (`#[error("cancelled")]`)
  - `pub enum ErrorClass { Retryable, Fatal, ContextOverflow }` (`Debug, Clone, Copy, PartialEq, Eq`)
  - `impl ModelError { pub fn class(&self) -> ErrorClass }`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn class_table() {
        use ErrorClass::*;
        let cases: Vec<(ModelError, ErrorClass)> = vec![
            (ModelError::Http("connect refused".into()), Retryable),
            (ModelError::Stream("byte stream cut".into()), Retryable),
            (ModelError::Process("claude exited (1)".into()), Retryable),
            (ModelError::Timeout(std::time::Duration::from_secs(120)), Retryable),
            (ModelError::Status { code: 500, body: "oops".into() }, Retryable),
            (ModelError::Status { code: 503, body: "busy".into() }, Retryable),
            (ModelError::Status { code: 408, body: "timeout".into() }, Retryable),
            (ModelError::Status { code: 429, body: "rate limited".into() }, Retryable),
            (ModelError::Status { code: 400, body: "invalid request".into() }, Fatal),
            (ModelError::Status { code: 401, body: "bad key".into() }, Fatal),
            (ModelError::Status { code: 403, body: "forbidden".into() }, Fatal),
            (ModelError::Status { code: 404, body: "no such model".into() }, Fatal),
            (ModelError::Decode("not json".into()), Fatal),
            (ModelError::Cancelled, Fatal), // defensive: must never reach a retry arm
        ];
        for (e, want) in cases {
            assert_eq!(e.class(), want, "wrong class for {e}");
        }
    }

    #[test]
    fn overflow_is_detected_on_status_and_stream_bodies() {
        use ErrorClass::*;
        let overflowing = [
            "This model's maximum CONTEXT LENGTH is 8192 tokens",
            "the request exceeds the available context size",
            "context window exceeded",
            "too many tokens in prompt",
            "your prompt is too long",
        ];
        for body in overflowing {
            for code in [400u16, 413, 422] {
                let e = ModelError::Status { code, body: body.into() };
                assert_eq!(e.class(), ContextOverflow, "code {code}, body {body}");
            }
            let e = ModelError::Stream(format!("server error in stream: {body}"));
            assert_eq!(e.class(), ContextOverflow, "stream body {body}");
        }
    }

    #[test]
    fn overflow_signatures_are_conservative() {
        use ErrorClass::*;
        // Near-misses must NOT match: degrade to the plain class.
        let e = ModelError::Status { code: 400, body: "context deadline exceeded".into() };
        assert_eq!(e.class(), Fatal);
        let e = ModelError::Stream("context deadline exceeded".into());
        assert_eq!(e.class(), Retryable);
        // Overflow bodies on non-overflow codes keep their code's class.
        let e = ModelError::Status { code: 500, body: "context length exceeded".into() };
        assert_eq!(e.class(), Retryable);
        let e = ModelError::Status { code: 404, body: "context length exceeded".into() };
        assert_eq!(e.class(), Fatal);
    }
```

If `types.rs` has no `#[cfg(test)] mod tests`, add one (`use super::*;`).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-model class_table`
Expected: COMPILE ERROR — no `Cancelled` variant, no `ErrorClass`, no `class()`.

- [ ] **Step 3: Implement**

Add to the `ModelError` enum (after `Timeout`):

```rust
    /// The caller's cancel token fired mid-call. Never retried; the loop's
    /// token check is authoritative, this variant exists so cancellation is
    /// not spoofable as a plain stream-error string.
    #[error("cancelled")]
    Cancelled,
```

Below the enum:

```rust
/// How the agent loop should react to a model error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Transient: transport, stream, timeout, 5xx, 408/429. Retry with backoff.
    Retryable,
    /// Permanent request problem: other 4xx, decode. Abort on first sight.
    Fatal,
    /// The request exceeds the model's context. Retrying verbatim cannot
    /// succeed; the caller should shrink the context and rebuild once.
    ContextOverflow,
}

/// Case-insensitive overflow signature check. Conservative by design: a miss
/// degrades to the code's plain class, never to a wrong retry storm.
fn body_is_overflow(body: &str) -> bool {
    let b = body.to_ascii_lowercase();
    [
        "context length",
        "context window",
        "context size",
        "too many tokens",
        "prompt is too long",
    ]
    .iter()
    .any(|sig| b.contains(sig))
}

impl ModelError {
    /// Classify for the retry loop. Overflow is checked before the 4xx-fatal
    /// rule (overflow usually arrives as a 400).
    pub fn class(&self) -> ErrorClass {
        match self {
            ModelError::Status { code: 400 | 413 | 422, body } if body_is_overflow(body) => {
                ErrorClass::ContextOverflow
            }
            ModelError::Stream(body) if body_is_overflow(body) => ErrorClass::ContextOverflow,
            ModelError::Status { code: 408 | 429 | 500..=599, .. } => ErrorClass::Retryable,
            ModelError::Status { .. } | ModelError::Decode(_) | ModelError::Cancelled => {
                ErrorClass::Fatal
            }
            ModelError::Http(_)
            | ModelError::Stream(_)
            | ModelError::Process(_)
            | ModelError::Timeout(_) => ErrorClass::Retryable,
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agent-model && cargo build`
Expected: PASS; whole workspace still builds (the new variant is additive — nothing matches exhaustively on `ModelError` outside `class()`; if the build says otherwise, add the arm the compiler names).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-model/src/types.rs
git commit -m "feat(model): ModelError classification — ErrorClass + overflow detection + Cancelled variant"
```

---

### Task 2: Classified retry loop + backoff + testkit (`agent-core`)

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` (`one_completion` cancel sites ~137/151; `completion_with_retry` ~195-221; turn-loop match ~295-302; tests)
- Modify: `agent/crates/agent-core/src/testkit.rs` (Scripted enum ~12-35, stream() match ~60)

**Interfaces:**
- Consumes (Task 1): `ModelError::Cancelled`, `ErrorClass`, `ModelError::class()`.
- Produces (used by Task 3):
  - loop-private `enum RetryFailure { Fatal(String), Cancelled, Overflow(String) }`
  - `async fn completion_with_retry(&self, base: &CompletionRequest, cancel: &CancellationToken) -> Result<AssistantTurn, RetryFailure>`
  - `fn backoff_delay(attempt: usize) -> Duration` (free fn in loop_.rs)
  - testkit `Scripted::Fail(ModelError)`
  - turn loop: `Fatal` → emit `Done(StopReason::Error)` + `return Err(AgentError::Model(msg))`; `Overflow` → temporarily same as Fatal (Task 3 replaces with recovery)

- [ ] **Step 1: testkit — add `Scripted::Fail`**

In the `Scripted` enum:

```rust
    /// Force a specific model error this turn (classification-aware tests).
    Fail(ModelError),
```

In `ScriptedModel::stream`'s match, next to `Scripted::Error`:

```rust
            Scripted::Fail(e) => Err(e),
```

- [ ] **Step 2: Write the failing loop tests** (in `loop_.rs`'s test module; mirror the existing test scaffolding — the file's tests build an `AgentLoop` with `ScriptedModel`, `CollectingSink`, `WindowContext`, and a `LoopConfig`; copy the setup of `transport_error_then_success_via_retry` (~1425) and adjust)

```rust
    #[tokio::test]
    async fn fatal_400_fails_fast_without_retry() {
        // One scripted 400; a Text follow-up that must NEVER be consulted.
        let model = std::sync::Arc::new(ScriptedModel::new(vec![
            Scripted::Fail(ModelError::Status { code: 400, body: "invalid request".into() }),
            Scripted::Text("should not be reached".into()),
        ]));
        /* build loop as in transport_error_then_success_via_retry, with
           max_retries: 3, and keep a handle to the CollectingSink */
        let mut ctx = WindowContext::new(Message::system("sys"));
        let err = agent.run(&mut ctx, "go".into()).await.unwrap_err();
        assert!(matches!(err, AgentError::Model(_)));
        let kinds = sink.kinds(); // or however neighboring tests read events
        assert!(kinds.iter().any(|k| k.starts_with("error")));
        assert_eq!(kinds.last().map(String::as_str), Some("done"));
        // the second scripted turn is still queued: the model was consulted once
        assert_eq!(model.remaining(), 1);
    }

    #[tokio::test]
    async fn rate_limit_429_is_retried_then_succeeds() {
        let model = std::sync::Arc::new(ScriptedModel::new(vec![
            Scripted::Fail(ModelError::Status { code: 429, body: "rate limited".into() }),
            Scripted::Text("recovered".into()),
        ]));
        /* same scaffolding, max_retries: 3 */
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert_eq!(sink.kinds().last().map(String::as_str), Some("done"));
    }

    #[tokio::test]
    async fn exhaustion_emits_done_error() {
        // All-retryable failures burn max_retries then abort WITH a Done.
        let model = std::sync::Arc::new(ScriptedModel::new(vec![
            Scripted::Fail(ModelError::Http("down".into())),
            Scripted::Fail(ModelError::Http("down".into())),
            Scripted::Fail(ModelError::Http("down".into())),
        ]));
        /* scaffolding with max_retries: 2 */
        let err = agent.run(&mut ctx, "go".into()).await.unwrap_err();
        assert!(matches!(err, AgentError::Model(_)));
        assert_eq!(sink.kinds().last().map(String::as_str), Some("done"));
    }

    #[test]
    fn backoff_is_exponential_and_capped() {
        assert_eq!(backoff_delay(1), Duration::from_millis(100));
        assert_eq!(backoff_delay(2), Duration::from_millis(200));
        assert_eq!(backoff_delay(3), Duration::from_millis(400));
        assert_eq!(backoff_delay(7), Duration::from_millis(5_000)); // 6400 capped
        assert_eq!(backoff_delay(60), Duration::from_millis(5_000)); // no overflow
    }
```

Notes for the implementer: (a) if `ScriptedModel` has no `remaining()` helper, add one (`self.turns.lock().unwrap().len()`); (b) `sink.kinds()` — use whatever accessor the neighboring tests use on `CollectingSink` (they match on event strings; follow that pattern exactly); (c) the fatal test's `done` must carry `StopReason::Error` — if the sink records reasons, assert it; if it only records kinds, asserting the trailing `done` is sufficient here because Task 4 adds the wire-level reason test.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p agent-core fatal_400`
Expected: COMPILE ERROR (`Scripted::Fail`, `backoff_delay` missing) — then after stubs, behavioral failure: 400 retried, no trailing `done`.

- [ ] **Step 4: Implement**

`one_completion` cancel sites (lines ~137 and ~151): replace both
`return Err(ModelError::Stream("cancelled".into()))` with
`return Err(ModelError::Cancelled)`.

Free function near `DEFAULT_MAX_PARALLEL_TOOLS`:

```rust
/// Exponential retry backoff: 100ms · 2^(attempt-1), capped at 5s.
fn backoff_delay(attempt: usize) -> Duration {
    let exp = (attempt.saturating_sub(1)).min(16) as u32; // 100ms << 16 is already > cap
    Duration::from_millis((100u64 << exp).min(5_000))
}
```

Loop-private enum above `impl AgentLoop` (or beside `AgentError` usage):

```rust
/// Why `completion_with_retry` gave up. Loop-private: the turn loop maps
/// these onto events + `AgentError`.
enum RetryFailure {
    /// Fatal on first sight, or retryable and retries exhausted.
    Fatal(String),
    /// Cancellation observed (token or `ModelError::Cancelled`).
    Cancelled,
    /// Context overflow: the same request can never succeed. Not counted
    /// against max_retries; the turn loop may compact-rebuild-retry once.
    Overflow(String),
}
```

Rewrite `completion_with_retry`:

```rust
    /// Stream with classified retry: transient errors retry with exponential
    /// backoff; permanent request errors fail on first sight; context
    /// overflow is deferred to the turn loop (retrying verbatim cannot help).
    async fn completion_with_retry(
        &self,
        base: &CompletionRequest,
        cancel: &CancellationToken,
    ) -> Result<AssistantTurn, RetryFailure> {
        let mut attempt = 0;
        loop {
            let mut req = base.clone();
            self.protocol.prepare(&mut req);
            match self.one_completion(req, cancel).await {
                Ok(turn) => return Ok(turn),
                Err(ModelError::Cancelled) => return Err(RetryFailure::Cancelled),
                Err(e) => {
                    if cancel.is_cancelled() {
                        return Err(RetryFailure::Cancelled);
                    }
                    match e.class() {
                        ErrorClass::ContextOverflow => {
                            tracing::warn!(error = %e,
                                "context overflow; deferring to turn-level recovery");
                            return Err(RetryFailure::Overflow(e.to_string()));
                        }
                        ErrorClass::Fatal => {
                            self.sink.emit(AgentEvent::Error(e.to_string()));
                            return Err(RetryFailure::Fatal(e.to_string()));
                        }
                        ErrorClass::Retryable => {
                            attempt += 1;
                            if attempt > self.config.max_retries {
                                self.sink.emit(AgentEvent::Error(e.to_string()));
                                return Err(RetryFailure::Fatal(e.to_string()));
                            }
                            tracing::warn!(attempt, error = %e, "model error, retrying");
                            tokio::time::sleep(backoff_delay(attempt)).await;
                        }
                    }
                }
            }
        }
    }
```

Turn-loop call site (~295-302) — Task 3 extends the Overflow arm; for THIS task:

```rust
            let assistant = match self.completion_with_retry(&base, &cancel).await {
                Ok(t) => t,
                Err(RetryFailure::Cancelled) => {
                    self.sink.emit(AgentEvent::Done(StopReason::Cancelled));
                    return Ok(());
                }
                Err(RetryFailure::Fatal(msg) | RetryFailure::Overflow(msg)) => {
                    self.sink.emit(AgentEvent::Done(StopReason::Error));
                    return Err(AgentError::Model(msg));
                }
            };
```

Wait — `StopReason::Error` does not exist until Task 4? NO: it is required here. **Add it in this task** (it belongs to whichever task first emits it): in `agent-model/src/types.rs`, add `Error,` to `StopReason` (~line 95, after `Cancelled`), and in `agent-server/src/wire.rs` `stop_reason_str` (~150-158) add `StopReason::Error => "error",`. (Task 4 adds the remaining emission sites + the wire test.) Note for the Overflow arm: without Task 3 it degrades to fatal-with-Done — still strictly better than today, and `exhaustion_emits_done_error` covers the shape. Imports: add `ErrorClass` to the `agent_model` use list in loop_.rs.

Update the existing exhaustion test `idle_stall_times_out_and_fails_after_retries` (~1505): add a trailing-`done` assertion matching `exhaustion_emits_done_error`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p agent-core && cargo test -p agent-model && cargo build && cargo test -p agent-server`
Expected: PASS — new tests plus `transport_error_then_success_via_retry` / `stall_then_success_recovers_via_retry` unchanged; whole workspace builds with the new `StopReason` variant (the wire arm was added; if any other exhaustive `StopReason` match exists the compiler will name it — add the obvious arm).

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs agent/crates/agent-core/src/testkit.rs agent/crates/agent-model/src/types.rs agent/crates/agent-server/src/wire.rs
git commit -m "feat(core): classified retry loop — fail fast on fatal, exponential backoff, Done on abort"
```

---

### Task 3: Turn-level overflow recovery (`agent-core`)

**Files:**
- Modify: `agent/crates/agent-core/src/context.rs` (ContextManager trait, ~157-172)
- Modify: `agent/crates/agent-core/src/curated.rs` (override + test)
- Modify: `agent/crates/agent-core/src/loop_.rs` (turn loop; request-build helper; tests)

**Interfaces:**
- Consumes (Task 2): `RetryFailure::Overflow`, the turn-loop match shape.
- Produces:
  - `ContextManager::request_compaction(&mut self)` — provided method, default no-op
  - `CuratedContext` override setting its `compact_flag`
  - loop helper `fn completion_request(&self, messages: Vec<Message>, preserve_thinking: bool) -> CompletionRequest`
  - turn loop: first `Overflow` per turn → compact + rebuild + retry once (no retry budget consumed); second → fatal path from Task 2

- [ ] **Step 1: Write the failing tests**

`curated.rs` (mirror the existing compaction test that scripts a summarizer response — find the test using `ScriptedModel` + `compact_flag`; follow its harness):

```rust
    #[tokio::test]
    async fn request_compaction_takes_the_compaction_path_on_next_maintain() {
        /* build a CuratedContext exactly like the existing explicit-compaction
           test (history below high-water so ONLY the flag can trigger it),
           but instead of flag.store(true, ..) call: */
        ctx.request_compaction();
        let report = ctx.maintain(&deps).await;
        assert!(report.compacted_turns > 0);
    }
```

`loop_.rs` — a minimal recording context stub in the test module:

```rust
    /// Context stub for overflow recovery: counts request_compaction calls,
    /// and after the first one build() returns a shrunk history.
    struct OverflowCtx {
        history: Vec<Message>,
        compaction_requests: usize,
        maintains: usize,
    }
    #[async_trait::async_trait]
    impl ContextManager for OverflowCtx {
        fn append(&mut self, m: Message) { self.history.push(m); }
        fn set_system(&mut self, _: Message) {}
        fn set_recall(&mut self, _: Vec<String>) {}
        fn set_goal(&mut self, _: String) {}
        fn build(&self, _limit: usize) -> Vec<Message> {
            if self.compaction_requests > 0 {
                self.history.iter().take(1).cloned().collect() // "shrunk"
            } else {
                self.history.clone()
            }
        }
        async fn maintain(&mut self, _deps: &MaintCtx<'_>) -> MaintReport {
            self.maintains += 1;
            MaintReport::default()
        }
        fn request_compaction(&mut self) { self.compaction_requests += 1; }
    }

    #[tokio::test]
    async fn overflow_compacts_rebuilds_and_recovers_once() {
        let model = std::sync::Arc::new(ScriptedModel::new(vec![
            Scripted::Fail(ModelError::Status {
                code: 400, body: "maximum context length exceeded".into() }),
            Scripted::Text("recovered after compaction".into()),
        ]));
        /* loop scaffolding as in Task 2's tests, max_retries: 0 —
           proving overflow recovery does NOT consume retry budget */
        let mut ctx = OverflowCtx { history: vec![], compaction_requests: 0, maintains: 0 };
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert_eq!(ctx.compaction_requests, 1);
        assert!(ctx.maintains >= 1);
        assert_eq!(sink.kinds().last().map(String::as_str), Some("done"));
    }

    #[tokio::test]
    async fn second_overflow_in_a_turn_is_fatal() {
        let overflow = || Scripted::Fail(ModelError::Status {
            code: 400, body: "maximum context length exceeded".into() });
        let model = std::sync::Arc::new(ScriptedModel::new(vec![overflow(), overflow()]));
        /* same scaffolding, max_retries: 3 (unused — overflow skips budget) */
        let mut ctx = OverflowCtx { history: vec![], compaction_requests: 0, maintains: 0 };
        let err = agent.run(&mut ctx, "go".into()).await.unwrap_err();
        assert!(matches!(err, AgentError::Model(_)));
        assert_eq!(ctx.compaction_requests, 1, "recovery attempted exactly once");
        assert_eq!(sink.kinds().last().map(String::as_str), Some("done"));
        assert!(sink.kinds().iter().any(|k| k.starts_with("error")));
    }
```

(Check `ContextManager`'s exact trait items in context.rs before implementing the stub — if the trait has items not listed here, add trivial impls following `WindowContext`.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p agent-core overflow_compacts`
Expected: COMPILE ERROR — `request_compaction` not on the trait.

- [ ] **Step 3: Implement**

`context.rs`, in `trait ContextManager` (provided method):

```rust
    /// Ask the manager to compact on its next maintenance pass. Default: no-op
    /// (managers without a compaction concept ignore it). `CuratedContext`
    /// sets the same flag the `context_compact` tool uses.
    fn request_compaction(&mut self) {}
```

`curated.rs`, inside `impl ContextManager for CuratedContext`:

```rust
    fn request_compaction(&mut self) {
        self.compact_flag.store(true, Ordering::SeqCst);
    }
```

`loop_.rs` — factor the request build (turn prologue ~281-293) into:

```rust
    /// One place a built message list becomes a CompletionRequest (the turn
    /// prologue and the overflow-recovery rebuild must not drift apart).
    fn completion_request(
        &self,
        messages: Vec<Message>,
        preserve_thinking: bool,
    ) -> CompletionRequest {
        CompletionRequest {
            messages,
            tools: self.tools.schemas(),
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
            top_p: self.config.top_p,
            top_k: self.config.top_k,
            min_p: self.config.min_p,
            presence_penalty: self.config.presence_penalty,
            repeat_penalty: self.config.repeat_penalty,
            enable_thinking: self.config.enable_thinking,
            preserve_thinking,
        }
    }
```

Turn prologue: `let mut base = self.completion_request(messages, preserve_thinking);`

Replace the Task-2 call-site match with the recovery loop:

```rust
            let mut overflow_recovered = false;
            let assistant = loop {
                match self.completion_with_retry(&base, &cancel).await {
                    Ok(t) => break t,
                    Err(RetryFailure::Cancelled) => {
                        self.sink.emit(AgentEvent::Done(StopReason::Cancelled));
                        return Ok(());
                    }
                    Err(RetryFailure::Overflow(_)) if !overflow_recovered => {
                        overflow_recovered = true;
                        tracing::warn!("context overflow: forcing compaction and rebuilding once");
                        ctx.request_compaction();
                        let deps = crate::MaintCtx {
                            model_limit: self.config.model_limit,
                            model: &self.model,
                            sink: &self.sink,
                            cancel: &cancel,
                        };
                        ctx.maintain(&deps).await;
                        let messages = ctx.build(self.config.model_limit);
                        base = self.completion_request(messages, preserve_thinking);
                    }
                    Err(RetryFailure::Overflow(msg)) => {
                        self.sink.emit(AgentEvent::Error(msg.clone()));
                        self.sink.emit(AgentEvent::Done(StopReason::Error));
                        return Err(AgentError::Model(msg));
                    }
                    Err(RetryFailure::Fatal(msg)) => {
                        self.sink.emit(AgentEvent::Done(StopReason::Error));
                        return Err(AgentError::Model(msg));
                    }
                }
            };
```

(Note the second-`Overflow` arm emits `Error` here — `completion_with_retry` only `tracing::warn!`s for overflow; `Fatal`'s `Error` was already emitted inside. This asymmetry is per spec Section 2.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agent-core`
Expected: PASS — new tests plus every existing loop/curated test.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/context.rs agent/crates/agent-core/src/curated.rs agent/crates/agent-core/src/loop_.rs
git commit -m "feat(core): context-overflow recovery — force compaction and rebuild once per turn"
```

---

### Task 4: Done parity on the remaining terminal paths (`agent-core`, `agent-server`)

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` (max_tokens abort ~323-331; protocol-repair-exhausted ~340-343; tests)
- Modify: `agent/crates/agent-server/src/wire.rs` (test only — the `"error"` arm landed in Task 2)

**Interfaces:**
- Consumes (Task 2): `StopReason::Error` + wire arm.
- Produces: every terminal path of `run_with_cancel` emits `Done(..)`.

- [ ] **Step 1: Write the failing tests**

`loop_.rs` — find the existing test covering the max_tokens/`TruncatedCall` path (grep `TruncatedCall` in the test module) and the protocol-repair-exhausted path (grep `repair`); extend each to assert the trailing `done`. If either path has no test, add one:

```rust
    #[tokio::test]
    async fn max_tokens_truncation_emits_done_length() {
        let model = std::sync::Arc::new(ScriptedModel::new(vec![
            Scripted::TruncatedCall("write_file".into(), "{\"path\": \"big".into()),
        ]));
        /* standard scaffolding */
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let kinds = sink.kinds();
        assert!(kinds.iter().any(|k| k.starts_with("error")));
        assert_eq!(kinds.last().map(String::as_str), Some("done"));
    }
```

(A protocol-repair-exhausted turn needs two unparseable turns in a row under the prompted protocol or malformed native calls — mirror however the existing repair test provokes the first parse failure, scripting it twice.)

`wire.rs` — beside the existing stop-reason mapping test:

```rust
    #[test]
    fn stop_reason_error_maps_to_error() {
        assert_eq!(stop_reason_str(&StopReason::Error), "error");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p agent-core max_tokens_truncation && cargo test -p agent-server stop_reason`
Expected: loop test FAILS (no trailing `done`); wire test PASSES already (arm landed in Task 2 — keep it as the pin).

- [ ] **Step 3: Implement**

max_tokens abort (~323-331): before `return Ok(())` add

```rust
                    self.sink.emit(AgentEvent::Done(StopReason::Length));
```

protocol-repair exhausted (~340-343): before `return Ok(())` add

```rust
                    self.sink.emit(AgentEvent::Done(StopReason::Error));
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agent-core && cargo test -p agent-server`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs agent/crates/agent-server/src/wire.rs
git commit -m "feat(core): emit Done on max_tokens and protocol-repair terminal paths"
```

---

### Task 5: Full gate + spec status

**Files:**
- Modify: `docs/superpowers/specs/2026-07-01-retry-classification-design.md` (Status line)

- [ ] **Step 1: Run the full CI gate**

Run: `cd /home/kalen/rust-agent-runtime && bash scripts/ci.sh`
Expected: fmt, clippy, all agent tests, web typecheck + vitest — green. Fix anything flagged.

- [ ] **Step 2: Update the spec status**

Change `**Status:**` to `Implemented (this plan: docs/superpowers/plans/2026-07-01-retry-classification.md)`.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-07-01-retry-classification-design.md
git commit -m "docs(spec): mark retry classification spec implemented"
```
