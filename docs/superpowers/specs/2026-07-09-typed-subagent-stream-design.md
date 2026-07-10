# Typed sub-agent stream (3B-2) — design

**Date:** 2026-07-09
**Phase:** deepagents-refactor 3B-2 (product-level typed subagent stream)
**Base:** main @ 3490590 (3B-1c merged)
**Bundle evidence:** `docs/okf/deepagents-refactor/` — deepagents `stream_events`
exposes a `subagents` projection: per-delegation handles with status and nested
streams, the *product-level* view distinct from the internal "subgraphs" view
(`sources/deepagents-docs.md`; gap analysis marks our streaming row **partial**:
`parent_id`-tagged raw events, no typed per-delegation view).

## 1. Problem & goals

Today the only sub-agent visibility a frontend gets is `SubagentSink`'s
forwarded child `ToolStart`/`ToolResult` rows (ids `sub{n}:{id}`, names
`sub:{name}`, `parent_id` = the dispatching call's on-wire id) plus
`ServerUsage` frames. Child text/reasoning never leave the capture (trace tap
only). Frontends cannot tell that a delegation started, which named type
(3B-1 registry) is running, what the child is saying while it works, or how
and why it ended.

**Goal:** a typed, per-delegation event stream — lifecycle + live nested
child text/reasoning — that the web SPA renders as a first-class sub-agent
card and the CLI renders as start/end lifecycle lines.

**Non-goals:**
- No change to what the parent **model** sees (tool-result handoff content is
  byte-identical — 3B-1b sever + footers untouched).
- No loop changes: dispatch is a tool; emission lives in `dispatch.rs` and
  the wire/frontends (plus the compile-forced trace/testkit edit sites in
  §2.6).
- No removal of the `sub:`/`sub{n}:` prefix mechanism this phase (deprecated
  in doc comments only; owner decision "additive now, deprecate later").
- No per-delegation transport multiplexing (deepagents' handle *API shape* is
  not ported; a flat tagged stream reconstructs the same tree client-side).
- **Cards are live-only.** Reconnect/restore does not rebuild them — the SPA
  persists no transcript and the server has no replay buffer (pre-existing
  posture for the whole streaming surface; durable delegation state is the
  checkpointing phase's problem). The reducer's placeholder rule (§2.4)
  softens mid-run reloads but does not recover lost frames. *(Gate G3:
  approved — placeholder + live-only; no server replay buffer this phase.)*
- **Child approval attribution is deferred.** A child blocked on an approval
  still raises the global modal (shared approval channel), but neither the
  modal nor the card names the delegation — `ApprovalRequest` frames carry no
  `parent_id` today and the approval channel does not know the delegation id.
  Named as a known gap; a follow-up threads delegation identity through a
  per-child approval wrapper. *(Gate G4: approved — deferred.)*

## 2. Design

### 2.1 Event model (agent-core)

New `AgentEvent::Subagent(SubagentEvent)` variant. Note `AgentEvent` itself
is a bare, derive-free enum (no `Debug`/`Clone`/`Serialize`) — nothing may
assume `AgentEvent: Clone`; emitters construct fresh values. The *payload*
enum derives `Debug, Clone` like `ContextEvent` does. Every case carries the
**delegation id** = the dispatching call's on-wire id,
`format!("{id_prefix}{call_id}")` — exactly the string stamped as `parent_id`
on forwarded child rows today. This joins the typed stream to the existing
`dispatch_agent` tool row and composes through nesting: a grandchild's
delegation id is already fully qualified (`sub{n}:c3`) because
`nested.id_prefix = "sub{n}:"` (spec G8), and it equals the forwarded
`sub:dispatch_agent` row's on-wire id — so the same reducer rule attaches
cards at any depth (flat rendering, no special grandchild handling).

```rust
#[derive(Debug, Clone)]
pub enum SubagentEvent {
    /// One dispatch began. Emitted after all validation (a rejected dispatch
    /// never emits Start) and after the loop's own ToolStart for the
    /// dispatch_agent call, so frontends always see the host row first
    /// (verified: gate_tool emits ToolStart sequentially in phase 1 before
    /// any phase-2 execution begins, even with max_parallel_tools > 1).
    Start {
        id: String,
        /// Registry name, or "general-purpose" for the ad-hoc path.
        subagent_type: String,
        /// The per-call role arg (general-purpose only; None for named types,
        /// which ignore role).
        role: Option<String>,
        // NOTE: no `prompt` field — the dispatch prompt already rides the
        // host row's ToolStart args; duplicating an unbounded payload on the
        // wire bought nothing (panel: Scope MAJOR-5).
    },
    /// Child assistant text delta.
    Text { id: String, text: String },
    /// Child reasoning delta.
    Reasoning { id: String, text: String },
    /// Child mid-stream retry retracted in-flight deltas: frontends trim the
    /// tail of THIS delegation's transcript (mirrors top-level StreamRetry).
    /// (Gate G1: KEPT — retry fidelity is part of nested-token fidelity.)
    StreamRetry {
        id: String,
        discarded_text_chars: usize,
        discarded_reasoning_chars: usize,
    },
    /// The delegation finished, on any path.
    End {
        id: String,
        outcome: SubagentOutcome, // Completed | Timeout | Failed | Cancelled
        /// The child's own stop reason from the capture; None when the child
        /// never emitted Done (e.g. timeout before first turn completed).
        stop: Option<StopReason>,
        /// Human-readable failure/timeout detail (the `what` string
        /// failure_output already builds, e.g. "sub-agent failed: <err>");
        /// None on Completed. A Failed card without a reason is worse than
        /// today's readable tool-result (panel: Requirements MAJOR-1).
        detail: Option<String>,
        turns: usize,
        tool_calls: u64,
        duration_ms: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentOutcome { Completed, Timeout, Failed, Cancelled }
```

`SubagentOutcome` is deliberately not derived from `stop` + tool-result
status: the four exit paths collapse ambiguously onto tool status (Timeout
and Failed both surface non-`ok`; Cancelled returns `Err`), and `execute()`
is the only place that distinguishes them (panel: Scope MINOR-6 — earned
redundancy).

### 2.2 Emission (dispatch.rs)

Two emitters, both already holding everything they need:

- **`execute()`**: the delegation id local (`parent_id`) is **moved** into
  `SubagentSink::new` today — the implementation clones it into an owned
  local first and reuses that clone for both `Start` and `End` (panel:
  Assumptions MAJOR-2). `Start` is emitted to `self.deps.sink` right after
  that point — verified: there are **no `Err` returns between
  `next_dispatch_n()` and `run_with_cancel`** (every validation returns
  earlier), so a rejected dispatch never emits `Start`.
- **`End` rides an RAII drop-guard, not bare return paths.** A guard struct
  is armed immediately after `Start` (holding a sink Arc + the id + an
  `Instant`); each of the four ordinary exit paths *disarms* it and emits a
  precise `End`:
  - timeout → `outcome: Timeout`, detail = the timeout message,
  - fatal child error (`Ok(Err(e))`) → `Failed`, detail = the error string,
  - parent-cancel (`ctx.cancel.is_cancelled()` after a clean run) →
    `Cancelled`,
  - normal completion → `Completed` + the summary's stop reason.
  If the guard drops *without* being disarmed — a panic inside the child run
  future (caught by `execute_isolated`'s `catch_unwind`, which terminates
  `execute()` past the `.await`) or the loop's backstop timeout dropping the
  tool future — `Drop` emits `End { outcome: Failed, detail: Some("dispatch
  aborted (panic or executor drop)"), .. }` with whatever stats the sink
  captured. Exactly one `End` per `Start`, on every termination the runtime
  can produce (panel: Failure MAJOR-2). `stop`/`turns`/`tool_calls` come from
  the existing `CaptureSummary` (the same source the tool-result footer uses
  — one truth).
- **`SubagentSink`** forwards child deltas as typed events, stamped with the
  sink's `parent_call_id` (the delegation id): child `Token` →
  `Subagent(Text)` and child `StreamRetry` → `Subagent(StreamRetry)` gain
  forward calls inside their existing capture arms (capture behavior —
  segment append, retry trim — unchanged; `Token`'s text is borrowed for the
  append then moved into the forward). Child `Reasoning` → `Subagent(Reasoning)`
  is a **net-new arm**: reasoning is not captured today and stays uncaptured
  — the arm only forwards (panel: Assumptions MINOR-1). Child
  `Done`/`Error`/`Context`/`Usage` stay off the stream exactly as today, and
  the child-trace tap keeps recording every one of these events unchanged.

**No sink chaining concern:** `SubagentSink`s never nest — every dispatch at
any depth wraps `self.deps.sink`, which is the top-level sink (verified:
nested deps reassign depth/id_prefix/policy/base_tools but never `sink`).
Typed events emitted at any depth reach the top-level sink directly.
`SubagentSink` also never *receives* a `Subagent(...)` event (a nested
`execute()` emits to its `deps.sink` directly, and the child loop itself
never produces them), so its match needs no dedicated arm — a hypothetical
one falls to the catch-all (trace tap). A test pins this absence.

### 2.3 Wire (agent-server)

Additive `ServerEvent` frames, mapped 1:1 in `server_event_from`:

| frame | payload |
|---|---|
| `subagent_start` | `id`, `subagent_type`, `role?` |
| `subagent_text` | `id`, `text` |
| `subagent_reasoning` | `id`, `text` |
| `subagent_stream_retry` | `id`, `discarded_text_chars`, `discarded_reasoning_chars` |
| `subagent_end` | `id`, `outcome` (`"completed"\|"timeout"\|"failed"\|"cancelled"`), `stop?` (via `stop_reason_str`), `detail?`, `turns`, `tool_calls`, `duration_ms` |

None-valued optionals are omitted (`skip_serializing_if`), matching every
existing frame's pins. Compat verified both directions: the current SPA's
`parseInbound` validates only the outer `kind` (no schema throw on unknown
payload `type`) and the reducer's `default:` returns state unchanged — old
SPAs ignore the new frames; a new SPA against an old server simply never
populates the card.

**Per-delegation cost needs no new frame:** child `ServerUsage` frames
already reach the wire carrying `parent_id` (= the delegation id). The web
reducer accumulates them into the card's stats (§2.4) — core and wire are
untouched for cost (panel: Requirements MAJOR-2, resolved web-side).

### 2.4 Web SPA (the product surface)

Typed frames **enrich the existing `dispatch_agent` tool item** — no new
item kind in the common path, no double-render. The card UI is bounded to a
**minimal v1** (panel: Scope MAJOR-2; Gate G2: approved — "full card" =
transcript richness, not DOM containment): the items list stays FLAT —
child tool rows keep rendering exactly as today (`↳`-indented siblings via
`parentId`; `AnimatedToolCall` already strips the `sub:` prefix); they are
NOT re-parented into the card's DOM. The card is the dispatch row grown
richer, not a container re-architecture.

- **Reducer.** `subagent_*` frames locate the **most recent running**
  dispatch-row tool item whose `id` equals the delegation id (same
  last-running discipline `tool_result` correlation already uses — model-
  supplied call ids are only deduped within a turn and can repeat across
  turns; panel: Failure MINOR-6). A `subagent_start` whose id matches only
  an already-`done` card opens a fresh standalone card item instead of
  overwriting it. Frames matching **no** item create a minimal placeholder
  card (softens mid-run reload/reconnect, where the pre-reload `tool_start`
  is gone but the detached run keeps streaming — panel: Failure BLOCKER-1;
  Gate G3: approved). The matched/created item grows a `subagent` field:
  `{ subagentType, role?, status: "running"|"done", text, reasoning,
  outcome?, stop?, detail?, stats? }`.
- **Appends + cap.** `subagent_text`/`subagent_reasoning` append to the card
  transcript, bounded by a per-card char budget (head-trim with an
  "…(N chars elided)" marker) so a runaway child cannot grow a single React
  string unboundedly (append is a full-string copy — O(n²) without a cap;
  panel: Failure MAJOR-5). `subagent_stream_retry` trims the transcript tail
  char-wise (same logic as the top-level `stream_retry` handler).
- **Cost.** `server_usage` frames with a `parent_id` matching a running card
  accumulate `promptTokens`/`completionTokens`/`costUsd` into its stats (the
  reducer currently drops them for the turn readout — that skip stays; the
  card gains what the readout ignores).
- **End + safety net.** `subagent_end` sets `status: "done"` + outcome /
  stop / detail / stats. When the run-level `done` frame arrives, any card
  still `running` is finalized as `outcome: "unknown"` — a belt-and-braces
  client-side guarantee against a lost `End` (the server-side drop-guard is
  the primary guarantee).
- **Rendering.** `AnimatedToolCall` grows a card variant when `subagent` is
  present: header (name badge + status pill), collapsible live transcript
  (streaming text, dimmed reasoning), outcome footer (outcome, stop, detail,
  turns, tools, duration, tokens/cost when accumulated).
- **Fallbacks.** A dispatch row that never receives typed frames (old
  server, restored history) renders exactly as today.

### 2.5 CLI

`TerminalSink` (matches `AgentEvent` directly; its match is exhaustive, so
the new variant is a compile-forced arm) renders:
- `Start` → line like `↳ agent[research-critic] started` (name =
  subagent_type, role appended for general-purpose when present).
- `End` → `↳ agent[research-critic] done — stop, 4 turns, 7 tools, 12.3s`;
  non-Completed outcomes render the outcome word (`timed out` / `failed` /
  `cancelled`) plus `detail` when present.
- `Text`/`Reasoning`/`StreamRetry` → consumed silently (interleaved parallel
  child prose is terminal noise; owner decision). Accepted residual: a
  runaway child burns tokens with no live CLI signal — pre-existing (capture
  already swallows child text), and the `End` line's stats quantify it
  post-hoc.
Existing `↳` child tool rows unchanged.

### 2.6 Trace layer + full edit-site inventory (compile-forced)

`AgentEvent` is hand-projected, not blanket-serialized. Adding a variant
compile-breaks every exhaustive match; the full inventory (verified by
sweep):

| site | kind of edit |
|---|---|
| `agent-core/src/event.rs` | the new variant |
| `agent-core/src/dispatch.rs` | emitters (§2.2) |
| `agent-server/src/wire.rs` `server_event_from` | exhaustive — new arm (§2.3) |
| `agent-cli/src/render.rs` `TerminalSink` | exhaustive — new arm (§2.5) |
| `agent-runtime-config/src/trace.rs` `trace_event` + `TraceEvent` | exhaustive — new arm + on-disk shape (below) |
| `agent-core/src/testkit.rs` `LabelSink` | exhaustive — new label arm |
| `agent-core/src/stats.rs` | has `_ => {}` — NO edit; `Subagent` events must not move any counter (the existing `subagent_turns`/`subagent_tool_calls` fold from `parent_id`-tagged rows stays the only source; test pins counters unmoved) |

**Trace rule — no double-record.** The top-level sink is wrapped by
`ObservedSink`, which records every emitted event to the session JSONL; the
child's raw `Token`/`Reasoning`/`StreamRetry` are *already* recorded once via
the `ChildTraceTap`. To keep invariant §3.4 true, the trace layer records
`Subagent(Start)`/`Subagent(End)` as new `TraceEvent` arms (cheap, useful
lifecycle markers) and **skips** `Subagent(Text/Reasoning/StreamRetry)` —
the raw tap records remain the single source of the child transcript. A test
asserts a child token appears exactly once in the JSONL (panel: Failure
MAJOR-4 / Scope BLOCKER-1).

## 3. Do-not-regress invariants

1. **Every pre-existing wire frame byte-identical.** The `sub:`/`sub{n}:`
   renaming, `parent_id` tagging, and None-omission pins are untouched; new
   frames are additive-only. The prefix mechanism is marked deprecated in doc
   comments; removal is a future phase.
2. **No loop / calibration / estimator / pinned-block changes.** Scope is
   the §2.6 inventory — nothing else.
3. **Sub-agent context quarantine preserved.** Typed events are sink-side
   telemetry; nothing enters any `CuratedContext` or the model handoff. The
   dispatch tool-result content is byte-identical (3B-1b sever/footers).
4. **Child transcript recorded to the session trace exactly once** (raw tap
   path). Typed `Text`/`Reasoning`/`StreamRetry` are skipped at the trace
   boundary; only `Start`/`End` add trace records (§2.6).
5. **Cancellation/timeout semantics unchanged** — `End` observes outcomes,
   never alters control flow; the drop-guard emits, it does not catch or
   suppress.
6. **`CaptureSummary` is the single stats truth** — footer and `End` cannot
   disagree; `stats.rs` session counters unmoved by `Subagent` events.

## 4. Backward compatibility

- Old SPA / old CLI: additive frames are ignored (verified — no schema
  rejection layer in `wire.ts`/`state.ts`; reducer `default:` no-ops);
  behavior identical to today.
- New SPA against an old server: card fields never populate; rows render as
  today.
- Trace consumers: `trace_event` is an exhaustive hand-written mapper (no
  `Serialize` fallback) — the new variant is a compile-forced arm with the
  §2.6 record/skip rule. Child-trace tap records are unchanged.

## 5. Testing & acceptance

- **dispatch.rs unit tests:** Start carries registry name vs
  `general-purpose`+role; Start absent on arg-rejected dispatch; Start's id
  always matches an already-emitted `dispatch_agent` ToolStart id (ordering
  pin); End on all four exit paths with correct outcome/stop/detail/stats
  (scripted child: normal, fatal, cancel; timeout via tiny `ctx.timeout`);
  **panicking child model → drop-guard emits exactly one `End{Failed}`**;
  Failed/Timeout carry non-empty `detail`; one emission asserts BOTH forward
  (typed Text) and capture (segments); StreamRetry forwards typed AND trims
  capture; nested dispatch emits fully qualified delegation ids (`sub{n}:`
  composition); SubagentSink never receives a Subagent event (absence pin).
- **trace tests:** child token appears exactly once in the session JSONL;
  Start/End produce trace records; session stats counters unmoved by
  Subagent events.
- **wire.rs pins:** per-frame type tags, None-omission (`role`, `stop`,
  `detail`), outcome/stop strings.
- **web tests:** reducer — card assembly on the dispatch row, parallel
  children interleaved by id, last-running discipline under a duplicate
  call_id across turns, placeholder card on unmatched frames, transcript cap
  + elision marker, retry trim, per-delegation cost accumulation from
  `server_usage` frames, finalize-on-done safety net, no-typed-frames
  fallback; component test for the card variant; depth-2 card attaches to
  the forwarded `sub:dispatch_agent` row.
- **CLI render tests:** start/end lines (incl. outcome word + detail);
  Text/Reasoning silent.
- **Acceptance:** full `ci.sh` green, plus **one live WebDriver drive**
  (auto-drive-tauri harness): dispatch a real sub-agent in the desktop app
  and assert the card renders with streamed child text and a done footer.

## 6. Decision log (brainstorm, 2026-07-09)

- **Stream depth:** full parity — lifecycle + nested child text/reasoning
  tokens (owner picked over lifecycle-only and final-text-only).
- **Compat:** additive now; `sub:` prefix mechanism deprecated in docs,
  removed in a later phase (owner picked over keep-forever and
  replace-in-lockstep).
- **Frontend scope:** web full card + CLI lifecycle lines (owner picked over
  web-only and both-full).
- **Acceptance:** tests + one live GUI drive (owner picked over tests-only
  and tests+drive+soak).
- **Mechanism:** Approach A — typed `AgentEvent` variants at dispatch/sink
  layer; wire-projection rejected (child tokens never reach the wire layer
  today), per-delegation stream handles rejected (transport multiplexing for
  no product gain).

## Panel & review log

- **2026-07-09 — brainstorm:** decisions above; design sections approved by
  owner in-session (events/emission, wire/frontends, invariants/testing).
- **2026-07-09 — adversarial panel (4 opus reviewers: requirements /
  assumptions / failure-abuse / scope):**
  - *Blockers/majors fixed in place:* trace layer mischaracterized — §2.6
    now names `trace.rs` + `testkit.rs` as compile-forced sites with a
    record-Start/End-skip-deltas rule + exactly-once JSONL test (Assumptions
    BLOCKER-2/MAJOR-1, Failure MAJOR-4, Scope BLOCKER-1/MINOR-3); End now
    rides an RAII drop-guard so a child panic / backstop drop can't orphan a
    running card (Failure MAJOR-2) + client finalize-on-done safety net;
    `End.detail` carries the failure/timeout reason (Requirements MAJOR-1);
    `prompt` dropped from `Start` — duplicate unbounded payload
    (Scope MAJOR-5); reducer last-running discipline + fresh-card-on-reused
    id (Failure MINOR-6); per-card transcript cap (Failure MAJOR-5);
    per-delegation cost accumulated web-side from existing `parent_id`-tagged
    `server_usage` frames — no core change (Requirements MAJOR-2);
    `parent_id` move/clone mechanics + derive-free `AgentEvent` framing +
    Reasoning-is-a-new-arm corrected (Assumptions MAJOR-2/BLOCKER-1/MINOR-1);
    depth≥2 card attachment specified generically (Requirements MINOR-1);
    Start-joins-existing-row ordering pinned as a test (Requirements
    MINOR-2).
  - *Verified-true by the panel:* delegation-id join; no Err returns between
    ordinal mint and run; ToolStart-before-Start under parallel dispatch;
    sinks never nest; old/new SPA frame tolerance; CaptureSummary single
    truth; parentId nesting renders today; StopReason Copy + stop_reason_str
    coverage.
  - *Escalated to the gate:* G1 typed StreamRetry keep-vs-defer; G2 meaning
    of "full card" (flat siblings vs DOM containment); G3 reconnect posture;
    G4 child-approval attribution; G5 child-reasoning egress.
  - *Accepted residual minors:* CLI silent child-text burn (pre-existing,
    quantified by End stats); web card is live-only (named in Non-goals).
- **2026-07-09 — owner gate (all five escalations decided, each per the
  panel-synthesized recommendation):**
  - **G1 KEEP** `SubagentEvent::StreamRetry` — dropping it would re-introduce
    the duplicate-text artifact at card level that the top-level frame fixed.
  - **G2 minimal-v1 card, flat siblings** — "full card" is pinned to mean
    transcript richness (badge, status, live text + dimmed reasoning, outcome
    footer), NOT re-parenting child rows into the card's DOM; containment is
    a possible follow-up.
  - **G3 placeholder + live-only** — reducer materializes a placeholder card
    for subagent frames matching no item; no server replay buffer this phase.
  - **G4 child-approval attribution DEFERRED** — named gap in Non-goals; the
    global modal remains actionable, just unattributed.
  - **G5 child reasoning STREAMS to the UI, accepted residual** — local-first
    app, same trust boundary as the parent's own reasoning stream; recorded
    here as the explicit justification (the "no new exposure" wording the
    panel caught was corrected in §2.1/§2.3).
