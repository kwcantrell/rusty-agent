# Retry follow-up batch

**Date:** 2026-07-01
**Cluster:** the accepted residuals of the merged retry-classification cluster
(`docs/superpowers/specs/2026-07-01-retry-classification-design.md`, merged
`b0d2de5..c2abcc1`), as recorded in the retry-cluster re-stamp in
`.agents/skills/harness-engineering/audit.md`.

## Invariant

Context-overflow recovery fires on every backend — claude-cli included — and
is observable on every surface (a real event, not just tracing); after a
mid-turn rebuild the turn's `Usage` reflects the rebuilt request; retry timing
is pinned in-situ without real sleeps. `AgentError` carries no dead variants.

## Findings addressed (verified live)

1. **claude-cli overflow blindness — `agent-model/src/types.rs:200-219`.**
   `class()` signature-checks `Status{400|413|422}` and `Stream` bodies for
   overflow but routes ALL `Process` errors to `Retryable` (line 216,
   unconditional). claude-cli surfaces model errors as
   `Process("claude exited (1): <stderr>")` (`claude_cli.rs:121-122`), so an
   overflow on that backend retries forever instead of triggering the
   once-per-turn compact-and-rebuild. The original spec's edge case (lines
   255-257) even asserted "only the Stream body signature check can catch it"
   — factually wrong: `Process` carries a body string just like `Stream`.
2. **Stale `Usage` after rebuild — `agent-core/src/loop_.rs:330-360`.**
   `AgentEvent::Usage` is emitted from the pre-request build (line 331); the
   overflow-recovery arm (347-360) compacts and rebuilds `base` but never
   re-emits, so every consumer shows the pre-compaction token count for the
   request actually sent.
3. **Recovery invisible off-trace — `loop_.rs:349`.** Recovery's only output
   is `tracing::warn!`. `Compacted`/`CompactionFailed` may fire during
   `maintain()`, but nothing says *why* (overflow-forced) and nothing fires if
   maintenance no-ops.
4. **`AgentError::Cancelled` dead code — `loop_.rs:15-21`.** Zero construction
   sites in the workspace (`ModelError::Cancelled` + `RetryFailure::Cancelled`
   replaced it); no serde derives, no wire use, and every match uses
   `AgentError::Model(_)` non-exhaustively. Safe plain deletion.
5. **Real-sleep retry test / missing in-situ backoff pin.**
   `transport_error_then_success_via_retry` (`loop_.rs:1547`) runs a real
   ~100 ms tokio sleep; the timeout tests already use
   `#[tokio::test(start_paused = true)]` (lines 1756/1802/1842 — the pattern
   exists in-file). `backoff_delay` is unit-pinned but nothing pins that the
   RETRY LOOP actually sleeps those durations (the spec's paused-clock
   deviation).

## Decisions

- **`Process` gets the same signature check as `Stream`, not a bespoke one.**
  One guard arm in `class()`; `body_is_overflow` unchanged. False-positive
  exposure (stderr echoing e.g. "context window") is bounded: recovery runs at
  most once per turn and then classification falls back to the plain class —
  the same exposure `Stream` bodies already have, accepted by the original
  spec's "conservative by design" stance.
  *[Correction 2026-07-01, final whole-branch review: `class()` is stateless —
  there is no "fall back to the plain class." A SECOND overflow-classified
  error in the same turn is FATAL (`Error` + `Done(StopReason::Error)`),
  pinned by the pre-existing `second_overflow_in_a_turn_is_fatal` test. Still
  bounded — the turn fails fast instead of burning retry budget — and still
  the same exposure profile `Stream` already had.]*
- **Correct the original spec in place, dated.** The retry spec's claude-cli
  edge case gets a bracketed correction note rather than silent rewriting —
  the audit trail convention (specs are marked implemented; corrections are
  visible).
