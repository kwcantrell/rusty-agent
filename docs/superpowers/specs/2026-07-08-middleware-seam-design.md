# Middleware seam (deepagents refactor, Phase 1) — design

**Status:** APPROVED at spec-review gate 2026-07-08 (panel-reviewed; J1
approved, J9 resolved keep-provisional — see Panel & review log). Next:
implementation plan.
**Knowledge base:** `docs/okf/deepagents-refactor/` (commit d997eec). Design
judgments in `comparisons/refactor-priorities.md` are *unvalidated input*
(bundle log 2026-07-08) — this spec's panel must treat them as claims.
**Live-source baseline:** commit 3c930db, re-read 2026-07-08.

## 1. Problem

Cross-cutting loop behavior — memory recall injection, context-curation
maintenance, stuck detection — lives inline in `AgentLoop::run_with_cancel`
(`agent/crates/agent-core/src/loop_.rs`), and capability wiring is a
hand-ordered sequence in `assemble_loop()`
(`agent/crates/agent-runtime-config/src/assemble.rs:162`). The runtime has
trait *seams* (`ModelClient`, `ToolCallProtocol`, `PolicyEngine`, `EventSink`,
`SandboxStrategy`, `ContextManager`) but no *stack*: adding a capability means
editing the loop. deepagents demonstrates the alternative — every capability
is a middleware unit that ships tools, extends state, hooks loop nodes, and
wraps model/tool calls, composed in deterministic order by one factory
(bundle: `practices/middleware-composition.md`). Most of the gap analysis's
"absent" capabilities (planning, caching, tool-call repair, guardrail limits)
become one middleware each once the stack exists.

## 2. Goals

- G1. A `Middleware` trait in `agent-core` with the four capability surfaces:
  **node hooks**, **wrap hooks**, **tool contribution**, **state extension**.
- G2. `assemble_loop()` builds an ordered middleware stack and hands it to
  `AgentLoop`; the memory/context tool registrations and retriever wiring move
  into stack entries. Honest scope: the rest of the assembly sequence (MCP,
  skills, prompt composition, routed models, policy, sink, dispatch) stays
  imperative in Phase 1 — this is "three behaviors become stack entries," not
  a wholesale recast.
- G3. Migrate the three loop-resident behaviors into middleware with **no
  behavior change**: recall injection (`loop_.rs:475-480`), scheduled
  compaction maintenance (`loop_.rs:734-750`, `loop_.rs:1015-1030`), stuck
  detection (`loop_.rs:485-491`, `653-702`, `1007-1013`).
- G4. Sub-agent child loops (`dispatch.rs:503`) get the same migrated
  behaviors via per-child stacks — identical to today's shared-code behavior.
- G5. The full existing test suite passes with assertions unchanged
  (construction-site updates only), plus new tests for the stack mechanics.

### Non-goals (later phases, per bundle sequencing — itself a claim to review)

The Phase-2/3/4 labels below reuse `refactor-priorities.md`'s partition, which
is as unvalidated as the middleware-first claim itself — they scope what this
spec does *not* build, not a commitment to that ordering.

- No virtual filesystem / `Backend` trait (Phase 2).
- No planning tool, named sub-agent registry, cache-aware assembly, tool-call
  repair middleware, or guardrail-limit middleware (Phase 3).
- No migration of: model retry/backoff, overflow-recovery compaction,
  protocol repair, post-tool validators, budget wrap-up, token calibration —
  these stay loop-resident (§6).
- No user-configurable stacks, no same-name middleware replacement, no
  per-model `HarnessProfile`, no durable state/checkpointing.
- No change to the mid-session freeze: registry and stack still fix at
  assembly.

## 3. Do-not-regress invariants (gap analysis §"where the current design wins")

| Invariant | How this design preserves it |
|---|---|
| Goal block + folded-facts ledger | `CuratedContext` internals untouched; `set_goal` stays a loop-resident call (§6); maintenance still calls the same `ctx.maintain` |
| `ToolIntent` policy richness | Gating (`gate_tool`) untouched; wrap_tool_call sits *after* the gate, around execution only |
| Refusal-on-degraded sandbox | Sandbox surfacing (`loop_.rs:454-465`) and `SandboxStrategy` untouched |
| First-class MCP | MCP tools still registered in `assemble_loop` (not middleware-owned in Phase 1) |
| Calibrated token estimation | Calibration ratio stays private to `AgentLoop`; middleware see it only through read-only `maint_model_limit()`/`effective_model_limit()` accessors |

