use crate::{built_tokens, AgentEvent, ContextManager, EventSink, ToolStatus};
use agent_model::{
    AssistantTurn, Chunk, CompletionRequest, ErrorClass, Message, ModelClient, ModelError,
    RawToolCall, StopReason, ToolCallProtocol,
};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse, Decision, PolicyEngine};
use agent_tools::{Tool, ToolCall, ToolCtx, ToolError, ToolRegistry};
use futures::StreamExt;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("model error after retries: {0}")]
    Model(String),
}

/// Default idle timeout for model-stream consumption. Generous enough to cover
/// claude-cli cold-start + `thinking` blocks before the first token.
pub const DEFAULT_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Default bound on how many of a turn's tool calls execute concurrently.
/// A `LoopConfig.max_parallel_tools` of 0 (the `Default`) resolves to this.
pub const DEFAULT_MAX_PARALLEL_TOOLS: usize = 8;

/// Appended when `max_turns` exhausts with the model still issuing tool calls;
/// the run then gets ONE tools-disabled wrap-up completion (best-effort) so it
/// ends on a summary instead of mid-thought (spec: runtime-knobs Part 2).
const BUDGET_WRAP_UP_PROMPT: &str = "The turn limit for this run has been reached and \
tools are disabled for the remainder of this run. Reply with a brief summary of what \
you accomplished, what remains to be done, and any state or next steps the user needs.";

/// Surfaced when a tool call is cut off at `max_tokens` (args are incomplete
/// JSON). Shared by the whole-turn `Err(Length)` repair arm and the per-call
/// `Ok`-with-`invalid` + `Length` guard so a truncated call takes the
/// truncation path, not a per-call "re-emit" that would truncate again.
const LENGTH_TRUNCATION_MSG: &str = "the model reached the max_tokens limit before it \
    finished a tool call (e.g. writing a large file); increase max_tokens in settings \
    and try again";

/// Exponential retry backoff: 100ms · 2^(attempt-1), capped at 5s.
fn backoff_delay(attempt: usize) -> Duration {
    let exp = (attempt.saturating_sub(1)).min(16) as u32; // 100ms << 16 is already > cap
    Duration::from_millis((100u64 << exp).min(5_000))
}

