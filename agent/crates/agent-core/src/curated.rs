use crate::compactor::{compaction_is_worthwhile, run_compaction};
use crate::context::{
    evict_start, message_tokens, orphaned_tool_positions, recall_block,
    snap_split_to_unit_boundary, ContextManager, MaintCtx, MaintReport,
    DEFAULT_RECALL_TOKEN_BUDGET,
};
use crate::event::{AgentEvent, ContextEvent};
use crate::offload::OffloadStore;
use crate::offload_policy::{
    capped_preview, placeholder_for, select_offloads, select_oversized, OffloadConfig,
};
use agent_model::{Message, Role};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Default fraction of `model_limit` at which `maintain` triggers a compaction pass.
pub const DEFAULT_HIGH_WATER_PCT: f32 = 0.85;

/// A curating context manager. Pins `system → re-grounding → recall → compaction
/// summary → windowed recent history`, offloads stale/large tool results into a
/// side table each turn, and compacts the old span when over the high-water mark.
pub struct CuratedContext {
    system: Message,
    goal: Option<Message>,
    history: Vec<Message>,
    recall: Vec<String>,
    recall_budget: usize,
    pub(crate) compaction_summary: Option<Message>,
    pub(crate) store: Arc<dyn OffloadStore>,
    pub(crate) config: OffloadConfig,
    pub(crate) high_water_pct: f32,
    pub(crate) compact_flag: Arc<AtomicBool>,
    /// `(messages, est_tokens)` omitted by eviction at the last maintain pass;
    /// dedups repeated identical Evicted events while the window stays saturated,
    /// yet re-emits when either the count OR the token estimate changes (offload
    /// can shrink the evicted span's tokens without changing its message count).
    last_evicted: (usize, usize),
}

impl CuratedContext {
    pub fn new(
        system: Message,
        store: Arc<dyn OffloadStore>,
        compact_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            system,
            goal: None,
            history: Vec::new(),
            recall: Vec::new(),
            recall_budget: DEFAULT_RECALL_TOKEN_BUDGET,
            compaction_summary: None,
            store,
            config: OffloadConfig::default(),
            high_water_pct: DEFAULT_HIGH_WATER_PCT,
            compact_flag,
            last_evicted: (0, 0),
        }
    }

    pub fn with_recall_budget(mut self, budget: usize) -> Self {
        self.recall_budget = budget;
        self
    }

    pub fn with_offload_config(mut self, config: OffloadConfig) -> Self {
        self.config = config;
        self
    }

    /// Override the fraction of `model_limit` at which `maintain` triggers a
    /// compaction pass (default `DEFAULT_HIGH_WATER_PCT`). A value `>= 1.0`
    /// effectively disables automatic compaction.
    pub fn with_high_water_pct(mut self, pct: f32) -> Self {
        self.high_water_pct = pct;
        self
    }

    /// The pinned blocks, in assembly order, that precede windowed history.
    fn pinned(&self) -> Vec<Message> {
        let mut out = vec![self.system.clone()];
        if let Some(g) = &self.goal {
            out.push(g.clone());
        }
        if let Some(r) = recall_block(&self.recall, self.recall_budget) {
            out.push(r);
        }
        if let Some(c) = &self.compaction_summary {
            out.push(c.clone());
        }
        out
    }

    /// Estimated tokens of the pinned blocks, counted over references — the
    /// non-cloning twin of `self.pinned().iter().map(message_tokens).sum()`
    /// (the recall block is constructed regardless). Same total, no per-message
    /// clone of `system`/`goal`/`compaction_summary`.
    fn pinned_tokens(&self) -> usize {
        let mut t = message_tokens(&self.system);
        if let Some(g) = &self.goal {
            t += message_tokens(g);
        }
        if let Some(r) = recall_block(&self.recall, self.recall_budget) {
            t += message_tokens(&r);
        }
        if let Some(c) = &self.compaction_summary {
            t += message_tokens(c);
        }
        t
    }

    /// Borrow history (used by the compaction-failure test).
    #[cfg(test)]
    pub(crate) fn history(&self) -> &[Message] {
        &self.history
    }

    /// Per-category breakdown of the current context window, for the explorer UI.
    /// Token figures are estimates; the faithful total comes from server usage.
    pub fn snapshot(&self, model_limit: usize, turn: usize) -> crate::ContextSnapshot {
        crate::snapshot::build_snapshot(
            turn,
            model_limit,
            &self.system,
            self.goal.as_ref(),
            &self.recall,
            self.recall_budget,
            self.compaction_summary.as_ref(),
            &self.history,
        )
    }
}

