# Turn-Atomic Context Curation — Design

**Date:** 2026-07-01
**Status:** Approved (brainstorming) → ready for plan
**Source:** Cluster 5 of the harness deep audit
(`docs/superpowers/audits/2026-07-01-harness-deep-audit.md`, Spine B — Context
Engineering, HIGH at `context.rs:147-155; curated.rs:134-141,174-184`; Top-10
fix #5), plus the folded MED "eviction is silent" finding
(`context.rs:144-155; curated.rs:131-141`). Anchors re-verified against live
`main` (`81e768f`) on 2026-07-01.

## Invariant

At every point where curation reshapes history, an assistant message carrying
`tool_calls` and the consecutive `Role::Tool` result messages that follow it
move as **one atomic unit**. A built request never contains a tool result
whose parent `tool_calls` message was dropped — the shape OpenAI-compatible
servers reject with a 400, exactly when the window is fullest.

## Findings addressed (verified live)

1. **HIGH — torn eviction in both builds.** `WindowContext::build`
   (`context.rs:144-155`) and `CuratedContext::build` (`curated.rs:131-145`)
   walk history newest-first per-message and cut where the budget runs out. A
   cut inside a tool turn keeps the `Role::Tool` results (newer) and evicts
   the parent assistant `tool_calls` message (older) → serialized
   `tool_call_id` with no parent → mid-session 400. (The reverse tear —
   parent kept, results evicted — cannot occur under prefix eviction; results
   are always newer than their parent.)
2. **HIGH — torn compaction split.** `CuratedContext::maintain`
   (`curated.rs:177`) computes `split = len - keep_recent`, which can land
   mid-turn: the recent window then *starts* with orphaned tool results while
   their parent goes to the summarizer.
3. **MED (folded, per scope decision) — silent eviction.** Window eviction
   emits no event and leaves no placeholder; offload and compaction both emit
   `ContextEvent`s, eviction does not. Invisible to model and user.

Context confirmed during verification:

- `maintain()` runs after tool results are appended (`loop_.rs:514`);
  `build()` runs at the top of every model call (`loop_.rs:274`). Both must
  independently preserve the invariant.
- Production contexts are `CuratedContext` everywhere (`agent-cli/main.rs:268`,
  `agent-server/session.rs:68,270`); `WindowContext` is a test/e2e fallback.
- The offload pass replaces tool-message *content* with a placeholder but
  keeps role/`tool_call_id` intact — it cannot create orphans and is
  untouched.
- `ContextEvent` plumbing core → wire → CLI/web already exists (observability
  cluster): `render.rs` dims context notices, the server forwards `Context`
  events, the web UI renders context markers.

## Decisions (resolved with the user)

1. **Scope:** fold the silent-eviction MED into this cluster (same code, the
   event plumbing already exists; re-touching the walk later re-pays review).
2. **Approach:** shared turn-unit grouping helper (the audit's own suggested
   shape). Rejected: post-build repair filter (equivalent for the two builds
   but doesn't help the compaction split, pays budget for messages it then
   discards, and repairs tears instead of not creating them);
   serialization-layer guard in `agent-model` (catches everything but
   silently mutates transcripts at the wire, hides curation bugs, and must be
   re-implemented per protocol).

## Section 1 — Shared turn-unit helpers (`agent-core/src/context.rs`)

New `pub(crate)` helpers beside `message_tokens`:

- `turn_unit_ranges(history: &[Message]) -> Vec<std::ops::Range<usize>>` —
  chronological grouping. A message with `tool_calls` `Some` and non-empty
  starts a unit that absorbs every consecutive `Role::Tool` message following
  it; every other message is a singleton unit. Defensive case: a stray
  `Role::Tool` with no preceding parent (we never create one; could only come
  from a corrupted pre-existing history) stays a singleton — grouping must
  not panic or mis-attach it.
- `snap_split_to_unit_boundary(history: &[Message], split: usize) -> usize` —
  the largest unit boundary `<= split`. Snapping only moves **left** (keeps
  more), never right.
- `orphaned_tool_positions(messages: &[Message]) -> Vec<usize>` — invariant
  checker: walking the built sequence, a `Role::Tool` message is orphaned
  unless its `tool_call_id` appears in the `tool_calls` of the nearest
  preceding assistant message with only `Role::Tool` messages in between.
  Used by tests and by a `debug_assert!` at the end of both `build()`s
  (debug builds only; zero release cost).

## Section 2 — Unit-based eviction in both `build()`s

`WindowContext::build` and `CuratedContext::build` replace the per-message
newest-first walk with a newest-first walk over `turn_unit_ranges`:

- Unit cost = sum of `message_tokens` over its messages.
- Keep whole units while `used + unit_cost <= budget`.
- Always keep at least the newest unit, even if it alone exceeds budget —
  today's keep-≥1 floor, now unit-shaped.

Pinned blocks, recall handling, and budget arithmetic are unchanged. Output
message order is unchanged (chronological after pinned).

## Section 3 — Compaction split snapping (`agent-core/src/curated.rs`)

In `maintain()`, `let split = self.history.len() - self.config.keep_recent`
becomes:

```rust
let split = snap_split_to_unit_boundary(
    &self.history,
    self.history.len() - self.config.keep_recent,
);
```

- A torn turn now lands wholly in the recent window (snap-left), never half
  in the summarizer.
- Consequence: `keep_recent` occasionally retains up to one extra turn's
  worth of messages for that pass — the safe direction; the next pass
  re-evaluates.
- If snapping reaches 0, the old span is empty; the existing
  `to_summarize.is_empty()` early return exits without a model call. (The
  outer `history.len() > keep_recent + 1` gate is unchanged.)
- The verbatim-user partition is unaffected: user messages are always
  singleton units and never sit inside a tool unit.

## Section 4 — Evicted event (finding 3)

**`agent-core/src/event.rs`** — new variant:

```rust
/// Emitted by `maintain` when the built window omits history messages
/// (plain eviction — distinct from offload/compaction, which transform
/// rather than drop). `est_tokens` uses the same estimate the window
/// evicts against.
Evicted { messages: usize, est_tokens: usize },
```

**`CuratedContext`** — new private field `last_evicted: usize` (init 0). At
the end of `maintain()` (after offload + compaction): build the window for
`deps.model_limit`, count history messages omitted and their estimated
tokens; if `count > 0 && count != self.last_evicted`, emit
`ContextEvent::Evicted { messages, est_tokens }`. Update `last_evicted =
count` every pass (including back to 0) so a later re-eviction re-emits and a
saturated-but-stable window doesn't spam identical events.

The eviction check must run on **every** `maintain` exit. The current
compaction arm has an early `return report` on the nothing-to-summarize path
(`curated.rs:195-197`) that would skip it — restructure so the compaction
logic no longer early-returns past the eviction check (e.g. replace the
`return` with a scoped block/labelled break, or extract the compaction arm
into a private method whose return value `maintain` consumes before the
eviction check).

**Surfacing** (follows the existing `Offloaded`/`Compacted` pattern):

- `agent-cli/src/render.rs`: dimmed notice `⟲ evicted N messages (~T tokens)`.
- `agent-server/src/wire.rs`: additive mapping for the new variant, matching
  however `Offloaded`/`Compacted` cross the wire today.
- `web/src/wire.ts` + `state.ts` + the existing context-marker component:
  an "evicted" marker type rendered like the other context markers.

`WindowContext` stays silent: it has no sink (no `maintain` override) and is
not used in production.

## Error handling & edge cases

- **Parallel tool calls:** one parent + N results = one unit; a cut cannot
  split them.
- **Oversized newest unit** (parent + huge results alone over budget): kept
  whole — same failure mode as today's keep-≥1 message floor, now
  turn-shaped. The audit's separate "single oversized message" MED stays out
  of scope; the offload pass already shrinks large tool results before
  eviction matters.
- **Assistant with `tool_calls` whose results haven't been appended yet:**
  a smaller unit; grouping is order-safe regardless of when `build` runs.
- **Empty history / all-pinned:** `turn_unit_ranges([]) == []`; builds return
  pinned blocks only, as today.
- **Pre-existing orphan in history:** singleton unit, kept or evicted as-is;
  the invariant guards what curation *creates*, and the checker will surface
  such a history in tests.

## Testing

- **Helpers (`context.rs`):** grouping (plain interleaved; parallel calls;
  tool-turn at start/end; stray orphan tool as singleton; empty);
  `snap_split_to_unit_boundary` (exact boundary unchanged; mid-unit snaps
  left; snap to 0);
  `orphaned_tool_positions` (clean sequence → empty; orphan detected;
  interloping non-tool message breaks adjacency → flagged).
- **Build atomicity (both impls):** history mixing user turns and tool turns,
  budget chosen to force the cut inside a tool turn → built output has no
  orphans (checker returns empty) and the torn turn was dropped whole.
- **Budget sweep (cheap exhaustive property, both impls):** one fixed mixed
  history; for every `model_limit` from 1 to (total tokens + slack), the
  built output passes the orphan checker. Deterministic, no randomness.
- **Compaction snap (`curated.rs`):** `keep_recent` chosen so the naive split
  lands between a parent and its results → after `maintain`, no orphans in
  history, the torn turn is wholly in the recent window, user turns still
  verbatim.
- **Evicted event:** over-budget history → `maintain` emits
  `Evicted{messages>0}`; second identical pass emits nothing; changed count
  re-emits; under-budget emits nothing and resets.
- **Wire/web:** serde mapping test for the new variant beside the existing
  context-event tests; web reducer test for the "evicted" marker.
- Existing `e2e_context_management` / `stress_context_management` suites and
  the full `bash scripts/ci.sh` gate stay green.

## Files touched

- `agent/crates/agent-core/src/context.rs` — helpers + unit-based
  `WindowContext::build` + `debug_assert`; tests.
- `agent/crates/agent-core/src/curated.rs` — unit-based build, split snap,
  `last_evicted` + emit; tests.
- `agent/crates/agent-core/src/event.rs` — `ContextEvent::Evicted`.
- `agent/crates/agent-cli/src/render.rs` — dimmed notice arm.
- `agent/crates/agent-server/src/wire.rs` — wire mapping; test.
- `web/src/wire.ts`, `web/src/state.ts`, context-marker component — evicted
  marker; reducer test.