/// `backoff_delay(attempt)` plus additive jitter in `0..=delay/4`, to
/// de-correlate concurrent retriers hitting the same backend. The randomness
/// is advisory (a thundering-herd smoother, not a security primitive), so it
/// is `rand`-free: a hash of the current `Instant`'s nanos. Bound: the result
/// is always in `[backoff_delay, backoff_delay * 1.25]`.
fn jittered_backoff(attempt: usize) -> Duration {
    use std::hash::{Hash, Hasher};
    let base = backoff_delay(attempt);
    let span = base.as_millis() as u64 / 4; // max jitter (25% of base)
    if span == 0 {
        return base;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::time::Instant::now().hash(&mut hasher);
    let jitter = hasher.finish() % (span + 1); // uniform-ish 0..=span
    base + Duration::from_millis(jitter)
}

/// Why `completion_with_retry` gave up. `pub` so the wrap-chain base case in
/// `middleware.rs` can name it (provisional contract, Task 3); the turn loop
/// maps it onto events + `AgentError`.
pub enum CompletionFailure {
    /// Fatal on first sight, or retryable and retries exhausted.
    Fatal(String),
    /// Cancellation observed (token or `ModelError::Cancelled`).
    Cancelled,
    /// Context overflow: the same request can never succeed. Not counted
    /// against max_retries; the turn loop may compact-rebuild-retry once. The
    /// tuple carries the (text, reasoning) chars this attempt streamed before
    /// overflowing, so the turn loop can retract a partial answer before the
    /// once-per-turn rebuild re-streams (spec §2); the second-overflow arm
    /// ignores it (that path is terminal, no further attempt).
    Overflow(String, (usize, usize)),
}

#[derive(Clone)]
pub struct LoopConfig {
    pub model_limit: usize,
    pub max_turns: usize,
    pub max_retries: usize,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    pub workspace: PathBuf,
    pub tool_timeout: Duration,
    /// Max time with no stream progress (stream-open or inter-chunk) before a turn
    /// is treated as a stalled-backend `ModelError::Timeout`.
    pub stream_idle_timeout: Duration,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub min_p: Option<f32>,
    pub presence_penalty: Option<f32>,
    pub repeat_penalty: Option<f32>,
    pub enable_thinking: bool,
    pub preserve_thinking: bool,
    pub sandbox: std::sync::Arc<dyn agent_tools::SandboxStrategy>,
    /// Max tool calls from one assistant turn to execute concurrently.
    /// 0 (the default) means `DEFAULT_MAX_PARALLEL_TOOLS`.
    pub max_parallel_tools: usize,
    /// Shell commands run after a mutating turn (see RuntimeConfig). Empty = off.
    pub post_tool_validators: Vec<String>,
    /// Declared context window of a routed compaction model; None = same as
    /// `model_limit`. Maintenance targets min(model window, this) — a span the
    /// compactor cannot read cannot be evicted (finding 4.2).
    pub compaction_model_limit: Option<usize>,
}

impl Default for LoopConfig {
    /// Test convenience only — production wiring (`assemble_loop` →
    /// `loop_config_from`) sets every field explicitly, `sandbox` included.
    /// The default sandbox is an explicit `HostExecutor`: the same posture
    /// `sandbox_mode: "off"` selects, never a silent fallback at gate time.
    fn default() -> Self {
        Self {
            model_limit: 0,
            max_turns: 0,
            max_retries: 0,
            temperature: 0.0,
            max_tokens: None,
            workspace: PathBuf::new(),
            tool_timeout: Duration::default(),
            stream_idle_timeout: Duration::default(),
            top_p: None,
            top_k: None,
            min_p: None,
            presence_penalty: None,
            repeat_penalty: None,
            enable_thinking: false,
            preserve_thinking: false,
            sandbox: std::sync::Arc::new(agent_tools::HostExecutor),
            max_parallel_tools: 0,
            post_tool_validators: Vec::new(),
            compaction_model_limit: None,
        }
    }
}

pub struct AgentLoop {
    model: Arc<dyn ModelClient>,
    protocol: Arc<dyn ToolCallProtocol>,
    tools: Arc<ToolRegistry>,
    policy: Arc<dyn PolicyEngine>,
    approval: Arc<dyn ApprovalChannel>,
    sink: Arc<dyn EventSink>,
    config: LoopConfig,
    compaction_model: Option<Arc<dyn ModelClient>>,
    /// Observed (server prompt_tokens / chars-4 estimate) ratio, EMA-smoothed,
    /// fixed-point micros. 1_000_000 = 1.0 = uncalibrated. Shrink-only: clamped
    /// to [1.0, 4.0] and applied as a divisor on model_limit (spec 2026-07-02
    /// server-usage-calibrated budgeting).
    calib_ratio_micros: std::sync::atomic::AtomicU64,
    /// Node-hook/wrap middleware stack (spec middleware-seam §5). Empty by
    /// default: every dispatch point is then a no-op and behavior is
    /// bit-identical to pre-middleware runs.
    middleware: Vec<Arc<dyn crate::Middleware>>,
    /// The virtual filesystem handed to tools via `ToolCtx` (spec §5.3).
    /// Default (see `new`): a `HostBackend` rooted at `config.workspace` —
    /// bare-loop parity with pre-backend-seam behavior.
    backend: Arc<dyn agent_tools::backend::Backend>,
    /// Park-point checkpointer (spec 4B-1). None (default) ⇒ the loop is
    /// byte-identical to pre-checkpoint behavior: zero checkpoint I/O (E1).
    checkpointer: Option<Arc<crate::Checkpointer>>,
}

impl AgentLoop {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: Arc<dyn ModelClient>,
        protocol: Arc<dyn ToolCallProtocol>,
        tools: Arc<ToolRegistry>,
        policy: Arc<dyn PolicyEngine>,
        approval: Arc<dyn ApprovalChannel>,
        sink: Arc<dyn EventSink>,
        config: LoopConfig,
    ) -> Self {
        let backend: Arc<dyn agent_tools::backend::Backend> = Arc::new(
            agent_tools::backend::HostBackend::new(config.workspace.clone()),
        );
        Self {
            model,
            protocol,
            tools,
            policy,
            approval,
            sink,
            config,
            compaction_model: None,
            calib_ratio_micros: std::sync::atomic::AtomicU64::new(1_000_000),
            middleware: Vec::new(),
            backend,
            checkpointer: None,
        }
    }

    /// The live sandbox posture (cached; never re-probes Docker).
    pub fn sandbox_descriptor(&self) -> agent_tools::SandboxDescriptor {
        self.config.sandbox.describe()
    }

    /// The registered tool schemas (read-only; the architecture viewer's tool list).
    pub fn tool_schemas(&self) -> Vec<agent_tools::ToolSchema> {
        self.tools.schemas()
    }

    /// Re-derive a tool's intent from stored args (resume display path —
    /// spec §3.4: what the human sees is never a trusted stored string).
    /// `None` when the named tool is not registered under the current config.
    pub fn derive_intent(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Option<agent_tools::ToolIntent> {
        self.tools.get(name)?.intent(args).ok()
    }

    /// Route context compaction to a (typically cheaper) dedicated model
    /// (spec 2026-07-02 sub-spec #3, G4). None = the session model.
    pub fn with_compaction_model(mut self, model: Arc<dyn ModelClient>) -> Self {
        self.compaction_model = Some(model);
        self
    }

    /// Replace the virtual filesystem the loop hands to tools (spec §5.3).
    /// Default: a HostBackend rooted at `config.workspace` — bare-loop parity.
    pub fn with_backend(mut self, backend: Arc<dyn agent_tools::backend::Backend>) -> Self {
        self.backend = backend;
        self
    }

    /// Install the park-point checkpointer (spec 4B-1). Default: None — zero
    /// checkpoint I/O, byte-identical to pre-checkpoint behavior (E1).
    pub fn with_checkpointer(mut self, ck: Arc<crate::Checkpointer>) -> Self {
        self.checkpointer = Some(ck);
        self
    }

    /// Install the middleware stack (spec §5.4 ordering contract).
    ///
    /// WARNING: this REPLACES any prior stack — it does not append. An empty
    /// stack means NO recall injection, NO scheduled curation, and NO stuck
    /// detection: those behaviors used to live in the loop itself and now
    /// arrive only via middleware on this stack. Production wiring is
    /// `assemble_loop`; call sites that build their own stack (tests
    /// included) must include every middleware they need.
    pub fn with_middleware(mut self, stack: Vec<Arc<dyn crate::Middleware>>) -> Self {
        self.middleware = stack;
        self
    }

    /// The model that serves maintenance (compaction) completions.
    fn maint_model(&self) -> &Arc<dyn ModelClient> {
        self.compaction_model.as_ref().unwrap_or(&self.model)
    }

    /// The configured window shrunk by the observed estimate-undercount ratio
    /// (server prompt_tokens vs chars/4 estimate). Never exceeds the configured
    /// limit; floor at 1/4 of it. Keeps chars/4 as the per-message currency while
    /// making the *budget* honest (audit Spine B #2).
    fn effective_model_limit(&self) -> usize {
        let ratio = self
            .calib_ratio_micros
            .load(std::sync::atomic::Ordering::Relaxed) as f64
            / 1e6;
        ((self.config.model_limit as f64 / ratio) as usize).max(1)
    }

    /// The window maintenance targets: the calibrated loop window, further
    /// capped by a routed compaction model's declared window (finding 4.2).
    fn maint_model_limit(&self) -> usize {
        match self.config.compaction_model_limit {
            Some(cl) => self.effective_model_limit().min(cl),
            None => self.effective_model_limit(),
        }
    }

    /// Precompute the read-only maintenance view for one hook point.
    fn maint_view(&self) -> crate::MaintView<'_> {
        crate::MaintView {
            maint_model: self.maint_model(),
            maint_model_limit: self.maint_model_limit(),
            effective_model_limit: self.effective_model_limit(),
        }
    }

    // Node-hook dispatch (spec §5.1/§5.4). A generic HRTB-closure dispatcher
    // was tried first; the `for<'c> FnMut(&'c .., &'c mut RunCx<'_>) ->
    // BoxFuture<'c, _>` shape fights the borrow checker as soon as the
    // closure captures anything borrowed from the caller's stack (the
    // captured lifetime can't unify with the HRTB `'c`) — confirmed on both
    // `on_run_start` (borrows `user_input`) and `after_model` (borrows
    // `turn_view`). Falling back to five small concrete methods per the
    // brief's noted fallback: each is the same reborrow-per-iteration loop,
    // inlined. Positions and ordering are the contract, not the dispatcher's
    // genericity.

    /// Bottom-of-loop invariant check shared by all five call-outs: no
    /// middleware may leave an orphaned tool_calls message (spec §5.2).
    fn assert_no_orphans(ctx: &dyn ContextManager, mw_name: &str) {
        debug_assert!(
            crate::orphaned_tool_positions(&ctx.build(usize::MAX)).is_empty(),
            "middleware {mw_name} left an orphaned tool_calls message"
        );
    }

    /// Forward order (stack order): before the goal is set / user message
    /// appended.
    async fn fire_run_start(
        &self,
        ctx: &mut dyn ContextManager,
        state: &mut crate::RunState,
        cancel: &CancellationToken,
        input: &str,
        shared: &crate::RunShared,
    ) -> crate::Flow {
        for mw in self.middleware.iter() {
            let mut cx = crate::RunCx {
                ctx: &mut *ctx,
                sink: &self.sink,
                cancel,
                state: &mut *state,
                turn: None,
                maint: self.maint_view(),
                shared,
            };
            let flow = mw.on_run_start(&mut cx, input).await;
            Self::assert_no_orphans(ctx, mw.name());
            if let crate::Flow::EndRun(reason) = flow {
                return crate::Flow::EndRun(reason);
            }
        }
        crate::Flow::Continue
    }

    /// Reverse order: between id normalization and the assistant append.
    async fn fire_after_model(
        &self,
        ctx: &mut dyn ContextManager,
        state: &mut crate::RunState,
        cancel: &CancellationToken,
        turn: usize,
        turn_view: &crate::TurnView,
        shared: &crate::RunShared,
    ) -> crate::Flow {
        for mw in self.middleware.iter().rev() {
            let mut cx = crate::RunCx {
                ctx: &mut *ctx,
                sink: &self.sink,
                cancel,
                state: &mut *state,
                turn: Some(turn),
                maint: self.maint_view(),
                shared,
            };
            let flow = mw.after_model(&mut cx, turn_view).await;
            Self::assert_no_orphans(ctx, mw.name());
            if let crate::Flow::EndRun(reason) = flow {
                return crate::Flow::EndRun(reason);
            }
        }
        crate::Flow::Continue
    }

    /// Reverse order: after the turn's tool results (and any post-validator
    /// message).
    async fn fire_after_tools(
        &self,
        ctx: &mut dyn ContextManager,
        state: &mut crate::RunState,
        cancel: &CancellationToken,
        turn: usize,
        shared: &crate::RunShared,
    ) -> crate::Flow {
        for mw in self.middleware.iter().rev() {
            let mut cx = crate::RunCx {
                ctx: &mut *ctx,
                sink: &self.sink,
                cancel,
                state: &mut *state,
                turn: Some(turn),
                maint: self.maint_view(),
                shared,
            };
            let flow = mw.after_tools(&mut cx).await;
            Self::assert_no_orphans(ctx, mw.name());
            if let crate::Flow::EndRun(reason) = flow {
                return crate::Flow::EndRun(reason);
            }
        }
        crate::Flow::Continue
    }

    /// Reverse order: bottom of a completed tool turn.
    async fn fire_turn_end(
        &self,
        ctx: &mut dyn ContextManager,
        state: &mut crate::RunState,
        cancel: &CancellationToken,
        turn: usize,
        shared: &crate::RunShared,
    ) -> crate::Flow {
        for mw in self.middleware.iter().rev() {
            let mut cx = crate::RunCx {
                ctx: &mut *ctx,
                sink: &self.sink,
                cancel,
                state: &mut *state,
                turn: Some(turn),
                maint: self.maint_view(),
                shared,
            };
            let flow = mw.on_turn_end(&mut cx).await;
            Self::assert_no_orphans(ctx, mw.name());
            if let crate::Flow::EndRun(reason) = flow {
                return crate::Flow::EndRun(reason);
            }
        }
        crate::Flow::Continue
    }

    /// Reverse order: only on the text-only exit path; no `Flow` (the run is
    /// already ending).
    async fn fire_final_reply(
        &self,
        ctx: &mut dyn ContextManager,
        state: &mut crate::RunState,
        cancel: &CancellationToken,
        turn: usize,
        shared: &crate::RunShared,
    ) {
        for mw in self.middleware.iter().rev() {
            let mut cx = crate::RunCx {
                ctx: &mut *ctx,
                sink: &self.sink,
                cancel,
                state: &mut *state,
                turn: Some(turn),
                maint: self.maint_view(),
                shared,
            };
            mw.after_final_reply(&mut cx).await;
            Self::assert_no_orphans(ctx, mw.name());
        }
    }

    /// Reverse order: on a total tool-call parse failure. The FIRST middleware
    /// returning `ReAsk` wins and short-circuits; all `GiveUp` (or an empty
    /// stack) yields today's terminal give-up. `raw` is a borrow of the turn
    /// the loop still holds (consumed only on the branches after this returns).
    #[allow(clippy::too_many_arguments)]
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

    /// Fold one completed request's ground-truth prompt_tokens against our chars/4
    /// estimate into the EMA ratio (alpha 0.5), clamped to [1.0, 4.0] so the
    /// effective limit only ever shrinks the configured window (spec §1). A backend
    /// that reports no usage (`prompt_tokens == 0`) leaves the ratio untouched.
    fn record_calibration_sample(&self, server_prompt_tokens: u32, est: usize) {
        if server_prompt_tokens == 0 || est == 0 {
            return;
        }
        let sample = server_prompt_tokens as f64 / est as f64;
        let _ = self.calib_ratio_micros.fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |old| {
                let old_f = old as f64 / 1e6;
                let new_f = (0.5 * old_f + 0.5 * sample).clamp(1.0, 4.0);
                if (new_f - old_f).abs() / old_f > 0.05 {
                    tracing::debug!(
                        old = old_f,
                        new = new_f,
                        "token-estimate calibration shifted"
                    );
                }
                Some((new_f * 1e6) as u64)
            },
        );
    }

    /// Drive one streamed completion to an `AssistantTurn`, emitting tokens as they arrive.
    ///
    /// `emitted` accumulates (text chars, reasoning chars) actually pushed to the
    /// sink this attempt, so on an error return the caller knows what partial
    /// output leaked and can retract it before a retry re-streams (spec §2). The
    /// caller resets it per attempt.
    async fn one_completion(
        &self,
        req: CompletionRequest,
        cancel: &CancellationToken,
        emitted: &mut (usize, usize),
    ) -> Result<AssistantTurn, ModelError> {
        let idle = self.config.stream_idle_timeout;
        let mut stream = tokio::select! {
            _ = cancel.cancelled() => return Err(ModelError::Cancelled),
            opened = tokio::time::timeout(idle, self.model.stream(req)) => match opened {
                Err(_) => return Err(ModelError::Timeout(idle)),
                Ok(opened) => opened?,
            },
        };
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut raw_tool_calls: Vec<RawToolCall> = Vec::new();
        let mut stop = StopReason::Stop;
        let mut usage = (0u32, 0u32);
        let mut usage_details: (Option<u32>, Option<u32>, Option<f64>) = (None, None, None);
        loop {
            let step = tokio::select! {
                _ = cancel.cancelled() => return Err(ModelError::Cancelled),
                s = tokio::time::timeout(idle, stream.next()) => s,
            };
            match step {
                // Stalled: dropping `stream` on return fires kill_on_drop / tears down the connection.
                Err(_) => return Err(ModelError::Timeout(idle)),
                Ok(None) => break,
                Ok(Some(item)) => match item? {
                    Chunk::Text(t) => {
                        self.sink.emit(AgentEvent::Token(t.clone()));
                        emitted.0 += t.chars().count();
                        text.push_str(&t);
                    }
                    Chunk::Reasoning(r) => {
                        self.sink.emit(AgentEvent::Reasoning(r.clone()));
                        emitted.1 += r.chars().count();
                        reasoning.push_str(&r);
                    }
                    Chunk::ToolCallDelta(rc) => merge_tool_call(&mut raw_tool_calls, rc),
                    Chunk::Done(r) => stop = r,
                    Chunk::Usage {
                        prompt_tokens,
                        completion_tokens,
                        reasoning_tokens,
                        cached_tokens,
                        cost_usd,
                    } => {
                        usage = (prompt_tokens, completion_tokens);
                        usage_details = (reasoning_tokens, cached_tokens, cost_usd);
                    }
                },
            }
        }
        Ok(AssistantTurn {
            text,
            raw_tool_calls,
            stop,
            reasoning,
            prompt_tokens: usage.0,
            completion_tokens: usage.1,
            reasoning_tokens: usage_details.0,
            cached_tokens: usage_details.1,
            cost_usd: usage_details.2,
        })
    }

    /// Stream with classified retry: transient errors retry with exponential
    /// backoff; permanent request errors fail on first sight; context
    /// overflow is deferred to the turn loop (retrying verbatim cannot help).
    /// `pub(crate)`: the base case of the model wrap chain in
    /// `middleware.rs::ModelNext::run` calls this directly (Task 3).
    pub(crate) async fn completion_with_retry(
        &self,
        base: &CompletionRequest,
        cancel: &CancellationToken,
    ) -> Result<AssistantTurn, CompletionFailure> {
        let mut attempt = 0;
        loop {
            let mut req = base.clone();
            self.protocol.prepare(&mut req);
            // Chars this attempt streamed to the sink; feeds the StreamRetry
            // retraction when another attempt follows a partial (spec §2).
            let mut emitted = (0usize, 0usize);
            match self.one_completion(req, cancel, &mut emitted).await {
                Ok(turn) => return Ok(turn),
                Err(ModelError::Cancelled) => return Err(CompletionFailure::Cancelled),
                Err(e) => {
                    if cancel.is_cancelled() {
                        return Err(CompletionFailure::Cancelled);
                    }
                    match e.class() {
                        ErrorClass::ContextOverflow => {
                            tracing::warn!(error = %e,
                                "context overflow; deferring to turn-level recovery");
                            // Defer retraction to the turn loop: only its FIRST
                            // overflow arm re-attempts; a second overflow is terminal.
                            return Err(CompletionFailure::Overflow(e.to_string(), emitted));
                        }
                        ErrorClass::Fatal => {
                            self.sink.emit(AgentEvent::Error(e.to_string()));
                            return Err(CompletionFailure::Fatal(e.to_string()));
                        }
                        ErrorClass::Retryable => {
                            attempt += 1;
                            if attempt > self.config.max_retries {
                                self.sink.emit(AgentEvent::Error(e.to_string()));
                                return Err(CompletionFailure::Fatal(e.to_string()));
                            }
                            // Another attempt follows: retract any partial output this
                            // attempt already streamed, before the backoff sleep and
                            // the fresh stream (spec §2). Skip when nothing leaked.
                            if emitted != (0, 0) {
                                self.sink.emit(AgentEvent::StreamRetry {
                                    discarded_text_chars: emitted.0,
                                    discarded_reasoning_chars: emitted.1,
                                });
                            }
                            // Honor a server-sent Retry-After (integer seconds,
                            // capped at 30s) when present, but never sleep less
                            // than the jittered exponential backoff.
                            let retry_after = match &e {
                                ModelError::Status { retry_after, .. } => *retry_after,
                                _ => None,
                            };
                            let delay = match retry_after {
                                Some(secs) => {
                                    jittered_backoff(attempt).max(Duration::from_secs(secs.min(30)))
                                }
                                None => jittered_backoff(attempt),
                            };
                            tracing::warn!(attempt, error = %e, "model error, retrying");
                            tokio::time::sleep(delay).await;
                        }
                    }
                }
            }
        }
    }

    /// One place a built message list becomes a CompletionRequest (the turn
    /// prologue and the overflow-recovery rebuild must not drift apart).
    fn completion_request(
        &self,
        messages: Vec<Message>,
        preserve_thinking: bool,
    ) -> CompletionRequest {
        CompletionRequest {
            messages,
            tools: self.tools.schemas(),
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
            top_p: self.config.top_p,
            top_k: self.config.top_k,
            min_p: self.config.min_p,
            presence_penalty: self.config.presence_penalty,
            repeat_penalty: self.config.repeat_penalty,
            enable_thinking: self.config.enable_thinking,
            preserve_thinking,
        }
    }

    /// Convenience entry point with no external cancel source (server + tests).
    /// Live cancellation goes through [`Self::run_with_cancel`].
    pub async fn run(
        &self,
        ctx: &mut dyn ContextManager,
        user_input: String,
    ) -> Result<(), AgentError> {
        self.run_with_cancel(ctx, user_input, CancellationToken::new())
            .await
    }

    pub async fn run_with_cancel(
        &self,
        ctx: &mut dyn ContextManager,
        user_input: String,
        cancel: CancellationToken,
    ) -> Result<(), AgentError> {
        // Surface a degraded sandbox loudly at run start. While degraded,
        // exec-capable tools (execute_command, git, MCP spawns) are REFUSED
        // rather than run unconfined on the host. The per-approval posture
        // string carries this too, but a run may never hit an approval
        // prompt — emit it unconditionally, once, here.
        let d = self.sandbox_descriptor();
        if let Some(reason) = d.degraded {
            self.sink.emit(AgentEvent::SandboxDegraded {
                mechanism: d.mechanism,
                reason,
            });
        }

        // Record the run's inputs for trace replay / eval harvest (audit 6.1).
        // Full user input + composed system prompt, every run (owner call — no
        // dedup). Never a wire frame; ObservedSink writes it to the trace.
        self.sink.emit(AgentEvent::RunStart {
            input: user_input.clone(),
            system: ctx.system().map(|m| m.content.clone()),
        });

        // Per-run middleware state (spec §5.3): fresh for every invocation so
        // middleware stay stateless `&self` objects. Empty `self.middleware`
        // makes every `fire_*` hook method below a no-op loop (behavior unchanged).
        let mut mw_state = crate::RunState::default();
        // Per-run wrap/node shared state (spec §5.2), fresh per invocation so
        // middleware stay stateless. Empty stacks never touch it.
        let run_shared = crate::RunShared::default();
        let flow = self
            .fire_run_start(ctx, &mut mw_state, &cancel, &user_input, &run_shared)
            .await;
        if let crate::Flow::EndRun(reason) = flow {
            self.sink.emit(AgentEvent::Done(reason));
            return Ok(());
        }

        ctx.set_goal(user_input.clone());
        ctx.append(Message::user(user_input));

        // Agentic (tool-bearing) runs auto-preserve reasoning so the model keeps
        // its chain-of-thought across the within-turn tool loop; each backend then
        // decides how to surface it (Qwen3.6 via reasoning_content + the kwarg;
        // claude_cli inline). Plain config still controls the tool-less case.
        let preserve_thinking = self.config.preserve_thinking || !self.tools.schemas().is_empty();

        self.turn_loop(
            ctx,
            &mut mw_state,
            &run_shared,
            cancel,
            0,
            preserve_thinking,
        )
        .await
    }

    /// The per-turn model→tool loop plus the budget wrap-up epilogue — moved
    /// verbatim from `run_with_cancel`. `run_with_cancel` is now prologue +
    /// `turn_loop(.., 0, ..)`; Task 8 re-enters here with a non-zero start.
    #[allow(clippy::too_many_arguments)]
    async fn turn_loop(
        &self,
        ctx: &mut dyn ContextManager,
        mw_state: &mut crate::RunState,
        run_shared: &crate::RunShared,
        cancel: CancellationToken,
        start_turn: usize,
        preserve_thinking: bool,
    ) -> Result<(), AgentError> {
        for turn in start_turn..self.config.max_turns {
            if cancel.is_cancelled() {
                self.sink.emit(AgentEvent::Done(StopReason::Cancelled));
                return Ok(());
            }
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
            let messages = ctx.build(self.effective_model_limit());
            // The chars/4 estimate for the request actually sent; reused for both the
            // Usage event and the post-completion calibration sample. The
            // overflow-recovery rebuild reassigns it so the sample always matches the
            // FINAL request (spec §1).
            let mut est_prompt_tokens = built_tokens(&messages);
            self.sink.emit(AgentEvent::Usage {
                prompt_tokens: est_prompt_tokens,
                context_limit: self.config.model_limit,
                turn: turn + 1,
                max_turns: self.config.max_turns,
            });
            let mut base = self.completion_request(messages, preserve_thinking);
            let turn_started = std::time::Instant::now();
            let mut overflow_recovered = false;
            let assistant = loop {
                let model_result = crate::ModelNext {
                    loop_: self,
                    chain: &self.middleware,
                    cancel: &cancel,
                    shared: run_shared,
                }
                .run(base.clone())
                .await;
                match model_result {
                    Ok(t) => break t,
                    Err(CompletionFailure::Cancelled) => {
                        self.sink.emit(AgentEvent::Done(StopReason::Cancelled));
                        return Ok(());
                    }
                    Err(CompletionFailure::Overflow(_, emitted)) if !overflow_recovered => {
                        overflow_recovered = true;
                        // A partial answer streamed before the overflow is abandoned
                        // by the compaction rebuild; retract it before the rebuilt
                        // attempt re-streams (spec §2). Skip when nothing leaked.
                        if emitted != (0, 0) {
                            self.sink.emit(AgentEvent::StreamRetry {
                                discarded_text_chars: emitted.0,
                                discarded_reasoning_chars: emitted.1,
                            });
                        }
                        tracing::warn!("context overflow: forcing compaction and rebuilding once");
                        self.sink
                            .emit(AgentEvent::Context(crate::ContextEvent::OverflowRecovery));
                        ctx.request_compaction();
                        let deps = crate::MaintCtx {
                            model_limit: self.maint_model_limit(),
                            model: self.maint_model(),
                            sink: &self.sink,
                            cancel: &cancel,
                        };
                        ctx.maintain(&deps).await;
                        let messages = ctx.build(self.effective_model_limit());
                        // The pre-request Usage is stale after compaction; re-emit so
                        // every surface sees the rebuilt request's estimate (latest wins).
                        // Reassign the calibration denominator to this FINAL request.
                        est_prompt_tokens = built_tokens(&messages);
                        self.sink.emit(AgentEvent::Usage {
                            prompt_tokens: est_prompt_tokens,
                            context_limit: self.config.model_limit,
                            turn: turn + 1,
                            max_turns: self.config.max_turns,
                        });
                        base = self.completion_request(messages, preserve_thinking);
                    }
                    Err(CompletionFailure::Overflow(msg, _)) => {
                        // Second overflow in the turn — terminal, no further attempt,
                        // so no StreamRetry (the partial stays; Done explains).
                        self.sink.emit(AgentEvent::Error(msg.clone()));
                        self.sink.emit(AgentEvent::Done(StopReason::Error));
                        return Err(AgentError::Model(msg));
                    }
                    Err(CompletionFailure::Fatal(msg)) => {
                        self.sink.emit(AgentEvent::Done(StopReason::Error));
                        return Err(AgentError::Model(msg));
                    }
                }
            };
            self.sink.emit(AgentEvent::ServerUsage {
                prompt_tokens: assistant.prompt_tokens,
                completion_tokens: assistant.completion_tokens,
                reasoning_tokens: assistant.reasoning_tokens,
                cached_tokens: assistant.cached_tokens,
                cost_usd: assistant.cost_usd,
                turn_duration_ms: turn_started.elapsed().as_millis() as u64,
                turn: turn + 1,
                parent_id: None,
            });
            // Fold the server's ground-truth prompt_tokens for the request just sent
            // against our estimate into the shrink-only calibration ratio (spec §1).
            self.record_calibration_sample(assistant.prompt_tokens, est_prompt_tokens);

            let mut parsed = match self.protocol.parse(&assistant) {
                Ok(p) => p,
                // The completion was cut off at `max_tokens` mid-tool-call (e.g.
                // writing a large file), so the args are incomplete JSON. A
                // "re-emit it correctly" repair is futile — it truncates again at
                // the same limit — so surface the real cause instead of a cryptic
                // JSON parse error.
                Err(_) if assistant.stop == StopReason::Length => {
                    self.sink
                        .emit(AgentEvent::Error(LENGTH_TRUNCATION_MSG.into()));
                    self.sink.emit(AgentEvent::Done(StopReason::Length));
                    return Ok(());
                }
                Err(e) => {
                    // Total parse failure: consult the stack (spec §5.1). The
                    // Length short-circuit above already handled max_tokens
                    // truncation; the second Length guard below is downstream.
                    let err_str = e.to_string();
                    match self
                        .fire_on_parse_failure(
                            ctx,
                            &mut *mw_state,
                            run_shared,
                            &cancel,
                            turn,
                            &assistant,
                            &err_str,
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
            };

            // A call truncated by max_tokens must take the truncation path, not a
            // per-call "re-emit" error that would truncate again (spec §3). The
            // native protocol yields Ok-with-`invalid` here (not `Err`), so this
            // guard mirrors the `Err(_) if Length` arm above for that shape.
            if !parsed.invalid.is_empty() && assistant.stop == StopReason::Length {
                self.sink
                    .emit(AgentEvent::Error(LENGTH_TRUNCATION_MSG.into()));
                self.sink.emit(AgentEvent::Done(StopReason::Length));
                return Ok(());
            }

            // Enforce the per-call id invariant for EVERY protocol before the ids
            // feed the assistant message and the Phase-3 tool-result drain. Invalid
            // (unparseable) calls participate in uniqueness too — each becomes its
            // own tool message and needs a distinct id against the valid calls.
            normalize_tool_call_ids(&mut parsed.tool_calls);
            normalize_invalid_ids(&parsed.tool_calls, &mut parsed.invalid);

            // Node hook: after_model (spec §5.1). OWNED TurnView so no borrow of
            // `parsed` crosses the hook `.await` (spec §5.2).
            let turn_view = crate::TurnView {
                text: parsed.text.clone(),
                tool_calls: parsed.tool_calls.clone(),
                invalid: parsed
                    .invalid
                    .iter()
                    .map(|i| (i.name.clone(), i.error.clone()))
                    .collect(),
            };
            let flow = self
                .fire_after_model(ctx, &mut *mw_state, &cancel, turn, &turn_view, run_shared)
                .await;
            if let crate::Flow::EndRun(reason) = flow {
                self.sink.emit(AgentEvent::Done(reason));
                return Ok(());
            }

            // The assistant message must carry ALL ids (valid + invalid) so every
            // tool message keeps a matching parent call; invalid calls carry `{}`
            // args since their real args could not be parsed.
            let mut all_calls = parsed.tool_calls.clone();
            all_calls.extend(parsed.invalid.iter().map(|inv| ToolCall {
                id: inv.id.clone(),
                name: inv.name.clone(),
                args: serde_json::json!({}),
            }));

            let mut msg = Message::assistant(
                parsed.text.clone(),
                if all_calls.is_empty() {
                    None
                } else {
                    Some(all_calls.clone())
                },
            );
            // Preserve reasoning as data, not inline text — the model backend
            // decides how to render it (claude_cli inlines <think>; openai sends
            // reasoning_content for Qwen3.6). Gated by the effective flag above.
            if preserve_thinking && !assistant.reasoning.is_empty() {
                msg = msg.with_reasoning(assistant.reasoning.clone());
            }
            ctx.append(msg);

            if all_calls.is_empty() {
                // End-of-run curation for PURE text-only runs now lives in
                // ContextCurationMiddleware::after_final_reply (spec §5.5):
                // it fires only when no tool turn's on_turn_end already
                // maintained this run (the Maintained run-state marker).
                //
                // Node hook: after_final_reply (spec §5.1) — text-only exit only
                // (spec §6 J5); no Flow, the run is already ending.
                self.fire_final_reply(ctx, &mut *mw_state, &cancel, turn, run_shared)
                    .await;

                self.sink.emit(AgentEvent::Done(assistant.stop));
                return Ok(());
            }

            // (id, name, error) triples for the unparseable calls — re-seeded as
            // per-call ERROR results inside tool_phase (Phase 1).
            let invalid: Vec<crate::InvalidParked> = parsed
                .invalid
                .iter()
                .map(|i| crate::InvalidParked {
                    id: i.id.clone(),
                    name: i.name.clone(),
                    error: i.error.clone(),
                })
                .collect();
            match self
                .tool_phase(
                    ctx,
                    &mut *mw_state,
                    run_shared,
                    &cancel,
                    turn,
                    parsed.text.clone(),
                    parsed.tool_calls,
                    invalid,
                    None,
                )
                .await
            {
                TurnFlow::Continue => {}
                TurnFlow::End(r) => return r,
            }
        }
        // Budget exhausted with the model still tool-hungry (text-only replies
        // exit earlier with Done(Stop)). One best-effort, tools-disabled wrap-up
        // completion; it must never fail the run. Single attempt by design: no
        // retry, no overflow recovery, no StreamRetry accounting (spec Part 2).
        if !cancel.is_cancelled() {
            // Finding 4.3: inject the wrap-up instruction into THIS request only.
            // Durable history never sees it — appended, it would survive into
            // later runs of the session as a stale "tools are disabled" claim
            // (models measurably imitate such visible history patterns).
            // Reserve headroom for the injected prompt so the single, no-recovery
            // wrap-up request cannot exceed the window that build() just filled:
            // build() budgets against message_tokens, the same accounting
            // built_tokens sums, so the pushed prompt lands within the limit.
            let wrap_up = Message::user(BUDGET_WRAP_UP_PROMPT);
            let headroom = built_tokens(std::slice::from_ref(&wrap_up));
            let mut messages = ctx.build(self.effective_model_limit().saturating_sub(headroom));
            messages.push(wrap_up);
            let mut req = self.completion_request(messages, preserve_thinking);
            req.tools = Vec::new();
            let started = std::time::Instant::now();
            let mut emitted = (0usize, 0usize);
            match self.one_completion(req, &cancel, &mut emitted).await {
                Ok(wrap) => {
                    if wrap.prompt_tokens > 0 || wrap.completion_tokens > 0 {
                        self.sink.emit(AgentEvent::ServerUsage {
                            prompt_tokens: wrap.prompt_tokens,
                            completion_tokens: wrap.completion_tokens,
                            reasoning_tokens: wrap.reasoning_tokens,
                            cached_tokens: wrap.cached_tokens,
                            cost_usd: wrap.cost_usd,
                            turn_duration_ms: started.elapsed().as_millis() as u64,
                            turn: self.config.max_turns,
                            parent_id: None,
                        });
                    }
                    // Text-only append: stray tool calls are discarded so no
                    // dangling tool_call ids enter persistent history. An empty
                    // reply appends nothing.
                    if !wrap.text.is_empty() {
                        ctx.append(Message::assistant(wrap.text, None));
                    }
                }
                Err(ModelError::Cancelled) => {
                    self.sink.emit(AgentEvent::Done(StopReason::Cancelled));
                    return Ok(());
                }
                Err(e) => {
                    tracing::debug!(error = %e, "budget wrap-up completion skipped");
                }
            }
        }
        self.sink
            .emit(AgentEvent::Done(StopReason::BudgetExhausted));
        Ok(())
    }

    /// Re-enter a parked run (spec §2.4 step 6). `ctx` is ALREADY restored by
    /// the caller (dispatch for children, the server coordinator for the root)
    /// from a verified `Checkpoint` — the loop never reads checkpoint files
    /// itself. This replays the checkpointed turn's gate outcomes (no model
    /// call, no re-prompt for decided calls), executes Phase 2 fresh, then
    /// continues as a normal run.
    pub async fn resume_with_cancel(
        &self,
        ctx: &mut dyn ContextManager,
        resume: ResumeTurn,
        cancel: CancellationToken,
    ) -> Result<(), AgentError> {
        let d = self.sandbox_descriptor();
        if let Some(reason) = d.degraded {
            self.sink.emit(AgentEvent::SandboxDegraded {
                mechanism: d.mechanism,
                reason,
            });
        }
        let mut mw_state = crate::RunState::default();
        let run_shared = crate::RunShared::default();
        // Tallies survive restart (spec §3.8): seed, never refill. The
        // monotonic clamp against restored history ran at load time (caller
        // side, verify_tally_floor); here the values are trusted-verified.
        run_shared.with::<crate::ToolCallCount, _>(|c| c.0 = resume.guardrails.tool_calls as usize);
        run_shared
            .with::<crate::ModelCallCount, _>(|c| c.0 = resume.guardrails.model_calls as usize);
        // run_start hooks fire (memory index re-load, curation setup); a
        // middleware EndRun is honored exactly as on a fresh run. NO
        // set_goal / NO user append — history is restored, goal pinned.
        let flow = self
            .fire_run_start(ctx, &mut mw_state, &cancel, &resume.goal_text, &run_shared)
            .await;
        if let crate::Flow::EndRun(reason) = flow {
            self.sink.emit(AgentEvent::Done(reason));
            return Ok(());
        }
        let preserve_thinking = self.config.preserve_thinking || !self.tools.schemas().is_empty();
        let start_turn = resume.turn as usize;
        // Dispatch-kind commit point (plan review finding 1c): the whole batch
        // is pre-decided and about to execute — delete the stale park NOW. A
        // crash from here loses the run (D1); it never replays the batch.
        // Gate-kind parks are NOT cleared here: an answered ask clears at
        // consume (inside tool_phase); an unanswered one keeps its park and
        // rewrites it when the live re-ask parks again.
        if resume.parked_index.is_none() {
            if let Some(ck) = self.checkpointer.as_ref() {
                ck.clear_park();
            }
        }
        let pre = PreDecided {
            records: resume.gate_records,
            dispatch_kind: resume.parked_index.is_none(),
            parked_index: resume.parked_index.unwrap_or(usize::MAX),
            parked_decision: resume.parked_decision,
        };
        match self
            .tool_phase(
                ctx,
                &mut mw_state,
                &run_shared,
                &cancel,
                start_turn,
                resume.assistant_text,
                resume.tool_calls,
                resume.invalid,
                Some(pre),
            )
            .await
        {
            TurnFlow::End(r) => return r,
            TurnFlow::Continue => {}
        }
        // `turn_loop(start_turn+1)` with start_turn+1 >= max_turns runs zero
        // turns and lands in the budget wrap-up epilogue — exactly right for a
        // final-turn park.
        self.turn_loop(
            ctx,
            &mut mw_state,
            &run_shared,
            cancel,
            start_turn + 1,
            preserve_thinking,
        )
        .await
    }

    /// Phase 1 (gate) + Phase 2 (execute) + Phase 3 (drain) + post-tool
    /// validators + after_tools/on_turn_end hooks — moved verbatim from the
    /// turn loop. Task 7 parks inside Phase 1; Task 8 consumes `pre`.
    #[allow(clippy::too_many_arguments)]
    async fn tool_phase(
        &self,
        ctx: &mut dyn ContextManager,
        mw_state: &mut crate::RunState,
        run_shared: &crate::RunShared,
        cancel: &CancellationToken,
        turn: usize,
        // The turn's model text (park record; Task 7). Unused this task.
        assistant_text: String,
        tool_calls: Vec<ToolCall>,
        invalid: Vec<crate::InvalidParked>,
        // Task 8; None in this task.
        pre: Option<PreDecided>,
    ) -> TurnFlow {
        // Park-support state (E1: pure memory unless an Ask actually parks;
        // None checkpointer ⇒ every branch below is a no-op).
        let batch_for_park: Option<Vec<ToolCall>> =
            self.checkpointer.as_ref().map(|_| tool_calls.clone());
        let mut records: Vec<crate::GateRecord> = Vec::new();
        // Phase 1 — gate every call sequentially (one approval prompt at a time).
        let mut order: Vec<String> = Vec::with_capacity(tool_calls.len() + invalid.len());
        let mut results: HashMap<String, (String, Resolved)> = HashMap::new();
        let mut ready: Vec<ReadyCall> = Vec::new();
        // Seed the unparseable calls first: each emits a ToolStart and joins the
        // Phase-3 drain as a per-call ERROR result (the "re-emit only this call"
        // prompt). N-1 good calls still gate + execute normally below.
        for inv in &invalid {
            self.sink.emit(AgentEvent::ToolStart {
                id: inv.id.clone(),
                name: inv.name.clone(),
                args: serde_json::json!({}),
                parent_id: None,
            });
            order.push(inv.id.clone());
            results.insert(
                inv.id.clone(),
                (
                    inv.name.clone(),
                    Resolved::Err {
                        status: ToolStatus::Error,
                        content: format!(
                            "ERROR: this tool call could not be parsed ({}); the other \
                                 calls in this turn ran normally — re-emit only this call, \
                                 with valid JSON arguments",
                            inv.error
                        ),
                        duration_ms: 0,
                    },
                ),
            );
        }
        for (idx, call) in tool_calls.into_iter().enumerate() {
            // Task 8: on a resume, consult the pre-decided gate outcomes by
            // index BEFORE gating — never re-prompt an already-decided call
            // (spec §3.7). None (fresh run) ⇒ every arm falls through to
            // gate_tool. `gate_preapproved` emits its own ToolStart, mirroring
            // gate_tool, so the pre-decided arms must NOT emit before calling
            // it (exactly one ToolStart per call id).
            let outcome = match pre.as_ref() {
                // P1 (owner 2026-07-10): on a dispatch-kind resume, a Ready
                // NON-dispatch sibling must not re-execute — its pre-crash side
                // effects persist on the host. Synthetic lost-result instead;
                // the model re-runs it if needed.
                Some(p)
                    if p.dispatch_kind
                        && idx < p.records.len()
                        && matches!(p.records[idx], crate::GateRecord::Ready)
                        && call.name != "dispatch_agent" =>
                {
                    self.sink.emit(AgentEvent::ToolStart {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        args: call.args.clone(),
                        parent_id: None,
                    });
                    order.push(call.id.clone());
                    // Build the message ONCE so the gate record and the tool
                    // result never diverge (branch review I4).
                    let lost_result_msg = "ERROR: result lost across daemon restart — \
                                          re-run this call if it is still needed"
                        .to_string();
                    records.push(crate::GateRecord::Rejected {
                        content: lost_result_msg.clone(),
                    });
                    results.insert(
                        call.id,
                        (
                            call.name,
                            Resolved::Err {
                                status: ToolStatus::Error,
                                content: lost_result_msg,
                                duration_ms: 0,
                            },
                        ),
                    );
                    continue;
                }
                Some(p) if idx < p.records.len() => match &p.records[idx] {
                    crate::GateRecord::Rejected { content } => {
                        // Replay the decided rejection. No gate function runs,
                        // so this arm emits its own ToolStart to keep the pair.
                        self.sink.emit(AgentEvent::ToolStart {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            args: call.args.clone(),
                            parent_id: None,
                        });
                        GateOutcome::Rejected {
                            id: call.id,
                            name: call.name,
                            content: content.clone(),
                        }
                    }
                    // gate_preapproved emits ToolStart itself — do NOT emit here.
                    crate::GateRecord::Ready => self.gate_preapproved(call, cancel),
                },
                Some(p) if idx == p.parked_index && p.parked_decision.is_some() => {
                    // The answered parked ask: consume the committed decision —
                    // never re-prompt (spec §2.4 step 6). CONSUME-TIME COMMIT
                    // (plan review BLOCKER 1): delete the park + answer NOW,
                    // before anything executes — a crash after this point loses
                    // the run from here (D1); it never re-prompts an
                    // already-consumed approval, so the approved call can never
                    // execute twice.
                    if let Some(ck) = self.checkpointer.as_ref() {
                        ck.clear_park();
                    }
                    let approve = p.parked_decision.unwrap();
                    if approve {
                        // gate_preapproved emits ToolStart itself.
                        self.gate_preapproved(call, cancel)
                    } else {
                        self.sink.emit(AgentEvent::ToolStart {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            args: call.args.clone(),
                            parent_id: None,
                        });
                        GateOutcome::Rejected {
                            id: call.id,
                            name: call.name,
                            content: format!(
                                "ERROR: {}",
                                ToolError::Denied("user declined".into())
                            ),
                        }
                    }
                }
                // Past the pre-decided prefix (or an unanswered parked ask):
                // gate normally — a later Ask parks again (spec §2.4).
                _ => self.gate_tool(call, cancel).await,
            };
            match outcome {
                GateOutcome::Rejected { id, name, content } => {
                    records.push(crate::GateRecord::Rejected {
                        content: content.clone(),
                    });
                    order.push(id.clone());
                    results.insert(
                        id,
                        (
                            name,
                            Resolved::Err {
                                status: ToolStatus::Denied,
                                content,
                                duration_ms: 0,
                            },
                        ),
                    );
                }
                GateOutcome::Ready(rc) => {
                    records.push(crate::GateRecord::Ready);
                    order.push(rc.id.clone());
                    ready.push(rc);
                }
                GateOutcome::NeedsApproval(pa) => {
                    let PendingAsk {
                        tool,
                        call,
                        req,
                        access,
                    } = *pa;
                    self.sink.emit(AgentEvent::Approval(req.clone()));
                    // Park BEFORE blocking (spec §2.3). Write failure degrades
                    // to live-only — log + event, never block the run.
                    if let (Some(ck), Some(batch)) =
                        (self.checkpointer.as_ref(), batch_for_park.as_ref())
                    {
                        if let Some(state) = ctx.checkpoint_state() {
                            let idx = records.len();
                            let chk = crate::Checkpoint {
                                version: crate::CHECKPOINT_VERSION,
                                session_id: ck.session_id().to_string(),
                                subagent_path: ck.subagent_path().to_vec(),
                                turn: turn as u64,
                                context: state,
                                guardrails: crate::Guardrails {
                                    tool_calls: run_shared.with::<crate::ToolCallCount, _>(|c| c.0)
                                        as u64,
                                    model_calls: run_shared
                                        .with::<crate::ModelCallCount, _>(|c| c.0)
                                        as u64,
                                },
                                parked: crate::ParkedTurn {
                                    assistant_text: assistant_text.clone(),
                                    tool_calls: batch.clone(),
                                    invalid: invalid.clone(),
                                    gate_records: records.clone(),
                                    parked_index: Some(idx),
                                    origin: ck.origin().cloned(),
                                },
                            };
                            let artifacts = ctx
                                .artifacts()
                                .unwrap_or_else(|| Arc::new(crate::SessionArtifacts::new()));
                            if let Err(e) = ck.write_park(chk, &artifacts).await {
                                tracing::warn!(target: "checkpoint", error = %e,
                                    "park write failed; approval is live-only this ask");
                                self.sink.emit(AgentEvent::Error(format!(
                                    "checkpoint write failed (approval not durable): {e}"
                                )));
                            }
                        }
                    }
                    // P2 deadline disarm: while this durable Ask waits,
                    // every enclosing dispatch deadline is suspended.
                    // RAII — deny/cancel/drop all unwind the count.
                    let _ask_guard = self.checkpointer.as_ref().map(|ck| ck.enter_ask());
                    // Race the approval wait against cancellation: Ctrl-C during a
                    // pending prompt must end the run promptly rather than wedge until
                    // the prompt is answered. Cancel-during-prompt counts as a deny.
                    let allowed = tokio::select! {
                        _ = cancel.cancelled() => false,
                        resp = self.approval.request(req) => matches!(
                            resp,
                            ApprovalResponse::Approve | ApprovalResponse::ApproveAlways
                        ),
                    };
                    // Answer commit (spec §2.3): delete the park before
                    // proceeding. Auto-deny (E5 knob) and cancel-deny commit
                    // the same way.
                    if let Some(ck) = self.checkpointer.as_ref() {
                        ck.clear_park();
                    }
                    if allowed {
                        records.push(crate::GateRecord::Ready);
                        let rc = self.ready_call(tool, call, access, cancel);
                        order.push(rc.id.clone());
                        ready.push(rc);
                    } else {
                        // Distinguish a cancel-driven denial (the run is ending) from
                        // an explicit user "no" so the tool result reads correctly.
                        let reason = if cancel.is_cancelled() {
                            "run cancelled"
                        } else {
                            "user declined"
                        };
                        let content = format!("ERROR: {}", ToolError::Denied(reason.into()));
                        records.push(crate::GateRecord::Rejected {
                            content: content.clone(),
                        });
                        order.push(call.id.clone());
                        results.insert(
                            call.id,
                            (
                                call.name,
                                Resolved::Err {
                                    status: ToolStatus::Denied,
                                    content,
                                    duration_ms: 0,
                                },
                            ),
                        );
                    }
                }
            }
        }

        // Ancestor snapshot (spec §2.5, header note 4): memory-only; flushed
        // to disk only if a descendant parks. Dispatch-bearing turns only.
        if let Some(ck) = self.checkpointer.as_ref() {
            if ready.iter().any(|rc| rc.name == "dispatch_agent") {
                if let Some(state) = ctx.checkpoint_state() {
                    ck.set_turn_snapshot(crate::PendingSnapshot {
                        context: state,
                        guardrails: crate::Guardrails {
                            tool_calls: run_shared.with::<crate::ToolCallCount, _>(|c| c.0) as u64,
                            model_calls: run_shared.with::<crate::ModelCallCount, _>(|c| c.0)
                                as u64,
                        },
                        turn: turn as u64,
                        assistant_text: assistant_text.clone(),
                        tool_calls: batch_for_park.clone().unwrap_or_default(),
                        invalid: invalid.clone(),
                        gate_records: records.clone(),
                        artifacts: ctx
                            .artifacts()
                            .unwrap_or_else(|| Arc::new(crate::SessionArtifacts::new())),
                    });
                }
            }
        }

        // Phase 2 — execute approved calls concurrently, bounded. Each call is
        // panic- and timeout-isolated (execute_isolated) so one bad tool can
        // neither crash the loop nor wedge the batch.
        let cap = if self.config.max_parallel_tools == 0 {
            DEFAULT_MAX_PARALLEL_TOOLS
        } else {
            self.config.max_parallel_tools
        };
        let executed: Vec<(String, String, Executed, u64, Duration, agent_tools::Access)> =
            futures::stream::iter(ready.into_iter().map(|rc| {
                let ReadyCall {
                    tool,
                    args,
                    id,
                    name,
                    ctx,
                    access,
                } = rc;
                // The effective per-call deadline (may be a tool's
                // timeout_override, not the loop default) — logged on timeout.
                let timeout = ctx.timeout;
                let middleware = &self.middleware;
                let shared = run_shared;
                async move {
                    let started = std::time::Instant::now();
                    // The gate already ran (Phase 1); this call is the
                    // already-approved ToolCall the wrap-tool chain sees.
                    let call = ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        args: args.clone(),
                    };
                    let ex = crate::ToolNext {
                        tool,
                        name: &name,
                        tctx: &ctx,
                        chain: middleware,
                        call: &call,
                        shared,
                    }
                    .run(args)
                    .await;
                    (
                        id,
                        name,
                        ex,
                        started.elapsed().as_millis() as u64,
                        timeout,
                        access,
                    )
                }
            }))
            .buffer_unordered(cap)
            .collect()
            .await;
        // A mutating turn is one where at least one Write/Destroy/TrustedWrite call ran to
        // an Ok result. Read-only turns and turns whose only mutations failed
        // do NOT trigger post-execution validation.
        let turn_mutated = executed.iter().any(|(_, _, ex, _, _, access)| {
            matches!(ex, Executed::Ok(_))
                && matches!(
                    access,
                    agent_tools::Access::Write
                        | agent_tools::Access::Destroy
                        | agent_tools::Access::TrustedWrite
                )
        });
        for (id, name, ex, duration_ms, timeout, _access) in executed {
            let resolved = match ex {
                Executed::Ok(o) => Resolved::Ok(o, duration_ms),
                Executed::ToolErr(s) => Resolved::Err {
                    status: ToolStatus::Error,
                    content: s,
                    duration_ms,
                },
                Executed::Panicked(s) => {
                    tracing::error!(target: "loop", tool = %name,
                            "tool panicked during parallel dispatch");
                    self.sink.emit(AgentEvent::Error(s.clone()));
                    Resolved::Err {
                        status: ToolStatus::Panic,
                        content: s,
                        duration_ms,
                    }
                }
                Executed::TimedOut(s) => {
                    tracing::warn!(target: "loop", tool = %name,
                            timeout = ?timeout,
                            "tool timed out during parallel dispatch");
                    self.sink.emit(AgentEvent::Error(s.clone()));
                    Resolved::Err {
                        status: ToolStatus::Timeout,
                        content: s,
                        duration_ms,
                    }
                }
            };
            results.insert(id, (name, resolved));
        }

        // Phase 3 — append one tool message per call, in the model's call order.
        for id in order {
            // Normalization guarantees a slot per id; if that invariant is ever
            // violated, drop the result rather than crash on untrusted input.
            let (name, resolved) = match results.remove(&id) {
                Some(v) => v,
                // Unreachable while normalize_tool_call_ids and
                // normalize_invalid_ids hold. If a future change ever breaks
                // the one-slot-per-id invariant, emit an error
                // rather than silently drop the result and desync the transcript
                // (an assistant tool_call with no matching tool message).
                None => (
                    String::new(),
                    Resolved::Err {
                        status: ToolStatus::Error,
                        content: format!("ERROR: internal: no result for tool_call_id {id}"),
                        duration_ms: 0,
                    },
                ),
            };
            let content = match resolved {
                Resolved::Ok(output, duration_ms) => {
                    self.sink.emit(AgentEvent::ToolResult {
                        id: id.clone(),
                        name: name.clone(),
                        status: ToolStatus::Ok,
                        output: output.clone(),
                        duration_ms,
                        parent_id: None,
                    });
                    output.content
                }
                Resolved::Err {
                    status,
                    content,
                    duration_ms,
                } => {
                    self.sink.emit(AgentEvent::ToolResult {
                        id: id.clone(),
                        name: name.clone(),
                        status,
                        output: agent_tools::ToolOutput {
                            content: content.clone(),
                            display: None,
                        },
                        duration_ms,
                        parent_id: None,
                    });
                    content
                }
            };
            ctx.append(Message::tool(id, name, content));
        }

        // Post-execution validation: once per turn, only when a mutating call
        // succeeded and validators are configured. Emits synthetic
        // `post_tool_validate` tool events (reusing existing ToolStart/ToolResult
        // wire kinds) and appends any failures as a single user message so the
        // model sees them. Sits AFTER the Phase-3 tool results and BEFORE the
        // stuck-nudge append (OpenAI-compat ordering: a user message must not
        // land between the assistant tool_calls and its Role::Tool results).
        if turn_mutated && !self.config.post_tool_validators.is_empty() {
            let mut failures: Vec<String> = Vec::new();
            for (n, command) in self.config.post_tool_validators.iter().enumerate() {
                let vid = format!("validate:{}:{}", turn + 1, n);
                self.sink.emit(AgentEvent::ToolStart {
                    id: vid.clone(),
                    name: "post_tool_validate".into(),
                    args: serde_json::json!({ "command": command }),
                    parent_id: None,
                });
                let started = std::time::Instant::now();
                let outcome = run_validator(
                    &self.config.sandbox,
                    &self.config.workspace,
                    command,
                    self.config.tool_timeout,
                    cancel,
                )
                .await;
                let (status, content) = match &outcome {
                    ValidatorOutcome::Passed => (ToolStatus::Ok, "validator passed".to_string()),
                    ValidatorOutcome::Failed { code, output } => {
                        failures.push(format!("$ {command}  (exit {code})\n{output}"));
                        (ToolStatus::Error, format!("exit {code}\n{output}"))
                    }
                    ValidatorOutcome::Skipped { reason } => {
                        (ToolStatus::Error, format!("validator skipped: {reason}"))
                    }
                };
                self.sink.emit(AgentEvent::ToolResult {
                    id: vid,
                    name: "post_tool_validate".into(),
                    status,
                    output: agent_tools::ToolOutput {
                        content,
                        display: None,
                    },
                    duration_ms: started.elapsed().as_millis() as u64,
                    parent_id: None,
                });
            }
            if !failures.is_empty() {
                ctx.append(Message::user(format!(
                    "Post-edit validation reported problems. Fix these before continuing:\n\n{}",
                    failures.join("\n\n")
                )));
            }
        }

        // Node hook: after_tools (spec §5.1). An EndRun here still falls
        // through to the shared turn-end tail below (plan review finding 2):
        // returning inline would leak a flushed dispatch-kind park.
        let flow = self
            .fire_after_tools(ctx, &mut *mw_state, cancel, turn, run_shared)
            .await;
        let out = if let crate::Flow::EndRun(reason) = flow {
            self.sink.emit(AgentEvent::Done(reason));
            TurnFlow::End(Ok(()))
        } else {
            // Scheduled loop-bottom maintain now lives in
            // ContextCurationMiddleware::on_turn_end (spec §5.5), fired below;
            // it also sets the Maintained run-state marker that gates the
            // text-only-exit maintain in after_final_reply.
            //
            // Node hook: on_turn_end (spec §5.1).
            let flow = self
                .fire_turn_end(ctx, &mut *mw_state, cancel, turn, run_shared)
                .await;
            if let crate::Flow::EndRun(reason) = flow {
                self.sink.emit(AgentEvent::Done(reason));
                TurnFlow::End(Ok(()))
            } else {
                TurnFlow::Continue
            }
        };
        // Turn over either way: drop the memory snapshot; remove a dispatch-kind
        // park if a child flushed it this turn (all children finished — Phase 2
        // drained). Zero I/O unless a flush happened. Runs on EVERY exit of this
        // shared tail, including the two EndRun exits above.
        if let Some(ck) = self.checkpointer.as_ref() {
            ck.end_turn();
        }
        out
    }

    /// Resolve, policy-check, and (if needed) get approval for one call — but do NOT
    /// execute it. Sequential by design so approval prompts never overlap.
    async fn gate_tool(&self, call: ToolCall, cancel: &CancellationToken) -> GateOutcome {
        self.sink.emit(AgentEvent::ToolStart {
            id: call.id.clone(),
            name: call.name.clone(),
            args: call.args.clone(),
            parent_id: None,
        });
        // Gate entry: if the run was already cancelled (e.g. Ctrl-C during an
        // earlier prompt in this Phase-1 batch), short-circuit the rest of the
        // batch without touching policy/approval. Placed AFTER the ToolStart emit
        // so every call still gets a start/terminal event pair.
        if cancel.is_cancelled() {
            return GateOutcome::Rejected {
                id: call.id,
                name: call.name,
                content: format!("ERROR: {}", ToolError::Denied("run cancelled".into())),
            };
        }
        let tool = match self.tools.get(&call.name) {
            Some(t) => t,
            None => {
                return GateOutcome::Rejected {
                    id: call.id,
                    name: call.name.clone(),
                    content: format!(
                        "ERROR: {}",
                        ToolError::NotFound(format!("unknown tool {}", call.name))
                    ),
                }
            }
        };
        let intent = match tool.intent(&call.args) {
            Ok(i) => i,
            Err(e) => {
                return GateOutcome::Rejected {
                    id: call.id,
                    name: call.name,
                    content: format!("ERROR: {e}"),
                }
            }
        };
        // Capture the access tier before `intent` may be moved into the
        // `Decision::Ask` arm below (Access is Copy).
        let access = intent.access;
        match self.policy.check(&intent) {
            Decision::Allow => {}
            Decision::Deny(reason) => {
                return GateOutcome::Rejected {
                    id: call.id,
                    name: call.name,
                    content: format!("ERROR: {}", ToolError::Denied(reason)),
                }
            }
            Decision::Ask => {
                let d = self.sandbox_descriptor();
                let posture = if d.degraded.is_some() {
                    format!(" (sandbox: {} degraded; exec refused)", d.mechanism)
                } else {
                    format!(
                        " (sandbox: {}, network {})",
                        d.mechanism,
                        if d.network { "on" } else { "off" }
                    )
                };
                let mut intent = intent;
                if intent.command.is_some() {
                    intent.summary.push_str(&posture);
                }
                // diff preview is produced by execute(); the approval prompt shows the summary.
                let req = ApprovalRequest {
                    intent,
                    display: None,
                    origin: None,
                };
                // Policy said Ask; the CALLER prompts (and, Task 7, parks) —
                // gate_tool no longer awaits the channel itself.
                return GateOutcome::NeedsApproval(Box::new(PendingAsk {
                    tool,
                    call,
                    req,
                    access,
                }));
            }
        };
        GateOutcome::Ready(self.ready_call(tool, call, access, cancel))
    }

    /// Rebuild a `ReadyCall` for a call whose gate outcome was already decided
    /// (resume splice, spec §3.7). No policy check, no prompt — the decision is
    /// reused. Emits its own `ToolStart` (mirroring `gate_tool`) so the caller
    /// must NOT emit before calling this. Resolution/intent failures (a tool
    /// removed or its args invalidated by a config change since the run parked)
    /// surface as honest per-call errors rather than silent re-approval.
    fn gate_preapproved(&self, call: ToolCall, cancel: &CancellationToken) -> GateOutcome {
        self.sink.emit(AgentEvent::ToolStart {
            id: call.id.clone(),
            name: call.name.clone(),
            args: call.args.clone(),
            parent_id: None,
        });
        let tool = match self.tools.get(&call.name) {
            Some(t) => t,
            None => {
                return GateOutcome::Rejected {
                    id: call.id,
                    name: call.name.clone(),
                    content: format!(
                        "ERROR: {}",
                        ToolError::NotFound(format!(
                            "unknown tool {} (removed since this run parked)",
                            call.name
                        ))
                    ),
                }
            }
        };
        let access = match tool.intent(&call.args) {
            Ok(i) => i.access,
            Err(e) => {
                return GateOutcome::Rejected {
                    id: call.id,
                    name: call.name,
                    content: format!("ERROR: {e}"),
                }
            }
        };
        GateOutcome::Ready(self.ready_call(tool, call, access, cancel))
    }

    /// Build the `ToolCtx` + `ReadyCall` for an approved call — the tail of the
    /// old `gate_tool`. Shared by the Allow path and the Phase-1 approval arm.
    fn ready_call(
        &self,
        tool: Arc<dyn Tool>,
        call: ToolCall,
        access: agent_tools::Access,
        cancel: &CancellationToken,
    ) -> ReadyCall {
        // The live run token: a tool that honors `ctx.cancel` (shell/git/fetch_url)
        // aborts when the caller cancels the run (Ctrl-C / SIGINT via the CLI).
        let ctx = ToolCtx {
            workspace: self.config.workspace.clone(),
            timeout: tool.timeout_override().unwrap_or(self.config.tool_timeout),
            cancel: cancel.clone(),
            sandbox: self.config.sandbox.clone(),
            backend: self.backend.clone(),
            call_id: call.id.clone(),
        };
        ReadyCall {
            tool,
            args: call.args,
            id: call.id,
            name: call.name,
            ctx,
            access,
        }
    }
}

/// A call that passed policy/approval and is ready to execute.
struct ReadyCall {
    tool: Arc<dyn Tool>,
    args: serde_json::Value,
    id: String,
    name: String,
    ctx: ToolCtx,
    /// The gated call's declared access tier — carried through execution so the
    /// turn loop can tell a mutating turn (Write/Destroy) from a read-only one.
    access: agent_tools::Access,
}

/// Outcome of gating a single call before execution.
enum GateOutcome {
    Ready(ReadyCall),
    /// Rejected before execution (unknown tool / intent error / denied). `content`
    /// is the final `ERROR: …` text to append as this call's tool result.
    Rejected {
        id: String,
        name: String,
        content: String,
    },
    /// Policy said Ask; the CALLER prompts (and, Task 7, parks) — `gate_tool`
    /// no longer awaits the channel itself.
    NeedsApproval(Box<PendingAsk>),
}

/// One gated-but-unanswered call: everything the caller needs to prompt +
/// proceed once the human answers.
struct PendingAsk {
    tool: Arc<dyn Tool>,
    call: ToolCall,
    /// intent(+posture), display: None, origin: None
    req: ApprovalRequest,
    access: agent_tools::Access,
}