## 4. Alternatives considered

**A — Faithful deepagents port** (four node hooks `before_agent` /
`before_model` / `after_model` / `after_agent` + jump semantics). Rejected:
the loop's real seams don't map onto four nodes. The stuck nudge must append
*after* the turn's tool results (OpenAI-compat ordering, `loop_.rs:1004-1013`),
the stuck abort must fire *before* the assistant tool_calls message is
appended (`loop_.rs:681-694`), and the text-exit maintain fires on exactly one
of the loop's **eleven** exit points (`loop_.rs:751`; the others are lines
509, 531, 573, 577, 609, 622, 634, 693, 1076, and the 1085 fall-through).
Forcing these into four generic nodes either changes durable-history ordering
(behavior change) or smuggles loop knowledge back into middleware.

**B — Loop-native hook points on a single trait (chosen).** Keep the
deepagents semantics that are load-bearing — one trait with four capability
surfaces, deterministic stack order, first-outermost wrap nesting, tools
entering the registry because a middleware declares them — but name the node
hooks after the loop's actual seams. The port is structural, not nominal.

**C — tower-style `Layer`/`Service` composition.** Idiomatic Rust for wrap
hooks, but node hooks, tool contribution, and shared `&mut dyn ContextManager`
don't fit the request/response shape; generic composition also fights the
runtime's dyn-first architecture. Rejected.

**D — plain extract-object refactor (no trait).** Pull the three behaviors
into three structs with direct calls at the existing call sites; no trait, no
stack, no run-state map. Cheapest way to clean up the loop, and children keep
sharing the code for free. What it does *not* buy: the seam later phases want
— and "later phases want it" is precisely the unvalidated sequencing claim
(§6 J1). The panel's scope reviewer argued D (or a one-behavior spike) is the
right-sized Phase 1 *if* Phase 3 is not yet committed; this spec proceeds
with B because the session mandate is the seam itself, but the choice is
surfaced at the spec gate rather than buried (§6 J1).

## 5. Design

New module: `agent/crates/agent-core/src/middleware.rs`.

### 5.1 The trait

```rust
#[async_trait]
pub trait Middleware: Send + Sync {
    /// Stable identifier for tracing spans.
    fn name(&self) -> &str;

    /// Tools this unit contributes at assembly. `child_visible` controls
    /// membership in the sub-agent base snapshot (§5.5).
    fn tools(&self) -> Vec<ToolContribution> { Vec::new() }

    // ---- node hooks (all default no-op / Flow::Continue) ----
    /// Before the goal is set and the user message is appended.
    async fn on_run_start(&self, cx: &mut RunCx<'_>, input: &str) -> Flow;
    /// Fires exactly between id normalization (loop_.rs:642) and the
    /// assistant-message append (loop_.rs:704): only on turns that parsed
    /// successfully. Skipped on the protocol-repair `continue`
    /// (loop_.rs:611-618) and on every early return — so stuck state is
    /// untouched by repair turns, as today.
    async fn after_model(&self, cx: &mut RunCx<'_>, turn: &TurnView) -> Flow;
    /// After the turn's tool-result messages (and any post-tool-validator
    /// message) are appended — today's nudge point (loop_.rs:1004-1013).
    async fn after_tools(&self, cx: &mut RunCx<'_>) -> Flow;
    /// Bottom of a completed tool turn (today's scheduled-maintenance point,
    /// loop_.rs:1015-1030). A turn ended by `continue` or return never
    /// reaches it.
    async fn on_turn_end(&self, cx: &mut RunCx<'_>) -> Flow;
    /// Only at loop_.rs:751 — the text-only exit: the model replied with no
    /// tool calls, the reply is appended, Done not yet emitted. It does NOT
    /// fire on the Length-stop returns (609, 634), error/cancel returns, or
    /// the BudgetExhausted fall-through (including after its wrap-up append).
    async fn after_final_reply(&self, cx: &mut RunCx<'_>);

    // ---- wrap hooks (default: pass through) ----
    /// Wraps ONE completion_with_retry invocation (the retry-with-backoff
    /// unit). Overflow recovery stays loop-resident (§6 J4): on an overflow
    /// the loop compacts/rebuilds OUTSIDE any wrap, then invokes the wrapped
    /// unit a SECOND, independent time with the rebuilt request. A wrap
    /// middleware never observes the recovery between its two invocations.
    async fn wrap_model_call(
        &self, req: CompletionRequest, next: ModelNext<'_>,
    ) -> Result<AssistantTurn, CompletionFailure>;
    /// Wraps one gated, approved call's execution (execute_isolated).
    /// Runs concurrently with other calls: no RunCx access (§5.4).
    async fn wrap_tool_call(
        &self, call: ToolCall, next: ToolNext<'_>,
    ) -> Executed;
}

pub enum Flow { Continue, EndRun(StopReason) }

pub struct ToolContribution {
    pub tool: Arc<dyn Tool>,
    /// In the child base snapshot? (memory tools: yes; context tools: no)
    pub child_visible: bool,
}
```

