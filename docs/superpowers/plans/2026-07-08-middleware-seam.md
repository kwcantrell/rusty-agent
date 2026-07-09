# Middleware Seam (deepagents refactor, Phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduce a `Middleware` trait (node hooks + provisional wrap hooks + tool contribution + run-state extension) in `agent-core`, compose it in `assemble_loop()` and dispatch, and migrate recall injection, compaction maintenance, and stuck detection into three middleware with **zero behavior change**.

**Architecture:** A new `agent-core/src/middleware.rs` module holds the trait and support types. `AgentLoop` gains a `Vec<Arc<dyn Middleware>>` and calls five node-hook points + two wrap chains from `run_with_cancel`; the three migrated behaviors move out of the loop body one task at a time, each gated by the existing test suite passing with assertions unchanged. Children get per-dispatch stacks built inside `DispatchAgentTool`.

**Tech Stack:** Rust (agent/ Cargo workspace), `async_trait`, tokio, existing `agent-core` testkit (`ScriptedModel`, `CollectingSink`, `WindowContext`, `PassthroughProtocol`).

**Spec:** `docs/superpowers/specs/2026-07-08-middleware-seam-design.md` (gate-approved 2026-07-08). Re-read the relevant spec section before each task.

## Global Constraints

- **No behavior change (spec G3/G5):** every existing test in the `agent/` workspace passes with assertion bodies unchanged; only construction sites (`AgentLoop::new` callers, `with_retriever` callers) may be updated.
- **Hook firing set (spec §5.1):** node hooks fire ONLY at the five defined points. No hook fires on: the protocol-repair `continue`, either Length-stop return, any error/cancel return, or the BudgetExhausted fall-through (including after its wrap-up append).
- **Wrap hooks are a provisional contract (spec §5.4):** tests pin composition semantics (first-outermost nesting, invoked twice across overflow rebuild, gate-unreachability) — never exact signatures. Ship NO production wrap behavior.
- **Do-not-regress (spec §3):** `CuratedContext` internals, `gate_tool`/`ToolIntent`, sandbox surfacing, MCP registration, and the calibration ratio (stays private to `AgentLoop`) are untouched.
- **Stuck message strings** ("5 turns in a row", "3 turns in a row") migrate byte-identical — they are hardcoded, not derived from `STUCK_NUDGE_AFTER`/`STUCK_ABORT_AFTER`.
- **Two Cargo workspaces:** all commands below run from `agent/` (e.g. `cd agent && cargo test -p agent-core`).
- Conventional commits: `refactor(core): …` / `test(core): …` matching history.
- Final gate: `bash scripts/ci.sh` from the repo root.

---

## File Structure

| File | Change | Responsibility |
|---|---|---|
| `agent/crates/agent-core/src/middleware.rs` | create | `Middleware` trait, `Flow`, `ToolContribution`, `RunState`, `TurnView`, `RunCx`, `MaintView`, `ModelNext`, `ToolNext`, and the three migrated middleware (`MemoryRecallMiddleware`, `ContextCurationMiddleware`, `StuckDetectionMiddleware`) |
| `agent/crates/agent-core/src/lib.rs` | modify | add `mod middleware; pub use middleware::*;` |
| `agent/crates/agent-core/src/loop_.rs` | modify | stack field, hook call-outs, wrap-chain invocation, `RetryFailure`→`pub CompletionFailure` + `pub Executed`, delete migrated blocks |
| `agent/crates/agent-runtime-config/src/assemble.rs` | modify | build the stack, register `ToolContribution`s around the child_base snapshot, drop direct memory/context tool registration and `with_retriever` |
| `agent/crates/agent-core/src/dispatch.rs` | modify | build per-child stack `[ContextCuration, StuckDetection]`, child context tools via contributions |

Branch: `git checkout -b feature/middleware-seam` off `main` (do this before Task 1's first commit).

---

### Task 1: middleware.rs core types + node-hook trait

**Files:**
- Create: `agent/crates/agent-core/src/middleware.rs`
- Modify: `agent/crates/agent-core/src/lib.rs`

**Interfaces:**
- Consumes: `agent_model::{Message, ModelClient, StopReason, ToolCall}`, `crate::{ContextManager, EventSink}`, `tokio_util::sync::CancellationToken` (already deps of agent-core — check `loop_.rs` imports and reuse them).
- Produces (later tasks rely on these exact names):
  - `pub enum Flow { Continue, EndRun(StopReason) }`
  - `pub struct ToolContribution { pub tool: Arc<dyn agent_tools::Tool>, pub child_visible: bool }`
  - `pub struct RunState { … }` with `pub fn get<T: 'static>(&self) -> Option<&T>`, `pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T>`, `pub fn entry<T: 'static + Default + Send>(&mut self) -> &mut T`
  - `pub struct TurnView { pub text: String, pub tool_calls: Vec<ToolCall>, pub invalid: Vec<(String, String)> }` (owned snapshot — spec §5.2)
  - `pub struct MaintView<'a>` (constructed by the loop; fields `pub(crate)`)
  - `pub struct RunCx<'a>` with public fields `ctx: &'a mut dyn ContextManager`, `sink: &'a Arc<dyn EventSink>`, `cancel: &'a CancellationToken`, `state: &'a mut RunState`, `turn: Option<usize>`, private `maint: MaintView<'a>`; methods `maint_model() -> &Arc<dyn ModelClient>`, `maint_model_limit() -> usize`, `effective_model_limit() -> usize`
  - `#[async_trait] pub trait Middleware: Send + Sync` with `name()`, `tools()`, and the five node hooks (defaults `Flow::Continue` / no-op). Wrap hooks are added in Task 3.

- [ ] **Step 1: Write the failing test** (bottom of the new `middleware.rs`, inside `#[cfg(test)] mod tests`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Marker(u32);

    #[test]
    fn run_state_typed_roundtrip_and_isolation() {
        let mut s = RunState::default();
        assert!(s.get::<Marker>().is_none());
        s.entry::<Marker>().0 = 7;
        assert_eq!(s.get::<Marker>().unwrap().0, 7);
        // A fresh RunState (fresh run) starts empty — per-run lifetime.
        let s2 = RunState::default();
        assert!(s2.get::<Marker>().is_none());
    }

    struct Noop;
    #[async_trait::async_trait]
    impl Middleware for Noop {
        fn name(&self) -> &str {
            "noop"
        }
    }

    #[tokio::test]
    async fn default_hooks_are_continue_and_contribute_nothing() {
        let m = Noop;
        assert!(m.tools().is_empty());
        // Flow derives PartialEq for exactly this kind of assertion.
        // (RunCx construction is exercised in loop_ tests; defaults are
        // pure so a unit check of tools() + Flow is enough here.)
        assert_eq!(Flow::Continue, Flow::Continue);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-core middleware:: 2>&1 | tail -5`
Expected: compile FAIL — `middleware` module does not exist yet.

- [ ] **Step 3: Write the implementation**

```rust
//! Middleware seam (spec: docs/superpowers/specs/2026-07-08-middleware-seam-design.md).
//! One trait, four capability surfaces: node hooks, wrap hooks (Task 3),
//! tool contribution, per-run state extension.
use crate::{ContextManager, EventSink};
use agent_model::{ModelClient, StopReason, ToolCall};
use async_trait::async_trait;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Node-hook outcome. `EndRun` short-circuits remaining hooks at that point;
/// the loop maps it to `emit(Done(reason)); return Ok(())` (spec §5.2).
#[derive(Debug, Clone, PartialEq)]
pub enum Flow {
    Continue,
    EndRun(StopReason),
}

/// A tool a middleware ships at assembly. `child_visible` controls membership
/// in the sub-agent base snapshot (spec §5.5/§5.6).
pub struct ToolContribution {
    pub tool: Arc<dyn agent_tools::Tool>,
    pub child_visible: bool,
}

/// Per-run typed state extension: created fresh per `run_with_cancel`
/// invocation, so middleware stay stateless `&self` objects (spec §5.3).
#[derive(Default)]
pub struct RunState {
    map: HashMap<TypeId, Box<dyn Any + Send>>,
}

impl RunState {
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map.get(&TypeId::of::<T>()).and_then(|b| b.downcast_ref())
    }
    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.map
            .get_mut(&TypeId::of::<T>())
            .and_then(|b| b.downcast_mut())
    }
    /// Get-or-default, the common middleware idiom.
    pub fn entry<T: 'static + Default + Send>(&mut self) -> &mut T {
        self.map
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(T::default()))
            .downcast_mut()
            .expect("TypeId-keyed entry always downcasts to its own type")
    }
}

