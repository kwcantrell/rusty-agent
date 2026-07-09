# Loop middleware wave (deepagents refactor, Phase 3A) — design

**Status:** PLAN-READY 2026-07-09. Decomposition resolved (§0); design approved
at brainstorm, adversarial spec panel run, spec-review gate cleared (E1–E5
resolved), and the post-gate light-tier consistency read passed CLEAN (E3
mechanism source-verified) — all 2026-07-09. See Panel & review log. Next stage
(a later session): implementation plan via `writing-plans`.
**Governing goal (owner, stated at the Phase-2 gate and reaffirmed for
Phase 3):** the refactor exists to provide deepagents-style **modularity** —
custom middleware/filesystems/extensions change runtime behavior. Every
scope judgment below is argued against that criterion.
**Knowledge base:** `docs/okf/deepagents-refactor/` (commit d997eec). Design
judgments in `comparisons/refactor-priorities.md` are *unvalidated input* —
including the Phase-3 bundle bullet and the five-item grouping; the
decomposition (§0) treats them as claims.
**Live-source baseline:** commit 71e23d1, re-read 2026-07-09. All `file:line`
anchors are orientation only — locate quoted code by content before editing.
**Builds on:** Phase-1 middleware seam
(`docs/superpowers/specs/2026-07-08-middleware-seam-design.md`, merged
707d7fd) and Phase-2 backend seam
(`docs/superpowers/specs/2026-07-08-backend-seam-design.md`, merged 71e23d1).
This spec designs ON the Phase-1 stack and preserves both phases' invariants
(§3), deliberately superseding four enumerated pins (§3.1).

## 0. Decomposition — Phase 3 is a mini-sequence, not one spec

The bundle's "Phase 3 — cheap high-value middleware" bullet lists five
items (six capabilities). Per the standing constraint that this partition is
an unvalidated bundle claim, the decomposition was examined first at the
brainstorm and resolved by the owner (2026-07-09): **Phase 3 splits into two
independently-reviewable specs plus one deferral**, cut along blast radius /
review boundary / risk:

- **3A (this spec) — Loop middleware wave:** `write_todos` planning, tool-call
  repair, and model/tool-call limit guardrails. All `agent-core`; this is
  where the Phase-1 **provisional wrap-hook contract comes due** (constraint
  carried from Phase-1 §5.4). Zero wire/frontend change.
- **3B — Sub-agent maturity:** named sub-agent registry + the product-level
  typed subagent stream. Touches `dispatch` + the `EventSink` wire + all
  three frontends. Later session.
- **Cache-aware prompt assembly — deferred (gate decision, not dropped).**
  Live-source grounding (2026-07-09, **panel-verified** R-F6: an independent
  `grep -rE 'cache_control|CacheControl|cache_marker|ephemeral' agent/crates/`
  returns only claude-cli's server-usage fixtures — `cache_creation`/
  `cache_read` tokens — and zero request-side cache markers): there is **no
  `cache_control` / cache-marker machinery anywhere** in `agent/crates/`. The
  only caching is
  claude-cli's automatic server-side ephemeral caching (visible as
  `cache_creation`/`cache_read` tokens in usage) and llama.cpp's automatic
  KV-prefix cache. So deepagents' "preserve cache-control markers" has no
  analog here; "cache-aware ordering" reduces to *keep the static prefix
  stable and first so the automatic caches hit, and do not reorder the
  eval-calibrated pinned blocks* — a documented invariant + a stability
  test, not a pluggable runtime behavior. Under the modularity goal it barely
  qualifies (nothing pluggable changes), so it is deferred to Phase 4 or to
  the arrival of an Anthropic-native client with explicit `cache_control`.
  Recorded here so the sequence is complete, not silently pruned.

Rationale for splitting rather than one spec: the six capabilities span the
`agent-core` wrap contract, `dispatch`, the wire, three frontends, and
assembly ordering — the "multiple independent subsystems in one spec" shape
the SDLC warns against, and each cluster produces independently reviewable
value. 3A is sequenced first because it finalizes the wrap contract; 3B does
not depend on that contract (registry is dispatch config, stream is wire), so
the two are independent and could run in either order — 3A goes first only to
discharge the due wrap-contract question.

## 1. Problem

Three cross-cutting behaviors are still hard-wired into the loop, and the
Phase-1 wrap hooks have no shipping consumer:

- **No plan state.** The runtime has no planning surface at all — planning is
  implicit in prompts and skills, with no audit trail of plan vs. outcome
  (bundle: `practices/planning-by-recitation.md`). LangChain ships a no-op
  `write_todos` (in `langchain/agents/middleware/todo.py`, consumed by
  deepagents — the same library-of-origin the Phase-1 panel corrected)
  whose entire value is context engineering: rewriting the plan into recent
  context keeps long-horizon goals in the attention window. **Note (panel):
  in *this* runtime that carrier — a tool-result message — is offload-eligible,
  so the recitation must be made curation-durable (§5.4), which the first
  draft missed.**
- **Repair is hard-wired.** Malformed-tool-call recovery is a loop-resident
  one-shot re-ask: `protocol_repairs < 1` at `loop_.rs:789`, a hardcoded
  count and a hardcoded message, then terminal. It cannot be tuned, replaced,
  or composed (bundle: gap analysis, "Tool-call repair" = partial).
- **No guardrail limits.** `max_turns` bounds the turn loop and
  `StuckDetection` catches *identical* repeats, but nothing bounds total
  **tool-call** volume. A runaway that *varies* its calls (different args
  each turn) slips past `StuckDetection` — verified: its signature is over
  `(name, args)` (`middleware.rs:371-378`), so varying args changes the
  signature every turn and `repeats` never climbs — and can burn the whole
  turn budget issuing unbounded tool calls (bundle: "Guardrail limits" =
  partial; LangChain ships `ModelCallLimit`/`ToolCallLimit` middleware,
  consumed by deepagents).
- **The wrap seam is unproven.** Phase-1 shipped `wrap_model_call` /
  `wrap_tool_call` with default pass-throughs and a **provisional** contract
  (Phase-1 §5.4): "Phase 3's first real wrapper may revise the signatures;
  tests pin composition semantics, not signatures." That revision is due.

deepagents demonstrates the alternative: each of these is one middleware on an
ordered stack (bundle: `practices/middleware-composition.md`). Phase 1 built
the stack; 3A adds the first three real behavioral consumers.

**Honest framing (panel R-F1/R-F8).** The three items advance the phase in
*three different ways* — not symmetrically "modularity":

- `write_todos` **closes a capability gap** (planning: absent → present). As a
  hookless tool-contributing middleware it exercises a surface Phase 1 already
  shipped; its modularity contribution is marginal (droppable stack entry).
- Guardrails are **the real seam delivery**: the `wrap → RunShared → node-hook`
  round-trip is a genuinely new extension capability (Phase-1 wrap hooks had
  no state; §5.2).
- Repair **discharges the Phase-1 provisional wrap contract** — but only for
  the *non-mutating* case. 3A does **not** finalize the mutating-`wrap_model_call`
  / calibration boundary (that stays deferred with the cache item, §5.2); so
  after 3A a custom *request-mutating* model wrapper is not yet safe. "Finalizes
  the wrap contract" would overstate it — 3A finalizes the **non-mutating**
  contract.

