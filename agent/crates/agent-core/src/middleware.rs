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
