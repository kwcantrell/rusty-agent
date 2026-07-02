# Sub-agent dispatch — core tool

**Date:** 2026-07-01
**Cluster:** harness deep audit, missing capability #1 — sub-agent /
decompose-and-delegate orchestration (Component 4 build opportunity; the audit's
"two capability categories are absent entirely" item, first of two).
**Audit:** `docs/superpowers/audits/2026-07-01-harness-deep-audit.md`.
**Playbook:** `.agents/skills/harness-engineering/build.md` — patterns applied:
orchestrator-workers, **sub-agents-as-tools**, one-feature-at-a-time.

## Capability decomposition

One spec cannot cover the whole capability. Split (per build playbook
"one-feature-at-a-time"); **this spec is sub-spec #1 only**:

1. **Core dispatch tool (THIS SPEC)** — a `dispatch_agent` tool that runs a
   nested `AgentLoop` with a fresh context and a scoped registry, inheriting the
   parent's policy/approval/sandbox, returning the child's final text as the
   tool result. Minimal, forward-compatible observability on existing frames.
2. **Surfaces & rich observability (later)** — first-class parent/child
   attribution on the wire (nested event hierarchy, per-child trace linkage,
   CLI indented rendering, web sub-agent panel, stats attribution instead of
   fold-in), `ToolCtx.call_id` for real id lineage.
3. **Advanced dispatch (later)** — per-child model routing (whitepaper p42;
   first customer `run_compaction`), role/agent-type prompts (skill-defined
   agents), depth>1 with a depth budget, fan-out ergonomics.

## Invariant

A sub-agent is **never more privileged than its parent**. Concretely: the child
loop holds the *same* `Arc<dyn PolicyEngine>` and `Arc<dyn ApprovalChannel>`
instances as the parent (Ask prompts reach the parent's operator; the Destroy
tier and hard floor apply unchanged), the same `Arc<dyn SandboxStrategy>`, and a
tool registry that is a **subset** of the parent's — never containing
`dispatch_agent` itself (structural recursion depth 1).

## Verified live-source facts the design rests on

- `AgentLoop::run_with_cancel` returns `Result<(), AgentError>` — **no final
  text**; the child's answer exists only as `Token` events through its sink
  (`agent-core/src/loop_.rs:288-293`, emission at `:176`).
- Every collaborator is already `Arc<dyn …>` and `Send + Sync`-shareable:
  model (`loop_.rs:113`), protocol (`:114`), registry (`:115`), policy (`:116`),
  approval (`:117`), sink (`:118`), sandbox (`LoopConfig.sandbox`, `:65`).
- `ToolCtx` is `{workspace, timeout, cancel, sandbox}` (`agent-tools/src/types.rs:131-136`)
  — no sink/model/call-id; a dispatch tool must receive its dependencies at
  construction (the `context_tools` pattern, `assemble.rs:102-108`).
- `gate_tool` builds `ToolCtx` with `timeout: self.config.tool_timeout`
  (`loop_.rs:686`), fixed at 120 s (`assemble.rs:69`); `execute_isolated` arms a
  2× backstop (`loop_.rs:762`). A multi-minute child dies at 240 s without a
  per-tool override.
- Parallel calls ride `buffer_unordered(cap)` (`loop_.rs:485-502`); each call is
  panic/timeout-isolated. Two dispatch calls in one turn run concurrently for
  free.
- `IpcApprovalChannel` correlates concurrent requests by id
  (`agent-server/src/approval.rs:16,44`); `TerminalApproval` is a blocking
  stdin read with **no** serialization between concurrent requesters
  (`agent-cli/src/approval.rs:33-64`) — two children asking at once would
  interleave prompts.
- Old-SPA forward compat: unknown **top-level** wire event types are silently
  dropped by the deployed web reducer (`web/src/state.ts:174-175`, pinned by
  test); unknown **context kinds** render opaquely (`state.ts:63`). Existing
  `tool_start`/`tool_result`/`server_usage` frames render natively.