## 2. Goals

- **G1.** `TodoListMiddleware` — contributes a `write_todos` tool
  (child-visible) whose current list is rendered as a **durable pinned block**
  (E3 pin/recall, §5.4), giving planning-by-recitation a hard in-window
  guarantee; discipline in the tool description. Hookless; holds a shared todos
  handle (the `compact_flag` shape); appends its block *after* the existing
  eval-calibrated pinned order (existing order byte-identical).
- **G2.** `RepairMiddleware` — the loop-resident parse-repair
  (`loop_.rs:789`) becomes a middleware behind **one new node hook**
  (`on_parse_failure`); default behavior is **byte-identical** to today (one
  re-ask, same message string).
- **G3.** `ToolCallLimit` (and `ModelCallLimit`, panel recommends cut — §6 E1)
  guardrail middleware —
  count via the wrap hooks, enforce via node hooks; always-on generous
  hardcoded caps; a new runaway backstop `max_turns`/`StuckDetection` do not
  provide.
- **G4.** Finalize the **non-mutating** wrap-hook contract by the **minimal**
  revision Posture 1 chose: add a per-run **thread-safe** state facility
  (`RunShared`) reachable from both wrap hooks and node hooks. **No** mutating
  `wrap_model_call` is introduced; the calibrated-token-estimation boundary is
  untouched (§5.2), and the mutating-wrap boundary stays deferred (a custom
  request-mutating model wrapper is a known post-3A limit, not solved here).
- **G5.** The full existing `agent/` suite passes with assertion bodies
  unchanged except at the four deliberately-superseded pins (§3.1), plus new
  tests for the hook, the state facility, and the three middleware.

### Non-goals

- **No cache-aware assembly** (deferred, §0). No mutating `wrap_model_call`;
  no `est_prompt_tokens` relocation; no J3/J4 overflow-vs-retry re-nesting
  (that was Posture 2, not chosen).
- **No named sub-agent registry, no typed subagent stream** (3B). **No wire
  or frontend change** in 3A — `write_todos` is visible as an ordinary tool
  call in every frontend already; its *typed* progress projection rides 3B.
- **No config knobs** for repair counts or guardrail caps — hardcoded
  constants, matching the stuck-detection precedent ("not configurable until
  a real workload needs the knob," spec 2026-07-02 §4). Constants are named so
  a future knob is a one-line change.
- **No `StuckDetection` refactor** — guardrails are *siblings*, its code and
  pins are untouched.
- **No guardrails or planning middleware on dispatch children** — children
  already carry `subagent_max_turns`; adding call-limits is an unmotivated
  child behavior change (§5.6). Children *do* keep repair (behavior parity)
  and *can* call `write_todos` via the base snapshot.
- **Parked, out of 3A:** HostBackend `grep`/`glob` `spawn_blocking` + Host
  grep-hit sort (orthogonal — 3A touches no backend code; a standalone
  hygiene fix, and neither Phase-3 spec touches `grep`); context-evolve
  ceiling re-measurement (memory note `context-evolve-needs-backend-migration`).

## 3. Do-not-regress invariants

Gap-analysis keep-invariants + Phase-1 seam invariants + Phase-2 backend
invariants. **Preserved:**

| Invariant | How this design preserves it |
|---|---|
| Goal block + folded-facts ledger; eval-calibrated **pinned-block order** | The existing pinned order (`curated.rs pinned()`: goal/ledger → recall → summary+pointer) is **byte-identical** — 3A **appends** a todos block strictly *after* it (E3, §5.4), never interleaving. The existing blocks' order and rendering are untouched. Honest caveat: appending *any* pinned block extends the pinned region and consumes pinned-token budget — a **measured-behavior change** flagged for the parked context-evolve re-measurement. This is 3A's one deliberate curation touch (the E3 hard-guarantee cost); the caution's concern (reordering the *existing* calibrated blocks) does not occur |
| Curation cadence (Phase-1/2) | `ContextCurationMiddleware` unchanged; `on_turn_end`/`after_final_reply` firing unchanged; `Maintained` marker unchanged |
| `StuckDetection` pins | Its middleware is untouched; guardrails are siblings added *after* it in stack order |
| Calibrated token estimation | **No mutating wrap.** Repair re-iterates via the loop's existing `continue` → `est_prompt_tokens` recomputed at `loop_.rs:687`; guardrails only count (non-mutating). `calib_ratio_micros` logic is not read or written by any new code |
| Backend/artifacts integrity, recovery grammar (Phase-2) | Untouched — no backend, curation, or offload code changes |
| `ToolIntent` policy richness | `write_todos` ships an `Access` intent like any tool; guardrail/repair middleware add no tools that bypass the gate; `wrap_tool_call` still sits *after* the gate (Phase-1) |
| Refusal-on-degraded sandbox, first-class MCP | Untouched |
| Caller-owned `SessionArtifacts` survival (Phase-2) | Untouched — `LoopParts`/`SessionArtifacts` unchanged |

### 3.1 Deliberately superseded pins (behavior/contract changes 3A owns)

Each is a conscious change, listed with the §5 decision it maps to and the
re-pin that replaces it:

1. **Hook-firing-set pin (Phase-1 §7).** Extended by the new
   `on_parse_failure` node hook. Re-pinned: `on_parse_failure` fires at
   exactly the total-parse-failure error arms (`loop_.rs:789-801` — the
   `ReAsk`-equivalent arm at 789-796 and the terminal arm at 797-801),
   *after* the Length short-circuit (`loop_.rs:783-787`) and *not* on the
   second Length guard (`loop_.rs:808-813`, downstream), and nowhere else;
   all Phase-1 firing-set assertions for the existing five hooks still hold
   (§5.1).
2. **Wrap-hook contract (Phase-1 §5.4, marked provisional).** Extended
   **additively**: wrap hooks gain access to `RunShared` via
   `ModelNext`/`ToolNext`. Existing signatures' *return* types and the
   gate-unreachability / first-outermost-nesting / invoked-twice-on-overflow
   composition semantics are unchanged; the Phase-1 synthetic-middleware
   composition pins keep passing (§5.2).
3. **Child-stack composition pin (Phase-2 §3, `[curation, stuck]`).** Extended
   to `[curation, stuck, repair]` — *required to preserve behavior*: repair is
   loop-resident today and children share `run_with_cancel`, so children get
   repair now; moving it to a middleware forces its inclusion in the child
   stack or a regression. Re-pinned with child repair asserted byte-identical
   (§5.6).
4. **Loop-resident repair location.** The `protocol_repairs < 1` block leaves
   the loop and becomes `RepairMiddleware` behind `on_parse_failure`. Behavior
   (count, message, terminal-on-second-failure, Length short-circuit
   ordering) is byte-identical; a parity pin diffs the appended messages of a
   malformed→repaired→resolved sequence against baseline (§5.3).

## 4. Alternatives considered

**A — Posture 1: minimal wrap revision (chosen).** Repair = loop-driven
re-iteration via the new `on_parse_failure` node hook (est recomputed → no
calibration skew); guardrails = the one deliberate additive wrap revision
(`RunShared` for non-mutating counting), enforced at node hooks. Discharges
the due wrap contract with a real shipping consumer at low, reversible cost.

**B — Posture 2: fuller wrap realization (rejected at brainstorm).** Make
repair a `wrap_model_call` consumer and take on the calibration rework now —
relocate `est_prompt_tokens` after the wrap chain so a mutating wrapper is
honestly measured, and resolve the J3/J4 overflow-vs-retry nesting Phase 1
explicitly deferred. Most deepagents-faithful, but: (1) touches the
calibrated-token-estimation do-not-regress invariant, with *silent* failure
modes (ratio drift → context over-trimming); (2) un-defers J3/J4; (3) builds
the mutating-wrap boundary for a consumer (cache) that the grounding shows may
never materialize in this runtime (§0). YAGNI points away. Rejected by the
owner 2026-07-09; recorded because the J9 "design the chain twice" concern is
real and the decision to pay it *never* rather than *now* is a judgment, not a
foregone conclusion.

**C — Keep wrap hooks stateless / defer the revision (rejected).** Guardrails
via loop-side counting + node hooks only; no wrap-state. Lowest risk, but the
wrap seam then has **no** real consumer after 3A, contradicting the Phase-1 J9
premise that Phase 3's first wrapper would exercise/revise it — the hooks stay
test-only scaffolding. Rejected: a non-mutating counting consumer is exactly
the low-risk way to prove the seam, which is the point of building it.

**D — `write_todos` as a hardcoded base tool, not a middleware (rejected).**
Cheapest, but a hardcoded tool is always-on and not pluggable; the modularity
goal wants planning to be a stack entry you can drop or swap. A
tool-contributing middleware with no hooks is *not* the Phase-2 "tool bundle
in a middleware costume" anti-pattern (that failure was construction-bound
routing for stateful children; `write_todos` is stateless with no routing).

