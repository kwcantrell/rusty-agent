# Sub-agent dispatch — surfaces & observability (sub-spec #2)

**Date:** 2026-07-02
**Cluster:** sub-agent orchestration capability, sub-spec #2 of 3 (decomposition
recorded in `docs/superpowers/specs/2026-07-01-subagent-dispatch-core-design.md`,
merged `af4dd14`). Sub-spec #3 (model routing, role prompts, depth>1) stays out.
**Charter (from sub-spec #1):** first-class parent/child attribution on the wire,
per-child trace linkage, CLI indented rendering, web nesting, stats attribution
instead of fold-in, `ToolCtx.call_id` for real id lineage. Plus one folded
residual assigned here by the #1 final review: the `tools` allowlist rejects
context-tool names the child implicitly has.

## Invariant

Attribution is **additive and lossless**: every surface keeps exactly the
sub-spec #1 behavior when it doesn't understand the new fields. Concretely, the
deployed (old) web SPA — which duck-types `parseInbound` and ignores unknown
JSON fields (`web/src/wire.ts:81-97`) — renders a session with sub-agents
byte-identically to v1; old trace readers see the same records they saw before
(new fields are `skip_serializing_if = None`); and no event that v1 suppressed
starts reaching any frontend.

## Verified live-source facts the design rests on

- v1 forwarding (`agent-core/src/dispatch.rs:85-137`): `SubagentSink` forwards
  ToolStart/ToolResult with `sub{n}:{id}` / `sub:{name}` rewrites and
  ServerUsage verbatim; suppresses Token/Reasoning/Usage/Done/Error/Approval/
  Context/SandboxDegraded. Nothing ties `sub{n}` to the dispatch call's id.
- `ToolCtx` is `{workspace, timeout, cancel, sandbox}` — no call id; `gate_tool`
  has `call.id` in hand when it builds the ctx (`loop_.rs:684-696`). 31 literal
  `ToolCtx {` construction sites across 15 files (grep-verified) — a mechanical
  sweep, no builder needed.
- Old-SPA tolerance: `parseInbound` JSON-parses and switches on `kind`/`type`
  only; **extra fields on known frames flow through and are ignored**
  (`wire.ts:81-97`). Unknown top-level `type`s still reduce to no-op (pinned by
  the forward-compat test). So additive optional fields are the only wire
  mechanism that is both structured and old-SPA-safe.
- The web reducer correlates `tool_result` to the last **running item with the
  same name** (`state.ts:141-152`) — the observability cluster's open backlog
  item ("id-based tool correlation; id is on the wire"). Parallel same-named
  child tools make this correlation actively wrong, so this sub-spec closes it.
- `server_usage` reducer arm overwrites `{promptTokens, turn}` unconditionally
  (`state.ts:117-118`) — the documented v1 "turn readout flicker" when a child's
  ServerUsage arrives.
- `SessionStats::fold` (`stats.rs`): ToolStart → `tool_calls`; ServerUsage →
  token/cost sums + `turns = max(turn)` (child turn indices pollute `turns` —
  documented v1 distortion); surfaces: `render::format_stats_line` (CLI,
  `main.rs:307-308`) and `web/src/components/StatsPanel.tsx` rows.
- Trace: `TraceRecord{seq, ts_ms, event}` + `TraceEvent` serializable mirror of
  AgentEvent (`trace.rs:140-202`), written by `ObservedSink`. Forwarded child
  events are traced (rewritten ids) but unattributed; suppressed child events
  (the child's actual transcript) are traced **nowhere** — a failed child turn
  cannot be replayed.
- Dep direction: `agent-runtime-config` (owns TraceWriter) depends on
  `agent-core` (owns SubagentSink) — a child-trace hook must be a trait defined
  in agent-core, implemented in runtime-config (the `Retriever`/`EventSink`
  pattern).

## Decisions

- **E1 — Attribution rides existing frames as optional fields.**
  `AgentEvent::ToolStart/ToolResult/ServerUsage` gain `parent_id:
  Option<String>` (`None` everywhere except SubagentSink forwards). The wire
  `ServerEvent` and trace `TraceEvent` mirrors gain the same optional field with
  `#[serde(skip_serializing_if = "Option::is_none")]` — a v1 session serializes
  byte-identically. Rejected: a new top-level frame (silently invisible to the
  old SPA — the exact failure the observability cluster's release note warns
  about); `Context`-kind lifecycle events (stringly, unstructured); a wrapper
  `AgentEvent::Child{inner}` variant (viral through every match arm).
- **E2 — `ToolCtx.call_id: String`.** `gate_tool` fills it with `call.id`; the
  31 literal sites sweep mechanically (tests pass a fixed id). This is the
  lineage root: the dispatch tool sets `parent_id = ctx.call_id` on everything
  it forwards. The `sub{n}` ordinal **stays** as the id-uniqueness prefix —
  parent call ids are only per-turn unique (`normalize_tool_call_ids`), so they
  cannot replace it.
- **E3 — `SubagentSink::new(parent, n, parent_call_id, child_trace)`.**
  Forwards ToolStart/ToolResult/ServerUsage with `parent_id:
  Some(parent_call_id)`; the `sub{n}:` id and `sub:` name rewrites stay (the
  old SPA's and CLI's only hierarchy hint; the new SPA strips `sub:` for
  display). No new event kinds reach the frontend.
- **E4 — Per-child trace linkage (full child transcript).** New agent-core
  trait: `pub trait SubagentTrace: Send + Sync { fn record(&self, ordinal: u64,
  event: &AgentEvent); }`. `DispatchDeps` gains `child_trace:
  Option<Arc<dyn SubagentTrace>>`; `SubagentSink` calls it for **suppressed
  events only** (Token/Reasoning/Usage/Done/Error/Approval/Context/
  SandboxDegraded) — forwarded events are already traced via ObservedSink with
  `parent_id`, so the session trace gains the child's full transcript with zero
  duplication. runtime-config implements the trait on a small adapter over
  `TraceWriter` (`TraceRecord` gains `#[serde(skip_serializing_if)] sub:
  Option<u64>`, set to the dispatch ordinal); `assemble_loop` wires it from
  `parts.trace` (None when tracing is off). Rejected: per-child trace *files*
  (cross-file correlation burden, retention interplay — one attributed session
  file replays fine).
- **E5 — Stats attribution instead of distortion.** `SessionStats` gains
  `subagent_tool_calls: u64` and `subagent_turns: u64` (serde-additive; the
  wire `session_stats` frame carries them — old SPA ignores). `fold()`:
  ToolStart with `parent_id` increments **both** `tool_calls` and
  `subagent_tool_calls` (subset semantics — totals stay totals); ServerUsage
  with `parent_id` sums tokens/cost/turn-time as today (billed truth) and
  increments `subagent_turns`, but **no longer bumps `turns = max(turn)`** —
  the v1 turns distortion is gone. ToolResult status counters stay unified.
  Surfaces: `format_stats_line` appends `sub-agent: {calls} calls/{turns}
  turns` and StatsPanel adds a `Sub-agent` row — both only when nonzero.
- **E6 — Web: id-correlation + flat nesting.** (a) `tool_result` correlates by
  **id first** (fall back to name-match only for items that predate ids in
  state) — closes the observability backlog item and is required now that
  parallel same-named child tools exist. Tool items store `id` and `parentId`.
  (b) `tool_start` with `parent_id` renders as a **nested child row**: the
  items list stays flat, the item carries `parentId`, and the renderer indents
  it under the dispatch row (display name strips the `sub:` prefix). If no
  running dispatch item matches `parent_id`, append flat (graceful, not an
  error). (c) `server_usage` with `parent_id` does **not** touch the turn
  readout — the flicker fix. Rejected: a dedicated sub-agent panel (YAGNI —
  inline nesting is where the user's eye already is); a nested `children`
  array on items (reducer churn, ordering questions; flat+indent is one field).
- **E7 — CLI: indent child rows.** `TerminalSink` prints ToolStart/ToolResult
  with a two-space `↳` indent when `parent_id.is_some()`; name otherwise
  unchanged (`sub:` prefix stays greppable). No other CLI changes.
- **E8 — Allowlist accepts implicit context tools (folded residual).** The
  `tools` arg validation accepts `context_recall`/`context_compact` (they are
  always registered for the child); the unknown-name error lists them as
  "always available"; the schema description mentions it. Validation still
  rejects genuinely unknown names.
- **E9 — What still stays private.** Child Token/Reasoning text never reaches
  any frontend (D9 rationale intact — the parent transcript must not be
  corrupted; the nested tool rows are the progress signal). Live child text
  streaming, if ever wanted, is future work beyond sub-spec #3.

## Section 1 — Events + gate (agent-core, agent-tools)

- `agent-tools/src/types.rs`: `ToolCtx` gains `pub call_id: String`.
- `agent-core/src/event.rs`: `parent_id: Option<String>` on the three variants.
- `agent-core/src/loop_.rs`: `gate_tool` sets `call_id: call.id.clone()`; the
  loop's own emissions set `parent_id: None`.
- Sweep: 31 `ToolCtx{` literals + every `ToolStart/ToolResult/ServerUsage`
  construction/match site (loop, testkit CollectingSink label untouched —
  labels don't include parent_id).

## Section 2 — Dispatch plumbing (agent-core/src/dispatch.rs)

- `pub trait SubagentTrace` (E4). `DispatchDeps` + `child_trace` field.
- `SubagentSink::new(parent, n, parent_call_id: String, child_trace:
  Option<Arc<dyn SubagentTrace>>)`; forwarding arms add `parent_id`; the
  suppression arm becomes `other => { if let Some(t) = &self.child_trace {
  t.record(self.n, &other) } }` — capture arms (Token/Usage/Done) also record
  to the tap before/after capturing (the tap sees the FULL child stream minus
  the three forwarded kinds).
- `execute()` passes `ctx.call_id.clone()` and `self.deps.child_trace.clone()`.
- E8 allowlist change in `execute()` + schema description line.

## Section 3 — Trace + assembly (agent-runtime-config)

- `trace.rs`: `TraceRecord.sub: Option<u64>` (skip-if-None); `TraceEvent`
  ToolStart/ToolResult/ServerUsage gain `parent_id: Option<&'a str>`
  (skip-if-None); `TraceWriter::record` keeps its signature (sub: None);
  new `TraceWriter::record_child(&self, ordinal, &AgentEvent)`; adapter
  `struct ChildTraceTap(Arc<TraceWriter>)` implementing `SubagentTrace`.
- `assemble.rs`: `child_trace: parts.trace.clone().map(|t|
  Arc::new(ChildTraceTap(t)) as Arc<dyn SubagentTrace>)` into `DispatchDeps`.
- `wire.rs` (agent-server): `ServerEvent` ToolStart/ToolResult/ServerUsage gain
  `parent_id` (skip-if-None); `server_event_from` maps it through.

## Section 4 — Stats (agent-core + surfaces)

Per E5: two fields, fold arms keyed on `parent_id.is_some()`,
`format_stats_line` suffix, StatsPanel row, web `SessionStats` TS type fields.

## Section 5 — Web (web/src/)

Per E6: `wire.ts` optional `parent_id` on the three frame types + `id` on
tool items; `state.ts` id-first correlation, `parentId` on tool items,
`server_usage` guard; renderer indent + `sub:` strip; TS `SessionStats` +
StatsPanel row.

## Error handling & edge cases

- `parent_id` on a frame the old SPA reads: ignored (duck-typed parse) — v1
  rendering, pinned by an explicit test.
- Child row arriving after its dispatch row already resolved (late forward
  vs. parent ToolResult ordering): the dispatch row is no longer `running` —
  the web falls back to flat append; CLI just prints. Never an error.
- Two open dispatch calls with the same wire id (cross-turn reuse of `call_1`):
  the earlier is resolved by the time the later opens (per-turn drain), so
  "most recent running item with this id" is unambiguous.
- Trace disabled (`parts.trace = None`): `child_trace = None`; SubagentSink
  skips the tap — no behavior change from v1.
- A tap write failing is already TraceWriter's silent-disable domain (64 MB
  cap); no new failure surface.

## Testing

- agent-core unit: SubagentSink forwards with `parent_id` (exact-triple test
  extended); tap receives exactly the suppressed kinds (mock `SubagentTrace`
  recording (ordinal, discriminant) — pins no-duplication); capture behavior
  unchanged.
- agent-core integration (`dispatch_tool.rs`): forwarded events carry
  `parent_id == ctx.call_id` end-to-end; `gate_tool` fills `call_id` (extend
  the timeout_override probe file with a call_id probe); allowlist accepts
  `context_recall` and still rejects unknowns (E8).
- stats: fold ToolStart/ServerUsage with parent_id → subset counters + turns
  unbumped; without → byte-identical to v1 (regression pin).
- runtime-config trace: `record_child` line carries `"sub":N`; parent-stream
  record with parent_id serializes it; a None parent_id serializes WITHOUT the
  key (old-schema pin, assert on the raw JSON line).
- agent-server wire: `server_event_from` maps parent_id; None → key absent in
  the serialized frame (old-SPA byte-compat pin).
- agent-cli render: child ToolStart/ToolResult lines indented (snapshot-ish
  string assert).
- web vitest: id-first tool_result correlation with two same-named running
  tools; nested `parentId` item + indent + `sub:` strip; `server_usage` with
  parent_id leaves the turn readout unchanged; frame WITH parent_id through
  the old reducer path (forward-compat: no crash, flat render); StatsPanel
  Sub-agent row only when nonzero.
- `bash scripts/ci.sh` green.

## Files touched

- `agent/crates/agent-tools/src/types.rs` (ToolCtx.call_id) + sweep sites.
- `agent/crates/agent-core/src/event.rs`, `loop_.rs`, `dispatch.rs`,
  `stats.rs`, testkit if labels shift, tests.
- `agent/crates/agent-runtime-config/src/trace.rs`, `assemble.rs`, tests.
- `agent/crates/agent-server/src/wire.rs`, tests.
- `agent/crates/agent-cli/src/render.rs`, tests.
- `web/src/wire.ts`, `state.ts`, `components/StatsPanel.tsx`, the item
  renderer component, tests.

## The 20% (human-judgment points)

The flat-list+`parentId` web nesting model (vs. true tree), the
suppressed-only tap split (no-duplication vs. one-contiguous-stream), and
keeping the `sub:`/`sub{n}:` rewrites alongside structured attribution are
judgment calls — they are the review points for this spec.

## Out of scope (recorded residuals)

- Sub-spec #3: per-child model routing, role prompts, depth>1, fan-out
  ergonomics.
- Live child token streaming to any frontend (E9).
- Per-dispatch token roll-up on the web dispatch row (E6c — YAGNI'd).
- Persistent offload store / child transcript beyond the session trace file.
- v1's remaining accepted residuals not named here (stale IPC prompt on
  dispatch timeout; TerminalApproval orphan race) — unchanged.
