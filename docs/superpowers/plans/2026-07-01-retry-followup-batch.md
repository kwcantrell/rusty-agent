# Retry Follow-Up Batch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the merged retry-classification cluster's accepted residuals: claude-cli (`Process`) overflow now recovers, recovery is observable as a real event with a fresh `Usage`, `AgentError::Cancelled` dead code is gone, and retry backoff is pinned in-situ on a paused clock.

**Architecture:** Four small changes along the existing retry/observability seams. (1) One guard arm in `ModelError::class()` signature-checks `Process` bodies like `Stream`, plus a dated correction to the original spec. (2) The overflow-recovery arm in `loop_.rs` emits a new payload-free `ContextEvent::OverflowRecovery` before maintenance and re-emits `AgentEvent::Usage` after the rebuild; the compiler surfaces the four exhaustive `ContextEvent` matches (wire, trace, CLI render, testkit sink) and web gets a cosmetic case. (3) Delete the never-constructed `AgentError::Cancelled`. (4) Convert the last real-sleep retry test to `start_paused` and add a virtual-elapsed backoff-growth pin.

**Tech Stack:** Rust (Cargo workspace under `agent/`, NOT `src-tauri/`), tokio paused-clock tests, TypeScript (web/src/state.ts, one switch case).

**Spec:** `docs/superpowers/specs/2026-07-01-retry-followup-batch-design.md`

## Global Constraints

- Work from repo root `/home/kalen/rust-agent-runtime`; Rust commands need `source ~/.cargo/env` and run inside `agent/`.
- Conventional commits: `type(scope): summary`.
- Invariant (spec): overflow recovery fires on every backend (claude-cli included) and is observable on every surface (event, not just tracing); after a mid-turn rebuild the turn's `Usage` reflects the rebuilt request; retry timing is pinned in-situ without real sleeps; `AgentError` carries no dead variants.
- `body_is_overflow` (the five signatures) is NOT modified — `Process` reuses it verbatim.
- Out of scope (do NOT touch): Retry-After/jitter, server-usage-calibrated budgeting, recovery metrics beyond the single event.
- Final gate: `bash scripts/ci.sh` green.

---

### Task 1: `Process` overflow arm + end-to-end pin + spec correction

**Files:**
- Modify: `agent/crates/agent-model/src/types.rs:198-219` (`class()` + doc comment) and its tests module (~lines 222-378)
- Modify: `agent/crates/agent-core/src/loop_.rs` tests (the overflow test block near `overflow_compacts_rebuilds_and_recovers_once`, ~line 3463)
- Modify: `docs/superpowers/specs/2026-07-01-retry-classification-design.md:255-257` (dated correction)

**Interfaces:**
- Consumes: existing `body_is_overflow(&str) -> bool` (types.rs:184), `ErrorClass::ContextOverflow`, testkit `Scripted::Fail(ModelError)`, the in-file `OverflowCtx` test context.
- Produces: `ModelError::Process(body).class() == ContextOverflow` when the body matches an overflow signature; Retryable otherwise (unchanged).

- [ ] **Step 1: Write the failing agent-model test**

Append inside the existing tests module in `agent/crates/agent-model/src/types.rs` (mirror the file's existing test imports — `class_table` already uses the `ErrorClass` variants unqualified or via the module's `use`):

```rust
    #[test]
    fn overflow_is_detected_on_process_bodies() {
        // claude-cli surfaces model errors as Process("claude exited (1): <stderr>").
        for body in [
            "claude exited (1): This model's maximum CONTEXT LENGTH is 8192 tokens",
            "claude exited (1): your prompt is too long",
        ] {
            assert_eq!(
                ModelError::Process(body.into()).class(),
                ErrorClass::ContextOverflow,
                "expected overflow for {body:?}"
            );
        }
        // Conservative: near-miss stays Retryable.
        assert_eq!(
            ModelError::Process("claude exited (1): context deadline exceeded".into()).class(),
            ErrorClass::Retryable
        );
        // Spawn-style bodies without signatures stay Retryable.
        assert_eq!(
            ModelError::Process("spawn claude: No such file or directory".into()).class(),
            ErrorClass::Retryable
        );
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-model overflow_is_detected_on_process`
Expected: FAIL — the first assertion gets `Retryable` (Process is currently unconditionally Retryable at types.rs:216).

- [ ] **Step 3: Add the guard arm**

In `class()` (types.rs), directly below the existing `Stream` overflow guard:

```rust
        ModelError::Stream(body) if body_is_overflow(body) => ErrorClass::ContextOverflow,
        ModelError::Process(body) if body_is_overflow(body) => ErrorClass::ContextOverflow,
```

