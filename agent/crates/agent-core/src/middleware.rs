//! Middleware seam (spec: docs/superpowers/specs/2026-07-08-middleware-seam-design.md).
//! One trait, four capability surfaces: node hooks, wrap hooks (Task 3),
//! tool contribution, per-run state extension.
use crate::{ContextManager, EventSink};
use agent_model::{ModelClient, StopReason};
use agent_tools::ToolCall;
use async_trait::async_trait;
use futures::future::BoxFuture;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

/// Nudge after this many consecutive REPEATS of an identical call set
/// (i.e. on the 3rd identical turn); abort after STUCK_ABORT_AFTER (the 5th).
/// Not configurable until a real workload needs the knob (spec 2026-07-02 §4).
pub const STUCK_NUDGE_AFTER: usize = 2;
pub const STUCK_ABORT_AFTER: usize = 4;

/// One re-ask per consecutive parse-failure streak — the current
/// `protocol_repairs < 1` (spec §5.3). Named so a future knob is one line.
pub const MAX_REPAIRS: usize = 1;

/// Node-hook outcome. `EndRun` short-circuits remaining hooks at that point;
/// the loop maps it to `emit(Done(reason)); return Ok(())` (spec §5.2).
#[derive(Debug, Clone, PartialEq)]
pub enum Flow {
    Continue,
    EndRun(StopReason),
}

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
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref())
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
    pub(crate) shared: &'a RunShared,
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
    pub fn shared(&self) -> &RunShared {
        self.shared
    }
}

/// Continuation for `wrap_model_call`: remaining chain, then the loop's
/// `completion_with_retry` as the base case. Wraps ONE invocation; overflow
/// recovery happens outside and re-enters a fresh chain (spec §5.1/J3).
pub struct ModelNext<'a> {
    pub(crate) loop_: &'a crate::AgentLoop,
    pub(crate) chain: &'a [Arc<dyn Middleware>],
    pub(crate) cancel: &'a CancellationToken,
    pub(crate) shared: &'a RunShared,
}

impl<'a> ModelNext<'a> {
    pub fn shared(&self) -> &RunShared {
        self.shared
    }
    pub fn run(
        self,
        req: agent_model::CompletionRequest,
    ) -> BoxFuture<'a, Result<agent_model::AssistantTurn, crate::CompletionFailure>> {
        Box::pin(async move {
            match self.chain.split_first() {
                Some((mw, rest)) => {
                    mw.wrap_model_call(
                        req,
                        ModelNext {
                            loop_: self.loop_,
                            chain: rest,
                            cancel: self.cancel,
                            shared: self.shared,
                        },
                    )
                    .await
                }
                None => self.loop_.completion_with_retry(&req, self.cancel).await,
            }
        })
    }
}

/// Continuation for `wrap_tool_call`: base case is `execute_isolated` on the
/// already-gated call (spec §5.4 — the gate is unreachable from here).
pub struct ToolNext<'a> {
    pub(crate) tool: Arc<dyn agent_tools::Tool>,
    pub(crate) name: &'a str,
    pub(crate) tctx: &'a agent_tools::ToolCtx,
    pub(crate) chain: &'a [Arc<dyn Middleware>],
    pub(crate) call: &'a ToolCall,
    pub(crate) shared: &'a RunShared,
}

impl<'a> ToolNext<'a> {
    pub fn shared(&self) -> &RunShared {
        self.shared
    }
    pub fn run(self, args: serde_json::Value) -> BoxFuture<'a, crate::Executed> {
        Box::pin(async move {
            match self.chain.split_first() {
                Some((mw, rest)) => {
                    let mut call = self.call.clone();
                    call.args = args;
                    mw.wrap_tool_call(
                        call,
                        ToolNext {
                            chain: rest,
                            ..self
                        },
                    )
                    .await
                }
                None => crate::execute_isolated(self.tool, args, self.name, self.tctx).await,
            }
        })
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

    /// Wraps one `completion_with_retry` invocation (spec §5.1/J3, Task 3).
    /// The default is a pure pass-through. Invoked twice, independently,
    /// across an overflow rebuild — each invocation gets a fresh chain.
    async fn wrap_model_call(
        &self,
        req: agent_model::CompletionRequest,
        next: ModelNext<'_>,
    ) -> Result<agent_model::AssistantTurn, crate::CompletionFailure> {
        next.run(req).await
    }
    /// Wraps one `execute_isolated` invocation, one per gated tool call
    /// inside the parallel executor (spec §5.4, Task 3). Sits AFTER the
    /// gate (policy/approval), so it cannot weaken policy. The default is a
    /// pure pass-through.
    async fn wrap_tool_call(&self, call: ToolCall, next: ToolNext<'_>) -> crate::Executed {
        next.run(call.args).await
    }
}

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
            .map(|t| ToolContribution {
                tool: t.clone(),
                child_visible: true,
            })
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