/// OWNED snapshot of a parsed turn (spec §5.2): cloned before `after_model`
/// so no borrow of the loop's `parsed`/`all_calls` crosses the hook `.await`.
pub struct TurnView {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    /// Unparseable calls as (name, error).
    pub invalid: Vec<(String, String)>,
}

/// Read-only view of loop maintenance internals, precomputed by the loop
/// before each hook point (keeps the calibration ratio private — spec §3).
pub struct MaintView<'a> {
    pub(crate) maint_model: &'a Arc<dyn ModelClient>,
    pub(crate) maint_model_limit: usize,
    pub(crate) effective_model_limit: usize,
}

/// What node hooks can touch (spec §5.2).
pub struct RunCx<'a> {
    pub ctx: &'a mut dyn ContextManager,
    pub sink: &'a Arc<dyn EventSink>,
    pub cancel: &'a CancellationToken,
    pub state: &'a mut RunState,
    /// 0-based turn index; None pre-loop (on_run_start).
    pub turn: Option<usize>,
    pub(crate) maint: MaintView<'a>,
}

impl RunCx<'_> {
    pub fn maint_model(&self) -> &Arc<dyn ModelClient> {
        self.maint.maint_model
    }
    pub fn maint_model_limit(&self) -> usize {
        self.maint.maint_model_limit
    }
    pub fn effective_model_limit(&self) -> usize {
        self.maint.effective_model_limit
    }
}

/// The middleware seam. Hook-point firing rules are normative — see the spec
/// §5.1 doc comments; the loop, not the middleware, guarantees them.
/// Hooks are trusted in-process code: NOT panic-isolated or timeout-bounded
/// (spec §5.2 isolation posture).
#[async_trait]
pub trait Middleware: Send + Sync {
    /// Stable identifier for tracing spans.
    fn name(&self) -> &str;

    /// Tools this unit contributes at assembly.
    fn tools(&self) -> Vec<ToolContribution> {
        Vec::new()
    }