**E — Refactor `StuckDetection` into a general guardrail framework
(rejected).** "Generalizing stuck detection" (bundle wording) could mean
absorbing it into a `Guardrail` base with stuck as one instance. Rejected:
`StuckDetection` is parity-pinned and load-bearing; a rewrite risks those pins
for no capability gain. Guardrails join it as *siblings*.

**F — Repair stays loop-resident, reads its policy from an optional
`RepairMiddleware` looked up by type (surfaced by the panel, Scope F3 —
ESCALATED, §6 E2).** Instead of a new `on_parse_failure` node hook, the loop
keeps the `protocol_repairs`/message block at `loop_.rs:789-801` but reads
`MAX_REPAIRS` + the message from an optional stack member (a `RepairPolicy`
trait the loop `find_map`s), defaulting to today's constants when absent.
**Cost saved:** no new trait method, no `Repair` enum, no firing-set pin
change, and — because children run the loop and inherit loop-resident repair —
**no child-stack change** (collapses §3.1 #1 and #3 to zero). **Cost paid:**
repair is no longer a genuine *middleware* (the bundle's Phase-3 framing is
"repair as a pluggable **unit**"), and the loop gains a bespoke
capability-lookup idiom the codebase does not otherwise use — less uniform
with the Phase-1 hook pattern and less deepagents-faithful (LangChain/deepagents
ship repair as middleware). This alternative was **absent from the first
draft's §4** — a real gap the panel caught. The hook (chosen) vs. lookup
trade is escalated to the gate (E2); it turns on whether "repair is a genuine
pluggable middleware" is worth one trait method + two routine pin
re-pins.

## 5. Design

All new code lands in `agent/crates/agent-core/src/middleware.rs` (trait
extension + the three middleware) and `loop_.rs` (the two call-outs: the
`on_parse_failure` site and the `RunShared` plumbing), with the stack wiring
in `agent-runtime-config/src/assemble.rs` and `agent-core/src/dispatch.rs`
(child stack).

### 5.1 The new node hook: `on_parse_failure`

```rust
/// The model's output could not be parsed into tool calls at all
/// (protocol.parse returned Err). The loop consults this hook at the
/// total-parse-failure error arms (loop_.rs:789-801: the current
/// protocol_repairs<1 re-ask arm and the terminal give-up arm), AFTER the
/// Length-truncation short-circuit (loop_.rs:783-787) and upstream of the
/// second Length guard (loop_.rs:808-813). ReAsk maps to the 789-arm body,
/// GiveUp to the 797-arm body. Fires on nothing else: not on success, not on
/// partial-invalid turns (those flow through TurnView.invalid + after_model),
/// not on Length truncation, cancel, overflow, or budget exhaustion.
async fn on_parse_failure(
    &self, cx: &mut RunCx<'_>, raw: &AssistantTurn, err: &str,
) -> Repair { Repair::GiveUp }

pub enum Repair {
    /// Append the raw assistant text + this user message, then `continue`
    /// (fresh turn iteration; est_prompt_tokens recomputed → no skew).
    ReAsk(String),
    /// Terminal: today's behavior — emit Error + Done(Error), return Ok.
    GiveUp,
}
```

**Fold semantics (single decision, deterministic).** The loop consults the
stack in **reverse stack order** (after-side convention); the **first**
middleware returning `ReAsk` wins and short-circuits the rest; if all return
`GiveUp` (or none implement the hook), the loop performs today's terminal
give-up unchanged. With the default stack's single `RepairMiddleware` this is
unambiguous; the order rule is specified so a second repair middleware is
well-defined.

**Re-ask bounds (panel Fa-F4).** A `ReAsk` runs the loop's existing
`continue`, which **re-enters `for turn` and advances the turn index**
(verified A6/A12) — so **`max_turns` bounds re-asks** exactly as today (each
re-ask consumes a turn; parity). Two caveats the draft owed: (1) **empty-text
append is preserved behavior** — on the common shape where the model emits an
unparseable tool-call blob and no prose, `raw.text` is empty and the loop
appends an empty, tool-call-free assistant message (today's `loop_.rs:791`
behavior; balanced, no orphan). (2) **No *global* re-ask cap across multiple
repair middleware**: `MAX_REPAIRS` is per-`RepairMiddleware` run state, so
composing two repairers *sums* their budgets. This is **bounded by `max_turns`
regardless** (each re-ask still burns a turn), so it is not unbounded; it is
accepted as residual for the single default repairer, with a loop-owned per-run
re-ask ceiling recorded as an optional hardening (§8) since the modularity goal
invites custom repairers.

**Why a node hook, not a wrap.** Parse happens in the loop *after* the wrap
chain returns `AssistantTurn` (raw), so malformed-ness is a loop fact a wrap
cannot observe without duplicating the protocol. And the existing repair path
already uses `continue` → a fresh turn iteration that recomputes
`est_prompt_tokens` at `loop_.rs:687`, so a loop-driven repair is
calibration-honest *by construction*. The node hook makes the *policy*
(count, message, give-up) pluggable while the loop keeps *driving* the
re-ask — the minimal faithful change.

`raw: &AssistantTurn` is a borrow of the turn the loop already holds at the
failure site (it is consumed only on the terminal/continue branches after the
hook returns); no clone needed because the hook cannot outlive the call and
does not stash the borrow (contrast `TurnView`, which is cloned precisely
because `after_model`'s result could collide with later moves — that
constraint does not apply here since both `Repair` branches are followed
immediately by the loop consuming/dropping `raw`).

### 5.2 `RunShared` — per-run thread-safe state for wrap hooks (the wrap revision)

Node hooks keep today's `RunState` (`&mut`, sequential — `StuckState`,
`Maintained`, the new repair counter). Wrap hooks need a **different**
concurrency contract: `wrap_tool_call` runs inside the `buffer_unordered`
parallel executor, so it cannot take `&mut` anything shared. New facility:

