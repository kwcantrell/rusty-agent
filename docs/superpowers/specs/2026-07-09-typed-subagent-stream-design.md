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
  the wire/frontends.
- No removal of the `sub:`/`sub{n}:` prefix mechanism this phase (deprecated
  in doc comments only; owner decision "additive now, deprecate later").
- No per-delegation transport multiplexing (deepagents' handle *API shape* is
  not ported; a flat tagged stream reconstructs the same tree client-side).

## 2. Design

### 2.1 Event model (agent-core)

New `AgentEvent::Subagent(SubagentEvent)` variant, following the
`Context(ContextEvent)` precedent. Every case carries the **delegation id**
= the dispatching call's on-wire id, `format!("{id_prefix}{call_id}")` —
exactly the string stamped as `parent_id` on forwarded child rows today. This
joins the typed stream to the existing `dispatch_agent` tool row for free and
composes through nesting: a grandchild's delegation id is already fully
qualified (`sub{n}:c3`) because `nested.id_prefix = "sub{n}:"` (spec G8).

```rust
#[derive(Debug, Clone)]
pub enum SubagentEvent {
    /// One dispatch began. Emitted after all validation (a rejected dispatch
    /// never emits Start) and after the loop's own ToolStart for the
    /// dispatch_agent call, so frontends always see the host row first.
    Start {
        id: String,
        /// Registry name, or "general-purpose" for the ad-hoc path.
        subagent_type: String,
        /// The per-call role arg (general-purpose only; None for named types,
        /// which ignore role).
        role: Option<String>,
        /// The dispatch prompt, unabridged (ToolStart args already carry it
        /// on the wire today — no new exposure).
        prompt: String,
    },
    /// Child assistant text delta.
    Text { id: String, text: String },
    /// Child reasoning delta.
    Reasoning { id: String, text: String },
    /// Child mid-stream retry retracted in-flight deltas: frontends trim the
    /// tail of THIS delegation's transcript (mirrors top-level StreamRetry).
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
        turns: usize,
        tool_calls: u64,
        duration_ms: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentOutcome { Completed, Timeout, Failed, Cancelled }
```

### 2.2 Emission (dispatch.rs only)

Two emitters, both already holding everything they need:

- **`execute()`** emits `Start` to `self.deps.sink` immediately after the
  delegation id (`parent_id` local) is computed — i.e. after every
  validation/`Err` return (`subagent_type` resolution, `child_policy` parse,
  tools allowlist) — and before `run_with_cancel`. It emits `End` on **all
  four exit paths**, with a duration measured (`Instant`) around the run:
  - timeout → `outcome: Timeout` (before returning `failure_output`),
  - fatal child error (`Ok(Err(e))`) → `Failed`,
  - parent-cancel (`ctx.cancel.is_cancelled()` after a clean run) →
    `Cancelled` (before the `Err` return),
  - normal completion → `Completed`.
  `stop`/`turns`/`tool_calls` come from the existing `CaptureSummary` (the
  same source the tool-result footer uses — one truth).
- **`SubagentSink`** gains three forwarding arms inside today's catch-all:
  child `Token` → `Subagent(Text)`, child `Reasoning` → `Subagent(Reasoning)`,
  child `StreamRetry` → `Subagent(StreamRetry)` — each stamped with the
  sink's `parent_call_id` (the delegation id). **Capture behavior is
  unchanged**: Token still appends to segments, StreamRetry still trims them
  — the sink now captures *and* forwards. Child `Done`/`Error`/`Context`/
  `Usage` stay off the stream exactly as today, and the child-trace tap keeps
  recording every non-forwarded event unchanged (raw `Token`/`Reasoning`/
  `StreamRetry` remain tap-recorded even though a typed twin is now also
  forwarded — trace format compatibility).

**No sink chaining concern:** `SubagentSink`s never nest — every dispatch at
any depth wraps `self.deps.sink`, which is the top-level sink (nested deps
clone it untouched; only `id_prefix` composes). Typed events emitted at any
depth therefore reach the top-level sink directly; no pass-through arms are
needed, and a nested delegation's events carry its qualified id whose prefix
ties it to its host row (client-side tree reconstruction).

Note that `SubagentSink` never *receives* a `Subagent(...)` event: a nested
`execute()` emits to its `deps.sink` directly (the top-level sink), and the
child loop itself never produces `Subagent` events, so the sink's match needs
no dedicated arm — a hypothetical one would fall to the catch-all (trace
tap). A test pins this absence.

### 2.3 Wire (agent-server)

Five additive `ServerEvent` frames, mapped 1:1 in `server_event_from`:

| frame | payload |
|---|---|
| `subagent_start` | `id`, `subagent_type`, `role?`, `prompt` |
| `subagent_text` | `id`, `text` |
| `subagent_reasoning` | `id`, `text` |
| `subagent_stream_retry` | `id`, `discarded_text_chars`, `discarded_reasoning_chars` |
| `subagent_end` | `id`, `outcome` (`"completed"\|"timeout"\|"failed"\|"cancelled"`), `stop?` (via `stop_reason_str`), `turns`, `tool_calls`, `duration_ms` |

None-valued optionals are omitted (`skip_serializing_if`), matching every
existing frame's pins. Old SPAs ignore unknown `type` tags (the
`stream_retry` precedent); every existing frame stays byte-identical.

### 2.4 Web SPA (the product surface)

Typed frames **enrich the existing `dispatch_agent` tool item** — no new
item kind, no double-render:

- Reducer: `subagent_*` frames locate the tool item whose `id` equals the
  delegation id and grow an optional `subagent` field:
  `{ subagentType, role?, status: "running"|"done", text, reasoning,
  outcome?, stop?, stats? }`. `subagent_text`/`subagent_reasoning` append;
  `subagent_stream_retry` trims the transcript tail char-wise (same logic as
  the top-level `stream_retry` handler); `subagent_end` sets
  `status: "done"` + outcome/stats. Frames whose id matches no item are
  dropped (old-history restore never contains them).
- Rendering: `AnimatedToolCall` grows a card variant when `subagent` is
  present — header (name badge + status), collapsible live child transcript
  (streaming text, dimmed reasoning), and the child tool rows that already
  nest under the row via `parentId` render inside the card (they are the
  same `sub:`-prefixed rows as today — one render path). Outcome footer on
  end (stop reason, turns, tools, duration).
- Fallbacks: a dispatch row that never receives typed frames (old server,
  restored history) renders exactly as today.

### 2.5 CLI

`TerminalSink` (matches `AgentEvent` directly) gains a `Subagent` arm:
- `Start` → line like `↳ agent[research-critic] started` (name = subagent_type,
  role appended for general-purpose when present).
- `End` → `↳ agent[research-critic] done — stop, 4 turns, 7 tools, 12.3s`
  (outcome word when not Completed: `timed out` / `failed` / `cancelled`).
- `Text`/`Reasoning`/`StreamRetry` → consumed silently (interleaved parallel
  child prose is terminal noise; owner decision).
Existing `↳` child tool rows unchanged.

## 3. Do-not-regress invariants

1. **Every pre-existing wire frame byte-identical.** The `sub:`/`sub{n}:`
   renaming, `parent_id` tagging, and None-omission pins are untouched; new
   frames are additive-only. The prefix mechanism is marked deprecated in doc
   comments; removal is a future phase.
2. **No loop / calibration / estimator / pinned-block changes.** Scope is
   dispatch.rs, event.rs, wire.rs, web, CLI.
3. **Sub-agent context quarantine preserved.** Typed events are sink-side
   telemetry; nothing enters any `CuratedContext` or the model handoff. The
   dispatch tool-result content is byte-identical (3B-1b sever/footers).
4. **Child-trace tap unchanged** — same events recorded as today;
   replayability preserved.
5. **Cancellation/timeout semantics unchanged** — `End` observes outcomes,
   never alters control flow.
6. **`CaptureSummary` is the single stats truth** — footer and `End` cannot
   disagree.

## 4. Backward compatibility

- Old SPA / old CLI: additive frames are ignored (unknown-type precedent);
  behavior identical to today.
- New SPA against an old server: card fields simply never populate; rows
  render as today.
- Trace consumers: `AgentEvent` gains a variant; the trace writer serializes
  it like any event. Child-trace records are unchanged.

## 5. Testing & acceptance

- **dispatch.rs unit tests:** Start carries registry name vs
  `general-purpose`+role; Start absent on arg-rejected dispatch; End on all
  four exit paths with correct outcome/stop/stats (scripted child: normal,
  fatal, cancel; timeout via tiny `ctx.timeout`); one emission asserts BOTH
  forward (typed Text) and capture (segments) so the dual role can't split;
  StreamRetry forwards typed AND trims capture; nested dispatch emits fully
  qualified delegation ids (`sub{n}:` composition).
- **wire.rs pins:** per-frame type tags, None-omission, outcome/stop strings.
- **web tests:** reducer — card assembly on the dispatch row, parallel
  children interleaved by id, retry trim, no-typed-frames fallback;
  component test for the card variant.
- **CLI render tests:** start/end lines; Text/Reasoning silent.
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
- *Adversarial panel: pending.*