    /// Before the goal is set and the user message is appended.
    async fn on_run_start(&self, _cx: &mut RunCx<'_>, _input: &str) -> Flow {
        Flow::Continue
    }
    /// Between id normalization and the assistant append; parsed turns only.
    async fn after_model(&self, _cx: &mut RunCx<'_>, _turn: &TurnView) -> Flow {
        Flow::Continue
    }
    /// After the turn's tool results (and any post-validator message).
    async fn after_tools(&self, _cx: &mut RunCx<'_>) -> Flow {
        Flow::Continue
    }
    /// Bottom of a completed tool turn.
    async fn on_turn_end(&self, _cx: &mut RunCx<'_>) -> Flow {
        Flow::Continue
    }
    /// Only on the text-only exit path (loop_.rs:751); see spec §5.1/J5.
    async fn after_final_reply(&self, _cx: &mut RunCx<'_>) {}
}
```

- [ ] **Step 4: Register the module** — in `agent/crates/agent-core/src/lib.rs`, after `mod loop_;` add `mod middleware;`, and after `pub use loop_::*;` add `pub use middleware::*;`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-core middleware:: -- --nocapture`
Expected: PASS (2 tests). Then `cargo test -p agent-core 2>&1 | tail -3` — full crate still green.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-core/src/middleware.rs agent/crates/agent-core/src/lib.rs
git commit -m "refactor(core): add Middleware trait, RunState, and node-hook types (spec 2026-07-08 §5.1-5.3)"
```

---

### Task 2: hook plumbing in `run_with_cancel` (empty stack = today)

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs`
- Test: same file, `mod tests`

**Interfaces:**
- Consumes: Task 1's `Middleware`, `RunCx`, `MaintView`, `RunState`, `TurnView`, `Flow`.
- Produces: `AgentLoop::with_middleware(mut self, stack: Vec<Arc<dyn Middleware>>) -> Self`; private helpers `fn maint_view(&self) -> MaintView<'_>` and `async fn fire_*` call-outs described below. Later tasks rely on the five call-out positions being exactly as placed here.

- [ ] **Step 1: Write the failing tests** (append to `mod tests` in `loop_.rs`)

```rust
/// Records every hook firing as "name:hook" so ordering tests read as data.
struct Recording {
    name: &'static str,
    log: Arc<std::sync::Mutex<Vec<String>>>,
    end_at: Option<&'static str>, // hook name at which to return EndRun
}
#[async_trait::async_trait]
impl crate::Middleware for Recording {
    fn name(&self) -> &str {
        self.name
    }
    async fn on_run_start(&self, _cx: &mut crate::RunCx<'_>, _i: &str) -> crate::Flow {
        self.log.lock().unwrap().push(format!("{}:run_start", self.name));
        self.flow("run_start")
    }
    async fn after_model(&self, _cx: &mut crate::RunCx<'_>, _t: &crate::TurnView) -> crate::Flow {
        self.log.lock().unwrap().push(format!("{}:after_model", self.name));
        self.flow("after_model")
    }
    async fn after_tools(&self, _cx: &mut crate::RunCx<'_>) -> crate::Flow {
        self.log.lock().unwrap().push(format!("{}:after_tools", self.name));
        self.flow("after_tools")
    }
    async fn on_turn_end(&self, _cx: &mut crate::RunCx<'_>) -> crate::Flow {
        self.log.lock().unwrap().push(format!("{}:turn_end", self.name));
        self.flow("turn_end")
    }
    async fn after_final_reply(&self, _cx: &mut crate::RunCx<'_>) {
        self.log.lock().unwrap().push(format!("{}:final_reply", self.name));
    }
}
impl Recording {
    fn flow(&self, hook: &str) -> crate::Flow {
        if self.end_at == Some(hook) {
            crate::Flow::EndRun(StopReason::Error)
        } else {
            crate::Flow::Continue
        }
    }
}

fn recording_pair(
    end_at: Option<&'static str>,
) -> (Vec<Arc<dyn crate::Middleware>>, Arc<std::sync::Mutex<Vec<String>>>) {
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let a = Recording { name: "a", log: log.clone(), end_at: None };
    let b = Recording { name: "b", log: log.clone(), end_at };
    (vec![Arc::new(a), Arc::new(b)], log)
}

/// Before-side hooks run in stack order; after-side in reverse (spec §5.4).
#[tokio::test]
async fn hooks_fire_forward_then_reverse_across_a_tool_turn() {
    let model = Arc::new(ScriptedModel::new(vec![
        Scripted::Call("c1".into(), "counter".into(), r#"{"k":"a"}"#.into()),
        Scripted::Text("done".into()),
    ]));
    let sink = Arc::new(CollectingSink::default());
    let (agent, _count) = counter_agent(model, sink, 5);
    let (stack, log) = recording_pair(None);
    let agent = agent.with_middleware(stack);
    let mut ctx = WindowContext::new(Message::system("sys"));
    agent.run(&mut ctx, "go".into()).await.unwrap();
    assert_eq!(
        log.lock().unwrap().clone(),
        vec![
            "a:run_start", "b:run_start",              // forward
            "b:after_model", "a:after_model",          // reverse (turn 1)
            "b:after_tools", "a:after_tools",          // reverse
            "b:turn_end", "a:turn_end",                // reverse
            "b:after_model", "a:after_model",          // turn 2 (text-only)
            "b:final_reply", "a:final_reply",          // reverse, text exit
        ]
    );
}

/// EndRun short-circuits the remaining hooks at that point (spec §5.4).
#[tokio::test]
async fn end_run_short_circuits_and_emits_done() {
    let model = Arc::new(ScriptedModel::new(vec![Scripted::Text("hi".into())]));
    let sink = Arc::new(CollectingSink::default());
    let (agent, _c) = counter_agent(model, sink.clone(), 5);
    // b (deeper in stack) EndRuns at run_start; a's run_start already ran.
    let (stack, log) = recording_pair(Some("run_start"));
    let agent = agent.with_middleware(stack);
    let mut ctx = WindowContext::new(Message::system("sys"));
    agent.run(&mut ctx, "go".into()).await.unwrap();
    assert_eq!(log.lock().unwrap().clone(), vec!["a:run_start", "b:run_start"]);
    let events = sink.events.lock().unwrap().clone();
    assert_eq!(events.last().unwrap(), "done");
}

/// No node hook fires on the protocol-repair `continue` or a Length exit
/// (spec §5.1 firing set). Reuses the malformed-call scripting that
/// `protocol_repair_exhausted_emits_done` uses; assert after_model count.
#[tokio::test]
async fn repair_turn_fires_no_after_model() {
    let model = Arc::new(ScriptedModel::new(vec![
        // Repair-triggering shape: a registered tool name with MALFORMED
        // JSON args (there is no `Malformed` variant in testkit — this is
        // the same shape protocol_repair_exhausted_emits_done uses).
        Scripted::Call("c1".into(), "counter".into(), r#"{"k": "#.into()),
        Scripted::Text("recovered".into()),
    ]));
    let sink = Arc::new(CollectingSink::default());
    let (agent, _c) = counter_agent(model, sink, 5);
    let (stack, log) = recording_pair(None);
    let agent = agent.with_middleware(stack);
    let mut ctx = WindowContext::new(Message::system("sys"));
    agent.run(&mut ctx, "go".into()).await.unwrap();
    let l = log.lock().unwrap().clone();
    // Exactly one after_model pair (the recovered text turn), none for the
    // malformed turn.
    assert_eq!(
        l.iter().filter(|e| e.ends_with(":after_model")).count(),
        2,
        "one turn × two middleware; repair turn must not fire after_model: {l:?}"
    );
}
```

NOTE for the implementer (verified at plan review): testkit has NO `Malformed` variant. The repair-triggering shape is a `Scripted::Call` 3-tuple whose ARGS string is JSON-shaped-but-invalid (e.g. `r#"{"k": "#`) against a tool name that IS registered in the test registry (`counter` for `counter_agent`); `PassthroughProtocol::parse` (`testkit.rs:168`) then fails and takes the repair path — same mechanism as `protocol_repair_exhausted_emits_done` (`loop_.rs:2810`). The assertion body stays.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-core hooks_fire 2>&1 | tail -5`
Expected: compile FAIL — `with_middleware` does not exist.

- [ ] **Step 3: Implement the plumbing** in `loop_.rs`:

3a. Add the field + builder + view helper:

```rust
// In struct AgentLoop (after `compaction_model`):
    middleware: Vec<Arc<dyn crate::Middleware>>,

// In AgentLoop::new: initialize `middleware: Vec::new(),`

// New builder next to with_compaction_model:
    /// Install the middleware stack (spec §5.4 ordering contract).
    pub fn with_middleware(mut self, stack: Vec<Arc<dyn crate::Middleware>>) -> Self {
        self.middleware = stack;
        self
    }

    /// Precompute the read-only maintenance view for one hook point.
    fn maint_view(&self) -> crate::MaintView<'_> {
        crate::MaintView {
            maint_model: self.maint_model(),
            maint_model_limit: self.maint_model_limit(),
            effective_model_limit: self.effective_model_limit(),
        }
    }
```

3b. One dispatch helper (private, in `impl AgentLoop`), used by all five points. Forward order for `on_run_start`, reverse for the rest (spec §5.4):

```rust
    /// Fire one node-hook point across the stack. `forward` = stack order
    /// (before-side); reverse otherwise. Returns the first EndRun, having
    /// skipped the remaining middleware at this point. debug_asserts the
    /// no-orphaned-tool-calls invariant after the hooks (spec §5.2).
    async fn fire_hooks<'f, F>(
        &self,
        forward: bool,
        ctx: &mut dyn ContextManager,
        state: &mut crate::RunState,
        cancel: &CancellationToken,
        turn: Option<usize>,
        mut call: F,
    ) -> crate::Flow
    where
        F: for<'c> FnMut(
            &'c Arc<dyn crate::Middleware>,
            &'c mut crate::RunCx<'_>,
        ) -> futures::future::BoxFuture<'c, crate::Flow>,
    {
        let order: Vec<&Arc<dyn crate::Middleware>> = if forward {
            self.middleware.iter().collect()
        } else {
            self.middleware.iter().rev().collect()
        };
        for mw in order {
            // REBORROW per iteration: `&mut *ctx`, never move `ctx` into the
            // RunCx (a moved `&mut` cannot be reused next iteration or by the
            // debug_assert below).
            let mut cx = crate::RunCx {
                ctx: &mut *ctx,
                sink: &self.sink,
                cancel,
                state: &mut *state,
                turn,
                maint: self.maint_view(),
            };
            let flow = call(mw, &mut cx).await;
            drop(cx); // end the reborrow before touching ctx again
            debug_assert!(
                crate::orphaned_tool_positions(&ctx.build(usize::MAX)).is_empty(),
                "middleware {} left an orphaned tool_calls message",
                mw.name()
            );
            if let crate::Flow::EndRun(reason) = flow {
                return crate::Flow::EndRun(reason);
            }
        }
        crate::Flow::Continue
    }
