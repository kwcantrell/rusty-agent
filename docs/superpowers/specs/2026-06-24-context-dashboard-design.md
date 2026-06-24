# Context Dashboard — Design

**Date:** 2026-06-24
**Status:** Approved (design); pending implementation plan
**Scope:** Rust agent (`agent/`) + web frontend (`web/`)

## Summary

Add a "context dashboard": a compact, expandable strip sitting between the
conversation and the chat composer in the agent column. It surfaces, at a
glance, how full the agent's context window is, plus the active config and
session stats.

The one genuinely new capability is **token telemetry** — the agent does not
currently report token usage over the wire. Everything else the dashboard shows
is derived from data the client already holds (`RuntimeSettings`, the connection
status, and the `Item[]` transcript).

## Goals

- Show a live context-window gauge: prompt tokens used vs `context_limit`.
- Show, on expand, the active config snapshot (model, temperature, active
  skills) and session stats (turns, tool calls, artifacts).
- Keep the composer prominent — the strip is one line when collapsed.
- Add the telemetry as a real backend signal, not a client-side estimate.

## Non-goals (v1)

- Completion-token / per-turn cost accounting (noted future extension).
- Historical usage graphs over time.
- Editing config from the dashboard (Settings UI already owns that).

## Data flow

```
AgentLoop.run()  ──build context──►  estimate_tokens (already exists)
       │
       └─ emit AgentEvent::Usage { prompt_tokens, context_limit, turn, max_turns }   ← NEW variant
                   │
       wire_event_from ──► WireEvent::Usage { prompt_tokens, context_limit, turn, max_turns }   ← NEW
                   │
              WS envelope ──► client parseInbound
                   │
        reducer stores latest usage on ConversationState   ← NEW field
                   │
        ContextDashboard reads usage + settings + derived stats   ← NEW component
```

### Why server-side usage, not a client estimate

The backend already runs `estimate_tokens` (`agent-core/src/context.rs`,
~4 chars/token) over the **actually-built** context every turn inside
`WindowContext::build`. That count is authoritative: it reflects post-eviction
trimming, which a client-side guess could never reproduce. We emit one `Usage`
event per turn, computed from the same per-message estimate the window manager
uses, so the gauge always matches what was really sent to the model.

`prompt_tokens` = built-context token sum for the turn.
`context_limit` = `model_limit` (the configured `context_limit`).
`turn` = 1-based current tool-loop iteration within this run (`_turn + 1`).
`max_turns` = the run's tool-loop budget (`config.max_turns`).

**Note on "turn" semantics:** the agent emits one `AgentEvent::Done` per
user-message *exchange* (so the client's `turnIndex` counts completed
exchanges), whereas `max_turns` is the agent's *tool-loop iteration budget per
run* — a different counter. To avoid mixing the two, the dashboard's "turns
N/max" reads the live `turn`/`max_turns` carried on the `Usage` event itself.
These reset at the start of each run and climb as the agent loops through tool
calls, pairing naturally with the context gauge.

## Backend changes (Rust)

1. **`agent-core/src/context.rs`** — expose a small helper that sums the
   per-message token estimate over a built `&[Message]`, reusing the existing
   `message_tokens` logic (currently private). This avoids duplicating the
   estimate in the loop.
2. **`agent-core/src/event.rs`** — add variant
   `AgentEvent::Usage { prompt_tokens: usize, context_limit: usize, turn: usize, max_turns: usize }`.
3. **`agent-core/src/loop_.rs`** — in the turn loop in `run()`, after
   `ctx.build(self.config.model_limit)`, compute the built-context token sum and
   `sink.emit(AgentEvent::Usage { prompt_tokens, context_limit, turn: _turn + 1, max_turns })`
   once per turn (rename `_turn` to `turn` so it can be read).
4. **`agent-server/src/wire.rs`** — add
   `WireEvent::Usage { prompt_tokens: usize, context_limit: usize, turn: usize, max_turns: usize }`
   and the `AgentEvent::Usage => WireEvent::Usage { .. }` arm in `wire_event_from`.
   Serializes as `{"type":"usage", ...}` (snake_case, matching siblings).

## Frontend changes (TypeScript / React)

