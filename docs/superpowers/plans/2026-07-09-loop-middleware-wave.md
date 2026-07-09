# Loop middleware wave (Phase 3A) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move three cross-cutting loop behaviors — malformed-tool-call *repair*, model/tool-call *guardrail limits*, and `write_todos` *planning-by-recitation* — off `AgentLoop`'s hard-wired body and onto the Phase-1 middleware stack, discharging the Phase-1 provisional (non-mutating) wrap contract with a real shipping consumer.

**Architecture:** Three deliverables, argued against the governing goal (deepagents-style *modularity*: custom middleware/filesystems/extensions change runtime behavior). (1) A new node hook `on_parse_failure` + a `RepairMiddleware` reproduce today's one-shot re-ask **byte-identically**, off the loop. (2) A new per-run thread-safe state facility `RunShared` lets `ToolCallLimit`/`ModelCallLimit` count in wrap hooks and enforce in node hooks — the genuinely-new `wrap → RunShared → node-hook` extension capability. (3) `TodoListMiddleware` contributes a `write_todos` tool whose current list is rendered as a **durable pinned block** in `CuratedContext` (pin/recall, E3), giving planning a hard in-window guarantee. Staged in two independently-reviewable waves (parity seam first, then behavior), mirroring Phase 2.

**Tech Stack:** Rust (async_trait, tokio, futures `buffer_unordered`), all in the `agent/` Cargo workspace. Primary crate: `agent-core` (`middleware.rs`, `loop_.rs`, `curated.rs`, `dispatch.rs`, a new `todos.rs`). Stack wiring in `agent-runtime-config` (`assemble.rs`) + frontends (`agent-cli`, `agent-server`). No new dependencies.

**Source of truth:** `docs/superpowers/specs/2026-07-09-loop-middleware-wave-design.md` (PLAN-READY; E1–E5 resolved at gate). Builds on Phase-1 (`2026-07-08-middleware-seam-design.md`, merged 707d7fd) and Phase-2 (`2026-07-08-backend-seam-design.md`, merged 71e23d1). **Live-source baseline: commit 71e23d1, re-read 2026-07-09.**

---

## Global Constraints

Every task's requirements implicitly include this section. Values copied verbatim from the spec.