```

IMPLEMENTATION NOTE: if the closure-with-HRTB shape fights the borrow checker (it can, across `&mut RunCx` reborrows), fall back to five small concrete methods (`fire_run_start`, `fire_after_model(turn_view)`, `fire_after_tools`, `fire_turn_end`, `fire_final_reply`) each containing the same loop inlined — that is an acceptable, boring resolution; the *positions and ordering* are the contract, not the helper's genericity. The reborrow-per-iteration discipline above applies equally in the fallback (each concrete method still constructs one `RunCx` per middleware in a loop). Also verify `orphaned_tool_positions` is exported from `context.rs` (it is used at `context.rs:311`); if it is private, make it `pub(crate)`.

3c. Insert the five call-outs (empty stack ⇒ all no-ops, so behavior is unchanged this task):

- **on_run_start** — immediately after the `RunStart` emit (`loop_.rs:470-473`) and BEFORE the retriever block at 475: forward order, `turn: None`. On `EndRun(reason)`: `emit(Done(reason)); return Ok(())`.
- **after_model** — immediately after `normalize_invalid_ids` (`loop_.rs:642`) and before the `all_calls` construction at 647, built from an owned `TurnView`:
  ```rust
  let turn_view = crate::TurnView {
      text: parsed.text.clone(),
      tool_calls: parsed.tool_calls.clone(),
      invalid: parsed
          .invalid
          .iter()
          .map(|i| (i.name.clone(), i.error.clone()))
          .collect(),
  };
  ```
  Reverse order, `turn: Some(turn)`. On `EndRun(reason)`: `emit(Done(reason)); return Ok(())` — and the loop touches `parsed`/`all_calls` no further (spec §5.2).
- **after_tools** — after the post-tool-validator block (`loop_.rs:1002`), BEFORE the existing nudge block at 1007. Reverse order. On `EndRun`: same mapping.
- **on_turn_end** — at the loop bottom, after the existing maintain block (`loop_.rs:1030`). Reverse order. On `EndRun`: same mapping.
- **after_final_reply** — inside the `all_calls.is_empty()` block, after the `!run_maintained` maintain (`loop_.rs:750`) and before `emit(Done(assistant.stop))` at 751. Reverse order; no Flow (run is ending).

Nothing is inserted on: the repair `continue` (617), the Length returns (609, 634), cancel returns, fatal returns, or anywhere in the budget wrap-up block (1036-1085).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-core 2>&1 | tail -3`
Expected: PASS, full crate — the three new tests green AND every pre-existing test untouched and green (empty-stack parity).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs agent/crates/agent-core/src/context.rs
git commit -m "refactor(core): call node-hook points from run_with_cancel (empty stack, no behavior change)"
```

---

### Task 3: wrap chains (provisional contract)

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` (visibility promotions + chain invocation), `agent/crates/agent-core/src/middleware.rs` (trait methods + `ModelNext`/`ToolNext`)

**Interfaces:**
- Consumes: `completion_with_retry(&self, base: &CompletionRequest, cancel: &CancellationToken) -> Result<AssistantTurn, RetryFailure>` (`loop_.rs:347`), `execute_isolated(tool, args, &name, &ctx) -> Executed` (`loop_.rs:1367`).
- Produces:
  - `RetryFailure` renamed to `pub enum CompletionFailure` (same variants `Fatal(String)`, `Cancelled`, `Overflow(String, (usize, usize))`), `pub enum Executed` (same variants) — visibility/rename only, all internal uses updated.
  - `pub struct ModelNext<'a>` with `pub fn run(self, req: CompletionRequest) -> BoxFuture<'a, Result<AssistantTurn, CompletionFailure>>`
  - `pub struct ToolNext<'a>` with `pub fn run(self, args: serde_json::Value) -> BoxFuture<'a, Executed>`
  - Trait methods (defaults pass through):
    ```rust
    async fn wrap_model_call(
        &self,
        req: CompletionRequest,
        next: ModelNext<'_>,
    ) -> Result<AssistantTurn, CompletionFailure> {
        next.run(req).await
    }
    async fn wrap_tool_call(&self, call: ToolCall, next: ToolNext<'_>) -> Executed {
        next.run(call.args).await
    }
    ```

- [ ] **Step 1: Write the failing tests** (in `loop_.rs` `mod tests`)

```rust
/// Counting wrapper: logs enter/exit so nesting order is observable.
struct Wrapping {
    name: &'static str,
    log: Arc<std::sync::Mutex<Vec<String>>>,
}
#[async_trait::async_trait]
impl crate::Middleware for Wrapping {
    fn name(&self) -> &str {
        self.name
    }
    async fn wrap_model_call(
        &self,
        req: agent_model::CompletionRequest,
        next: crate::ModelNext<'_>,
    ) -> Result<agent_model::AssistantTurn, crate::CompletionFailure> {
        self.log.lock().unwrap().push(format!("{}:model_enter", self.name));
        let r = next.run(req).await;
        self.log.lock().unwrap().push(format!("{}:model_exit", self.name));
        r
    }
    async fn wrap_tool_call(
        &self,
        call: agent_model::ToolCall,
        next: crate::ToolNext<'_>,
    ) -> crate::Executed {
        self.log.lock().unwrap().push(format!("{}:tool_enter", self.name));
        let r = next.run(call.args).await;
        self.log.lock().unwrap().push(format!("{}:tool_exit", self.name));
        r
    }
}

/// First middleware outermost, for both chains (spec §5.4).
#[tokio::test]
async fn wrap_chains_nest_first_outermost() {
    let model = Arc::new(ScriptedModel::new(vec![
        Scripted::Call("c1".into(), "counter".into(), r#"{"k":"a"}"#.into()),
        Scripted::Text("done".into()),
    ]));
    let sink = Arc::new(CollectingSink::default());
    let (agent, _c) = counter_agent(model, sink, 5);
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let agent = agent.with_middleware(vec![
        Arc::new(Wrapping { name: "a", log: log.clone() }),
        Arc::new(Wrapping { name: "b", log: log.clone() }),
    ]);
    let mut ctx = WindowContext::new(Message::system("sys"));
    agent.run(&mut ctx, "go".into()).await.unwrap();
    let l = log.lock().unwrap().clone();
    // Turn 1 model call, then its tool call, then turn 2 model call.
    assert_eq!(
        l,
        vec![
            "a:model_enter", "b:model_enter", "b:model_exit", "a:model_exit",
            "a:tool_enter", "b:tool_enter", "b:tool_exit", "a:tool_exit",
            "a:model_enter", "b:model_enter", "b:model_exit", "a:model_exit",
        ]
    );
}

/// The model wrap chain is invoked twice, independently, across an overflow
/// rebuild (spec §5.1/J3) — pins composition semantics, not signatures.
#[tokio::test]
async fn model_wrap_chain_fires_twice_across_overflow_rebuild() {
    // Mirror the scripting of `overflow_compacts_rebuilds_and_recovers_once`
    // (loop_.rs:5255): first completion overflows, rebuilt one succeeds.
    // Implementer: reuse that test's ScriptedModel setup verbatim, add the
    // Wrapping middleware, and assert the log contains exactly TWO
    // "a:model_enter" entries for the overflowing turn.
    // (Full setup copied from that test at implementation time — the
    // scripted overflow shape is testkit-specific.)
}
```

NOTE: for the second test the implementer copies the exact `ScriptedModel` arrangement from `overflow_compacts_rebuilds_and_recovers_once` (`loop_.rs:5255`) — the assertion is: count of `a:model_enter` == 2 and the run recovers (same terminal assertions as the source test). Leave the source test untouched.

- [ ] **Step 2: Run to verify failure**