1. **`web/src/wire.ts`** — add
   `{ type: "usage"; prompt_tokens: number; context_limit: number; turn: number; max_turns: number }`
   to the `WireEvent` union.
2. **`web/src/state.ts`**
   - Add
     `usage: { promptTokens: number; contextLimit: number; turn: number; maxTurns: number } | null`
     to `ConversationState`.
   - Handle the `usage` event in `reduceFrame`: store the latest value only. It
     is **not** an `Item` and is never rendered in the transcript.
   - Reset `usage` to `null` in `initialState` / the `reset` action.
3. **`web/src/components/ContextDashboard.tsx`** (new) — the compact expandable
   strip.
   - **Collapsed (always visible):** status dot, `12.4k / 128k` context figure,
     a thin progress bar, percent, and a `▸` expand affordance.
   - **Expanded:** context gauge + `model · temp` + `turns N/max · T tools ·
     A artifacts` + active skills. (`turns N/max` from the `usage` event's
     `turn`/`maxTurns`; tools and artifacts derived from `items`.)
   - Expanded/collapsed state persisted to `localStorage`, matching the existing
     workspace mode/viewport persistence pattern. Default collapsed.
4. **`web/src/components/AgentColumn.tsx`** — render `<ContextDashboard/>`
   between `ApprovalPrompt` and `Composer`. Threads `usage`, `settings`,
   `settingsMeta`, and `items` (for derived counts) via props from `App.tsx`.
5. **Derived stats (no new data):** tool count from
   `items.filter(it => it.kind === "tool")`; artifacts from the existing
   `artifactsFrom(items)`. Turn position comes from the `usage` event (see
   above), not the client's `turnIndex`.

## Edge cases & styling

- **No usage yet** (before first turn): gauge shows a muted "—" state; config
  and stats still render from settings.
- **Near / over limit:** gauge color shifts from `--accent` to a warning tone
  past ~80% so eviction pressure is visible. (Threshold: 80%.)
- **Settings not loaded:** strip still renders the gauge; config rows show muted
  placeholders.
- Reuses existing CSS vars (`--surface-overlay`, `--border`, `--text-muted`,
  `--accent`) and `font-mono` for numerals, consistent with `AgentHeader` and
  `Composer`.

## Form factor

Compact strip, expandable (chosen over a multi-stat grid and a gauge-only
strip):

```
┌─ conversation ────────────┐
│  ...messages...           │
├───────────────────────────┤
│ ● 12.4k/128k ███░░░░ ·8% ▸ │  ← collapsed strip
├───────────────────────────┤
│ [ Message the agent… ] Send│
└───────────────────────────┘

expanded ▾:
│ ctx 12.4k/128k ███░░ ·8%      │
│ model gpt-4o · temp 0.7      │
│ turns 3/20 · 5 tools · 2 art │
│ skills: search, files        │
```

## Testing

- **Rust**
  - `agent-core`: unit test that the loop emits a `Usage` event with
    `0 < prompt_tokens <= context_limit` and `1 <= turn <= max_turns` (extend
    the existing `testkit` event-string assertions).
  - `agent-server/wire.rs`: test that `AgentEvent::Usage` maps to a payload
    serializing with `"type":"usage"` and round-trips the two fields.
- **TypeScript**
  - `state` reducer test: a `usage` frame updates `state.usage`; `reset` clears
    it to `null`.
  - `ContextDashboard` render test: collapsed shows the gauge; expand toggles
    the panel; derived counts (turns/tools/artifacts) are correct. Follows the
    existing `*.test.tsx` patterns.

## Risk / blast radius

Tightly scoped. One new wire event reusing an existing estimator; one new
component in an existing layout slot; no change to the token/reasoning/tool
streaming path. The new `WireEvent` variant is additive: the envelope is still a
valid `event`-kind frame that `parseInbound` accepts, and the client we ship
adds the matching `usage` case to `reduceFrame`. (Note: `reduceFrame`'s inner
`switch (p.type)` currently has no default arm, so an *un-updated* client would
return `undefined` on an unknown event type — we are updating the client in
lockstep, so this is not a concern here, but the variant should ship to client
and server together.)