/// Everything needed to re-enter a checkpointed turn. Built by the caller
/// (dispatch for children, the server coordinator for the root) from a
/// verified `Checkpoint` — the loop never reads checkpoint files itself.
pub struct ResumeTurn {
    pub assistant_text: String,
    pub tool_calls: Vec<ToolCall>,
    pub invalid: Vec<crate::InvalidParked>,
    pub gate_records: Vec<crate::GateRecord>,
    /// Some(i) = gate-kind (re-ask or consume `parked_decision` at i);
    /// None = dispatch-kind (whole batch pre-decided; re-enter Phase 2).
    pub parked_index: Option<usize>,
    /// Some(_) when a durable answer.json committed a decision (E2 note: an
    /// ApproveAlways answered pre-restart arrives as plain approve=true).
    pub parked_decision: Option<bool>,
    pub turn: u64,
    pub guardrails: crate::Guardrails,
    /// Goal text handed to fire_run_start (memory recall etc.); from the
    /// restored context's goal, or empty.
    pub goal_text: String,
}

/// Pre-decided gate outcomes for a resumed batch (Task 8 fills it in).
struct PreDecided {
    records: Vec<crate::GateRecord>,
    parked_index: usize,
    /// Some(_) when answer.json committed a decision; None ⇒ re-ask live.
    parked_decision: Option<bool>,
    /// Dispatch-kind resume (owner decision P1): Ready records re-execute
    /// ONLY dispatch_agent calls; other siblings get a synthetic
    /// lost-result error so host side effects never replay.
    dispatch_kind: bool,
}

/// Control flow out of `tool_phase`.
enum TurnFlow {
    /// Turn completed; continue with the next turn.
    Continue,
    /// The run ended inside the phase (Done already emitted) or errored.
    End(Result<(), AgentError>),
}

/// Final per-call result feeding the tool-result message + terminal event.
enum Resolved {
    Ok(agent_tools::ToolOutput, u64),
    /// Terminal `ERROR: …` content (rejected, failed, timed out, or panicked).
    Err {
        status: ToolStatus,
        content: String,
        duration_ms: u64,
    },
}

/// Outcome of an isolated tool execution: the terminal result plus a tag the
/// caller uses to decide how loudly to surface it. `pub` so the wrap-chain
/// base case in `middleware.rs` can name it (provisional contract, Task 3).
#[derive(Debug)]
pub enum Executed {
    Ok(agent_tools::ToolOutput),
    /// Tool returned `Err` — a normal outcome, surfaced only to the model.
    ToolErr(String),
    /// Tool panicked — caught; surfaced loudly.
    Panicked(String),
    /// Dispatch timeout tripped — surfaced loudly.
    TimedOut(String),
}

/// Outcome of one post-execution validator command. Best-effort: a runner
/// failure is `Skipped`, never a run failure.
enum ValidatorOutcome {
    Passed,
    Failed { code: i32, output: String },
    Skipped { reason: String },
}

const VALIDATOR_OUTPUT_CAP: usize = 4096;

/// Run one validator command via `sh -c` through the sandbox, cwd = workspace.
/// Sink-free (caller owns event emission). Combined stdout+stderr capped to
/// VALIDATOR_OUTPUT_CAP (char-boundary safe). A degraded sandbox / spawn error /
/// cancellation yields `Skipped`.
async fn run_validator(
    sandbox: &Arc<dyn agent_tools::SandboxStrategy>,
    workspace: &std::path::Path,
    command: &str,
    timeout: Duration,
    cancel: &CancellationToken,
) -> ValidatorOutcome {
    use tokio::io::AsyncReadExt;
    if cancel.is_cancelled() {
        return ValidatorOutcome::Skipped {
            reason: "run cancelled".into(),
        };
    }
    let spec = agent_tools::CommandSpec {
        program: "sh".into(),
        args: vec!["-c".into(), command.to_string()],
        cwd: workspace.to_path_buf(),
        env: Default::default(),
        kind: agent_tools::ProcKind::OneShot,
    };
    let mut child = match sandbox.launch(spec) {
        Ok(c) => c,
        Err(agent_tools::SandboxError::Unavailable(m)) => {
            return ValidatorOutcome::Skipped {
                reason: format!("sandbox refused: {m}"),
            }
        }
        Err(e) => {
            return ValidatorOutcome::Skipped {
                reason: e.to_string(),
            }
        }
    };
    let mut out_pipe = child.take_stdout();
    let mut err_pipe = child.take_stderr();
    let read_out = async {
        let mut s = String::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_string(&mut s).await;
        }
        s
    };
    let read_err = async {
        let mut s = String::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_string(&mut s).await;
        }
        s
    };
    let run = async {
        let (status, o, e) = tokio::join!(child.wait(), read_out, read_err);
        (status, o, e)
    };
    let (status, stdout, stderr) = tokio::select! {
        _ = cancel.cancelled() => return ValidatorOutcome::Skipped { reason: "run cancelled".into() },
        r = tokio::time::timeout(timeout, run) => match r {
            Ok(v) => v,
            Err(_) => return ValidatorOutcome::Skipped { reason: format!("validator exceeded {timeout:?}") },
        },
    };
    let mut combined = stdout;
    if !stderr.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&stderr);
    }
    let output = truncate_on_char_boundary(&combined, VALIDATOR_OUTPUT_CAP);
    // status: io::Result<ExitStatus>. A signal-killed child has no code(); treat
    // as failure.
    let code = status.ok().and_then(|s| s.code());
    match code {
        Some(0) => ValidatorOutcome::Passed,
        Some(c) => ValidatorOutcome::Failed { code: c, output },
        None => ValidatorOutcome::Failed { code: -1, output },
    }
}

/// Truncate to at most `cap` bytes on a char boundary, appending a marker when cut.
fn truncate_on_char_boundary(s: &str, cap: usize) -> String {
    if s.len() <= cap {
        return s.to_string();
    }
    let mut end = cap;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n…(truncated)", &s[..end])
}

/// Run one tool with panic + timeout isolation. Sink-free and free of `'static`
/// bounds so it is unit-testable without driving the loop; the caller owns event
/// emission. `catch_unwind` keeps a panicking tool from unwinding the loop's task;
/// `timeout` bounds a tool that ignores `ctx.timeout` so one hang can't wedge the
/// whole `buffer_unordered` batch. `pub(crate)`: the base case of the tool
/// wrap chain in `middleware.rs::ToolNext::run` calls this directly (Task 3).
pub(crate) async fn execute_isolated(
    tool: Arc<dyn Tool>,
    args: serde_json::Value,
    name: &str,
    ctx: &ToolCtx,
) -> Executed {
    use futures::FutureExt;
    let fut = std::panic::AssertUnwindSafe(tool.execute(args, ctx)).catch_unwind();
    // Grace margin: arm the backstop at 2x the tool budget so a tool that honors
    // `ctx.timeout` itself resolves first (returning ToolError::Timeout, routed
    // quietly through ToolErr). The backstop only fires for a tool that ignores
    // its deadline entirely — the one case surfaced loudly.
    let backstop = ctx.timeout.saturating_mul(2);
    match tokio::time::timeout(backstop, fut).await {
        Ok(Ok(Ok(output))) => Executed::Ok(output),
        // A tool's own ToolError::Timeout arrives here and stays quiet.
        Ok(Ok(Err(e))) => Executed::ToolErr(format!("ERROR: {e}")),
        Ok(Err(_panic)) => {
            Executed::Panicked(format!("ERROR: tool '{name}' panicked during execution"))
        }
        Err(_elapsed) => Executed::TimedOut(format!(
            "ERROR: tool '{name}' exceeded its {:?} timeout and was force-stopped \
             by the dispatch backstop",
            ctx.timeout
        )),
    }
}

/// Guarantee every tool call in a turn has a unique, non-empty id. Model-supplied
/// ids are passed through verbatim by the protocols, so a model can send duplicate
/// or empty ids; the per-call result contract (one `order` entry + one `results`
/// slot per call) requires uniqueness. Rewrites only offending ids, order-stable
/// and deterministically (no clock/random), and bumps the synthetic id if it would
/// collide with a literal the model also supplied.
fn normalize_tool_call_ids(calls: &mut [ToolCall]) {
    let mut seen = std::collections::HashSet::new();
    for (i, c) in calls.iter_mut().enumerate() {
        if c.id.is_empty() || !seen.insert(c.id.clone()) {
            let mut candidate = format!("call_{i}");
            let mut n = 1;
            while !seen.insert(candidate.clone()) {
                candidate = format!("call_{i}_{n}");
                n += 1;
            }
            c.id = candidate;
        }
    }
}

/// Make each invalid (unparseable) call's id unique against the already-normalized
/// valid calls AND against the other invalid entries. Invalid calls each become
/// their own tool message, so a collision with a valid call — or an empty/duplicate
/// invalid id — would desync the transcript. Rewrites only offending ids,
/// order-stable and deterministically. `valid` is treated as read-only (already
/// normalized by [`normalize_tool_call_ids`]).
fn normalize_invalid_ids(valid: &[ToolCall], invalid: &mut [agent_model::InvalidToolCall]) {
    let mut seen: std::collections::HashSet<String> = valid.iter().map(|c| c.id.clone()).collect();
    for (k, inv) in invalid.iter_mut().enumerate() {
        if inv.id.is_empty() || !seen.insert(inv.id.clone()) {
            let mut candidate = format!("{}_inv{k}", inv.id);
            let mut n = 1;
            while !seen.insert(candidate.clone()) {
                candidate = format!("{}_inv{k}_{n}", inv.id);
                n += 1;
            }
            inv.id = candidate;
        }
    }
}