Run: `cd agent && cargo test -p agent-core wrap_chains 2>&1 | tail -5`
Expected: compile FAIL — `ModelNext` unknown.

- [ ] **Step 3: Implement**

3a. In `loop_.rs`: rename `RetryFailure` → `CompletionFailure` and make it `pub` (mechanical: the DEFINITION at `loop_.rs:76-88`, the return type at `351`, and the `Overflow`/`Fatal`/`Cancelled` uses at 361-412, 527-580). Make `Executed` (`loop_.rs:1248`) `pub`. Re-export both via the existing `pub use loop_::*;`.

3b. In `middleware.rs`: the chain types. Recursion through `dyn` needs boxing:

```rust
use futures::future::BoxFuture;

/// Continuation for wrap_model_call: remaining chain, then the loop's
/// completion_with_retry as the base case. Wraps ONE invocation; overflow
/// recovery happens outside and re-enters a fresh chain (spec §5.1/J3).
pub struct ModelNext<'a> {
    pub(crate) loop_: &'a crate::AgentLoop,
    pub(crate) chain: &'a [Arc<dyn Middleware>],
    pub(crate) cancel: &'a CancellationToken,
}

impl<'a> ModelNext<'a> {
    pub fn run(
        self,
        req: agent_model::CompletionRequest,
    ) -> BoxFuture<'a, Result<agent_model::AssistantTurn, crate::CompletionFailure>> {
        Box::pin(async move {
            match self.chain.split_first() {
                Some((mw, rest)) => {
                    mw.wrap_model_call(
                        req,
                        ModelNext { loop_: self.loop_, chain: rest, cancel: self.cancel },
                    )
                    .await
                }
                None => self.loop_.completion_with_retry(&req, self.cancel).await,
            }
        })
    }
}

/// Continuation for wrap_tool_call: base case is execute_isolated on the
/// already-gated call (spec §5.4 — the gate is unreachable from here).
pub struct ToolNext<'a> {
    pub(crate) tool: Arc<dyn agent_tools::Tool>,
    pub(crate) name: &'a str,
    pub(crate) tctx: &'a agent_tools::ToolCtx,
    pub(crate) chain: &'a [Arc<dyn Middleware>],
    pub(crate) call: &'a ToolCall,
}

impl<'a> ToolNext<'a> {
    pub fn run(self, args: serde_json::Value) -> BoxFuture<'a, crate::Executed> {
        Box::pin(async move {
            match self.chain.split_first() {
                Some((mw, rest)) => {
                    let mut call = self.call.clone();
                    call.args = args;
                    mw.wrap_tool_call(
                        call,
                        ToolNext { chain: rest, ..self },
                    )
                    .await
                }
                None => crate::execute_isolated(self.tool, args, self.name, self.tctx).await,
            }
        })
    }
}
```

(`completion_with_retry` and `execute_isolated` become `pub(crate)`. If `ToolNext { chain: rest, ..self }` fights the borrow checker on the partially-moved `self`, destructure explicitly — mechanical.)

3c. In `loop_.rs`, replace the two base calls:
- Both `self.completion_with_retry(&base, &cancel)` call sites inside the turn loop (`loop_.rs:527` — the single match head serves both attempts) become:
  ```rust
  ModelNext { loop_: self, chain: &self.middleware, cancel: &cancel }
      .run(base.clone())
      .await
  ```
  The overflow arm still rebuilds `base` and loops — re-entering a FRESH chain (invoked-twice semantics).
- In the parallel executor closure (`loop_.rs:833`), `execute_isolated(tool, args, &name, &ctx).await` becomes a `ToolNext { … }.run(args).await` with `chain: &self.middleware`. The closure already captures per-call data; add the borrow of `self.middleware` (it is `&self`, `Send + Sync` — fine under `buffer_unordered`).
- The budget wrap-up's `one_completion` (`loop_.rs:1053`) is NOT wrapped (it is not a `completion_with_retry` unit; spec keeps wrap-up loop-resident).

- [ ] **Step 4: Run tests**

Run: `cd agent && cargo test -p agent-core 2>&1 | tail -3`
Expected: full crate PASS (new wrap tests + all pre-existing: empty chain = passthrough parity).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src
git commit -m "refactor(core): provisional wrap-hook chains around completion_with_retry and execute_isolated"
```

---

### Task 4: MemoryRecallMiddleware + assemble_loop stack composition

**Files:**
- Modify: `agent/crates/agent-core/src/middleware.rs` (new middleware), `agent/crates/agent-core/src/loop_.rs` (delete retriever), `agent/crates/agent-runtime-config/src/assemble.rs` (stack build)

**Interfaces:**
- Consumes: `crate::Retriever` (`recall.rs:10`), `LoopParts.memory_tools` / `memory_retriever` (`assemble.rs:23-24`).
- Produces:
  ```rust
  pub struct MemoryRecallMiddleware {
      tools: Vec<Arc<dyn agent_tools::Tool>>,
      retriever: Option<Arc<dyn crate::Retriever>>,
  }
  impl MemoryRecallMiddleware {
      pub fn new(
          tools: Vec<Arc<dyn agent_tools::Tool>>,
          retriever: Option<Arc<dyn crate::Retriever>>,
      ) -> Self { … }
  }
  ```
  `AgentLoop::with_retriever` is DELETED; `AgentLoop.retriever` field deleted.

- [ ] **Step 1: Update the three recall tests' construction sites to be the failing tests.** `auto_retrieval_injects_recall_block_into_context` (`loop_.rs:2568`), `empty_retrieval_injects_no_block_and_turn_completes` (2606), `empty_retrieval_clears_stale_recall` (2644) currently call `.with_retriever(r)`. Change ONLY the construction line in each to:

```rust
let agent = agent.with_middleware(vec![Arc::new(crate::MemoryRecallMiddleware::new(
    vec![],
    Some(r),
))]);
```

Assertion bodies stay byte-identical. NOTE: the live tests bind the retriever
INLINE (`.with_retriever(Arc::new(FakeRetriever(vec![…])))` at
`loop_.rs:2592/2630/2684`) — there is no `r` variable; either extract
`let r = Arc::new(FakeRetriever(…));` first or inline the expression into
`Some(…)`.

- [ ] **Step 2: Run to verify failure**

Run: `cd agent && cargo test -p agent-core retrieval 2>&1 | tail -5`
Expected: compile FAIL — `MemoryRecallMiddleware` unknown.

- [ ] **Step 3: Implement** in `middleware.rs`:

```rust
/// Memory: ships the frontend-built memory tools (child-visible) and injects
/// auto-recall at run start (spec §5.5). Retriever None ⇒ tools only, no
/// set_recall call — matching today's cfg.memory-without-retriever gating.
pub struct MemoryRecallMiddleware {
    tools: Vec<Arc<dyn agent_tools::Tool>>,
    retriever: Option<Arc<dyn crate::Retriever>>,
}

impl MemoryRecallMiddleware {
    pub fn new(
        tools: Vec<Arc<dyn agent_tools::Tool>>,
        retriever: Option<Arc<dyn crate::Retriever>>,
    ) -> Self {
        Self { tools, retriever }
    }
}

