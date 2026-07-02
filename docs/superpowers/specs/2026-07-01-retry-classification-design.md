# Retry Classification + Overflow Recovery — Design

**Date:** 2026-07-01
**Status:** Implemented (this plan: docs/superpowers/plans/2026-07-01-retry-classification.md)
**Source:** Cluster 8 of the harness deep audit
(`docs/superpowers/audits/2026-07-01-harness-deep-audit.md`, Component 4 —
Orchestration, MED at `loop_.rs:132-152`; Top-10 fix #8), plus three folds:
the deferred Spine-B MED half "compact-and-rebuild once on context overflow"
(`loop_.rs:140-151,196-216`), the terminal-`Done`-parity MED
(`loop_.rs:237-243,251-254,143-145`), and the backoff part of the
loop-robustness LOW (exponential, no Retry-After). Anchors re-verified
against live `main` (`e4e3bc2`) on 2026-07-01.

## Invariant

A model error is retried only when retrying can plausibly succeed. Permanent
request failures (4xx other than 408/429, response decode failures) abort on
the first attempt. A context-overflow error triggers one forced
compaction-and-rebuild per turn before it may count as fatal. Every terminal
path of the run loop emits `Done(StopReason)` — no consumer keyed on `Done`
ever hangs.

## Findings addressed (verified live)

1. **MED — no classification.** `completion_with_retry` (`loop_.rs:195-221`)
   treats every `ModelError` identically: a `Status{code: 400}` (auth error,
   malformed request, context overflow) is re-sent verbatim `max_retries = 3`
   times (4 total attempts) with linear 100/200/300 ms sleeps before failing.
   No `match` on the error anywhere; `ModelError` (`agent-model/src/types.rs:146-160`)
   has no classification helper. Retries also cannot help an over-limit
   request: the loop clones the same `base` request every attempt
   (`loop_.rs:203`) — the rebuilt-context escape hatch doesn't exist.
2. **MED (folded) — context overflow unrecoverable.** An over-limit request
   surfaces as `Status{code: 400, body}` (body preserved, trimmed to 1000
   chars, `openai.rs:270-277`) or as an in-band 200-stream error →
   `Stream(String)` (`openai.rs:189-197`). Zero overflow detection exists in
   the codebase; `maintain()` never runs on a failed model call (it runs only
   after tool execution, `loop_.rs:508-522`), and the loop holds
   `&mut dyn ContextManager`, which exposes no way to force compaction
   (`compact_old_span` is private on `CuratedContext`; the `compact_flag` is
   reachable only via the `context_compact` tool).
3. **MED (folded) — three terminal paths never emit `Done`.** Verified still
   live post-observability-cluster: retry-exhausted (`loop_.rs:301` returns
   `Err`, only `Error` emitted at 213), max_tokens/Length abort
   (`loop_.rs:323-330`, `Error` then `Ok`, no `Done`), protocol-repair
   exhausted (`loop_.rs:340-343`, `Error` then `Ok`, no `Done`). Frontends
   keyed on `Done` hang on all three. The clean exits (cancel, no-tool
   finish, budget) all emit `Done` correctly.
4. **LOW part (folded) — backoff.** Linear `100·attempt` ms (~600 ms total),
   no cap logic, inadequate against rate-limited backends.
5. **Cancellation encoding (found during verification).** Cancel during a
   stream is encoded as `ModelError::Stream("cancelled")` (`loop_.rs:137,151`);
   the retry loop distinguishes cancel only via `cancel.is_cancelled()`
   (`loop_.rs:208`), not via the error value. A genuine stream error string
   is indistinguishable from a cancellation by inspection.

Context confirmed during verification:

- `StopReason` (`types.rs:89-96`) has `Stop/ToolCalls/Length/BudgetExhausted/
  Cancelled` — no `Error`. The wire serializes it as an opaque string
  (`wire.rs:150-158`); the web type is `{ type: "done"; reason: string }` —
  adding a variant is additive and old-client safe.
- `Scripted::Error` (testkit) always yields `ModelError::Http("scripted
  error")` — no way to script a `Status`/`Decode`/specific failure today.
- Production `max_retries = 3` (`assemble.rs:65`); exhaustion aborts the whole
  run (`loop_.rs:301` propagates `Err`), not just the turn — semantics kept.

## Decisions

1. **Classification lives on the type**: `ModelError::class() -> ErrorClass`
   in `agent-model`, beside the enum — the client that constructs the errors
   owns their meaning; the loop just consumes the class. Rejected: matching
   inline in `completion_with_retry` (spreads taxonomy knowledge into the
   loop; untestable without a loop harness).
2. **Classes**: `Retryable` (`Http`, `Stream`, `Process`, `Timeout`,
   `Status{500..=599 | 408 | 429}`), `Fatal` (`Status{other 4xx}`, `Decode`),
   `ContextOverflow` (`Status{400 | 413 | 422}` or `Stream` whose body
   matches an overflow signature — checked before the Fatal 4xx rule).
   408/429 are the two retryable 4xxs (timeout, rate limit) — a deliberate
   deviation from the audit's blanket "abort on 4xx".
3. **Overflow signatures** (case-insensitive substring on the error body):
   `"context length"`, `"context window"`, `"context size"`,
   `"too many tokens"`, `"prompt is too long"`. Conservative by design: a
   miss degrades to Fatal-on-400 (same as the audit's baseline fix), never
   to a wrong retry storm.
4. **New `ModelError::Cancelled` variant** replaces the
   `Stream("cancelled")` encoding at the two cancel sites; classified out
   before anything else. The token check at `loop_.rs:208` stays (authoritative);
   the variant removes the spoofable string and makes `class()` exhaustive.
5. **Overflow recovery is turn-level, once per turn.**
   `completion_with_retry` returns overflow WITHOUT consuming retry budget;
   the turn loop forces `ctx.request_compaction()` + `ctx.maintain(&deps)`,
   rebuilds the request from the shrunk context, and retries the turn once.
   A second overflow in the same turn is fatal. New provided trait method
   `ContextManager::request_compaction(&mut self)` (default no-op;
   `CuratedContext` sets its own `compact_flag`) — rejected: exposing
   `compact_old_span` (bypasses the worthwhile/gate logic) and wiring the
   raw `AtomicBool` into the loop (couples the loop to a CuratedContext
   implementation detail).
6. **Done parity**: add `StopReason::Error`; emit `Done(StopReason::Error)`
   on the fatal/exhausted model path and the protocol-repair-exhausted path,
   and `Done(StopReason::Length)` on the max_tokens abort. Run-abort
   semantics unchanged (fatal still returns `Err` after emitting).
7. **Backoff**: exponential `100ms · 2^(attempt-1)` capped at 5 s
   (100/200/400 for the production 3 retries; the cap matters only for
   raised configs). No jitter, no Retry-After — single local client, and
   Retry-After needs header capture in `ModelError`; both recorded as
   deferred residuals.
8. **Testkit**: add `Scripted::Fail(ModelError)`; keep `Scripted::Error` as
   the existing `Http` shorthand so current tests stand.
9. **Out of scope**: mid-stream duplicate-output MED (buffer/retract),
   approval-cancel MED, `LoopConfig::Default` zero-timeouts LOW, per-retry
   structured events (tracing::warn stays), server-usage-calibrated budgeting.

## Section 1 — Error taxonomy (`agent-model/src/types.rs`)

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
```

- New variant `ModelError::Cancelled` (`#[error("cancelled")]`).
- `impl ModelError { pub fn class(&self) -> ErrorClass }`:
  - `Cancelled` → caller handles before classing (see Section 2); `class()`
    returns `Fatal` for it defensively — it must never reach the retry arm.
  - overflow check first: `Status { code: 400 | 413 | 422, body }` or
    `Stream(body)` where `body_is_overflow(body)` → `ContextOverflow`.
  - `Status { code: 408 | 429 | 500..=599, .. }` → `Retryable`.
  - `Status { .. }` (remaining = other 4xx and any non-5xx oddity) → `Fatal`.
  - `Decode(_)` → `Fatal`.
  - `Http(_) | Stream(_) | Process(_) | Timeout(_)` → `Retryable`.
- `fn body_is_overflow(body: &str) -> bool` — lowercase once, `contains` any
  of the five signatures.
- The two cancel construction sites in `loop_.rs` (`137`, `151`) switch from
  `ModelError::Stream("cancelled".into())` to `ModelError::Cancelled`.

## Section 2 — Classified retry loop (`agent-core/src/loop_.rs`)

`completion_with_retry` gains a signature-level distinction for overflow —
return `Result<AssistantTurn, RetryFailure>` with a loop-private enum:

```rust
enum RetryFailure {
    /// Fatal or retries exhausted; the flat message for Error/AgentError.
    Fatal(String),
    /// Cancellation observed (token or ModelError::Cancelled).
    Cancelled,
    /// Context overflow: the same request can never succeed. Not counted
    /// against max_retries; the turn loop may compact-rebuild-retry once.
    Overflow(String),
}
```

Body per attempt (cancel checks unchanged and first):

- `ModelError::Cancelled` or `cancel.is_cancelled()` → `Cancelled`.
- `class() == ContextOverflow` → emit
  `tracing::warn!(error, "context overflow; deferring to turn-level recovery")`,
  return `Overflow(e.to_string())` immediately.
- `class() == Fatal` → `self.sink.emit(AgentEvent::Error(e.to_string()))`,
  return `Fatal(e.to_string())` immediately (attempt 1, no sleep).
- `class() == Retryable` → count attempt; if `attempt > max_retries`, emit
  `Error` and return `Fatal` (today's exhaustion behavior); else
  `tracing::warn!` and sleep
  `Duration::from_millis((100u64 << (attempt - 1)).min(5_000))`.

The turn loop (`loop_.rs:295-301` region) becomes:

```rust
let mut overflow_recovered = false;
let t = loop {
    match self.completion_with_retry(&base, &cancel).await {
        Ok(t) => break t,
        Err(RetryFailure::Cancelled) => { /* Done(Cancelled); return Ok — as today */ }
        Err(RetryFailure::Overflow(msg)) if !overflow_recovered => {
            overflow_recovered = true;
            ctx.request_compaction();
            let deps = /* MaintCtx as at loop_.rs:508 */;
            ctx.maintain(&deps).await;
            let messages = ctx.build(self.config.model_limit);
            base = /* rebuild CompletionRequest from messages, as at 281-293 */;
            continue;
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

(The `Error` event for `Fatal` was already emitted inside
`completion_with_retry`; only `Done` is added at the turn level. The
request-building block at 281-293 is factored into a small private helper so
the overflow arm and the turn prologue share it rather than duplicating.)

## Section 3 — `request_compaction` (`agent-core/src/context.rs`, `curated.rs`)

```rust
// ContextManager trait — provided method:
/// Ask the manager to compact on its next maintenance pass. Default: no-op
/// (managers without a compaction concept ignore it).
fn request_compaction(&mut self) {}
```

`CuratedContext` overrides it: `self.compact_flag.store(true, SeqCst)` — the
same flag the `context_compact` tool sets, so `maintain` takes the existing
requested-compaction path (`curated.rs:176-183`) including its
`history.len() > keep_recent + 1` gate and worthwhile check. If compaction
declines (nothing to summarize / not worthwhile), the rebuild is a no-op and
the single overflow retry fails → fatal path. That is correct: the runtime
cannot shrink further; the failure is real. `WindowContext` inherits the
no-op default.

## Section 4 — Done parity (`agent-model`, `agent-server`, `loop_.rs`)

- `StopReason::Error` added (`types.rs:89-96`).
- `wire.rs:150-158` `stop_reason_str`: `StopReason::Error => "error"` —
  additive; web already treats `reason` as an opaque string (`wire.ts:62`),
  no web change needed.
- Emission sites:
  - fatal/exhausted/second-overflow model failure → `Done(StopReason::Error)`
    (Section 2).
  - max_tokens abort (`loop_.rs:323-330`): add
    `self.sink.emit(AgentEvent::Done(StopReason::Length))` before `return Ok(())`.
  - protocol-repair exhausted (`loop_.rs:340-343`): add
    `self.sink.emit(AgentEvent::Done(StopReason::Error))` before `return Ok(())`.

## Section 5 — Testkit (`agent-core/src/testkit.rs`)

`Scripted::Fail(ModelError)` — the stub returns exactly the given error for
that scripted turn. `Scripted::Error` stays as-is (`Http("scripted error")`).

## Error handling & edge cases

- **Overflow on the rebuilt request in a later turn**: the `overflow_recovered`
  flag is per-turn; a later turn gets its own single recovery.
- **Overflow recovery does not consume retry budget**; retryable errors after
  a successful overflow rebuild get the full `max_retries` (the attempt
  counter lives inside `completion_with_retry`, which is re-entered fresh).
- **claude-cli backend**: overflow surfaces as `Process(..)`/`Stream(..)`
  without a status code — only the `Stream` body signature check can catch
  it; a miss retries as today (no regression).
  *[Correction 2026-07-01, retry follow-up batch: `Process` bodies carry the CLI's stderr
  text and are now signature-checked exactly like `Stream` — see
  `2026-07-01-retry-followup-batch-design.md`.]*
- **In-band `Stream` server errors that aren't overflow** stay `Retryable`
  (today's behavior — e.g. llama.cpp slot exhaustion is genuinely transient).
- **`Decode` mid-stream** is already skipped inside the stream reader
  (`openai.rs:296-299`) and never reaches the classifier; only
  initial-response decode fails → correctly Fatal.
- **Cancel raced with overflow/fatal**: the `cancel.is_cancelled()` check
  precedes classification, so cancel still wins and maps to
  `Done(Cancelled)`.
- **Old web clients**: `"error"` arrives as an unknown reason *string* value
  in an existing frame shape — rendered as-is, no crash (unlike new frame
  *types*).

## Testing

- **`agent-model` unit tests**: `class()` table — every variant; 400/413/422
  with and without each overflow signature (and case variations); 401/403/404
  Fatal; 408/429/500/503 Retryable; `Cancelled` never Retryable;
  `body_is_overflow` negative cases ("context deadline exceeded" must NOT
  match — deliberately absent from the signature list).
- **`loop_.rs` tests** (using `Scripted::Fail`):
  - 400 fails fast: model consulted exactly once, `Error` + `Done("error")`
    emitted, run returns `AgentError::Model`.
  - 429 then success: retried, recovers, `Done` clean.
  - Overflow then success: context stub records `request_compaction` +
    `maintain` called once, request rebuilt (second attempt sees the
    stub's shrunk message list), no retry budget consumed.
  - Overflow twice in one turn: single recovery, then `Error` +
    `Done("error")` + `Err`.
  - Exhaustion still aborts after `max_retries` retryable failures and now
    also emits `Done("error")` (update `idle_stall_times_out_and_fails_after_retries`
    to assert the `Done`).
  - Existing recovery tests (`transport_error_then_success_via_retry`,
    `stall_then_success_recovers_via_retry`) stay green unchanged.
  - Backoff: with `start_paused` time, assert exponential sleep growth and
    the 5 s cap (extend the existing paused-clock pattern).
  - max_tokens and protocol-repair paths emit their `Done`s (extend the
    existing tests for those paths if present; add minimal ones if not).
- **`curated.rs`**: `request_compaction()` sets the flag → next `maintain`
  takes the compaction path (reuse the existing compaction test harness).
- **`wire.rs`**: `stop_reason_str(Error) == "error"` beside the existing
  mapping test.
- Full `bash scripts/ci.sh` green.

## Files touched

- `agent/crates/agent-model/src/types.rs` — `ErrorClass`, `class()`,
  `body_is_overflow`, `ModelError::Cancelled`; tests.
- `agent/crates/agent-core/src/loop_.rs` — `RetryFailure`, classified
  retry loop, exponential backoff, turn-level overflow recovery, request-
  build helper, `Done` emissions, cancel-site variant swap; tests.
- `agent/crates/agent-core/src/context.rs` — `request_compaction` provided
  method.
- `agent/crates/agent-core/src/curated.rs` — override + test.
- `agent/crates/agent-core/src/testkit.rs` — `Scripted::Fail`.
- `agent/crates/agent-server/src/wire.rs` — `"error"` arm; test.