```rust
/// Per-run, thread-safe typed state, created fresh per run_with_cancel.
/// Reachable from BOTH wrap hooks (via ModelNext/ToolNext) and node hooks
/// (via RunCx), so a middleware can WRITE in a wrap and READ in a node hook.
#[derive(Clone, Default)]
pub struct RunShared(Arc<Mutex<HashMap<TypeId, Box<dyn Any + Send>>>>);

impl RunShared {
    /// Get-or-default then apply `f` under the lock. The SYNCHRONOUS
    /// `FnOnce(&mut T) -> R` structurally forbids `.await` inside the lock
    /// (panel Fa-F6 CLEAN: a guard cannot cross an await by construction).
    /// Poison recovery: the impl takes the lock via
    /// `.lock().unwrap_or_else(|e| e.into_inner())` — RunShared DELIBERATELY
    /// DOES NOT PROPAGATE POISON (see poison note below). NON-REENTRANT: `f`
    /// must not call `with()` again (one Mutex guards the whole map → nested
    /// `with()` self-deadlocks); 3A's guardrails touch one key each, so no
    /// nesting. Values are small counters.
    pub fn with<T: 'static + Default + Send, R>(&self, f: impl FnOnce(&mut T) -> R) -> R;
}
```

- `ModelNext` and `ToolNext` gain a `shared: &RunShared` field (additive; the
  Phase-1 composition semantics — first-outermost nesting, base-case
  `completion_with_retry`/`execute_isolated`, gate-unreachability — are
  unchanged). `RunCx` gains a `shared: &RunShared` field so node hooks read
  the same store.
- **Design call (escalation candidate, §6 J2): typed concurrent map, not a
  concrete two-atomics struct.** 3A's own keys are enumerable (a model-call
  count, a tool-call count), so a concrete `struct GuardrailCounters {
  model: AtomicUsize, tool: AtomicUsize }` would suffice *today* — exactly
  Phase-1's `RunState` sizing situation. The typed map is chosen because the
  governing goal is that *custom* wrap middleware change behavior, and a
  custom guardrail/wrap unit (an edit-loop detector, a per-tool rate limiter)
  needs a state home without editing a core struct — this is the wrap-side
  analog of Phase-1's node `RunState` extension surface. The concrete-struct
  alternative is the YAGNI counter-argument; surfaced to the panel.
- **Poison isolation (panel Fa-F6, corrected — the draft's claim here was
  WRONG and is the sharpest fix in this revision).** The parallel path is the
  whole reason `RunShared` exists, and that path makes naive poisoning *worse*
  than today, not equal: `execute_isolated` catches a tool/wrap panic and
  returns `Executed::Panicked` (containment by design), but if that panic
  fired *inside* a `with()` closure while the guard was held, a naive
  `.lock().unwrap()` in every sibling `wrap_tool_call` still in the
  `buffer_unordered` batch — and in the `after_tools` guardrail (node hooks are
  NOT panic-isolated) — would re-panic on `PoisonError` and unwind the whole
  run. That escapes the containment `execute_isolated` was built to provide.
  **Decision:** `with()` recovers via `into_inner()` and never propagates
  poison. The counters are monotonic, so a torn value can only *over*-count,
  which for a guardrail **fails safe** (it stops the run slightly early, never
  late). Pinned by a poison test (§7): a panicking wrap in a parallel batch
  must not cascade into sibling calls or the `after_tools` guardrail. With
  this decision recorded, Fa-F6 is fixed-in-place; without it, it is a
  BLOCKER for the guardrail-under-parallelism story.