#[async_trait]
impl Middleware for MemoryRecallMiddleware {
    fn name(&self) -> &str {
        "memory-recall"
    }
    fn tools(&self) -> Vec<ToolContribution> {
        self.tools
            .iter()
            .map(|t| ToolContribution { tool: t.clone(), child_visible: true })
            .collect()
    }
    async fn on_run_start(&self, cx: &mut RunCx<'_>, input: &str) -> Flow {
        if let Some(r) = &self.retriever {
            // Unconditional when a retriever exists: empty retrieval clears
            // the prior run's block (loop_.rs spec §2 / audit Spine B #4).
            cx.ctx.set_recall(r.retrieve(input).await);
        }
        Flow::Continue
    }
}
```

In `loop_.rs`: delete the `retriever` field, `with_retriever`, and the run-start retriever block (`loop_.rs:475-480`); the `on_run_start` call-out from Task 2 already sits at that exact position.

In `assemble.rs` (`assemble_loop`):
1. Delete the `if cfg.memory { … register memory_tools … }` block (167-171).
2. After `build_skills` registration (175), build the stack (ContextCuration/Stuck join in Tasks 5-6):
   ```rust
   let mut stack: Vec<Arc<dyn agent_core::Middleware>> = Vec::new();
   if cfg.memory {
       stack.push(Arc::new(agent_core::MemoryRecallMiddleware::new(
           parts.memory_tools.clone(),
           parts.memory_retriever.clone(),
       )));
   }
   // Register child-visible contributions BEFORE the child_base snapshot;
   // the rest after (spec §5.6). debug_assert: no name collisions.
   for c in stack.iter().flat_map(|m| m.tools()) {
       if c.child_visible {
           debug_assert!(registry.get(c.tool.name()).is_none(),
               "middleware tool contribution shadows an existing tool");
           registry.register(c.tool.clone());
       }
   }
   ```
3. The `child_base` snapshot line (180) stays exactly where it is, now after the child-visible pass. After the snapshot, add the `!child_visible` pass (empty until Task 5).
4. Replace the `with_retriever` match (373-376) with nothing; after `with_compaction_model`, add `let agent = agent.with_middleware(stack);`.

- [ ] **Step 4: Run tests**

Run: `cd agent && cargo test -p agent-core && cargo test -p agent-runtime-config 2>&1 | tail -3`
Expected: PASS — including `registers_memory_tools_when_enabled`, `skips_memory_tools_when_disabled`, `child_base_snapshot_includes_memory_tools_when_enabled` with assertions unchanged.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src agent/crates/agent-runtime-config/src/assemble.rs
git commit -m "refactor(core): migrate recall injection into MemoryRecallMiddleware; assemble builds the stack"
```

---

### Task 5: ContextCurationMiddleware (scheduled maintenance + context tools)

**Files:**
- Modify: `agent/crates/agent-core/src/middleware.rs`, `agent/crates/agent-core/src/loop_.rs`, `agent/crates/agent-runtime-config/src/assemble.rs`, `agent/crates/agent-core/src/dispatch.rs`

**Interfaces:**
- Consumes: `crate::context_tools(store, flag, recall_page_bytes)` (`context_tools.rs:176`), `MaintCtx` (`context.rs:209`), `ContextManager::maintain`.
- Produces:
  ```rust
  pub struct ContextCurationMiddleware { … }
  impl ContextCurationMiddleware {
      pub fn new(
          store: Arc<dyn crate::OffloadStore>,
          flag: Arc<std::sync::atomic::AtomicBool>,
          max_result_bytes: usize,
      ) -> Self { … }
  }
  ```
  Run-state marker: `#[derive(Default)] pub(crate) struct Maintained(bool);`