`CompletionFailure` is today's private `RetryFailure` (Fatal / Cancelled /
Overflow) promoted to a public enum, and `Executed` (Ok / ToolErr / Panicked /
TimedOut, `loop_.rs:1248`) is likewise promoted — visibility changes only.

### 5.2 `RunCx` — what hooks can touch

```rust
pub struct RunCx<'a> {
    pub ctx: &'a mut dyn ContextManager,   // append/set_recall/maintain/…
    pub sink: &'a Arc<dyn EventSink>,
    pub cancel: &'a CancellationToken,
    pub state: &'a mut RunState,           // §5.3
    pub turn: Option<usize>,               // 0-based; None pre-loop (run start)
    maint: MaintView<'a>,                  // read-only loop internals
}
impl RunCx<'_> {
    pub fn maint_model(&self) -> &Arc<dyn ModelClient>;
    pub fn maint_model_limit(&self) -> usize;      // calibrated, compaction-capped
    pub fn effective_model_limit(&self) -> usize;  // calibrated
}
```

`TurnView` is an **owned snapshot** of the parsed turn (`text`, valid
`tool_calls`, `invalid` calls as name + error), cloned before the hook runs —
NOT a borrow of the loop's `parsed`/`all_calls`. The loop still owns and
consumes those after the hook returns; a borrowed view would force the
`#[async_trait]` future to hold borrows of loop locals across `.await` and
collide with their later moves (`loop_.rs:762`, `787`). The clone cost is one
small struct per turn.

Hooks that end the run are responsible for any terminal context appends and
`Error` events; the loop maps `Flow::EndRun(reason)` to
`emit(Done(reason)); return Ok(())` — exactly the stuck-abort contract today.
On the `EndRun` branch the loop touches `parsed`/`all_calls` no further.

**Hook append discipline:** whatever a hook appends must leave the context
OpenAI-compat-valid — in particular, an assistant message carrying
`tool_calls` must be balanced by matching `Role::Tool` results before the run
ends, or the persisted context 400s the next run (the invariant the
stuck-abort code guards, `loop_.rs:681-694`). After each hook returns, the
loop `debug_assert!`s the no-orphaned-tool-calls check that already exists
(`context.rs:310-312`) so a violating middleware trips in tests, not in
production.

**Isolation posture (explicit, Phase 1):** node and wrap hooks are trusted,
in-process code compiled into the runtime — they are NOT panic-isolated or
timeout-bounded the way tools are (`execute_isolated`). A panicking hook
unwinds the run; a hanging hook wedges it. This matches today's posture for
the same code inline in the loop (e.g. a hanging retriever already blocks at
`loop_.rs:479`). Revisit if/when stacks ever accept out-of-tree middleware.

### 5.3 State extension

`RunState` is a typed anymap (`HashMap<TypeId, Box<dyn Any + Send>>`) with
`get::<T>() / get_mut::<T>() / entry::<T>()`, created fresh per
`run_with_cancel` invocation. This matches the lifetime of the state being
migrated: `last_sig`/`repeats`/`nudged` and `run_maintained` are per-run
locals today (`loop_.rs:489-504`); contexts persist across runs but this
state must not. Middleware stay stateless `&self` objects (safe under
concurrent runs of one assembled loop). Honest sizing note: Phase 1's keys
are enumerable (stuck state + maintained marker), so a concrete struct would
suffice today; the typemap is the state-*extension* surface the phase
mandate names, kept because adding a middleware must not require editing a
core struct. (Panel scope finding, accepted as a conscious trade.)