- `SessionStats::fold` sums whatever reaches the `ObservedSink`
  (`agent-core/src/stats.rs`); child events forwarded there are counted.
- `CuratedContext::new(system, store, flag)` + `with_offload_config`
  (`agent-core/src/curated.rs:40-76`); the parent's `context_recall`/
  `context_compact` tools are **bound to the parent's** store/flag — a child
  must get its own pair or eager-offloaded child results become unrecallable.
- `RuntimeConfig` additive-field pattern: serde default fn + partial-merge arm +
  default/merge tests (`runtime_config.rs:27-28,120-122,316-318`).
- No sub-agent/dispatch code exists anywhere (grep-verified).

## Decisions

- **D1 — Shape: sub-agents-as-tools.** `DispatchAgentTool` (name
  `dispatch_agent`) in a new `agent-core/src/dispatch.rs`. Rejected: a
  loop-level spawn primitive outside the tool vocabulary (the model can't drive
  it through existing protocols; invasive) and server-side session spawning
  (surface-specific; CLI gets nothing).
- **D2 — Policy/approval inheritance by Arc identity.** The child receives the
  parent's exact `policy`/`approval` Arcs. No child-local config can widen them.
- **D3 — Dispatch itself is `Access::Read`** (path-less, command-less intent →
  auto-allow, like `recall`): spawning computation is not an effect; every
  effectful child action is gated by the same policy + approval as the parent.
  Cost exposure is bounded by D6's budgets. Pinned by test.
- **D4 — Structural no-recursion.** The child registry is built from a snapshot
  of the parent's tools taken **before** `dispatch_agent` (and before the
  parent's context tools) are registered. No depth counter to get wrong;
  depth>1 is sub-spec #3.
- **D5 — Fresh, fully-owned child context.** Per call: fresh
  `InMemoryOffloadStore` + fresh `compact_flag` + fresh `CuratedContext` with an
  `OffloadConfig` honoring `cfg.max_tool_result_bytes`, plus **child-bound**
  `context_tools` registered into the child registry. Child system prompt =
  parent's composed system prompt + a fixed sub-agent preamble (const in
  `dispatch.rs`: role statement + "your last message is returned verbatim to
  the parent; no one will answer questions").
- **D6 — Budgets.** New `RuntimeConfig` fields (serde-default, partial-merge):
  `subagents: bool` (default **true**), `subagent_max_turns: usize` (default
  **10**), `subagent_timeout_secs: u64` (default **600**). Child `LoopConfig` =
  parent values except `max_turns = subagent_max_turns`.