- **Recovery event is a new `ContextEvent` variant, payload-free.**
  `ContextEvent::OverflowRecovery` emitted at the moment recovery starts
  (before `maintain()`), so it appears even when compaction no-ops; the
  existing `Compacted`/`CompactionFailed` still narrate what maintenance did.
  Wire mapping kind `"overflow_recovery"`, empty `detail` — additive; web's
  `describeContext` default renders unknown kinds opaquely, so only a cosmetic
  case is added. The compiler finds every exhaustive `ContextEvent` match
  (wire.rs confirmed; any CLI match surfaces at build time).
- **Usage re-emit reuses the existing event.** After the recovery rebuild,
  emit `AgentEvent::Usage` again with the new `built_tokens(&messages)` and
  the SAME `turn`/`max_turns`. Semantics: `Usage` is a stream and latest-wins
  per turn. No wire/schema change.
- **`AgentError` stays an enum** after deleting `Cancelled` — single-variant
  today, but the type is the loop's public error surface and a struct-ification
  is churn with no payoff (YAGNI cuts the refactor, not the enum).
- **Retry-After and jitter remain deferred** (original spec's own deferral;
  requires header capture in `ModelError` — not part of this batch).

## Section 1 — Process overflow arm + spec correction (agent-model, docs)

`agent-model/src/types.rs::class()`: add, adjacent to the existing `Stream`
overflow guard:

```rust
ModelError::Process(body) if body_is_overflow(body) => ErrorClass::ContextOverflow,
```

(placed with the other overflow guards, before the Retryable catch group; the
final `Process(_)` arm keeps non-matching bodies Retryable). Update the
`class()` doc comment to name all three body-checked variants.

`docs/superpowers/specs/2026-07-01-retry-classification-design.md` lines
~255-257: append a bracketed correction, e.g.
`[Correction 2026-07-01, retry follow-up batch: Process bodies carry stderr
text and ARE signature-checked — see 2026-07-01-retry-followup-batch-design.md.]`

Loop-level pin: a `loop_.rs` test drives `Scripted::Fail(ModelError::Process(
"claude exited (1): ... maximum context length ..."))` then success, asserting
the recovery path fires (compaction requested, rebuild, turn completes) — the
end-to-end proof that claude-cli overflow now recovers.

## Section 2 — Usage re-emit after rebuild (agent-core)

In the overflow-recovery arm (`loop_.rs:347-360`), after
`let messages = ctx.build(...)` and before rebuilding `base`, emit:

```rust
self.sink.emit(AgentEvent::Usage {
    prompt_tokens: built_tokens(&messages),
    context_limit: self.config.model_limit,
    turn: turn + 1,
    max_turns: self.config.max_turns,
});
```

(mirroring the pre-request emission verbatim; extract a small helper only if
the borrow structure makes duplication awkward — two call sites is fine).

## Section 3 — `ContextEvent::OverflowRecovery` (agent-core, agent-server, web)