#[async_trait]
impl ContextManager for CuratedContext {
    fn append(&mut self, msg: Message) {
        self.history.push(msg);
    }

    fn set_system(&mut self, system: Message) {
        self.system = system;
    }

    fn set_recall(&mut self, items: Vec<String>) {
        self.recall = items;
    }

    fn set_goal(&mut self, goal: String) {
        if self.goal.is_none() {
            self.goal = Some(Message::system(format!("Original goal: {goal}")));
        }
    }

    fn build(&self, model_limit: usize) -> Vec<Message> {
        let pinned = self.pinned();
        let budget = model_limit.saturating_sub(self.pinned_tokens());
        // Walk history newest-first in turn units, keep whole units while they
        // fit — never split a tool_calls parent from its Role::Tool results.
        let start = evict_start(&self.history, budget);
        let mut out = pinned;
        out.extend(self.history[start..].iter().cloned());
        debug_assert!(
            orphaned_tool_positions(&out).is_empty(),
            "CuratedContext::build produced an orphaned tool message"
        );
        out
    }

    async fn maintain(&mut self, deps: &MaintCtx<'_>) -> MaintReport {
        let mut report = MaintReport::default();

        // (0) Ingestion cap — an oversized fresh result is offloaded whole
        // before it can reach a model call; the window keeps a bounded
        // preview + recall marker. Age is irrelevant here, only size.
        let cap = self.config.max_result_bytes;
        for hit in select_oversized(&self.history, &self.config) {
            let content = hit.entry.content.clone();
            let tool = hit.entry.tool_name.clone();
            self.lift(hit, &mut report, deps, |id| {
                capped_preview(&content, cap, id, &tool)
            });
        }

        // (a) Deterministic age-based offload — sync, cheap, every turn.
        for hit in select_offloads(&self.history, &self.config) {
            let tool = hit.entry.tool_name.clone();
            let kind = hit.entry.kind.clone();
            let bytes = hit.entry.bytes;
            self.lift(hit, &mut report, deps, |id| {
                placeholder_for(id, &tool, &kind, bytes)
            });
        }

        // (b) Compaction — async, gated by the high-water mark or an explicit request.
        let requested = self.compact_flag.swap(false, Ordering::SeqCst);
        let over_high_water = {
            let built = self.build(deps.model_limit);
            let used: usize = built.iter().map(message_tokens).sum();
            (used as f32) > (deps.model_limit as f32 * self.high_water_pct)
        };
        if (requested || over_high_water) && self.history.len() > self.config.keep_recent + 1 {
            self.compact_old_span(deps, &mut report).await;
        }

        // (c) Eviction visibility — runs on EVERY maintain exit.
        self.emit_eviction(deps);
        report
    }

    fn request_compaction(&mut self) {
        self.compact_flag.store(true, Ordering::SeqCst);
    }
}

impl CuratedContext {
    /// Store a hit's full content in the offload table and replace the window
    /// copy with whatever `replacement` renders for the assigned id. Shared by
    /// the ingestion-cap and age-based offload passes.
    fn lift(
        &mut self,
        hit: crate::offload_policy::OffloadHit,
        report: &mut MaintReport,
        deps: &MaintCtx<'_>,
        replacement: impl FnOnce(crate::OffloadId) -> String,
    ) {
        let idx = hit.history_index;
        let tool = hit.entry.tool_name.clone();
        let bytes = hit.entry.bytes;
        let id = self.store.put(hit.entry);
        self.history[idx].content = replacement(id);
        report.offloaded += 1;
        report.offloaded_bytes += bytes;
        deps.sink.emit(AgentEvent::Context(ContextEvent::Offloaded {
            id,
            bytes,
            tool,
        }));
    }