**Calibration boundary — explicitly undisturbed.** No new code reads or
writes `calib_ratio_micros`, and no `wrap_model_call` in 3A mutates its
request. `est_prompt_tokens` is still computed at `loop_.rs:687` before the
chain and sampled at `loop_.rs:771`; guardrail counting is side-effect-free
w.r.t. the request bytes. The Phase-1 watch item ("a mutating
`wrap_model_call` skews calibration") remains *open for a future mutating
wrapper* (the deferred cache item) and is **not** triggered by 3A — stated so
the panel can confirm the boundary is genuinely untouched.

### 5.3 `RepairMiddleware`

Implements only `on_parse_failure`. Owns a per-run repair counter in
`RunState` (`entry::<RepairState>()`). Default policy reproduces
`loop_.rs:789` exactly:

- First total-parse-failure in a turn-sequence where `repairs < MAX_REPAIRS`
  (`MAX_REPAIRS = 1`, the current `protocol_repairs < 1`): increment, return
  `ReAsk("Your tool call could not be parsed: {err}. Re-emit it correctly.")`
  — the current message string, byte-identical.
- Otherwise `GiveUp`.

The loop's `on_parse_failure` call-out replaces the inline
`protocol_repairs`/message block; the loop still owns the `continue` (on
`ReAsk`: append `Message::assistant(raw.text, None)` then
`Message::user(msg)` then `continue`) and the terminal branch (on `GiveUp`:
today's `emit(Error); emit(Done(Error)); return Ok(())`). **Parity pin:** a
malformed→re-ask→well-formed sequence appends exactly today's messages in
today's order, and a malformed→malformed sequence is terminal after exactly
one re-ask, with stuck counters untouched by the repair turns (the Phase-1
"repair turns don't advance/reset stuck" pin still holds — repair still uses
`continue`, so `after_model` never fires on the failed turn).

### 5.4 `TodoListMiddleware`

Provenance (panel B2): `write_todos` / `TodoListMiddleware` live in
**LangChain** (`langchain/agents/middleware/todo.py`), consumed by deepagents —
not "deepagents ships." The upstream tool is a verified no-op that only echoes
the list back (panel B3).

**Durability model — pin/recall, decided at the gate (E3).** The panel showed
(R-F2/Fa-F1) that recitation-via-tool-result is silently defeated by curation:
an ordinary tool result ≥ `output_min_bytes` (1024 B) offloads once it ages
past `keep_recent` (3), and a ≥ `max_result_bytes` (16 KiB) list offloads at
ingestion — so on the long runs the feature exists for, the plan leaves the
window. The gate chose the **hard in-window guarantee** over the cheaper
`exclude_tools` carve-out: the current todo list is rendered as a **durable
pinned block**, so it is always in-window until the model overwrites it —
matching deepagents' "plan stays in the attention window" contract, at the
cost of touching the eval-calibrated curation region (bounded below).

- Contributes `write_todos` as a `ToolContribution { child_visible: true }`
  (children plan for themselves — the plan is never merged back from subagents;
  each child gets its own todos handle, §5.6).
- `write_todos(todos: [{content, status: "pending"|"in_progress"|
  "completed"}])` — rewrites the whole list into a **shared handle**
  (`Arc<Mutex<Vec<TodoItem>>>`) the tool sets on execute, exactly mirroring the
  established `compact_flag` pattern (a tool sets an `Arc`, `CuratedContext`
  reads it — `assemble.rs`/`LoopParts`). The tool returns a **compact
  confirmation**, not the full list, so its own tool-result message is tiny
  and offload-irrelevant; the authoritative recitation is the pinned block.
  No node/wrap hooks.
- **`CuratedContext` renders the todos block (the E3 curation touch, bounded).**
  `CuratedContext` gains a `todos` handle field (caller-wired via `LoopParts`
  like `compact_flag`/`artifacts`) and renders the non-empty list as the
  **last pinned block**, strictly *after* `goal/ledger → recall →
  summary+pointer`. **The existing pinned order is byte-identical** (todos is
  appended, never interleaved), and last-position is nearest the windowed
  conversation — faithful to "recent context." `pinned_tokens()` accounts for
  the todos block in lockstep with `pinned()` (the `curated.rs` invariant).
  **Measured-behavior flag:** adding a pinned block consumes pinned-token
  budget (less windowed-history room), which the parked context-evolve ceiling
  re-measurement must account for alongside the backend migration — recorded
  in memory `context-evolve-needs-backend-migration`. 3A stays within
  `agent-core` (CuratedContext lives here), but this is the one place it
  deliberately drops its "no curation change" self-limit and touches the
  eval-calibrated curation subsystem — the accepted cost of the E3
  hard-guarantee choice.
- **Behavioral contract** (in the tool `description`, no system-prompt
  surface): 3+ distinct steps / non-trivial planning, not
  single/conversational turns; **keep at least one task `in_progress` while
  work remains, and multiple `in_progress` are allowed when independent and
  parallelizable** (panel B1 — the draft's "exactly one" *inverted* the real
  LangChain contract, and this ships verbatim to the model); mark completed
  immediately, do not batch. `Access::Read` intent.
- **State note (J10 reversed at E3).** `write_todos` now *does* hold state (the
  shared handle) — the deliberate cost of the pin/recall durability guarantee.
  It remains hookless (tool contribution + a caller-wired handle, exactly the
  `ContextCurationMiddleware`+`compact_flag` shape), so J7 (hookless
  tool-contributing middleware) still holds.

### 5.5 `ToolCallLimit` and `ModelCallLimit`

Provenance (panel B5): `ToolCallLimit`/`ModelCallLimit` are **LangChain**
built-ins (consumed by deepagents), not "deepagents ships." LangChain's
`ToolCallLimit` offers three exit behaviors (`continue`/`error`/`end`); 3A
ships only the `end`-equivalent (`EndRun`) — a deliberate simplification
consistent with the no-config-knob non-goal, noted so the cut is intentional.

**Abort signal (panel A10/Fa-F2 — corrected).** Guardrails end the run with
`Flow::EndRun(StopReason::Error)` + an emitted `AgentEvent::Error` message —
**exactly the sibling `StuckDetection` abort pattern** (`middleware.rs:397-407`).
The draft used `BudgetExhausted`, which **overloads** the reason: the loop's
natural `max_turns` exhaustion also emits `Done(BudgetExhausted)` but *first
runs a tools-disabled wrap-up completion* (`loop_.rs:1168-1221`), whereas a
guardrail `EndRun` emits `Done` with **no** wrap-up — same reason, divergent
transcript, breaking any consumer that keys on "BudgetExhausted ⇒ closing
message." Using `Error` (a) removes that ambiguity, (b) matches the sibling
guardrail, and (c) correctly signals "the run was aborted by a guardrail," not
"the budget was gracefully exhausted."

**`ToolCallLimit`** (the load-bearing guardrail):
- `wrap_tool_call`: `shared.with::<ToolCallCount, _>(|c| c.0 += 1)` then
  `next.run(...)` — a non-mutating count around execution (serialized under
  the `RunShared` lock; no lost updates under parallel `buffer_unordered` —
  panel F7 CLEAN). **Count-on-failure is intentional (panel Fa-F3):** the
  increment is *before* `next.run`, so panicked/timed-out/errored calls are
  counted (they still cost an orchestration round-trip). Pinned explicitly so
  "count successful only" is not a plausible alternate reading.
- Enforcement + **overshoot bound (panel Fa-F3).** A turn's tool calls run in
  parallel and the loop imposes no *count* cap per turn (`buffer_unordered`
  bounds concurrency, not batch size), so a turn-boundary check alone lets a
  crossing turn execute its **full emitted batch** past the cap (count 999,
  a 5000-call turn → 5999 executed). The cap is therefore honest as "≤ cap
  **at turn boundaries**, plus at most one turn's batch." To bound the
  overshoot to exactly one batch, add a **pre-turn guard**: at the top of the
  turn loop, if `count >= TOOL_CALL_LIMIT`, `EndRun(Error)` before issuing the
  next batch (cheap; complements the `after_tools` check). A hard mid-batch
  cutoff (aborting in-flight futures) is still rejected as racy; §7 pins the
  precise bound rather than claiming a tight one.
- New backstop value: `max_turns` bounds turns, not total tool calls; a
  runaway varying its args each turn escapes `StuckDetection` (verified §1).
  `TOOL_CALL_LIMIT` is a generous order-of-magnitude constant above any
  realistic run (proposed default: 1000), so no existing bounded test trips
  it (no parity break) while an infinite tool loop is capped. §7 pins that it
  fires on a *varying-args* runaway — the shape `StuckDetection` cannot catch
  (panel R-F5) — not merely a fabricated identical loop.

**`ModelCallLimit`** (E1 resolved — **kept, default-off**):
- `wrap_model_call` counts each invocation into `RunShared`; `after_model`:
  `EndRun(Error)` at `MODEL_CALL_LIMIT` (a generous constant, applied **only
  when the guardrail is enabled**).
- **Owner decision (E1, 2026-07-09): keep it, default-off** — against the
  panel's cut recommendation, and for a stated reason: it gives `wrap_model_call`
  a real (if dormant-by-default) consumer, so **both** wrap surfaces are
  exercised and the seam is proven on both halves, not just the tool half. The
  panel's finding stands and is respected — it is *subsumed by `max_turns`*
  (true calls ≤ `max_turns × ~max_retries`, a finite bound) with no runaway
  scenario — which is exactly why it is **default-off**, not always-on: it
  ships as an available, tested, opt-in guardrail (and a `wrap_model_call`
  consumer), not imposed behavior. A workload that wants a true-model-call cap
  enables it.

**Always-on decided per-guardrail (E1 + panel S-F5).** `ToolCallLimit` is
**always-on** in the parent stack with its generous cap — real backstop
(varying-args runaway) + Phase-2 E3 "seam tested in anger" value.
`ModelCallLimit` is **default-off** (subsumed by `max_turns`; opt-in). So the
wrap seam gets an always-on tool-side consumer and an opt-in model-side
consumer — both halves have a real consumer after 3A.

### 5.6 Stack order & child stack

**Parent stack** (`assemble.rs`), in composition order:

```
[ TodoListMiddleware?          // iff cfg enables planning (default on); hookless
  MemoryRecallMiddleware?      // iff cfg.memory (unchanged)
  ContextCurationMiddleware    // unchanged
  StuckDetectionMiddleware     // unchanged
  ModelCallLimit               // kept, DEFAULT-OFF (E1); a wrap_model_call consumer
  ToolCallLimit                // always-on backstop
  RepairMiddleware ]
```

- `TodoList` first (LangChain/deepagents convention; harmless — no hooks, so
  its stack position affects only tool-registration precedence in the
  last-wins namespace, and `write_todos` collides with nothing).
- Guardrails after `StuckDetection`. After-side hooks fire **reverse stack
  order**, so on a turn where both a guardrail and stuck detection would act,
  the guardrail's `after_tools`/`after_model` resolves *before* stuck's.
  **Consequence pinned (panel Fa-F5):** a `ToolCallLimit` `EndRun` at
  `after_tools` short-circuits *before* `StuckDetection::after_tools`, so a
  `nudge_pending` marker StuckDetection set that turn is silently dropped. This
  is **benign** — at `after_tools` the assistant tool_calls message and its
  results are already appended and balanced, so dropping the pending *user*
  nudge leaves the transcript orphan-free (strictly safer than the Phase-1
  `after_model`-EndRun case already pinned). §7 asserts the *content* outcome
  (nudge dropped, transcript balanced, no orphan), not merely that ordering is
  deterministic. (`RepairMiddleware`'s only hook is `on_parse_failure`, a
  distinct firing point, so its stack position is inert for the other hooks.)
- Child-visible tool registration (Phase-1 §5.6 machinery) is unchanged:
  `write_todos` registers before the child-base snapshot (child-visible);
  guardrail/repair middleware contribute no tools.

**Child stack** (`dispatch.rs`): `[ContextCuration, StuckDetection]` →
`[ContextCuration, StuckDetection, RepairMiddleware]`. This **supersedes the
Phase-2 child-stack pin** (§3.1 #3) and is *required for behavior parity*:
repair is loop-resident today, children share `run_with_cancel`, so children
repair today; the middleware move forces explicit inclusion. Child repair
state isolates correctly — each child run mints a fresh `RunState`, so its
`RepairState` counter cannot bleed across children or to the parent (panel F8
CLEAN). Guardrails stay **parent-only** (E4 below). The `write_todos` *tool*
still reaches children via the base snapshot; because E3 makes todos stateful,
each child builds its **own** todos handle + `CuratedContext` todos block
(mirroring the per-child `artifacts`/`compact_flag` wiring in `dispatch.rs`),
so a child plans for itself and its plan is never merged to the parent
(deepagents contract). New pin: a dispatched child, given a malformed child
turn, re-asks exactly once with the same message — byte-identical to today.

**Child guardrail — deferred to 3B (E4 resolved 2026-07-09).** The panel
(Fa-F8) correctly showed the varying-args runaway threat applies to a child
*identically* to the parent (`subagent_max_turns × max_parallel_tools` is a
large uncapped child tool-call budget), so the draft's "unmotivated" was
wrong. But E2 already re-opens the Phase-2 child-stack pin once (for repair
parity); adding a child `ToolCallLimit` here would re-open it a second time in
this spec. The owner's call: **defer the child guardrail to 3B**, where the
dispatch/child subsystem is already open and the named-subagent registry gives
it richer shape (per-subagent limits). 3A records the shared-threat residual as
an explicit **3B input** (children are turn-bounded meanwhile, so the residual
is contained within one dispatch). Not a silent omission — a scheduled one.

### 5.7 What `run_with_cancel` looks like after

The loop keeps everything Phase-1/2 left it owning. It gains: the
`on_parse_failure` call-out at the total-parse-failure arms (`loop_.rs:789-801`,
replacing the inline `protocol_repairs`/give-up block), the optional pre-turn
`ToolCallLimit` guard at the turn-loop top (§5.5), the `RunShared` construction
(fresh per run) and its threading into `ModelNext`/`ToolNext`/`RunCx`, and
nothing else. The
model-call and tool-call wrap-chain construction sites are unchanged except
for the added `shared` field. Everything emitted, appended, and ordered today
is emitted, appended, and ordered identically; the three migrated/added
behaviors are the only new owners.

## 6. Judgments and gate escalations

Escalations (conflict with or reinterpret the mandate — none silently adopted
or dismissed). **All resolved at the spec-review gate, 2026-07-09 (Kalen):**

- **E1 — Ship `ModelCallLimit`? RESOLVED: keep, default-off.** Against the
  panel's cut recommendation (Req F4 + Scope F1: subsumed by `max_turns`, no
  runaway scenario). Owner's reason: it gives `wrap_model_call` a real
  consumer, so both wrap surfaces are proven after 3A, not just the tool half.
  Default-off honors the panel finding (it is opt-in, not imposed behavior),
  while keeping the seam-proving benefit (§5.5).
- **E2 — Repair hook vs lookup? RESOLVED: the new `on_parse_failure` node
  hook** (§4 F was the surfaced alternative). Repair becomes a genuine
  pluggable middleware (bundle framing) and stays uniform with the Phase-1
  hook pattern; the two pin re-pins (§3.1 #1, #3) + child-stack change are
  accepted as routine. The lookup alternative is recorded (§4 F) but not taken.
- **E3 — `write_todos` durability? RESOLVED: pin/recall (durable pinned
  block)** over the cheaper `exclude_tools` carve-out — the hard in-window
  guarantee, matching deepagents. Consequences accepted and bounded (§5.4,
  §3): `write_todos` becomes stateful (**J10 reversed**), `CuratedContext`
  gains a todos pinned block appended *after* the existing calibrated order
  (existing order byte-identical), and the extended pinned region is a
  measured-behavior change flagged for the parked context-evolve
  re-measurement.
- **E4 — Child `ToolCallLimit`? RESOLVED: deferred to 3B.** The panel (Fa-F8)
  correctly showed the runaway threat is shared, so the draft's "unmotivated"
  was wrong; but E2 already re-opens the child-stack pin once, and the
  child-guardrail decision fits 3B (subagent subsystem already open, per-
  subagent limits). 3A records the residual as a 3B input (§5.6).
- **E5 — `RunShared` shape? RESOLVED: keep the typed map.** Panel CLEARED it
  (Scope F2) as the wrap-side analog of Phase-1's accepted node-`RunState`
  typemap; owner confirms. The per-tool-call `Mutex`+downcast cost is the
  accepted price of the extension surface the modularity goal wants.

**Decomposition (owner-resolved, recorded):**

- **J1 — Decomposition into 3A/3B + cache deferral (§0).** Resolved by the
  owner at brainstorm 2026-07-09. Panel: the split is a clean requirements
  decision (Req F6 clean), cache deferral grounding independently verified
  (R-F6, §0).

Judgments (held, panel-tested):

- **J5 — Repair is a node hook, not a wrap (§5.1).** Parse is a loop fact
  post-wrap (verified A5/A6: parse follows the wrap chain, and the re-ask
  `continue` recomputes `est` at 687); `continue` keeps calibration honest.
  The Posture-1 resolution of the Phase-1 provisional-wrap watch item. (The
  hook-vs-lookup *mechanism* for making it pluggable is E2.)
- **J6 — `RunState` (node, `&mut`) and `RunShared` (wrap, `Arc`) are two
  facilities (§5.2).** Different concurrency contracts; one `&mut` map cannot
  serve the parallel tool executor (verified A8: `buffer_unordered`).
- **J7 — `TodoListMiddleware` is a hookless tool-contributing middleware, not
  a base tool (§4 D, §5.4).** Still holds after E3: it is hookless (tool
  contribution + a caller-wired state handle), the exact
  `ContextCurationMiddleware`+`compact_flag` shape already in-tree.
- **J8 — Guardrails are `StuckDetection` siblings, not a refactor (§4 E).**
- **J9 — Children get repair (parity), not planning-middleware or guardrails
  (§5.6).** Child `ToolCallLimit` **deferred to 3B (E4)** — the shared runaway
  threat the draft dismissed as "unmotivated" is real but scheduled, not
  ignored.
- **J10 — REVERSED at E3.** `write_todos` now holds state (the shared todos
  handle) rendered as a durable pinned block (§5.4) — the accepted cost of the
  pin/recall in-window guarantee. It remains hookless (J7).

## 7. Testing

**Parity (assertion bodies unchanged):** the whole Phase-1 stack-mechanics +
cadence suite, the Phase-2 backend/curation/child suites, `StuckDetection`
pins, the Phase-1 wrap-composition pins (first-outermost nesting,
invoked-twice-on-overflow, gate-unreachability — the added `shared` field does
not perturb them), calibration pins.

**Superseded pins (map to §3.1):**
- Phase-1 hook-firing-set assertions extended: add "`on_parse_failure` fires
  only at the total-parse-failure arms (`loop_.rs:789-801`), after the Length
  short-circuit and upstream of the second Length guard (808-813)" and
  re-confirm the existing five hooks' firing points are unchanged.
- Phase-1 wrap-contract pins re-pinned on the `shared`-carrying
  `ModelNext`/`ToolNext` (composition semantics identical).
- Phase-2 child-stack pin re-pinned as `[curation, stuck, repair]` (subject to
  E2 — vanishes if the lookup alternative is chosen).
- Loop-resident-repair parity: the malformed→repaired appended-message diff
  moves from a loop test to a `RepairMiddleware` test, same assertion.

**New tests:**
- `on_parse_failure`: fires at the failure arms; does **not** fire on success,
  partial-invalid, Length-truncation, cancel, overflow, budget; reverse-order
  fold picks the first `ReAsk`; all-`GiveUp` → today's terminal behavior;
  empty-`raw.text` re-ask appends a balanced (orphan-free) assistant message.
- `RunShared`: typed roundtrip; fresh-per-run isolation; concurrent increments
  from parallel `wrap_tool_call` closures don't lose updates (panel F7);
  written-in-wrap / read-in-node-hook works. **Poison test (panel Fa-F6):** a
  wrap closure that panics inside `with()` in a `buffer_unordered` batch does
  **not** poison-cascade — sibling calls and the `after_tools` guardrail see a
  recovered (`into_inner`) value, the panicking call is contained as
  `Executed::Panicked`, and the run is not unwound by a `PoisonError`.
- `RepairMiddleware`: default = byte-identical one re-ask; second failure
  terminal; stuck counters untouched by repair turns; re-asks consume `turn`
  (so `max_turns` bounds them).
- `TodoListMiddleware` (E3 pin/recall): `write_todos` sets the shared handle
  and returns a compact confirmation; `CuratedContext` renders the list as the
  **last pinned block**. **Existing-order-preserved test:** goal/ledger →
  recall → summary+pointer is byte-identical with and without todos (strictly
  appended); `pinned_tokens()` stays in lockstep with `pinned()`.
  **Hard-durability test (panel R-F2/Fa-F1):** the todos block is still present
  in the built window after N subsequent tool turns AND after an
  offload/compaction cycle (it is pinned → unconditionally in-window until
  overwritten — the guarantee `exclude_tools` could not give). Child-visible;
  each child renders its own todos block (no parent merge). Tool-description
  snapshot test guards the multiple-independent-`in_progress` wording (B1).
- `ToolCallLimit`: crossing the cap ends the run with `StopReason::Error` (not
  `BudgetExhausted` — panel A10/Fa-F2); a **varying-args** runaway (distinct
  args each turn — the shape `StuckDetection` cannot catch) is what drives it
  to the cap (panel R-F5), not a fabricated identical loop; the pre-turn guard
  bounds overshoot to at most one turn's batch, and the test asserts the
  **precise** bound including a fat crossing batch (panel Fa-F3);
  count-on-failure pinned (a panicked/timed-out call still increments); a
  bounded run well under the cap is unaffected (no parity break).
- `ModelCallLimit` (E1: ships default-off): when **enabled**, analogous at
  `after_model`, ends with `Error`; when **default (off)**, inert (no behavior
  effect) — pinned so the default stays non-imposing.
- Guardrail/stuck ordering (panel Fa-F5): on a turn where a `ToolCallLimit`
  `EndRun` at `after_tools` co-fires with a `StuckDetection` `nudge_pending`,
  assert the **content** outcome — nudge dropped, transcript balanced, no
  orphaned tool_calls — not merely deterministic ordering.
- Child repair parity: a dispatched child re-asks exactly once, same message;
  child `RepairState` isolates (no cross-child bleed).
- **Gate:** `bash scripts/ci.sh` green.

## 8. Open questions

- `Repair` reserving a third variant for patch-in-place — deepagents'
  `PatchToolCallsMiddleware` *fixes* dangling/malformed calls in message
  history rather than re-asking (panel B4 confirmed this characterization is
  accurate; that middleware lives in **deepagents**, unlike the count-limit
  ones). 3A's repair is the loop's existing *re-ask*, a different pattern; a
  patch variant has no 3A consumer and is omitted (YAGNI), revisit if a patch
  strategy lands.
- **Loop-owned per-run re-ask ceiling (panel Fa-F4).** `max_turns` already
  bounds re-asks (each consumes a turn), but composing multiple repair
  middleware *sums* their `MAX_REPAIRS` with no global cap under `max_turns`.
  A loop-side ceiling independent of any middleware's counter is the defensible
  hardening given the modularity goal invites custom repairers; deferred as
  optional since the single default repairer is `max_turns`-bounded.
- Guardrail cap constant (1000 tool) is an order-of-magnitude guess; a workload
  that legitimately exceeds it turns the "no config knob" non-goal into a real
  knob request — the named constant makes that a one-line change (plan-level).
- Whether `on_parse_failure`'s fold should let a middleware *transform* the
  give-up (e.g. a custom terminal message) — Phase 3A keeps give-up
  loop-owned; a `GiveUp(Option<String>)` is a trivial later extension.

## Panel & review log

- **2026-07-09 — adversarial spec panel** (4 independent skeptical reviewers,
  opus×4: Requirements, Assumptions incl. live LangChain/deepagents provenance
  drift check, Failure & abuse, Scope & simpler design). Every load-bearing
  live-source premise was independently verified TRUE (repair→node-hook
  calibration honesty via `continue`-recompute at `loop_.rs:687`; wrap-seam
  statelessness; `buffer_unordered` forcing `RunShared`; child-repair-today;
  stuck-untouched-by-repair; `EndRun`→`Done` with no `BudgetExhausted`
  collision; `est`/`calib_ratio_micros` boundary genuinely undisturbed). Clean
  bills: no double-`Done`, no `RunShared` lost updates, guard-can't-cross-await
  by construction, child repair isolation, decomposition split, cache-deferral
  grounding (independently grepped). Dispositions:

  - **Blockers/majors fixed in place:**
    1. **`write_todos` recitation offloaded** (Req F2 + Fa F1, independent) —
       the recited plan is an offload-eligible tool result, silently evicted on
       the long runs it's for. Fixed at panel time with an `exclude_tools`
       carve-out; **the gate then chose the stronger pin/recall durable-block
       fix (E3 below)**, which supersedes the carve-out. §3 invariant row now
       reads "existing pinned order byte-identical; todos appended after it."
    2. **`RunShared` poison escapes tool-panic isolation** (Fa F6) — a panic in
       one parallel `wrap_tool_call` would poison the shared `Mutex` and unwind
       the whole run via un-isolated node hooks, *worse* than today. Fixed:
       `with()` recovers via `into_inner()`, never propagates poison
       (monotonic counters fail safe); non-reentrancy documented; poison test
       added (§5.2, §7).
    3. **`BudgetExhausted` overload** (A10 + Fa F2) — guardrail `EndRun`
       skipped the wrap-up completion natural exhaustion runs. Fixed: guardrails
       use `StopReason::Error` like the sibling `StuckDetection` (§5.5).
    4. **`write_todos` "one in_progress" inverted the real LangChain contract**
       (B1) — ships verbatim to the model. Fixed: permit multiple independent
       `in_progress` (§5.4) + description snapshot test.
    5. **`ToolCallLimit` overshoot + count-on-failure** (Fa F3) — precise
       turn-boundary bound stated, pre-turn guard added, count-on-failure
       pinned (§5.5, §7).
    6. **Provenance mis-homing** (B2/B5) — `TodoList`/`ModelCallLimit`/
       `ToolCallLimit` are LangChain (consumed by deepagents), not "deepagents
       ships"; corrected throughout; LangChain's `continue` exit-behavior
       noted as a deliberate non-port.
    7. **Framing** — "finalizes the wrap contract" → **non-mutating** only
       (Req F8, §1/G4); modularity payoff stated as three distinct
       justifications, not symmetric (Req F1, §1); repair-site anchor corrected
       to the two arms `789-801` (A1); guardrail/stuck nudge-drop pinned at the
       content level (Fa F5); re-ask bounds (turn-bounded, empty-text parity,
       multi-repairer residual) stated (Fa F4); §4 gained the missing
       lookup-repair alternative F (Scope F3).

  - **Escalated to the gate (owner's call — not silently adopted or dismissed):**
    - **E1 — cut `ModelCallLimit`?** Panel converged (Req F4 + Scope F1): CUT
      (no scenario; subsumed by `max_turns`). Kept pending owner because the
      `wrap_model_call`-consumer rationale touches the seam-proving goal.
    - **E2 — repair hook vs loop-resident-policy-lookup** (Scope F3). Hook
      chosen (uniform + faithful); lookup is cheaper (collapses two pin re-pins
      + the child-stack change). Owner's design call.
    - **E3 — ratify the `write_todos` `exclude_tools` carve-out** vs pin/recall
      vs accept-degradation.
    - **E4 — child `ToolCallLimit`?** (Fa F8) The runaway threat is shared;
      the draft's "unmotivated" was wrong. Ship on children vs accept the
      turn-bounded residual.
    - **E5 — `RunShared` typed map vs concrete atomics** (was J2). Panel
      **cleared** the typed map on Phase-1 precedent; owner confirms the
      per-tool-call `Mutex`+downcast cost is wanted.

  - **Minors accepted as residual:** child-visible planning is "free" not
    requirement-driven (Req F3); `max_turns`-bounded multi-repairer re-ask sum
    (Fa F4, loop-ceiling hardening parked §8); `G5`-as-acceptance-criterion
    (Req F7); LangChain `ToolCallLimit` richer exit surface deliberately not
    ported (B5).

- **2026-07-09 — spec-review gate (Kalen): all escalations resolved.**
  **E1** keep `ModelCallLimit` **default-off** (a `wrap_model_call` consumer so
  both wrap halves are proven; opt-in, not imposed — honors the panel's
  subsumed-by-`max_turns` finding). **E2** the new `on_parse_failure` **hook**
  (repair is a genuine pluggable middleware; the two pin re-pins + child-stack
  change accepted). **E3** **pin/recall** — the durable-pinned-block fix over
  the cheaper carve-out, for the hard in-window guarantee; consequences
  accepted and bounded (J10 reversed, todos appended after the calibrated
  order, measured-behavior flag for the parked context-evolve re-measurement).
  **E4** child `ToolCallLimit` **deferred to 3B** (the child-stack pin is
  already re-opened once by E2; the subagent subsystem is 3B's; residual
  recorded as a 3B input). **E5** keep the **typed `RunShared` map** (panel-
  cleared on Phase-1 precedent).
- **Post-gate design change flagged for a consistency read (per AGENTS.md).**
  E3's switch from `exclude_tools` to a stateful pin/recall block is more than
  disposition prose — it adds a `CuratedContext` field, a shared handle, a
  pinned-block render, per-child todos wiring, and a measured-behavior touch of
  the eval-calibrated pinned region. A light-tier consistency + soundness read
  over the revised §5.4/§3/§5.6 ran **before** plan (below). Not a full
  re-panel (localized, not structural rework).
- **2026-07-09 — post-gate consistency read (light tier, sonnet): CLEAN.**
  (a) Stale-language / contradiction / cross-reference sweep: no pre-gate
  language survives (no "stateless"/"holds no state"/"no CuratedContext
  change" for `write_todos`; `ModelCallLimit` "kept, default-off" everywhere,
  "cut" only as the rejected panel recommendation; guardrails uniformly
  `EndRun(Error)`, `BudgetExhausted` only as the corrected-draft mention;
  `exclude_tools` only as the superseded alternative); all §/E/F/J
  cross-references resolve; no disposition contradicts a normative section.
  (b) E3 soundness verified against live source: `pinned()` order
  (`curated.rs:108-137`) is goal/ledger → recall → summary+pointer, so a
  last-appended todos block leaves the existing order byte-identical (TRUE);
  `pinned_tokens()` carries a pre-existing "Kept in lockstep with `pinned()`"
  invariant (`curated.rs:173-196`) that adding a block genuinely extends
  (TRUE); the `compact_flag` per-child template is exact —
  `dispatch.rs:474-496` mints a fresh `Arc` + fresh `CuratedContext` per child,
  precisely the shape §5.6 mirrors for the todos handle (TRUE); no blocking
  problem in the mechanism. (c) Cross-section coherence (§5.4/§3/§5.6/§7/G1/
  J7/J10): one consistent stateful-pinned-todos story, no drift. **No fixes
  required.** Spec is plan-ready.