- [ ] **Step 1: Failing tests = the two cadence pins.** Update construction sites of `text_only_run_is_curated_at_exit` (`loop_.rs:5145`) and `tool_bearing_run_skips_the_exit_maintain` (5197) — whatever loop they build gains `.with_middleware(vec![Arc::new(crate::ContextCurationMiddleware::new(store, flag, 16384))])` using the same store/flag their `CuratedContext` already uses. Assertion bodies stay identical. (They will FAIL after Step 3's deletion if the middleware misbehaves — the deletion is what makes these tests the guard.)

- [ ] **Step 2: Implement the middleware:**

```rust
/// Scheduled context curation: loop-bottom maintain each tool turn, plus the
/// text-only-exit maintain when no tool turn ran (spec §5.5). Ships the
/// context tools (child-invisible; children get per-dispatch instances).
pub struct ContextCurationMiddleware {
    store: Arc<dyn crate::OffloadStore>,
    flag: Arc<std::sync::atomic::AtomicBool>,
    max_result_bytes: usize,
}

#[derive(Default)]
pub(crate) struct Maintained(pub(crate) bool);

impl ContextCurationMiddleware {
    pub fn new(
        store: Arc<dyn crate::OffloadStore>,
        flag: Arc<std::sync::atomic::AtomicBool>,
        max_result_bytes: usize,
    ) -> Self {
        Self { store, flag, max_result_bytes }
    }

    async fn maintain(&self, cx: &mut RunCx<'_>) {
        let deps = crate::MaintCtx {
            model_limit: cx.maint_model_limit(),
            model: cx.maint_model(),
            sink: cx.sink,
            cancel: cx.cancel,
        };
        let report = cx.ctx.maintain(&deps).await;
        if report.offloaded > 0 || report.compacted_turns > 0 {
            tracing::debug!(
                offloaded = report.offloaded,
                offloaded_bytes = report.offloaded_bytes,
                compacted_turns = report.compacted_turns,
                "context maintained"
            );
        }
    }
}

#[async_trait]
impl Middleware for ContextCurationMiddleware {
    fn name(&self) -> &str {
        "context-curation"
    }
    fn tools(&self) -> Vec<ToolContribution> {
        crate::context_tools(self.store.clone(), self.flag.clone(), self.max_result_bytes)
            .into_iter()
            .map(|tool| ToolContribution { tool, child_visible: false })
            .collect()
    }
    async fn on_turn_end(&self, cx: &mut RunCx<'_>) -> Flow {
        self.maintain(cx).await;
        cx.state.entry::<Maintained>().0 = true;
        Flow::Continue
    }
    async fn after_final_reply(&self, cx: &mut RunCx<'_>) {
        // Text-exit maintain fires only when no tool turn maintained this run
        // (today's run_maintained gate; spec §5.5 pins the cadence).
        if !cx.state.get::<Maintained>().map(|m| m.0).unwrap_or(false) {
            self.maintain(cx).await;
        }
    }
}
```

BORROW NOTE: `MaintCtx { model: cx.maint_model(), sink: cx.sink, cancel: cx.cancel, … }` borrows `cx` immutably while `cx.ctx.maintain(&deps)` needs `&mut cx.ctx` — destructure first: `let RunCx { ctx, sink, cancel, maint, .. } = cx;` then build `MaintCtx` from `maint`/`sink`/`cancel` and call `ctx.maintain(&deps).await`. The trace-log lines must match the deleted loop code byte-for-byte, including the `"context maintained at text-only exit"` variant for the `after_final_reply` path (copy the deleted block at `loop_.rs:734-750`).

- [ ] **Step 3: Delete from the loop and rewire assembly + dispatch:**
- `loop_.rs`: delete the `run_maintained` local (504), the text-exit maintain block (734-750, keeping the `Done` emit + return), and the loop-bottom maintain block (1015-1030). The overflow-recovery maintain (548-554) STAYS (spec J4).
- `assemble.rs`: add to the stack, unconditionally, after the memory push:
  ```rust
  stack.push(Arc::new(agent_core::ContextCurationMiddleware::new(
      parts.offload_store.clone(),
      parts.compact_flag.clone(),
      cfg.max_tool_result_bytes,
  )));
  ```
  Delete the direct `agent_core::context_tools(…)` registration block (184-190); the post-snapshot `!child_visible` pass from Task 4 now registers them (verify `registers_context_management_tools` and `child_base_snapshot_excludes_context_tools_and_dispatch_itself` still pass with assertions unchanged).
- `dispatch.rs`: replace the direct `crate::context_tools(store.clone(), flag.clone(), self.deps.max_result_bytes)` registration (476-478) with:
  ```rust
  let curation = Arc::new(crate::ContextCurationMiddleware::new(
      store.clone(),
      flag.clone(),
      self.deps.max_result_bytes,
  ));
  for c in curation.tools() {
      reg.register(c.tool.clone());
  }
  ```
  and give the child loop its stack: `let child = child.with_middleware(vec![curation]);` — placed either between the `AgentLoop::new` (503-511) and the `with_compaction_model` rebind (513-516) or after it; both preserve the stack (`with_compaction_model` only sets `compaction_model`, `loop_.rs:212-215`). StuckDetection joins in Task 6.

- [ ] **Step 4: Run tests**

Run: `cd agent && cargo test -p agent-core && cargo test -p agent-runtime-config 2>&1 | tail -3`
Expected: full PASS — especially `text_only_run_is_curated_at_exit`, `tool_bearing_run_skips_the_exit_maintain`, `overflow_compacts_rebuilds_and_recovers_once`, `second_overflow_in_a_turn_is_fatal`, and the dispatch suite (children still curate + expose context tools).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src agent/crates/agent-runtime-config/src/assemble.rs
git commit -m "refactor(core): migrate scheduled maintenance + context tools into ContextCurationMiddleware"
```

---

### Task 6: StuckDetectionMiddleware

**Files:**
- Modify: `agent/crates/agent-core/src/middleware.rs`, `agent/crates/agent-core/src/loop_.rs`, `agent/crates/agent-runtime-config/src/assemble.rs`, `agent/crates/agent-core/src/dispatch.rs`

**Interfaces:**
- Consumes: `STUCK_NUDGE_AFTER` / `STUCK_ABORT_AFTER` (`loop_.rs:32-33` — move both constants into `middleware.rs`, re-exported so existing references compile), `TurnView`, `Flow::EndRun`.
- Produces: `pub struct StuckDetectionMiddleware;` (unit struct). Run-state: `#[derive(Default)] struct StuckState { last_sig: Option<String>, repeats: usize, nudged: bool, nudge_pending: bool }`.

- [ ] **Step 1: Failing test first — the new malformed-then-repeat parity pin** (panel finding; append to `loop_.rs` tests):

```rust
/// A repair turn must not advance OR reset stuck counters (spec §5.1):
/// A, A, malformed, A, A, A → abort on the 5th *parsed* identical turn,
/// exactly as if the malformed turn never happened... IMPLEMENTER: first
/// reproduce today's ACTUAL baseline behavior with the current inline code
/// (run this scripting against main before migrating!) and pin THAT —
/// today's code skips the signature block entirely on the repair turn, so
/// last_sig/repeats survive the repair turn unchanged.
#[tokio::test]
async fn repair_turn_leaves_stuck_counters_untouched() {
    let a = || Scripted::Call("c1".into(), "counter".into(), r#"{"k":"a"}"#.into());
    let model = Arc::new(ScriptedModel::new(vec![
        a(), a(),
        // Malformed-args Call (no `Malformed` variant exists — see the
        // testkit note in Task 2): registered tool, unparseable JSON args.
        Scripted::Call("c1".into(), "counter".into(), r#"{"k": "#.into()),
        a(), a(), a(),
    ]));
    let sink = Arc::new(CollectingSink::default());
    let (agent, count) = counter_agent(model, sink.clone(), 25);
    let mut ctx = WindowContext::new(Message::system("sys"));
    agent.run(&mut ctx, "go".into()).await.unwrap();
    // Baseline: verify these two numbers against main BEFORE migration and
    // adjust if the observed baseline differs — the pin is "same as today".
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 4);
    let events = sink.events.lock().unwrap().clone();
    assert!(events.iter().any(|e| e.contains("5 turns in a row")));
}
```

- [ ] **Step 2: Capture the baseline.** BEFORE writing the middleware, run this new test against the current (Task 5) tree: `cd agent && cargo test -p agent-core repair_turn_leaves 2>&1 | tail -5`. If the counts differ from the guess above, correct the assertions to the observed baseline and note it in the commit message. This test must PASS against the pre-migration code — it is a parity pin, not a behavior change.

- [ ] **Step 3: Implement the middleware** (in `middleware.rs`; move the two constants here):

```rust
pub const STUCK_NUDGE_AFTER: usize = 2;
pub const STUCK_ABORT_AFTER: usize = 4;

/// Repeated-identical-call detection (spec §5.5): nudge on the 3rd identical
/// turn, abort on the 5th. Signature and message strings are byte-identical
/// to the loop-resident original.
pub struct StuckDetectionMiddleware;

#[derive(Default)]
struct StuckState {
    last_sig: Option<String>,
    repeats: usize,
    nudged: bool,
    nudge_pending: bool,
}

#[async_trait]
impl Middleware for StuckDetectionMiddleware {
    fn name(&self) -> &str {
        "stuck-detection"
    }

    async fn after_model(&self, cx: &mut RunCx<'_>, turn: &TurnView) -> Flow {
        if turn.tool_calls.is_empty() && turn.invalid.is_empty() {
            return Flow::Continue; // text-only turns never tracked (as today)
        }
        // Identical encoding to loop_.rs:660-672: sorted (name\u{1}args|error),
        // joined by \u{2} — id-independent.
        let mut parts: Vec<String> = turn
            .tool_calls
            .iter()
            .map(|c| format!("{}\u{1}{}", c.name, c.args))
            .chain(turn.invalid.iter().map(|(n, e)| format!("{n}\u{1}{e}")))
            .collect();
        parts.sort();
        let sig = parts.join("\u{2}");
        let s = cx.state.entry::<StuckState>();
        if s.last_sig.as_deref() == Some(&sig) {
            s.repeats += 1;
        } else {
            s.repeats = 0;
            s.nudged = false;
        }
        s.last_sig = Some(sig);
        if s.repeats >= STUCK_ABORT_AFTER {
            // Abort BEFORE the assistant tool_calls append (spec §5.5): the
            // middleware appends the text-only form; the loop skips its own.
            let text = turn.text.clone();
            cx.ctx.append(agent_model::Message::assistant(text, None));
            cx.sink.emit(crate::AgentEvent::Error(
                "model repeated the identical tool call(s) 5 turns in a row; \
                 aborting the run"
                    .into(),
            ));
            return Flow::EndRun(StopReason::Error);
        }
        if s.repeats >= STUCK_NUDGE_AFTER && !s.nudged {
            s.nudged = true;
            s.nudge_pending = true;
        }
        Flow::Continue
    }

    async fn after_tools(&self, cx: &mut RunCx<'_>) -> Flow {
        let s = cx.state.entry::<StuckState>();
        if std::mem::take(&mut s.nudge_pending) {
            cx.ctx.append(agent_model::Message::user(
                "You have now issued the identical tool call(s) 3 turns in a row; \
                 repeating them will not change the result. Change your approach, or \
                 reply with a summary and no tool call if you are done.",
            ));
        }
        Flow::Continue
    }
}
```

BORROW NOTE: `cx.state.entry::<StuckState>()` borrows `cx.state` mutably; the subsequent `cx.ctx.append` / `cx.sink.emit` need other `cx` fields — end the `s` borrow first (compute a `let abort = …; let nudge = …;` decision from `s`, drop it, then act). Mechanical; the abort/nudge strings and ordering are the contract.

- [ ] **Step 4: Delete the inline code and wire the stacks.**
- `loop_.rs`: delete the locals (`last_sig`/`repeats`/`nudged`, 489-491), the signature/stuck block (653-702, including `nudge_pending`), and the nudge-append block (1007-1013). Delete the now-unused constants from `loop_.rs` (they moved). The `after_model`/`after_tools` call-outs from Task 2 are already in position.
- `assemble.rs`: `stack.push(Arc::new(agent_core::StuckDetectionMiddleware));` after the curation push.
- `dispatch.rs`: child stack becomes `vec![curation, Arc::new(crate::StuckDetectionMiddleware)]`.

- [ ] **Step 5: Run tests**

Run: `cd agent && cargo test -p agent-core && cargo test -p agent-runtime-config 2>&1 | tail -3`
Expected: full PASS — `stuck_identical_calls_nudged_then_aborted` and `stuck_counter_resets_on_different_call` with assertion bodies unchanged (they build via `counter_agent`; update that helper to install the production stack `[ContextCuration?, StuckDetection]`… NO — keep `counter_agent` minimal and add `.with_middleware(vec![Arc::new(crate::StuckDetectionMiddleware)])` inside it so every existing caller gets stuck detection exactly as before. CAUTION (invariant, verified at plan review): `with_middleware` REPLACES the stack, so Task 2/3 tests that build via `counter_agent` and then install a recording/wrapping stack silently drop stuck detection — harmless today because only the two stuck tests script ≥5 identical calls and neither overrides, but any future `counter_agent` caller that both overrides the stack and scripts repeats must re-add `StuckDetectionMiddleware` itself. `repair_turn_leaves_stuck_counters_untouched` must report the same baseline numbers captured in Step 2.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-core/src agent/crates/agent-runtime-config/src/assemble.rs
git commit -m "refactor(core): migrate stuck detection into StuckDetectionMiddleware"
```

---

### Task 7: remaining parity pins + full gate

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` (tests only), `agent/crates/agent-core/src/dispatch.rs` (tests only)

**Interfaces:** consumes everything above; produces only tests.

- [ ] **Step 1: Append the remaining spec-§7 pins** (each is a small test; write, run, adjust to observed-baseline where marked):

```rust
/// Spec §5.4 timeline: validator failure + pending nudge + maintain, in
/// today's exact durable order (tool results → validator msg → nudge).
#[tokio::test]
async fn nudged_turn_with_validator_failure_keeps_message_order() {
    // Build via counter_agent with post_tool_validators = ["false"] (a
    // command that always fails) in LoopConfig, script 3 identical call
    // turns + 1 text turn, then assert the built context's message sequence:
    // …, Assistant(tool_calls), Tool, User(validation), User(nudge), …
    // IMPLEMENTER: run against a pre-Task-6 checkout if unsure of baseline;
    // the order is pinned by loop_.rs:946-1013 (validators, then nudge).
}

/// Spec §5.1: no node hook fires on cancellation or budget exhaustion.
#[tokio::test]
async fn cancel_and_budget_paths_fire_no_hooks() {
    // (a) precancelled token (mirror precancelled_token_stops_before_calling_model,
    //     loop_.rs:2152) with a Recording stack → log is EMPTY except run_start?
    //     No: cancel is checked at turn top AFTER run_start fires. Baseline it:
    //     expected log == ["a:run_start", "b:run_start"] then Done(Cancelled).
    // (b) budget exhaustion (mirror budget_exhaustion_runs_tools_disabled_wrap_up,
    //     loop_.rs:1714) → no final_reply entry in the log; turn hooks fire for
    //     each completed tool turn only.
}

/// Spec §7: no maintain on either Length-stop exit. Mirror
/// truncated_tool_call_reports_max_tokens_not_bad_args (loop_.rs:2753) with
/// a Recording stack + a CuratedContext spy: assert no on_turn_end /
/// after_final_reply fired after the Length return.
#[tokio::test]
async fn length_exit_fires_no_maintain_hooks() { /* per above */ }
```

- [ ] **Step 2: Child-stack invariant pin** (in `dispatch.rs` tests): dispatch a child (reuse an existing dispatch test's harness) with memory tools present in `base_tools`, and assert (a) the child's registered tool names include the memory tool and the context tools, (b) the child ran with a stack of exactly two middleware — expose `#[cfg(test)] pub(crate) fn child_stack_names(&self) -> Vec<&'static str>` or assert via the Recording-in-child-events pattern, whichever the harness supports; the normative claim is "children: `[context-curation, stuck-detection]`, never `memory-recall`".

- [ ] **Step 3: Full workspace + CI gate**

Run: `cd agent && cargo test 2>&1 | tail -5` — full workspace PASS.
Run: `cd /home/kalen/rust-agent-runtime && bash scripts/ci.sh` — green (okf check, skills lint, fmt, clippy, agent tests, conditional src-tauri, web).
Expected: all green. Any clippy warning introduced by the new module is fixed here (no `#[allow]` unless the pattern already exists in the file).

- [ ] **Step 4: Commit**

```bash
git add agent/crates/agent-core/src
git commit -m "test(core): parity pins for hook firing set, message order, and child stacks"
```

- [ ] **Step 5: Spec cross-check.** Re-read spec §7's two test lists; tick each named test as present-and-green. If any is missing, add it now. Then hand off per `superpowers:finishing-a-development-branch`.

---

## Self-Review (done at plan-writing time)

- **Spec coverage:** G1→Tasks 1+3; G2→Task 4 (stack build + contribution passes); G3→Tasks 4/5/6 (one behavior each, deletion gated by pinned tests); G4→Tasks 5/6 dispatch wiring + Task 7 Step 2; G5→every task's Step "run full crate" + Task 7 Step 3. Spec §5.2 orphan debug_assert → Task 2 Step 3b. §5.6 collision debug_assert → Task 4 Step 3. J3 invoked-twice → Task 3 test 2. J5 firing set → Task 2 test 3 + Task 7. Overflow maintain stays resident → Task 5 Step 3 explicitly.
- **Known open shapes (flagged, not placeholders):** two Task 7 tests and Task 3's overflow-wrap test direct the implementer to mirror a NAMED existing test's scripting rather than inline it — the testkit's scripted-overflow/malformed shapes are file-local and must be copied at implementation time from the named line anchors. Baseline-capture steps (Task 6 Step 2) make "same as today" executable.
- **Type consistency:** `with_middleware(Vec<Arc<dyn Middleware>>)` used in Tasks 2/4/5/6/7; `ToolContribution{tool, child_visible}` in 1/4/5; `CompletionFailure`/`Executed` promoted in 3 and consumed nowhere else; `RunCx` accessors used by Task 5 match Task 1's definitions.