    /// Compact the old span into the pinned summary. Extracted from `maintain`
    /// so its early exits cannot skip the eviction check that follows.
    async fn compact_old_span(&mut self, deps: &MaintCtx<'_>, report: &mut MaintReport) {
        // Snap left to a unit boundary so the cut never separates a
        // tool_calls parent from its results; the torn turn lands wholly
        // in the recent window (keep_recent temporarily keeps a bit more).
        let split = snap_split_to_unit_boundary(
            &self.history,
            self.history.len() - self.config.keep_recent,
        );
        let prior = self.compaction_summary.clone();
        // User turns are durable, author-authored instructions — the facts most
        // costly to lose. Keep them VERBATIM and never feed them to the lossy
        // summarizer; only assistant/tool chatter in the old span is compacted, with
        // the prior summary carried forward so re-compaction accumulates instead of
        // collapsing to the most recent turn.
        let (verbatim, to_summarize): (Vec<Message>, Vec<Message>) = self.history[..split]
            .iter()
            .cloned()
            .partition(|m| matches!(m.role, Role::User));
        // "Worthwhile" (non-empty AND a net token win) is judged against everything
        // actually replaced: the prior summary plus the chatter being summarized.
        let mut replaced: Vec<Message> = prior.iter().cloned().collect();
        replaced.extend_from_slice(&to_summarize);
        let tokens_before: usize = replaced.iter().map(message_tokens).sum();
        // Nothing summarizable beyond the prior summary — the verbatim user turns are
        // already the most compact faithful form, so don't burn a model call.
        if to_summarize.is_empty() {
            return;
        }
        match run_compaction(&to_summarize, prior.as_ref(), deps.model, deps.cancel).await {
            Ok(summary) if compaction_is_worthwhile(&summary, &replaced) => {
                let tokens_after = message_tokens(&summary);
                // Reassemble history: verbatim user instructions (chronological), then
                // the recent window. The summarized chatter becomes the pinned summary.
                let recent = self.history.split_off(split);
                self.history = verbatim;
                self.history.extend(recent);
                self.compaction_summary = Some(summary);
                report.compacted_turns = to_summarize.len();
                deps.sink.emit(AgentEvent::Context(ContextEvent::Compacted {
                    turns_replaced: to_summarize.len(),
                    tokens_before,
                    tokens_after,
                }));
            }
            Ok(_) => {
                tracing::debug!("compaction not worthwhile; discarded");
            }
            Err(e) => {
                tracing::warn!(error = %e, "compaction failed; leaving history intact");
                deps.sink
                    .emit(AgentEvent::Context(ContextEvent::CompactionFailed {
                        reason: e.to_string(),
                    }));
            }
        }
    }