- **Parity bar (spec §3, §7):** the whole `agent/` suite passes with **assertion bodies unchanged** except at the four deliberately-superseded pins (§3.1). Construction-site updates (adding a middleware to a test's stack) are allowed; assertion *bodies* are not.
- **Four superseded pins only** (§3.1): (1) hook-firing-set — extended by `on_parse_failure`; (2) wrap-hook contract — extended additively with the `shared` field; (3) child-stack `[curation, stuck]` → `[curation, stuck, repair]`; (4) loop-resident repair location → `RepairMiddleware`.
- **Byte-identical:** the repair re-ask message string is exactly `"Your tool call could not be parsed: {err}. Re-emit it correctly."`; the terminal give-up emits `AgentEvent::Error(err)` then `AgentEvent::Done(StopReason::Error)` then `return Ok(())`; the malformed→repaired→resolved appended-message sequence is unchanged.
- **No config knobs** for repair counts or guardrail caps — hardcoded, **named** constants (`MAX_REPAIRS = 1`, `TOOL_CALL_LIMIT = 1000`, `MODEL_CALL_LIMIT = 500`) so a future knob is a one-line change (matches the stuck-detection precedent).
- **No wire or frontend change** (spec §2): `write_todos` is already visible as an ordinary tool call in every frontend; no new event types, no `ContextSnapshot` wire-shape change (see Deviation D5).
- **Guardrail abort contract (E1/§5.5):** `Flow::EndRun(StopReason::Error)` + an emitted `AgentEvent::Error` — **never** `BudgetExhausted` (which the loop reserves for graceful `max_turns` exhaustion, and which triggers a wrap-up completion a guardrail abort must not).
- **`StuckDetection` is untouched** — guardrails are *siblings* added after it in stack order; its code and pins do not change.
- **Calibration untouched:** no new code reads or writes `calib_ratio_micros`; no `wrap_model_call` in 3A mutates its request. Repair re-iterates via the loop's existing `continue` → `est_prompt_tokens` recomputed at the turn top; guardrails only count.
- **Locate quoted code by content, not line number.** All `loop_.rs:NNN` anchors below are orientation only and drift; the quoted snippet is authoritative. Any sub-agent prompt derived from this plan must repeat this instruction.
- **Branch:** this is a code change → the implementation session branches off `main` (`feature/loop-middleware-wave`). Do **not** commit or push unless asked.
- **Gate:** `bash scripts/ci.sh` green (okf check + skills lint + fmt + clippy + `cargo test` in `agent/` + conditional `src-tauri` + web typecheck/vitest).

---

## Spec discrepancies found during live-source grounding

These were caught reading the code at commit 71e23d1 and must be surfaced at the **plan-review gate**. None blocks 3A; each is handled as noted. Record the gate's disposition in the spec's Panel & review log (or a plan log) before implementation.

**S1 — `execute_isolated` does NOT contain wrap-level panics (affects the §5.2/§7 poison claim).**
`execute_isolated` (`loop_.rs`, `pub(crate) async fn execute_isolated`) wraps **only `tool.execute(args, ctx)`** in `catch_unwind` — it is the *base case inside* the `ToolNext` chain (`middleware.rs::ToolNext::run` calls it as `None =>`). A panic in a `wrap_tool_call` closure (e.g. inside `RunShared::with`) fires *outside* that `catch_unwind`, propagates up through `buffer_unordered` → `collect().await`, and **unwinds the whole run** — it does **not** become `Executed::Panicked`. So the spec's §7 wording *"the panicking call is contained as `Executed::Panicked`"* is inaccurate.
- **What is still true and worth implementing:** `RunShared::with` recovering poison via `into_inner()` (never propagating `PoisonError`) prevents a *poison cascade* if a poisoned lock is later read, and 3A's guardrail closures (`|c| c.0 += 1`) never panic anyway (monotonic increments fail safe). The `into_inner` recovery is kept as the defensive contract.
- **Plan handling:** the poison test (Task A1) asserts only the true property — a `RunShared` whose mutex was poisoned by a panicking `with()` closure still returns the recovered value from a later `with()`, with no re-panic. The plan does **not** write a loop-level test asserting a wrap panic becomes `Executed::Panicked`. Flag S1 for a spec-text correction.

**S2 — repair reset-on-success mechanism (spec §5.3 is underspecified).**
Today the loop resets `protocol_repairs = 0` on **every successful parse** (the `Ok(p) => { protocol_repairs = 0; p }` arm), so a run may have multiple re-ask *episodes*, each capped at one consecutive re-ask; only **two consecutive** failures are terminal. Spec §5.3 says `RepairMiddleware` "implements only `on_parse_failure`" and does not describe a reset — which, taken literally, would make the *second failure anywhere in the run* terminal (a behavior change). The spec's own wording *"in a turn-sequence"* (§5.3) and the "terminal-on-second-failure" (§3.1 #4) point at per-consecutive-sequence semantics.
- **Plan handling (byte-identical, no extra hook):** `RepairState` tracks `{ repairs, last_fail_turn }` and resets `repairs` to 0 whenever the current failing turn is **not contiguous** with `last_fail_turn` (an intervening successful parse leaves a turn-index gap, because each re-ask `continue`s and advances `turn`). This reproduces today's reset-on-success using only `on_parse_failure` + `cx.turn`. Verified against the trace in the task. Flag S2 as the chosen reading.

**S3 — the pre-turn `ToolCallLimit` guard is provably redundant given `after_tools` (kept per §5.7).**
`after_tools` fires on every tool turn and always ends the run on the crossing turn, so it alone bounds overshoot to one turn's batch; the pre-turn guard at the turn-loop top (§5.7) is therefore a backstop that never fires in normal operation. It is implemented (faithful to §5.5/§5.7, cheap, and generic — it reads a `ToolCallCount` that stays 0 unless some middleware counts) and documented as a backstop; the overshoot-bound test is satisfied by `after_tools`. Flag S3 so the gate knows the guard is intentional belt-and-suspenders, not the effective bound.

**S4 — `TodoListMiddleware` config gating (spec §5.6/G1 says "iff cfg enables planning (default on)").**
There is no `cfg.planning` field today, and adding one ripples through `RuntimeConfig` construction. To keep blast radius minimal and match the spec's own no-runtime-knob philosophy (named constants, one-line future knob), the plan pushes `TodoListMiddleware` **unconditionally** into the parent stack (like `ContextCurationMiddleware`/`StuckDetectionMiddleware`), which realizes "default on" and keeps the stack entry droppable in code. A `cfg.planning` toggle is a one-line future addition. Flag S4 for the gate to accept "unconditional" or request the config field.

**S5 — `ContextSnapshot` must gain a `todos` segment to preserve the audit-7.3 `est_total` invariant (resolved during review).**
`snapshot.rs` maintains a documented invariant: `est_total == pinned_tokens() + history_tokens` (`build_snapshot` sums per-segment `est_tokens`; the `ledger` segment exists *specifically* to keep this equal to the budget math — see the "audit 7.3" comment). The test `snapshot_est_total_includes_the_pinned_ledger` (`curated.rs`) pins it. The plan's first draft left `build_snapshot` unchanged, which would silently **break** that invariant whenever todos is non-empty (updated `pinned_tokens()`, stale `est_total`). Adversarial review caught this. **Resolution (applied in Task B3):** add a `todos` segment to `build_snapshot` mirroring the `ledger` segment. This is **not** a forbidden wire/frontend change (§2): `ContextSnapshot`'s *shape* is unchanged (`segments` is already a variable-length `Vec<ContextSegment>` whose membership depends on what's present — `ledger`/`memory`/`summary` come and go), and the web renders segments **generically** (`ContextExplorer.tsx`: `snap.segments.map(...)` with `COLORS[category] ?? "var(--text-muted)"`; `breakdown.ts`: generic map) — an unknown `todos` category renders as a muted row with **zero web code change** and no typecheck break (verified in `web/src`). §2's "no frontend change" targets new event types / the typed subagent stream (3B), not an incidental estimate segment. `build()`'s window budget already used the correct updated `pinned_tokens()`, so the real context was never mis-budgeted; this fix only realigns the display estimate. Flag S5 for the gate only to confirm the segment-addition reading of §2 (recommended: accept — it is strictly more correct than a broken invariant).

---

## File structure

| File | Responsibility | Wave |
|---|---|---|
| `agent/crates/agent-core/src/middleware.rs` (modify) | `RunShared` facility; `Repair` enum + `on_parse_failure` trait method; `RepairMiddleware`; `ToolCallLimit`; `ModelCallLimit`; `ToolCallCount`; the `shared` field on `ModelNext`/`ToolNext`/`RunCx`; `TodoListMiddleware`. | A + B |
| `agent/crates/agent-core/src/loop_.rs` (modify) | Construct `RunShared` fresh per run; thread `shared` into all `fire_*` + `ModelNext` + `ToolNext`; `fire_on_parse_failure` dispatcher; refactor the parse-failure arms; the loop-top pre-turn guard. | A + B |
| `agent/crates/agent-core/src/todos.rs` (**create**) | `TodoItem`, `TodoStatus`, `WriteTodosTool`, `render_todos_block()`. | B |
| `agent/crates/agent-core/src/curated.rs` (modify) | `todos` handle field + `with_todos` builder; render the list as the **last** pinned block in `pinned()`; account it in `pinned_tokens()`. | B |
| `agent/crates/agent-core/src/dispatch.rs` (modify) | Add `RepairMiddleware` to the child stack; per-child todos handle + rebind child `write_todos` + `child_ctx.with_todos`. | A + B |
| `agent/crates/agent-core/src/lib.rs` (modify) | `pub mod todos; pub use todos::*;` | B |
| `agent/crates/agent-runtime-config/src/assemble.rs` (modify) | Push `TodoListMiddleware`, `ModelCallLimit`, `ToolCallLimit`, `RepairMiddleware` in stack order; `LoopParts.todos` field wired into `TodoListMiddleware`. | A + B |
| `agent/crates/agent-cli/src/main.rs` (modify) | Create a `todos` handle; put it in `LoopParts.todos`; `.with_todos(...)` the parent `CuratedContext`. | B |
| `agent/crates/agent-server/src/runtime.rs` + `session.rs` (modify) | `RuntimeState` holds a session-stable `todos` handle + `todos()` accessor; `LoopParts.todos`; `.with_todos(...)` both `CuratedContext` sites. | B |

`agent-core` already depends on `agent-tools`; no new dependency edges. `TodoItem`/`TodoStatus` live in `agent-core` (used by both the tool and `curated.rs`).

---

# WAVE A — parity seam (no new behavior)

Uncontroversial, byte-identical work: the `RunShared` facility, the `on_parse_failure` hook, and the repair migration. Lands independently reviewable before any behavior change.

## Task A1: `RunShared` — per-run thread-safe typed state

**Files:**
- Modify: `agent/crates/agent-core/src/middleware.rs` (add `RunShared` near `RunState`; extend the `use` for `std::sync::Mutex`)
- Test: `agent/crates/agent-core/src/middleware.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `pub struct RunShared(Arc<Mutex<HashMap<TypeId, Box<dyn Any + Send>>>>)` with `#[derive(Clone, Default)]` and `pub fn with<T: 'static + Default + Send, R>(&self, f: impl FnOnce(&mut T) -> R) -> R`.

- [ ] **Step 1: Write the failing tests.** Add to `middleware.rs`'s `#[cfg(test)] mod tests`:

```rust
#[derive(Default)]
struct Counter(usize);

#[test]
fn run_shared_typed_roundtrip_and_fresh_isolation() {
    let s = RunShared::default();
    assert_eq!(s.with::<Counter, _>(|c| c.0), 0); // get-or-default
    s.with::<Counter, _>(|c| c.0 = 7);
    assert_eq!(s.with::<Counter, _>(|c| c.0), 7);
    // A fresh RunShared (fresh run) starts empty — per-run lifetime.
    let s2 = RunShared::default();
    assert_eq!(s2.with::<Counter, _>(|c| c.0), 0);
    // Clones share the same underlying store (Arc): a write via one clone is
    // visible through another — this is what threads a wrap-write to a node-read.
    let s3 = s.clone();
    s3.with::<Counter, _>(|c| c.0 = 99);
    assert_eq!(s.with::<Counter, _>(|c| c.0), 99);
}

#[test]
fn run_shared_concurrent_increments_dont_lose_updates() {
    // Serialized under the Mutex: 8 threads × 1000 increments == 8000, no lost
    // updates (the property the parallel buffer_unordered tool executor needs).
    let s = RunShared::default();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let s = s.clone();
            std::thread::spawn(move || {
                for _ in 0..1000 {
                    s.with::<Counter, _>(|c| c.0 += 1);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(s.with::<Counter, _>(|c| c.0), 8000);
}

#[test]
fn run_shared_recovers_from_poison_never_propagates() {
    // A panic inside a with() closure poisons the std Mutex. RunShared MUST
    // recover via into_inner() and never re-panic on PoisonError (spec §5.2;
    // see plan discrepancy S1 — this is the true, achievable poison property).
    let s = RunShared::default();
    s.with::<Counter, _>(|c| c.0 = 5);
    let s2 = s.clone();
    let joined = std::thread::spawn(move || {
        s2.with::<Counter, _>(|_| panic!("boom while holding the guard"));
    })
    .join();
    assert!(joined.is_err(), "the closure panic propagates on ITS thread");
    // Despite the poisoned mutex, a later with() recovers the prior value.
    assert_eq!(
        s.with::<Counter, _>(|c| c.0),
        5,
        "poisoned mutex recovered via into_inner; value intact"
    );
}
```

- [ ] **Step 2: Run to verify they fail.**
Run: `cargo test -p agent-core --lib middleware::tests::run_shared`
Expected: FAIL — `RunShared` not defined.

- [ ] **Step 3: Implement `RunShared`.** In `middleware.rs`, add `Mutex` to the imports (change `use std::sync::Arc;` to `use std::sync::{Arc, Mutex};`) and add, directly after the `RunState` impl block:

```rust
/// Per-run, thread-safe typed state (spec §5.2 — the wrap-hook state facility).
/// Created fresh per `run_with_cancel`, reachable from BOTH wrap hooks (via
/// `ModelNext`/`ToolNext`) and node hooks (via `RunCx`), so a middleware can
/// WRITE in a wrap and READ in a node hook. `RunState` (node, `&mut`,
/// sequential) and `RunShared` (wrap, `Arc`, concurrent) are two facilities
/// with different concurrency contracts — a single `&mut` map cannot serve the
/// parallel `buffer_unordered` tool executor.
#[derive(Clone, Default)]
pub struct RunShared(Arc<Mutex<HashMap<TypeId, Box<dyn Any + Send>>>>);

impl RunShared {
    /// Get-or-default then apply `f` under the lock. The SYNCHRONOUS
    /// `FnOnce(&mut T) -> R` structurally forbids `.await` inside the lock (a
    /// guard cannot cross an await by construction). Poison recovery: recovers
    /// via `into_inner()` and DELIBERATELY DOES NOT PROPAGATE POISON — a tool
    /// panic must stay contained, and the monotonic counters this holds fail
    /// safe (a torn value can only over-count, stopping a run slightly early,
    /// never late). NON-REENTRANT: `f` must not call `with()` again (one Mutex
    /// guards the whole map → nested `with()` self-deadlocks); 3A's guardrails
    /// touch one key each, so no nesting.
    pub fn with<T: 'static + Default + Send, R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let slot = guard
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(T::default()));
        let val = slot
            .downcast_mut::<T>()
            .expect("TypeId-keyed entry always downcasts to its own type");
        f(val)
    }
}
```

- [ ] **Step 4: Run to verify they pass.**
Run: `cargo test -p agent-core --lib middleware::tests::run_shared`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit.**
```bash
git add agent/crates/agent-core/src/middleware.rs
git commit -m "feat(core): RunShared per-run thread-safe typed state (Phase 3A wave A)"
```

## Task A2: Thread `RunShared` through the wrap/node seam

**Files:**
- Modify: `agent/crates/agent-core/src/middleware.rs` (`shared` field on `ModelNext`, `ToolNext`, `RunCx`; accessor methods)
- Modify: `agent/crates/agent-core/src/loop_.rs` (construct `run_shared` once; add a `shared` param to the six `fire_*` helpers; thread into `ModelNext`/`ToolNext`)
- Test: `agent/crates/agent-core/src/loop_.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `RunShared` (A1).
- Produces: `RunCx::shared(&self) -> &RunShared`, `ModelNext::shared(&self) -> &RunShared`, `ToolNext::shared(&self) -> &RunShared`. `fire_*` helpers gain a `shared: &RunShared` parameter.

- [ ] **Step 1: Write the failing test.** A synthetic middleware writes a `RunShared` counter in `wrap_tool_call` and reads it back in `after_tools`, proving the same store threads both surfaces. Add to `loop_.rs` tests (reuse the existing `ScriptedModel`/registry test scaffolding — locate a nearby loop test that drives one tool turn, e.g. near `repair_turn_leaves_stuck_counters_untouched`, and mirror its harness):

```rust
#[tokio::test]
async fn run_shared_write_in_wrap_is_readable_in_node_hook() {
    use std::sync::{Arc, Mutex};
    #[derive(Default)]
    struct WrapCount(usize);
    // Records what after_tools observed, so the assertion can read it out.
    struct Probe(Arc<Mutex<Option<usize>>>);
    #[async_trait::async_trait]
    impl crate::Middleware for Probe {
        fn name(&self) -> &str {
            "probe"
        }
        async fn wrap_tool_call(
            &self,
            call: agent_tools::ToolCall,
            next: crate::ToolNext<'_>,
        ) -> crate::Executed {
            next.shared().with::<WrapCount, _>(|c| c.0 += 1);
            next.run(call.args).await
        }
        async fn after_tools(&self, cx: &mut crate::RunCx<'_>) -> crate::Flow {
            let seen = cx.shared().with::<WrapCount, _>(|c| c.0);
            *self.0.lock().unwrap() = Some(seen);
            crate::Flow::Continue
        }
    }
    let seen = Arc::new(Mutex::new(None));
    // Build an AgentLoop whose stack is [Probe], scripted to issue ONE tool
    // call then finish. (Use the file's existing single-tool-turn harness:
    // ScriptedModel with a tool-call turn then a text turn; registry() with a
    // no-op tool; .with_middleware(vec![Arc::new(Probe(seen.clone()))]).)
    // ... harness elided; see neighboring loop tests for the exact builder ...
    // assert the node hook saw the wrap's increment:
    assert_eq!(*seen.lock().unwrap(), Some(1));
}
```

*(Implementer note: fill the elided harness by copying the nearest existing single-tool-turn loop test's `ScriptedModel` + `registry()` + `AgentLoop::new(...).with_middleware(...)` setup. The assertion body — `Some(1)` — is the contract.)*

- [ ] **Step 2: Run to verify it fails.**
Run: `cargo test -p agent-core --lib run_shared_write_in_wrap_is_readable_in_node_hook`
Expected: FAIL — `ToolNext::shared`/`RunCx::shared` not defined.

- [ ] **Step 3a: Add the `shared` field + accessors in `middleware.rs`.**
`RunCx`: add field `pub(crate) shared: &'a RunShared,` (next to `maint`) and an accessor in the `impl RunCx<'_>` block:
```rust
    pub fn shared(&self) -> &RunShared {
        self.shared
    }
```
`ModelNext`: add field `pub(crate) shared: &'a RunShared,` and, in `impl<'a> ModelNext<'a>`, an accessor `pub fn shared(&self) -> &RunShared { self.shared }`. In `ModelNext::run`, when constructing the recursive `ModelNext { loop_, chain: rest, cancel }`, add `shared: self.shared`.
`ToolNext`: add field `pub(crate) shared: &'a RunShared,` and `pub fn shared(&self) -> &RunShared { self.shared }`. Its recursive `ToolNext { chain: rest, ..self }` already forwards `shared` via `..self`.

- [ ] **Step 3b: Construct `run_shared` and thread it in `loop_.rs`.**
Locate `let mut mw_state = crate::RunState::default();` (the per-run state creation, ~`loop_.rs:658`). Immediately after it add:
```rust
        // Per-run wrap/node shared state (spec §5.2), fresh per invocation so
        // middleware stay stateless. Empty stacks never touch it.
        let run_shared = crate::RunShared::default();
```
Add a `shared: &crate::RunShared` parameter to each of the six dispatcher helpers (`fire_run_start`, `fire_after_model`, `fire_after_tools`, `fire_turn_end`, `fire_final_reply`, and the new `fire_on_parse_failure` from Task A3), and inside each, add `shared,` to the `RunCx { ... }` literal. Update the five existing call sites (`self.fire_run_start(...)`, etc. — locate each by content) to pass `&run_shared`.
In the model-call site (`let model_result = crate::ModelNext { loop_: self, chain: &self.middleware, cancel: &cancel }`), add `shared: &run_shared,`.
In the tool executor, before `futures::stream::iter(...)`, the closure binds `let middleware = &self.middleware;`. Alongside it add `let shared = &run_shared;` (a `&RunShared`, `Copy`), and in the `crate::ToolNext { tool, name: &name, tctx: &ctx, chain: middleware, call: &call }` literal add `shared,`.

- [ ] **Step 4: Run the new test + the Phase-1 wrap-composition parity suite.**
Run: `cargo test -p agent-core --lib`
Expected: PASS. The Phase-1 wrap pins (first-outermost nesting, invoked-twice-on-overflow, gate-unreachability) pass with assertion bodies unchanged — the added `shared` field does not perturb composition semantics.

- [ ] **Step 5: Commit.**
```bash
git add agent/crates/agent-core/src/middleware.rs agent/crates/agent-core/src/loop_.rs
git commit -m "feat(core): thread RunShared through wrap/node hooks (Phase 3A wave A)"
```

## Task A3: Migrate repair to `RepairMiddleware` behind `on_parse_failure`

Atomic migration — the loop's inline repair moves to a middleware in one task so every commit stays byte-identical (a bare, no-middleware loop no longer repairs; the parent and child stacks gain `RepairMiddleware` to preserve behavior).

**Files:**
- Modify: `agent/crates/agent-core/src/middleware.rs` (`Repair` enum; `Middleware::on_parse_failure` default; `RepairMiddleware`; `RepairState`; `MAX_REPAIRS`)
- Modify: `agent/crates/agent-core/src/loop_.rs` (`fire_on_parse_failure` dispatcher; refactor the parse-failure match arms; remove the `protocol_repairs` local)
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (push `RepairMiddleware` last in the parent stack)
- Modify: `agent/crates/agent-core/src/dispatch.rs` (add `RepairMiddleware` to the child stack)
- Test: `middleware.rs` (RepairMiddleware unit); `loop_.rs` (hook-firing + parity pins, incl. construction-site updates to existing repair tests)

**Interfaces:**
- Consumes: `RunCx` (with `turn: Option<usize>`), `RunState::entry`.
- Produces: `pub enum Repair { ReAsk(String), GiveUp }`; `Middleware::on_parse_failure(&self, cx, raw: &agent_model::AssistantTurn, err: &str) -> Repair`; `pub struct RepairMiddleware` (`#[derive(Default)]`); `pub const MAX_REPAIRS: usize = 1`. `fire_on_parse_failure(&self, ctx, state, shared, cancel, turn, raw, err) -> Repair`.

- [ ] **Step 1: Write the failing RepairMiddleware unit tests** (in `middleware.rs` tests). These pin the byte-identical default policy and the S2 reset-on-non-contiguity:

```rust
fn parse_fail_cx<'a>(
    ctx: &'a mut dyn ContextManager,
    state: &'a mut RunState,
    shared: &'a RunShared,
    cancel: &'a CancellationToken,
    model: &'a Arc<dyn ModelClient>,
    turn: usize,
) -> RunCx<'a> {
    RunCx {
        ctx,
        sink: /* a test sink Arc */ unimplemented!("use CollectingSink"),
        cancel,
        state,
        shared,
        turn: Some(turn),
        maint: MaintView { maint_model: model, maint_model_limit: 100_000, effective_model_limit: 100_000 },
    }
}

#[tokio::test]
async fn repair_default_reasks_once_then_gives_up_on_contiguous_failure() {
    let raw = agent_model::AssistantTurn { text: "garbage".into(), ..Default::default() };
    let m = RepairMiddleware;
    // ... construct ctx/state/shared/cancel/model/sink via existing testkit ...
    // Turn 3: first failure of a streak → ReAsk with the exact message.
    let r = m.on_parse_failure(&mut cx_turn3, &raw, "bad json").await;
    assert_eq!(
        r,
        Repair::ReAsk("Your tool call could not be parsed: bad json. Re-emit it correctly.".into())
    );
    // Turn 4 (contiguous): second consecutive failure → terminal GiveUp.
    let r = m.on_parse_failure(&mut cx_turn4, &raw, "bad json").await; // same RunState
    assert_eq!(r, Repair::GiveUp);
}

#[tokio::test]
async fn repair_resets_on_non_contiguous_failure() {
    // Reproduces today's reset-on-successful-parse (loop_.rs Ok-arm): a failure
    // separated from the prior one by a gap in turn index (an intervening
    // success) re-asks again (plan discrepancy S2).
    let raw = agent_model::AssistantTurn { text: "garbage".into(), ..Default::default() };
    let m = RepairMiddleware;
    // Turn 3 fail → ReAsk (repairs 0→1, last_fail=Some(3)).
    assert!(matches!(m.on_parse_failure(&mut cx_turn3, &raw, "e").await, Repair::ReAsk(_)));
    // Turn 7 fail (gap: turns 4..6 parsed OK, no on_parse_failure) → non-contiguous
    // → repairs reset to 0 → ReAsk again.
    assert!(matches!(m.on_parse_failure(&mut cx_turn7, &raw, "e").await, Repair::ReAsk(_)));
}
```

*(Implementer note: `Repair` must derive `Debug, PartialEq` for these assertions. Build the shared `RunState`/`ctx`/`sink`/`model` from the existing testkit — `CollectingSink`, `ScriptedModel`, a `WindowContext`/`CuratedContext`. `AssistantTurn` construction: use its real fields; `..Default::default()` if it derives `Default`, otherwise fill required fields.)*

- [ ] **Step 2: Run to verify they fail.**
Run: `cargo test -p agent-core --lib middleware::tests::repair_`
Expected: FAIL — `Repair`/`RepairMiddleware`/`on_parse_failure` not defined.

- [ ] **Step 3a: Add the hook + enum + middleware in `middleware.rs`.**
Add the constant near the stuck constants:
```rust
/// One re-ask per consecutive parse-failure streak — the current
/// `protocol_repairs < 1` (spec §5.3). Named so a future knob is one line.
pub const MAX_REPAIRS: usize = 1;
```
Add the enum (near `Flow`):
```rust
/// Outcome of consulting the stack on a total tool-call parse failure (spec §5.1).
#[derive(Debug, Clone, PartialEq)]
pub enum Repair {
    /// Append the raw assistant text + this user message, then the loop
    /// `continue`s (fresh turn iteration; `est_prompt_tokens` recomputed → no
    /// calibration skew).
    ReAsk(String),
    /// Terminal: today's behavior — emit Error + Done(Error), return Ok.
    GiveUp,
}
```
Add the trait method to `trait Middleware` (after `after_final_reply`, before the wrap hooks):
```rust
    /// The model's output could not be parsed into tool calls at all
    /// (`protocol.parse` returned `Err`). The loop consults this at the
    /// total-parse-failure arms, AFTER the Length-truncation short-circuit and
    /// upstream of the second Length guard. Fires on nothing else: not on
    /// success, not on partial-invalid turns, not on Length truncation, cancel,
    /// overflow, or budget. Reverse stack order; the first `ReAsk` wins.
    async fn on_parse_failure(
        &self,
        _cx: &mut RunCx<'_>,
        _raw: &agent_model::AssistantTurn,
        _err: &str,
    ) -> Repair {
        Repair::GiveUp
    }
```
Add the middleware + state (near `StuckDetectionMiddleware`):
```rust
/// Malformed-tool-call recovery as a pluggable unit (spec §5.3): reproduces the
/// loop-resident one-shot re-ask byte-identically. Implements only
/// `on_parse_failure`; the loop still owns the `continue`/terminal.
#[derive(Default)]
pub struct RepairMiddleware;

#[derive(Default)]
struct RepairState {
    repairs: usize,
    /// The last turn index that failed to parse. A gap (an intervening success)
    /// resets `repairs`, reproducing the loop's reset-on-successful-parse
    /// (spec §5.3; plan discrepancy S2).
    last_fail_turn: Option<usize>,
}

#[async_trait]
impl Middleware for RepairMiddleware {
    fn name(&self) -> &str {
        "repair"
    }
    async fn on_parse_failure(
        &self,
        cx: &mut RunCx<'_>,
        _raw: &agent_model::AssistantTurn,
        err: &str,
    ) -> Repair {
        let turn = cx.turn;
        let s = cx.state.entry::<RepairState>();
        // Contiguous with the previous failure? (each re-ask `continue`s and
        // advances `turn`, so consecutive failures have consecutive indices).
        let contiguous = matches!((s.last_fail_turn, turn), (Some(l), Some(t)) if t == l + 1);
        if !contiguous {
            s.repairs = 0;
        }
        s.last_fail_turn = turn;
        if s.repairs < MAX_REPAIRS {
            s.repairs += 1;
            Repair::ReAsk(format!(
                "Your tool call could not be parsed: {err}. Re-emit it correctly."
            ))
        } else {
            Repair::GiveUp
        }
    }
}
```

- [ ] **Step 3b: Add `fire_on_parse_failure` in `loop_.rs`** (next to the other `fire_*` helpers):
```rust
    /// Reverse order: on a total tool-call parse failure. The FIRST middleware
    /// returning `ReAsk` wins and short-circuits; all `GiveUp` (or an empty
    /// stack) yields today's terminal give-up. `raw` is a borrow of the turn
    /// the loop still holds (consumed only on the branches after this returns).
    async fn fire_on_parse_failure(
        &self,
        ctx: &mut dyn ContextManager,
        state: &mut crate::RunState,
        shared: &crate::RunShared,
        cancel: &CancellationToken,
        turn: usize,
        raw: &agent_model::AssistantTurn,
        err: &str,
    ) -> crate::Repair {
        for mw in self.middleware.iter().rev() {
            let mut cx = crate::RunCx {
                ctx: &mut *ctx,
                sink: &self.sink,
                cancel,
                state: &mut *state,
                shared,
                turn: Some(turn),
                maint: self.maint_view(),
            };
            let r = mw.on_parse_failure(&mut cx, raw, err).await;
            Self::assert_no_orphans(ctx, mw.name());
            if let crate::Repair::ReAsk(m) = r {
                return crate::Repair::ReAsk(m);
            }
        }
        crate::Repair::GiveUp
    }
```

- [ ] **Step 3c: Refactor the parse-failure arms in `loop_.rs`.** Locate the block `let mut parsed = match self.protocol.parse(&assistant) { ... }`. Delete the `let mut protocol_repairs = 0;` local (above the turn loop) and the `Ok(p) => { protocol_repairs = 0; p }` reset (becomes `Ok(p) => p`). Keep the Length arm (`Err(_) if assistant.stop == StopReason::Length => { ... return Ok(()); }`) exactly as-is, **first**. Replace the two arms `Err(e) if protocol_repairs < 1 => {...}` and `Err(e) => {...}` with the single arm:
```rust
                Err(e) => {
                    // Total parse failure: consult the stack (spec §5.1). The
                    // Length short-circuit above already handled max_tokens
                    // truncation; the second Length guard below is downstream.
                    let err_str = e.to_string();
                    match self
                        .fire_on_parse_failure(
                            ctx, &mut mw_state, &run_shared, &cancel, turn, &assistant, &err_str,
                        )
                        .await
                    {
                        crate::Repair::ReAsk(msg) => {
                            ctx.append(Message::assistant(assistant.text.clone(), None));
                            ctx.append(Message::user(msg));
                            continue;
                        }
                        crate::Repair::GiveUp => {
                            self.sink.emit(AgentEvent::Error(err_str));
                            self.sink.emit(AgentEvent::Done(StopReason::Error));
                            return Ok(());
                        }
                    }
                }
```
The second Length guard (`if !parsed.invalid.is_empty() && assistant.stop == StopReason::Length { ... }`) below stays unchanged; `on_parse_failure` does not fire there.

- [ ] **Step 3d: Wire `RepairMiddleware` into both stacks.**
`assemble.rs`: after `stack.push(Arc::new(agent_core::StuckDetectionMiddleware));`, add:
```rust
    // Malformed-tool-call repair as a pluggable unit (spec §5.3). Reproduces
    // the loop-resident one-shot re-ask byte-identically; last in the stack.
    stack.push(Arc::new(agent_core::RepairMiddleware));
```
*(In Wave B the guardrails land between `StuckDetectionMiddleware` and `RepairMiddleware`; Repair stays last.)*
`dispatch.rs`: locate `.with_middleware(vec![curation, Arc::new(StuckDetectionMiddleware)])` and change it to:
```rust
            .with_middleware(vec![
                curation,
                Arc::new(StuckDetectionMiddleware),
                Arc::new(RepairMiddleware),
            ])
```
Add `RepairMiddleware` to the `use agent_core::{...}` import at the top of `dispatch.rs`.

- [ ] **Step 3e: Update the superseded repair pins (construction-site only, MANDATORY — a green run without these edits is a silent parity loss).** These existing loop tests exercised loop-resident repair; a bare loop no longer repairs, so `RepairMiddleware` must join the stack each drives, leaving assertion bodies unchanged. Locate each by content and note the **exact edit site** (some wire the stack through a shared helper, not inline):
  - `protocol_repair_exhausted_emits_done` (builds its loop inline) — add `RepairMiddleware` to its `.with_middleware(...)` (or add the call if absent). **Mandatory, not optional:** without the edit the test still emits `Done(Error)` (an empty stack gives up on the first failure), so it passes *for the wrong reason* — the two-turn re-ask→exhaust behavior is no longer pinned.
  - `repair_turn_leaves_stuck_counters_untouched` — this test builds its agent through the **shared `counter_agent(...)` helper**, which installs `.with_middleware(vec![Arc::new(StuckDetectionMiddleware)])` internally (the test body never calls `.with_middleware`). Add `Arc::new(RepairMiddleware)` to **`counter_agent`'s** stack vector — do NOT try to edit the test body. The helper has other callers; adding `RepairMiddleware` is **benign for them** (its only hook, `on_parse_failure`, is inert absent a parse failure).
  - The whole-turn-repair message test (asserts `"Re-emit it correctly"` appears; near the `Re-emit it correctly` grep hit) — add `RepairMiddleware` to whatever stack it drives (check inline-vs-helper before editing).
  - Leave `malformed_call_length_stop_takes_truncation_path` and the per-call `"re-emit only this call"` invalid-path test **unchanged**: the Length short-circuit fires before `on_parse_failure`, and partial-invalid never reaches it.

- [ ] **Step 3f: Add the new hook-firing pins** (loop_.rs tests), extending the Phase-1 firing-set:
```rust
#[tokio::test]
async fn on_parse_failure_fires_only_at_total_parse_failure() {
    // A recording middleware logs on_parse_failure; assert it fires on a
    // total-parse-failure turn and NOT on success / Length-truncation.
    // (Reuse the recording-middleware harness from the Phase-1 firing tests.)
    // - malformed turn (Err from parse, stop != Length): on_parse_failure fires.
    // - well-formed turn: on_parse_failure does NOT fire.
    // - Length-truncated turn: on_parse_failure does NOT fire (short-circuited).
    // ...
}

#[tokio::test]
async fn on_parse_failure_reverse_order_first_reask_wins() {
    // Stack [GiveUpMw, ReAskMw]; reverse order consults ReAskMw first? No:
    // reverse of [GiveUp, ReAsk] is [ReAsk, GiveUp] — ReAsk (stack-last) is
    // consulted first and wins. Assert the run re-asks (not terminal), and a
    // stack of [ReAsk] where the sole member GiveUps yields today's terminal.
    // ...
}

#[tokio::test]
async fn empty_text_reask_appends_balanced_message() {
    // On the common shape (unparseable tool-call blob, no prose), raw.text is
    // empty: the loop appends an empty tool-call-free assistant message + the
    // user re-ask — balanced, no orphan (spec §5.1 panel Fa-F4).
    // Assert build() has no orphaned tool_calls after the re-ask.
    // ...
}
```

- [ ] **Step 4: Run the whole agent-core suite.**
Run: `cargo test -p agent-core`
Expected: PASS — repair pins hold with `RepairMiddleware` in-stack; new hook-firing pins pass; `repair_turn_fires_no_after_model` still holds (the failed turn `continue`s before `after_model`).

- [ ] **Step 5: Commit.**
```bash
git add agent/crates/agent-core/src/middleware.rs agent/crates/agent-core/src/loop_.rs \
        agent/crates/agent-core/src/dispatch.rs agent/crates/agent-runtime-config/src/assemble.rs
git commit -m "feat(core): repair as on_parse_failure middleware, byte-identical (Phase 3A wave A)"
```

- [ ] **Step 6: Wave A gate.**
Run: `bash scripts/ci.sh`
Expected: green. Wave A is independently reviewable — pure seam + byte-identical repair, no new behavior.

---

# WAVE B — new behavior (guardrails + planning)

The genuinely-new capabilities: guardrail limits over `RunShared`, and `write_todos` planning-by-recitation as a durable pinned block (the one deliberate curation touch).

## Task B1: `ToolCallLimit` + `ModelCallLimit` guardrails

**Files:**
- Modify: `agent/crates/agent-core/src/middleware.rs` (`ToolCallLimit`, `ModelCallLimit`, `ToolCallCount`, `ModelCallCount`, `TOOL_CALL_LIMIT`, `MODEL_CALL_LIMIT`)
- Modify: `agent/crates/agent-core/src/loop_.rs` (the loop-top pre-turn `ToolCallLimit` guard)
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (push both guardrails after `StuckDetectionMiddleware`, before `RepairMiddleware`)
- Test: `loop_.rs` (runaway, overshoot bound, count-on-failure, nudge-drop ordering, ModelCallLimit off/on)

**Interfaces:**
- Consumes: `RunShared` (A1/A2).
- Produces: `pub struct ToolCallLimit` (`ToolCallLimit::new()` cap = `TOOL_CALL_LIMIT`; `ToolCallLimit::with_cap(usize)`); `pub struct ModelCallLimit` (`ModelCallLimit::disabled()`; `ModelCallLimit::enabled_with_cap(usize)`); `#[derive(Default)] pub struct ToolCallCount(pub usize)` (read by the loop's pre-turn guard); `pub const TOOL_CALL_LIMIT: usize = 1000`; `pub const MODEL_CALL_LIMIT: usize = 500`.

- [ ] **Step 1: Write the failing guardrail tests** (loop_.rs). Reuse a scripted-model harness where the model issues a tool call **with different args each turn** (defeats `StuckDetection`, whose signature is over `(name, args)`):

```rust
#[tokio::test]
async fn tool_call_limit_ends_run_on_varying_args_runaway() {
    // A model that issues one tool call per turn with DISTINCT args each turn —
    // the shape StuckDetection cannot catch. With ToolCallLimit::with_cap(5) and
    // a generous max_turns, the run ends with StopReason::Error (NOT
    // BudgetExhausted) once the count crosses the cap (spec §5.5, A10/Fa-F2).
    // Stack: [ToolCallLimit::with_cap(5)].
    // assert Done == Some(StopReason::Error)
    // assert the tool executed <= cap + one turn's batch (overshoot bound); here
    //   batch size is 1, so total executed is exactly the cap (5).
    // ...
}

#[tokio::test]
async fn tool_call_limit_overshoot_is_bounded_to_one_batch() {
    // Fat crossing batch: the model issues N calls in the crossing turn. Total
    // executed <= cap-1 (prior turns) + N (the crossing batch) — bounded to one
    // batch, not unbounded (spec §5.5 Fa-F3). Assert the precise count.
    // ...
}

#[tokio::test]
async fn tool_call_limit_counts_failed_calls() {
    // Count-on-failure is intentional (increment BEFORE next.run): a panicking /
    // denied / errored call still counts (spec §5.5 Fa-F3). Drive a tool that
    // errors and assert the count still advances toward the cap.
    // ...
}

#[tokio::test]
async fn guardrail_endrun_at_after_tools_drops_pending_nudge_without_orphans() {
    // Stack order [StuckDetection, ToolCallLimit]: after_tools fires in reverse,
    // so ToolCallLimit's after_tools EndRun short-circuits BEFORE StuckDetection
    // flushes a nudge it set that turn. Assert the CONTENT outcome: nudge
    // dropped, transcript balanced, no orphaned tool_calls (spec §5.6 Fa-F5).
    // (Mirror the Phase-1 end_run_from_after_tools_leaves_no_orphans harness,
    //  with a low cap + a stuck-repeat to co-fire both.)
    // ...
}

#[tokio::test]
async fn model_call_limit_default_off_is_inert_but_enabled_ends_run() {
    // Default (disabled): counts in wrap_model_call but never enforces — a run
    // well within max_turns is unaffected.
    // Enabled (ModelCallLimit::enabled_with_cap(2)): after 2 model calls,
    // after_model EndRuns with StopReason::Error.
    // ...
}
```

- [ ] **Step 2: Run to verify they fail.**
Run: `cargo test -p agent-core --lib tool_call_limit model_call_limit guardrail_endrun`
Expected: FAIL — guardrail types not defined.

- [ ] **Step 3a: Implement the guardrails in `middleware.rs`.**
```rust
/// Generous order-of-magnitude backstops (spec §5.5). Named so a future knob is
/// a one-line change; NOT runtime-configurable (matches the stuck precedent).
pub const TOOL_CALL_LIMIT: usize = 1000;
pub const MODEL_CALL_LIMIT: usize = 500;

/// Tool-call tally in `RunShared`. `pub` so the loop's pre-turn guard reads it
/// (spec §5.7); stays 0 unless a counting middleware is in the stack.
#[derive(Default)]
pub struct ToolCallCount(pub usize);

#[derive(Default)]
struct ModelCallCount(usize);

/// Always-on runaway backstop (spec §5.5): a runaway that VARIES its args each
/// turn slips past `StuckDetection` and can burn the whole turn budget. Counts
/// every tool execution in `wrap_tool_call`, enforces at `after_tools`.
pub struct ToolCallLimit {
    cap: usize,
}

impl ToolCallLimit {
    pub fn new() -> Self {
        Self { cap: TOOL_CALL_LIMIT }
    }
    pub fn with_cap(cap: usize) -> Self {
        Self { cap }
    }
}

impl Default for ToolCallLimit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for ToolCallLimit {
    fn name(&self) -> &str {
        "tool-call-limit"
    }
    async fn wrap_tool_call(&self, call: ToolCall, next: ToolNext<'_>) -> crate::Executed {
        // Count BEFORE execution: a panicked/timed-out/denied call still costs an
        // orchestration round-trip, so count-on-failure is intentional (Fa-F3).
        next.shared().with::<ToolCallCount, _>(|c| c.0 += 1);
        next.run(call.args).await
    }
    async fn after_tools(&self, cx: &mut RunCx<'_>) -> Flow {
        if cx.shared().with::<ToolCallCount, _>(|c| c.0) >= self.cap {
            cx.sink.emit(crate::AgentEvent::Error(
                "tool-call guardrail: the run exceeded the maximum number of tool \
                 calls; aborting"
                    .into(),
            ));
            return Flow::EndRun(StopReason::Error);
        }
        Flow::Continue
    }
}

/// Model-call cap (spec §5.5, E1). Ships DEFAULT-OFF: always counts in
/// `wrap_model_call` (so `wrap_model_call` has a real consumer and both wrap
/// surfaces are exercised), enforces at `after_model` only when enabled. It is
/// subsumed by `max_turns`, hence opt-in, not imposed.
pub struct ModelCallLimit {
    cap: usize,
    enabled: bool,
}

impl ModelCallLimit {
    pub fn disabled() -> Self {
        Self { cap: MODEL_CALL_LIMIT, enabled: false }
    }
    pub fn enabled_with_cap(cap: usize) -> Self {
        Self { cap, enabled: true }
    }
}

#[async_trait]
impl Middleware for ModelCallLimit {
    fn name(&self) -> &str {
        "model-call-limit"
    }
    async fn wrap_model_call(
        &self,
        req: agent_model::CompletionRequest,
        next: ModelNext<'_>,
    ) -> Result<agent_model::AssistantTurn, crate::CompletionFailure> {
        next.shared().with::<ModelCallCount, _>(|c| c.0 += 1);
        next.run(req).await
    }
    async fn after_model(&self, cx: &mut RunCx<'_>, _turn: &TurnView) -> Flow {
        if self.enabled && cx.shared().with::<ModelCallCount, _>(|c| c.0) >= self.cap {
            cx.sink.emit(crate::AgentEvent::Error(
                "model-call guardrail: the run exceeded the maximum number of model \
                 calls; aborting"
                    .into(),
            ));
            return Flow::EndRun(StopReason::Error);
        }
        Flow::Continue
    }
}
```

- [ ] **Step 3b: Add the loop-top pre-turn guard in `loop_.rs`** (spec §5.7; backstop per S3). Locate the top of the turn loop — the cancel check `if cancel.is_cancelled() { self.sink.emit(AgentEvent::Done(StopReason::Cancelled)); return Ok(()); }` inside `for turn in 0..self.config.max_turns`. Immediately after it, add:
```rust
            // Pre-turn ToolCallLimit backstop (spec §5.7): bounds overshoot to at
            // most one turn's batch. Reads the RunShared tally, which stays 0
            // unless a counting guardrail is in the stack (so this is inert for
            // children and any stack without ToolCallLimit). `after_tools` is the
            // effective bound; this is belt-and-suspenders (plan discrepancy S3).
            if run_shared.with::<crate::ToolCallCount, _>(|c| c.0) >= crate::TOOL_CALL_LIMIT {
                self.sink.emit(AgentEvent::Error(
                    "tool-call guardrail: the run exceeded the maximum number of tool \
                     calls; aborting"
                        .into(),
                ));
                self.sink.emit(AgentEvent::Done(StopReason::Error));
                return Ok(());
            }
```

- [ ] **Step 3c: Wire the guardrails into the parent stack (`assemble.rs`).** Between `stack.push(Arc::new(agent_core::StuckDetectionMiddleware));` and `stack.push(Arc::new(agent_core::RepairMiddleware));`, insert:
```rust
    // Guardrail siblings of StuckDetection (spec §5.5/§5.6). ModelCallLimit is
    // default-off (a wrap_model_call consumer; opt-in). ToolCallLimit is
    // always-on: the varying-args runaway backstop. Both after StuckDetection so
    // a co-firing guardrail EndRun resolves before stuck's nudge (Fa-F5).
    stack.push(Arc::new(agent_core::ModelCallLimit::disabled()));
    stack.push(Arc::new(agent_core::ToolCallLimit::new()));
```
Guardrails are **parent-only** (E4/J9): do not add them to the child stack in `dispatch.rs`.

- [ ] **Step 4: Run the guardrail tests + the full suite.**
Run: `cargo test -p agent-core`
Expected: PASS. No existing bounded test trips the generous production caps (`TOOL_CALL_LIMIT = 1000`), so no parity break. `StuckDetection` pins unchanged.

- [ ] **Step 5: Commit.**
```bash
git add agent/crates/agent-core/src/middleware.rs agent/crates/agent-core/src/loop_.rs \
        agent/crates/agent-runtime-config/src/assemble.rs
git commit -m "feat(core): ToolCallLimit/ModelCallLimit guardrails over RunShared (Phase 3A wave B)"
```

## Task B2: `write_todos` tool + `TodoListMiddleware` (foundation)

**Files:**
- Create: `agent/crates/agent-core/src/todos.rs`
- Modify: `agent/crates/agent-core/src/lib.rs` (`pub mod todos; pub use todos::*;`)
- Modify: `agent/crates/agent-core/src/middleware.rs` (`TodoListMiddleware`)
- Test: `agent/crates/agent-core/src/todos.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `pub struct TodoItem { pub content: String, pub status: TodoStatus }`; `pub enum TodoStatus { Pending, InProgress, Completed }` (serde `snake_case`); `pub type TodoHandle = Arc<Mutex<Vec<TodoItem>>>` (or the bare `Arc<Mutex<Vec<TodoItem>>>` spelled out); `pub struct WriteTodosTool` (`WriteTodosTool::new(handle)`); `pub fn render_todos_block(items: &[TodoItem]) -> Option<Message>`; `pub struct TodoListMiddleware` (`TodoListMiddleware::new(handle)`).

- [ ] **Step 1: Write the failing tests** (`todos.rs`):
```rust
#[tokio::test]
async fn write_todos_sets_the_handle_and_returns_a_compact_confirmation() {
    let handle: Arc<Mutex<Vec<TodoItem>>> = Arc::new(Mutex::new(Vec::new()));
    let tool = WriteTodosTool::new(handle.clone());
    let out = tool
        .execute(
            json!({"todos": [
                {"content": "parse", "status": "in_progress"},
                {"content": "wire", "status": "pending"}
            ]}),
            &tool_ctx(),
        )
        .await
        .unwrap();
    // The list is now in the handle...
    let items = handle.lock().unwrap().clone();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].status, TodoStatus::InProgress);
    // ...and the tool result is a COMPACT confirmation, not the full list, so
    // its own tool-result message is offload-irrelevant (spec §5.4).
    assert!(out.content.len() < 80, "compact confirmation: {}", out.content);
    assert!(!out.content.contains("parse"), "must not echo the list back");
}

#[test]
fn render_todos_block_is_none_when_empty_and_lists_statuses_when_set() {
    assert!(render_todos_block(&[]).is_none());
    let block = render_todos_block(&[
        TodoItem { content: "parse".into(), status: TodoStatus::InProgress },
        TodoItem { content: "wire".into(), status: TodoStatus::Pending },
    ])
    .expect("non-empty renders a block");
    assert!(matches!(block.role, agent_model::Role::System));
    assert!(block.content.contains("in_progress"));
    assert!(block.content.contains("parse"));
}

#[test]
fn write_todos_description_permits_multiple_in_progress() {
    // Panel B1: the real LangChain contract allows multiple independent
    // in_progress tasks; the earlier draft's "exactly one" inverted it, and this
    // ships verbatim to the model. Snapshot-guard the wording.
    let tool = WriteTodosTool::new(Arc::new(Mutex::new(Vec::new())));
    let d = tool.description().to_lowercase();
    assert!(d.contains("multiple") && d.contains("in_progress") && d.contains("parallel"));
    assert!(d.contains("in_progress") && d.contains("completed"));
}
```

- [ ] **Step 2: Run to verify they fail.**
Run: `cargo test -p agent-core --lib todos::tests`
Expected: FAIL — `todos` module not defined.

- [ ] **Step 3a: Create `todos.rs`.**
```rust
//! Planning-by-recitation (spec §5.4). `write_todos` rewrites a shared list the
//! curator renders as a durable PINNED block (E3 pin/recall) — the tool itself
//! performs no computation; its value is keeping the plan in the attention
//! window over long tasks. The list is never merged back from subagents.
use agent_model::Message;
use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

impl TodoStatus {
    fn label(self) -> &'static str {
        match self {
            TodoStatus::Pending => "pending",
            TodoStatus::InProgress => "in_progress",
            TodoStatus::Completed => "completed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
}

/// The plan list shared between `WriteTodosTool` (writer) and `CuratedContext`
/// (renderer) — the `compact_flag` shape (spec §5.4/§5.6).
pub type TodoHandle = Arc<Mutex<Vec<TodoItem>>>;

/// The non-empty list as the pinned todos block, or `None` when empty (spec
/// §5.4). Rendered as the LAST pinned block by `CuratedContext::pinned()`.
pub fn render_todos_block(items: &[TodoItem]) -> Option<Message> {
    if items.is_empty() {
        return None;
    }
    let lines = items
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{}. [{}] {}", i + 1, t.status.label(), t.content))
        .collect::<Vec<_>>()
        .join("\n");
    Some(Message::system(format!(
        "Current task plan (from write_todos) — keep working the in_progress \
         items until the plan is complete:\n{lines}"
    )))
}

/// Rewrites the whole plan list into the shared handle. Returns a COMPACT
/// confirmation (not the list) so its own tool-result message is offload-
/// irrelevant; the authoritative recitation is the pinned block (spec §5.4).
pub struct WriteTodosTool {
    handle: TodoHandle,
}

impl WriteTodosTool {
    pub fn new(handle: TodoHandle) -> Self {
        Self { handle }
    }
}

#[derive(Deserialize)]
struct WriteTodosArgs {
    todos: Vec<TodoItem>,
}

#[async_trait]
impl Tool for WriteTodosTool {
    fn name(&self) -> &str {
        "write_todos"
    }
    fn description(&self) -> &str {
        "Record or update your task plan for a complex, multi-step objective \
         (3+ distinct steps or non-trivial planning). Do NOT use it for single, \
         straightforward, or conversational turns — for a simple objective, just \
         do the work directly. Each call REPLACES the whole list. Keep at least \
         one task in_progress while work remains; multiple tasks may be \
         in_progress at once when they are independent and can proceed in \
         parallel. Mark a task completed immediately when it is done — do not \
         batch completions. The plan stays visible in your context so you stay \
         on track over long tasks."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "write_todos".into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "description": "The full task list; replaces any prior list.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": {"type": "string", "description": "The task."},
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"],
                                    "description": "Task status."
                                }
                            },
                            "required": ["content", "status"]
                        }
                    }
                },
                "required": ["todos"]
            }),
        }
    }
    fn intent(&self, _args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "write_todos".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "update the task plan".into(),
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &ToolCtx,
    ) -> Result<ToolOutput, ToolError> {
        let parsed: WriteTodosArgs = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgs(format!("write_todos: {e}")))?;
        let n = parsed.todos.len();
        *self.handle.lock().unwrap() = parsed.todos;
        Ok(ToolOutput {
            content: format!("Plan updated ({n} task(s))."),
            display: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn tool_ctx() -> ToolCtx {
        ToolCtx {
            workspace: std::env::temp_dir(),
            timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(),
            sandbox: Arc::new(agent_tools::HostExecutor),
            backend: Arc::new(agent_tools::backend::HostBackend::new(std::env::temp_dir())),
            call_id: "test".into(),
        }
    }
    // ... the three tests from Step 1 ...
}
```
*(Implementer: verify `ToolError::InvalidArgs` exists; if the variant differs, use the crate's actual "bad arguments" `ToolError` variant — locate by content in `agent-tools`.)*

- [ ] **Step 3b: Add `TodoListMiddleware` in `middleware.rs`.**
```rust
/// Planning-by-recitation (spec §5.4): a hookless tool-contributing middleware
/// (the `ContextCurationMiddleware`+flag shape). Holds the shared todos handle,
/// contributes a CHILD-VISIBLE `write_todos`; the curator renders the list as a
/// durable pinned block. No node/wrap hooks.
pub struct TodoListMiddleware {
    handle: crate::TodoHandle,
}

impl TodoListMiddleware {
    pub fn new(handle: crate::TodoHandle) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl Middleware for TodoListMiddleware {
    fn name(&self) -> &str {
        "todo-list"
    }
    fn tools(&self) -> Vec<ToolContribution> {
        vec![ToolContribution {
            tool: Arc::new(crate::WriteTodosTool::new(self.handle.clone())),
            child_visible: true,
        }]
    }
}
```

- [ ] **Step 3c: Export from `lib.rs`.** Add `mod todos;` (or `pub mod todos;`) and `pub use todos::*;` alongside the existing `pub use context_tools::*;` etc.

- [ ] **Step 4: Run.**
Run: `cargo test -p agent-core --lib todos`
Expected: PASS.

- [ ] **Step 5: Commit.**
```bash
git add agent/crates/agent-core/src/todos.rs agent/crates/agent-core/src/lib.rs \
        agent/crates/agent-core/src/middleware.rs
git commit -m "feat(core): write_todos tool + TodoListMiddleware foundation (Phase 3A wave B)"
```

## Task B3: `CuratedContext` renders the todos block (the E3 curation touch)

**Files:**
- Modify: `agent/crates/agent-core/src/curated.rs` (`todos` field; `with_todos` builder; `pinned()` render; `pinned_tokens()` lockstep; `snapshot()` passes the todos list)
- Modify: `agent/crates/agent-core/src/snapshot.rs` (`build_snapshot` gains a `todos: &[TodoItem]` param + a `todos` segment mirroring `ledger`, preserving the audit-7.3 `est_total` invariant — S5)
- Test: `agent/crates/agent-core/src/curated.rs` + `snapshot.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `TodoHandle`, `TodoItem`, `render_todos_block` (B2).
- Produces: `CuratedContext::with_todos(handle) -> Self` (builder; default is an empty handle → no block). `pinned()` appends the todos block **last**; `pinned_tokens()` accounts for it; `build_snapshot` gains a `todos` segment so `est_total == pinned_tokens() + history_tokens` holds with todos non-empty.

- [ ] **Step 1: Write the failing tests** (`curated.rs` tests):
```rust
#[test]
fn todos_render_as_the_last_pinned_block_after_existing_order() {
    // The existing pinned order (system → goal/ledger → recall → summary+pointer)
    // stays byte-identical; todos is APPENDED after it (spec §5.4/E3).
    let handle: Arc<std::sync::Mutex<Vec<crate::TodoItem>>> =
        Arc::new(std::sync::Mutex::new(vec![crate::TodoItem {
            content: "do the thing".into(),
            status: crate::TodoStatus::InProgress,
        }]));
    let mut with = ctx().with_todos(handle.clone());
    with.set_goal("ship it".into());
    with.set_recall(vec!["a memory".into()]);
    with.compaction_summary = Some(Message::system("Summary: earlier"));
    with.append(Message::user("hello"));
    let built = with.build(100_000);

    // Same prefix as without todos:
    let mut without = ctx();
    without.set_goal("ship it".into());
    without.set_recall(vec!["a memory".into()]);
    without.compaction_summary = Some(Message::system("Summary: earlier"));
    without.append(Message::user("hello"));
    let base = without.build(100_000);

    // The todos block is the LAST pinned message (before windowed history), and
    // the pinned prefix up to it matches `base`'s pinned prefix byte-for-byte.
    let todo_msg = built
        .iter()
        .find(|m| m.content.starts_with("Current task plan (from write_todos)"))
        .expect("todos block present");
    assert!(todo_msg.content.contains("[in_progress] do the thing"));
    // Existing pinned order preserved: system/goal/recall/summary identical.
    assert_eq!(built[0].content, base[0].content); // system
    assert_eq!(built[1].content, base[1].content); // goal
    assert!(built[2].content.starts_with("Relevant memories")); // recall
    // history tail unchanged
    assert_eq!(built.last().unwrap().content, "hello");
}

#[test]
fn empty_todos_render_nothing() {
    let mut c = ctx().with_todos(Arc::new(std::sync::Mutex::new(Vec::new())));
    c.append(Message::user("hi"));
    assert!(!c
        .build(100_000)
        .iter()
        .any(|m| m.content.starts_with("Current task plan")));
}

#[test]
fn pinned_tokens_accounts_for_the_todos_block_in_lockstep() {
    let items = vec![crate::TodoItem {
        content: "a longish task description to make the block non-trivial".into(),
        status: crate::TodoStatus::Pending,
    }];
    let handle = Arc::new(std::sync::Mutex::new(items.clone()));
    let empty = ctx();
    let with = ctx().with_todos(handle);
    let block = crate::render_todos_block(&items).unwrap();
    assert_eq!(
        with.pinned_tokens_for_test(),
        empty.pinned_tokens_for_test() + message_tokens(&block),
        "pinned_tokens() extends by exactly the todos block (lockstep with pinned())"
    );
}

#[tokio::test]
async fn todos_block_survives_offload_and_compaction() {
    // Pinned → unconditionally in-window until overwritten (the hard guarantee
    // exclude_tools could not give — spec §7 durability pin).
    let handle = Arc::new(std::sync::Mutex::new(vec![crate::TodoItem {
        content: "durable plan".into(),
        status: crate::TodoStatus::InProgress,
    }]));
    let mut c = ctx().with_todos(handle);
    c.high_water_pct = 0.0; // force compaction
    c.config.keep_recent = 1;
    for i in 0..6 {
        c.append(Message::assistant(
            format!("turn {i} {}", "padding ".repeat(12)),
            None,
        ));
    }
    let model: Arc<dyn ModelClient> =
        Arc::new(ScriptedModel::new(vec![Scripted::Text("summary".into())]));
    let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
    let cancel = CancellationToken::new();
    c.maintain(&maint_deps(&model, &sink, &cancel)).await;
    assert!(
        c.build(100_000)
            .iter()
            .any(|m| m.content.contains("durable plan")),
        "the todos block is pinned and survives compaction"
    );
}

#[test]
fn snapshot_est_total_stays_consistent_with_pinned_todos() {
    // S5: a non-empty todos plan adds a `todos` segment so est_total keeps
    // matching the budget math (audit 7.3), exactly like the ledger segment.
    let items = vec![crate::TodoItem {
        content: "plan the work in several steps".into(),
        status: crate::TodoStatus::InProgress,
    }];
    let mut c = ctx().with_todos(Arc::new(std::sync::Mutex::new(items)));
    c.append(Message::user("do the thing"));
    c.append(Message::assistant("on it", None));
    let snap = c.snapshot(100_000, 1);
    let history_tokens: usize = c.history().iter().map(message_tokens).sum();
    assert_eq!(snap.est_total, c.pinned_tokens_for_test() + history_tokens);
    assert!(
        snap.segments.iter().any(|s| s.category == "todos"),
        "a non-empty plan surfaces a todos segment"
    );
}
```
*(Implementer: `pinned()`/`pinned_tokens()` are private. Expose a `#[cfg(test)] pub(crate) fn pinned_tokens_for_test(&self) -> usize { self.pinned_tokens() }` — or make the token assertion via `snapshot`/`build` if a test accessor is undesirable. The existing test `snapshot_est_total_includes_the_pinned_ledger` uses `pinned_tokens()` indirectly; keep its assertion body unchanged — it uses folded_facts, not todos, per deviation S5.)*

- [ ] **Step 2: Run to verify they fail.**
Run: `cargo test -p agent-core --lib curated::tests::todos`
Expected: FAIL — `with_todos` not defined.

- [ ] **Step 3: Implement in `curated.rs`.**
Add the field to `struct CuratedContext` (near `compact_flag`):
```rust
    /// Shared plan list (spec §5.4). Rendered as the LAST pinned block; default
    /// is an empty handle (no block) until a caller wires `with_todos`.
    todos: crate::TodoHandle,
```
In `CuratedContext::new`, initialize `todos: Arc::new(std::sync::Mutex::new(Vec::new())),`.
Add the builder (near `with_artifact_prefix`):
```rust
    /// Wire the shared todos handle (the same one `TodoListMiddleware`'s
    /// `write_todos` tool sets), so the current plan renders as a pinned block.
    pub fn with_todos(mut self, todos: crate::TodoHandle) -> Self {
        self.todos = todos;
        self
    }
```
In `pinned()`, after the `compaction_summary` block push (the last existing push, before `out`), add:
```rust
        // The todos plan is the LAST pinned block — after goal/ledger → recall →
        // summary+pointer (existing order byte-identical), nearest the windowed
        // conversation (spec §5.4/E3). Empty list → nothing.
        if let Some(t) = crate::render_todos_block(&self.todos.lock().unwrap()) {
            out.push(t);
        }
```
In `pinned_tokens()`, after the `compaction_summary` accounting (before `t`), add — **in lockstep with `pinned()`**:
```rust
        {
            let todos = self.todos.lock().unwrap();
            if let Some(block) = crate::render_todos_block(&todos) {
                t += message_tokens(&block);
            }
        }
```
Thread todos into `snapshot()` (spec §5.4 accounting / S5). In `CuratedContext::snapshot(...)`, before the `build_snapshot(...)` call, read the list and pass it — locate the existing `ledger` local + `build_snapshot(...)` call and add a `todos` argument:
```rust
        let todos = self.todos.lock().unwrap().clone();
        // ... existing let ledger = ... ;
        crate::snapshot::build_snapshot(
            // ... existing args ...,
            &self.history,
            &todos, // NEW trailing arg (S5)
        )
```
In `snapshot.rs`, add `todos: &[crate::TodoItem]` as the final `build_snapshot` parameter and push a `todos` segment **after** the `summary` segment and **before** the `messages` segment (mirroring `pinned()`'s last-pinned position and the existing `ledger` segment), guarded on non-empty so the invariant `est_total == pinned_tokens() + history_tokens` holds:
```rust
    // Pinned todos plan (spec §5.4 / S5): its own segment keeps est_total equal
    // to the budget math, exactly like the ledger segment (audit 7.3).
    if let Some(block) = crate::render_todos_block(todos) {
        segments.push(ContextSegment {
            category: "todos".into(),
            est_tokens: message_tokens(&block),
            items: todos.iter().map(|t| preview(&t.content, 100)).collect(),
            count: todos.len(),
        });
    }
```
Update `snapshot.rs`'s existing `build_snapshot` test call(s) (e.g. `snapshot_has_system_and_messages_and_sums_total`) to pass a trailing `&[]` — assertion bodies unchanged (empty todos → no `todos` segment, so `cats == ["system", "messages"]` still holds).

- [ ] **Step 4: Run.**
Run: `cargo test -p agent-core --lib curated snapshot`
Expected: PASS. All existing curated pins (ledger order, compaction, fold) hold — todos is appended, never interleaved. `snapshot_est_total_includes_the_pinned_ledger` unchanged (todos empty there → no divergence); the new S5 segment keeps `est_total` consistent when todos is non-empty.

- [ ] **Step 5: Commit.**
```bash
git add agent/crates/agent-core/src/curated.rs
git commit -m "feat(core): render write_todos plan as durable pinned block (Phase 3A wave B, E3)"
```

## Task B4: Wire `TodoListMiddleware` into stacks + frontends + children

Integration: `LoopParts.todos`, the parent stack push, the parent `CuratedContext.with_todos` in both frontends, and per-child todos isolation.

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (`LoopParts.todos` field; push `TodoListMiddleware` first; update the test `parts()` helper)
- Modify: `agent/crates/agent-cli/src/main.rs` (create the todos handle; `LoopParts.todos`; `.with_todos(...)`)
- Modify: `agent/crates/agent-server/src/runtime.rs` (`RuntimeState.todos` + `todos()` accessor; `LoopParts.todos`) and `session.rs` (`.with_todos(...)` at both `CuratedContext::new` sites)
- Modify: `agent/crates/agent-core/src/dispatch.rs` (per-child todos handle; rebind child `write_todos`; `child_ctx.with_todos`)
- Modify: test `LoopParts { ... }` constructors that break on the new field
- Test: `dispatch.rs` (child todos isolation) + `assemble.rs` (write_todos registered, child-visible)

**Interfaces:**
- Consumes: `TodoListMiddleware`, `WriteTodosTool`, `TodoHandle`, `CuratedContext::with_todos`.
- Produces: `LoopParts.todos: TodoHandle`.

- [ ] **Step 1: Write the failing integration tests.**
`assemble.rs` tests:
```rust
#[test]
fn registers_write_todos_child_visible() {
    let dir = tempfile::tempdir().unwrap();
    let built = assemble_loop(&cfg(), parts(dir.path().to_path_buf(), vec![]));
    assert!(built.registered_names.iter().any(|n| n == "write_todos"));
    // child-visible: it is in the child base snapshot (registered before it).
    let base = built.dispatch_base_names.expect("subagents on by default");
    assert!(base.iter().any(|n| n == "write_todos"), "{base:?}");
}
```
`dispatch.rs` tests (child isolation): drive a child that calls `write_todos`, and assert the child's plan renders in the child's context but the **parent** todos handle stays empty (never merged). Mirror the existing child-stack harness (`child_stack_is_exactly_curation_and_stuck_detection_never_memory_recall`), plus a child-repair parity probe:
```rust
#[tokio::test]
async fn child_write_todos_is_isolated_from_the_parent() {
    // A child that calls write_todos updates ITS OWN handle/pinned block; the
    // parent's todos handle is never touched (deepagents contract, spec §5.6).
    // Build DispatchDeps whose base_tools include a parent-bound write_todos;
    // dispatch must rebind it to the child's handle. Assert the parent handle
    // stays empty after the child plans.
    // ...
}

#[tokio::test]
async fn dispatched_child_repairs_a_malformed_turn_once() {
    // Child stack is [curation, stuck, repair]: a malformed child turn re-asks
    // exactly once with the byte-identical message, then resolves (spec §5.6).
    // ...
}
```

- [ ] **Step 2: Run to verify they fail.**
Run: `cargo test -p agent-runtime-config --lib registers_write_todos; cargo test -p agent-core --lib child_write_todos dispatched_child_repairs`
Expected: FAIL (`LoopParts.todos` missing; child rebind absent).

- [ ] **Step 3a: Add `LoopParts.todos` (`assemble.rs`).** In `pub struct LoopParts`, after `compact_flag`, add:
```rust
    /// Shared plan list the `write_todos` tool sets; the caller's `CuratedContext`
    /// reads it (the `compact_flag` shape, spec §5.4/§5.6).
    pub todos: agent_core::TodoHandle,
```
Push `TodoListMiddleware` **first** in the stack (before the `if cfg.memory` block):
```rust
    // Planning-by-recitation (spec §5.4/§5.6): first in the stack (convention;
    // hookless, so position affects only tool-registration precedence — child-
    // visible `write_todos` collides with nothing). Default-on (plan S4).
    stack.push(Arc::new(agent_core::TodoListMiddleware::new(parts.todos.clone())));
```
Update the test `parts()` helper: add `todos: Arc::new(std::sync::Mutex::new(Vec::new())),` to its `LoopParts { ... }` literal.

- [ ] **Step 3b: CLI (`agent-cli/src/main.rs`).** Where `compact_flag` is created (`let compact_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));`), add:
```rust
    let todos: agent_core::TodoHandle = Arc::new(std::sync::Mutex::new(Vec::new()));
```
In the `LoopParts { ... }` literal (near `compact_flag: compact_flag.clone(),`), add `todos: todos.clone(),`. On the `CuratedContext::new(...)` builder chain (the `.with_recall_budget(...).with_offload_config(...)` chain), add `.with_todos(todos.clone())`.

- [ ] **Step 3c: Server (`agent-server/src/runtime.rs` + `session.rs`).**
`runtime.rs`: add a field `todos: agent_core::TodoHandle,` to `RuntimeState` (next to `compact_flag`), initialize it in the constructor (`let todos = Arc::new(std::sync::Mutex::new(Vec::new()));` where `compact_flag` is created; store `todos` in the struct), add an accessor mirroring `compact_flag()`:
```rust
    pub fn todos(&self) -> agent_core::TodoHandle {
        self.todos.clone()
    }
```
and in the `LoopParts { ... }` construction (where `compact_flag: compact_flag.clone(),` is), add `todos: todos.clone(),` (referring to the same session-stable handle — thread it like `compact_flag` through the `build_loop_parts`-style helper that takes `compact_flag: &Arc<AtomicBool>`; add a `todos: &agent_core::TodoHandle` parameter and pass `&self.todos`).
`session.rs`: at BOTH `CuratedContext::new(...)` sites (initial build ~line 69 and `set_workspace` ~line 280), add `.with_todos(runtime.todos())` / `.with_todos(self.runtime.todos())` respectively to the builder chain (the session-stable handle survives loop rebuilds, matching `artifacts()`/`compact_flag()`).

- [ ] **Step 3d: Per-child todos isolation (`dispatch.rs`).** In `DispatchAgentTool::execute`, after `let flag = Arc::new(AtomicBool::new(false));` (the per-child compact flag), add a per-child todos handle:
```rust
        let todos: crate::TodoHandle = Arc::new(std::sync::Mutex::new(Vec::new()));
        // If this child is permitted write_todos (present in its filtered base
        // snapshot), rebind it to the child's OWN handle (last-wins over the
        // parent-bound instance) so the child plans for itself and its plan is
        // never merged to the parent (spec §5.6).
        if filtered_base.iter().any(|t| t.name() == "write_todos") {
            reg.register(Arc::new(crate::WriteTodosTool::new(todos.clone())));
        }
```
On the child `CuratedContext::new(...).with_offload_config(...).with_artifact_prefix(...)` chain, add `.with_todos(todos.clone())`. (Guardrails and `TodoListMiddleware` stay OUT of the child stack — only the *tool* is rebound; the child stack remains `[curation, stuck, repair]` from Task A3.)
Add `TodoHandle`/`WriteTodosTool` to the `use agent_core::{...}` import if not already re-exported through `crate::`.

- [ ] **Step 3e: Update the remaining `LoopParts { ... }` constructors.** Add `todos: Arc::new(std::sync::Mutex::new(Vec::new())),` to each test/frontend literal that now fails to compile: `agent-server/src/runtime.rs:370`, `agent-cli/src/main.rs:273` (if not covered by 3b), and the test files `eval_context.rs:279`, `e2e_robustness.rs:74`, `soak_live.rs:233`, `e2e_auto_retrieval.rs:115/181/244`. Locate each by content (`LoopParts {`).

- [ ] **Step 4: Run the full workspace suite.**
Run: `cargo test -p agent-core && cargo test -p agent-runtime-config`
Expected: PASS. `write_todos` registers child-visible; child plans are isolated; child repair holds; the child-stack pin (now `[curation, stuck, repair]`) passes.

- [ ] **Step 5: Commit.**
```bash
git add agent/crates/agent-runtime-config/src/assemble.rs agent/crates/agent-cli/src/main.rs \
        agent/crates/agent-server/src/runtime.rs agent/crates/agent-server/src/session.rs \
        agent/crates/agent-core/src/dispatch.rs \
        agent/crates/agent-runtime-config/tests agent/crates/agent-core/tests
git commit -m "feat: wire write_todos into stacks, frontends, and isolated children (Phase 3A wave B)"
```

- [ ] **Step 6: Wave B gate + full CI.**
Run: `bash scripts/ci.sh`
Expected: green (agent tests + conditional src-tauri + web typecheck/vitest). No wire/frontend change means web is unaffected.

---

## Testing summary (maps to spec §7)

**Parity (assertion bodies unchanged):** the whole Phase-1 stack-mechanics + cadence suite; Phase-2 backend/curation/child suites; `StuckDetection` pins; the Phase-1 wrap-composition pins (the added `shared` field does not perturb them); calibration pins; all `curated.rs` ledger/fold/compaction pins (todos is appended, not interleaved).

**Superseded pins (four, §3.1):**
- Hook-firing-set — extended: `on_parse_failure` fires only at total-parse-failure (Task A3 Step 3f); the existing five hooks' firing points unchanged.
- Wrap-contract — re-pinned on the `shared`-carrying `ModelNext`/`ToolNext` (Task A2 Step 4); composition semantics identical.
- Child-stack — `[curation, stuck, repair]` (Task A3 Step 3d; child-repair parity in Task B4).
- Loop-resident-repair — moves to `RepairMiddleware` with the same appended-message assertions (Task A3).

**New tests:** RunShared roundtrip/isolation/concurrency/poison (A1); write-in-wrap/read-in-node (A2); on_parse_failure firing + reverse-order fold + empty-text balanced re-ask + RepairMiddleware default + non-contiguous reset (A3); ToolCallLimit varying-args runaway + overshoot bound + count-on-failure + guardrail/stuck nudge-drop + ModelCallLimit off/on (B1); write_todos handle/compact-confirmation/description-wording + render (B2); todos last-pinned-block + existing-order-preserved + pinned_tokens lockstep + durability-through-compaction (B3); write_todos child-visible + child isolation + child repair (B4).

**Poison (S1):** asserted only at the achievable level (RunShared `into_inner` recovery, no cascade); the spec's "contained as `Executed::Panicked`" wording is flagged for correction.

**Gate:** `bash scripts/ci.sh` green.

---

## Self-review (against the spec, fresh eyes)

**Spec coverage.** G1 write_todos durable pinned block → B2+B3+B4. G2 RepairMiddleware behind on_parse_failure, byte-identical → A3. G3 ToolCallLimit(+ModelCallLimit) count-via-wrap/enforce-via-node → B1. G4 non-mutating wrap contract via RunShared → A1+A2. G5 suite green + new tests → all tasks + CI gate. §5.1 hook + fold semantics → A3. §5.2 RunShared + poison → A1. §5.3 RepairMiddleware → A3. §5.4 TodoList pin/recall → B2/B3/B4. §5.5 guardrails + Error-not-BudgetExhausted + overshoot → B1. §5.6 stack order + child stack + child todos isolation + guardrails parent-only + nudge-drop → A3/B1/B4. §5.7 loop after-picture (on_parse_failure call-out, pre-turn guard, RunShared construction) → A2/A3/B1.

**Gaps found and dispositioned:** S1–S5 above (each flagged for the plan-review gate). None blocks implementation; each has a concrete plan handling.

**Placeholder scan.** New load-bearing code (RunShared, Repair/RepairMiddleware, fire_on_parse_failure, guardrails, TodoItem/WriteTodosTool/TodoListMiddleware, render_todos_block, curated wiring) is written out in full. Elided items are limited to test *harness boilerplate* that must be copied from a named neighboring test (the assertion bodies — the contracts — are given). This is deliberate: the exact `ScriptedModel`/`registry()` builder differs per test file and must be mirrored locally, not guessed.

**Type consistency.** `TodoHandle = Arc<Mutex<Vec<TodoItem>>>` is used identically in `todos.rs`, `middleware.rs` (`TodoListMiddleware`), `curated.rs` (`with_todos`), `assemble.rs`/frontends (`LoopParts.todos`), and `dispatch.rs`. `ToolCallCount(pub usize)` is written in `middleware.rs` and read in `loop_.rs` via `crate::ToolCallCount`. `Repair`/`RepairMiddleware`/`MAX_REPAIRS` names are consistent across `middleware.rs` and `loop_.rs`. Guardrail constructors (`ToolCallLimit::new`/`with_cap`, `ModelCallLimit::disabled`/`enabled_with_cap`) match between definition, assemble, and tests.

---

## Plan review log

- **2026-07-09 — plan review (buildability/coverage/decomposition, opus).** Verdict: **BUILDABLE AS WRITTEN, no BLOCKERs.** Spec coverage complete (every G/§5/§7 item mapped); decomposition sound (A3 correctly atomic — splitting the repair migration would leave a red intermediate state). All borrow-check, last-wins registry, byte-identical, and turn-contiguity claims verified against live source at 71e23d1. Findings dispositioned:
  - **MAJOR-1 (fixed in plan):** `repair_turn_leaves_stuck_counters_untouched` wires its stack through the shared `counter_agent(...)` helper, not inline — A3 Step 3e now names the helper as the edit site and notes the edit is benign for its other callers.
  - **MINOR-1 (fixed in plan):** the `protocol_repair_exhausted_emits_done` construction-site edit is now marked **mandatory** (an unedited run passes for the wrong reason — silent parity loss).
  - **MINOR-2 (accepted):** A3 Step 1's `parse_fail_cx` unit helper elides sink/ctx construction — within the plan's placeholder policy; the ingredients (`testkit`, `pub(crate)` `RunCx`/`MaintView`, `AssistantTurn: Default`) are confirmed in scope.
  - **MINOR-3 (escalated to gate):** Wave A ships an intermediate stack order `[Memory?, Curation, Stuck, Repair]`; Wave B reaches the final §5.6 order. By design (no test pins absolute stack membership by index); confirm a two-commit intermediate order is acceptable.
  - Corrections absorbed: actual `CuratedContext::new` sites = 23 (builder approach keeps all untouched); `LoopParts {` sites = 10 (all listed in B4).
- **2026-07-09 — adversarial plan review (architecture/decomposition, skeptical, opus).** Verdict: **architecture SOUND, no BLOCKER/MAJOR.** All seven attacked decisions survive against live source (S1 poison finding correct + soundly handled; S2 turn-contiguity equals today's reset-on-success incl. the overflow-inner-loop probe — parse fires once per turn, re-ask advances turn by one; S3 pre-turn guard redundant but harmless, no double-emit; S4 unconditional TodoList honest; RunShared threading airtight — no guard-across-await, no deadlock, node-hook reads strictly after the `buffer_unordered` batch completes; byte-identical repair incl. `Ok(())` return). One sharpened MINOR:
  - **S5 (fixed in plan):** leaving `build_snapshot` unchanged **breaks** the documented `est_total == pinned_tokens()+history` invariant (audit-7.3; pinned by `snapshot_est_total_includes_the_pinned_ledger`). Resolved by adding a `todos` segment mirroring `ledger` (Task B3) — verified frontend-safe (web renders segments generically with a fallback color, zero web change) and not a wire-shape change.
- **Escalations for the owner's plan-review gate:** S1 (spec-text correction — the §7 "contained as `Executed::Panicked`" wording is inaccurate); S2 (ratify the turn-contiguity reading of "in a turn-sequence"); S3 (ratify keeping the provably-redundant pre-turn guard per §5.7); S4 (unconditional `TodoListMiddleware` vs a new `cfg.planning` field); S5 (confirm the `todos`-segment addition is within §2's "no frontend change"); MINOR-3 (two-commit intermediate stack order).
- **2026-07-09 — owner plan-review gate (Kalen): ALL SIX ESCALATIONS RESOLVED. Gate CLOSED; plan cleared to execute.**
  - **S1 (poison) — APPROVED.** Implement only the true property (`RunShared::with` `into_inner` recovery, no cascade); do **not** write the false "wrap panic → `Executed::Panicked`" loop-level test. A spec Panel-log note flagging the §7 wording as inaccurate is added to the spec (done at gate).
  - **S2 (repair reset via turn-contiguity) — APPROVED** as the reading of "in a turn-sequence" (§5.3). `RepairState{repairs, last_fail_turn}` + non-contiguity reset ships as written.
  - **S3 (redundant pre-turn `ToolCallLimit` guard) — APPROVED.** Kept per §5.7 as the documented belt-and-suspenders backstop; `after_tools` remains the effective bound.
  - **S4 (unconditional `TodoListMiddleware`) — APPROVED.** Pushed unconditionally (no `cfg.planning` field); realizes "default on"; a config toggle stays a one-line future change.
  - **S5 (`todos` segment in `build_snapshot`) — APPROVED.** Within §2 ("no frontend change") and restores the audit-7.3 `est_total` invariant; ships as written in Task B3.
  - **MINOR-3 (two-commit intermediate stack order across Waves A/B) — APPROVED.** Wave A ships `[Memory?, Curation, Stuck, Repair]`; Wave B reaches the final §5.6 order. No test pins absolute stack membership by index.

## Execution handoff

**Plan complete and saved to `docs/superpowers/plans/2026-07-09-loop-middleware-wave.md`.**

Per the session mandate, this stops at the **plan-review gate** — no implementation. The next step (this session) is plan review per AGENTS.md (single reviewer: spec coverage, decomposition, buildability; plus a light adversarial pass scoped to architecture/decomposition since the wrap-contract/curation-touch decisions carry design weight), synthesized into the owner's plan-review gate. Discrepancies **S1–S5** must be dispositioned by the owner at that gate before any code lands.

Execution (a later session) will branch off `main` (`feature/loop-middleware-wave`) and run subagent-driven, task-by-task, with per-task spec-adherence review, a whole-branch review, and `ci.sh` green — as in Phases 1–2.