/// Merge a streamed tool-call delta into the accumulator (handles fragmented args).
///
/// Prefers the streaming `index` to correlate fragments, so parallel tool calls
/// reassemble correctly even if their argument fragments interleave. Falls back
/// to the legacy order-based merge for servers that omit `index`.
fn merge_tool_call(acc: &mut Vec<RawToolCall>, delta: RawToolCall) {
    if let Some(idx) = delta.index {
        if let Some(existing) = acc.iter_mut().find(|c| c.index == Some(idx)) {
            if existing.id.is_none() {
                existing.id = delta.id;
            }
            if existing.name.is_none() {
                existing.name = delta.name;
            }
            existing.args_fragment.push_str(&delta.args_fragment);
        } else {
            acc.push(delta);
        }
        return;
    }
    // No index field: correlate by arrival order (a new call announces an id).
    if delta.id.is_some() || acc.is_empty() {
        acc.push(delta);
    } else if let Some(last) = acc.last_mut() {
        if last.name.is_none() {
            last.name = delta.name;
        }
        last.args_fragment.push_str(&delta.args_fragment);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::*;
    use crate::WindowContext;
    use agent_model::Message;
    use agent_policy::RulePolicy;
    use agent_tools::{fs::ReadFile, ToolCall, ToolRegistry};
    use std::sync::Arc;

    fn registry() -> Arc<ToolRegistry> {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(ReadFile {
            max_bytes: 16 * 1024,
        }));
        Arc::new(r)
    }

    fn tc(id: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: "read_file".into(),
            args: serde_json::json!({}),
        }
    }

    /// Finding 4.2: the maintenance target is the min of the loop window and a
    /// routed compaction model's declared window — a span the compactor can't
    /// read can't be evicted. None = unchanged.
    #[test]
    fn maint_model_limit_is_min_of_loop_and_compaction_windows() {
        let mk = |compaction_model_limit| {
            AgentLoop::new(
                Arc::new(ScriptedModel::new(vec![])),
                Arc::new(PassthroughProtocol),
                registry(),
                policy(std::env::temp_dir()),
                Arc::new(AlwaysApprove),
                Arc::new(CollectingSink::default()),
                LoopConfig {
                    model_limit: 10_000,
                    compaction_model_limit,
                    ..Default::default()
                },
            )
        };
        assert_eq!(mk(None).maint_model_limit(), 10_000);
        assert_eq!(mk(Some(4_000)).maint_model_limit(), 4_000);
        assert_eq!(
            mk(Some(20_000)).maint_model_limit(),
            10_000,
            "a larger compaction window never widens the target"
        );
    }

    #[test]
    fn normalize_ids_makes_empty_and_duplicate_ids_unique() {
        let mut calls = vec![tc(""), tc(""), tc("x"), tc("x")];
        normalize_tool_call_ids(&mut calls);
        let ids: Vec<&str> = calls.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids.len(), 4);
        assert!(ids.iter().all(|s| !s.is_empty()), "no empty ids: {ids:?}");
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), 4, "all ids distinct: {ids:?}");
        assert_eq!(
            ids[2], "x",
            "an already-unique id is left intact when first seen"
        );
    }

    #[test]
    fn normalize_ids_synthetic_avoids_collision_with_model_supplied_literal() {
        // id-less first call AND a model literally sending "call_0" -> still distinct.
        let mut calls = vec![tc(""), tc("call_0")];
        normalize_tool_call_ids(&mut calls);
        assert_ne!(
            calls[0].id,
            calls[1].id,
            "synthetic id must not collide: {:?}",
            calls.iter().map(|c| c.id.clone()).collect::<Vec<_>>()
        );
        assert!(!calls[0].id.is_empty() && !calls[1].id.is_empty());
    }

    #[test]
    fn merge_tool_call_keys_on_index_for_interleaved_parallel_calls() {
        let mut acc = Vec::new();
        // Two calls open (each first fragment carries id+name+index)...
        merge_tool_call(
            &mut acc,
            RawToolCall {
                index: Some(0),
                id: Some("a".into()),
                name: Some("f0".into()),
                args_fragment: "{\"x\":".into(),
            },
        );
        merge_tool_call(
            &mut acc,
            RawToolCall {
                index: Some(1),
                id: Some("b".into()),
                name: Some("f1".into()),
                args_fragment: "{\"y\":".into(),
            },
        );
        // ...then INTERLEAVED arg fragments (id/name absent, only index correlates them).
        merge_tool_call(
            &mut acc,
            RawToolCall {
                index: Some(0),
                id: None,
                name: None,
                args_fragment: "1}".into(),
            },
        );
        merge_tool_call(
            &mut acc,
            RawToolCall {
                index: Some(1),
                id: None,
                name: None,
                args_fragment: "2}".into(),
            },
        );
        assert_eq!(acc.len(), 2);
        assert_eq!(acc[0].name.as_deref(), Some("f0"));
        assert_eq!(acc[0].args_fragment, "{\"x\":1}");
        assert_eq!(acc[1].name.as_deref(), Some("f1"));
        assert_eq!(acc[1].args_fragment, "{\"y\":2}");
    }

    #[test]
    fn merge_tool_call_falls_back_to_order_when_no_index() {
        let mut acc = Vec::new();
        merge_tool_call(
            &mut acc,
            RawToolCall {
                index: None,
                id: Some("a".into()),
                name: Some("f".into()),
                args_fragment: "{".into(),
            },
        );
        merge_tool_call(
            &mut acc,
            RawToolCall {
                index: None,
                id: None,
                name: None,
                args_fragment: "}".into(),
            },
        );
        assert_eq!(acc.len(), 1);
        assert_eq!(acc[0].args_fragment, "{}");
    }

    #[tokio::test]
    async fn scripted_calls_yields_multiple_native_tool_calls() {
        let model = ScriptedModel::new(vec![Scripted::Calls(vec![
            ("c1".into(), "f0".into(), "{}".into()),
            ("c2".into(), "f1".into(), "{}".into()),
        ])]);
        let mut stream = model.stream(CompletionRequest::default()).await.unwrap();
        let mut raw = Vec::new();
        while let Some(item) = stream.next().await {
            if let Chunk::ToolCallDelta(rc) = item.unwrap() {
                raw.push(rc);
            }
        }
        assert_eq!(raw.len(), 2);
        assert_eq!(raw[0].name.as_deref(), Some("f0"));
        assert_eq!(raw[1].id.as_deref(), Some("c2"));
    }

    fn policy(ws: std::path::PathBuf) -> Arc<RulePolicy> {
        Arc::new(RulePolicy {
            workspace: ws,
            command_allowlist: vec![],
            command_denylist: vec![],
        })
    }

    #[tokio::test]
    async fn server_usage_event_carries_token_totals() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let model = Arc::new(ScriptedModel::new(vec![Scripted::TextWithUsage(
            "done".into(),
            900,
            12,
        )]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "hi".into()).await.unwrap();
        let events = sink.events.lock().unwrap().clone();
        assert!(
            events.iter().any(|e| e == "server_usage:900:12"),
            "expected server_usage event with real token totals; got {events:?}"
        );
    }

    /// Wraps a `ScriptedModel` and records each request's tool-schema count, so a
    /// test can prove the wrap-up completion carries no tools (spec Part 2).
    struct ToolCountingModel {
        inner: ScriptedModel,
        tool_counts: std::sync::Mutex<Vec<usize>>,
    }
    #[async_trait::async_trait]
    impl agent_model::ModelClient for ToolCountingModel {
        async fn stream(
            &self,
            req: agent_model::CompletionRequest,
        ) -> Result<
            futures::stream::BoxStream<
                'static,
                Result<agent_model::Chunk, agent_model::ModelError>,
            >,
            agent_model::ModelError,
        > {
            self.tool_counts.lock().unwrap().push(req.tools.len());
            self.inner.stream(req).await
        }
    }

    /// Budget exhaustion triggers ONE tools-disabled wrap-up completion; its text
    /// streams and lands as a text-only assistant append; run still ends
    /// Done(BudgetExhausted). (spec: runtime-knobs Part 2)
    #[tokio::test]
    async fn budget_exhaustion_runs_tools_disabled_wrap_up() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        std::fs::write(ws.join("f.txt"), "x").unwrap();
        let model = Arc::new(ToolCountingModel {
            inner: ScriptedModel::new(vec![
                Scripted::Call(
                    "c1".into(),
                    "read_file".into(),
                    format!(r#"{{"path":"{}"}}"#, ws.join("f.txt").display()),
                ),
                Scripted::Text("wrap-up summary".into()),
            ]),
            tool_counts: Default::default(),
        });
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model.clone(),
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 1,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        let counts = model.tool_counts.lock().unwrap().clone();
        assert_eq!(counts.len(), 2, "turn + wrap-up = exactly two model calls");
        assert!(counts[0] > 0, "the real turn carries tool schemas");
        assert_eq!(counts[1], 0, "the wrap-up is tools-disabled");
        let events = sink.events.lock().unwrap().clone();
        assert!(
            events.iter().any(|e| e == "token:wrap-up summary"),
            "wrap-up streamed: {events:?}"
        );
    }

    /// Finding 4.3: the wrap-up instruction ("tools are disabled...") must never
    /// enter durable history — it would survive into later runs of the same
    /// session as a stale, false capability statement models imitate.
    #[tokio::test]
    async fn budget_wrap_up_prompt_stays_out_of_durable_history() {
        use crate::ContextManager;
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        std::fs::write(ws.join("f.txt"), "x").unwrap();
        let model = Arc::new(ScriptedModel::new(vec![
            // Run 1, turn 1 (budget = 1): a tool call.
            Scripted::Call(
                "c1".into(),
                "read_file".into(),
                format!(r#"{{"path":"{}"}}"#, ws.join("f.txt").display()),
            ),
            // Run 1: the tools-disabled wrap-up completion.
            Scripted::Text("wrap-up summary".into()),
            // Run 2: a plain reply.
            Scripted::Text("second run reply".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink,
            LoopConfig {
                model_limit: 100_000,
                max_turns: 1,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        // Durable history after run 1: summary yes, instruction no.
        let msgs = ctx.build(100_000);
        assert!(
            msgs.iter().any(|m| m.content.contains("wrap-up summary")),
            "the assistant summary IS durable"
        );
        assert!(
            !msgs
                .iter()
                .any(|m| m.content.contains("turn limit for this run")),
            "the wrap-up instruction must not be durable: {msgs:?}"
        );
        // Run 2 (same session context): still no stale instruction in the build.
        agent.run(&mut ctx, "next".into()).await.unwrap();
        let msgs = ctx.build(100_000);
        assert!(
            !msgs
                .iter()
                .any(|m| m.content.contains("turn limit for this run")),
            "no tools-disabled instruction may reach a later run: {msgs:?}"
        );
    }

    /// Finding 4.3 (other half): the wrap-up REQUEST must still end with the
    /// instruction — injection is request-local, not dropped.
    #[tokio::test]
    async fn budget_wrap_up_request_still_carries_the_prompt() {
        struct LastRequestModel {
            inner: ScriptedModel,
            last: std::sync::Mutex<Vec<Message>>,
        }
        #[async_trait::async_trait]
        impl agent_model::ModelClient for LastRequestModel {
            async fn stream(
                &self,
                req: agent_model::CompletionRequest,
            ) -> Result<
                futures::stream::BoxStream<
                    'static,
                    Result<agent_model::Chunk, agent_model::ModelError>,
                >,
                agent_model::ModelError,
            > {
                *self.last.lock().unwrap() = req.messages.clone();
                self.inner.stream(req).await
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        std::fs::write(ws.join("f.txt"), "x").unwrap();
        let model = Arc::new(LastRequestModel {
            inner: ScriptedModel::new(vec![
                Scripted::Call(
                    "c1".into(),
                    "read_file".into(),
                    format!(r#"{{"path":"{}"}}"#, ws.join("f.txt").display()),
                ),
                Scripted::Text("wrap-up summary".into()),
            ]),
            last: std::sync::Mutex::new(Vec::new()),
        });
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model.clone(),
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink,
            LoopConfig {
                model_limit: 100_000,
                max_turns: 1,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let last = model.last.lock().unwrap();
        let tail = last.last().expect("wrap-up request has messages");
        assert!(
            tail.content.contains("turn limit for this run"),
            "the wrap-up request must end with the instruction: {tail:?}"
        );
    }

    /// Review finding: the wrap-up build must reserve headroom for the injected
    /// prompt. `build()` fills history up to the window; pushing the prompt ON
    /// TOP then overruns it — and the wrap-up arm has NO overflow recovery, so in
    /// exactly the budget-exhausted, near-full runs that reach it the overrun
    /// would error the completion and silently drop the summary. Pin: with a
    /// small window and history fat enough to saturate it, the wrap-up request
    /// must fit inside `model_limit` AND still end with the instruction.
    #[tokio::test]
    async fn budget_wrap_up_request_fits_the_window() {
        use crate::ContextManager;
        struct LastRequestModel {
            inner: ScriptedModel,
            last: std::sync::Mutex<Vec<Message>>,
        }
        #[async_trait::async_trait]
        impl agent_model::ModelClient for LastRequestModel {
            async fn stream(
                &self,
                req: agent_model::CompletionRequest,
            ) -> Result<
                futures::stream::BoxStream<
                    'static,
                    Result<agent_model::Chunk, agent_model::ModelError>,
                >,
                agent_model::ModelError,
            > {
                *self.last.lock().unwrap() = req.messages.clone();
                self.inner.stream(req).await
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        std::fs::write(ws.join("f.txt"), "x").unwrap();
        let model = Arc::new(LastRequestModel {
            inner: ScriptedModel::new(vec![
                Scripted::Call(
                    "c1".into(),
                    "read_file".into(),
                    format!(r#"{{"path":"{}"}}"#, ws.join("f.txt").display()),
                ),
                Scripted::Text("wrap-up summary".into()),
            ]),
            last: std::sync::Mutex::new(Vec::new()),
        });
        // A small window, and calibration stays at 1.0 (Scripted::Call/Text emit
        // no server Usage), so effective_model_limit == model_limit == this value.
        let model_limit = 400usize;
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model.clone(),
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink,
            LoopConfig {
                model_limit,
                max_turns: 1,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        // Fat history: many small units (~24 tokens each, well under the prompt's
        // ~59-token headroom) that together far exceed the window, so build() packs
        // right up to the limit and an un-reserved push would overrun it.
        for i in 0..60 {
            ctx.append(Message::user(format!(
                "prior context line {i} carrying a little padding text here"
            )));
        }
        agent.run(&mut ctx, "go".into()).await.unwrap();

        let last = model.last.lock().unwrap();
        assert!(!last.is_empty(), "wrap-up request has messages");
        // The invariant: the single, no-recovery wrap-up request never exceeds the
        // window. Pre-fix (build to full limit, then push) this overruns.
        assert!(
            built_tokens(&last) <= model_limit,
            "wrap-up request must fit the window: {} > {model_limit}",
            built_tokens(&last)
        );
        // The headroom reservation must not drop the instruction — it is still last.
        let tail = last.last().unwrap();
        assert!(
            tail.content.contains("turn limit for this run"),
            "the wrap-up request must still end with the instruction: {tail:?}"
        );
    }

    /// A wrap-up failure is swallowed: no append, still Done(BudgetExhausted).
    #[tokio::test]
    async fn budget_wrap_up_failure_is_best_effort() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        std::fs::write(ws.join("f.txt"), "x").unwrap();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "read_file".into(),
                format!(r#"{{"path":"{}"}}"#, ws.join("f.txt").display()),
            ),
            Scripted::Fail(ModelError::Http("boom".into())),
        ]));
        let sink = Arc::new(DetailSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 1,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        assert_eq!(
            *sink.done.lock().unwrap(),
            Some(StopReason::BudgetExhausted),
            "a failed wrap-up still lands Done(BudgetExhausted)"
        );
        use crate::ContextManager;
        let msgs = ctx.build(100_000);
        assert!(
            !msgs
                .iter()
                .any(|m| m.content.contains("turn limit for this run")),
            "a failed wrap-up must leave no stale instruction in history: {msgs:?}"
        );
    }

    /// Cancel during the wrap-up ends Done(Cancelled), matching loop-entry behavior.
    #[tokio::test]
    async fn budget_wrap_up_cancel_yields_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        std::fs::write(ws.join("f.txt"), "x").unwrap();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "read_file".into(),
                format!(r#"{{"path":"{}"}}"#, ws.join("f.txt").display()),
            ),
            Scripted::Hang,
        ]));
        let sink = Arc::new(DetailSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 1,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        let cancel = CancellationToken::new();
        let cancel_bg = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            cancel_bg.cancel();
        });
        agent
            .run_with_cancel(&mut ctx, "go".into(), cancel)
            .await
            .unwrap();

        assert_eq!(
            *sink.done.lock().unwrap(),
            Some(StopReason::Cancelled),
            "a cancel racing the wrap-up ends Done(Cancelled)"
        );
    }

    /// Stray tool calls in the wrap-up reply are discarded — text-only append,
    /// no dangling ids, still Done(BudgetExhausted).
    #[tokio::test]
    async fn budget_wrap_up_discards_stray_tool_calls() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        std::fs::write(ws.join("f.txt"), "x").unwrap();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "read_file".into(),
                format!(r#"{{"path":"{}"}}"#, ws.join("f.txt").display()),
            ),
            Scripted::Call(
                "c2".into(),
                "read_file".into(),
                format!(r#"{{"path":"{}"}}"#, ws.join("f.txt").display()),
            ),
        ]));
        let sink = Arc::new(DetailSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 1,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        assert_eq!(
            *sink.done.lock().unwrap(),
            Some(StopReason::BudgetExhausted),
            "wrap-up's stray tool call does not change the terminal reason"
        );
        assert_eq!(
            sink.tool_starts.lock().unwrap().len(),
            1,
            "only the real turn's call executed — the wrap-up call was NOT run"
        );
    }

    #[tokio::test]
    async fn precancelled_token_stops_before_calling_model() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let model = Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "should never run".into(),
        )]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        let cancel = CancellationToken::new();
        cancel.cancel(); // already cancelled before the run starts

        agent
            .run_with_cancel(&mut ctx, "go".into(), cancel)
            .await
            .unwrap();

        // Stopped at the turn boundary: the run-inputs record (emitted at run
        // start, audit 6.1) then the terminal Done(Cancelled) — no Usage / Token
        // events (the model was never consulted).
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(
            events,
            vec![
                "run_start:go:system_none=false".to_string(),
                "done".to_string()
            ],
            "events were: {events:?}"
        );
    }

    struct HangsUntilCancel {
        started: Arc<tokio::sync::Notify>,
    }

    #[async_trait::async_trait]
    impl Tool for HangsUntilCancel {
        fn name(&self) -> &str {
            "hang"
        }
        fn description(&self) -> &str {
            "hangs until cancelled"
        }
        fn schema(&self) -> agent_tools::ToolSchema {
            agent_tools::ToolSchema {
                name: "hang".into(),
                description: "".into(),
                parameters: serde_json::json!({"type":"object"}),
            }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<agent_tools::ToolIntent, ToolError> {
            Ok(agent_tools::ToolIntent {
                tool: "hang".into(),
                access: agent_tools::Access::Read,
                paths: vec![],
                command: None,
                summary: "hang".into(),
            })
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
            ctx: &ToolCtx,
        ) -> Result<agent_tools::ToolOutput, ToolError> {
            self.started.notify_one();
            ctx.cancel.cancelled().await; // blocks until the loop's token is cancelled
            Err(ToolError::Timeout)
        }
    }

    #[tokio::test]
    async fn cancel_aborts_a_hung_tool() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let started = Arc::new(tokio::sync::Notify::new());
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(HangsUntilCancel {
            started: started.clone(),
        }));
        let registry = Arc::new(reg);
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "hang".into(), "{}".into()),
            Scripted::Text("after".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry,
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));

        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        // Cancel as soon as the tool reports it has started and is blocking.
        let waiter = tokio::spawn(async move {
            started.notified().await;
            c2.cancel();
        });

        // Without cancellation this never returns (the tool blocks forever); returning
        // at all proves the hang was aborted.
        agent
            .run_with_cancel(&mut ctx, "go".into(), cancel)
            .await
            .unwrap();
        waiter.await.unwrap();

        assert_eq!(sink.events.lock().unwrap().last().unwrap(), "done");
    }

    #[tokio::test]
    async fn duplicate_tool_call_ids_do_not_panic_and_yield_distinct_tool_ids() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "BODY").unwrap();
        let ws = dir.path().to_path_buf();
        // Two calls share id "c1" — collides under the order/results contract and
        // panics the Phase-3 drain on current code.
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                (
                    "c1".into(),
                    "read_file".into(),
                    r#"{"path":"a.txt"}"#.into(),
                ),
                (
                    "c1".into(),
                    "read_file".into(),
                    r#"{"path":"a.txt"}"#.into(),
                ),
            ]),
            Scripted::Text("done".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));

        // Must NOT panic.
        agent.run(&mut ctx, "read twice".into()).await.unwrap();

        // Both calls produced a result — the second was not dropped by a collision.
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(
            events
                .iter()
                .filter(|e| *e == "tool_result:read_file:ok")
                .count(),
            2
        );

        // The transcript carries two DISTINCT tool ids.
        let built = ctx.build(100_000);
        let tool_ids: Vec<String> = built
            .iter()
            .filter(|m| matches!(m.role, agent_model::Role::Tool))
            .map(|m| m.tool_call_id.clone().unwrap_or_default())
            .collect();
        assert_eq!(
            tool_ids.len(),
            2,
            "two tool messages expected: {tool_ids:?}"
        );
        assert_ne!(
            tool_ids[0], tool_ids[1],
            "duplicate ids must normalize to distinct"
        );
    }

    /// Counts how many times it executes — lets a stuck-detection test assert the
    /// aborting turn's call never ran.
    struct Counter(Arc<std::sync::atomic::AtomicUsize>);
    #[async_trait::async_trait]
    impl Tool for Counter {
        fn name(&self) -> &str {
            "counter"
        }
        fn description(&self) -> &str {
            "records each execution"
        }
        fn schema(&self) -> agent_tools::ToolSchema {
            agent_tools::ToolSchema {
                name: "counter".into(),
                description: "".into(),
                parameters: serde_json::json!({"type":"object"}),
            }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<agent_tools::ToolIntent, ToolError> {
            Ok(agent_tools::ToolIntent {
                tool: "counter".into(),
                access: agent_tools::Access::Read,
                paths: vec![],
                command: None,
                summary: "count".into(),
            })
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> Result<agent_tools::ToolOutput, ToolError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(agent_tools::ToolOutput {
                content: "ok".into(),
                display: None,
            })
        }
    }

    fn counter_agent(
        model: Arc<ScriptedModel>,
        sink: Arc<CollectingSink>,
        max_turns: usize,
    ) -> (AgentLoop, Arc<std::sync::atomic::AtomicUsize>) {
        let ws = std::env::temp_dir();
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(Counter(count.clone())));
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            Arc::new(reg),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink,
            LoopConfig {
                model_limit: 100_000,
                max_turns,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        // CAUTION (invariant, verified at plan review, task-6 brief): `with_middleware`
        // REPLACES the whole stack, so any caller that later installs its own stack
        // (e.g. a recording/wrapping chain via `.with_middleware(...)` after calling
        // this helper) silently drops stuck detection too. Harmless today because
        // only the two dedicated stuck tests (plus the malformed-turn parity pin)
        // script >=5 identical calls and none of them override the stack — but any
        // future `counter_agent` caller that BOTH overrides the stack AND scripts
        // repeats must re-add `StuckDetectionMiddleware` itself.
        // A3: `RepairMiddleware` joins the helper's stack so callers driving a
        // malformed turn through `counter_agent` still repair (a bare loop no
        // longer does). Benign for other callers — `on_parse_failure` is inert
        // absent a parse failure.
        let agent = agent.with_middleware(vec![
            Arc::new(crate::StuckDetectionMiddleware),
            Arc::new(crate::RepairMiddleware),
        ]);
        (agent, count)
    }

    /// The 3rd consecutive identical call-set gets a nudge; the 5th aborts the
    /// run without executing (spec §4) — a stuck model burns 4 turns, not 25.
    #[tokio::test]
    async fn stuck_identical_calls_nudged_then_aborted() {
        // 6 identical single-call turns; only turns 1-4 should execute, turn 5 aborts.
        let one = || Scripted::Call("c1".into(), "counter".into(), r#"{"k":"a"}"#.into());
        let model = Arc::new(ScriptedModel::new(vec![
            one(),
            one(),
            one(),
            one(),
            one(),
            one(),
        ]));
        let sink = Arc::new(CollectingSink::default());
        // max_turns=25 (prod default): proves the abort caps burn at 4, not 25.
        let (agent, count) = counter_agent(model.clone(), sink.clone(), 25);
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        // Turns 1-4 executed; turn 5 was consulted (parsed) then aborted BEFORE exec.
        assert_eq!(
            count.load(std::sync::atomic::Ordering::SeqCst),
            4,
            "tool must execute exactly 4 times (turns 1-4; turn 5 aborts pre-exec)"
        );
        // Turn 5 stream WAS consumed (abort is post-parse), turn 6 was not.
        assert_eq!(
            model.remaining(),
            1,
            "abort fires after turn 5 is consulted"
        );

        let events = sink.events.lock().unwrap().clone();
        assert!(
            events
                .iter()
                .any(|e| e.starts_with("error:") && e.contains("5 turns in a row")),
            "expected abort Error mentioning '5 turns in a row'; got {events:?}"
        );
        assert_eq!(events.last().unwrap(), "done", "run ends Done after Error");

        let built = ctx.build(100_000);
        // Exactly one nudge user message.
        let nudge_idx: Vec<usize> = built
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                matches!(m.role, agent_model::Role::User)
                    && m.content.contains("identical tool call")
            })
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            nudge_idx.len(),
            1,
            "exactly one nudge user message expected; got {nudge_idx:?}"
        );
        // Constraint (b): the nudge lands AFTER the turn's tool results, never
        // between the assistant tool_calls message and its Role::Tool results.
        assert!(
            matches!(built[nudge_idx[0] - 1].role, agent_model::Role::Tool),
            "nudge must follow tool results (message before it is Role::Tool)"
        );
        // Constraint (a): the aborted turn's assistant message carries NO tool_calls
        // (no dangling tool_calls with no answering tool messages in persistent history).
        let last_assistant = built
            .iter()
            .rev()
            .find(|m| matches!(m.role, agent_model::Role::Assistant))
            .expect("an assistant message exists");
        assert!(
            last_assistant.tool_calls.is_none(),
            "aborted turn's assistant message must not carry tool_calls"
        );
    }

    /// A differing call-set resets the stuck counter, so an interleaved workload
    /// never trips the abort within its turn budget.
    #[tokio::test]
    async fn stuck_counter_resets_on_different_call() {
        // A A B A A B A A — a differing turn (B) resets before any 5-in-a-row of
        // an identical set can accumulate; run ends by budget, all 8 turns execute.
        let a = || Scripted::Call("c1".into(), "counter".into(), r#"{"k":"a"}"#.into());
        let b = || Scripted::Call("c1".into(), "counter".into(), r#"{"k":"b"}"#.into());
        let model = Arc::new(ScriptedModel::new(vec![
            a(),
            a(),
            b(),
            a(),
            a(),
            b(),
            a(),
            a(),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let (agent, count) = counter_agent(model.clone(), sink.clone(), 8);
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        assert_eq!(
            count.load(std::sync::atomic::Ordering::SeqCst),
            8,
            "all 8 turns execute — no abort"
        );
        assert_eq!(
            model.remaining(),
            0,
            "all scripted turns consumed (budget end)"
        );
        let events = sink.events.lock().unwrap().clone();
        assert!(
            !events.iter().any(|e| e.contains("aborting")),
            "no abort should fire; got {events:?}"
        );
        assert_eq!(events.last().unwrap(), "done");
    }

    /// A repair turn must not advance OR reset stuck counters (spec §5.1):
    /// A, A, malformed, A, A, A → abort on the 5th *parsed* identical turn,
    /// exactly as if the malformed turn never happened — today's inline code
    /// skips the signature block entirely on the repair turn, so
    /// last_sig/repeats survive the repair turn unchanged. This is a parity
    /// pin: captured against the pre-migration (inline) implementation in
    /// Task 6 Step 2, then held unchanged across the middleware migration.
    #[tokio::test]
    async fn repair_turn_leaves_stuck_counters_untouched() {
        let a = || Scripted::Call("c1".into(), "counter".into(), r#"{"k":"a"}"#.into());
        let model = Arc::new(ScriptedModel::new(vec![
            a(),
            a(),
            // Malformed-args Call (no `Malformed` variant exists — see the
            // testkit note in Task 2): registered tool, unparseable JSON args.
            Scripted::Call("c1".into(), "counter".into(), r#"{"k": "#.into()),
            a(),
            a(),
            a(),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let (agent, count) = counter_agent(model, sink.clone(), 25);
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        // Baseline captured against the pre-migration (inline stuck-detection)
        // tree in Task 6 Step 2 — see task-6-report.md for the raw run.
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 4);
        let events = sink.events.lock().unwrap().clone();
        assert!(events.iter().any(|e| e.contains("5 turns in a row")));
    }

    /// `RunShared` (spec §5.2) threads from the wrap-tool-call seam
    /// (`ToolNext::shared`) to the node-hook seam (`RunCx::shared`) through
    /// the SAME per-run store: a write in `wrap_tool_call` must be visible to
    /// `after_tools` on that same run.
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
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "counter".into(), r#"{"k":"a"}"#.into()),
            Scripted::Text("done".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let (agent, _count) = counter_agent(model, sink, 10);
        let agent = agent.with_middleware(vec![Arc::new(Probe(seen.clone()))]);
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        assert_eq!(*seen.lock().unwrap(), Some(1));
    }

    #[tokio::test]
    async fn runs_tool_then_finishes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "FILEBODY").unwrap();
        let ws = dir.path().to_path_buf();

        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "read_file".into(),
                r#"{"path":"a.txt"}"#.into(),
            ),
            Scripted::Text("The file says FILEBODY".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );

        let mut ctx = WindowContext::new(Message::system("you are a test agent"));
        agent.run(&mut ctx, "read a.txt".into()).await.unwrap();

        let events = sink.events.lock().unwrap().clone();
        assert!(events.iter().any(|e| e == "tool_start:read_file"));
        assert!(events.iter().any(|e| e == "tool_result:read_file:ok"));
        assert!(events.last().unwrap() == "done");
    }

    #[tokio::test]
    async fn truncated_tool_call_reports_max_tokens_not_bad_args() {
        let ws = std::env::temp_dir();
        // Both turns truncate mid-args (as a real re-emit of a large file would),
        // so the loop can't recover by "re-emitting correctly".
        let truncated = r##"{"path":"big.py","content":"#!/usr/bin/env python3\nprint('hi"##;
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::TruncatedCall("write_file".into(), truncated.into()),
            Scripted::TruncatedCall("write_file".into(), truncated.into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: Some(2048),
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent
            .run(&mut ctx, "write a big file".into())
            .await
            .unwrap();

        let events = sink.events.lock().unwrap().clone();
        let err = events
            .iter()
            .find(|e| e.starts_with("error:"))
            .expect("expected an error event for the truncated turn");
        let low = err.to_lowercase();
        assert!(
            low.contains("max_tokens") || low.contains("truncat"),
            "error should explain the truncation cause, got: {err}"
        );
        assert!(
            !err.contains("EOF while parsing"),
            "must not surface the raw JSON EOF parse error: {err}"
        );
        assert_eq!(
            events.last().map(String::as_str),
            Some("done"),
            "the truncation abort must still terminate with Done; events were: {events:?}"
        );
    }

    #[tokio::test]
    async fn protocol_repair_exhausted_emits_done() {
        let ws = std::env::temp_dir();
        // Two consecutive unparseable tool calls (malformed JSON args, stop is
        // ToolCalls not Length): the first triggers a re-emit repair, the second
        // exhausts the single repair budget and the turn aborts.
        let bad = r#"{"path": "a.txt", "#.to_string();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "read_file".into(), bad.clone()),
            Scripted::Call("c2".into(), "read_file".into(), bad),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        // A3: a bare loop no longer repairs — `RepairMiddleware` in-stack is
        // what makes turn 1 re-ask and turn 2 exhaust the single budget. Without
        // it the run would give up on turn 1 and pass for the WRONG reason.
        let agent = agent.with_middleware(vec![Arc::new(crate::RepairMiddleware)]);
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "read a.txt".into()).await.unwrap();

        let events = sink.events.lock().unwrap().clone();
        assert!(
            events.iter().any(|e| e.starts_with("error:")),
            "expected an error event for the exhausted repair; events were: {events:?}"
        );
        assert_eq!(
            events.last().map(String::as_str),
            Some("done"),
            "the repair-exhausted abort must still terminate with Done; events were: {events:?}"
        );
    }

    use agent_policy::PolicyEngine;
    use agent_tools::ToolIntent;

    struct DenyAll;
    impl PolicyEngine for DenyAll {
        fn check(&self, _i: &ToolIntent) -> Decision {
            Decision::Deny("nope".into())
        }
    }

    #[tokio::test]
    async fn denied_tool_feeds_error_back_and_continues() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "X").unwrap();
        let ws = dir.path().to_path_buf();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "read_file".into(),
                r#"{"path":"a.txt"}"#.into(),
            ),
            Scripted::Text("Understood, it was denied.".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            Arc::new(DenyAll),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let events = sink.events.lock().unwrap().clone();
        // No successful tool_result (it was denied) — the call terminates in a
        // Denied ToolResult instead — and the loop still reached done.
        assert!(!events.iter().any(|e| e == "tool_result:read_file:ok"));
        assert!(
            events.iter().any(|e| e == "tool_result:read_file:denied"),
            "a denied call must still emit a terminal ToolResult: {events:?}"
        );
        assert_eq!(events.last().unwrap(), "done");
    }

    /// Ctrl-C during a pending approval prompt must end the run promptly as
    /// Cancelled — not wedge until the prompt is answered (audit Component 4).
    #[tokio::test(start_paused = true)]
    async fn cancel_during_pending_approval_ends_run() {
        use std::sync::Mutex;

        // Approval channel that never answers — models a prompt left hanging.
        struct NeverApprove;
        #[async_trait::async_trait]
        impl ApprovalChannel for NeverApprove {
            async fn request(&self, _req: ApprovalRequest) -> ApprovalResponse {
                std::future::pending().await
            }
        }

        // Captures the fields the string-label CollectingSink can't: the Done
        // reason, the terminal ToolResult content, and an Approval-emitted signal.
        #[derive(Default)]
        struct CancelRaceSink {
            approval_seen: tokio::sync::Notify,
            done: Mutex<Option<StopReason>>,
            results: Mutex<Vec<(ToolStatus, String)>>,
        }
        impl EventSink for CancelRaceSink {
            fn emit(&self, event: AgentEvent) {
                match event {
                    AgentEvent::Approval(_) => self.approval_seen.notify_one(),
                    AgentEvent::ToolResult { status, output, .. } => {
                        self.results.lock().unwrap().push((status, output.content));
                    }
                    AgentEvent::Done(r) => *self.done.lock().unwrap() = Some(r),
                    _ => {}
                }
            }
        }

        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "read_file".into(),
                r#"{"path":"a.txt"}"#.into(),
            ),
            Scripted::Text("should not run".into()),
        ]));
        let sink = Arc::new(CancelRaceSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            Arc::new(AskAll),
            Arc::new(NeverApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );

        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        let run = tokio::spawn(async move {
            let mut ctx = WindowContext::new(Message::system("sys"));
            agent.run_with_cancel(&mut ctx, "go".into(), cancel).await
        });

        // Cancel only once the approval prompt is actually pending.
        sink.approval_seen.notified().await;
        c2.cancel();

        // The run must return promptly; on today's code it would wedge on the
        // never-resolving prompt and this timeout would fire.
        tokio::time::timeout(std::time::Duration::from_secs(5), run)
            .await
            .expect("run must finish promptly after cancel, not wedge on the prompt")
            .expect("run task must not panic")
            .expect("run returns Ok");

        assert_eq!(
            *sink.done.lock().unwrap(),
            Some(StopReason::Cancelled),
            "cancel during a prompt must end the run as Cancelled"
        );
        let results = sink.results.lock().unwrap();
        let (status, content) = results
            .last()
            .expect("the gated call gets a terminal result");
        assert_eq!(*status, ToolStatus::Denied);
        assert!(
            content.contains("run cancelled"),
            "cancel-driven denial must say 'run cancelled', got: {content}"
        );
    }

    #[tokio::test]
    async fn emits_usage_event_before_completing() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![Scripted::Text("hi".into())]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            Arc::new(DenyAll),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let events = sink.events.lock().unwrap().clone();
        // A usage event is emitted, and it precedes the terminal done.
        let usage_idx = events
            .iter()
            .position(|e| e.starts_with("usage:"))
            .expect("usage event present");
        let done_idx = events
            .iter()
            .rposition(|e| e == "done")
            .expect("done present");
        assert!(usage_idx < done_idx);
    }

    #[tokio::test(start_paused = true)]
    async fn transport_error_then_success_via_retry() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Error,
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 3,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert_eq!(sink.events.lock().unwrap().last().unwrap(), "done");
    }

    #[tokio::test(start_paused = true)]
    async fn retry_backoff_sleeps_grow_exponentially_in_situ() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Error,
            Scripted::Error,
            Scripted::Error,
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 3,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        let start = tokio::time::Instant::now();
        agent.run(&mut ctx, "go".into()).await.unwrap();
        // Paused clock: virtual elapsed is the loop's backoff sleeps — three
        // failures -> jittered_backoff(1..=3), each `backoff_delay + 0..=25%`.
        // Base schedule is 100 + 200 + 400 = 700ms; with per-attempt jitter the
        // total lands in [700ms, 875ms]. This pins that the LOOP sleeps the
        // schedule, which the pure backoff_delay unit test cannot.
        let elapsed = start.elapsed();
        assert!(
            elapsed >= std::time::Duration::from_millis(700)
                && elapsed <= std::time::Duration::from_millis(875),
            "backoff elapsed {elapsed:?} outside [700ms, 875ms]"
        );
        assert_eq!(sink.events.lock().unwrap().last().unwrap(), "done");
    }

    #[tokio::test(start_paused = true)]
    async fn retry_after_header_delays_at_least_its_seconds() {
        let ws = std::env::temp_dir();
        // A single 429 carrying Retry-After: 3 seconds, then success. The loop
        // must sleep >= 3s virtual (Retry-After dominates the ~100ms backoff),
        // capped at 30s.
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Fail(ModelError::Status {
                code: 429,
                body: "rate limited".into(),
                retry_after: Some(3),
            }),
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 3,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        let start = tokio::time::Instant::now();
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert!(
            start.elapsed() >= std::time::Duration::from_secs(3),
            "Retry-After: 3 should force >= 3s sleep, got {:?}",
            start.elapsed()
        );
        assert_eq!(sink.events.lock().unwrap().last().unwrap(), "done");
    }

    #[tokio::test]
    async fn fatal_400_fails_fast_without_retry() {
        let ws = std::env::temp_dir();
        // One scripted 400; a Text follow-up that must NEVER be consulted.
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Fail(ModelError::Status {
                code: 400,
                body: "invalid request".into(),
                retry_after: None,
            }),
            Scripted::Text("should not be reached".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model.clone(),
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 3,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        let err = agent.run(&mut ctx, "go".into()).await.unwrap_err();
        assert!(matches!(err, AgentError::Model(_)));
        let events = sink.events.lock().unwrap().clone();
        assert!(
            events.iter().any(|e| e.starts_with("error")),
            "expected an error event: {events:?}"
        );
        assert_eq!(events.last().map(String::as_str), Some("done"));
        // The second scripted turn is still queued: the model was consulted once.
        assert_eq!(model.remaining(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_429_is_retried_then_succeeds() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Fail(ModelError::Status {
                code: 429,
                body: "rate limited".into(),
                retry_after: None,
            }),
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 3,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert_eq!(
            sink.events.lock().unwrap().last().map(String::as_str),
            Some("done")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn exhaustion_emits_done_error() {
        let ws = std::env::temp_dir();
        // All-retryable failures burn max_retries then abort WITH a Done.
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Fail(ModelError::Http("down".into())),
            Scripted::Fail(ModelError::Http("down".into())),
            Scripted::Fail(ModelError::Http("down".into())),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        let err = agent.run(&mut ctx, "go".into()).await.unwrap_err();
        assert!(matches!(err, AgentError::Model(_)));
        assert_eq!(
            sink.events.lock().unwrap().last().map(String::as_str),
            Some("done")
        );
    }

    #[test]
    fn backoff_is_exponential_and_capped() {
        assert_eq!(backoff_delay(1), Duration::from_millis(100));
        assert_eq!(backoff_delay(2), Duration::from_millis(200));
        assert_eq!(backoff_delay(3), Duration::from_millis(400));
        assert_eq!(backoff_delay(7), Duration::from_millis(5_000)); // 6400 capped
        assert_eq!(backoff_delay(60), Duration::from_millis(5_000)); // no overflow
    }

    #[tokio::test]
    async fn budget_exhaustion_stops_the_loop() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "X").unwrap();
        let ws = dir.path().to_path_buf();
        // Model always calls a tool, never finishes -> must hit max_turns.
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call(
                "c".into(),
                "read_file".into(),
                r#"{"path":"a.txt"}"#.into()
            );
            100
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 3,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "loop forever".into()).await.unwrap();
        // 3 turns, each a tool call, then done (BudgetExhausted).
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(
            events
                .iter()
                .filter(|e| *e == "tool_start:read_file")
                .count(),
            3
        );
        assert_eq!(events.last().unwrap(), "done");
    }

    #[tokio::test(start_paused = true)]
    async fn idle_stall_times_out_and_fails_after_retries() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Hang,
            Scripted::Hang,
            Scripted::Hang,
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: Duration::from_secs(5),
                stream_idle_timeout: Duration::from_secs(10),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        // Guard >> the loop's 10s idle timeout so the loop terminates first.
        let result =
            tokio::time::timeout(Duration::from_secs(600), agent.run(&mut ctx, "go".into()))
                .await
                .expect("loop must terminate on a stalled stream, not hang");
        let err = result.unwrap_err();
        assert!(matches!(err, AgentError::Model(_)));
        assert!(
            err.to_string().contains("timeout"),
            "expected a timeout-caused failure, got: {err}"
        );
        let events = sink.events.lock().unwrap().clone();
        assert!(events.iter().any(|e| e.starts_with("error:")));
        // Exhaustion now aborts WITH a terminal Done(StopReason::Error).
        assert_eq!(events.last().map(String::as_str), Some("done"));
    }

    #[tokio::test(start_paused = true)]
    async fn stream_open_stall_times_out() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::HangOpen,
            Scripted::HangOpen,
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: Duration::from_secs(5),
                stream_idle_timeout: Duration::from_secs(10),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        let result =
            tokio::time::timeout(Duration::from_secs(600), agent.run(&mut ctx, "go".into()))
                .await
                .expect("loop must terminate when the stream never opens, not hang");
        let err = result.unwrap_err();
        assert!(matches!(err, AgentError::Model(_)));
        assert!(
            err.to_string().contains("timeout"),
            "expected a timeout-caused failure, got: {err}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn stall_then_success_recovers_via_retry() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Hang,
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 3,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: Duration::from_secs(5),
                stream_idle_timeout: Duration::from_secs(10),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        let result =
            tokio::time::timeout(Duration::from_secs(600), agent.run(&mut ctx, "go".into()))
                .await
                .expect("loop must terminate, not hang");
        assert!(result.is_ok());
        assert_eq!(sink.events.lock().unwrap().last().unwrap(), "done");
    }

    struct SlowModel {
        gap: Duration,
    }
    #[async_trait::async_trait]
    impl agent_model::ModelClient for SlowModel {
        async fn stream(
            &self,
            _req: CompletionRequest,
        ) -> Result<futures::stream::BoxStream<'static, Result<Chunk, ModelError>>, ModelError>
        {
            let gap = self.gap;
            let chunks = vec![
                Ok(Chunk::Text("hel".into())),
                Ok(Chunk::Text("lo".into())),
                Ok(Chunk::Done(StopReason::Stop)),
            ];
            Ok(futures::stream::iter(chunks)
                .then(move |c| async move {
                    tokio::time::sleep(gap).await;
                    c
                })
                .boxed())
        }
    }

    fn empty_registry() -> Arc<ToolRegistry> {
        Arc::new(ToolRegistry::new())
    }

    async fn run_reasoning_with(preserve: bool, tools: Arc<ToolRegistry>) -> Vec<Message> {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let model = Arc::new(ScriptedModel::new(vec![Scripted::Reasoning(
            "secret plan".into(),
            "final answer".into(),
        )]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            tools,
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink,
            LoopConfig {
                model_limit: 100_000,
                max_turns: 5,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                preserve_thinking: preserve,
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        ctx.build(100_000)
    }

    // No tools registered, so preservation is driven purely by the config flag.
    async fn run_reasoning(preserve: bool) -> Vec<Message> {
        run_reasoning_with(preserve, empty_registry()).await
    }

    #[tokio::test]
    async fn tools_present_force_reasoning_preservation_even_with_flag_off() {
        // Agentic workloads need within-turn reasoning continuity across the
        // tool loop, so registered tools auto-enable preservation regardless of
        // the config flag (Qwen3.6 keeps it via reasoning_content + the kwarg).
        let msgs = run_reasoning_with(false, registry()).await;
        let a = msgs
            .iter()
            .find(|m| matches!(m.role, agent_model::Role::Assistant))
            .unwrap();
        assert_eq!(a.reasoning.as_deref(), Some("secret plan"));
    }

    #[tokio::test]
    async fn preserve_thinking_keeps_reasoning_as_message_data() {
        let msgs = run_reasoning(true).await;
        let a = msgs
            .iter()
            .find(|m| matches!(m.role, agent_model::Role::Assistant))
            .unwrap();
        // Reasoning is preserved as separate data, NOT baked into content — each
        // backend renders it per its own contract (see agent-model adapters).
        assert_eq!(a.reasoning.as_deref(), Some("secret plan"));
        assert_eq!(a.content, "final answer");
        assert!(!a.content.contains("<think>"));
    }

    #[tokio::test]
    async fn default_strips_reasoning_from_history() {
        let msgs = run_reasoning(false).await;
        let a = msgs
            .iter()
            .find(|m| matches!(m.role, agent_model::Role::Assistant))
            .unwrap();
        assert_eq!(a.reasoning, None);
        assert_eq!(a.content, "final answer");
    }

    #[tokio::test]
    async fn loop_routes_execute_command_through_injected_sandbox() {
        use agent_tools::{
            CommandSpec, HostExecutor, Mode, SandboxDescriptor, SandboxError, SandboxStrategy,
            SandboxedChild,
        };
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        struct CountingSandbox {
            inner: HostExecutor,
            hits: Arc<AtomicUsize>,
        }
        impl SandboxStrategy for CountingSandbox {
            fn launch(&self, spec: CommandSpec) -> Result<SandboxedChild, SandboxError> {
                self.hits.fetch_add(1, Ordering::SeqCst);
                self.inner.launch(spec)
            }
            fn describe(&self) -> SandboxDescriptor {
                SandboxDescriptor {
                    mode: Mode::Off,
                    mechanism: "counting",
                    image: None,
                    network: true,
                    degraded: None,
                }
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let hits = Arc::new(AtomicUsize::new(0));
        let sandbox = Arc::new(CountingSandbox {
            inner: HostExecutor,
            hits: hits.clone(),
        });

        // Register execute_command tool
        let mut r = ToolRegistry::new();
        r.register(Arc::new(agent_tools::shell::ExecuteCommand));
        let tools = Arc::new(r);

        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "execute_command".into(),
                r#"{"command":"echo hello"}"#.into(),
            ),
            Scripted::Text("Done.".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            tools,
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                sandbox,
                ..Default::default()
            },
        );

        let mut ctx = WindowContext::new(Message::system("you are a test agent"));
        agent.run(&mut ctx, "run echo hello".into()).await.unwrap();

        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn approval_summary_includes_sandbox_posture() {
        use agent_policy::ApprovalChannel;
        use agent_tools::HostExecutor;
        use std::sync::{Arc, Mutex};

        struct RecordingApproval {
            captured: Arc<Mutex<Option<String>>>,
        }
        #[async_trait::async_trait]
        impl ApprovalChannel for RecordingApproval {
            async fn request(&self, req: ApprovalRequest) -> ApprovalResponse {
                *self.captured.lock().unwrap() = Some(req.intent.summary.clone());
                ApprovalResponse::Deny
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();

        // Register execute_command tool
        let mut r = ToolRegistry::new();
        r.register(Arc::new(agent_tools::shell::ExecuteCommand));
        let tools = Arc::new(r);

        // Empty allowlist -> Decision::Ask for any command
        let pol = Arc::new(RulePolicy {
            workspace: ws.clone(),
            command_allowlist: vec![],
            command_denylist: vec![],
        });

        let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let approval = Arc::new(RecordingApproval {
            captured: captured.clone(),
        });

        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "execute_command".into(),
                r#"{"command":"echo hello"}"#.into(),
            ),
            Scripted::Text("Done.".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            tools,
            pol,
            approval,
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                sandbox: Arc::new(HostExecutor),
                ..Default::default()
            },
        );

        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "run echo hello".into()).await.unwrap();

        let summary = captured
            .lock()
            .unwrap()
            .clone()
            .expect("approval must have been requested");
        assert!(
            summary.contains("(sandbox: host, network on)"),
            "summary does not contain posture: {summary:?}"
        );
    }

    #[tokio::test]
    async fn degraded_posture_shows_exec_refused() {
        use agent_policy::ApprovalChannel;
        use agent_tools::{
            CommandSpec, HostExecutor, Mode, SandboxDescriptor, SandboxError, SandboxStrategy,
            SandboxedChild,
        };
        use std::sync::{Arc, Mutex};

        struct DegradedFake;
        impl SandboxStrategy for DegradedFake {
            fn launch(&self, spec: CommandSpec) -> Result<SandboxedChild, SandboxError> {
                HostExecutor.launch(spec) // degraded == runs on host
            }
            fn describe(&self) -> SandboxDescriptor {
                SandboxDescriptor {
                    mode: Mode::Auto,
                    mechanism: "docker",
                    image: Some("debian:stable-slim".into()),
                    network: false,
                    degraded: Some("no daemon".into()),
                }
            }
        }

        struct RecordingApproval {
            captured: Arc<Mutex<Option<String>>>,
        }
        #[async_trait::async_trait]
        impl ApprovalChannel for RecordingApproval {
            async fn request(&self, req: ApprovalRequest) -> ApprovalResponse {
                *self.captured.lock().unwrap() = Some(req.intent.summary.clone());
                ApprovalResponse::Deny
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();

        let mut r = ToolRegistry::new();
        r.register(Arc::new(agent_tools::shell::ExecuteCommand));
        let tools = Arc::new(r);

        // Empty allowlist -> Decision::Ask for any command
        let pol = Arc::new(RulePolicy {
            workspace: ws.clone(),
            command_allowlist: vec![],
            command_denylist: vec![],
        });

        let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let approval = Arc::new(RecordingApproval {
            captured: captured.clone(),
        });

        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "execute_command".into(),
                r#"{"command":"echo hello"}"#.into(),
            ),
            Scripted::Text("Done.".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            tools,
            pol,
            approval,
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                sandbox: Arc::new(DegradedFake),
                ..Default::default()
            },
        );

        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "run echo hello".into()).await.unwrap();

        let summary = captured
            .lock()
            .unwrap()
            .clone()
            .expect("approval must have been requested");
        assert!(
            summary.contains("degraded; exec refused"),
            "summary should signal degraded fail-closed state: {summary:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn slow_but_progressing_stream_does_not_trip() {
        let ws = std::env::temp_dir();
        // gap (5s) < idle timeout (10s): healthy progress must NOT trip the timeout.
        let model = Arc::new(SlowModel {
            gap: Duration::from_secs(5),
        });
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: Duration::from_secs(5),
                stream_idle_timeout: Duration::from_secs(10),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        let result =
            tokio::time::timeout(Duration::from_secs(600), agent.run(&mut ctx, "go".into()))
                .await
                .expect("loop must terminate, not hang");
        assert!(result.is_ok());
        let events = sink.events.lock().unwrap().clone();
        assert!(!events.iter().any(|e| e.starts_with("error:")));
        assert_eq!(events.last().unwrap(), "done");
    }

    // ---- Parallel tool-call execution -------------------------------------
    use agent_model::Role;
    use agent_tools::{Access, Tool, ToolOutput, ToolSchema};

    struct AllowAll;
    impl PolicyEngine for AllowAll {
        fn check(&self, _i: &ToolIntent) -> Decision {
            Decision::Allow
        }
    }

    /// Tool that blocks on a shared 2-party barrier — only completes if a sibling
    /// call runs concurrently. Sequential execution deadlocks it.
    struct BarrierTool {
        name: String,
        barrier: Arc<tokio::sync::Barrier>,
    }
    #[async_trait::async_trait]
    impl Tool for BarrierTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "waits on a shared barrier"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: self.name.clone(),
                description: "barrier".into(),
                parameters: serde_json::json!({"type":"object","properties":{}}),
            }
        }
        fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent {
                tool: self.name.clone(),
                access: Access::Read,
                paths: vec![],
                command: None,
                summary: "barrier".into(),
            })
        }
        async fn execute(
            &self,
            _a: serde_json::Value,
            _c: &ToolCtx,
        ) -> Result<ToolOutput, ToolError> {
            self.barrier.wait().await;
            Ok(ToolOutput {
                content: format!("{} done", self.name),
                display: None,
            })
        }
    }

    #[tokio::test]
    async fn parallel_tool_calls_execute_concurrently() {
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let mut r = ToolRegistry::new();
        r.register(Arc::new(BarrierTool {
            name: "wait_a".into(),
            barrier: barrier.clone(),
        }));
        r.register(Arc::new(BarrierTool {
            name: "wait_b".into(),
            barrier: barrier.clone(),
        }));
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "wait_a".into(), "{}".into()),
                ("c2".into(), "wait_b".into(), "{}".into()),
            ]),
            Scripted::Text("both done".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            Arc::new(r),
            Arc::new(AllowAll),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: std::env::temp_dir(),
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        // Sequential execution would block wait_a forever (wait_b never starts) -> timeout.
        let res = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            agent.run(&mut ctx, "go".into()),
        )
        .await;
        assert!(
            res.is_ok(),
            "parallel calls did not run concurrently (barrier deadlock)"
        );
        res.unwrap().unwrap();
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.starts_with("tool_result:"))
                .count(),
            2
        );
    }

    /// Deterministic tool: sleeps `delay_ms`, then returns `body` as its content.
    struct FakeTool {
        name: String,
        delay_ms: u64,
        body: String,
    }
    #[async_trait::async_trait]
    impl Tool for FakeTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "fake"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: self.name.clone(),
                description: "fake".into(),
                parameters: serde_json::json!({"type":"object","properties":{}}),
            }
        }
        fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent {
                tool: self.name.clone(),
                access: Access::Read,
                paths: vec![],
                command: None,
                summary: "fake".into(),
            })
        }
        async fn execute(
            &self,
            _a: serde_json::Value,
            _c: &ToolCtx,
        ) -> Result<ToolOutput, ToolError> {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
            Ok(ToolOutput {
                content: self.body.clone(),
                display: None,
            })
        }
    }

    fn tool_messages(ctx: &WindowContext) -> Vec<(String, String)> {
        ctx.build(usize::MAX)
            .into_iter()
            .filter(|m| m.role == Role::Tool)
            .map(|m| (m.tool_call_id.unwrap_or_default(), m.content))
            .collect()
    }

    #[tokio::test]
    async fn tool_results_keep_model_call_order_despite_completion_order() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(FakeTool {
            name: "slow".into(),
            delay_ms: 150,
            body: "SLOW".into(),
        }));
        r.register(Arc::new(FakeTool {
            name: "fast".into(),
            delay_ms: 5,
            body: "FAST".into(),
        }));
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "slow".into(), "{}".into()), // finishes LAST
                ("c2".into(), "fast".into(), "{}".into()),
            ]), // finishes FIRST
            Scripted::Text("done".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            Arc::new(r),
            Arc::new(AllowAll),
            Arc::new(AlwaysApprove),
            sink,
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: std::env::temp_dir(),
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let msgs = tool_messages(&ctx);
        assert_eq!(
            msgs,
            vec![("c1".into(), "SLOW".into()), ("c2".into(), "FAST".into())]
        );
    }

    #[tokio::test]
    async fn multiple_tool_calls_produce_matched_results_in_order() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(FakeTool {
            name: "ta".into(),
            delay_ms: 0,
            body: "AAA".into(),
        }));
        r.register(Arc::new(FakeTool {
            name: "tb".into(),
            delay_ms: 0,
            body: "BBB".into(),
        }));
        r.register(Arc::new(FakeTool {
            name: "tc".into(),
            delay_ms: 0,
            body: "CCC".into(),
        }));
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "ta".into(), "{}".into()),
                ("c2".into(), "tb".into(), "{}".into()),
                ("c3".into(), "tc".into(), "{}".into()),
            ]),
            Scripted::Text("done".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            Arc::new(r),
            Arc::new(AllowAll),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: std::env::temp_dir(),
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert_eq!(
            tool_messages(&ctx),
            vec![
                ("c1".into(), "AAA".into()),
                ("c2".into(), "BBB".into()),
                ("c3".into(), "CCC".into())
            ]
        );
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.starts_with("tool_result:"))
                .count(),
            3
        );
    }

    #[tokio::test]
    async fn one_failing_call_does_not_abort_the_others() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(FakeTool {
            name: "ta".into(),
            delay_ms: 0,
            body: "AAA".into(),
        }));
        r.register(Arc::new(FakeTool {
            name: "tc".into(),
            delay_ms: 0,
            body: "CCC".into(),
        }));
        // "tb" is intentionally NOT registered -> unknown-tool rejection for c2.
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "ta".into(), "{}".into()),
                ("c2".into(), "tb".into(), "{}".into()),
                ("c3".into(), "tc".into(), "{}".into()),
            ]),
            Scripted::Text("done".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            Arc::new(r),
            Arc::new(AllowAll),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: std::env::temp_dir(),
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let msgs = tool_messages(&ctx);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0], ("c1".into(), "AAA".into()));
        assert_eq!(msgs[1].0, "c2");
        assert!(msgs[1].1.starts_with("ERROR:"), "got {:?}", msgs[1].1);
        assert_eq!(msgs[2], ("c3".into(), "CCC".into()));
        // Every call terminates in a ToolResult — two ok, one denied (unknown
        // tool is gate-rejected) — and the loop still completes.
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.starts_with("tool_result:"))
                .count(),
            3
        );
        assert_eq!(events.iter().filter(|e| e.ends_with(":ok")).count(), 2);
        assert!(
            events.iter().any(|e| e == "tool_result:tb:denied"),
            "events: {events:?}"
        );
        assert_eq!(events.last().unwrap(), "done");
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    struct AskAll;
    impl PolicyEngine for AskAll {
        fn check(&self, _i: &ToolIntent) -> Decision {
            Decision::Ask
        }
    }

    /// Approval channel that records the peak number of concurrent in-flight requests.
    struct CountingApproval {
        inflight: AtomicUsize,
        peak: AtomicUsize,
    }
    #[async_trait::async_trait]
    impl ApprovalChannel for CountingApproval {
        async fn request(&self, _req: ApprovalRequest) -> ApprovalResponse {
            let n = self.inflight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(n, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await; // widen any overlap
            self.inflight.fetch_sub(1, Ordering::SeqCst);
            ApprovalResponse::Approve
        }
    }

    #[tokio::test]
    async fn run_emits_sandbox_degraded_even_without_tool_calls() {
        use agent_tools::{
            CommandSpec, HostExecutor, Mode, SandboxDescriptor, SandboxError, SandboxStrategy,
            SandboxedChild,
        };
        use std::sync::Arc;

        struct DegradedFake;
        impl SandboxStrategy for DegradedFake {
            fn launch(&self, spec: CommandSpec) -> Result<SandboxedChild, SandboxError> {
                HostExecutor.launch(spec)
            }
            fn describe(&self) -> SandboxDescriptor {
                SandboxDescriptor {
                    mode: Mode::Auto,
                    mechanism: "docker",
                    image: Some("debian:stable-slim".into()),
                    network: false,
                    degraded: Some("no daemon".into()),
                }
            }
        }

        let ws = std::env::temp_dir();
        // A plain text turn: no tool calls, so no approval prompt is ever hit.
        let model = Arc::new(ScriptedModel::new(vec![Scripted::Text("hi".into())]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                sandbox: Arc::new(DegradedFake),
                ..Default::default()
            },
        );

        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "hello".into()).await.unwrap();

        let events = sink.events.lock().unwrap();
        assert!(
            events.iter().any(|e| e == "sandbox_degraded:docker"),
            "degraded sandbox must be surfaced even with no tool calls: {events:?}"
        );
    }

    #[tokio::test]
    async fn run_emits_run_start_with_the_exact_input() {
        // Finding 6.1: the run's inputs land in the event stream (and thus the
        // trace) before any model output, so a failed top-level turn is
        // replayable from the trace alone.
        struct StubCtx(Vec<Message>);
        #[async_trait::async_trait]
        impl ContextManager for StubCtx {
            fn append(&mut self, msg: Message) {
                self.0.push(msg);
            }
            fn build(&self, _model_limit: usize) -> Vec<Message> {
                self.0.clone()
            }
            fn set_system(&mut self, _system: Message) {}
            // `system()` deliberately left at the trait default (None).
        }

        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![Scripted::Text("hi".into())]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );

        let mut ctx = StubCtx(Vec::new());
        agent.run(&mut ctx, "hello world".into()).await.unwrap();

        let events = sink.events.lock().unwrap().clone();
        // Exact input, and system None for a stub context without one.
        let run_start = events
            .iter()
            .position(|e| e == "run_start:hello world:system_none=true")
            .unwrap_or_else(|| panic!("RunStart with the exact input: {events:?}"));
        let first_output = events
            .iter()
            .position(|e| e.starts_with("token:") || e.starts_with("tool_start:"))
            .expect("the scripted turn streams a token");
        assert!(
            run_start < first_output,
            "RunStart must precede any Token/ToolStart: {events:?}"
        );
    }

    #[tokio::test]
    async fn approvals_are_serialized_across_parallel_calls() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(FakeTool {
            name: "ta".into(),
            delay_ms: 0,
            body: "AAA".into(),
        }));
        r.register(Arc::new(FakeTool {
            name: "tb".into(),
            delay_ms: 0,
            body: "BBB".into(),
        }));
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "ta".into(), "{}".into()),
                ("c2".into(), "tb".into(), "{}".into()),
            ]),
            Scripted::Text("done".into()),
        ]));
        let approval = Arc::new(CountingApproval {
            inflight: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            Arc::new(r),
            Arc::new(AskAll),
            approval.clone(),
            Arc::new(CollectingSink::default()),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: std::env::temp_dir(),
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert_eq!(
            approval.peak.load(Ordering::SeqCst),
            1,
            "approval prompts must never overlap"
        );
    }

    // ---- Panic + timeout isolation (execute_isolated) ---------------------

    /// Install (once) a panic hook that swallows ONLY the sentinel panic our
    /// PanicTool raises, so the expected caught-panic line does not pollute test
    /// output. Any unexpected panic still prints via the default hook. Race-free
    /// (Once), no restore needed.
    fn silence_sentinel_panics() {
        use std::sync::Once;
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            let default = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                let is_sentinel = info
                    .payload()
                    .downcast_ref::<&str>()
                    .map(|s| *s == "SENTINEL_TEST_PANIC")
                    .unwrap_or(false);
                if !is_sentinel {
                    default(info);
                }
            }));
        });
    }

    /// A tool that panics inside `execute` (with the sentinel payload).
    struct PanicTool {
        name: String,
    }
    #[async_trait::async_trait]
    impl Tool for PanicTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "panics"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: self.name.clone(),
                description: "panics".into(),
                parameters: serde_json::json!({"type":"object","properties":{}}),
            }
        }
        fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent {
                tool: self.name.clone(),
                access: Access::Read,
                paths: vec![],
                command: None,
                summary: "panics".into(),
            })
        }
        async fn execute(
            &self,
            _a: serde_json::Value,
            _c: &ToolCtx,
        ) -> Result<ToolOutput, ToolError> {
            panic!("SENTINEL_TEST_PANIC");
        }
    }

    fn test_ctx(timeout: Duration) -> ToolCtx {
        ToolCtx {
            workspace: std::env::temp_dir(),
            timeout,
            cancel: CancellationToken::new(),
            sandbox: Arc::new(agent_tools::HostExecutor),
            backend: Arc::new(agent_tools::backend::HostBackend::new(std::env::temp_dir())),
            call_id: "test".into(),
        }
    }

    #[tokio::test]
    async fn execute_isolated_catches_panic() {
        silence_sentinel_panics();
        let tool: Arc<dyn Tool> = Arc::new(PanicTool {
            name: "boom".into(),
        });
        let ex = execute_isolated(
            tool,
            serde_json::json!({}),
            "boom",
            &test_ctx(Duration::from_secs(5)),
        )
        .await;
        assert!(
            matches!(ex, Executed::Panicked(ref s) if s.contains("boom") && s.contains("panicked")),
            "panic must be caught as Executed::Panicked"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn execute_isolated_trips_timeout() {
        // Huge tool sleep vs a 100ms budget: under paused time the timeout timer
        // fires first, so this is deterministic with no real wall-clock wait.
        let tool: Arc<dyn Tool> = Arc::new(FakeTool {
            name: "slow".into(),
            delay_ms: 3_600_000,
            body: "never".into(),
        });
        let ex = execute_isolated(
            tool,
            serde_json::json!({}),
            "slow",
            &test_ctx(Duration::from_millis(100)),
        )
        .await;
        assert!(
            matches!(ex, Executed::TimedOut(ref s) if s.contains("slow") && s.contains("backstop")),
            "a tool ignoring ctx.timeout must be force-stopped by the backstop"
        );
    }

    #[tokio::test]
    async fn execute_isolated_passes_through_ok_and_err() {
        let ok_tool: Arc<dyn Tool> = Arc::new(FakeTool {
            name: "ok".into(),
            delay_ms: 0,
            body: "hi".into(),
        });
        let ex = execute_isolated(
            ok_tool,
            serde_json::json!({}),
            "ok",
            &test_ctx(Duration::from_secs(5)),
        )
        .await;
        assert!(matches!(ex, Executed::Ok(ref o) if o.content == "hi"));

        let err_tool: Arc<dyn Tool> = Arc::new(ErrTool { name: "err".into() });
        let ex = execute_isolated(
            err_tool,
            serde_json::json!({}),
            "err",
            &test_ctx(Duration::from_secs(5)),
        )
        .await;
        assert!(matches!(ex, Executed::ToolErr(ref s) if s.starts_with("ERROR: ")));
    }

    #[tokio::test(start_paused = true)]
    async fn execute_isolated_keeps_tool_honored_timeout_quiet() {
        // The tool self-times-out at ctx.timeout (100ms), before the 200ms backstop,
        // so it lands on the quiet ToolErr path, not the loud TimedOut path.
        let tool: Arc<dyn Tool> = Arc::new(SelfTimeoutTool {
            name: "polite".into(),
        });
        let ex = execute_isolated(
            tool,
            serde_json::json!({}),
            "polite",
            &test_ctx(Duration::from_millis(100)),
        )
        .await;
        assert!(
            matches!(ex, Executed::ToolErr(ref s) if s.contains("timed out")),
            "a tool honoring ctx.timeout stays on the quiet ToolErr path: {ex:?}"
        );
    }

    /// A well-behaved tool that enforces `ctx.timeout` itself and returns
    /// ToolError::Timeout on elapse (never runs past its own deadline).
    struct SelfTimeoutTool {
        name: String,
    }
    #[async_trait::async_trait]
    impl Tool for SelfTimeoutTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "self-times-out"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: self.name.clone(),
                description: "self-times-out".into(),
                parameters: serde_json::json!({"type":"object","properties":{}}),
            }
        }
        fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent {
                tool: self.name.clone(),
                access: Access::Read,
                paths: vec![],
                command: None,
                summary: "self-times-out".into(),
            })
        }
        async fn execute(
            &self,
            _a: serde_json::Value,
            ctx: &ToolCtx,
        ) -> Result<ToolOutput, ToolError> {
            tokio::time::timeout(ctx.timeout, std::future::pending::<()>())
                .await
                .map_err(|_| ToolError::Timeout)?;
            unreachable!("pending never resolves")
        }
    }

    /// A tool that returns Err (not a panic) from `execute`.
    struct ErrTool {
        name: String,
    }
    #[async_trait::async_trait]
    impl Tool for ErrTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "errs"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: self.name.clone(),
                description: "errs".into(),
                parameters: serde_json::json!({"type":"object","properties":{}}),
            }
        }
        fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent {
                tool: self.name.clone(),
                access: Access::Read,
                paths: vec![],
                command: None,
                summary: "errs".into(),
            })
        }
        async fn execute(
            &self,
            _a: serde_json::Value,
            _c: &ToolCtx,
        ) -> Result<ToolOutput, ToolError> {
            Err(ToolError::Failed {
                message: "nope".into(),
                stderr: None,
            })
        }
    }

    #[tokio::test]
    async fn panicking_tool_is_isolated_from_the_batch() {
        silence_sentinel_panics();
        let mut r = ToolRegistry::new();
        r.register(Arc::new(PanicTool {
            name: "boom".into(),
        }));
        r.register(Arc::new(FakeTool {
            name: "ok".into(),
            delay_ms: 0,
            body: "OK".into(),
        }));
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "boom".into(), "{}".into()),
                ("c2".into(), "ok".into(), "{}".into()),
            ]),
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            Arc::new(r),
            Arc::new(AllowAll),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: std::env::temp_dir(),
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));

        // The panic must NOT abort the run.
        agent
            .run(&mut ctx, "go".into())
            .await
            .expect("panic must be isolated, run completes");

        let msgs = tool_messages(&ctx);
        let boom = msgs
            .iter()
            .find(|(id, _)| id == "c1")
            .expect("c1 tool message present");
        assert!(
            boom.1.contains("panicked"),
            "panicker yields an error tool-result: {boom:?}"
        );
        let ok = msgs
            .iter()
            .find(|(id, _)| id == "c2")
            .expect("c2 tool message present");
        assert_eq!(ok.1, "OK", "the sibling tool still ran");

        let events = sink.events.lock().unwrap().clone();
        assert!(
            events
                .iter()
                .any(|e| e.starts_with("error:") && e.contains("panicked")),
            "a panic emits a loud AgentEvent::Error: {events:?}"
        );
        assert!(
            events.iter().any(|e| e == "tool_result:boom:panic"),
            "the panicking call emits a terminal ToolResult with Panic status: {events:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn deadline_ignoring_tool_is_force_stopped_by_backstop() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(FakeTool {
            name: "hang".into(),
            delay_ms: 3_600_000,
            body: "never".into(),
        }));
        r.register(Arc::new(FakeTool {
            name: "ok".into(),
            delay_ms: 0,
            body: "OK".into(),
        }));
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "hang".into(), "{}".into()),
                ("c2".into(), "ok".into(), "{}".into()),
            ]),
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            Arc::new(r),
            Arc::new(AllowAll),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: std::env::temp_dir(),
                tool_timeout: std::time::Duration::from_millis(100),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));

        // Under paused time, the 100ms dispatch timeout fires before the 3600s sleep,
        // so the turn completes instead of hanging.
        agent
            .run(&mut ctx, "go".into())
            .await
            .expect("hang must be bounded, run completes");

        let msgs = tool_messages(&ctx);
        let hang = msgs
            .iter()
            .find(|(id, _)| id == "c1")
            .expect("c1 tool message present");
        assert!(
            hang.1.contains("backstop"),
            "the offender is force-stopped by the backstop: {hang:?}"
        );
        let ok = msgs
            .iter()
            .find(|(id, _)| id == "c2")
            .expect("c2 tool message present");
        assert_eq!(ok.1, "OK", "the sibling tool still ran");

        let events = sink.events.lock().unwrap().clone();
        assert!(
            events
                .iter()
                .any(|e| e.starts_with("error:") && e.contains("backstop")),
            "the backstop emits a loud AgentEvent::Error: {events:?}"
        );
        assert!(
            events.iter().any(|e| e == "tool_result:hang:timeout"),
            "the force-stopped call emits a terminal ToolResult with Timeout status: {events:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tool_honored_timeout_is_quiet_at_loop_level() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(SelfTimeoutTool {
            name: "polite".into(),
        }));
        r.register(Arc::new(FakeTool {
            name: "ok".into(),
            delay_ms: 0,
            body: "OK".into(),
        }));
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "polite".into(), "{}".into()),
                ("c2".into(), "ok".into(), "{}".into()),
            ]),
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            Arc::new(r),
            Arc::new(AllowAll),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: std::env::temp_dir(),
                tool_timeout: std::time::Duration::from_millis(100),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));

        agent
            .run(&mut ctx, "go".into())
            .await
            .expect("run completes");

        let msgs = tool_messages(&ctx);
        let polite = msgs
            .iter()
            .find(|(id, _)| id == "c1")
            .expect("c1 tool message present");
        assert!(
            polite.1.contains("timed out"),
            "tool's own timeout message is used: {polite:?}"
        );
        let events = sink.events.lock().unwrap().clone();
        assert!(
            !events.iter().any(|e| e.starts_with("error:")),
            "a tool-honored timeout must NOT emit a loud AgentEvent::Error: {events:?}"
        );
    }

    // ---- Per-call terminal ToolResult events (id, status, duration_ms) -----
    use crate::ToolStatus;
    use std::sync::Mutex;

    /// Structured capture sink for fields the CollectingSink string labels
    /// can't carry: ids, statuses, and durations.
    #[derive(Default)]
    struct ToolEventCapture {
        results: Mutex<Vec<(String, String, ToolStatus, u64)>>,
        starts: Mutex<Vec<String>>,
    }
    impl EventSink for ToolEventCapture {
        fn emit(&self, event: AgentEvent) {
            match event {
                AgentEvent::ToolStart { id, .. } => self.starts.lock().unwrap().push(id),
                AgentEvent::ToolResult {
                    id,
                    name,
                    status,
                    duration_ms,
                    ..
                } => self
                    .results
                    .lock()
                    .unwrap()
                    .push((id, name, status, duration_ms)),
                _ => {}
            }
        }
    }

    fn loop_with_capture(
        model: Arc<ScriptedModel>,
        tools: ToolRegistry,
        capture: Arc<ToolEventCapture>,
    ) -> AgentLoop {
        AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            Arc::new(tools),
            Arc::new(AllowAll),
            Arc::new(AlwaysApprove),
            capture,
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: std::env::temp_dir(),
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        )
    }

    #[tokio::test]
    async fn every_resolved_call_emits_tool_result() {
        // One turn with three calls: one ok, one unknown tool (gate-rejected ->
        // Denied), one erroring tool (-> Error). Every call must terminate in
        // exactly one ToolResult event.
        let mut r = ToolRegistry::new();
        r.register(Arc::new(FakeTool {
            name: "echo".into(),
            delay_ms: 0,
            body: "OK".into(),
        }));
        r.register(Arc::new(ErrTool { name: "err".into() }));
        // "ghost" is intentionally NOT registered -> unknown-tool rejection.
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "echo".into(), "{}".into()),
                ("c2".into(), "ghost".into(), "{}".into()),
                ("c3".into(), "err".into(), "{}".into()),
            ]),
            Scripted::Text("done".into()),
        ]));
        let capture = Arc::new(ToolEventCapture::default());
        let agent = loop_with_capture(model, r, capture.clone());
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        let results = capture.results.lock().unwrap();
        assert_eq!(
            results.len(),
            3,
            "one terminal event per call, got {results:?}"
        );
        let statuses: std::collections::HashSet<_> = results.iter().map(|r| r.2).collect();
        assert!(statuses.contains(&ToolStatus::Ok));
        assert!(statuses.contains(&ToolStatus::Denied));
        assert!(statuses.contains(&ToolStatus::Error));
    }

    #[tokio::test]
    async fn tool_result_ids_match_tool_start() {
        // Two parallel ok calls: every ToolResult id must match a ToolStart id.
        let mut r = ToolRegistry::new();
        r.register(Arc::new(FakeTool {
            name: "ta".into(),
            delay_ms: 0,
            body: "A".into(),
        }));
        r.register(Arc::new(FakeTool {
            name: "tb".into(),
            delay_ms: 0,
            body: "B".into(),
        }));
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "ta".into(), "{}".into()),
                ("c2".into(), "tb".into(), "{}".into()),
            ]),
            Scripted::Text("done".into()),
        ]));
        let capture = Arc::new(ToolEventCapture::default());
        let agent = loop_with_capture(model, r, capture.clone());
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        let starts: std::collections::HashSet<_> =
            capture.starts.lock().unwrap().iter().cloned().collect();
        let result_ids: std::collections::HashSet<_> = capture
            .results
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.0.clone())
            .collect();
        assert_eq!(starts.len(), 2);
        assert_eq!(starts, result_ids);
    }

    #[tokio::test]
    async fn executed_calls_report_nonzero_duration_and_denied_zero() {
        // One ok call whose tool sleeps ~10ms, one unknown tool (never executed).
        let mut r = ToolRegistry::new();
        r.register(Arc::new(FakeTool {
            name: "sleepy".into(),
            delay_ms: 10,
            body: "Z".into(),
        }));
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "sleepy".into(), "{}".into()),
                ("c2".into(), "ghost".into(), "{}".into()),
            ]),
            Scripted::Text("done".into()),
        ]));
        let capture = Arc::new(ToolEventCapture::default());
        let agent = loop_with_capture(model, r, capture.clone());
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        let results = capture.results.lock().unwrap();
        let ok = results.iter().find(|r| r.2 == ToolStatus::Ok).unwrap();
        let denied = results.iter().find(|r| r.2 == ToolStatus::Denied).unwrap();
        assert!(
            ok.3 >= 5,
            "executed duration_ms should reflect the ~10ms sleep, got {}",
            ok.3
        );
        assert_eq!(denied.3, 0, "gate-rejected calls never executed");
    }

    #[tokio::test]
    async fn server_usage_carries_turn_duration() {
        struct UsageCapture {
            turn_duration_ms: Mutex<Option<u64>>,
        }
        impl EventSink for UsageCapture {
            fn emit(&self, event: AgentEvent) {
                if let AgentEvent::ServerUsage {
                    turn_duration_ms, ..
                } = event
                {
                    *self.turn_duration_ms.lock().unwrap() = Some(turn_duration_ms);
                }
            }
        }
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![Scripted::Text("hi".into())]));
        let capture = Arc::new(UsageCapture {
            turn_duration_ms: Mutex::new(None),
        });
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            capture.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert!(
            capture.turn_duration_ms.lock().unwrap().is_some(),
            "ServerUsage must carry a measured turn_duration_ms"
        );
    }

    /// Context stub for overflow recovery: counts request_compaction calls,
    /// and after the first one build() returns a shrunk history.
    struct OverflowCtx {
        history: Vec<Message>,
        compaction_requests: usize,
        maintains: usize,
    }
    #[async_trait::async_trait]
    impl ContextManager for OverflowCtx {
        fn append(&mut self, m: Message) {
            self.history.push(m);
        }
        fn set_system(&mut self, _: Message) {}
        fn set_memory_index(&mut self, _: Vec<String>) {}
        fn set_goal(&mut self, _: String) {}
        fn build(&self, _limit: usize) -> Vec<Message> {
            if self.compaction_requests > 0 {
                self.history.iter().take(1).cloned().collect() // "shrunk"
            } else {
                self.history.clone()
            }
        }
        async fn maintain(&mut self, _deps: &crate::MaintCtx<'_>) -> crate::MaintReport {
            self.maintains += 1;
            crate::MaintReport::default()
        }
        fn request_compaction(&mut self) {
            self.compaction_requests += 1;
        }
    }

    /// Context stub recording the order of maintain() and build() calls.
    /// `usize::MAX` is the sentinel the middleware seam's own
    /// `assert_no_orphans` invariant self-check uses (loop_.rs
    /// `AgentLoop::assert_no_orphans`, fired after every node hook,
    /// independent of what that hook does) — it is not a "the loop is
    /// building a request" event, so it is excluded from the recorded
    /// cadence these tests pin.
    struct SeqCtx {
        history: Vec<Message>,
        calls: std::sync::Mutex<Vec<&'static str>>,
    }
    #[async_trait::async_trait]
    impl ContextManager for SeqCtx {
        fn append(&mut self, m: Message) {
            self.history.push(m);
        }
        fn set_system(&mut self, _: Message) {}
        fn set_memory_index(&mut self, _: Vec<String>) {}
        fn set_goal(&mut self, _: String) {}
        fn build(&self, limit: usize) -> Vec<Message> {
            if limit != usize::MAX {
                self.calls.lock().unwrap().push("build");
            }
            self.history.clone()
        }
        async fn maintain(&mut self, _deps: &crate::MaintCtx<'_>) -> crate::MaintReport {
            self.calls.lock().unwrap().push("maintain");
            crate::MaintReport::default()
        }
        fn request_compaction(&mut self) {}
    }

    #[tokio::test]
    async fn text_only_run_is_curated_at_exit() {
        // The text-reply exit used to return before the loop-bottom maintain,
        // so a pure chat run was never curated at all — only silently
        // truncated by build(). The exit path must maintain before Done.
        // (Deliberately NOT start-of-turn: maintaining with the fresh user
        // prompt already appended pushed the previous tool turn into the
        // compactable span one run earlier and regressed memory-roster.)
        let ws = std::env::temp_dir();
        let model = std::sync::Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "just a chat reply".into(),
        )]));
        let sink = Arc::new(CollectingSink::default());
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink,
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        )
        .with_middleware(vec![Arc::new(crate::ContextCurationMiddleware::new(flag))]);
        let mut ctx = SeqCtx {
            history: vec![],
            calls: Default::default(),
        };
        agent.run(&mut ctx, "hello".into()).await.unwrap();
        let calls = ctx.calls.lock().unwrap().clone();
        assert!(
            calls.contains(&"maintain"),
            "a text-only run must still be curated: {calls:?}"
        );
        // Exit-path semantics: the run's final maintain comes AFTER the model
        // call's build, curating the settled context (reply included).
        let m = calls.iter().rposition(|c| *c == "maintain").unwrap();
        let b = calls.iter().rposition(|c| *c == "build").unwrap();
        assert!(
            b < m,
            "the exit maintain must follow the last build: {calls:?}"
        );
    }

    #[tokio::test]
    async fn tool_bearing_run_skips_the_exit_maintain() {
        // A run with tool turns is already curated by the loop-bottom maintain
        // after each turn; the text-exit pass is for PURE text-only runs only.
        // An extra exit maintain compacts one unit deeper per run and wobbled
        // locked-portmap/memory-roster in paired eval batches.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "FILEBODY").unwrap();
        let ws = dir.path().to_path_buf();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "read_file".into(),
                r#"{"path":"a.txt"}"#.into(),
            ),
            Scripted::Text("The file says FILEBODY".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink,
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        )
        .with_middleware(vec![Arc::new(crate::ContextCurationMiddleware::new(flag))]);
        let mut ctx = SeqCtx {
            history: vec![],
            calls: Default::default(),
        };
        agent.run(&mut ctx, "read it".into()).await.unwrap();
        let calls = ctx.calls.lock().unwrap().clone();
        let maintains = calls.iter().filter(|c| **c == "maintain").count();
        assert_eq!(
            maintains, 1,
            "exactly the loop-bottom maintain, no exit pass: {calls:?}"
        );
        // The single maintain is the loop-bottom one: after the tool turn's
        // build, before the final text turn's build.
        let m = calls.iter().position(|c| *c == "maintain").unwrap();
        let last_b = calls.iter().rposition(|c| *c == "build").unwrap();
        assert!(
            m < last_b,
            "loop-bottom maintain precedes the final turn's build: {calls:?}"
        );
    }

    #[tokio::test]
    async fn overflow_compacts_rebuilds_and_recovers_once() {
        let ws = std::env::temp_dir();
        let model = std::sync::Arc::new(ScriptedModel::new(vec![
            Scripted::Fail(ModelError::Status {
                code: 400,
                body: "maximum context length exceeded".into(),
                retry_after: None,
            }),
            Scripted::Text("recovered after compaction".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 0, // proving overflow recovery does NOT consume retry budget
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = OverflowCtx {
            history: vec![],
            compaction_requests: 0,
            maintains: 0,
        };
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert_eq!(ctx.compaction_requests, 1);
        assert!(ctx.maintains >= 1);
        let events = sink.events.lock().unwrap().clone();
        assert!(
            events.iter().any(|e| e == "overflow_recovery"),
            "recovery must be observable as a context event: {events:?}"
        );
        let usages: Vec<&String> = events.iter().filter(|e| e.starts_with("usage:")).collect();
        assert!(
            usages.len() >= 2,
            "expected pre-request + post-rebuild Usage events: {events:?}"
        );
        assert_eq!(events.last().map(String::as_str), Some("done"));
    }

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
            self.log
                .lock()
                .unwrap()
                .push(format!("{}:model_enter", self.name));
            let r = next.run(req).await;
            self.log
                .lock()
                .unwrap()
                .push(format!("{}:model_exit", self.name));
            r
        }
        async fn wrap_tool_call(
            &self,
            call: agent_tools::ToolCall,
            next: crate::ToolNext<'_>,
        ) -> crate::Executed {
            self.log
                .lock()
                .unwrap()
                .push(format!("{}:tool_enter", self.name));
            let r = next.run(call.args).await;
            self.log
                .lock()
                .unwrap()
                .push(format!("{}:tool_exit", self.name));
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
            Arc::new(Wrapping {
                name: "a",
                log: log.clone(),
            }),
            Arc::new(Wrapping {
                name: "b",
                log: log.clone(),
            }),
        ]);
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let l = log.lock().unwrap().clone();
        // Turn 1 model call, then its tool call, then turn 2 model call.
        assert_eq!(
            l,
            vec![
                "a:model_enter",
                "b:model_enter",
                "b:model_exit",
                "a:model_exit",
                "a:tool_enter",
                "b:tool_enter",
                "b:tool_exit",
                "a:tool_exit",
                "a:model_enter",
                "b:model_enter",
                "b:model_exit",
                "a:model_exit",
            ]
        );
    }

    /// The model wrap chain is invoked twice, independently, across an overflow
    /// rebuild (spec §5.1/J3) — pins composition semantics, not signatures.
    /// Mirrors the ScriptedModel arrangement of
    /// `overflow_compacts_rebuilds_and_recovers_once` verbatim, adding the
    /// Wrapping middleware; asserts the log contains exactly TWO
    /// "a:model_enter" entries and the run recovers (same terminal
    /// assertions as the source test).
    #[tokio::test]
    async fn model_wrap_chain_fires_twice_across_overflow_rebuild() {
        let ws = std::env::temp_dir();
        let model = std::sync::Arc::new(ScriptedModel::new(vec![
            Scripted::Fail(ModelError::Status {
                code: 400,
                body: "maximum context length exceeded".into(),
                retry_after: None,
            }),
            Scripted::Text("recovered after compaction".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 0, // proving overflow recovery does NOT consume retry budget
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let agent = agent.with_middleware(vec![Arc::new(Wrapping {
            name: "a",
            log: log.clone(),
        })]);
        let mut ctx = OverflowCtx {
            history: vec![],
            compaction_requests: 0,
            maintains: 0,
        };
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert_eq!(ctx.compaction_requests, 1);
        assert!(ctx.maintains >= 1);
        let events = sink.events.lock().unwrap().clone();
        assert!(
            events.iter().any(|e| e == "overflow_recovery"),
            "recovery must be observable as a context event: {events:?}"
        );
        assert_eq!(events.last().map(String::as_str), Some("done"));
        let enters = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.as_str() == "a:model_enter")
            .count();
        assert_eq!(
            enters, 2,
            "the model wrap chain fires once per completion_with_retry \
             invocation: the overflowing attempt, then the rebuilt attempt"
        );
    }

    #[tokio::test]
    async fn second_overflow_in_a_turn_is_fatal() {
        let ws = std::env::temp_dir();
        let overflow = || {
            Scripted::Fail(ModelError::Status {
                code: 400,
                body: "maximum context length exceeded".into(),
                retry_after: None,
            })
        };
        let model = std::sync::Arc::new(ScriptedModel::new(vec![overflow(), overflow()]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 3, // unused — overflow skips budget
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = OverflowCtx {
            history: vec![],
            compaction_requests: 0,
            maintains: 0,
        };
        let err = agent.run(&mut ctx, "go".into()).await.unwrap_err();
        assert!(matches!(err, AgentError::Model(_)));
        assert_eq!(
            ctx.compaction_requests, 1,
            "recovery attempted exactly once"
        );
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(events.last().map(String::as_str), Some("done"));
        assert!(events.iter().any(|k| k.starts_with("error")));
    }

    #[tokio::test]
    async fn process_overflow_recovers_like_status_overflow() {
        // claude-cli surfaces overflow as Process stderr text (no status code);
        // recovery must fire exactly as it does for Status{400}.
        let ws = std::env::temp_dir();
        let model = std::sync::Arc::new(ScriptedModel::new(vec![
            Scripted::Fail(ModelError::Process(
                "claude exited (1): maximum context length exceeded".into(),
            )),
            Scripted::Text("recovered after compaction".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 0, // recovery must not consume retry budget
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = OverflowCtx {
            history: vec![],
            compaction_requests: 0,
            maintains: 0,
        };
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert_eq!(ctx.compaction_requests, 1);
        assert_eq!(
            sink.events.lock().unwrap().last().map(String::as_str),
            Some("done")
        );
    }

    /// A mid-stream failure that already emitted chunks must retract them before
    /// the retry re-streams (spec §2); a clean or chunk-less failure must not.
    #[tokio::test(start_paused = true)]
    async fn stream_retry_retracts_partial_output() {
        let ws = std::env::temp_dir();
        // Attempt 1 streams "ab","cd" (4 chars) then the stream dies (retryable);
        // attempt 2 succeeds with "xy".
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::ChunksThenFail(
                vec![Chunk::Text("ab".into()), Chunk::Text("cd".into())],
                ModelError::Http("mid-stream drop".into()),
            ),
            Scripted::Text("xy".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let events = sink.events.lock().unwrap().clone();
        // The retraction carries the exact char counts and sits between the
        // abandoned partial and the re-streamed tokens.
        let sr = events
            .iter()
            .position(|e| e == "stream_retry:4:0")
            .unwrap_or_else(|| panic!("expected stream_retry:4:0; got {events:?}"));
        let ab = events.iter().position(|e| e == "token:ab").unwrap();
        let cd = events.iter().position(|e| e == "token:cd").unwrap();
        let xy = events.iter().position(|e| e == "token:xy").unwrap();
        assert!(
            ab < sr && cd < sr,
            "abandoned partial precedes the retraction: {events:?}"
        );
        assert!(sr < xy, "the re-stream follows the retraction: {events:?}");
        // Exactly one retraction for one mid-stream failure.
        assert_eq!(
            events
                .iter()
                .filter(|e| e.starts_with("stream_retry"))
                .count(),
            1
        );
        assert_eq!(events.last().map(String::as_str), Some("done"));
    }

    #[tokio::test(start_paused = true)]
    async fn no_stream_retry_when_nothing_emitted() {
        let ws = std::env::temp_dir();
        // Attempt 1 fails before emitting any chunk; attempt 2 succeeds. Nothing
        // streamed, so nothing to retract.
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Fail(ModelError::Http("down".into())),
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let events = sink.events.lock().unwrap().clone();
        assert!(
            !events.iter().any(|e| e.starts_with("stream_retry")),
            "no retraction when nothing was emitted: {events:?}"
        );
        assert_eq!(events.last().map(String::as_str), Some("done"));
    }

    /// The once-per-turn overflow rebuild is also a "retry after a partial":
    /// the partial answer is retracted before the rebuilt attempt re-streams,
    /// and the retraction precedes the OverflowRecovery marker.
    #[tokio::test]
    async fn stream_retry_retracts_partial_before_overflow_rebuild() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::ChunksThenFail(
                vec![Chunk::Text("part".into())], // 4 chars
                ModelError::Status {
                    code: 400,
                    body: "maximum context length exceeded".into(),
                    retry_after: None,
                },
            ),
            Scripted::Text("done".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 0, // overflow recovery must not consume retry budget
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = OverflowCtx {
            history: vec![],
            compaction_requests: 0,
            maintains: 0,
        };
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let events = sink.events.lock().unwrap().clone();
        let sr = events
            .iter()
            .position(|e| e == "stream_retry:4:0")
            .unwrap_or_else(|| panic!("expected stream_retry:4:0; got {events:?}"));
        let or = events
            .iter()
            .position(|e| e == "overflow_recovery")
            .unwrap_or_else(|| panic!("expected overflow_recovery; got {events:?}"));
        assert!(
            sr < or,
            "retraction must precede the overflow rebuild: {events:?}"
        );
        assert_eq!(events.last().map(String::as_str), Some("done"));
    }

    /// A sink that keeps full per-event detail (ids, status, content, done
    /// reason) — the shared `CollectingSink` collapses events to
    /// `name:status` strings, too lossy for the per-call isolation asserts.
    #[derive(Default)]
    struct DetailSink {
        tool_starts: std::sync::Mutex<Vec<(String, String)>>, // (id, name)
        tool_results: std::sync::Mutex<Vec<(String, ToolStatus, String)>>, // (id, status, content)
        errors: std::sync::Mutex<Vec<String>>,
        done: std::sync::Mutex<Option<StopReason>>,
    }
    impl EventSink for DetailSink {
        fn emit(&self, event: AgentEvent) {
            match event {
                AgentEvent::ToolStart { id, name, .. } => {
                    self.tool_starts.lock().unwrap().push((id, name));
                }
                AgentEvent::ToolResult {
                    id, status, output, ..
                } => {
                    self.tool_results
                        .lock()
                        .unwrap()
                        .push((id, status, output.content));
                }
                AgentEvent::Error(e) => self.errors.lock().unwrap().push(e),
                AgentEvent::Done(r) => *self.done.lock().unwrap() = Some(r),
                _ => {}
            }
        }
    }

    /// One malformed call must not discard the turn: good calls execute, the bad
    /// one gets a per-call ERROR result, and the assistant message keeps all ids.
    #[tokio::test]
    async fn malformed_call_isolated_good_calls_execute() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "BODY").unwrap();
        let ws = dir.path().to_path_buf();
        // Turn 1: one good read_file call + one bad-args call (unparseable JSON).
        // Turn 2: plain text stop.
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                (
                    "c_good".into(),
                    "read_file".into(),
                    r#"{"path":"a.txt"}"#.into(),
                ),
                ("c_bad".into(), "read_file".into(), "{not json".into()),
            ]),
            Scripted::Text("done".into()),
        ]));
        let sink = Arc::new(DetailSink::default());
        let agent = AgentLoop::new(
            model,
            // The native protocol is the one that fills `parsed.invalid`.
            Arc::new(agent_model::NativeProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "read twice".into()).await.unwrap();

        // Both ids emitted a ToolStart.
        let starts = sink.tool_starts.lock().unwrap().clone();
        let start_ids: std::collections::HashSet<&str> =
            starts.iter().map(|(id, _)| id.as_str()).collect();
        assert!(
            start_ids.contains("c_good") && start_ids.contains("c_bad"),
            "expected ToolStart for both ids, got {starts:?}"
        );

        // The good call actually executed (Ok result); the bad one is an Error
        // result whose content is the per-call re-emit guidance.
        let results = sink.tool_results.lock().unwrap().clone();
        assert!(
            results
                .iter()
                .any(|(id, st, _)| id == "c_good" && *st == ToolStatus::Ok),
            "good call must run to an Ok ToolResult, got {results:?}"
        );
        let bad = results
            .iter()
            .find(|(id, _, _)| id == "c_bad")
            .expect("bad call must produce a ToolResult");
        assert_eq!(bad.1, ToolStatus::Error, "bad call result must be Error");
        assert!(
            bad.2.contains("could not be parsed") && bad.2.contains("re-emit only this call"),
            "bad-call content must be the per-call re-emit guidance, got: {}",
            bad.2
        );

        // Run ended normally, NOT via the whole-turn protocol-repair path.
        assert_eq!(*sink.done.lock().unwrap(), Some(StopReason::Stop));

        // The assistant message in history carries BOTH ids.
        let built = ctx.build(100_000);
        let assistant_ids: Vec<String> = built
            .iter()
            .filter(|m| matches!(m.role, agent_model::Role::Assistant))
            .filter_map(|m| m.tool_calls.as_ref())
            .flat_map(|calls| calls.iter().map(|c| c.id.clone()))
            .collect();
        assert!(
            assistant_ids.iter().any(|id| id == "c_good")
                && assistant_ids.iter().any(|id| id == "c_bad"),
            "assistant message must keep both tool-call ids, got {assistant_ids:?}"
        );
        // No whole-turn protocol-repair user message was appended.
        assert!(
            !built
                .iter()
                .any(|m| m.content.contains("Re-emit it correctly")),
            "a malformed call must not trigger the whole-turn repair prompt"
        );
    }

    /// A call truncated by max_tokens (Ok-parse with a non-empty `invalid` and
    /// stop == Length) must take the max_tokens truncation path, not the
    /// per-call re-emit error — and execute nothing.
    #[tokio::test]
    async fn malformed_call_length_stop_takes_truncation_path() {
        let ws = std::env::temp_dir();
        // A single tool call cut off mid-args with a `length` finish reason. The
        // native protocol yields Ok-with-invalid (bad JSON) + stop == Length.
        let truncated = r##"{"path":"big.py","content":"#!/usr/bin/env python3\nprint('hi"##;
        let model = Arc::new(ScriptedModel::new(vec![Scripted::TruncatedCall(
            "write_file".into(),
            truncated.into(),
        )]));
        let sink = Arc::new(DetailSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(agent_model::NativeProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: Some(2048),
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent
            .run(&mut ctx, "write a big file".into())
            .await
            .unwrap();

        // The terminal event is Done(Length), and the error explains truncation.
        assert_eq!(*sink.done.lock().unwrap(), Some(StopReason::Length));
        let errors = sink.errors.lock().unwrap().clone();
        assert!(
            errors.iter().any(|e| {
                let low = e.to_lowercase();
                low.contains("max_tokens") || low.contains("truncat")
            }),
            "expected a max_tokens/truncation error, got {errors:?}"
        );
        // No tool ran and no per-call re-emit error was emitted.
        assert!(
            sink.tool_starts.lock().unwrap().is_empty(),
            "the Length-truncation path must not start any tool"
        );
        assert!(
            sink.tool_results.lock().unwrap().is_empty(),
            "the Length-truncation path must not emit a tool result"
        );
    }

    // ---- Server-usage-calibrated effective model limit (spec 2026-07-02 §1) ----

    /// A ContextManager that records the `model_limit` passed to every `build`
    /// call and returns a FIXED canned message list, so `built_tokens` of each
    /// request is deterministic — the denominator of the calibration sample.
    struct RecordingContext {
        system: Message,
        canned: Vec<Message>,
        builds: Arc<Mutex<Vec<usize>>>,
    }
    impl ContextManager for RecordingContext {
        fn append(&mut self, _msg: Message) {}
        fn set_system(&mut self, system: Message) {
            self.system = system;
        }
        fn build(&self, model_limit: usize) -> Vec<Message> {
            self.builds.lock().unwrap().push(model_limit);
            let mut out = Vec::with_capacity(self.canned.len() + 1);
            out.push(self.system.clone());
            out.extend(self.canned.iter().cloned());
            out
        }
    }

    /// Captures the `context_limit` field of every `Usage` event (the string
    /// CollectingSink can't carry it).
    #[derive(Default)]
    struct UsageLimitSink {
        limits: Mutex<Vec<usize>>,
    }
    impl EventSink for UsageLimitSink {
        fn emit(&self, event: AgentEvent) {
            if let AgentEvent::Usage { context_limit, .. } = event {
                self.limits.lock().unwrap().push(context_limit);
            }
        }
    }

    /// One assistant turn: a `counter` tool call (distinct args per `n` so the
    /// stuck-repeat detector never trips across the many calibration turns) plus a
    /// server `Usage` chunk reporting `prompt_tokens`.
    fn tool_turn_with_usage(n: usize, prompt_tokens: u32) -> Scripted {
        Scripted::Chunks(vec![
            Chunk::ToolCallDelta(RawToolCall {
                index: Some(0),
                id: Some(format!("c{n}")),
                name: Some("counter".into()),
                args_fragment: format!("{{\"n\":{n}}}"),
            }),
            Chunk::Usage {
                prompt_tokens,
                completion_tokens: 0,
                reasoning_tokens: None,
                cached_tokens: None,
                cost_usd: None,
            },
            Chunk::Done(StopReason::ToolCalls),
        ])
    }

    fn text_turn() -> Scripted {
        Scripted::Chunks(vec![
            Chunk::Text("done".into()),
            Chunk::Done(StopReason::Stop),
        ])
    }

    fn calib_agent(model: Arc<ScriptedModel>, sink: Arc<dyn EventSink>) -> AgentLoop {
        let ws = std::env::temp_dir();
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(Counter(Arc::new(
            std::sync::atomic::AtomicUsize::new(0),
        ))));
        AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            Arc::new(reg),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink,
            LoopConfig {
                model_limit: 100_000,
                max_turns: 20,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        )
    }

    /// The deterministic chars/4 estimate the loop computes for the canned request.
    fn canned_est(system: &Message, canned: &[Message]) -> usize {
        let mut all = vec![system.clone()];
        all.extend(canned.iter().cloned());
        built_tokens(&all)
    }

    fn recording_ctx(builds: Arc<Mutex<Vec<usize>>>) -> (RecordingContext, Message, Vec<Message>) {
        let system = Message::system("sys");
        let canned = vec![Message::user("x".repeat(400))];
        let ctx = RecordingContext {
            system: system.clone(),
            canned: canned.clone(),
            builds,
        };
        (ctx, system, canned)
    }

    /// Server-reported prompt_tokens 2× our estimate must shrink the NEXT turn's
    /// build budget (EMA 0.5: 1.0 → 1.5), never the Usage event's configured
    /// context_limit (spec §1).
    #[tokio::test]
    async fn server_usage_shrinks_effective_budget() {
        let builds = Arc::new(Mutex::new(Vec::new()));
        let (mut ctx, system, canned) = recording_ctx(builds.clone());
        let est = canned_est(&system, &canned);

        let model = Arc::new(ScriptedModel::new(vec![
            tool_turn_with_usage(1, (2 * est) as u32), // turn 1: server says 2× est
            text_turn(),                               // turn 2: plain-text stop
        ]));
        let sink: Arc<UsageLimitSink> = Arc::new(UsageLimitSink::default());
        let agent = calib_agent(model, sink.clone());
        agent.run(&mut ctx, "hi".into()).await.unwrap();

        let recorded = builds.lock().unwrap().clone();
        assert_eq!(recorded.len(), 2, "one build per turn: {recorded:?}");
        assert_eq!(
            recorded[0], 100_000,
            "turn 1 builds at the configured limit"
        );
        let expected = (100_000f64 / 1.5) as usize; // 66_666
        assert!(
            (recorded[1] as i64 - expected as i64).abs() <= 1,
            "turn 2 build shrinks to ~model_limit/1.5, got {}",
            recorded[1]
        );

        // Usage EVENTS always carry the CONFIGURED limit (display truth).
        let limits = sink.limits.lock().unwrap().clone();
        assert!(!limits.is_empty(), "at least one Usage event expected");
        assert!(
            limits.iter().all(|&l| l == 100_000),
            "Usage events must carry the configured limit: {limits:?}"
        );
    }

    /// A backend that reports no usage leaves the budget exactly configured.
    #[tokio::test]
    async fn zero_prompt_tokens_leaves_budget_configured() {
        let builds = Arc::new(Mutex::new(Vec::new()));
        let (mut ctx, ..) = recording_ctx(builds.clone());

        let model = Arc::new(ScriptedModel::new(vec![
            tool_turn_with_usage(1, 0), // server reports 0 → no sample
            text_turn(),
        ]));
        let sink: Arc<UsageLimitSink> = Arc::new(UsageLimitSink::default());
        let agent = calib_agent(model, sink);
        agent.run(&mut ctx, "hi".into()).await.unwrap();

        let recorded = builds.lock().unwrap().clone();
        assert!(
            recorded.iter().all(|&l| l == 100_000),
            "zero prompt_tokens keeps the configured limit every turn: {recorded:?}"
        );
    }

    /// A wildly-high sample clamps the shrink at 4× (floor = model_limit/4), and a
    /// run of low samples decays the ratio back toward 1.0 — never above the
    /// configured limit.
    #[tokio::test]
    async fn calibration_clamps_at_4x_and_never_grows() {
        let builds = Arc::new(Mutex::new(Vec::new()));
        let (mut ctx, system, canned) = recording_ctx(builds.clone());
        let est = canned_est(&system, &canned);

        let mut turns = vec![tool_turn_with_usage(0, (100 * est) as u32)]; // 100× → clamp 4.0
        for i in 1..=5 {
            turns.push(tool_turn_with_usage(i, (est / 10) as u32)); // ~0.1× samples
        }
        turns.push(text_turn());
        let model = Arc::new(ScriptedModel::new(turns));
        let sink: Arc<UsageLimitSink> = Arc::new(UsageLimitSink::default());
        let agent = calib_agent(model, sink);
        agent.run(&mut ctx, "hi".into()).await.unwrap();

        let recorded = builds.lock().unwrap().clone();
        assert_eq!(recorded[0], 100_000, "turn 1 uncalibrated: {recorded:?}");
        assert_eq!(
            recorded[1],
            100_000 / 4,
            "a 100× sample clamps the ratio at 4.0 → build floor at model_limit/4: {recorded:?}"
        );
        assert!(
            recorded.iter().all(|&l| l <= 100_000),
            "the effective limit never exceeds the configured window: {recorded:?}"
        );
        assert!(
            recorded.iter().all(|&l| l >= 100_000 / 4),
            "the effective limit never drops below the model_limit/4 floor: {recorded:?}"
        );
        assert_eq!(
            *recorded.last().unwrap(),
            100_000,
            "low samples decay the ratio back to 1.0 (configured limit): {recorded:?}"
        );
    }

    // ---- post-execution validator hook (Task 2) ----------------------------

    /// A deterministic Write-tier tool: intent declares `Access::Write`, execute
    /// always succeeds. Lets a validator test drive a "mutating turn" without any
    /// real fs coupling. Mirrors `HangsUntilCancel`'s trait shape.
    struct WriteStub;
    #[async_trait::async_trait]
    impl Tool for WriteStub {
        fn name(&self) -> &str {
            "write_stub"
        }
        fn description(&self) -> &str {
            "writes (test stub)"
        }
        fn schema(&self) -> agent_tools::ToolSchema {
            agent_tools::ToolSchema {
                name: "write_stub".into(),
                description: "".into(),
                parameters: serde_json::json!({"type":"object"}),
            }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<agent_tools::ToolIntent, ToolError> {
            Ok(agent_tools::ToolIntent {
                tool: "write_stub".into(),
                access: agent_tools::Access::Write,
                paths: vec![],
                command: None,
                summary: "write".into(),
            })
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> Result<agent_tools::ToolOutput, ToolError> {
            Ok(agent_tools::ToolOutput {
                content: "wrote".into(),
                display: None,
            })
        }
    }

    fn write_stub_registry() -> Arc<ToolRegistry> {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(WriteStub));
        Arc::new(r)
    }

    /// A Write-tier tool whose `execute` FAILS. Same `Access::Write` intent as
    /// `WriteStub` (so the gate treats it as a mutation) but returns `Err`, so the
    /// call never becomes `Executed::Ok` and must NOT set `turn_mutated`. Named
    /// `write_stub` so it drops into `validator_loop`'s scripted call.
    struct FailStub;
    #[async_trait::async_trait]
    impl Tool for FailStub {
        fn name(&self) -> &str {
            "write_stub"
        }
        fn description(&self) -> &str {
            "writes then fails (test stub)"
        }
        fn schema(&self) -> agent_tools::ToolSchema {
            agent_tools::ToolSchema {
                name: "write_stub".into(),
                description: "".into(),
                parameters: serde_json::json!({"type":"object"}),
            }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<agent_tools::ToolIntent, ToolError> {
            Ok(agent_tools::ToolIntent {
                tool: "write_stub".into(),
                access: agent_tools::Access::Write,
                paths: vec![],
                command: None,
                summary: "write".into(),
            })
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> Result<agent_tools::ToolOutput, ToolError> {
            Err(ToolError::Failed {
                message: "write failed".into(),
                stderr: None,
            })
        }
    }

    fn fail_stub_registry() -> Arc<ToolRegistry> {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(FailStub));
        Arc::new(r)
    }

    /// A Write-tier tool that succeeds (setting `turn_mutated`) but cancels the run
    /// token from inside `execute`. By the time the validator phase runs, the token
    /// is already cancelled, so `run_validator` short-circuits to `Skipped` without
    /// ever launching the (blocking) command. Named `write_stub` for `validator_loop`.
    struct WriteStubSelfCancel;
    #[async_trait::async_trait]
    impl Tool for WriteStubSelfCancel {
        fn name(&self) -> &str {
            "write_stub"
        }
        fn description(&self) -> &str {
            "writes then cancels the run (test stub)"
        }
        fn schema(&self) -> agent_tools::ToolSchema {
            agent_tools::ToolSchema {
                name: "write_stub".into(),
                description: "".into(),
                parameters: serde_json::json!({"type":"object"}),
            }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<agent_tools::ToolIntent, ToolError> {
            Ok(agent_tools::ToolIntent {
                tool: "write_stub".into(),
                access: agent_tools::Access::Write,
                paths: vec![],
                command: None,
                summary: "write".into(),
            })
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
            ctx: &ToolCtx,
        ) -> Result<agent_tools::ToolOutput, ToolError> {
            // Cancel the live run token (a Ctrl-C mid-turn, after the mutation
            // succeeded). The validator phase must then skip cleanly, never blocking.
            ctx.cancel.cancel();
            Ok(agent_tools::ToolOutput {
                content: "wrote".into(),
                display: None,
            })
        }
    }

    /// A TrustedWrite-tier stub (the MCP Trust::Allow encoding): auto-allowed
    /// at the gate, but a successful call must count as a mutation and trigger
    /// configured validators (audit 5.1).
    struct TrustedStub;
    #[async_trait::async_trait]
    impl Tool for TrustedStub {
        fn name(&self) -> &str {
            "trusted_stub"
        }
        fn description(&self) -> &str {
            "trusted third-party stub"
        }
        fn schema(&self) -> agent_tools::ToolSchema {
            agent_tools::ToolSchema {
                name: "trusted_stub".into(),
                description: "".into(),
                parameters: serde_json::json!({"type":"object"}),
            }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<agent_tools::ToolIntent, ToolError> {
            Ok(agent_tools::ToolIntent {
                tool: "trusted_stub".into(),
                access: agent_tools::Access::TrustedWrite,
                paths: vec![],
                command: None,
                summary: "trusted mutation".into(),
            })
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> Result<agent_tools::ToolOutput, ToolError> {
            Ok(agent_tools::ToolOutput {
                content: "did".into(),
                display: None,
            })
        }
    }

    fn self_cancel_registry() -> Arc<ToolRegistry> {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(WriteStubSelfCancel));
        Arc::new(r)
    }

    /// Build a loop that scripts one call then finishes, with the
    /// given validator list. Returns (agent, sink).
    fn validator_loop(
        ws: std::path::PathBuf,
        validators: Vec<String>,
        registry: Arc<ToolRegistry>,
        tool_name: &str,
    ) -> (AgentLoop, Arc<DetailSink>) {
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call("c1".into(), tool_name.into(), "{}".into()),
            Scripted::Text("done".into()),
        ]));
        let sink = Arc::new(DetailSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry,
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                post_tool_validators: validators,
                ..Default::default()
            },
        );
        (agent, sink)
    }

    fn appended_validation_msg(ctx: &WindowContext) -> Option<String> {
        ctx.build(100_000)
            .into_iter()
            .find(|m| {
                matches!(m.role, agent_model::Role::User)
                    && m.content.contains("Post-edit validation reported problems")
            })
            .map(|m| m.content)
    }

    fn validate_events(sink: &DetailSink) -> Vec<(String, ToolStatus, String)> {
        sink.tool_results
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, _, _)| id.starts_with("validate:"))
            .cloned()
            .collect()
    }

    /// A write-tier tool succeeds, then a failing validator -> a validation
    /// message is appended AND a `post_tool_validate` Error event is emitted.
    #[tokio::test]
    async fn failing_validator_appends_feedback_and_emits_event() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let (agent, sink) = validator_loop(
            ws,
            vec!["echo boom 1>&2; exit 3".into()],
            write_stub_registry(),
            "write_stub",
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        let ev = validate_events(&sink);
        assert_eq!(
            ev.len(),
            1,
            "one post_tool_validate result expected: {ev:?}"
        );
        assert_eq!(ev[0].1, ToolStatus::Error, "failing validator -> Error");
        assert!(
            ev[0].2.contains("exit 3") && ev[0].2.contains("boom"),
            "event content carries exit code + output: {}",
            ev[0].2
        );

        let appended = appended_validation_msg(&ctx).expect("validation feedback appended");
        assert!(
            appended.contains("boom") && appended.contains("exit 3"),
            "appended user message carries the failure: {appended}"
        );
    }

    /// A passing validator emits an Ok event but appends nothing to context.
    #[tokio::test]
    async fn passing_validator_emits_event_but_appends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let (agent, sink) =
            validator_loop(ws, vec!["true".into()], write_stub_registry(), "write_stub");
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        let ev = validate_events(&sink);
        assert_eq!(
            ev.len(),
            1,
            "one post_tool_validate result expected: {ev:?}"
        );
        assert_eq!(ev[0].1, ToolStatus::Ok, "passing validator -> Ok");
        assert_eq!(
            ev[0].2, "validator passed",
            "a passing validator's Ok event carries the canonical message: {}",
            ev[0].2
        );
        assert!(
            appended_validation_msg(&ctx).is_none(),
            "a passing validator appends no feedback message"
        );
    }

    /// A read-only turn (the only call is a read_file) never runs validators.
    #[tokio::test]
    async fn read_only_turn_does_not_run_validators() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "BODY").unwrap();
        let ws = dir.path().to_path_buf();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "read_file".into(),
                r#"{"path":"a.txt"}"#.into(),
            ),
            Scripted::Text("done".into()),
        ]));
        let sink = Arc::new(DetailSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                post_tool_validators: vec!["false".into()],
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        assert!(
            validate_events(&sink).is_empty(),
            "read-only turn must emit no post_tool_validate events"
        );
        assert!(appended_validation_msg(&ctx).is_none());
    }

    /// TrustedWrite success = mutating turn: configured validators must run
    /// (this is the audit-5.1 regression pin — Trust::Allow MCP mutations were
    /// invisible to the validator while encoded as Access::Read).
    #[tokio::test]
    async fn trusted_write_turn_runs_validators() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let mut r = ToolRegistry::new();
        r.register(Arc::new(TrustedStub));
        let (agent, sink) = validator_loop(ws, vec!["true".into()], Arc::new(r), "trusted_stub");
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        let ev = validate_events(&sink);
        assert_eq!(
            ev.len(),
            1,
            "TrustedWrite success must trigger validators: {ev:?}"
        );
        assert_eq!(ev[0].1, ToolStatus::Ok);
    }

    /// An empty validator list is a strict no-op: a write-tier call succeeds and
    /// still ZERO validator events fire (regression guard).
    #[tokio::test]
    async fn empty_validators_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let (agent, sink) = validator_loop(ws, vec![], write_stub_registry(), "write_stub");
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        assert!(
            validate_events(&sink).is_empty(),
            "empty validator list must emit no post_tool_validate events"
        );
        assert!(appended_validation_msg(&ctx).is_none());
        // The write tool itself still ran (Ok result) — the turn was unaffected.
        let results = sink.tool_results.lock().unwrap().clone();
        assert!(
            results
                .iter()
                .any(|(id, st, _)| id == "c1" && *st == ToolStatus::Ok),
            "the write call still executed normally: {results:?}"
        );
    }

    /// A mutating call that FAILS does not set `turn_mutated`, so validators never
    /// run — guards the `matches!(ex, Executed::Ok(_))` trigger against refactors.
    #[tokio::test]
    async fn failed_mutation_does_not_trigger_validators() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        // FailStub declares Access::Write but its execute returns Err; a validator
        // that would always fail ("false") proves nothing fires on a failed mutation.
        let (agent, sink) =
            validator_loop(ws, vec!["false".into()], fail_stub_registry(), "write_stub");
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        assert!(
            validate_events(&sink).is_empty(),
            "a failed mutation must emit no post_tool_validate events"
        );
        assert!(
            appended_validation_msg(&ctx).is_none(),
            "a failed mutation appends no validation feedback"
        );
    }

    /// Two validators where the first passes and the second fails: BOTH events fire
    /// (Ok then Error) and the single appended feedback names only the failing one.
    #[tokio::test]
    async fn multiple_validators_first_passes_second_fails() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let (agent, sink) = validator_loop(
            ws,
            vec!["true".into(), "false".into()],
            write_stub_registry(),
            "write_stub",
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        let ev = validate_events(&sink);
        assert_eq!(ev.len(), 2, "both validators emit a result: {ev:?}");
        assert_eq!(ev[0].1, ToolStatus::Ok, "first validator passes -> Ok");
        assert_eq!(
            ev[0].2, "validator passed",
            "passing event message: {}",
            ev[0].2
        );
        assert_eq!(
            ev[1].1,
            ToolStatus::Error,
            "second validator fails -> Error"
        );
        assert!(
            ev[1].2.contains("exit 1"),
            "failing event carries its exit code: {}",
            ev[1].2
        );

        let appended = appended_validation_msg(&ctx).expect("validation feedback appended");
        assert!(
            appended.contains("false") && appended.contains("exit 1"),
            "appended message names the failing command + exit: {appended}"
        );
        assert!(
            !appended.contains("true"),
            "appended message must not mention the passing command: {appended}"
        );
    }

    /// A run cancelled mid-turn (after the mutation succeeds) skips the validator
    /// phase cleanly: the run returns without hanging on the blocking command, the
    /// validator outcome is a Skip, and no failure feedback is appended.
    #[tokio::test]
    async fn cancelled_run_skips_validators_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        // WriteStubSelfCancel succeeds (turn_mutated=true) then cancels the run
        // token. "sleep 30" would hang for 30s if ever launched; a clean skip proves
        // it never is (run_validator short-circuits on the cancelled token).
        let (agent, sink) = validator_loop(
            ws,
            vec!["sleep 30".into()],
            self_cancel_registry(),
            "write_stub",
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        // Returning at all (well under 30s) proves the validator did not block.
        agent.run(&mut ctx, "go".into()).await.unwrap();

        let ev = validate_events(&sink);
        assert_eq!(ev.len(), 1, "the validator is reached but skips: {ev:?}");
        assert_eq!(
            ev[0].1,
            ToolStatus::Error,
            "a skip surfaces as an Error event"
        );
        assert!(
            ev[0].2.contains("skipped"),
            "a cancelled validator is reported as skipped: {}",
            ev[0].2
        );
        assert!(
            appended_validation_msg(&ctx).is_none(),
            "a skipped validator appends no failure feedback"
        );
    }

    /// `run_validator` caps combined output on a char boundary and marks the cut.
    #[tokio::test]
    async fn validator_helper_truncates_large_output() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox: Arc<dyn agent_tools::SandboxStrategy> = Arc::new(agent_tools::HostExecutor);
        let cancel = CancellationToken::new();
        let outcome = run_validator(
            &sandbox,
            dir.path(),
            "yes x | head -c 20000; exit 3",
            std::time::Duration::from_secs(5),
            &cancel,
        )
        .await;
        match outcome {
            ValidatorOutcome::Failed { code, output } => {
                assert_eq!(code, 3);
                assert!(
                    output.len() <= VALIDATOR_OUTPUT_CAP + 20,
                    "output capped near VALIDATOR_OUTPUT_CAP, got {} bytes",
                    output.len()
                );
                assert!(
                    output.ends_with("(truncated)"),
                    "truncated output carries the marker: {:?}",
                    &output[output.len().saturating_sub(32)..]
                );
            }
            other => panic!(
                "expected Failed with capped output, got a different outcome: {}",
                match other {
                    ValidatorOutcome::Passed => "Passed",
                    ValidatorOutcome::Skipped { .. } => "Skipped",
                    ValidatorOutcome::Failed { .. } => unreachable!(),
                }
            ),
        }
    }

    #[test]
    fn tool_schemas_exposes_registered_tools() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let l = AgentLoop::new(
            Arc::new(ScriptedModel::new(vec![])),
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            Arc::new(CollectingSink::default()),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 1,
                max_retries: 0,
                temperature: 0.0,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let names: Vec<String> = l.tool_schemas().into_iter().map(|s| s.name).collect();
        assert!(
            !names.is_empty(),
            "fixture loop registers at least one tool"
        );
    }

    // --- Task 2: middleware node-hook plumbing -----------------------------

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
            self.log
                .lock()
                .unwrap()
                .push(format!("{}:run_start", self.name));
            self.flow("run_start")
        }
        async fn after_model(
            &self,
            _cx: &mut crate::RunCx<'_>,
            _t: &crate::TurnView,
        ) -> crate::Flow {
            self.log
                .lock()
                .unwrap()
                .push(format!("{}:after_model", self.name));
            self.flow("after_model")
        }
        async fn after_tools(&self, _cx: &mut crate::RunCx<'_>) -> crate::Flow {
            self.log
                .lock()
                .unwrap()
                .push(format!("{}:after_tools", self.name));
            self.flow("after_tools")
        }
        async fn on_turn_end(&self, _cx: &mut crate::RunCx<'_>) -> crate::Flow {
            self.log
                .lock()
                .unwrap()
                .push(format!("{}:turn_end", self.name));
            self.flow("turn_end")
        }
        async fn after_final_reply(&self, _cx: &mut crate::RunCx<'_>) {
            self.log
                .lock()
                .unwrap()
                .push(format!("{}:final_reply", self.name));
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

    #[allow(clippy::type_complexity)]
    fn recording_pair(
        end_at: Option<&'static str>,
    ) -> (
        Vec<Arc<dyn crate::Middleware>>,
        Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let a = Recording {
            name: "a",
            log: log.clone(),
            end_at: None,
        };
        let b = Recording {
            name: "b",
            log: log.clone(),
            end_at,
        };
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
                "a:run_start",
                "b:run_start", // forward
                "b:after_model",
                "a:after_model", // reverse (turn 1)
                "b:after_tools",
                "a:after_tools", // reverse
                "b:turn_end",
                "a:turn_end", // reverse
                "b:after_model",
                "a:after_model", // turn 2 (text-only)
                "b:final_reply",
                "a:final_reply", // reverse, text exit
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
        assert_eq!(
            log.lock().unwrap().clone(),
            vec!["a:run_start", "b:run_start"]
        );
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(events.last().unwrap(), "done");
    }

    /// Spec §7 gap-fill (missing from the pre-Task-7 tree): an `EndRun` fired
    /// from `after_tools` — AFTER the turn's real tool-call/tool-result pair
    /// is already durably appended — must end the run cleanly with no
    /// orphaned tool_calls message. Stack is `[a, b]`: `b` (reverse-order
    /// first) EndRuns at `after_tools`, so `a`'s `after_tools` (and — in a
    /// real assembled stack — StuckDetection's, the hook that would flush any
    /// pending nudge) never fires this turn; the invariant under test is
    /// `assert_no_orphans`'s guarantee holding across the short-circuit, not
    /// any nudge content (this fixture's `counter_agent` replaces the whole
    /// stack via `with_middleware`, so no StuckDetection instance is even
    /// present here). See `end_run_from_after_model_drops_pending_nudge_without_orphans`
    /// for the sibling case where a real `StuckDetectionMiddleware` instance
    /// IS present and an `EndRun` fires from `after_model` instead.
    #[tokio::test]
    async fn end_run_from_after_tools_leaves_no_orphans() {
        let model = Arc::new(ScriptedModel::new(vec![Scripted::Call(
            "c1".into(),
            "counter".into(),
            r#"{"k":"a"}"#.into(),
        )]));
        let sink = Arc::new(CollectingSink::default());
        let (agent, count) = counter_agent(model, sink.clone(), 5);
        let (stack, log) = recording_pair(Some("after_tools"));
        let agent = agent.with_middleware(stack);
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            log.lock().unwrap().clone(),
            vec![
                "a:run_start",
                "b:run_start",
                "b:after_model",
                "a:after_model",
                "b:after_tools"
            ],
            "a:after_tools and everything past it must never fire"
        );
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(events.last().unwrap(), "done");

        use crate::ContextManager;
        let built = ctx.build(100_000);
        assert!(
            crate::orphaned_tool_positions(&built).is_empty(),
            "no orphaned tool_calls message after an after_tools EndRun: {built:?}"
        );
    }

    /// EndRuns from `after_model` on a specific 0-based turn index only —
    /// `Recording::end_at` is unconditional on hook name (it would EndRun on
    /// turn 1 already), and this test needs to let StuckDetection accumulate
    /// repeats across several turns before ending the run on the exact turn
    /// where a nudge goes pending.
    struct EndAtTurn {
        turn: usize,
        log: Arc<std::sync::Mutex<Vec<String>>>,
    }
    #[async_trait::async_trait]
    impl crate::Middleware for EndAtTurn {
        fn name(&self) -> &str {
            "end-at-turn"
        }
        async fn on_run_start(&self, _cx: &mut crate::RunCx<'_>, _i: &str) -> crate::Flow {
            self.log
                .lock()
                .unwrap()
                .push("end_at_turn:run_start".into());
            crate::Flow::Continue
        }
        async fn after_model(
            &self,
            cx: &mut crate::RunCx<'_>,
            _t: &crate::TurnView,
        ) -> crate::Flow {
            self.log
                .lock()
                .unwrap()
                .push(format!("end_at_turn:after_model(turn={:?})", cx.turn));
            if cx.turn == Some(self.turn) {
                crate::Flow::EndRun(StopReason::Error)
            } else {
                crate::Flow::Continue
            }
        }
    }

    /// Sibling of `end_run_from_after_tools_leaves_no_orphans`: this time a
    /// real `StuckDetectionMiddleware` is on the stack, and the `EndRun`
    /// fires one hook earlier — from `after_model` — so the nudge it sets
    /// pending never gets the chance to flush (that only happens in
    /// `after_tools`, which the `EndRun` short-circuits).
    ///
    /// Firing order note: `after_model` dispatches in REVERSE stack order
    /// (`fire_after_model` iterates `.iter().rev()`, confirmed by
    /// `hooks_fire_forward_then_reverse_across_a_tool_turn`). To get
    /// StuckDetection's `after_model` to run BEFORE the `EndAtTurn` recorder's
    /// — so the pending-nudge marker is actually set before the run ends —
    /// StuckDetection must sit DEEPER in the stack vec than the recorder:
    /// `[recorder, StuckDetection]`. Reverse iteration then visits
    /// StuckDetection first (sets `nudge_pending`), then the recorder
    /// (returns `EndRun` on the targeted turn), short-circuiting before
    /// StuckDetection's `after_tools` would have flushed that pending nudge
    /// into a `Message::user`.
    ///
    /// Four identical `counter` calls are scripted: turns 0-1 build no
    /// repeats yet; turn 2 (0-based) is the 3rd identical call, where
    /// StuckDetection's `repeats` counter crosses `STUCK_NUDGE_AFTER` (2) and
    /// sets the pending-nudge marker — the same threshold
    /// `stuck_identical_calls_nudged_then_aborted` proves nudges on. The
    /// recorder is configured to EndRun exactly on turn 2's `after_model`,
    /// right after StuckDetection has set that marker but before
    /// `after_tools` (where it would flush) ever runs. A 4th scripted turn
    /// is never reached, proving the short-circuit actually took effect.
    #[tokio::test]
    async fn end_run_from_after_model_drops_pending_nudge_without_orphans() {
        let one = || Scripted::Call("c1".into(), "counter".into(), r#"{"k":"a"}"#.into());
        let model = Arc::new(ScriptedModel::new(vec![one(), one(), one(), one()]));
        let sink = Arc::new(CollectingSink::default());
        let (agent, count) = counter_agent(model.clone(), sink.clone(), 5);
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = EndAtTurn {
            turn: 2,
            log: log.clone(),
        };
        let stack: Vec<Arc<dyn crate::Middleware>> = vec![
            Arc::new(recorder),
            Arc::new(crate::StuckDetectionMiddleware),
        ];
        let agent = agent.with_middleware(stack);
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        // Turns 0-1 execute the tool normally; turn 2 is consulted (its
        // identical-call signature is what crosses the nudge threshold) then
        // the recorder's after_model EndRuns before the tool call for turn 2
        // ever executes.
        assert_eq!(
            count.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "tool executes exactly twice; turn index 2 EndRuns from after_model, pre-exec"
        );
        assert_eq!(
            log.lock().unwrap().clone(),
            vec![
                "end_at_turn:run_start",
                "end_at_turn:after_model(turn=Some(0))",
                "end_at_turn:after_model(turn=Some(1))",
                "end_at_turn:after_model(turn=Some(2))",
            ],
            "recorder's after_model on turn index 2 ends the run; turn 3 never consulted"
        );
        assert_eq!(
            model.remaining(),
            1,
            "the 4th scripted turn must never be consumed"
        );
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(events.last().unwrap(), "done");

        use crate::ContextManager;
        let built = ctx.build(100_000);
        assert!(
            crate::orphaned_tool_positions(&built).is_empty(),
            "no orphaned tool_calls message after an after_model EndRun: {built:?}"
        );
        let nudge_present = built.iter().any(|m| {
            matches!(m.role, agent_model::Role::User) && m.content.contains("identical tool call")
        });
        assert!(
            !nudge_present,
            "the pending nudge must never flush — after_tools (where it's \
             injected) is short-circuited by the after_model EndRun: {built:?}"
        );
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
        let (mut stack, log) = recording_pair(None);
        // A3: the malformed turn must re-ask to reach the recovered text turn;
        // a bare recording stack no longer repairs, so add `RepairMiddleware`.
        // It only implements `on_parse_failure`, so it adds no after_model rows.
        stack.push(Arc::new(crate::RepairMiddleware));
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

    // --- A3: on_parse_failure firing set + fold semantics -------------------

    /// A recording middleware that logs each `on_parse_failure` call and
    /// re-asks once per streak (so a run can proceed past a malformed turn),
    /// then gives up — enough to prove the firing set without depending on
    /// `RepairMiddleware`.
    struct ParseFailRecorder {
        log: Arc<std::sync::Mutex<Vec<String>>>,
        seen: Arc<std::sync::atomic::AtomicUsize>,
    }
    #[async_trait::async_trait]
    impl crate::Middleware for ParseFailRecorder {
        fn name(&self) -> &str {
            "parse-fail-recorder"
        }
        async fn on_parse_failure(
            &self,
            _cx: &mut crate::RunCx<'_>,
            _raw: &agent_model::AssistantTurn,
            err: &str,
        ) -> crate::Repair {
            self.log.lock().unwrap().push("on_parse_failure".into());
            // Re-ask on the first observed failure, give up thereafter.
            if self.seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                crate::Repair::ReAsk(format!(
                    "Your tool call could not be parsed: {err}. Re-emit it correctly."
                ))
            } else {
                crate::Repair::GiveUp
            }
        }
    }

    #[tokio::test]
    async fn on_parse_failure_fires_only_at_total_parse_failure() {
        // (a) A malformed turn (Err from parse, stop != Length) followed by a
        // recovered text turn: on_parse_failure fires exactly once (the
        // malformed turn), never on the well-formed one.
        {
            let log = Arc::new(std::sync::Mutex::new(Vec::new()));
            let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let model = Arc::new(ScriptedModel::new(vec![
                Scripted::Call("c1".into(), "counter".into(), r#"{"k": "#.into()),
                Scripted::Text("recovered".into()),
            ]));
            let sink = Arc::new(CollectingSink::default());
            let (agent, _c) = counter_agent(model, sink, 5);
            let agent = agent.with_middleware(vec![Arc::new(ParseFailRecorder {
                log: log.clone(),
                seen: seen.clone(),
            })]);
            let mut ctx = WindowContext::new(Message::system("sys"));
            agent.run(&mut ctx, "go".into()).await.unwrap();
            assert_eq!(
                log.lock().unwrap().len(),
                1,
                "on_parse_failure fires once on the malformed turn only"
            );
        }
        // (b) A Length-truncated turn: the Length short-circuit fires FIRST, so
        // on_parse_failure does NOT fire.
        {
            let log = Arc::new(std::sync::Mutex::new(Vec::new()));
            let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let ws = std::env::temp_dir();
            let truncated = r##"{"path":"big.py","content":"#!/usr/bin/env python3\nprint('hi"##;
            let model = Arc::new(ScriptedModel::new(vec![Scripted::TruncatedCall(
                "write_file".into(),
                truncated.into(),
            )]));
            let sink = Arc::new(CollectingSink::default());
            let agent = AgentLoop::new(
                model,
                Arc::new(agent_model::NativeProtocol),
                registry(),
                policy(ws.clone()),
                Arc::new(AlwaysApprove),
                sink,
                LoopConfig {
                    model_limit: 100_000,
                    max_turns: 10,
                    max_retries: 2,
                    temperature: 0.0,
                    max_tokens: Some(2048),
                    workspace: ws,
                    tool_timeout: std::time::Duration::from_secs(5),
                    stream_idle_timeout: std::time::Duration::from_secs(60),
                    ..Default::default()
                },
            );
            let agent = agent.with_middleware(vec![Arc::new(ParseFailRecorder {
                log: log.clone(),
                seen,
            })]);
            let mut ctx = WindowContext::new(Message::system("sys"));
            agent.run(&mut ctx, "write big".into()).await.unwrap();
            assert!(
                log.lock().unwrap().is_empty(),
                "on_parse_failure must NOT fire on Length-truncation (short-circuited)"
            );
        }
    }

    #[tokio::test]
    async fn on_parse_failure_reverse_order_first_reask_wins() {
        // Middleware that always GiveUps, and one that always ReAsks (logging).
        struct GiveUpMw;
        #[async_trait::async_trait]
        impl crate::Middleware for GiveUpMw {
            fn name(&self) -> &str {
                "giveup"
            }
            async fn on_parse_failure(
                &self,
                _cx: &mut crate::RunCx<'_>,
                _raw: &agent_model::AssistantTurn,
                _err: &str,
            ) -> crate::Repair {
                crate::Repair::GiveUp
            }
        }
        struct ReAskMw;
        #[async_trait::async_trait]
        impl crate::Middleware for ReAskMw {
            fn name(&self) -> &str {
                "reask"
            }
            async fn on_parse_failure(
                &self,
                _cx: &mut crate::RunCx<'_>,
                _raw: &agent_model::AssistantTurn,
                _err: &str,
            ) -> crate::Repair {
                crate::Repair::ReAsk("re-ask please".into())
            }
        }

        // Stack [GiveUpMw, ReAskMw]: reverse order consults ReAskMw (stack-last)
        // first → ReAsk wins → the malformed turn re-asks and the run recovers.
        {
            let model = Arc::new(ScriptedModel::new(vec![
                Scripted::Call("c1".into(), "counter".into(), r#"{"k": "#.into()),
                Scripted::Text("recovered".into()),
            ]));
            let sink = Arc::new(CollectingSink::default());
            let (agent, _c) = counter_agent(model, sink.clone(), 5);
            let agent = agent.with_middleware(vec![Arc::new(GiveUpMw), Arc::new(ReAskMw)]);
            let mut ctx = WindowContext::new(Message::system("sys"));
            agent.run(&mut ctx, "go".into()).await.unwrap();
            let events = sink.events.lock().unwrap().clone();
            // No terminal Error from the malformed turn — the re-ask recovered.
            assert!(
                !events.iter().any(|e| e.starts_with("error:")),
                "reverse order: ReAsk (stack-last) wins, no terminal error: {events:?}"
            );
            let built = ctx.build(100_000);
            assert!(
                built.iter().any(|m| m.content.contains("re-ask please")),
                "the winning ReAsk message must be appended: {built:?}"
            );
        }
        // Stack [ReAskMw-as-sole-GiveUp]: a sole member that GiveUps yields
        // today's terminal give-up.
        {
            let model = Arc::new(ScriptedModel::new(vec![
                Scripted::Call("c1".into(), "counter".into(), r#"{"k": "#.into()),
                Scripted::Text("unreached".into()),
            ]));
            let sink = Arc::new(CollectingSink::default());
            let (agent, _c) = counter_agent(model, sink.clone(), 5);
            let agent = agent.with_middleware(vec![Arc::new(GiveUpMw)]);
            let mut ctx = WindowContext::new(Message::system("sys"));
            agent.run(&mut ctx, "go".into()).await.unwrap();
            let events = sink.events.lock().unwrap().clone();
            assert!(
                events.iter().any(|e| e.starts_with("error:")),
                "sole GiveUp yields today's terminal give-up: {events:?}"
            );
            assert_eq!(events.last().map(String::as_str), Some("done"));
        }
    }

    #[tokio::test]
    async fn empty_text_reask_appends_balanced_message() {
        use crate::ContextManager;
        // The common shape: an unparseable tool-call blob with NO prose, so
        // raw.text is empty. The re-ask appends an empty tool-call-free
        // assistant message + the user re-ask — balanced, no orphan.
        let model = Arc::new(ScriptedModel::new(vec![
            // Malformed args, and the scripted turn carries no assistant prose.
            Scripted::Call("c1".into(), "counter".into(), r#"{"k": "#.into()),
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        // counter_agent installs RepairMiddleware (A3), which drives the re-ask.
        let (agent, _c) = counter_agent(model, sink, 5);
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let built = ctx.build(100_000);
        assert!(
            crate::orphaned_tool_positions(&built).is_empty(),
            "the re-ask must leave no orphaned tool_calls message: {built:?}"
        );
        // The re-ask user message is present verbatim.
        assert!(
            built
                .iter()
                .any(|m| m.content.contains("Re-emit it correctly")),
            "the re-ask user message must be appended: {built:?}"
        );
    }

    // --- Task 7: remaining parity pins --------------------------------------

    /// A Write-tier tool (mirrors `WriteStub`'s Access::Write shape, but named
    /// `counter` so `stuck_identical_calls_nudged_then_aborted`-style repeated
    /// scripting drives BOTH the stuck-detection nudge and the post-tool
    /// validator path in the same run — the plain `Counter` test tool is
    /// Access::Read and would never trigger validators (task-7 brief note).
    struct WriteCounter(Arc<std::sync::atomic::AtomicUsize>);
    #[async_trait::async_trait]
    impl Tool for WriteCounter {
        fn name(&self) -> &str {
            "counter"
        }
        fn description(&self) -> &str {
            "records each execution (write-tier)"
        }
        fn schema(&self) -> agent_tools::ToolSchema {
            agent_tools::ToolSchema {
                name: "counter".into(),
                description: "".into(),
                parameters: serde_json::json!({"type":"object"}),
            }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<agent_tools::ToolIntent, ToolError> {
            Ok(agent_tools::ToolIntent {
                tool: "counter".into(),
                access: agent_tools::Access::Write,
                paths: vec![],
                command: None,
                summary: "count".into(),
            })
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> Result<agent_tools::ToolOutput, ToolError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(agent_tools::ToolOutput {
                content: "ok".into(),
                display: None,
            })
        }
    }

    /// Spec §5.4 timeline: validator failure + pending nudge + maintain, in
    /// today's exact durable order (tool results → validator msg → nudge).
    /// Three identical Write-tier call turns trip the stuck-detection nudge
    /// (3rd identical turn, spec §5.5) while `post_tool_validators =
    /// ["false"]` fails validation on every one of them (a mutating Ok call);
    /// a 4th, differing text turn ends the run cleanly.
    #[tokio::test]
    async fn nudged_turn_with_validator_failure_keeps_message_order() {
        let ws = std::env::temp_dir();
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(WriteCounter(count.clone())));
        let one = || Scripted::Call("c1".into(), "counter".into(), r#"{"k":"a"}"#.into());
        let model = Arc::new(ScriptedModel::new(vec![
            one(),
            one(),
            one(),
            Scripted::Text("done".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            Arc::new(reg),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink,
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                post_tool_validators: vec!["false".into()],
                ..Default::default()
            },
        );
        // Stuck-detection installed explicitly (mirrors counter_agent's own
        // caution note): default AgentLoop::new carries no middleware.
        let agent = agent.with_middleware(vec![Arc::new(crate::StuckDetectionMiddleware)]);
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        assert_eq!(
            count.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "all three identical Write-tier turns executed (no abort — only the nudge threshold)"
        );

        let built = ctx.build(100_000);
        // Locate the 3rd turn's Tool-result message (the write's own result),
        // and confirm the durable order right after it: Tool -> User(validation
        // failure) -> User(nudge) -> (eventually) the next Assistant turn.
        let tool_idx: Vec<usize> = built
            .iter()
            .enumerate()
            .filter(|(_, m)| matches!(m.role, agent_model::Role::Tool))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            tool_idx.len(),
            3,
            "one tool-result message per Write-tier turn: {built:?}"
        );
        let last_tool_idx = *tool_idx.last().unwrap();
        let validation_idx = last_tool_idx + 1;
        let nudge_idx = last_tool_idx + 2;
        assert!(
            matches!(built[validation_idx].role, agent_model::Role::User)
                && built[validation_idx]
                    .content
                    .contains("Post-edit validation reported problems"),
            "validator failure must land immediately after the 3rd turn's tool result: {built:?}"
        );
        assert!(
            matches!(built[nudge_idx].role, agent_model::Role::User)
                && built[nudge_idx].content.contains("identical tool call"),
            "the stuck-detection nudge must land immediately after the validator message: {built:?}"
        );
    }

    /// Spec §5.1: no node hook fires on cancellation or budget exhaustion.
    ///
    /// (a) A precancelled token: `fire_run_start` runs BEFORE the turn-top
    /// cancel check (loop_.rs `run_with_cancel`), so — observed, not the
    /// brief's first guess of an empty log — both middleware's run_start DO
    /// fire; the run then ends Done(Cancelled) with no further hooks.
    /// (b) Budget exhaustion: the tools-disabled wrap-up completion is a bare
    /// model call outside the node-hook dispatcher entirely, so it fires no
    /// `final_reply` — only the one completed tool turn's hooks appear.
    #[tokio::test]
    async fn cancel_and_budget_paths_fire_no_hooks() {
        // (a) precancelled token.
        {
            let dir = tempfile::tempdir().unwrap();
            let ws = dir.path().to_path_buf();
            let model = Arc::new(ScriptedModel::new(vec![Scripted::Text(
                "should never run".into(),
            )]));
            let sink = Arc::new(CollectingSink::default());
            let agent = AgentLoop::new(
                model,
                Arc::new(PassthroughProtocol),
                registry(),
                policy(ws.clone()),
                Arc::new(AlwaysApprove),
                sink.clone(),
                LoopConfig {
                    model_limit: 100_000,
                    max_turns: 10,
                    max_retries: 2,
                    temperature: 0.0,
                    max_tokens: None,
                    workspace: ws,
                    tool_timeout: std::time::Duration::from_secs(5),
                    stream_idle_timeout: std::time::Duration::from_secs(60),
                    ..Default::default()
                },
            );
            let (stack, log) = recording_pair(None);
            let agent = agent.with_middleware(stack);
            let mut ctx = WindowContext::new(Message::system("sys"));
            let cancel = CancellationToken::new();
            cancel.cancel(); // already cancelled before the run starts

            agent
                .run_with_cancel(&mut ctx, "go".into(), cancel)
                .await
                .unwrap();

            assert_eq!(
                log.lock().unwrap().clone(),
                vec!["a:run_start", "b:run_start"],
                "run_start fires (it runs before the turn-top cancel check); \
                 no other hook fires on a precancelled run"
            );
            let events = sink.events.lock().unwrap().clone();
            assert_eq!(
                events,
                vec![
                    "run_start:go:system_none=false".to_string(),
                    "done".to_string()
                ],
                "no model call happened; events were: {events:?}"
            );
        }

        // (b) budget exhaustion.
        {
            let dir = tempfile::tempdir().unwrap();
            let ws = dir.path().to_path_buf();
            std::fs::write(ws.join("f.txt"), "x").unwrap();
            let model = Arc::new(ScriptedModel::new(vec![
                Scripted::Call(
                    "c1".into(),
                    "read_file".into(),
                    format!(r#"{{"path":"{}"}}"#, ws.join("f.txt").display()),
                ),
                Scripted::Text("wrap-up summary".into()),
            ]));
            let sink = Arc::new(CollectingSink::default());
            let agent = AgentLoop::new(
                model,
                Arc::new(PassthroughProtocol),
                registry(),
                policy(ws.clone()),
                Arc::new(AlwaysApprove),
                sink.clone(),
                LoopConfig {
                    model_limit: 100_000,
                    max_turns: 1,
                    max_retries: 2,
                    temperature: 0.0,
                    max_tokens: None,
                    workspace: ws,
                    tool_timeout: std::time::Duration::from_secs(5),
                    stream_idle_timeout: std::time::Duration::from_secs(60),
                    ..Default::default()
                },
            );
            let (stack, log) = recording_pair(None);
            let agent = agent.with_middleware(stack);
            let mut ctx = WindowContext::new(Message::system("sys"));
            agent.run(&mut ctx, "go".into()).await.unwrap();

            // The one real (budget = 1) tool turn runs the full forward+reverse
            // hook set; the tools-disabled wrap-up completion afterward is a bare
            // model call the node-hook dispatcher never sees — no second
            // after_model/after_tools/turn_end, and no final_reply (the run ends
            // Done(BudgetExhausted), not the text-only exit path).
            assert_eq!(
                log.lock().unwrap().clone(),
                vec![
                    "a:run_start",
                    "b:run_start",
                    "b:after_model",
                    "a:after_model",
                    "b:after_tools",
                    "a:after_tools",
                    "b:turn_end",
                    "a:turn_end",
                ],
                "budget exhaustion's wrap-up completion must fire no additional hooks"
            );
            let events = sink.events.lock().unwrap().clone();
            assert!(
                events.iter().any(|e| e == "token:wrap-up summary"),
                "the wrap-up still streamed its text: {events:?}"
            );
            assert_eq!(events.last().map(String::as_str), Some("done"));
        }
    }

    // ---- ToolCallLimit / ModelCallLimit guardrail tests (spec §5.5/§5.7; B1 brief) ----
    //
    // Mirrors: `counter_agent`/`Counter` (varying-args harness base),
    // `DetailSink` (precise `Some(StopReason::Error)` capture — see
    // `budget_wrap_up_failure_is_best_effort`), `FailStub` (an always-erroring
    // tool — see `fail_stub_registry`), and
    // `end_run_from_after_tools_leaves_no_orphans` /
    // `end_run_from_after_model_drops_pending_nudge_without_orphans` (the
    // nudge-drop-without-orphans co-fire pattern).

    /// A tool that records the JSON args of every call it executes — proves
    /// the model issued genuinely DISTINCT args each turn (defeats
    /// `StuckDetection`, whose signature is over `(name, args)`).
    struct ArgRecorder(Arc<std::sync::Mutex<Vec<String>>>);
    #[async_trait::async_trait]
    impl Tool for ArgRecorder {
        fn name(&self) -> &str {
            "recorder"
        }
        fn description(&self) -> &str {
            "records each call's args"
        }
        fn schema(&self) -> agent_tools::ToolSchema {
            agent_tools::ToolSchema {
                name: "recorder".into(),
                description: "".into(),
                parameters: serde_json::json!({"type":"object"}),
            }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<agent_tools::ToolIntent, ToolError> {
            Ok(agent_tools::ToolIntent {
                tool: "recorder".into(),
                access: agent_tools::Access::Read,
                paths: vec![],
                command: None,
                summary: "record".into(),
            })
        }
        async fn execute(
            &self,
            args: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> Result<agent_tools::ToolOutput, ToolError> {
            self.0.lock().unwrap().push(args.to_string());
            Ok(agent_tools::ToolOutput {
                content: "ok".into(),
                display: None,
            })
        }
    }

    /// Build an `AgentLoop` wired to a single `ArgRecorder` tool, no
    /// middleware installed yet (caller adds the guardrail stack via
    /// `.with_middleware`). Mirrors `counter_agent`'s shape.
    fn recorder_agent(
        model: Arc<ScriptedModel>,
        sink: Arc<dyn EventSink>,
        max_turns: usize,
    ) -> (AgentLoop, Arc<std::sync::Mutex<Vec<String>>>) {
        let ws = std::env::temp_dir();
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(ArgRecorder(calls.clone())));
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            Arc::new(reg),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink,
            LoopConfig {
                model_limit: 100_000,
                max_turns,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        (agent, calls)
    }

    /// A model that issues one `recorder` call per turn with DISTINCT args
    /// each turn — the shape `StuckDetection` cannot catch. With
    /// `ToolCallLimit::with_cap(5)` and a generous `max_turns`, the run ends
    /// with `StopReason::Error` (NOT `BudgetExhausted`) once the count
    /// crosses the cap (spec §5.5, A10/Fa-F2).
    #[tokio::test]
    async fn tool_call_limit_ends_run_on_varying_args_runaway() {
        let calls: Vec<Scripted> = (0..20)
            .map(|i| Scripted::Call("c1".into(), "recorder".into(), format!(r#"{{"k":{i}}}"#)))
            .collect();
        let model = Arc::new(ScriptedModel::new(calls));
        let sink = Arc::new(DetailSink::default());
        // Generous max_turns (well past the cap) proves the guardrail — not the
        // turn budget — is what ends the run.
        let (agent, recorded) = recorder_agent(model.clone(), sink.clone(), 25);
        let agent = agent.with_middleware(vec![Arc::new(crate::ToolCallLimit::with_cap(5))]);
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        assert_eq!(
            *sink.done.lock().unwrap(),
            Some(StopReason::Error),
            "guardrail abort must be StopReason::Error, never BudgetExhausted"
        );
        // Overshoot bound: batch size is 1 here, so total executed is exactly
        // the cap (5) — the crossing call itself never runs.
        assert_eq!(
            recorded.lock().unwrap().len(),
            5,
            "exactly cap (5) calls execute; the 6th (crossing) call is blocked pre-exec"
        );
        // All recorded args are pairwise distinct — proves this scenario truly
        // defeats StuckDetection's (name, args) signature.
        let seen = recorded.lock().unwrap().clone();
        let unique: std::collections::HashSet<_> = seen.iter().cloned().collect();
        assert_eq!(
            unique.len(),
            seen.len(),
            "args must vary every turn: {seen:?}"
        );
        assert!(
            sink.errors
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.contains("tool-call guardrail")),
            "expected a tool-call guardrail Error event; got {:?}",
            sink.errors.lock().unwrap()
        );
    }

    /// Fat crossing batch: the model issues N calls in the crossing turn.
    /// Total executed <= cap-1 (prior turns) + N (the crossing batch) —
    /// bounded to one batch, not unbounded (spec §5.5 Fa-F3).
    #[tokio::test]
    async fn tool_call_limit_overshoot_is_bounded_to_one_batch() {
        // Turns 1-4: one call each, distinct args (4 total, under cap=5).
        // Turn 5: a batch of 4 calls in ONE turn, distinct args — crosses the
        // cap mid-batch. All 4 execute (wrap_tool_call counts before exec, and
        // the parallel executor runs the whole turn's batch), then after_tools
        // observes count=8 >= cap and EndRuns. Total executed = 4 + 4 = 8,
        // bounded to one batch past the cap (not unbounded across turns).
        let single =
            |i: usize| Scripted::Call("c1".into(), "recorder".into(), format!(r#"{{"k":{i}}}"#));
        let batch = Scripted::Calls(vec![
            ("b1".into(), "recorder".into(), r#"{"k":100}"#.into()),
            ("b2".into(), "recorder".into(), r#"{"k":101}"#.into()),
            ("b3".into(), "recorder".into(), r#"{"k":102}"#.into()),
            ("b4".into(), "recorder".into(), r#"{"k":103}"#.into()),
        ]);
        let model = Arc::new(ScriptedModel::new(vec![
            single(0),
            single(1),
            single(2),
            single(3),
            batch,
            Scripted::Text("done".into()),
        ]));
        let sink = Arc::new(DetailSink::default());
        let (agent, recorded) = recorder_agent(model.clone(), sink.clone(), 25);
        let agent = agent.with_middleware(vec![Arc::new(crate::ToolCallLimit::with_cap(5))]);
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        assert_eq!(
            *sink.done.lock().unwrap(),
            Some(StopReason::Error),
            "guardrail abort must be StopReason::Error"
        );
        assert_eq!(
            recorded.lock().unwrap().len(),
            8,
            "4 prior single calls + the full 4-call crossing batch = 8, not unbounded"
        );
        // The 6th scripted turn (the plain-text "done") must never be reached.
        assert_eq!(
            model.remaining(),
            1,
            "the turn after the crossing batch must never be consulted"
        );
    }

    /// Count-on-failure is intentional (increment BEFORE `next.run`): a
    /// panicking / denied / errored call still counts (spec §5.5 Fa-F3).
    /// Drives a tool that ALWAYS errors and asserts the count still advances
    /// toward the cap (the guardrail still trips on an all-failing runaway).
    #[tokio::test]
    async fn tool_call_limit_counts_failed_calls() {
        struct AlwaysFails;
        #[async_trait::async_trait]
        impl Tool for AlwaysFails {
            fn name(&self) -> &str {
                "always_fails"
            }
            fn description(&self) -> &str {
                "always errors (test stub)"
            }
            fn schema(&self) -> agent_tools::ToolSchema {
                agent_tools::ToolSchema {
                    name: "always_fails".into(),
                    description: "".into(),
                    parameters: serde_json::json!({"type":"object"}),
                }
            }
            fn intent(
                &self,
                _args: &serde_json::Value,
            ) -> Result<agent_tools::ToolIntent, ToolError> {
                Ok(agent_tools::ToolIntent {
                    tool: "always_fails".into(),
                    access: agent_tools::Access::Read,
                    paths: vec![],
                    command: None,
                    summary: "fail".into(),
                })
            }
            async fn execute(
                &self,
                _args: serde_json::Value,
                _ctx: &ToolCtx,
            ) -> Result<agent_tools::ToolOutput, ToolError> {
                Err(ToolError::Failed {
                    message: "boom".into(),
                    stderr: None,
                })
            }
        }

        let ws = std::env::temp_dir();
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(AlwaysFails));
        let calls: Vec<Scripted> = (0..20)
            .map(|i| {
                Scripted::Call(
                    "c1".into(),
                    "always_fails".into(),
                    format!(r#"{{"k":{i}}}"#),
                )
            })
            .collect();
        let model = Arc::new(ScriptedModel::new(calls));
        let sink = Arc::new(DetailSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            Arc::new(reg),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 25,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let agent = agent.with_middleware(vec![Arc::new(crate::ToolCallLimit::with_cap(5))]);
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        assert_eq!(
            *sink.done.lock().unwrap(),
            Some(StopReason::Error),
            "an all-failing runaway must still trip the guardrail"
        );
        // Every attempted call recorded an Error tool_result — 5 of them (the
        // cap), proving count-on-failure: the guardrail tripped on FAILED
        // calls, not successful ones.
        let results = sink.tool_results.lock().unwrap().clone();
        assert_eq!(
            results.len(),
            5,
            "5 failed calls attempted before the guardrail trips: {results:?}"
        );
        assert!(
            results
                .iter()
                .all(|(_, status, _)| *status == ToolStatus::Error),
            "every attempted call must have failed: {results:?}"
        );
    }

    /// Stack order `[StuckDetection, ToolCallLimit]`: `after_tools` fires in
    /// reverse, so `ToolCallLimit`'s `after_tools` `EndRun` short-circuits
    /// BEFORE `StuckDetection` flushes a nudge it set that turn. Assert the
    /// CONTENT outcome: nudge dropped, transcript balanced, no orphaned
    /// tool_calls (spec §5.6 Fa-F5). Mirrors
    /// `end_run_from_after_model_drops_pending_nudge_without_orphans`, but the
    /// EndRun source is `ToolCallLimit`'s `after_tools` (not an `after_model`
    /// recorder), co-firing with a real `StuckDetectionMiddleware` at a low
    /// cap so both accumulate on the SAME identical-call turns.
    #[tokio::test]
    async fn guardrail_endrun_at_after_tools_drops_pending_nudge_without_orphans() {
        // Identical calls (same name+args) every turn: StuckDetection's nudge
        // threshold (STUCK_NUDGE_AFTER = 2, i.e. the 3rd identical turn, 0-based
        // turn index 2) fires in the SAME turn ToolCallLimit's cap (3) is
        // crossed — both guardrails are live on turn index 2.
        let one = || Scripted::Call("c1".into(), "counter".into(), r#"{"k":"a"}"#.into());
        let model = Arc::new(ScriptedModel::new(vec![one(), one(), one(), one()]));
        let sink = Arc::new(CollectingSink::default());
        let (agent, count) = counter_agent(model.clone(), sink.clone(), 25);
        // Stack order [StuckDetection, ToolCallLimit]: after_tools fires in
        // REVERSE stack order, so ToolCallLimit's after_tools (added later in
        // the vec) runs FIRST and EndRuns before StuckDetection's after_tools
        // (which would flush the pending nudge) ever gets a turn.
        let agent = agent.with_middleware(vec![
            Arc::new(crate::StuckDetectionMiddleware),
            Arc::new(crate::ToolCallLimit::with_cap(3)),
        ]);
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        // Turns 0-2 (3 identical calls) execute; the cap (3) is reached at
        // after_tools on turn index 2, ending the run before turn 3.
        assert_eq!(
            count.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "exactly cap (3) identical calls execute before ToolCallLimit ends the run"
        );
        assert_eq!(
            model.remaining(),
            1,
            "the 4th scripted turn must never be consulted"
        );
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(events.last().unwrap(), "done");
        assert!(
            events
                .iter()
                .any(|e| e.starts_with("error:") && e.contains("tool-call guardrail")),
            "expected the tool-call guardrail Error event; got {events:?}"
        );

        use crate::ContextManager;
        let built = ctx.build(100_000);
        assert!(
            crate::orphaned_tool_positions(&built).is_empty(),
            "no orphaned tool_calls message after the guardrail's after_tools EndRun: {built:?}"
        );
        // Content outcome: the pending nudge StuckDetection set on turn index 2
        // (repeats crosses STUCK_NUDGE_AFTER=2 on the 3rd identical turn) must
        // NEVER flush — after_tools (where it's injected) is short-circuited by
        // ToolCallLimit's after_tools EndRun running first (reverse order).
        let nudge_present = built.iter().any(|m| {
            matches!(m.role, agent_model::Role::User) && m.content.contains("identical tool call")
        });
        assert!(
            !nudge_present,
            "the pending nudge must never flush — ToolCallLimit's after_tools \
             EndRun (reverse-order first) short-circuits before StuckDetection's \
             after_tools runs: {built:?}"
        );
    }

    /// Default (disabled): `ModelCallLimit` counts in `wrap_model_call` but
    /// never enforces — a run well within `max_turns` is unaffected. Enabled
    /// (`ModelCallLimit::enabled_with_cap(2)`): after 2 model calls,
    /// `after_model` EndRuns with `StopReason::Error`.
    #[tokio::test]
    async fn model_call_limit_default_off_is_inert_but_enabled_ends_run() {
        // Disabled: 3 turns, generous max_turns, distinct-args calls — the run
        // completes normally (budget-bounded), proving the disabled guardrail
        // never enforces despite counting every wrap_model_call.
        {
            let model = Arc::new(ScriptedModel::new(vec![
                Scripted::Call("c1".into(), "recorder".into(), r#"{"k":0}"#.into()),
                Scripted::Call("c1".into(), "recorder".into(), r#"{"k":1}"#.into()),
                Scripted::Text("done".into()),
            ]));
            let sink = Arc::new(DetailSink::default());
            let (agent, recorded) = recorder_agent(model.clone(), sink.clone(), 25);
            let agent = agent.with_middleware(vec![Arc::new(crate::ModelCallLimit::disabled())]);
            let mut ctx = WindowContext::new(Message::system("sys"));
            agent.run(&mut ctx, "go".into()).await.unwrap();

            assert_eq!(
                recorded.lock().unwrap().len(),
                2,
                "both tool calls executed"
            );
            assert_eq!(
                *sink.done.lock().unwrap(),
                Some(StopReason::Stop),
                "disabled ModelCallLimit never enforces; the run ends on its own \
                 text-only stop, not a guardrail abort"
            );
            assert!(
                sink.errors.lock().unwrap().is_empty(),
                "no guardrail Error event when disabled: {:?}",
                sink.errors.lock().unwrap()
            );
        }
        // Enabled with cap=2: 2 model calls trip the cap at after_model,
        // ending the run with StopReason::Error before a 3rd call is ever made.
        {
            let model = Arc::new(ScriptedModel::new(vec![
                Scripted::Call("c1".into(), "recorder".into(), r#"{"k":0}"#.into()),
                Scripted::Call("c1".into(), "recorder".into(), r#"{"k":1}"#.into()),
                Scripted::Text("done".into()),
            ]));
            let sink = Arc::new(DetailSink::default());
            let (agent, recorded) = recorder_agent(model.clone(), sink.clone(), 25);
            let agent =
                agent.with_middleware(vec![Arc::new(crate::ModelCallLimit::enabled_with_cap(2))]);
            let mut ctx = WindowContext::new(Message::system("sys"));
            agent.run(&mut ctx, "go".into()).await.unwrap();

            assert_eq!(
                *sink.done.lock().unwrap(),
                Some(StopReason::Error),
                "enabled ModelCallLimit must EndRun with StopReason::Error, never BudgetExhausted"
            );
            assert_eq!(
                recorded.lock().unwrap().len(),
                1,
                "only the 1st turn's tool call executes — after_model fires BEFORE \
                 that turn's tools run, so the 2nd model call's after_model trips \
                 the cap and EndRuns before its own tool call is ever dispatched"
            );
            assert_eq!(
                model.remaining(),
                1,
                "the 3rd scripted turn must never be consulted"
            );
            assert!(
                sink.errors
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|e| e.contains("model-call guardrail")),
                "expected a model-call guardrail Error event; got {:?}",
                sink.errors.lock().unwrap()
            );
        }
    }

    /// Spec §7: no maintain on either Length-stop exit. Mirrors
    /// `truncated_tool_call_reports_max_tokens_not_bad_args`: a
    /// `TruncatedCall` immediately yields `StopReason::Length`. OBSERVED (not
    /// the brief's first guess of an empty log): `fire_run_start` runs before
    /// the model is ever called, so both middleware's run_start DO fire; the
    /// Length return then short-circuits everything after — no after_model,
    /// no after_tools, no on_turn_end, and critically no after_final_reply
    /// (the maintain hook), since the run never reaches the text-only exit
    /// path either.
    #[tokio::test]
    async fn length_exit_fires_no_maintain_hooks() {
        let ws = std::env::temp_dir();
        let truncated = r##"{"path":"big.py","content":"#!/usr/bin/env python3\nprint('hi"##;
        let model = Arc::new(ScriptedModel::new(vec![Scripted::TruncatedCall(
            "write_file".into(),
            truncated.into(),
        )]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: Some(2048),
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let (stack, log) = recording_pair(None);
        let agent = agent.with_middleware(stack);
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent
            .run(&mut ctx, "write a big file".into())
            .await
            .unwrap();

        assert_eq!(
            log.lock().unwrap().clone(),
            vec!["a:run_start", "b:run_start"],
            "run_start fires (it runs before the model is ever called); a Length-stop \
             exit must fire no after_model/after_tools/turn_end/final_reply — in \
             particular no maintain hook runs on this exit"
        );
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(
            events.last().map(String::as_str),
            Some("done"),
            "the truncation abort must still terminate with Done; events were: {events:?}"
        );
    }

    // ---- Task 7: park-point checkpoint (write at Ask, delete on answer) ----

    /// Approval channel that answers `resp` and records how many times it
    /// was asked plus whether a park existed on disk WHILE it blocked.
    struct ParkApproval {
        resp: ApprovalResponse,
        asks: Arc<std::sync::atomic::AtomicUsize>,
        /// Some(bool) once asked: did a park exist at ask time?
        parked_at_ask: Arc<std::sync::Mutex<Option<bool>>>,
        park_dir: std::path::PathBuf,
    }
    #[async_trait::async_trait]
    impl ApprovalChannel for ParkApproval {
        async fn request(&self, _req: ApprovalRequest) -> ApprovalResponse {
            self.asks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *self.parked_at_ask.lock().unwrap() = Some(crate::checkpoint::has_park(&self.park_dir));
            self.resp
        }
    }

    /// Build a one-execute_command-call agent with an empty-allowlist policy
    /// (⇒ Ask) wired to `approval` and `ck`. Mirrors the
    /// `approval_summary_includes_sandbox_posture` rig.
    fn ask_agent(
        ws: std::path::PathBuf,
        approval: Arc<dyn ApprovalChannel>,
        ck: Arc<crate::Checkpointer>,
    ) -> (AgentLoop, Arc<CollectingSink>) {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(agent_tools::shell::ExecuteCommand));
        let tools = Arc::new(r);
        let pol = Arc::new(RulePolicy {
            workspace: ws.clone(),
            command_allowlist: vec![],
            command_denylist: vec![],
        });
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "execute_command".into(),
                r#"{"command":"echo hi"}"#.into(),
            ),
            Scripted::Text("Done.".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            tools,
            pol,
            approval,
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                temperature: 0.0,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                sandbox: Arc::new(agent_tools::HostExecutor),
                ..Default::default()
            },
        )
        .with_checkpointer(ck);
        (agent, sink)
    }

    fn curated(ws_note: &str) -> crate::CuratedContext {
        let _ = ws_note;
        crate::CuratedContext::new(
            Message::system("s"),
            Arc::new(crate::SessionArtifacts::new()),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
    }

    #[tokio::test]
    async fn ask_parks_before_blocking_and_answer_deletes_the_park() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let ck_dir = dir.path().join("checkpoint");
        let ck = crate::Checkpointer::new(ck_dir.clone(), [7u8; 32], "s1".into());
        let asks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let parked_at_ask = Arc::new(std::sync::Mutex::new(None));
        let approval = Arc::new(ParkApproval {
            resp: ApprovalResponse::Approve,
            asks: asks.clone(),
            parked_at_ask: parked_at_ask.clone(),
            park_dir: ck_dir.clone(),
        });
        let (agent, _sink) = ask_agent(ws, approval, ck);
        // MUST be CuratedContext: the park write is gated on
        // ctx.checkpoint_state() being Some — WindowContext silently skips it.
        let mut ctx = curated("ws");
        agent.run(&mut ctx, "go".into()).await.unwrap();
        // park existed WHILE the ask was pending (spec §2.3: write BEFORE block)…
        assert_eq!(*parked_at_ask.lock().unwrap(), Some(true));
        // …and the answer deleted it before the run proceeded (commit point).
        assert!(!crate::checkpoint::has_park(&ck_dir));
        assert_eq!(asks.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn deny_also_deletes_the_park() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let ck_dir = dir.path().join("checkpoint");
        let ck = crate::Checkpointer::new(ck_dir.clone(), [7u8; 32], "s1".into());
        let asks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let parked_at_ask = Arc::new(std::sync::Mutex::new(None));
        let approval = Arc::new(ParkApproval {
            resp: ApprovalResponse::Deny,
            asks: asks.clone(),
            parked_at_ask: parked_at_ask.clone(),
            park_dir: ck_dir.clone(),
        });
        let (agent, _sink) = ask_agent(ws, approval, ck);
        let mut ctx = curated("ws");
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert_eq!(
            *parked_at_ask.lock().unwrap(),
            Some(true),
            "parked while blocked"
        );
        // Denial commits the answer the same way an approval does: park deleted.
        assert!(!crate::checkpoint::has_park(&ck_dir));
    }

    #[tokio::test]
    async fn non_ask_runs_write_zero_checkpoint_bytes() {
        // E1: an allowlisted command auto-allows (Decision::Allow) so no Ask
        // ever parks — the checkpoint dir must never be touched.
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let ck_dir = dir.path().join("checkpoint");
        let ck = crate::Checkpointer::new(ck_dir.clone(), [7u8; 32], "s1".into());
        let mut r = ToolRegistry::new();
        r.register(Arc::new(agent_tools::shell::ExecuteCommand));
        let tools = Arc::new(r);
        let pol = Arc::new(RulePolicy {
            workspace: ws.clone(),
            command_allowlist: vec!["echo".into()],
            command_denylist: vec![],
        });
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "execute_command".into(),
                r#"{"command":"echo hi"}"#.into(),
            ),
            Scripted::Text("Done.".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            tools,
            pol,
            Arc::new(AlwaysApprove),
            sink,
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                temperature: 0.0,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                sandbox: Arc::new(agent_tools::HostExecutor),
                ..Default::default()
            },
        )
        .with_checkpointer(ck);
        let mut ctx = curated("ws");
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert!(!ck_dir.exists(), "E1: no checkpoint I/O at all");
    }

    #[tokio::test]
    async fn park_checkpoint_carries_context_records_and_parked_index() {
        // Two calls in one turn: read_file (Allow) then execute_command (Ask).
        // Approval channel loads the on-disk checkpoint WHILE blocked, then
        // denies. Assertions run against the stashed checkpoint.
        struct CapturingApproval {
            ck_dir: std::path::PathBuf,
            key: [u8; 32],
            stash: Arc<std::sync::Mutex<Option<crate::Checkpoint>>>,
        }
        #[async_trait::async_trait]
        impl ApprovalChannel for CapturingApproval {
            async fn request(&self, _req: ApprovalRequest) -> ApprovalResponse {
                *self.stash.lock().unwrap() =
                    crate::checkpoint::load_checkpoint(&self.ck_dir, &self.key).unwrap();
                ApprovalResponse::Deny
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        std::fs::write(ws.join("a.txt"), "FILEBODY").unwrap();
        let ck_dir = dir.path().join("checkpoint");
        let ck = crate::Checkpointer::new(ck_dir.clone(), [7u8; 32], "s1".into());
        let stash = Arc::new(std::sync::Mutex::new(None));
        let approval = Arc::new(CapturingApproval {
            ck_dir: ck_dir.clone(),
            key: [7u8; 32],
            stash: stash.clone(),
        });

        let mut r = ToolRegistry::new();
        r.register(Arc::new(agent_tools::fs::ReadFile {
            max_bytes: 16 * 1024,
        }));
        r.register(Arc::new(agent_tools::shell::ExecuteCommand));
        let tools = Arc::new(r);
        // read_file inside workspace -> Allow; execute_command empty allowlist -> Ask.
        let pol = Arc::new(RulePolicy {
            workspace: ws.clone(),
            command_allowlist: vec![],
            command_denylist: vec![],
        });
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                (
                    "c1".into(),
                    "read_file".into(),
                    r#"{"path":"a.txt"}"#.into(),
                ),
                (
                    "c2".into(),
                    "execute_command".into(),
                    r#"{"command":"echo hi"}"#.into(),
                ),
            ]),
            Scripted::Text("Done.".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            tools,
            pol,
            approval,
            sink,
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                temperature: 0.0,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                sandbox: Arc::new(agent_tools::HostExecutor),
                ..Default::default()
            },
        )
        .with_checkpointer(ck);
        let mut ctx = curated("ws");
        agent.run(&mut ctx, "go".into()).await.unwrap();

        let parked = stash
            .lock()
            .unwrap()
            .clone()
            .expect("checkpoint present on disk while the ask blocked");
        assert_eq!(
            parked.parked.parked_index,
            Some(1),
            "blocked at call index 1"
        );
        assert_eq!(
            parked.parked.gate_records,
            vec![crate::GateRecord::Ready],
            "one Ready record precedes the parked call"
        );
        assert_eq!(parked.parked.tool_calls.len(), 2, "full batch persisted");
        assert_eq!(parked.session_id, "s1");
        // The assistant message with both calls was appended pre-gate.
        let last = parked.context.history.last().expect("history non-empty");
        assert_eq!(last.role, agent_model::Role::Assistant);
        assert_eq!(
            last.tool_calls.as_ref().map(|c| c.len()),
            Some(2),
            "the appended assistant message carries the full pre-gate batch"
        );
        // Guardrail tallies are captured (0 here — no counting middleware in
        // this rig; the ModelCallLimit/ToolCallLimit units feed these).
        assert_eq!(parked.guardrails.model_calls, 0);
    }

    #[tokio::test]
    async fn checkpoint_write_failure_degrades_to_live_only() {
        // Point the checkpoint dir at a FILE so the write fails; the run must
        // still complete normally (spec §2.3: never block the run on I/O).
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let ck_dir = dir.path().join("checkpoint");
        // A regular file where the dir needs to be -> write_checkpoint's
        // create_dir_all under it fails.
        std::fs::write(&ck_dir, "x").unwrap();
        let ck = crate::Checkpointer::new(ck_dir.clone(), [7u8; 32], "s1".into());
        let asks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let parked_at_ask = Arc::new(std::sync::Mutex::new(None));
        let approval = Arc::new(ParkApproval {
            resp: ApprovalResponse::Approve,
            asks: asks.clone(),
            parked_at_ask: parked_at_ask.clone(),
            park_dir: ck_dir.clone(),
        });
        let (agent, sink) = ask_agent(ws, approval, ck);
        let mut ctx = curated("ws");
        // No panic, no error return.
        agent.run(&mut ctx, "go".into()).await.unwrap();
        // The tool still ran and the run reached a normal terminal Done.
        assert_eq!(asks.load(std::sync::atomic::Ordering::SeqCst), 1);
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(events.last().map(String::as_str), Some("done"));
    }

    /// Plan review finding 2 / brief Step-4 note: a dispatch-kind ancestor
    /// snapshot flushed to disk this turn MUST be cleared at turn end on
    /// EVERY exit of `tool_phase`, including the on_turn_end EndRun exit —
    /// otherwise a later scan misreads the stale park as a phantom session.
    ///
    /// This repo has no in-loop dispatch wiring yet, so we pin the leak-prone
    /// PATH directly: a middleware sets a turn snapshot + flushes it (mimicking
    /// a descendant park) and then EndRuns at on_turn_end. Assert no
    /// dispatch-kind park survives.
    #[tokio::test]
    async fn end_turn_clears_flushed_park_on_end_run_exit() {
        use crate::{Flow, RunCx};
        struct FlushThenEnd {
            ck: Arc<crate::Checkpointer>,
        }
        #[async_trait::async_trait]
        impl crate::Middleware for FlushThenEnd {
            fn name(&self) -> &str {
                "flush-then-end"
            }
            async fn after_tools(&self, _cx: &mut RunCx<'_>) -> Flow {
                // Simulate a dispatch-bearing turn whose descendant parked:
                // set a pending snapshot and force it to disk this turn.
                self.ck.set_turn_snapshot(crate::PendingSnapshot {
                    context: crate::CuratedContext::new(
                        Message::system("s"),
                        Arc::new(crate::SessionArtifacts::new()),
                        Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    )
                    .checkpoint_state(),
                    guardrails: crate::Guardrails::default(),
                    turn: 0,
                    assistant_text: String::new(),
                    tool_calls: vec![],
                    invalid: vec![],
                    gate_records: vec![],
                    artifacts: Arc::new(crate::SessionArtifacts::new()),
                });
                self.ck
                    .write_park(
                        crate::Checkpoint {
                            version: crate::CHECKPOINT_VERSION,
                            session_id: "s1".into(),
                            subagent_path: vec![],
                            turn: 0,
                            context: crate::CuratedContext::new(
                                Message::system("s"),
                                Arc::new(crate::SessionArtifacts::new()),
                                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                            )
                            .checkpoint_state(),
                            guardrails: crate::Guardrails::default(),
                            parked: crate::ParkedTurn {
                                assistant_text: String::new(),
                                tool_calls: vec![],
                                invalid: vec![],
                                gate_records: vec![],
                                parked_index: None,
                                origin: None,
                            },
                        },
                        &crate::SessionArtifacts::new(),
                    )
                    .await
                    .unwrap();
                Flow::Continue
            }
            async fn on_turn_end(&self, _cx: &mut RunCx<'_>) -> Flow {
                Flow::EndRun(StopReason::Error)
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let ck_dir = dir.path().join("checkpoint");
        let ck = crate::Checkpointer::new(ck_dir.clone(), [7u8; 32], "s1".into());
        let mut r = ToolRegistry::new();
        r.register(Arc::new(agent_tools::shell::ExecuteCommand));
        let tools = Arc::new(r);
        let pol = Arc::new(RulePolicy {
            workspace: ws.clone(),
            command_allowlist: vec!["echo".into()],
            command_denylist: vec![],
        });
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "execute_command".into(),
                r#"{"command":"echo hi"}"#.into(),
            ),
            Scripted::Text("Done.".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            tools,
            pol,
            Arc::new(AlwaysApprove),
            sink,
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                temperature: 0.0,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                sandbox: Arc::new(agent_tools::HostExecutor),
                ..Default::default()
            },
        )
        .with_checkpointer(ck.clone())
        .with_middleware(vec![Arc::new(FlushThenEnd { ck: ck.clone() })]);
        let mut ctx = curated("ws");
        agent.run(&mut ctx, "go".into()).await.unwrap();
        // The run ended at on_turn_end (EndRun), yet the flushed dispatch-kind
        // park must NOT survive — end_turn ran on that exit.
        assert!(
            !crate::checkpoint::has_park(&ck_dir),
            "flushed dispatch-kind park leaked past an EndRun exit"
        );
    }

    // ---- Task 8: resume entry — pre-decided gate splice + tally seeding ----

    /// A sink that records every ToolStart / ToolResult with its call id so a
    /// test can assert exactly-one-ToolStart-per-id and per-id terminal status
    /// (the string CollectingSink drops ids). Runs alongside nothing else.
    #[derive(Default)]
    struct IdSink {
        starts: Mutex<Vec<String>>,
        results: Mutex<Vec<(String, String, String)>>, // (id, name, status)
        done: Mutex<Vec<StopReason>>,
    }
    impl crate::EventSink for IdSink {
        fn emit(&self, event: AgentEvent) {
            match event {
                AgentEvent::ToolStart { id, .. } => self.starts.lock().unwrap().push(id),
                AgentEvent::ToolResult {
                    id, name, status, ..
                } => self
                    .results
                    .lock()
                    .unwrap()
                    .push((id, name, status.as_str().into())),
                AgentEvent::Done(r) => self.done.lock().unwrap().push(r),
                _ => {}
            }
        }
    }
    impl IdSink {
        fn starts_for(&self, id: &str) -> usize {
            self.starts
                .lock()
                .unwrap()
                .iter()
                .filter(|s| *s == id)
                .count()
        }
        fn status_of(&self, id: &str) -> Option<String> {
            self.results
                .lock()
                .unwrap()
                .iter()
                .find(|(i, _, _)| i == id)
                .map(|(_, _, s)| s.clone())
        }
    }

    /// Approval channel that counts prompts and answers `resp`. Optionally
    /// records whether a park existed on disk while it blocked (re-ask test).
    struct SpliceApproval {
        resp: ApprovalResponse,
        asks: Arc<std::sync::atomic::AtomicUsize>,
        parked_at_ask: Arc<std::sync::Mutex<Option<bool>>>,
        park_dir: Option<std::path::PathBuf>,
    }
    #[async_trait::async_trait]
    impl ApprovalChannel for SpliceApproval {
        async fn request(&self, _req: ApprovalRequest) -> ApprovalResponse {
            self.asks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if let Some(d) = &self.park_dir {
                *self.parked_at_ask.lock().unwrap() = Some(crate::checkpoint::has_park(d));
            }
            self.resp
        }
    }

    /// Build a restored `CuratedContext` whose history ends with the assistant
    /// message carrying `batch` (mirrors what park writes pre-gate).
    fn restore_with_batch(goal: &str, batch: &[ToolCall]) -> crate::CuratedContext {
        let state = crate::CuratedContextState {
            goal: Some(Message::system(format!("Original goal: {goal}"))),
            history: vec![
                Message::user(goal.to_string()),
                Message::assistant("running the batch", Some(batch.to_vec())),
            ],
            compaction_summary: None,
            folded_facts: vec![],
            folded_sections: vec![],
            seq: 0,
            history_has_spans: false,
            history_incomplete: false,
            artifact_prefix: String::new(),
            todos: vec![],
        };
        crate::CuratedContext::restore(
            Message::system("s"),
            Arc::new(crate::SessionArtifacts::new()),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            Arc::new(std::sync::Mutex::new(Vec::new())),
            state,
        )
    }

    /// execute_command agent (empty allowlist ⇒ Ask), IdSink, optional ck.
    fn resume_agent(
        ws: std::path::PathBuf,
        approval: Arc<dyn ApprovalChannel>,
        ck: Option<Arc<crate::Checkpointer>>,
        model: Vec<Scripted>,
        extra_middleware: Vec<Arc<dyn crate::Middleware>>,
    ) -> (AgentLoop, Arc<IdSink>) {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(agent_tools::shell::ExecuteCommand));
        let tools = Arc::new(r);
        let pol = Arc::new(RulePolicy {
            workspace: ws.clone(),
            command_allowlist: vec![],
            command_denylist: vec![],
        });
        let sink = Arc::new(IdSink::default());
        let mut agent = AgentLoop::new(
            Arc::new(ScriptedModel::new(model)),
            Arc::new(PassthroughProtocol),
            tools,
            pol,
            approval,
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                temperature: 0.0,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                sandbox: Arc::new(agent_tools::HostExecutor),
                ..Default::default()
            },
        );
        if !extra_middleware.is_empty() {
            agent = agent.with_middleware(extra_middleware);
        }
        if let Some(ck) = ck {
            agent = agent.with_checkpointer(ck);
        }
        (agent, sink)
    }

    fn echo_batch() -> Vec<ToolCall> {
        vec![
            ToolCall {
                id: "c1".into(),
                name: "execute_command".into(),
                args: serde_json::json!({"command": "echo one"}),
            },
            ToolCall {
                id: "c2".into(),
                name: "execute_command".into(),
                args: serde_json::json!({"command": "echo two"}),
            },
        ]
    }

    #[tokio::test]
    async fn resume_splices_records_without_reprompting_and_executes_phase2() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let asks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let approval = Arc::new(SpliceApproval {
            resp: ApprovalResponse::Approve,
            asks: asks.clone(),
            parked_at_ask: Arc::new(std::sync::Mutex::new(None)),
            park_dir: None,
        });
        // The RESUMED turn consumes no completion; the model only serves the
        // turn AFTER the batch.
        let (agent, sink) = resume_agent(
            ws.clone(),
            approval,
            None,
            vec![Scripted::Text("wrapped up".into())],
            vec![],
        );
        let batch = echo_batch();
        let mut ctx = restore_with_batch("the goal", &batch);
        let resume = crate::ResumeTurn {
            assistant_text: "running the batch".into(),
            tool_calls: batch,
            invalid: vec![],
            gate_records: vec![crate::GateRecord::Ready],
            parked_index: Some(1),
            parked_decision: Some(true),
            turn: 3,
            guardrails: crate::Guardrails {
                tool_calls: 5,
                model_calls: 4,
            },
            goal_text: "the goal".into(),
        };
        agent
            .resume_with_cancel(&mut ctx, resume, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            asks.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "no re-prompt (spec §3.7)"
        );
        // exactly one ToolStart per call id (double-emit guard)
        assert_eq!(sink.starts_for("c1"), 1, "one ToolStart for c1");
        assert_eq!(sink.starts_for("c2"), 1, "one ToolStart for c2");
        // both executed to Ok
        assert_eq!(sink.status_of("c1").as_deref(), Some("ok"));
        assert_eq!(sink.status_of("c2").as_deref(), Some("ok"));
        // the run then finished on the scripted text turn: Done(Stop)
        assert_eq!(
            sink.done.lock().unwrap().last(),
            Some(&StopReason::Stop),
            "run finished on the post-batch text turn"
        );
    }

    #[tokio::test]
    async fn resumed_denial_yields_denied_result_without_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let asks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let approval = Arc::new(SpliceApproval {
            resp: ApprovalResponse::Approve,
            asks: asks.clone(),
            parked_at_ask: Arc::new(std::sync::Mutex::new(None)),
            park_dir: None,
        });
        let (agent, sink) = resume_agent(
            ws.clone(),
            approval,
            None,
            vec![Scripted::Text("wrapped up".into())],
            vec![],
        );
        let batch = echo_batch();
        let mut ctx = restore_with_batch("the goal", &batch);
        let resume = crate::ResumeTurn {
            assistant_text: "running the batch".into(),
            tool_calls: batch,
            invalid: vec![],
            gate_records: vec![crate::GateRecord::Ready],
            parked_index: Some(1),
            parked_decision: Some(false),
            turn: 3,
            guardrails: crate::Guardrails {
                tool_calls: 0,
                model_calls: 0,
            },
            goal_text: "the goal".into(),
        };
        agent
            .resume_with_cancel(&mut ctx, resume, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            asks.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "denial is consumed from the committed decision, never re-prompted"
        );
        assert_eq!(sink.status_of("c1").as_deref(), Some("ok"), "c1 still ran");
        assert_eq!(
            sink.status_of("c2").as_deref(),
            Some("denied"),
            "c2's committed deny replays as a Denied result"
        );
        // content carries "user declined"
        let c2 = sink
            .results
            .lock()
            .unwrap()
            .iter()
            .find(|(i, _, _)| i == "c2")
            .cloned();
        assert!(c2.is_some());
    }

    #[tokio::test]
    async fn resume_without_decision_reasks_live_and_can_park_again() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let ck_dir = dir.path().join("checkpoint");
        let ck = crate::Checkpointer::new(ck_dir.clone(), [7u8; 32], "s1".into());
        let asks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let parked_at_ask = Arc::new(std::sync::Mutex::new(None));
        let approval = Arc::new(SpliceApproval {
            resp: ApprovalResponse::Approve,
            asks: asks.clone(),
            parked_at_ask: parked_at_ask.clone(),
            park_dir: Some(ck_dir.clone()),
        });
        let (agent, sink) = resume_agent(
            ws.clone(),
            approval,
            Some(ck),
            vec![Scripted::Text("wrapped up".into())],
            vec![],
        );
        let batch = echo_batch();
        let mut ctx = restore_with_batch("the goal", &batch);
        let resume = crate::ResumeTurn {
            assistant_text: "running the batch".into(),
            tool_calls: batch,
            invalid: vec![],
            gate_records: vec![crate::GateRecord::Ready],
            parked_index: Some(1),
            parked_decision: None, // unanswered ⇒ re-ask live
            turn: 3,
            guardrails: crate::Guardrails::default(),
            goal_text: "the goal".into(),
        };
        agent
            .resume_with_cancel(&mut ctx, resume, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            asks.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the unanswered parked ask re-prompts live exactly once"
        );
        assert_eq!(
            *parked_at_ask.lock().unwrap(),
            Some(true),
            "a park was re-written before the live block"
        );
        assert!(
            !crate::checkpoint::has_park(&ck_dir),
            "the answered live ask deletes the park"
        );
        assert_eq!(sink.status_of("c2").as_deref(), Some("ok"));
    }

    #[tokio::test]
    async fn guardrail_tallies_survive_resume_without_refill() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let approval = Arc::new(SpliceApproval {
            resp: ApprovalResponse::Approve,
            asks: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            parked_at_ask: Arc::new(std::sync::Mutex::new(None)),
            park_dir: None,
        });
        // Counting middleware in the stack so ToolCallCount is live. Seed the
        // resumed tally to ONE below the limit: after the resumed batch runs
        // (2 calls), the pre-turn backstop must trip on the NEXT turn.
        let (agent, sink) = resume_agent(
            ws.clone(),
            approval,
            None,
            // model would serve a call turn next, but the pre-turn backstop
            // fires before any completion — script one anyway.
            vec![Scripted::Text("never reached".into())],
            vec![Arc::new(crate::ToolCallLimit::new())],
        );
        let batch = echo_batch();
        let mut ctx = restore_with_batch("the goal", &batch);
        let resume = crate::ResumeTurn {
            assistant_text: "running the batch".into(),
            tool_calls: batch,
            invalid: vec![],
            gate_records: vec![crate::GateRecord::Ready],
            parked_index: Some(1),
            parked_decision: Some(true),
            turn: 3,
            guardrails: crate::Guardrails {
                tool_calls: (crate::TOOL_CALL_LIMIT - 1) as u64,
                model_calls: 0,
            },
            goal_text: "the goal".into(),
        };
        agent
            .resume_with_cancel(&mut ctx, resume, CancellationToken::new())
            .await
            .unwrap();
        // The batch executed…
        assert_eq!(sink.status_of("c1").as_deref(), Some("ok"));
        assert_eq!(sink.status_of("c2").as_deref(), Some("ok"));
        // …and the tally did NOT refill: seeded at LIMIT-1, +2 in the batch ⇒
        // over LIMIT, so the run ends on the guardrail Error, not Stop.
        assert_eq!(
            sink.done.lock().unwrap().last(),
            Some(&StopReason::Error),
            "the seeded tally tripped the guardrail — the budget did not refill (spec §3.8)"
        );
    }

    /// A stub Tool named "dispatch_agent": the P1 splice keys on the NAME, so a
    /// trivial registered tool is enough to prove the dispatch-kind arm gates
    /// through and executes while non-dispatch siblings are lost-result'd.
    struct DispatchStub;
    #[async_trait::async_trait]
    impl agent_tools::Tool for DispatchStub {
        fn name(&self) -> &str {
            "dispatch_agent"
        }
        fn description(&self) -> &str {
            "stub"
        }
        fn schema(&self) -> agent_tools::ToolSchema {
            agent_tools::ToolSchema {
                name: "dispatch_agent".into(),
                description: "stub".into(),
                parameters: serde_json::json!({"type": "object"}),
            }
        }
        fn intent(
            &self,
            _a: &serde_json::Value,
        ) -> Result<agent_tools::ToolIntent, agent_tools::ToolError> {
            Ok(agent_tools::ToolIntent {
                tool: "dispatch_agent".into(),
                access: agent_tools::Access::Read,
                paths: vec![],
                command: None,
                summary: "dispatch".into(),
            })
        }
        async fn execute(
            &self,
            _a: serde_json::Value,
            _ctx: &agent_tools::ToolCtx,
        ) -> Result<agent_tools::ToolOutput, agent_tools::ToolError> {
            Ok(agent_tools::ToolOutput {
                content: "child done".into(),
                display: None,
            })
        }
    }

    #[tokio::test]
    async fn dispatch_kind_resume_lost_results_non_dispatch_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let marker = ws.join("side-effect.txt");
        // Register execute_command + a dispatch_agent stub.
        let mut r = ToolRegistry::new();
        r.register(Arc::new(agent_tools::shell::ExecuteCommand));
        r.register(Arc::new(DispatchStub));
        let tools = Arc::new(r);
        let pol = Arc::new(RulePolicy {
            workspace: ws.clone(),
            command_allowlist: vec![],
            command_denylist: vec![],
        });
        let sink = Arc::new(IdSink::default());
        let agent = AgentLoop::new(
            Arc::new(ScriptedModel::new(vec![Scripted::Text(
                "wrapped up".into(),
            )])),
            Arc::new(PassthroughProtocol),
            tools,
            pol,
            Arc::new(SpliceApproval {
                resp: ApprovalResponse::Approve,
                asks: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                parked_at_ask: Arc::new(std::sync::Mutex::new(None)),
                park_dir: None,
            }),
            sink.clone(),
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                temperature: 0.0,
                workspace: ws.clone(),
                tool_timeout: std::time::Duration::from_secs(5),
                sandbox: Arc::new(agent_tools::HostExecutor),
                ..Default::default()
            },
        );
        let batch = vec![
            ToolCall {
                id: "c1".into(),
                name: "execute_command".into(),
                // would create the marker file if it ran
                args: serde_json::json!({"command":
                    format!("touch {}", marker.display())}),
            },
            ToolCall {
                id: "c2".into(),
                name: "dispatch_agent".into(),
                args: serde_json::json!({}),
            },
        ];
        let mut ctx = restore_with_batch("the goal", &batch);
        // dispatch-kind resume: parked_index None, whole batch pre-decided Ready.
        let resume = crate::ResumeTurn {
            assistant_text: "running the batch".into(),
            tool_calls: batch,
            invalid: vec![],
            gate_records: vec![crate::GateRecord::Ready, crate::GateRecord::Ready],
            parked_index: None,
            parked_decision: None,
            turn: 3,
            guardrails: crate::Guardrails::default(),
            goal_text: "the goal".into(),
        };
        agent
            .resume_with_cancel(&mut ctx, resume, CancellationToken::new())
            .await
            .unwrap();
        // The non-dispatch sibling must NOT execute: synthetic lost-result.
        assert_eq!(
            sink.status_of("c1").as_deref(),
            Some("error"),
            "the non-dispatch sibling is lost-result'd, not re-executed"
        );
        assert!(
            !marker.exists(),
            "the execute_command side effect must not replay"
        );
        // The dispatch_agent call gates through gate_preapproved and executes.
        assert_eq!(
            sink.status_of("c2").as_deref(),
            Some("ok"),
            "dispatch_agent re-executes on a dispatch-kind resume"
        );
        // exactly one ToolStart per call id, no prompts.
        assert_eq!(sink.starts_for("c1"), 1);
        assert_eq!(sink.starts_for("c2"), 1);
    }
}
