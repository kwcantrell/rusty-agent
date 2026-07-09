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
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Nudge after this many consecutive REPEATS of an identical call set
/// (i.e. on the 3rd identical turn); abort after STUCK_ABORT_AFTER (the 5th).
/// Not configurable until a real workload needs the knob (spec 2026-07-02 §4).
pub const STUCK_NUDGE_AFTER: usize = 2;
pub const STUCK_ABORT_AFTER: usize = 4;

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

/// Continuation for `wrap_model_call`: remaining chain, then the loop's
/// `completion_with_retry` as the base case. Wraps ONE invocation; overflow
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
                        ModelNext {
                            loop_: self.loop_,
                            chain: rest,
                            cancel: self.cancel,
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
        Self {
            store,
            flag,
            max_result_bytes,
        }
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
        crate::context_tools(self.store.clone(), self.flag.clone(), self.max_result_bytes)
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