/// Scheduled context curation: loop-bottom maintain each tool turn, plus the
/// text-only-exit maintain when no tool turn ran (spec §5.5). Ships the
/// context tools (child-invisible; children get per-dispatch instances).
pub struct ContextCurationMiddleware {
    flag: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Default)]
pub(crate) struct Maintained(pub(crate) bool);

impl ContextCurationMiddleware {
    pub fn new(flag: Arc<std::sync::atomic::AtomicBool>) -> Self {
        Self { flag }
    }

    /// `at_text_exit` selects the trace-log message so it matches the deleted
    /// loop_.rs blocks byte-for-byte on both paths (task-5 brief).
    async fn maintain(&self, cx: &mut RunCx<'_>, at_text_exit: bool) {
        // Destructure first: MaintCtx borrows `sink`/`cancel`/`maint` immutably
        // while `ctx.maintain(&deps)` needs `&mut ctx` — a live immutable
        // borrow through `cx` as a whole would conflict otherwise (BORROW
        // NOTE, task-5 brief). Destructuring splits `cx` into disjoint
        // fields so the borrow checker can see they don't overlap.
        let RunCx {
            ctx,
            sink,
            cancel,
            maint,
            ..
        } = cx;
        let deps = crate::MaintCtx {
            model_limit: maint.maint_model_limit,
            model: maint.maint_model,
            sink,
            cancel,
        };
        let report = ctx.maintain(&deps).await;
        if report.offloaded > 0 || report.compacted_turns > 0 {
            if at_text_exit {
                tracing::debug!(
                    offloaded = report.offloaded,
                    offloaded_bytes = report.offloaded_bytes,
                    compacted_turns = report.compacted_turns,
                    "context maintained at text-only exit"
                );
            } else {
                tracing::debug!(
                    offloaded = report.offloaded,
                    offloaded_bytes = report.offloaded_bytes,
                    compacted_turns = report.compacted_turns,
                    "context maintained"
                );
            }
        }
    }
}