    /// Emit `ContextEvent::Evicted` when the built window omits history
    /// messages and the count changed since the last pass.
    fn emit_eviction(&mut self, deps: &MaintCtx<'_>) {
        let budget = deps.model_limit.saturating_sub(self.pinned_tokens());
        let start = evict_start(&self.history, budget);
        let est_tokens: usize = self.history[..start].iter().map(message_tokens).sum();
        // Dedup on (count, tokens): re-emit when EITHER changes, so an offload
        // that shrinks the evicted span's tokens without moving the count still
        // surfaces the reduced pressure.
        let key = (start, est_tokens);
        if start > 0 && key != self.last_evicted {
            deps.sink.emit(AgentEvent::Context(ContextEvent::Evicted {
                messages: start,
                est_tokens,
            }));
        }
        self.last_evicted = key;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::MaintCtx;
    use crate::event::EventSink;
    use crate::offload::{InMemoryOffloadStore, OffloadKind};
    use crate::testkit::{CollectingSink, Scripted, ScriptedModel};
    use agent_model::{ModelClient, Role};
    use tokio_util::sync::CancellationToken;

    fn ctx() -> CuratedContext {
        CuratedContext::new(
            Message::system("SYS"),
            Arc::new(InMemoryOffloadStore::new()),
            Arc::new(AtomicBool::new(false)),
        )
    }

    fn parent(id: &str) -> Message {
        Message::assistant(
            "calling",
            Some(vec![agent_tools::ToolCall {
                id: id.into(),
                name: "shell".into(),
                args: serde_json::json!({}),
            }]),
        )
    }

    fn maint_deps<'a>(
        model: &'a Arc<dyn ModelClient>,
        sink: &'a Arc<dyn EventSink>,
        cancel: &'a CancellationToken,
    ) -> MaintCtx<'a> {
        MaintCtx {
            model_limit: 100_000,
            model,
            sink,
            cancel,
        }
    }

    #[test]
    fn build_assembly_order_system_goal_recall_then_history() {
        let mut c = ctx();
        c.set_goal("ship the feature".into());
        c.set_recall(vec!["user likes rust".into()]);
        c.append(Message::user("hello"));
        let built = c.build(100_000);
        assert!(matches!(built[0].role, Role::System));
        assert_eq!(built[0].content, "SYS");
        assert_eq!(built[1].content, "Original goal: ship the feature");
        assert!(built[2].content.starts_with("Relevant memories"));
        assert_eq!(built.last().unwrap().content, "hello");
    }

    #[test]
    fn set_goal_is_set_once() {
        let mut c = ctx();
        c.set_goal("first goal".into());
        c.set_goal("second goal".into());
        let built = c.build(100_000);
        assert_eq!(built[1].content, "Original goal: first goal");
    }

    #[test]
    fn goal_block_survives_tiny_limit() {
        let mut c = ctx();
        c.set_goal("the goal".into());
        for i in 0..50 {
            c.append(Message::user(format!(
                "message number {i} with padding text"
            )));
        }
        let built = c.build(40);
        assert!(built.iter().any(|m| m.content == "Original goal: the goal"));
        assert!(built.len() < 51);
    }

    #[test]
    fn build_returns_pinned_plus_history_under_limit() {
        let mut c = ctx();
        c.append(Message::user("hi"));
        let built = c.build(100_000);
        assert_eq!(built.len(), 2); // system + history (no goal/recall set)
    }

    #[tokio::test]
    async fn maintain_offloads_stale_large_error_to_store_and_leaves_placeholder() {
        let mut c = ctx().with_offload_config(OffloadConfig {
            keep_recent: 0,
            ..Default::default()
        });
        let big_err = format!("ERROR: {}", "x".repeat(400));
        c.append(parent("call-1")); // tool results always follow a tool_calls parent
        c.append(Message::tool("call-1", "shell", big_err.clone()));

        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;

        assert_eq!(report.offloaded, 1);
        assert_eq!(c.store.len(), 1);
        // Live message is now a placeholder; full content recoverable from the store.
        let built = c.build(100_000);
        let tool_msg = built.iter().find(|m| matches!(m.role, Role::Tool)).unwrap();
        assert!(tool_msg.content.starts_with("[tool_result#1 offloaded"));
        assert_eq!(c.store.get(1).unwrap().content, big_err);
    }

    #[tokio::test]
    async fn maintain_is_idempotent() {
        let mut c = ctx().with_offload_config(OffloadConfig {
            keep_recent: 0,
            ..Default::default()
        });
        c.append(parent("call-1")); // tool results always follow a tool_calls parent
        c.append(Message::tool(
            "call-1",
            "shell",
            format!("ERROR: {}", "x".repeat(400)),
        ));
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        let report2 = c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        assert_eq!(report2.offloaded, 0, "second pass must not re-offload");
        assert_eq!(c.store.len(), 1);
    }

    #[tokio::test]
    async fn maintain_compacts_old_span_when_over_high_water() {
        let mut c = ctx();
        c.high_water_pct = 0.0; // force compaction regardless of size
        c.config.keep_recent = 1;
        for i in 0..6 {
            // Assistant chatter (not user instructions) is what gets summarized.
            c.append(Message::assistant(
                format!("turn {i} with a fair bit of padding text here"),
                None,
            ));
        }
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "compact summary".into(),
        )]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;

        assert!(report.compacted_turns > 0);
        let built = c.build(100_000);
        // A compaction summary block is present and the most-recent turn survives verbatim.
        assert!(built.iter().any(|m| m.content.contains("compact summary")));
        assert!(built.iter().any(|m| m.content.contains("turn 5")));
    }

    #[tokio::test]
    async fn maintain_keeps_user_instructions_verbatim_through_compaction() {
        // The durable facts live in user turns; compaction must never route them
        // through the lossy summarizer — they survive byte-for-byte.
        let mut c = ctx();
        c.high_water_pct = 0.0; // force compaction
        c.config.keep_recent = 1;
        // Interleave user instructions with assistant chatter.
        for i in 0..4 {
            c.append(Message::user(format!("instruction {i}: add value {i}{i}")));
            c.append(Message::assistant(
                format!("ok, acknowledged {i}, lots of filler chatter"),
                None,
            ));
        }
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "chatter summary".into(),
        )]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;

        assert!(
            report.compacted_turns > 0,
            "assistant chatter should have been summarized"
        );
        let built = c.build(100_000);
        // Every user instruction is still present verbatim, none lost to summarization.
        for i in 0..4 {
            let want = format!("instruction {i}: add value {i}{i}");
            assert!(
                built.iter().any(|m| m.content == want),
                "user instruction {i} must survive compaction verbatim"
            );
        }
        // And a compaction summary block exists for the chatter.
        assert!(built.iter().any(|m| m.content.contains("chatter summary")));
    }

    #[test]
    fn curated_snapshot_reports_system_recall_and_messages() {
        let mut c = CuratedContext::new(
            Message::system("SYS"),
            Arc::new(crate::offload::InMemoryOffloadStore::new()),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        c.set_recall(vec!["user likes rust".into()]);
        c.append(Message::user("hello"));
        let snap = c.snapshot(10_000, 7);
        assert_eq!(snap.turn, 7);
        assert_eq!(snap.model_limit, 10_000);
        assert!(snap.segments.iter().any(|s| s.category == "system"));
        assert!(snap.segments.iter().any(|s| s.category == "memory"));
        let msgs = snap
            .segments
            .iter()
            .find(|s| s.category == "messages")
            .unwrap();
        assert_eq!(msgs.count, 1);
    }

    #[test]
    fn curated_build_never_orphans_tool_results() {
        let mut c = ctx();
        c.append(Message::user(
            "old old old message with lots of padding text",
        ));
        c.append(parent("c1"));
        c.append(Message::tool("c1", "shell", "y".repeat(120)));
        c.append(Message::user("recent"));
        use crate::context::{built_tokens, message_tokens, orphaned_tool_positions};
        let tool_result_t = message_tokens(&Message::tool("c1", "shell", "y".repeat(120)));
        let recent_t = message_tokens(&Message::user("recent"));
        let sys_t = message_tokens(&Message::system("SYS"));
        let limit = sys_t + recent_t + tool_result_t + 2;
        let built = c.build(limit);
        assert!(orphaned_tool_positions(&built).is_empty());
        let _ = built_tokens(&built); // silence unused-import if optimized differently
    }

    #[tokio::test]
    async fn compaction_split_snaps_to_turn_boundary() {
        use crate::context::orphaned_tool_positions;
        let mut c = ctx();
        c.high_water_pct = 0.0; // force compaction
                                // keep_recent = 2 lands the naive split between parent and result.
        c.config.keep_recent = 2;
        c.append(Message::assistant("chatter zero with padding", None));
        c.append(Message::assistant("chatter one with padding", None));
        c.append(parent("c1"));
        c.append(Message::tool("c1", "shell", "result one")); // naive split cuts HERE
        c.append(Message::user("newest instruction"));
        let model: Arc<dyn ModelClient> =
            Arc::new(ScriptedModel::new(vec![Scripted::Text("summary".into())]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        // History after compaction has no orphaned tool results...
        assert!(
            orphaned_tool_positions(c.history()).is_empty(),
            "snapped split must keep parent+result together: {:?}",
            c.history()
                .iter()
                .map(|m| (&m.role, &m.content))
                .collect::<Vec<_>>()
        );
        // ...and the torn turn stayed whole in the recent window.
        let has_result = c
            .history()
            .iter()
            .any(|m| m.tool_call_id.as_deref() == Some("c1"));
        let has_parent = c.history().iter().any(|m| m.tool_calls.is_some());
        assert!(
            has_result && has_parent,
            "torn turn must land wholly in recent"
        );
    }

    #[test]
    fn curated_build_budget_sweep_never_orphans() {
        use crate::context::{built_tokens, orphaned_tool_positions};
        let mut c = ctx();
        c.set_goal("sweep goal".into());
        c.append(Message::user("intro message with padding"));
        c.append(parent("c1"));
        c.append(Message::tool("c1", "shell", "a".repeat(100)));
        c.append(Message::user("middle instruction"));
        c.append(parent("c2"));
        c.append(Message::tool("c2", "shell", "b".repeat(80)));
        c.append(Message::user("latest"));
        let total = built_tokens(&c.build(usize::MAX)) + 16;
        for limit in 1..=total {
            let built = c.build(limit);
            assert!(
                orphaned_tool_positions(&built).is_empty(),
                "orphan at model_limit={limit}"
            );
        }
    }

    #[tokio::test]
    async fn maintain_emits_evicted_once_per_change() {
        let mut c = ctx();
        c.high_water_pct = 2.0; // compaction off; isolate eviction
        for i in 0..30 {
            c.append(Message::user(format!(
                "filler message number {i} with padding text"
            )));
        }
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![]));
        let sink = Arc::new(CollectingSink::default());
        let sink_dyn: Arc<dyn EventSink> = sink.clone();
        let cancel = CancellationToken::new();
        let mut deps = maint_deps(&model, &sink_dyn, &cancel);
        deps.model_limit = 100; // tiny window → eviction certain
        c.maintain(&deps).await;
        let events = sink.events.lock().unwrap().clone();
        let evicted: Vec<_> = events
            .iter()
            .filter(|e| e.starts_with("evicted:"))
            .collect();
        assert_eq!(
            evicted.len(),
            1,
            "one Evicted on first saturated pass: {events:?}"
        );

        // Same state → same count → no duplicate event.
        c.maintain(&deps).await;
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(
            events.iter().filter(|e| e.starts_with("evicted:")).count(),
            1,
            "unchanged eviction count must not re-emit"
        );

        // Same count, different tokens → re-emit. Grow a message deep inside the
        // already-evicted span (far below the kept-window boundary): the evicted
        // message COUNT is unchanged, but its token estimate rises — the (count,
        // tokens) key must still trip a fresh Evicted.
        let start_before = evict_start(
            &c.history,
            deps.model_limit.saturating_sub(c.pinned_tokens()),
        );
        c.history[0]
            .content
            .push_str(" with a great deal of additional padding text appended to grow its tokens");
        let start_after = evict_start(
            &c.history,
            deps.model_limit.saturating_sub(c.pinned_tokens()),
        );
        assert_eq!(
            start_before, start_after,
            "evicted count must be unchanged for this case to isolate a token-only change"
        );
        c.maintain(&deps).await;
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(
            events.iter().filter(|e| e.starts_with("evicted:")).count(),
            2,
            "same count but different tokens must re-emit"
        );

        // More history → count changes → re-emit.
        c.append(Message::user(
            "one more message with plenty of padding here",
        ));
        c.maintain(&deps).await;
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(
            events.iter().filter(|e| e.starts_with("evicted:")).count(),
            3
        );
    }

    #[tokio::test]
    async fn maintain_emits_nothing_under_budget() {
        let mut c = ctx();
        c.append(Message::user("hello"));
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![]));
        let sink = Arc::new(CollectingSink::default());
        let sink_dyn: Arc<dyn EventSink> = sink.clone();
        let cancel = CancellationToken::new();
        c.maintain(&maint_deps(&model, &sink_dyn, &cancel)).await;
        let events = sink.events.lock().unwrap().clone();
        assert!(
            !events.iter().any(|e| e.starts_with("evicted:")),
            "no Evicted under budget: {events:?}"
        );
    }

    #[tokio::test]
    async fn ingestion_cap_offloads_fresh_oversized_result_before_next_build() {
        let store = Arc::new(InMemoryOffloadStore::new());
        let flag = Arc::new(AtomicBool::new(false));
        let mut ctx = CuratedContext::new(Message::system("s"), store.clone(), flag)
            .with_offload_config(OffloadConfig {
                max_result_bytes: 1024,
                ..Default::default()
            });
        let big = "x".repeat(50_000);
        ctx.append(parent("c1")); // tool results always follow a tool_calls parent
        ctx.append(Message::tool("c1", "shell", &big));

        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![]));
        let sink = Arc::new(CollectingSink::default());
        let sink_dyn: Arc<dyn EventSink> = sink.clone();
        let cancel = CancellationToken::new();
        let mut deps = maint_deps(&model, &sink_dyn, &cancel);
        deps.model_limit = 1_000_000; // no compaction/eviction interference

        let report = ctx.maintain(&deps).await;

        assert_eq!(report.offloaded, 1);
        assert_eq!(report.offloaded_bytes, 50_000);
        let msg = &ctx.history()[1];
        assert!(msg.content.len() <= 1024, "window copy exceeds cap");
        assert!(msg.content.contains("truncated: showing first"));
        assert_eq!(msg.tool_call_id.as_deref(), Some("c1"), "id must survive");
        // Full content stored, recallable.
        let entry = store.get(1).expect("entry stored");
        assert_eq!(entry.content.len(), 50_000);
        // Offloaded event emitted.
        assert!(
            sink.events
                .lock()
                .unwrap()
                .iter()
                .any(|e| e == "offloaded:1"),
            "Offloaded event must be emitted"
        );
        // Second pass is a no-op (idempotent).
        let report2 = ctx.maintain(&deps).await;
        assert_eq!(report2.offloaded, 0);
    }

    #[tokio::test]
    async fn capped_result_survives_build_under_a_small_model_limit() {
        // A fresh oversized tool result (50 000 B) would blow any small window
        // and, as its own turn unit, force an over-limit request. After the
        // ingestion cap replaces it with a bounded preview, the turn unit is
        // small enough to survive `build` under a limit far below what the
        // uncapped result needs — without orphaning the tool message.
        let store = Arc::new(InMemoryOffloadStore::new());
        let flag = Arc::new(AtomicBool::new(false));
        let mut ctx = CuratedContext::new(Message::system("s"), store.clone(), flag)
            .with_offload_config(OffloadConfig {
                max_result_bytes: 1024,
                ..Default::default()
            });
        ctx.append(parent("c1")); // parent must precede the Role::Tool result
        ctx.append(Message::tool("c1", "shell", "x".repeat(50_000)));

        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![]));
        let sink_dyn: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let mut deps = maint_deps(&model, &sink_dyn, &cancel);
        deps.model_limit = 1_000_000; // no compaction/eviction interference
        ctx.maintain(&deps).await;

        // Small model_limit: larger than pinned + capped unit (~a few hundred
        // tokens) but far below the ~12 500 tokens the uncapped 50 KB needs.
        let small_limit = 2 * 1024 / 4;
        let built = ctx.build(small_limit);
        // `build` has a debug_assert against orphaned tool messages; reaching
        // here means integrity held. The capped preview must be present.
        let preview = built
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some("c1"))
            .expect("capped tool message survives build");
        assert!(preview.content.len() <= 1024, "preview within cap");
        assert!(preview.content.contains("truncated: showing first"));
    }

    #[tokio::test]
    async fn capped_preview_is_age_offloaded_to_a_placeholder_later() {
        // keep_recent 0 lets the age pass run on the same maintain call: the
        // eager pass stores the full content (#1), then the age pass lifts the
        // preview into a second small entry (#2) whose content still carries
        // the marker to #1 — the recall chain stays intact.
        let store = Arc::new(InMemoryOffloadStore::new());
        let flag = Arc::new(AtomicBool::new(false));
        let mut ctx = CuratedContext::new(Message::system("s"), store.clone(), flag)
            .with_offload_config(OffloadConfig {
                max_result_bytes: 1024,
                output_min_bytes: 100,
                keep_recent: 0,
                ..Default::default()
            });
        ctx.append(parent("c1")); // tool results always follow a tool_calls parent
        ctx.append(Message::tool("c1", "shell", "x".repeat(50_000)));

        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![]));
        let sink_dyn: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let mut deps = maint_deps(&model, &sink_dyn, &cancel);
        deps.model_limit = 1_000_000;

        let report = ctx.maintain(&deps).await;

        assert_eq!(report.offloaded, 2, "eager + age in one pass");
        let msg = &ctx.history()[1];
        assert!(msg.content.starts_with("[tool_result#2 offloaded:"));
        assert!(store
            .get(2)
            .unwrap()
            .content
            .contains("context_recall(id: 1"));
        assert_eq!(store.get(1).unwrap().content.len(), 50_000);
    }

    #[tokio::test]
    async fn oversized_error_result_is_capped_too() {
        let store = Arc::new(InMemoryOffloadStore::new());
        let flag = Arc::new(AtomicBool::new(false));
        let mut ctx = CuratedContext::new(Message::system("s"), store.clone(), flag)
            .with_offload_config(OffloadConfig {
                max_result_bytes: 1024,
                ..Default::default()
            });
        ctx.append(parent("c1")); // tool results always follow a tool_calls parent
        ctx.append(Message::tool(
            "c1",
            "shell",
            format!("ERROR: {}", "e".repeat(50_000)),
        ));

        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![]));
        let sink_dyn: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let mut deps = maint_deps(&model, &sink_dyn, &cancel);
        deps.model_limit = 1_000_000;

        ctx.maintain(&deps).await;

        assert!(ctx.history()[1].content.len() <= 1024);
        assert!(matches!(store.get(1).unwrap().kind, OffloadKind::Error));
    }

    #[tokio::test]
    async fn request_compaction_takes_the_compaction_path_on_next_maintain() {
        // History stays below the high-water mark, so ONLY the requested flag
        // can trigger compaction — proving request_compaction() wires through.
        let mut c = ctx();
        c.high_water_pct = 2.0; // disable size-triggered compaction
        c.config.keep_recent = 1;
        for i in 0..6 {
            c.append(Message::assistant(
                format!("turn {i} with a fair bit of padding text here"),
                None,
            ));
        }
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "compact summary".into(),
        )]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        c.request_compaction();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        assert!(report.compacted_turns > 0);
    }

    #[tokio::test]
    async fn maintain_leaves_history_intact_when_compaction_fails() {
        let mut c = ctx();
        c.high_water_pct = 0.0;
        c.config.keep_recent = 1;
        for i in 0..6 {
            c.append(Message::assistant(
                format!("turn {i} with padding text"),
                None,
            ));
        }
        let before = c.history().len();
        // Empty script => stream yields nothing => empty summary => not worthwhile => discarded.
        let model: Arc<dyn ModelClient> =
            Arc::new(ScriptedModel::new(vec![Scripted::Text(String::new())]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        assert_eq!(report.compacted_turns, 0);
        assert_eq!(
            c.history().len(),
            before,
            "history must be untouched on failed/empty compaction"
        );
    }
}