- `agent-core/src/event.rs`: add payload-free variant with a doc comment
  ("model reported context overflow; the loop forced compaction and rebuilt
  the request — emitted before maintenance so it fires even if compaction
  no-ops").
- Emit it in the overflow-recovery arm right after the `tracing::warn!`
  (keep the trace line).
- `agent-server/src/wire.rs`: new match arm → kind `"overflow_recovery"`,
  `detail: json!({})`. Compiler enforces coverage; if agent-cli or trace code
  match `ContextEvent` exhaustively, add the equivalent arm there (render as
  a short "context overflow: compacted and retried" line).
- `web/src/state.ts::describeContext`: add
  `case "overflow_recovery": return "context overflow: compacted and retried";`
  (default case already keeps old SPAs safe).

## Section 4 — Delete `AgentError::Cancelled` (agent-core)

Remove the variant from `loop_.rs:15-21`. No other code changes required
(verified: zero constructions, no serde, all matches are `Model(_)` +
non-exhaustive). `cargo build` across the workspace is the proof.

## Section 5 — Paused-clock retry tests (agent-core)

- Convert `transport_error_then_success_via_retry` (and any other retry test
  still using real sleeps — sweep the retry test block) to
  `#[tokio::test(start_paused = true)]`, following the in-file pattern of
  `idle_stall_times_out_and_fails_after_retries`.
- Add the in-situ backoff-growth pin: with `start_paused`, script
  `Fail(Http)` three times then success; capture `tokio::time::Instant::now()`
  before `run`, assert the virtual elapsed time afterwards equals
  100 + 200 + 400 = 700 ms exactly (auto-advance makes paused sleeps
  instantaneous in wall-clock but exact in virtual time). This pins that the
  LOOP sleeps `backoff_delay(n)` per attempt, which the pure-function test
  cannot.

## Error handling & edge cases

- Process body that merely *mentions* an overflow phrase (false positive):
  classified ContextOverflow → one forced compaction + rebuild for that turn.
  *[Correction 2026-07-01, final whole-branch review: a second
  overflow-classified error in the same turn does NOT demote to Retryable —
  the loop's second-overflow arm is fatal (turn ends with `Done(Error)`,
  pinned by `second_overflow_in_a_turn_is_fatal`). Bounded either way: one
  recovery attempt, then fail-fast, no retry storm. Same exposure profile as
  Stream today.]*
- Spawn-failure Process bodies ("spawn <bin>: ...") never contain overflow
  phrases in practice; if one did, the bounded path above applies.
- `OverflowRecovery` before `maintain()`: guarantees visibility even when
  compaction no-ops or fails (then `CompactionFailed` narrates the outcome).
- Double `Usage` per turn: consumers must treat latest-as-current. The web
  stats panel keys off the most recent event by arrival order — verify in the
  sweep task; if it aggregates instead, fix the reducer to replace, not add.
- Virtual-time assertion brittleness: assert exact equality on virtual elapsed
  (paused clock is deterministic); if scripted-model internals ever add their
  own sleeps the test fails loudly — acceptable, it IS the pin.

## Testing

- `agent-model/src/types.rs`: class_table gains
  `Process("claude exited (1): ...context length...") → ContextOverflow` and
  keeps `Process("claude exited (1)") → Retryable`; extend
  `overflow_is_detected_on_status_and_stream_bodies` to Process bodies;
  extend `overflow_signatures_are_conservative` with a Process near-miss
  ("context deadline exceeded" → Retryable).
- `agent-core/src/loop_.rs`: Process-overflow recovery end-to-end test
  (Section 1); overflow-recovery test(s) additionally assert the re-emitted
  `Usage` (lower `prompt_tokens`, same turn) and the `OverflowRecovery`
  context event; backoff-growth virtual-time pin; converted tests stay green
  with `start_paused`.
- `agent-server/src/wire.rs`: kind-string test for `"overflow_recovery"`
  (mirroring the existing context-kind tests).
- `web`: `describeContext` unit test for the new kind if the file has a test
  peer; otherwise typecheck suffices (string-in-string-out).
- Full gate: `bash scripts/ci.sh`.

## Files touched

- `agent/crates/agent-model/src/types.rs` — Process overflow arm + tests
- `agent/crates/agent-core/src/loop_.rs` — Usage re-emit, OverflowRecovery emit,
  `AgentError::Cancelled` removal, paused-clock tests
- `agent/crates/agent-core/src/event.rs` — new ContextEvent variant
- `agent/crates/agent-server/src/wire.rs` — wire arm + test
- `web/src/state.ts` — describeContext case
- `docs/superpowers/specs/2026-07-01-retry-classification-design.md` — dated
  edge-case correction
- (any CLI/trace exhaustive ContextEvent match the compiler surfaces)

## Out of scope (recorded residuals)

- Retry-After header capture + backoff jitter (original spec deferral).
- Server-usage-calibrated token budgeting (separate backlog item; the Usage
  re-emit here fixes the stale *estimate*, not the estimate's accuracy).
- Recovery metrics/counters beyond the single event.