- **D7 — Per-tool timeout override (additive trait method).**
  `Tool::timeout_override(&self) -> Option<Duration> { None }`; `gate_tool`
  uses `tool.timeout_override().unwrap_or(self.config.tool_timeout)` when
  building `ToolCtx`. `dispatch_agent` returns `Some(subagent_timeout)`. The
  existing 2× backstop then scales with it. Rejected: living inside 120 s
  (children can't do real work) and exempting dispatch from `execute_isolated`
  (breaks the isolation contract).
- **D8 — Cancellation: child token.** The child runs under
  `ctx.cancel.child_token()`; parent cancel propagates, child self-cancel (on
  timeout) never touches the parent. The tool itself honors `ctx.timeout` by
  racing the child run against `tokio::time::timeout` and cancelling the child
  on elapse (the grace-margin contract: honor your own deadline, backstop only
  for runaways).
- **D9 — Observability v1: selective forwarding on existing frames.** The child
  sink is a `SubagentSink` wrapping the parent's (Observed) sink. It **forwards**
  `ToolStart`/`ToolResult` — with id rewritten `sub{n}:{child_id}` (process-wide
  atomic dispatch counter `n`; avoids parent/child and sibling/sibling id
  collisions) and name prefixed `sub:` (display-only hierarchy hint) — and
  `ServerUsage` unchanged (child token spend is real cost; stats must not
  undercount). It **suppresses** `Token`/`Reasoning`/`Usage`/`Done`/`Error`/
  `Approval`/`Context`/`SandboxDegraded` (child text must not corrupt the
  parent's streamed transcript; child terminal events are the tool result's
  business). Zero wire/web/CLI changes: everything rides frames the deployed
  SPA already renders. Rejected: new top-level frames (silently invisible to
  old SPA) and full silence (loses live progress, trace replay, and cost
  truth). Known accepted distortions, documented in code: child tool calls fold
  into session tool counters; `SessionStats.turns = max(turn)` may reflect a
  child's turn index; the web per-turn usage readout can flicker to a child
  turn until the parent's next `Usage` corrects it.
- **D10 — Result contract.** Tool output content = concatenated `Token` text
  emitted **after the last child `ToolResult`** (i.e. the final turn's text);
  if that tail is empty, fall back to all collected text. Footer line appended:
  `[sub-agent: {turns} turns, {tool_calls} tool calls, stop: {reason}]`. On
  child `Err(AgentError)` → `ToolError`; on parent-cancel → `ToolError`
  ("cancelled"); on wall-clock elapse → `ToolError` ("timed out after …"); on
  `BudgetExhausted` → Ok with the fallback text prefixed by a
  turn-budget-exhausted note. Oversized results need nothing new — the parent's
  16 KiB ingestion cap eager-offloads them (existing machinery).
- **D11 — Optional `tools` arg.** `dispatch_agent(prompt, tools?)`: `tools` is
  an optional string-array allowlist filtering the child base registry (child
  context tools are always present). Unknown names → per-call `ToolError`
  listing available names. Scoping is for focus/token economy; **safety never
  depends on it** (D2).
- **D12 — TerminalApproval serialization.** Add an internal async `Mutex` so
  concurrent requesters (two children, or child + parent) produce sequential
  prompts instead of interleaved stdin reads. IPC needs nothing.
- **D13 — Same model/backend for the child** (same `Arc<dyn ModelClient>` and
  protocol). Routing is sub-spec #3. No memory retriever on the child (recall
  pinning is a parent concern; memory *tools* still available via the snapshot
  if enabled).

## Section 1 — `DispatchAgentTool` (agent-core)

New module `agent-core/src/dispatch.rs`. agent-core already depends on
agent-model, agent-policy, agent-tools — everything needed is in scope.

```rust
pub struct DispatchAgentTool { deps: DispatchDeps }
pub struct DispatchDeps {
    pub model: Arc<dyn ModelClient>,
    pub protocol: Arc<dyn ToolCallProtocol>,
    pub policy: Arc<dyn PolicyEngine>,
    pub approval: Arc<dyn ApprovalChannel>,
    pub sink: Arc<dyn EventSink>,          // the parent's ObservedSink
    pub base_tools: Vec<Arc<dyn Tool>>,    // pre-dispatch, pre-context snapshot
    pub child_system_prompt: String,       // composed parent prompt + preamble
    pub loop_config: LoopConfig,           // parent values; max_turns overridden
    pub max_result_bytes: usize,           // child OffloadConfig + recall pages
}
```

`execute(args, ctx)`:
1. Parse `{prompt: String, tools: Option<Vec<String>>}`; validate allowlist
   against `base_tools` names (unknown → `ToolError::InvalidArgs` listing
   available).
2. Build child registry: (filtered) base tools + fresh
   `context_tools(fresh_store, fresh_flag, max_result_bytes)`.
3. Fresh `CuratedContext::new(system(child_system_prompt), fresh_store,
   fresh_flag)` `.with_offload_config(max_result_bytes …)`.
4. `SubagentSink::new(parent_sink, next_dispatch_n())` (D9/D10 capture).
5. Child `AgentLoop::new(model, protocol, child_registry, policy, approval,
   subagent_sink, child_config)` where `child_config` clones `deps.loop_config`
   (its `max_turns` was already set to `subagent_max_turns` at assemble time).
6. `let child_cancel = ctx.cancel.child_token();` run under
   `tokio::time::timeout(ctx.timeout, child.run_with_cancel(&mut child_ctx,
   prompt, child_cancel.clone()))`; on elapse `child_cancel.cancel()` and
   return `ToolError` (timeout). Check `ctx.cancel.is_cancelled()` after return
   → `ToolError` (cancelled).
7. Assemble output per D10 from the sink's captured state.

`intent()` → `Access::Read`, `paths: []`, `command: None`, summary
`"dispatch sub-agent: {prompt ≤80 chars}"`.
`description()` — delegate an independent, multi-step subtask to an isolated
sub-agent with its own context window; the sub-agent has the same permissions
and returns its final answer as this tool's result.
`when_not_to_call()` — "Do NOT use for a single operation another tool does
directly (call that tool); do not use when the result depends on this
conversation's context (the sub-agent cannot see it); the sub-agent cannot ask
you questions."

## Section 2 — `SubagentSink` (agent-core/src/dispatch.rs)

`EventSink` impl holding `parent: Arc<dyn EventSink>`, the dispatch ordinal
`n`, and a `Mutex<Capture>` (`segments: Vec<String>` token text split at
`ToolResult` boundaries — concretely: push tokens onto the current segment;
on `ToolResult`, forward then start a new segment — plus `tool_calls: u64`,
`turns: usize` from `Usage.turn`, `stop: Option<StopReason>`).

Forwarding table (D9): `ToolStart`/`ToolResult` → rewrite id to
`sub{n}:{child_id}`, prefix name `sub:`, forward; `ServerUsage` → forward
verbatim; everything else → capture-only/suppress.

## Section 3 — Trait + gate change (agent-tools, agent-core)

- `Tool::timeout_override()` default-`None` method (additive; no impl churn).
- `gate_tool` (`loop_.rs:684-689`): `timeout:
  tool.timeout_override().unwrap_or(self.config.tool_timeout)`.

## Section 4 — Assembly + config (agent-runtime-config)

`assemble_loop` (order matters):
1. After base+MCP+memory+skill registration and **before** parent
   `context_tools` registration, snapshot `child_base: Vec<Arc<dyn Tool>>`.
2. Register parent context tools; compose `system_prompt` (existing code).
3. Build the `ObservedSink` (moves up a few lines — it must exist before the
   dispatch tool is constructed).
4. If `cfg.subagents`: build the parent `LoopConfig` once (existing
   `loop_config_from` call), then construct `DispatchAgentTool` with
   `child_base`, the composed prompt + preamble, and a **clone of that same
   `LoopConfig`** with `max_turns = cfg.subagent_max_turns` — cloning (new
   `#[derive(Clone)]` on `LoopConfig`; all fields are `Arc`/`PathBuf`/scalars)
   shares the parent's sandbox Arc instead of re-probing via a second
   `build_sandbox`, preserving the Invariant. `tool_timeout` stays the standard
   120 s (the child's *own* tools keep the normal budget; the *dispatch call's*
   budget comes from `timeout_override` =
   `Duration::from_secs(cfg.subagent_timeout_secs)`). Register the tool.
5. `RuntimeConfig`: three new fields per D6, mirroring the
   `max_tool_result_bytes` serde-default + partial-merge + `apply` pattern.

`TerminalApproval` (agent-cli): wrap `request` body in an internal
`tokio::sync::Mutex<()>` guard (D12).

## Error handling & edge cases

- **Child fatal model error** (`AgentError::Model`) → `ToolError::Execution`
  with the message; parent loop records a normal tool error, parent turn
  continues (failure isolation — one bad child never kills the batch, per
  `execute_isolated`).
- **Child panic** → caught by the parent's `execute_isolated` catch_unwind
  (`ToolStatus::Panic`), as for any tool.
- **Parent cancel mid-child** → child token cancels; child loop emits
  `Done(Cancelled)` to the SubagentSink (suppressed) and returns `Ok`; tool
  sees `ctx.cancel` cancelled and returns `ToolError` — parent is tearing down
  anyway.
- **Known pre-existing gap, unchanged:** an approval wait doesn't race
  cancellation (audit Orchestration MED); a child blocked on approval ignores
  cancel until answered. Out of scope here; recorded residual.
- **Empty final text** (e.g. budget exhausted right after tools) → fallback to
  all captured text; if still empty, content is the footer alone.
- **`subagents: false`** → tool never registered; models see no schema.
- **Nested dispatch attempt** → child registry has no `dispatch_agent`; the
  child's own gate returns the standard unknown-tool error to the child model.

## Testing

All deterministic; `testkit::Scripted` for both parent and child model turns
(the tool holds its own model Arc, so tests inject scripted clients freely).

- agent-core unit/integration: happy path (child text → tool content); final
  text = post-last-ToolResult tail; fallback on empty tail; footer format;
  child tool execution with forwarded `sub{n}:`-id / `sub:`-name events on the
  parent sink; suppression table (no child Token/Done/Context on parent sink);
  ServerUsage forwarded; policy inheritance (child Write-tool Ask reaches the
  shared approval channel; Deny → child sees denied result); no-recursion
  (child calling `dispatch_agent` gets unknown-tool); `tools` allowlist filter +
  unknown-name error; cancellation propagation (paused clock); wall-clock
  timeout → `ToolError` + child cancelled (paused clock); parallel dispatch —
  two children, distinct `sub{n}` ordinals; `Access::Read` intent auto-allowed
  under a workspace `RulePolicy` pin.
- agent-tools: `timeout_override` default is `None`.
- agent-core loop: `gate_tool` honors `timeout_override` (ToolCtx.timeout).
- agent-runtime-config: `subagents`/`subagent_max_turns`/`subagent_timeout_secs`
  defaults, partial-merge, apply; assemble registers `dispatch_agent` by
  default and omits it when `subagents: false`; snapshot excludes parent
  context tools + dispatch itself from the child base; existing contract
  ratchets (required-param descriptions, confusables) stay green with the new
  schema.
- agent-cli: concurrent `TerminalApproval::request` calls serialize (paused
  clock or channel-based pin).

## Files touched

- `agent/crates/agent-core/src/dispatch.rs` (new), `lib.rs` (export),
  `loop_.rs` (gate timeout line + `#[derive(Clone)]` on `LoopConfig`), tests.
- `agent/crates/agent-tools/src/tool.rs` (`timeout_override`).
- `agent/crates/agent-runtime-config/src/runtime_config.rs` (3 fields),
  `assemble.rs` (snapshot + construction + registration order), tests.
- `agent/crates/agent-cli/src/approval.rs` (request mutex), tests.

## The 20% (human-judgment points, per build playbook)

The final-text extraction rule (D10), the stats fold-in distortions (D9), and
the `Access::Read` posture for dispatch itself (D3) are judgment calls, not
derivations — they are the review points for this spec and the first things
sub-spec #2 revisits.

## Out of scope (recorded residuals)

- Nested wire hierarchy / trace linkage / UI attribution — sub-spec #2.
- Per-child model routing, role prompts, depth>1, fan-out ergonomics — sub-spec #3.
- Approval-wait vs cancel race — pre-existing audit finding, untouched.
- `ToolCtx.call_id` (true id lineage) — deferred to sub-spec #2 to avoid
  breaking every literal `ToolCtx` construction now.
- Persistent child transcripts (child offload store is per-call, RAM-only —
  same posture as the parent's existing RAM-only backlog item).