(the later `ModelError::Process(_)` in the Retryable group stays — it now catches only non-overflow bodies). Update the `class()` doc comment (currently "Classify for the retry loop. Overflow is checked before the 4xx-fatal rule (overflow usually arrives as a 400).") to:

```rust
    /// Classify for the retry loop. Overflow is checked before the 4xx-fatal
    /// rule (overflow usually arrives as a 400); `Status{400|413|422}`,
    /// `Stream`, and `Process` bodies are all signature-checked — the
    /// claude-cli backend surfaces overflow as `Process` stderr text.
```

- [ ] **Step 4: Run agent-model tests**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-model`
Expected: PASS — new test plus `class_table` (its `Process("claude exited (1)") → Retryable` row still holds: no signature in that body) and both existing overflow tests.

- [ ] **Step 5: Write the failing end-to-end loop test**

Append in `agent/crates/agent-core/src/loop_.rs`, inside the same test module as `overflow_compacts_rebuilds_and_recovers_once` (so the `OverflowCtx` struct defined just above it is in scope):

```rust
    #[tokio::test]
    async fn process_overflow_recovers_like_status_overflow() {
        // claude-cli surfaces overflow as Process stderr text (no status code);
        // recovery must fire exactly as it does for Status{400}.
        let ws = std::env::temp_dir();
        let model = std::sync::Arc::new(ScriptedModel::new(vec![
            Scripted::Fail(ModelError::Process(
                "claude exited (1): maximum context length exceeded".into(),
            )),
            Scripted::Text("recovered after compaction".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 0, // recovery must not consume retry budget
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = OverflowCtx {
            history: vec![],
            compaction_requests: 0,
            maintains: 0,
        };
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert_eq!(ctx.compaction_requests, 1);
        assert_eq!(
            sink.events.lock().unwrap().last().map(String::as_str),
            Some("done")
        );
    }
```

- [ ] **Step 6: Run it — expect PASS (classification drives the loop)**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-core process_overflow_recovers`
Expected: PASS immediately — the loop matches on `RetryFailure::Overflow`, which is classification-driven; Step 3 made Process bodies classify as overflow. (This test is the pin, not a RED/GREEN cycle: its RED state was Step 2's crate-level RED. If it FAILS, stop — the loop has a second classification site; investigate before proceeding.)

- [ ] **Step 7: Correct the original spec (dated)**

In `docs/superpowers/specs/2026-07-01-retry-classification-design.md`, find:

```
**claude-cli backend**: overflow surfaces as Process(..)/Stream(..) without a status code
— only the Stream body signature check can catch it; a miss retries as today (no regression).
```

Append to that bullet (same paragraph):

```
*[Correction 2026-07-01, retry follow-up batch: `Process` bodies carry the CLI's stderr
text and are now signature-checked exactly like `Stream` — see
`2026-07-01-retry-followup-batch-design.md`.]*
```

- [ ] **Step 8: Commit**

```bash
git add agent/crates/agent-model/src/types.rs agent/crates/agent-core/src/loop_.rs docs/superpowers/specs/2026-07-01-retry-classification-design.md
git commit -m "fix(model): signature-check Process bodies for overflow — claude-cli overflow now recovers"
```

---

### Task 2: `ContextEvent::OverflowRecovery` + post-rebuild `Usage` re-emit

**Files:**
- Modify: `agent/crates/agent-core/src/event.rs:5-28` (ContextEvent enum)
- Modify: `agent/crates/agent-core/src/loop_.rs:347-360` (overflow-recovery arm) + the two overflow tests
- Modify: `agent/crates/agent-core/src/testkit.rs` (~line 191, CollectingSink Context arms)
- Modify: `agent/crates/agent-server/src/wire.rs:198-229` (match) + `:370-401` (`context_events_are_forwarded` test)
- Modify: `agent/crates/agent-cli/src/render.rs:117-129` (Context match)
- Modify: `agent/crates/agent-runtime-config/src/trace.rs:258-282` (Context match)
- Modify: `web/src/state.ts:54-63` (`describeContext`)

**Interfaces:**
- Consumes: existing `AgentEvent::Usage` emission shape (loop_.rs:331-336: `prompt_tokens: built_tokens(&messages), context_limit: self.config.model_limit, turn: turn + 1, max_turns: self.config.max_turns`); CollectingSink label conventions (`usage:{prompt_tokens}`, kebab labels per event).
- Produces: unit variant `ContextEvent::OverflowRecovery`; wire/trace kind string `"overflow_recovery"` with `detail: {}`; CollectingSink label `"overflow_recovery"`; a second `Usage` event after the recovery rebuild.

- [ ] **Step 1: Write the failing test (extend the existing recovery test)**

In `agent/crates/agent-core/src/loop_.rs`, extend `overflow_compacts_rebuilds_and_recovers_once` — replace its final assertion block:

```rust
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert_eq!(ctx.compaction_requests, 1);
        assert!(ctx.maintains >= 1);
        let events = sink.events.lock().unwrap().clone();
        assert!(
            events.iter().any(|e| e == "overflow_recovery"),
            "recovery must be observable as a context event: {events:?}"
        );
        let usages: Vec<&String> = events.iter().filter(|e| e.starts_with("usage:")).collect();
        assert!(
            usages.len() >= 2,
            "expected pre-request + post-rebuild Usage events: {events:?}"
        );
        assert_eq!(events.last().map(String::as_str), Some("done"));
```

- [ ] **Step 2: Run it to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-core overflow_compacts_rebuilds`
Expected: FAIL — no `overflow_recovery` label exists and only one `usage:` event is emitted.

- [ ] **Step 3: Add the variant**

In `agent/crates/agent-core/src/event.rs`, append to `ContextEvent`:

```rust
    /// The model reported context overflow; the loop forced compaction and
    /// rebuilt the request. Emitted BEFORE maintenance runs, so it fires even
    /// when compaction no-ops (`Compacted`/`CompactionFailed` then narrate the
    /// maintenance outcome).
    OverflowRecovery,
```

- [ ] **Step 4: Emit event + re-emit Usage in the recovery arm**

In `agent/crates/agent-core/src/loop_.rs` (the `Err(RetryFailure::Overflow(_)) if !overflow_recovered` arm), keep the `tracing::warn!` and update to:

```rust
                Err(RetryFailure::Overflow(_)) if !overflow_recovered => {
                    overflow_recovered = true;
                    tracing::warn!("context overflow: forcing compaction and rebuilding once");
                    self.sink
                        .emit(AgentEvent::Context(crate::ContextEvent::OverflowRecovery));
                    ctx.request_compaction();
                    let deps = crate::MaintCtx {
                        model_limit: self.config.model_limit,
                        model: &self.model,
                        sink: &self.sink,
                        cancel: &cancel,
                    };
                    ctx.maintain(&deps).await;
                    let messages = ctx.build(self.config.model_limit);
                    // The pre-request Usage is stale after compaction; re-emit so
                    // every surface sees the rebuilt request's estimate (latest wins).
                    self.sink.emit(AgentEvent::Usage {
                        prompt_tokens: built_tokens(&messages),
                        context_limit: self.config.model_limit,
                        turn: turn + 1,
                        max_turns: self.config.max_turns,
                    });
                    base = self.completion_request(messages, preserve_thinking);
                }
```

(If the surrounding scope names differ — e.g. the turn variable — mirror the pre-request `Usage` emission at ~line 331 exactly; it is the same function.)

- [ ] **Step 5: Add the four compiler-surfaced match arms**

Build to find them all: `cargo build 2>&1 | grep -A2 non-exhaustive` — expected sites:

`agent/crates/agent-core/src/testkit.rs` (CollectingSink):
```rust
            AgentEvent::Context(ContextEvent::OverflowRecovery) => "overflow_recovery".into(),
```

`agent/crates/agent-server/src/wire.rs` (Context match):
```rust
                CE::OverflowRecovery => ("overflow_recovery", serde_json::json!({})),
```

`agent/crates/agent-cli/src/render.rs` (Context match):
```rust
                    CE::OverflowRecovery =>
                        "⟲ context overflow: compacted and retried".to_string(),
```

`agent/crates/agent-runtime-config/src/trace.rs` (Context match):
```rust
            ContextEvent::OverflowRecovery => TraceEvent::Context {
                kind: "overflow_recovery",
                detail: serde_json::json!({}),
            },
```

- [ ] **Step 6: Extend the wire kind test**

In `agent/crates/agent-server/src/wire.rs`, `context_events_are_forwarded` (~line 370) iterates `for (ev, kind) in [...]` — add one entry to the array:

```rust
            (CE::OverflowRecovery, "overflow_recovery"),
```

(match the array's existing tuple construction style exactly).

- [ ] **Step 7: Add the web case**

In `web/src/state.ts`, `describeContext` switch, before `default`:

```typescript
    case "overflow_recovery":
      return "context overflow: compacted and retried";
```

- [ ] **Step 8: Run the covering tests**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-core && cargo test -p agent-server && cargo test -p agent-runtime-config && cargo test -p agent-cli`
Then: `cd ../web && npm run typecheck && npm test`
Expected: all PASS, including the Step-1 assertions and the extended wire test.

- [ ] **Step 9: Commit**

```bash
git add agent/crates/agent-core/src/event.rs agent/crates/agent-core/src/loop_.rs agent/crates/agent-core/src/testkit.rs agent/crates/agent-server/src/wire.rs agent/crates/agent-cli/src/render.rs agent/crates/agent-runtime-config/src/trace.rs web/src/state.ts
git commit -m "feat(core): overflow recovery emits OverflowRecovery event + fresh Usage after rebuild"
```

---

### Task 3: Delete `AgentError::Cancelled`

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs:15-21`

**Interfaces:**
- Consumes: nothing.
- Produces: `AgentError` with the single `Model(String)` variant (all existing matches use `AgentError::Model(_)` and stay valid; `ModelError::Cancelled` + `RetryFailure::Cancelled` remain the real cancellation encoding).

- [ ] **Step 1: Delete the variant**

```rust
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("model error after retries: {0}")]
    Model(String),
}
```

(Verified pre-plan: zero construction sites for `Cancelled`, no serde derives, no exhaustive matches — the enum stays an enum per spec.)

- [ ] **Step 2: Prove nothing breaks**

Run: `source ~/.cargo/env && cd agent && cargo build && cargo test -p agent-core && grep -rn "AgentError::Cancelled" crates/ | wc -l`
Expected: build + tests green; grep prints `0`.

- [ ] **Step 3: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs
git commit -m "refactor(core): drop dead AgentError::Cancelled (ModelError::Cancelled is the real encoding)"
```

---

### Task 4: Paused-clock retry tests + in-situ backoff pin

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` tests (`transport_error_then_success_via_retry` ~line 1547; new test alongside it)

**Interfaces:**
- Consumes: `Scripted::Error` (transport-style retryable failure), `backoff_delay` semantics (100ms·2^(n−1), 5s cap), the in-file `start_paused` pattern (`idle_stall_times_out_and_fails_after_retries`, line 1756).
- Produces: no production change; the loop's per-attempt sleeping is pinned in virtual time.

- [ ] **Step 1: Convert the real-sleep test**

Change `transport_error_then_success_via_retry`'s attribute from `#[tokio::test]` to `#[tokio::test(start_paused = true)]` (body unchanged — auto-advance makes the 100 ms backoff sleep instant in wall-clock). Sweep the retry test block for any other retry test that scripts failures without `start_paused` and drives real backoff sleeps; convert those the same way (the timeout trio at lines 1756/1802/1842 is already paused).

- [ ] **Step 2: Write the in-situ backoff-growth pin**

Add next to it:

```rust
    #[tokio::test(start_paused = true)]
    async fn retry_backoff_sleeps_grow_exponentially_in_situ() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Error,
            Scripted::Error,
            Scripted::Error,
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 3,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        let start = tokio::time::Instant::now();
        agent.run(&mut ctx, "go".into()).await.unwrap();
        // Paused clock: virtual elapsed is EXACTLY the loop's backoff sleeps —
        // three failures -> backoff_delay(1..=3) = 100 + 200 + 400 ms. This pins
        // that the LOOP sleeps the schedule, which the pure backoff_delay unit
        // test cannot.
        assert_eq!(start.elapsed(), std::time::Duration::from_millis(700));
        assert_eq!(sink.events.lock().unwrap().last().unwrap(), "done");
    }
```

- [ ] **Step 3: Run the retry tests**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-core retry && cargo test -p agent-core transport_error`
Expected: PASS, and noticeably faster than before (no real 100 ms sleep). If the elapsed assertion fails, print the actual value and investigate what other timer advanced the clock (systematic-debugging) — do NOT loosen the assertion without identifying the source.

- [ ] **Step 4: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs
git commit -m "test(core): paused-clock retry tests + in-situ exponential backoff pin"
```

---

### Task 5: Workspace sweep + full CI gate

**Files:**
- Possibly modify: any test/fixture broken by the new event variant or the Usage re-emit (e.g. tests counting `usage:` events, trace/wire snapshot tests).

**Interfaces:**
- Consumes: everything above.
- Produces: green workspace + green `scripts/ci.sh`.

- [ ] **Step 1: Full workspace test run**

Run: `source ~/.cargo/env && cd agent && cargo test`
Expected: green. Anticipated fallout class: a test asserting an exact event sequence around overflow recovery now sees `overflow_recovery` + a second `usage:` label — update the expectation (the new events are the point; do not suppress the emissions to make a test pass).

- [ ] **Step 2: Web checks**

Run: `cd web && npm run typecheck && npm test`
Expected: green (state.ts case is additive).

- [ ] **Step 3: CI gate**

Run: `bash scripts/ci.sh` (repo root)
Expected: fmt + clippy + cargo test + web all PASS. Fix any fmt/clippy fallout from Tasks 1-4 here.

- [ ] **Step 4: Commit any fallout fixes**

```bash
git add -A
git commit -m "test(core): adapt event-sequence fixtures to OverflowRecovery + Usage re-emit"
```

(Skip if Steps 1-3 produced no changes.)