### 5.4 Ordering and short-circuit contract

- "Before-side" hooks (`on_run_start`) run in **stack order**; "after-side"
  hooks (`after_model`, `after_tools`, `on_turn_end`, `after_final_reply`)
  run in **reverse stack order**; wrap hooks nest **first-outermost**. These
  are the semantics of LangChain's `AgentMiddleware` layer (which deepagents
  consumes rather than defines) — verified against live docs and
  `factory.py` composition code 2026-07-08, no drift.
- A `Flow::EndRun` short-circuits the remaining hooks *at that point* and
  ends the run.
- **Per-turn append timeline** (the load-bearing sequencing; durable-message
  order must equal today's byte-for-byte). For a tool turn with a validator
  failure and a pending nudge:

  | # | Append/action | Owner | Hook point |
  |---|---|---|---|
  | 1 | assistant message (text + tool_calls) | loop | — (after `after_model` returned Continue) |
  | 2 | one `Role::Tool` message per call, model order | loop | — |
  | 3 | validator-failure user message | loop (resident) | — |
  | 4 | nudge user message | StuckDetection | `after_tools` |
  | 5 | maintain (offload/compact) | ContextCuration | `on_turn_end` |

  The nudge lands in `after_tools` and the maintain in `on_turn_end`
  **because** the hook point, not stack order, is what guarantees
  nudge-before-maintain; a parity test diffs the full appended-message
  sequence of a nudged+validated turn against baseline.
- `wrap_tool_call` chains run inside the bounded-parallel executor
  (`buffer_unordered`, `loop_.rs:818-846`), so they get no `RunState` access;
  the gate (policy, approval, `ToolIntent`) has already run and is not
  wrappable — Phase 1 wrap hooks cannot weaken policy.
- Phase 1 ships **no behavior on wrap hooks** — the chains are built and
  exercised by tests with a synthetic recording middleware, so the seam is
  proven without migrating retry/caching behavior prematurely.
- **The wrap-hook signatures are a Phase-1 provisional contract.** Phase 3's
  first real wrapper (repair / guardrail limits / caching) is allowed to
  revise them — signature changes are cheap while all middleware live in
  `agent-core`. What the synthetic-middleware tests pin is the *composition
  semantics* (first-outermost nesting, invoked-twice-on-overflow,
  gate-unreachability), not the exact signatures. Known revision candidates:
  `wrap_tool_call` may need cross-call state (J7's edit-loop-detector
  example), and a retry middleware may want to sit inside the overflow loop
  rather than outside it (J3/J4). Deliberately NOT done in Phase 1:
  implementing any wrap behavior early just to "use" the hooks — that would
  violate both the no-behavior-change bar and the phase boundary. Watch item
  for Phase 3: a `wrap_model_call` that mutates the request will skew
  `est_prompt_tokens` calibration, which the loop computes pre-chain —
  revisit the boundary when the first mutating wrapper lands.

### 5.5 The three migrated middleware (all in `agent-core`)

**`MemoryRecallMiddleware`** — owns `Arc<dyn Retriever>` (from
`LoopParts.memory_retriever`) and the frontend-built memory tools
(`LoopParts.memory_tools`, `child_visible: true`).
`on_run_start`: `cx.ctx.set_recall(retriever.retrieve(input).await)` —
unconditional, so an empty retrieval still clears the prior run's block
(pins `empty_retrieval_clears_stale_recall`). In the stack iff `cfg.memory`;
retriever `None` ⇒ tools only, no recall — today's exact gating
(`assemble.rs:167-171,373-376`).

**`ContextCurationMiddleware`** — owns the caller-shared offload store +
compact flag; contributes `context_recall`/`context_compact`
(`child_visible: false`). `on_turn_end`: build `MaintCtx` from `RunCx`
accessors, `ctx.maintain(...)`, set a `Maintained` run-state marker, emit the
same debug trace. `after_final_reply`: maintain iff no `Maintained` marker —
today's `run_maintained` gate, preserving the tool-bearing-runs-skip-exit-
maintain cadence (pins `text_only_run_is_curated_at_exit` and
`tool_bearing_run_skips_the_exit_maintain`).

**`StuckDetectionMiddleware`** — `after_model`: compute the id-independent
signature (same `\u{1}`/`\u{2}` encoding, `loop_.rs:654-679`) over the
`TurnView`, track `(last_sig, repeats, nudged)` in run state with today's
exact reset rule (signature change resets `repeats` and `nudged` together,
`loop_.rs:676-677`). At `STUCK_ABORT_AFTER`: append the assistant text-only
message, emit the same `Error` event, return
`Flow::EndRun(StopReason::Error)` — abort still lands *before* the assistant
tool_calls append because `after_model` precedes it. At `STUCK_NUDGE_AFTER`:
set a pending-nudge marker; the same turn's `after_tools` consumes and clears
it — still after tool results and the post-validator message (which stays
loop-resident and runs before the `after_tools` hook point). The marker is
consumed exactly once; if another hook ends the run between the two points,
the run is over and the marker dies with the per-run state. The abort and
nudge message *strings* are hardcoded ("5 turns", "3 turns" —
`loop_.rs:688`, `1009`), not derived from the constants; they migrate
byte-identical. Pins `stuck_identical_calls_nudged_then_aborted`,
`stuck_counter_resets_on_different_call`.

### 5.6 `assemble_loop()` as stack composition

```text
build_registry(base) → register MCP tools → build skills/prompt (unchanged)
stack = [MemoryRecallMiddleware?  (iff cfg.memory),
         ContextCurationMiddleware,
         StuckDetectionMiddleware]
register stack tools where child_visible == true
child_base snapshot (iff cfg.subagents)          // position invariant kept
register stack tools where child_visible == false
register dispatch_agent (carrying a child stack builder, §5.7)
AgentLoop::new(..., stack)
```

`LoopParts` is unchanged. `AgentLoop::with_retriever` is deleted (retriever
moves into the middleware); `with_compaction_model` stays — the compaction
model also serves the loop-resident overflow-recovery maintain (§6). Since
`ToolRegistry` is a `HashMap` (arbitrary schema order today,
`registry.rs:6-35`; verified no order-sensitive consumer anywhere),
contribution placement affects only snapshot *membership*, which the existing
`child_base_snapshot_*` tests pin. Contribution names share the registry's
single last-wins namespace with base/MCP/skill tools
(`registry.rs:19-26`) — assembly `debug_assert!`s that stack contributions
don't collide with already-registered names, preserving today's
"context tools registered last, so they win" precedence by construction.

### 5.7 Sub-agent children

`DispatchAgentTool` currently inherits stuck detection and maintenance by
sharing `run_with_cancel`, and builds per-child context tools directly
(`dispatch.rs:474-478`). Recast: `DispatchDeps` carries what's needed to
build each child's stack per dispatch —
`[ContextCurationMiddleware(per-child store/flag), StuckDetectionMiddleware]`
— and the child's context tools come from its own middleware's contributions.
**Invariant: child stacks contain exactly `[ContextCuration,
StuckDetection]`** — memory tools ride the base snapshot as plain tools; the
`MemoryRecallMiddleware` itself is never instantiated for a child (children
have no retriever today: `dispatch.rs:503-511` never calls
`with_retriever`). Pinned by a new test: a dispatched child with memory
enabled sees the memory tools but performs zero `set_recall` calls on its
context. Child behavior is bitwise today's: same tools, same maintenance
cadence, same stuck thresholds.

### 5.8 What `run_with_cancel` looks like after

The loop keeps: sandbox-degraded surfacing, `RunStart`, `set_goal` + user
append, `ctx.build` + `Usage` emission, completion retry + overflow recovery,
`ServerUsage` + calibration, protocol parse/repair + Length handling, id
normalization, gating, parallel execution, phase-3 ordered result appends,
post-tool validators, budget wrap-up. It gains five node-hook call-outs and
the two wrap-chain constructions. Everything the loop emits, appends, and orders
today is emitted, appended, and ordered identically; only the *owner* of the
three migrated behaviors changes.

## 6. Judgments — panel-reviewed 2026-07-08, dispositions inline

- **J1 — APPROVED at the gate (Kalen, 2026-07-08).** Middleware-first
  sequencing is an unvalidated bundle claim, and the panel's scope reviewer
  pressed it: Phase 1's *own* payoff is only inline-cleanup + child parity;
  the trait is largely a Phase-3 affordance. Named cheaper alternatives: the
  extract-object refactor (§4 D) or a one-behavior spike. The owner committed
  to the full seam (and thereby to Phase 3 being real enough to build for),
  which is the premise J9's resolution rests on.
- **J2.** Loop-native hook taxonomy instead of deepagents' four nodes (§4) —
  panel-endorsed (the abort/nudge orderings genuinely don't fit four nodes).
- **J3 — resolved by panel (was a blocker).** `wrap_model_call` wraps a
  *single* `completion_with_retry` invocation; on overflow the loop performs
  recovery outside any wrap and invokes the wrapped unit a second,
  independent time. The earlier "wraps the attempt-group across the rebuild"
  framing was incoherent — the rebuild is `&mut ctx` maintenance a ctx-less
  wrap cannot contain.
- **J4.** Overflow-recovery compaction (`loop_.rs:533-567`) stays
  loop-resident: it is entangled with StreamRetry retraction, Usage re-emit,
  and calibration-denominator reassignment; migrating it is Phase 3 work
  (retry-as-middleware), and doing it now risks the no-behavior-change bar.
- **J5.** `after_final_reply` fires on exactly one exit path (text-only,
  `loop_.rs:751`); all other exits — including both Length-stop returns and
  the BudgetExhausted wrap-up — fire no node hooks. A general
  `on_run_end(reason)` was rejected: `reason` cannot discriminate the
  Length-text-exit from the Length-truncation returns, reintroducing the
  ambiguity the narrow hook avoids.
- **J6.** `child_visible` on `ToolContribution` + per-child stack builders in
  `DispatchDeps` — panel: encodes a real live distinction
  (`assemble.rs:177-190`, `dispatch.rs:474-478`), keep.
- **J7.** `wrap_tool_call` gets no run state (parallel execution) — panel:
  correct call (shared `&mut` there is a data race); revisit when a
  middleware needs cross-call state.
- **J8.** Keeping protocol repair, validators, budget wrap-up, and
  calibration loop-resident in Phase 1.
- **J9 — RESOLVED at the gate (Kalen, 2026-07-08): keep, as a provisional
  contract.** The scope reviewer's YAGNI case for cutting the wrap hooks
  rested substantially on Phase 3 being unvalidated; J1's approval removes
  that premise — repair, guardrail limits, and cache-aware assembly are all
  wrap-shaped, and cutting would mean designing the chain mechanics twice
  and re-deriving the hard-won wrap boundary (J3, two independent panel
  blockers) from a comment instead of tested code. The surviving argument —
  signatures guessed without a real consumer may be wrong — is answered by
  the provisional-contract rule in §5.4: Phase 3's first real wrapper may
  revise the signatures; tests pin composition semantics only.

## 7. Testing

- **Parity (the bar):** the whole `agent/` suite passes with assertion
  bodies unchanged; only construction sites may be touched. Named pins:
  `auto_retrieval_injects_recall_block_into_context`,
  `empty_retrieval_injects_no_block_and_turn_completes`,
  `empty_retrieval_clears_stale_recall`,
  `stuck_identical_calls_nudged_then_aborted`,
  `stuck_counter_resets_on_different_call`,
  `text_only_run_is_curated_at_exit`,
  `tool_bearing_run_skips_the_exit_maintain`,
  `overflow_compacts_rebuilds_and_recovers_once`,
  `second_overflow_in_a_turn_is_fatal`,
  `child_base_snapshot_includes_memory_tools_when_enabled`,
  `child_base_snapshot_excludes_context_tools_and_dispatch_itself`,
  `registers_context_management_tools`, and the dispatch test suite.
- **New stack-mechanics tests:** hook ordering (forward/reverse) via a
  recording middleware; `EndRun` short-circuit; `RunState` typed roundtrip +
  per-run reset; wrap nesting first-outermost for both chains; the model
  wrap chain is invoked twice (independently) across an overflow rebuild;
  tool contribution respects `child_visible`; a no-middleware stack behaves
  as a bare loop.
- **New parity pins (panel findings):**
  - no maintain fires on either Length-stop exit (`loop_.rs:609`, `634`);
  - a malformed-then-repeated call sequence leaves stuck counters exactly as
    today (repair turns don't advance or reset them);
  - full appended-message-order diff of a turn with a validator failure AND
    a pending nudge against today's baseline (the §5.4 timeline);
  - a hook returning `EndRun` between `after_model` and `after_tools` ends
    the run without a dangling pending-nudge or orphaned tool_calls message;
  - a dispatched child with memory enabled: memory tools visible, zero
    `set_recall` calls;
  - no node hook fires on cancellation or the BudgetExhausted fall-through
    (including after the wrap-up append).
- **Gate:** `bash scripts/ci.sh` green.

## 8. Open questions

- Should `Flow` reserve jump targets (deepagents allows jump-to-model/tools)?
  Phase 1 needs only `Continue`/`EndRun`; reserving variants now is
  speculative — left out (YAGNI), revisit in Phase 3.
- `before_model` (mutate ctx before `build`) has no Phase 1 consumer and is
  omitted; Phase 3 caching/repair will tell us its real shape.

## Panel & review log

- **2026-07-08 — adversarial spec panel** (4 independent skeptical reviewers:
  Requirements, Assumptions incl. live deepagents drift check, Failure &
  abuse, Scope & simpler design; opus×3 + sonnet×1). Findings and
  dispositions:
  - **Blockers (all fixed in place):** (1) hook firing set was ambiguous
    across the loop's 11 exit points — Length-stop returns, protocol-repair
    `continue`, cancel/budget paths now explicitly fire no hooks (§5.1, J5),
    with parity pins (§7). (2) `TurnView` respecified as an owned snapshot —
    a borrowed view forces `&mut` borrows of loop locals across `.await`
    (§5.2). (3) `wrap_model_call`/overflow contradiction (found independently
    by two reviewers) resolved: wrap = one `completion_with_retry`
    invocation, recovery outside, invoked twice (§5.1, J3).
  - **Majors (fixed):** per-turn append timeline table + order-diff pin
    (§5.4); hook append discipline + orphaned-tool-calls debug_assert (§5.2);
    hook panic/hang posture made explicit (§5.2); child-stack invariant +
    zero-`set_recall` pin (§5.7); stuck reset/marker semantics + hardcoded
    message strings (§5.5); tool-name collision note + debug_assert (§5.6);
    G2 "recast" claim downgraded to its honest scope (§2).
  - **Majors (escalated to the gate, not silently adopted):** the seam-vs-
    extract-object scope question (J1, §4 D) and the cut-wrap-hooks
    recommendation (J9) — both conflict with the session mandate and are the
    user's call.
  - **Minors (fixed):** exit-path count 7→11 (§4); ordering-semantics
    attribution corrected to LangChain's middleware layer (§5.4); Phase-2/3/4
    partition re-flagged as unvalidated (§2); `name()` justification trimmed;
    `RunState` sizing note (§5.3).
  - **Clean bills:** every line-anchored codebase claim verified true
    (including `ToolRegistry` order-insensitivity, checked against all
    consumers); live deepagents/LangChain drift check found no drift;
    concurrency + child-isolation reasoning survived attack; `ToolIntent`
    gate unreachable from wrap hooks confirmed.
- **2026-07-08 — spec-review gate (Kalen):** J1 approved (full seam; Phase 3
  committed). J9 resolved: wrap hooks kept, contract marked provisional
  (§5.4) — Phase 3's first real wrapper may revise signatures; tests pin
  composition semantics, not signatures; no early wrap behavior just to
  "use" the hooks.
- 2026-07-08 — implementation note (whole-branch review): §5.7's "DispatchDeps
  carries a child stack builder" shipped simplified — dispatch builds the
  child stack inline in `DispatchAgentTool::execute` from deps it already
  holds; the child-stack invariant `[context-curation, stuck-detection]` is
  pinned by test.
- 2026-07-08 — pin refinement (whole-branch review): `on_run_start` fires
  before the turn-top cancellation check and before a Length exit (it
  precedes the turn loop), so the §7 "no node hook on cancellation" pin is
  pinned as observed: run_start fires, all later hooks don't.