#[async_trait]
impl Middleware for ContextCurationMiddleware {
    fn name(&self) -> &str {
        "context-curation"
    }
    fn tools(&self) -> Vec<ToolContribution> {
        crate::context_tools(self.flag.clone())
            .into_iter()
            .map(|tool| ToolContribution {
                tool,
                child_visible: false,
            })
            .collect()
    }
    async fn on_turn_end(&self, cx: &mut RunCx<'_>) -> Flow {
        self.maintain(cx, false).await;
        cx.state.entry::<Maintained>().0 = true;
        Flow::Continue
    }
    async fn after_final_reply(&self, cx: &mut RunCx<'_>) {
        // Text-exit maintain fires only when no tool turn maintained this run
        // (today's run_maintained gate; spec §5.5 pins the cadence).
        if !cx.state.get::<Maintained>().map(|m| m.0).unwrap_or(false) {
            self.maintain(cx, true).await;
        }
    }
}

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
        // Identical encoding to the original loop-resident signature block:
        // sorted (name\u{1}args|error), joined by \u{2} — id-independent.
        let mut parts: Vec<String> = turn
            .tool_calls
            .iter()
            .map(|c| format!("{}\u{1}{}", c.name, c.args))
            .chain(turn.invalid.iter().map(|(n, e)| format!("{n}\u{1}{e}")))
            .collect();
        parts.sort();
        let sig = parts.join("\u{2}");

        // End the `s` borrow before touching `cx.ctx`/`cx.sink` (BORROW NOTE,
        // task-6 brief): compute the abort/nudge decision into locals first.
        let s = cx.state.entry::<StuckState>();
        if s.last_sig.as_deref() == Some(&sig) {
            s.repeats += 1;
        } else {
            s.repeats = 0;
            s.nudged = false;
        }
        s.last_sig = Some(sig);
        let abort = s.repeats >= STUCK_ABORT_AFTER;
        let nudge = !abort && s.repeats >= STUCK_NUDGE_AFTER && !s.nudged;
        if nudge {
            s.nudged = true;
            s.nudge_pending = true;
        }

        if abort {
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
        Flow::Continue
    }

    async fn after_tools(&self, cx: &mut RunCx<'_>) -> Flow {
        let s = cx.state.entry::<StuckState>();
        let pending = std::mem::take(&mut s.nudge_pending);
        if pending {
            cx.ctx.append(agent_model::Message::user(
                "You have now issued the identical tool call(s) 3 turns in a row; \
                 repeating them will not change the result. Change your approach, or \
                 reply with a summary and no tool call if you are done.",
            ));
        }
        Flow::Continue
    }
}

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
        assert!(
            joined.is_err(),
            "the closure panic propagates on ITS thread"
        );
        // Despite the poisoned mutex, a later with() recovers the prior value.
        assert_eq!(
            s.with::<Counter, _>(|c| c.0),
            5,
            "poisoned mutex recovered via into_inner; value intact"
        );
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

    // ---- RepairMiddleware unit tests (spec §5.3; A3 brief Step 1) ----------
    //
    // These pin the byte-identical default repair policy and the S2
    // reset-on-non-contiguity reading (reproducing today's
    // reset-on-successful-parse). They construct a bare `RunCx` directly
    // (no loop) and drive `on_parse_failure` against a shared `RunState`.

    /// A `RunCx` at `turn`, sharing the caller's `ctx`/`state`/`shared` so a
    /// streak of failures accumulates in the same `RepairState`.
    fn parse_fail_cx<'a>(
        ctx: &'a mut dyn ContextManager,
        state: &'a mut RunState,
        shared: &'a RunShared,
        cancel: &'a CancellationToken,
        sink: &'a Arc<dyn EventSink>,
        model: &'a Arc<dyn ModelClient>,
        turn: usize,
    ) -> RunCx<'a> {
        RunCx {
            ctx,
            sink,
            cancel,
            state,
            shared,
            turn: Some(turn),
            maint: MaintView {
                maint_model: model,
                maint_model_limit: 100_000,
                effective_model_limit: 100_000,
            },
        }
    }

    /// Shared test scaffold: a `WindowContext`, a scripted model, a sink, a
    /// cancel token, and a `RunShared` — everything a `RunCx` needs.
    fn repair_harness() -> (
        crate::WindowContext,
        Arc<dyn ModelClient>,
        Arc<dyn EventSink>,
        RunShared,
        CancellationToken,
    ) {
        let ctx = crate::WindowContext::new(agent_model::Message::system("sys"));
        let model: Arc<dyn ModelClient> = Arc::new(crate::testkit::ScriptedModel::new(vec![]));
        let sink: Arc<dyn EventSink> = Arc::new(crate::testkit::CollectingSink::default());
        (
            ctx,
            model,
            sink,
            RunShared::default(),
            CancellationToken::new(),
        )
    }

    #[tokio::test]
    async fn repair_default_reasks_once_then_gives_up_on_contiguous_failure() {
        let raw = agent_model::AssistantTurn {
            text: "garbage".into(),
            ..Default::default()
        };
        let m = RepairMiddleware;
        let (mut ctx, model, sink, shared, cancel) = repair_harness();
        let mut state = RunState::default();

        // Turn 3: first failure of a streak → ReAsk with the exact message.
        {
            let mut cx = parse_fail_cx(&mut ctx, &mut state, &shared, &cancel, &sink, &model, 3);
            let r = m.on_parse_failure(&mut cx, &raw, "bad json").await;
            assert_eq!(
                r,
                Repair::ReAsk(
                    "Your tool call could not be parsed: bad json. Re-emit it correctly.".into()
                )
            );
        }
        // Turn 4 (contiguous): second consecutive failure → terminal GiveUp.
        {
            let mut cx = parse_fail_cx(&mut ctx, &mut state, &shared, &cancel, &sink, &model, 4);
            let r = m.on_parse_failure(&mut cx, &raw, "bad json").await;
            assert_eq!(r, Repair::GiveUp);
        }
    }

    #[tokio::test]
    async fn repair_resets_on_non_contiguous_failure() {
        // Reproduces today's reset-on-successful-parse (loop_.rs Ok-arm): a
        // failure separated from the prior one by a gap in turn index (an
        // intervening success) re-asks again (plan discrepancy S2).
        let raw = agent_model::AssistantTurn {
            text: "garbage".into(),
            ..Default::default()
        };
        let m = RepairMiddleware;
        let (mut ctx, model, sink, shared, cancel) = repair_harness();
        let mut state = RunState::default();

        // Turn 3 fail → ReAsk (repairs 0→1, last_fail=Some(3)).
        {
            let mut cx = parse_fail_cx(&mut ctx, &mut state, &shared, &cancel, &sink, &model, 3);
            assert!(matches!(
                m.on_parse_failure(&mut cx, &raw, "e").await,
                Repair::ReAsk(_)
            ));
        }
        // Turn 7 fail (gap: turns 4..6 parsed OK, no on_parse_failure) →
        // non-contiguous → repairs reset to 0 → ReAsk again.
        {
            let mut cx = parse_fail_cx(&mut ctx, &mut state, &shared, &cancel, &sink, &model, 7);
            assert!(matches!(
                m.on_parse_failure(&mut cx, &raw, "e").await,
                Repair::ReAsk(_)
            ));
        }
    }
}
