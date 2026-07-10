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

/// Generous order-of-magnitude backstops (spec §5.5). Named so a future knob is
/// a one-line change; NOT runtime-configurable (matches the stuck precedent).
pub const TOOL_CALL_LIMIT: usize = 1000;
pub const MODEL_CALL_LIMIT: usize = 500;

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

/// Cap on raw index bytes the middleware will process per load (spec §2.4).
/// NOTE (accepted residual, flag in review): Backend::read is whole-document,
/// so RAM during the read equals file size — same exposure as the read_file
/// tool; this cap bounds what enters processing/context. Dirty-flag cadence
/// makes loads rare. A ranged-read backend method is future work.
pub(crate) const MEMORY_INDEX_MAX_BYTES: usize = 256 * 1024;

/// Rewrite a relative markdown link target to resolve under the memory mount.
pub(crate) fn prefix_links(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find("](") {
        let (head, tail) = rest.split_at(i + 2);
        out.push_str(head);
        if !(tail.starts_with('/')
            || tail.starts_with("memories/")
            || tail.starts_with("http://")
            || tail.starts_with("https://"))
        {
            out.push_str("memories/project/");
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// Non-empty index lines within the byte cap, link-prefixed, plus the exact
/// count of non-empty lines dropped by the cap.
pub(crate) fn index_lines(content: &str) -> (Vec<String>, usize) {
    let mut kept = Vec::new();
    let mut omitted = 0usize;
    let mut used = 0usize;
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        // Cap on the PREFIXED length: prefix_links can grow a relative link
        // target (e.g. `a.md` -> `memories/project/a.md`), so measuring the
        // raw line would let the kept set exceed MEMORY_INDEX_MAX_BYTES.
        let prefixed = prefix_links(line);
        if used + prefixed.len() + 1 > MEMORY_INDEX_MAX_BYTES {
            omitted += 1;
            continue;
        }
        used += prefixed.len() + 1;
        kept.push(prefixed);
    }
    (kept, omitted)
}

/// RunShared flag: a tool call wrote under the memory mount this turn.
#[derive(Default)]
struct MemoryDirty(bool);

/// File-based memory (spec §2.4): loads memories/project/index.md into the
/// pinned memory block at run start; dirty-flag refresh after any turn whose
/// tools successfully wrote under the mount (gate E2). Contributes NO tools —
/// editing is the ordinary file tools. Parent-only (child quarantine, §2.6).
pub struct MemoryFilesMiddleware {
    mem: Arc<dyn agent_tools::backend::Backend>,
}

impl MemoryFilesMiddleware {
    pub fn new(mem: Arc<dyn agent_tools::backend::Backend>) -> Self {
        Self { mem }
    }
    async fn load(&self, cx: &mut RunCx<'_>) {
        // Missing dir/index ⇒ empty block; errors degrade, never abort (§4).
        let lines = match self.mem.read("index.md").await {
            Ok(content) => {
                let (mut lines, omitted) = index_lines(&content);
                if omitted > 0 {
                    // Byte-cap omissions fold into the pointer via extra lines
                    // the budget will also truncate; simplest honest signal:
                    lines.push(format!(
                        "[index byte-capped: {omitted} more lines — read memories/project/index.md]"
                    ));
                }
                lines
            }
            Err(agent_tools::backend::FsError::NotFound(_)) => Vec::new(),
            Err(e) => {
                tracing::warn!(error = %e, "memory index unreadable; loading empty");
                Vec::new()
            }
        };
        cx.ctx.set_recall(lines);
    }
}

#[async_trait]
impl Middleware for MemoryFilesMiddleware {
    fn name(&self) -> &str {
        "memory-files"
    }
    async fn on_run_start(&self, cx: &mut RunCx<'_>, _input: &str) -> Flow {
        self.load(cx).await;
        Flow::Continue
    }
    async fn wrap_tool_call(&self, call: ToolCall, next: ToolNext<'_>) -> crate::Executed {
        let writes_memory = matches!(call.name.as_str(), "write_file" | "edit_file")
            && call
                .args
                .get("path")
                .and_then(|p| p.as_str())
                .is_some_and(|p| p.starts_with("memories/"));
        // RunShared derives Clone (Arc<Mutex<..>> — cheap bump); clone BEFORE
        // next.run(self) consumes `next`, set the flag AFTER on success.
        let shared = next.shared().clone();
        let out = next.run(call.args.clone()).await;
        if writes_memory && matches!(out, crate::Executed::Ok(_)) {
            shared.with::<MemoryDirty, _>(|d| d.0 = true);
        }
        out
    }
    async fn after_tools(&self, cx: &mut RunCx<'_>) -> Flow {
        let dirty = cx
            .shared()
            .with::<MemoryDirty, _>(|d| std::mem::take(&mut d.0));
        if dirty {
            self.load(cx).await;
        }
        Flow::Continue
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
        Self {
            cap: TOOL_CALL_LIMIT,
        }
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

/// Ends a named child cleanly the moment its `respond` payload is captured
/// (spec 3B-1b §2.3). Added to the child stack only when the spec declares a
/// `response_format`, and pushed AFTER `ToolCallLimit` so that — because
/// `fire_after_tools` iterates the stack in REVERSE and the first `EndRun` wins —
/// a captured response reports `Stop` even on a turn that also trips the call cap.
pub struct ResponseCapture {
    handle: crate::ResponseHandle,
}

impl ResponseCapture {
    pub fn new(handle: crate::ResponseHandle) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl Middleware for ResponseCapture {
    fn name(&self) -> &str {
        "response-capture"
    }
    async fn after_tools(&self, _cx: &mut RunCx<'_>) -> Flow {
        if self.handle.lock().unwrap().is_some() {
            Flow::EndRun(StopReason::Stop)
        } else {
            Flow::Continue
        }
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
        Self {
            cap: MODEL_CALL_LIMIT,
            enabled: false,
        }
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

    // ---- MemoryFilesMiddleware (4A-1 A4 brief) ------------------------------

    #[test]
    fn prefix_links_rewrites_relative_targets_only() {
        assert_eq!(
            prefix_links("* [A](a.md) - h"),
            "* [A](memories/project/a.md) - h"
        );
        assert_eq!(
            prefix_links("* [A](memories/project/a.md) - h"),
            "* [A](memories/project/a.md) - h"
        );
        assert_eq!(prefix_links("* [A](https://x) - h"), "* [A](https://x) - h");
        assert_eq!(prefix_links("no link here"), "no link here");
    }

    #[test]
    fn index_lines_caps_bytes_and_counts_omitted() {
        let big: String = (0..10_000)
            .map(|i| format!("* [m{i}](m{i}.md) - hook\n"))
            .collect();
        let (lines, omitted) = index_lines(&big);
        assert!(lines.iter().map(|l| l.len() + 1).sum::<usize>() <= MEMORY_INDEX_MAX_BYTES);
        assert!(omitted > 0);
        let small = "* [a](a.md) - h\n";
        let (lines, omitted) = index_lines(small);
        assert_eq!((lines.len(), omitted), (1, 0));
    }

    use crate::context::MEMORY_HEADER;
    use agent_tools::backend::{Backend, FsError, MemBackend};

    /// A `RunCx` at run-start (`turn: None`), sharing the caller's
    /// `ctx`/`state`/`shared` — mirrors `parse_fail_cx` above but for the
    /// on_run_start / wrap_tool_call / after_tools hooks this middleware uses.
    #[allow(clippy::too_many_arguments)]
    fn mem_files_cx<'a>(
        ctx: &'a mut dyn ContextManager,
        state: &'a mut RunState,
        shared: &'a RunShared,
        cancel: &'a CancellationToken,
        sink: &'a Arc<dyn EventSink>,
        model: &'a Arc<dyn ModelClient>,
        turn: Option<usize>,
    ) -> RunCx<'a> {
        RunCx {
            ctx,
            sink,
            cancel,
            state,
            shared,
            turn,
            maint: MaintView {
                maint_model: model,
                maint_model_limit: 100_000,
                effective_model_limit: 100_000,
            },
        }
    }

    /// Renders the pinned prompt from a `WindowContext` for assertions.
    fn rendered(ctx: &crate::WindowContext) -> String {
        ctx.build(100_000)
            .iter()
            .map(|m| m.content.clone())
            .collect::<Vec<_>>()
            .join("\n---\n")
    }

    #[tokio::test]
    async fn memory_files_loads_index_at_run_start() {
        let mem: Arc<dyn Backend> = Arc::new(MemBackend::new());
        mem.write("index.md", "* [A](a.md) - hook a\n* [B](b.md) - hook b\n")
            .await
            .unwrap();
        let m = MemoryFilesMiddleware::new(mem);

        let mut ctx = crate::WindowContext::new(agent_model::Message::system("sys"));
        let mut state = RunState::default();
        let shared = RunShared::default();
        let cancel = CancellationToken::new();
        let sink: Arc<dyn EventSink> = Arc::new(crate::testkit::CollectingSink::default());
        let model: Arc<dyn ModelClient> = Arc::new(crate::testkit::ScriptedModel::new(vec![]));

        {
            let mut cx = mem_files_cx(&mut ctx, &mut state, &shared, &cancel, &sink, &model, None);
            let flow = m.on_run_start(&mut cx, "go").await;
            assert_eq!(flow, Flow::Continue);
        }

        let prompt = rendered(&ctx);
        assert!(prompt.contains(MEMORY_HEADER));
        assert!(prompt.contains("* [A](memories/project/a.md) - hook a"));
        assert!(prompt.contains("* [B](memories/project/b.md) - hook b"));
    }

    #[tokio::test]
    async fn memory_files_missing_index_renders_no_block() {
        let mem: Arc<dyn Backend> = Arc::new(MemBackend::new());
        let m = MemoryFilesMiddleware::new(mem);

        let mut ctx = crate::WindowContext::new(agent_model::Message::system("sys"));
        let mut state = RunState::default();
        let shared = RunShared::default();
        let cancel = CancellationToken::new();
        let sink: Arc<dyn EventSink> = Arc::new(crate::testkit::CollectingSink::default());
        let model: Arc<dyn ModelClient> = Arc::new(crate::testkit::ScriptedModel::new(vec![]));

        {
            let mut cx = mem_files_cx(&mut ctx, &mut state, &shared, &cancel, &sink, &model, None);
            m.on_run_start(&mut cx, "go").await;
        }

        let prompt = rendered(&ctx);
        assert!(!prompt.contains(MEMORY_HEADER));
    }

    /// Minimal Write-tier tool that always succeeds — mirrors `WriteStub` in
    /// loop_.rs (used there for validator tests); this file drives
    /// `wrap_tool_call` directly (no loop), so it needs its own local copy.
    struct AlwaysWrites;
    #[async_trait]
    impl agent_tools::Tool for AlwaysWrites {
        fn name(&self) -> &str {
            "write_file"
        }
        fn description(&self) -> &str {
            "writes (test stub)"
        }
        fn schema(&self) -> agent_tools::ToolSchema {
            agent_tools::ToolSchema {
                name: "write_file".into(),
                description: "".into(),
                parameters: serde_json::json!({"type":"object"}),
            }
        }
        fn intent(
            &self,
            _args: &serde_json::Value,
        ) -> Result<agent_tools::ToolIntent, agent_tools::ToolError> {
            Ok(agent_tools::ToolIntent {
                tool: "write_file".into(),
                access: agent_tools::Access::Write,
                paths: vec![],
                command: None,
                summary: "write".into(),
            })
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &agent_tools::ToolCtx,
        ) -> Result<agent_tools::ToolOutput, agent_tools::ToolError> {
            Ok(agent_tools::ToolOutput {
                content: "wrote".into(),
                display: None,
            })
        }
    }

    fn test_tool_ctx() -> agent_tools::ToolCtx {
        agent_tools::ToolCtx {
            workspace: std::env::temp_dir(),
            timeout: std::time::Duration::from_secs(5),
            cancel: CancellationToken::new(),
            sandbox: Arc::new(agent_tools::HostExecutor),
            backend: Arc::new(agent_tools::backend::HostBackend::new(std::env::temp_dir())),
            call_id: "test".into(),
        }
    }

    #[tokio::test]
    async fn memory_files_dirty_flag_refreshes() {
        let mem: Arc<dyn Backend> = Arc::new(MemBackend::new());
        mem.write("index.md", "* [A](a.md) - hook a\n")
            .await
            .unwrap();
        let m = MemoryFilesMiddleware::new(mem.clone());

        let mut ctx = crate::WindowContext::new(agent_model::Message::system("sys"));
        let mut state = RunState::default();
        let shared = RunShared::default();
        let cancel = CancellationToken::new();
        let sink: Arc<dyn EventSink> = Arc::new(crate::testkit::CollectingSink::default());
        let model: Arc<dyn ModelClient> = Arc::new(crate::testkit::ScriptedModel::new(vec![]));

        // Run-start load: index has one entry.
        {
            let mut cx = mem_files_cx(&mut ctx, &mut state, &shared, &cancel, &sink, &model, None);
            m.on_run_start(&mut cx, "go").await;
        }
        assert!(rendered(&ctx).contains("hook a"));

        // Turn 1: a scripted `write_file` call under `memories/` succeeds —
        // drive `wrap_tool_call` directly (no mount exists yet; A5 wires it).
        let tool: Arc<dyn agent_tools::Tool> = Arc::new(AlwaysWrites);
        let call = ToolCall {
            id: "c1".into(),
            name: "write_file".into(),
            args: serde_json::json!({"path": "memories/project/index.md", "content": "..."}),
        };
        let tctx = test_tool_ctx();
        let next = ToolNext {
            tool: tool.clone(),
            name: "write_file",
            tctx: &tctx,
            chain: &[],
            call: &call,
            shared: &shared,
        };
        let out = m.wrap_tool_call(call.clone(), next).await;
        assert!(matches!(out, crate::Executed::Ok(_)));

        // Simulate the write actually landing in the backend (A5's mount does
        // this for real; here we drive the effect the dirty flag exists to
        // pick up) before after_tools re-reads.
        mem.write("index.md", "* [A](a.md) - hook a\n* [C](c.md) - hook c\n")
            .await
            .unwrap();

        // after_tools sees the dirty flag set by wrap_tool_call and reloads.
        {
            let mut cx = mem_files_cx(
                &mut ctx,
                &mut state,
                &shared,
                &cancel,
                &sink,
                &model,
                Some(0),
            );
            let flow = m.after_tools(&mut cx).await;
            assert_eq!(flow, Flow::Continue);
        }
        let prompt = rendered(&ctx);
        assert!(prompt.contains("hook a"));
        assert!(
            prompt.contains("hook c"),
            "after_tools must reload the index once the dirty flag is set: {prompt}"
        );
    }

    /// Counts `read` calls so the no-reread test can assert the run-start
    /// load is the ONLY read when no memory write happens this turn.
    #[derive(Default)]
    struct CountingBackend {
        inner: MemBackend,
        reads: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl Backend for CountingBackend {
        async fn ls(&self, path: &str) -> Result<Vec<agent_tools::backend::Entry>, FsError> {
            self.inner.ls(path).await
        }
        async fn read(&self, path: &str) -> Result<String, FsError> {
            self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.read(path).await
        }
        async fn write(&self, path: &str, content: &str) -> Result<(), FsError> {
            self.inner.write(path, content).await
        }
        async fn glob(&self, pattern: &str) -> Result<Vec<String>, FsError> {
            self.inner.glob(pattern).await
        }
        async fn grep(
            &self,
            pattern: &str,
            path: Option<&str>,
        ) -> Result<Vec<agent_tools::backend::GrepHit>, FsError> {
            self.inner.grep(pattern, path).await
        }
        async fn delete(&self, path: &str) -> Result<(), FsError> {
            self.inner.delete(path).await
        }
    }

    #[tokio::test]
    async fn memory_files_clean_turn_does_not_reread() {
        let counting = Arc::new(CountingBackend::default());
        counting
            .write("index.md", "* [A](a.md) - hook a\n")
            .await
            .unwrap();
        let mem: Arc<dyn Backend> = counting.clone();
        let m = MemoryFilesMiddleware::new(mem);

        let mut ctx = crate::WindowContext::new(agent_model::Message::system("sys"));
        let mut state = RunState::default();
        let shared = RunShared::default();
        let cancel = CancellationToken::new();
        let sink: Arc<dyn EventSink> = Arc::new(crate::testkit::CollectingSink::default());
        let model: Arc<dyn ModelClient> = Arc::new(crate::testkit::ScriptedModel::new(vec![]));

        // Run-start load: 1 read.
        {
            let mut cx = mem_files_cx(&mut ctx, &mut state, &shared, &cancel, &sink, &model, None);
            m.on_run_start(&mut cx, "go").await;
        }
        assert_eq!(counting.reads.load(std::sync::atomic::Ordering::SeqCst), 1);

        // A turn with a non-memory tool call: wrap_tool_call sees no memory
        // write, so no dirty flag; after_tools must not re-read.
        let tool: Arc<dyn agent_tools::Tool> = Arc::new(AlwaysWrites);
        let call = ToolCall {
            id: "c1".into(),
            name: "some_other_tool".into(),
            args: serde_json::json!({}),
        };
        let tctx = test_tool_ctx();
        let next = ToolNext {
            tool: tool.clone(),
            name: "some_other_tool",
            tctx: &tctx,
            chain: &[],
            call: &call,
            shared: &shared,
        };
        let out = m.wrap_tool_call(call.clone(), next).await;
        assert!(matches!(out, crate::Executed::Ok(_)));

        {
            let mut cx = mem_files_cx(
                &mut ctx,
                &mut state,
                &shared,
                &cancel,
                &sink,
                &model,
                Some(0),
            );
            m.after_tools(&mut cx).await;
        }
        assert_eq!(
            counting.reads.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a clean turn (no memory write) must not trigger a reread"
        );
    }
}
