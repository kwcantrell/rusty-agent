# Runtime knobs — max_parallel_tools config promotion + graceful max_turns landing

**Date:** 2026-07-02
**Status:** Approved (2026-07-02 product-decision round, items 8 and 7)
**Branch:** `feat/runtime-knobs`

Two small, independent runtime-loop improvements approved in the parked
product-decision round that followed the 2026-07 backlog drain. One branch, two
tasks; no interaction between them.

## Part 1 — Promote `max_parallel_tools` into `RuntimeConfig` (item 8)

### Problem

`LoopConfig.max_parallel_tools` exists (`agent-core/src/loop_.rs:105`; 0 = use
`DEFAULT_MAX_PARALLEL_TOOLS = 8`), but `loop_config_from` hardcodes `8`
(`agent-runtime-config/src/assemble.rs:117`). The knob is invisible to users and
not persisted/mergeable like its sibling limits.

### Design

Follow the `subagent_max_turns` pattern exactly:

- `RuntimeConfig.max_parallel_tools: usize` with
  `#[serde(default = "default_max_parallel_tools")]`;
  `fn default_max_parallel_tools() -> usize { agent_core::DEFAULT_MAX_PARALLEL_TOOLS }`.
- `PartialRuntimeConfig.max_parallel_tools: Option<usize>` + the corresponding
  merge arm (per-field, older files fall back to the default).
- `validate()` rejects `0` (`"max_parallel_tools must be >= 1"`). The
  `0`-means-default sentinel remains a `LoopConfig`-internal convention for
  direct constructors; the config path always delivers >= 1.
- `loop_config_from` passes `cfg.max_parallel_tools` (replaces the literal 8).
- Dispatch child loops inherit the parent `LoopConfig` clone — unchanged.

Wire/compat: `RuntimeConfig` is serialized on the wire and to disk; the field is
serde-defaulted, so old files and old clients parse (additive-only rule holds).

### Tests

- Serde: missing field defaults to 8; round-trip preserves an explicit value.
- Merge: partial file with/without the field.
- `validate()` rejects 0, accepts 1.
- Assemble passthrough (update the existing assertion at `assemble.rs:504`).

## Part 2 — Graceful max_turns landing (item 7)

### Problem

When the turn loop exhausts `max_turns` the run hard-stops:
`Done(StopReason::BudgetExhausted)` at `loop_.rs:879-881`, ending mid-thought —
the model gets no chance to summarize progress or hand back state. This path is
reached only when the model was still issuing tool calls (a text-only reply
exits earlier with `Done(Stop)`), so the transcript ends on tool results.

### Design

At the loop fall-through, before `Done(BudgetExhausted)`, run **one best-effort,
tools-disabled wrap-up completion**:

1. If `cancel.is_cancelled()` → emit `Done(BudgetExhausted)` as today (no wrap-up).
2. Append a user message to the context: the turn limit is reached, tools are
   disabled, summarize what was accomplished, what remains, and any state the
   caller needs. (Appending AFTER the turn's tool results respects the
   OpenAI-compat ordering constraint, same siting as the stuck nudge.)
3. Build the request from `ctx.build(effective_model_limit())` with
   `tools: vec![]` and otherwise identical parameters (same sampling knobs, the
   run's computed `preserve_thinking`).
4. Drive it through `one_completion` directly — Token/Reasoning events stream to
   the sink exactly like a normal turn. **Single attempt**: no retry, no
   overflow recovery, no StreamRetry accounting. Failure handling:
   - `ModelError::Cancelled` → emit `Done(Cancelled)` (matches loop-entry
     behavior for a user cancel).
   - Any other error → `tracing::debug!` and fall through to
     `Done(BudgetExhausted)`. The courtesy completion must never fail the run.
5. On success: append the reply as a **text-only** assistant message. Any
   `raw_tool_calls` the model still emitted are discarded (no dangling
   `tool_call` ids in persistent history — same rule as the stuck-abort append).
   Emit `ServerUsage` if the backend reported usage (cost truth), but NO
   estimate `Usage` event — turn indices stay <= `max_turns` for stats/UIs.
6. Emit `Done(StopReason::BudgetExhausted)` unchanged.

Not in scope / interactions:

- The stuck-abort path (`Done(Error)` on the 5th identical call set) is
  untouched — a stuck model gets no wrap-up.
- No config flag (YAGNI): always on. Existing tests that script an exact number
  of completions to reach `BudgetExhausted` gain one scripted wrap-up turn or
  pin the skip-on-error arm.
- Dispatch children inherit the behavior: a child hitting `subagent_max_turns`
  now hands its parent a real summary (the `SubagentSink` capture already reads
  the final-turn text tail + footer).
- Zero wire changes: Token/Reasoning/ServerUsage/Done all exist (old-SPA safe).

### Tests

- Loop test: scripted model that issues tool calls for all `max_turns` turns
  then a final text reply → wrap-up text streamed + appended text-only, then
  `Done(BudgetExhausted)`; the wrap-up request carried no tool schemas.
- Wrap-up failure (`Scripted::Fail`, non-cancel) → no assistant append, still
  `Done(BudgetExhausted)`.
- Cancel during wrap-up → `Done(Cancelled)`.
- Wrap-up reply containing tool calls → calls discarded, text appended.

## Out of scope (recorded)

- Item 9 (live trace toggle) and item 2 (persisted OffloadStore): DECLINED-BY-OWNER
  in the 2026-07-02 decision round.
- Making the wrap-up message text configurable.
