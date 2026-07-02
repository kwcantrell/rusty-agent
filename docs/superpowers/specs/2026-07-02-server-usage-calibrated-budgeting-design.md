# Server-usage-calibrated context budgeting + unconditional recall refresh — design

**Date:** 2026-07-02
**Status:** Approved (autonomous backlog-drain run; brief fixed by the deep audit's
Spine B #2 HIGH + #4 MED)
**Cluster:** 3 of 6 in the 2026-07 residual-backlog drain

## Problem

1. **Token budgeting runs on an uncalibrated estimate.** `estimate_tokens` is
   `chars/4` (context.rs:9-11); every budgeting decision — `ctx.build(model_limit)`
   eviction, `MaintCtx.model_limit`'s high-water compaction gate (curated.rs:178-180)
   and eviction budget (curated.rs:284) — trusts it against the configured
   `model_limit`. The estimate undercounts code (~3.2 chars/tok) and severely
   undercounts CJK/emoji, so compaction fires too late and real overflow happens;
   since the retry cluster, the once-per-turn overflow compact-and-rebuild is the de
   facto safety net for this known undercount (see `context-token-estimate-undercounts`
   memory). Meanwhile the server's ground truth — `assistant.prompt_tokens`, parsed
   and emitted as `ServerUsage` every turn (loop_.rs:395-404) — never flows back in.
2. **Stale recall blocks never clear.** `set_recall` is called only when retrieval
   returns lines (loop_.rs:322-327). Contexts persist across runs (REPL/server), so a
   run that retrieves nothing pins the previous run's recall block forever.

## Approaches considered

- **A (chosen): shrink the effective limit by an observed ratio.** Keep `chars/4`
  everywhere; learn `ratio = server_prompt_tokens / estimated_prompt_tokens` for each
  completed request and use `effective_model_limit = model_limit / ratio_smoothed`
  (only ever ≤ configured) for `build()` and `MaintCtx`. One knob, one direction
  (safety), no per-message re-estimation, no trait changes.
- **B: calibrate `estimate_tokens` itself** (global scale factor) — mutable global
  state consulted by every crate that estimates; wide blast radius, poisons
  unit-test determinism. Rejected.
- **C: pass observed tokens into `MaintCtx` as a gate override** (compare observed
  vs limit for *when*, estimates for *what to evict*) — two accounting systems in
  one pass; subtler invariants, no more accuracy than A where it matters. Rejected.

## Design

### 1. Calibration state and update (agent-core/src/loop_.rs)

- `AgentLoop` gains `calib_ratio_micros: std::sync::atomic::AtomicU64`, initial
  `1_000_000` (ratio 1.0 in fixed-point micros). Lives on the loop (Arc-shared per
  session) so calibration persists across runs; `&self` methods update it. Sub-agent
  child loops are separate `AgentLoop` instances → they self-calibrate (fresh at 1.0;
  acceptable — children are short and inherit the overflow safety net).
- **Sample:** each turn already computes `built_tokens(&messages)` for the `Usage`
  event; capture it as `est` for the request actually sent (the overflow-recovery
  rebuild recomputes both, so `est` always matches the final request). After a
  successful completion, if `assistant.prompt_tokens > 0 && est > 0`:
  `sample = prompt_tokens as f64 / est as f64`.
- **Smoothing:** EMA with alpha 0.5 — `new = 0.5 * old + 0.5 * sample` — then clamp
  to `[1.0, 4.0]`. Below 1.0 (overcounting) is clamped away: the configured limit is
  the ceiling, never raised. Above 4.0 is treated as anomalous (a backend reporting
  tool-schema/system overhead far beyond message text still converges safely at 4×).
- **Application:** private helper

  ```rust
  /// The configured window shrunk by the observed estimate-undercount ratio
  /// (server prompt_tokens vs chars/4 estimate). Never exceeds the configured
  /// limit; floor at 1/4 of it. Keeps chars/4 as the per-message currency while
  /// making the *budget* honest (audit Spine B #2).
  fn effective_model_limit(&self) -> usize
  ```

  used at the five budgeting sites: both `ctx.build(...)` calls (loop_.rs:391, 430)
  and both `MaintCtx { model_limit: ... }` constructions (loop_.rs:423-428, 777-782)
  — and the `ctx.build` inside overflow recovery. The two `Usage` **events** keep
  emitting the CONFIGURED `context_limit` (the real server window — display truth;
  the calibration is an internal safety margin, not a new fact about the server).
  `tracing::debug!` when the smoothed ratio moves by >5% in one update.
- Snapshot/`context_limit` on the wire: unchanged (configured limit).

### 2. Unconditional recall refresh

`loop_.rs:322-327` becomes:

```rust
if let Some(retriever) = &self.retriever {
    ctx.set_recall(retriever.retrieve(&user_input).await);
}
```

An empty retrieval now clears the prior run's recall block (`set_recall(vec![])`
already renders no block — pinned by an existing context.rs test). Per-turn
re-retrieval across the tool loop stays out of scope (recorded below).

## Error handling

- `prompt_tokens == 0` (backend doesn't report) → no sample; ratio stays at last
  value (initially 1.0 = today's behavior exactly).
- Degenerate tiny requests can't skew: alpha-0.5 EMA + clamp bounds any single
  sample's effect; ratio is always in [1.0, 4.0] so `effective_model_limit ∈
  [model_limit/4, model_limit]`.
- Atomic read-modify-write via `fetch_update` (relaxed ordering is fine — advisory
  value, no cross-thread invariants).

## Testing

1. Unit (loop_.rs, Scripted model + a recording ContextManager stub that captures
   the `model_limit` passed to `build`/`maintain`): scripted `Usage` chunk reporting
   `prompt_tokens = 2×est` → next turn's `build` receives ~`model_limit/1.5` (EMA
   0.5 from 1.0 → 1.5), and after repeated turns converges toward `/2`; a backend
   reporting 0 leaves the limit exactly configured; ratio never exceeds 4× shrink
   nor exceeds configured (overcounting backend).
2. Unit: `Usage` events still carry the configured `context_limit` while `build`
   receives the shrunken one.
3. Recall: retriever stub returning lines then empty across two `run` calls on the
   same context → recall block present after run 1, absent after run 2 (stub
   ContextManager records `set_recall` args, or use `WindowContext` and inspect
   `build()` output).
4. Existing suites green: context/curated property tests are limit-parameterized
   already; nothing changes below the loop boundary.

## Out of scope (recorded residuals)

- Per-turn recall re-retrieval inside the tool loop (retrieval still runs once per
  run against the initial user input).
- Persisting the learned ratio across process restarts (converges in 1-2 turns).
- Feeding calibration into `RuntimeConfig`/eval `CandidateConfig` — note for the
  context-evolve campaign: the champion was validated under uncalibrated chars/4;
  with well-behaved backends (ratio ≈ 1) behavior is unchanged, but example-heavy /
  CJK tasks may compact earlier and score differently. Watch on next campaign run.
- Child-loop calibration sharing (children start at 1.0).
- The estimator itself (chars/4 stays; the budget, not the currency, is calibrated).
