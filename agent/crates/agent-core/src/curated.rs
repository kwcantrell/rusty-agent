use crate::compactor::{compaction_is_worthwhile, run_compaction, run_extraction};
use crate::context::{
    estimate_tokens, message_tokens, orphaned_tool_positions, plan_retention, recall_block,
    snap_split_to_unit_boundary, turn_unit_ranges, ContextManager, MaintCtx, MaintReport,
    DEFAULT_RECALL_TOKEN_BUDGET,
};
use crate::event::{AgentEvent, ContextEvent};
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
    /// Extracted fact lines from folded (evicted) user units — rendered as a
    /// pinned ledger block. Append-only; never re-summarized (no generation
    /// loss by construction); capped at `FOLDED_FACTS_MAX_TOKENS`.
    pub(crate) folded_facts: Vec<String>,
    pub(crate) artifacts: Arc<crate::SessionArtifacts>,
    /// Prepended to artifact names; children get "sub{n}-" (spec §5.7).
    artifact_prefix: String,
    seq: u64,
    /// History-file state for the summary pointer (spec §5.5, E4).
    history_has_spans: bool,
    history_incomplete: bool,
    /// `## folded-{seq}` sections cited by the ledger (per-batch granularity).
    folded_sections: Vec<u64>,
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
        artifacts: Arc<crate::SessionArtifacts>,
        compact_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            system,
            goal: None,
            history: Vec::new(),
            recall: Vec::new(),
            recall_budget: DEFAULT_RECALL_TOKEN_BUDGET,
            compaction_summary: None,
            folded_facts: Vec::new(),
            artifacts,
            artifact_prefix: String::new(),
            seq: 0,
            history_has_spans: false,
            history_incomplete: false,
            folded_sections: Vec::new(),
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

    pub fn with_artifact_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.artifact_prefix = prefix.into();
        self
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
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
        match (&self.goal, self.folded_facts.is_empty()) {
            // The ledger rides INSIDE the goal block: the goal block is the
            // one pinned region with demonstrated per-run attention (its fact
            // is reproduced in 100% of observed runs), while a standalone
            // pinned ledger was used only intermittently at the decisive call
            // — pinned-block salience is unreliable, in line with the
            // marker-salience learning.
            (Some(g), false) => out.push(Message::system(format!(
                "{}\n\n{}",
                g.content,
                self.folded_block_body()
            ))),
            (Some(g), true) => out.push(g.clone()),
            (None, false) => out.push(Message::system(self.folded_block_body())),
            (None, true) => {}
        }
        if let Some(r) = recall_block(&self.recall, self.recall_budget) {
            out.push(r);
        }
        if let Some(c) = &self.compaction_summary {
            let mut msg = c.clone();
            if let Some(p) = self.history_pointer() {
                msg.content = format!("{}\n\n{p}", msg.content);
            }
            out.push(msg);
        }
        out
    }

    /// The ledger of facts extracted from folded user units. Compact
    /// (`name = value` lines), always visible, and requiring NO model action.
    /// Numbered lines + an explicit copy-all directive: transcription from an
    /// unnumbered list was observed dropping a contiguous mid-list block;
    /// numbering makes the block a checklist and a skipped line visible.
    /// Task-conditional wording so routine turns aren't over-influenced.
    fn folded_block_body(&self) -> String {
        let sections = self
            .folded_sections
            .iter()
            .map(|s| format!("## folded-{s}"))
            .collect::<Vec<_>>()
            .join(", ");
        let lines = self
            .folded_facts
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{}. {}", i + 1, l))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "Ledger of earlier user instructions (facts extracted verbatim; the full \
             original messages are preserved in conversation_history/history.md, sections \
             {sections} — read_file or grep it if ever needed). When a task needs ALL \
             earlier instructions — e.g. assembling a final list, manifest, or report — \
             copy EVERY numbered line below, without skipping any:\n{lines}"
        )
    }

    /// The ledger as a standalone message, for size accounting (`fold` cap).
    fn folded_block(&self) -> Message {
        Message::system(self.folded_block_body())
    }

    /// Estimated tokens of the pinned blocks. Kept in lockstep with
    /// `pinned()`; the goal/ledger merge means the ledger body is counted
    /// alongside the goal it rides in (the "\n\n" joiner is sub-token noise).
    fn pinned_tokens(&self) -> usize {
        let mut t = message_tokens(&self.system);
        if let Some(g) = &self.goal {
            t += message_tokens(g);
        }
        if !self.folded_facts.is_empty() {
            t += message_tokens(&self.folded_block());
        }
        if let Some(r) = recall_block(&self.recall, self.recall_budget) {
            t += message_tokens(&r);
        }
        if let Some(c) = &self.compaction_summary {
            // Lockstep with pinned(): count the composed summary+pointer message.
            let mut msg = c.clone();
            if let Some(p) = self.history_pointer() {
                msg.content = format!("{}\n\n{p}", msg.content);
            }
            t += message_tokens(&msg);
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
        let ledger = (!self.folded_facts.is_empty()).then(|| self.folded_block());
        crate::snapshot::build_snapshot(
            turn,
            model_limit,
            &self.system,
            self.goal.as_ref(),
            ledger.as_ref(),
            &self.folded_facts,
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

    fn system(&self) -> Option<&Message> {
        Some(&self.system)
    }

    fn set_goal(&mut self, goal: String) {
        if self.goal.is_none() {
            // Cap the pin at GOAL_MAX_TOKENS estimated tokens (char-prefix via the
            // chars/4 estimator; char-boundary safe). The marker's own ~15 tokens
            // ride on top of the cap — bounded, and the cap is order-of-magnitude.
            let goal = if estimate_tokens(&goal) > GOAL_MAX_TOKENS {
                let kept: String = goal.chars().take(GOAL_MAX_TOKENS * 4).collect();
                format!("{kept}{GOAL_TRUNCATION_MARKER}")
            } else {
                goal
            };
            self.goal = Some(Message::system(format!("Original goal: {goal}")));
        }
    }

    fn build(&self, model_limit: usize) -> Vec<Message> {
        let pinned = self.pinned();
        let budget = model_limit.saturating_sub(self.pinned_tokens());
        // Priority retention over whole turn units: the in-flight newest unit,
        // then user instructions (durable facts), then the rest newest-first —
        // never splitting a tool_calls parent from its Role::Tool results.
        let mut out = pinned;
        for r in plan_retention(&self.history, budget) {
            out.extend(self.history[r].iter().cloned());
        }
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
            self.lift(hit, &mut report, deps, |vpath| {
                capped_preview(&content, cap, vpath, &tool)
            })
            .await;
        }

        // (a) Deterministic age-based offload — every turn.
        for hit in select_offloads(&self.history, &self.config) {
            let tool = hit.entry.tool_name.clone();
            let kind = hit.entry.kind.clone();
            let bytes = hit.entry.bytes;
            self.lift(hit, &mut report, deps, |vpath| {
                placeholder_for(vpath, &tool, &kind, bytes)
            })
            .await;
        }

        // (a') Extractive fold — when the retention plan would evict user
        // units (invisible loss, observed to invite confabulation), the oldest
        // are folded: facts extracted to the pinned ledger, verbatim originals
        // offloaded, units removed. All-or-nothing per batch.
        self.fold_evicted_users(deps, &mut report).await;

        // (b) Compaction — async, gated by the high-water mark or an explicit request.
        let requested = self.compact_flag.swap(false, Ordering::SeqCst);
        let over_high_water = {
            let built = self.build(deps.model_limit);
            let used: usize = built.iter().map(message_tokens).sum();
            (used as f32) > (deps.model_limit as f32 * self.high_water_pct)
        };
        if (requested || over_high_water) && self.history.len() > self.config.keep_recent + 1 {
            self.compact_old_span(deps, &mut report, requested).await;
        }

        // (c) Eviction visibility — runs on EVERY maintain exit.
        self.emit_eviction(deps);
        report
    }

    fn request_compaction(&mut self) {
        self.compact_flag.store(true, Ordering::SeqCst);
    }
}

/// Cap on the estimated size of a turn unit that qualifies as a durable
/// placeholder anchor; a unit dragged large by parent chatter is summarized
/// like any other (a parent with several parallel placeholders stays under this).
const PLACEHOLDER_UNIT_MAX_TOKENS: usize = 160;

/// Cap on the pinned folded-facts ledger. Past it the OLDEST lines drop —
/// their verbatim originals remain in the offload store, so a cap eviction is
/// strictly no worse than the silent full eviction it replaces.
const FOLDED_FACTS_MAX_TOKENS: usize = 512;

/// Token cap for the pinned goal block (audit 7.1). Same scale as
/// `DEFAULT_RECALL_TOKEN_BUDGET` and `FOLDED_FACTS_MAX_TOKENS` — every pinned
/// block is budgeted. The full input stays in history; only the pin truncates.
const GOAL_MAX_TOKENS: usize = 512;
const GOAL_TRUNCATION_MARKER: &str =
    "… [goal truncated; the full input remains in the conversation history]";

/// Fraction of the effective window that verbatim user units are folded DOWN
/// to once eviction becomes imminent. Folding to well below the trigger point
/// is hysteresis: one batch per overflow episode instead of per-turn churn,
/// with headroom for the next incoming prompt (curation only runs before the
/// prompt after next — see the 2026-07-02 ordering spec).
const USER_FOLD_LOW_WATERMARK_PCT: f32 = 0.25;

/// Minimum estimated size of a PURE-ASSISTANT chatter span before it is
/// worth a summarizer pass. Re-running the summarizer over `prior + one
/// trivial ack` is generation loss — a small model degrades the prior
/// instead of extending it — and even without a prior it wastes a model
/// call. Tool-bearing spans are exempt at ANY size: their per-turn cadence
/// is load-bearing (a flat floor here regressed locked-portmap).
const TRIVIAL_CHATTER_SPAN_TOKENS: usize = 256;

/// A turn unit whose tool results are all age-offload placeholders and whose
/// total estimated size stays anchor-small. Like user turns, these are durable:
/// each placeholder is the model's only pointer for `context_recall`ing the
/// offloaded content, so it must survive compaction verbatim.
fn is_durable_placeholder_unit(unit: &[Message]) -> bool {
    unit.len() >= 2
        && unit[1..].iter().all(|m| {
            matches!(m.role, Role::Tool) && crate::offload_policy::is_placeholder(&m.content)
        })
        && unit.iter().map(message_tokens).sum::<usize>() <= PLACEHOLDER_UNIT_MAX_TOKENS
}

impl CuratedContext {
    /// Write a hit's full content to the results store and replace the window
    /// copy with whatever `replacement` renders for the assigned virtual path.
    /// Shared by the ingestion-cap and age-based offload passes. On write
    /// failure it skips (message intact, retried next maintain — spec §5.5).
    async fn lift(
        &mut self,
        hit: crate::offload_policy::OffloadHit,
        report: &mut MaintReport,
        deps: &MaintCtx<'_>,
        replacement: impl FnOnce(&str) -> String,
    ) {
        let idx = hit.history_index;
        let tool = hit.entry.tool_name.clone();
        let bytes = hit.entry.bytes;
        let seq = self.next_seq();
        let key = format!(
            "{}{}-{}",
            self.artifact_prefix,
            seq,
            agent_tools::backend::sanitize_component(&hit.entry.tool_call_id)
        );
        let vpath = format!("large_tool_results/{key}");
        if let Err(e) = self.artifacts.results.write(&key, &hit.entry.content).await {
            tracing::warn!(error = %e, "artifact write failed; offload skipped this pass");
            return;
        }
        self.history[idx].content = replacement(&vpath);
        report.offloaded += 1;
        report.offloaded_bytes += bytes;
        deps.sink.emit(AgentEvent::Context(ContextEvent::Offloaded {
            path: vpath,
            bytes,
            tool,
        }));
    }

    /// Append a `## {section}` block to the history file (spec §5.5, E4).
    async fn append_history(
        &self,
        section: &str,
        body: &str,
    ) -> Result<(), agent_tools::backend::FsError> {
        use agent_tools::backend::FsError;
        let existing = match self.artifacts.history.read("history.md").await {
            Ok(s) => s,
            Err(FsError::NotFound(_)) => String::new(),
            Err(e) => return Err(e),
        };
        let updated = format!("{existing}\n\n## {section}\n\n{body}");
        self.artifacts
            .history
            .write("history.md", updated.trim_start())
            .await
    }

    /// The transcript pointer (spec §5.5): tracked as flags, NEVER stored in
    /// compaction_summary — the summarizer must never see or paraphrase it.
    fn history_pointer(&self) -> Option<String> {
        if !self.history_has_spans && !self.history_incomplete {
            return None;
        }
        let prefix = if self.history_incomplete {
            "Evicted transcripts (INCOMPLETE — at least one span failed to record)"
        } else {
            "Evicted transcripts"
        };
        Some(format!(
            "{prefix}: conversation_history/history.md — grep it for \"## folded-\" / \
             \"## compacted-\" section headers, then read_file from the hit's line offset."
        ))
    }

    /// Fold user units the retention plan is about to evict: extract their
    /// durable facts into the pinned ledger (one extraction model call per
    /// batch), offload the verbatim originals, remove the units from history.
    /// Oldest-first, down to `USER_FOLD_LOW_WATERMARK_PCT` of the window
    /// (hysteresis); the `keep_recent` tail is never touched. All-or-nothing:
    /// an extraction failure leaves history intact for the next maintain.
    async fn fold_evicted_users(&mut self, deps: &MaintCtx<'_>, report: &mut MaintReport) {
        let budget = deps.model_limit.saturating_sub(self.pinned_tokens());
        let mut kept = vec![false; self.history.len()];
        for r in plan_retention(&self.history, budget) {
            kept[r].iter_mut().for_each(|k| *k = true);
        }
        let user_eviction_imminent = self
            .history
            .iter()
            .enumerate()
            .any(|(i, m)| matches!(m.role, Role::User) && !kept[i]);
        if !user_eviction_imminent {
            return;
        }
        // Only the old span is foldable — the in-flight tail stays put.
        let split = snap_split_to_unit_boundary(
            &self.history,
            self.history.len().saturating_sub(self.config.keep_recent),
        );
        let units = turn_unit_ranges(&self.history[..split]);
        let is_user_unit = |r: &std::ops::Range<usize>| {
            self.history[r.clone()]
                .iter()
                .all(|m| matches!(m.role, Role::User))
        };
        let low_watermark = (deps.model_limit as f32 * USER_FOLD_LOW_WATERMARK_PCT) as usize;
        let mut fold: Vec<std::ops::Range<usize>> = Vec::new();
        let mut user_tokens = 0usize;
        let mut over = false;
        for r in units.iter().rev() {
            if !is_user_unit(r) {
                continue;
            }
            let t: usize = self.history[r.clone()].iter().map(message_tokens).sum();
            if over || user_tokens + t > low_watermark {
                over = true;
                fold.push(r.clone());
            } else {
                user_tokens += t;
            }
        }
        if fold.is_empty() {
            return;
        }
        fold.reverse(); // chronological order for extraction + storage
        let folded: Vec<Message> = fold
            .iter()
            .flat_map(|r| self.history[r.clone()].iter().cloned())
            .collect();
        let lines = match run_extraction(&folded, deps.model, deps.cancel).await {
            Ok(l) if !l.is_empty() => l,
            Ok(_) => {
                tracing::debug!("fold extraction produced no lines; skipped");
                return;
            }
            Err(e) => {
                tracing::warn!(error = %e, "fold extraction failed; leaving history intact");
                return;
            }
        };
        // Verbatim originals to the history file, one batch section. Write
        // FIRST, abort-on-failure (all-or-nothing keeps its exact shape, §5.5).
        let content = folded
            .iter()
            .map(|m| format!("[user] {}", m.content))
            .collect::<Vec<_>>()
            .join("\n");
        let bytes = content.len();
        let seq = self.next_seq();
        if let Err(e) = self.append_history(&format!("folded-{seq}"), &content).await {
            tracing::warn!(error = %e, "history write failed; leaving fold for the next maintain");
            return;
        }
        self.folded_sections.push(seq);
        self.folded_facts.extend(lines);
        // Cap the ledger, dropping OLDEST lines first (originals stay in the
        // store, so this is strictly no worse than silent eviction).
        while message_tokens(&self.folded_block()) > FOLDED_FACTS_MAX_TOKENS
            && self.folded_facts.len() > 1
        {
            self.folded_facts.remove(0);
        }
        // Remove the folded units from history (whole units, orphan-safe).
        let folded_idx: std::collections::HashSet<usize> =
            fold.iter().flat_map(|r| r.clone()).collect();
        let mut i = 0;
        self.history.retain(|_| {
            let keep = !folded_idx.contains(&i);
            i += 1;
            keep
        });
        report.offloaded += 1;
        report.offloaded_bytes += bytes;
        deps.sink.emit(AgentEvent::Context(ContextEvent::Offloaded {
            path: "conversation_history/history.md".into(),
            bytes,
            tool: "user_history".into(),
        }));
    }

    /// Compact the old span into the pinned summary. Extracted from `maintain`
    /// so its early exits cannot skip the eviction check that follows.
    /// `forced` marks an explicit `request_compaction()` (overflow recovery):
    /// it bypasses the trivial-chatter cadence skip — an imperative "shrink
    /// now" outranks a heuristic that only exists to batch routine passes.
    async fn compact_old_span(
        &mut self,
        deps: &MaintCtx<'_>,
        report: &mut MaintReport,
        forced: bool,
    ) {
        // Snap left to a unit boundary so the cut never separates a
        // tool_calls parent from its results; the torn turn lands wholly
        // in the recent window (keep_recent temporarily keeps a bit more).
        let split = snap_split_to_unit_boundary(
            &self.history,
            self.history.len() - self.config.keep_recent,
        );
        let prior = self.compaction_summary.clone();
        // Tool results leaving the live window through compaction are lifted
        // into the offload store FIRST — age protection (`keep_recent`) no
        // longer applies to a result that is leaving regardless. Without this,
        // a large fresh result can be destroyed by the summarizer before the
        // age-based pass ever placeholders it, severing the recall chain.
        let boundary_cfg = OffloadConfig {
            keep_recent: 0,
            ..self.config.clone()
        };
        for hit in select_offloads(&self.history[..split], &boundary_cfg) {
            let tool = hit.entry.tool_name.clone();
            let kind = hit.entry.kind.clone();
            let bytes = hit.entry.bytes;
            self.lift(hit, report, deps, |vpath| {
                placeholder_for(vpath, &tool, &kind, bytes)
            })
            .await;
        }
        // Durable anchors never enter the lossy summarizer: user turns (the
        // author-authored facts most costly to lose) and offload-placeholder
        // units (the model's only pointers for recalling offloaded content —
        // paraphrasing one away severs the recall chain). Both stay VERBATIM in
        // history; only real assistant/tool chatter in the old span is compacted,
        // with the prior summary carried forward so re-compaction accumulates
        // instead of collapsing to the most recent turn. Partitioned in whole
        // turn units so a kept placeholder never orphans its tool_calls parent.
        let mut verbatim: Vec<Message> = Vec::new();
        let mut to_summarize: Vec<Message> = Vec::new();
        for r in turn_unit_ranges(&self.history[..split]) {
            let unit = &self.history[r];
            let durable = unit.iter().all(|m| matches!(m.role, Role::User))
                || is_durable_placeholder_unit(unit);
            if durable {
                verbatim.extend(unit.iter().cloned());
            } else {
                to_summarize.extend(unit.iter().cloned());
            }
        }
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
        // Degenerate span: pure assistant chatter too small to be worth a
        // summarizer pass (see TRIVIAL_CHATTER_SPAN_TOKENS). The chatter
        // accumulates until the span is substantial or gains a tool-bearing
        // unit; history and the prior summary stay untouched. An explicit
        // compaction request (overflow recovery) is exempt.
        let span_tokens: usize = to_summarize.iter().map(message_tokens).sum();
        let all_assistant = to_summarize
            .iter()
            .all(|m| matches!(m.role, Role::Assistant));
        if !forced && all_assistant && span_tokens < TRIVIAL_CHATTER_SPAN_TOKENS {
            return;
        }
        match run_compaction(&to_summarize, prior.as_ref(), deps.model, deps.cancel).await {
            Ok(summary) if compaction_is_worthwhile(&summary, &replaced) => {
                let tokens_after = message_tokens(&summary);
                // Monotone prior guard: the compaction prompt mandates a
                // strict superset of the prior summary, so a candidate
                // smaller than the prior it replaces is a degraded pass —
                // the collapse mechanism under repeated re-compaction. Keep
                // the prior; the span stays in history and is retried once
                // it has grown. (`compaction_is_worthwhile` cannot catch
                // this: a collapsed summary looks like a huge token win.)
                if let Some(p) = prior.as_ref() {
                    if tokens_after < message_tokens(p) {
                        tracing::debug!("compaction shrank the prior summary; discarded");
                        return;
                    }
                }
                // Reassemble history: verbatim user instructions (chronological), then
                // the recent window. The summarized chatter becomes the pinned summary.
                let recent = self.history.split_off(split);
                self.history = verbatim;
                self.history.extend(recent);
                self.compaction_summary = Some(summary);
                report.compacted_turns = to_summarize.len();
                // Record the summarized span verbatim to the history file so the
                // pinned summary can point back at it (spec §5.5, E4).
                let seq = self.next_seq();
                let rendered: String = to_summarize
                    .iter()
                    .map(|m| {
                        let role = match m.role {
                            Role::System => "system",
                            Role::User => "user",
                            Role::Assistant => "assistant",
                            Role::Tool => "tool",
                        };
                        format!("[{role}] {}\n", m.content)
                    })
                    .collect();
                match self.append_history(&format!("compacted-{seq}"), &rendered).await {
                    Ok(()) => self.history_has_spans = true,
                    Err(e) => {
                        // E4 gate decision: commit anyway; the pointer goes
                        // permanently honest-incomplete.
                        tracing::warn!(error = %e, "history write failed; compaction commits without this span");
                        self.history_incomplete = true;
                    }
                }
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
        // Mirror build()'s priority retention: evicted = the plan's complement
        // (no longer a contiguous oldest prefix — user units are kept out of order).
        let mut kept = vec![false; self.history.len()];
        for r in plan_retention(&self.history, budget) {
            kept[r].iter_mut().for_each(|k| *k = true);
        }
        let evicted: Vec<usize> = (0..self.history.len()).filter(|&i| !kept[i]).collect();
        let est_tokens: usize = evicted
            .iter()
            .map(|&i| message_tokens(&self.history[i]))
            .sum();
        // Dedup on (count, tokens): re-emit when EITHER changes, so an offload
        // that shrinks the evicted span's tokens without moving the count still
        // surfaces the reduced pressure.
        let key = (evicted.len(), est_tokens);
        if !evicted.is_empty() && key != self.last_evicted {
            deps.sink.emit(AgentEvent::Context(ContextEvent::Evicted {
                messages: evicted.len(),
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
    use crate::testkit::{CollectingSink, Scripted, ScriptedModel};
    use agent_model::{ModelClient, Role};
    use tokio_util::sync::CancellationToken;

    fn ctx() -> CuratedContext {
        CuratedContext::new(
            Message::system("SYS"),
            Arc::new(crate::SessionArtifacts::new()),
            Arc::new(AtomicBool::new(false)),
        )
    }

    /// Count of keys in the results store (successor of `store.len()`).
    async fn results_count(c: &CuratedContext) -> usize {
        c.artifacts.results.ls("").await.unwrap().len()
    }

    #[test]
    fn curated_context_system_getter_returns_the_system_message() {
        let c = ctx();
        let sys = c.system().expect("CuratedContext always holds a system");
        assert_eq!(sys.content, "SYS");
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
    fn set_goal_caps_oversized_input_with_marker() {
        let mut c = ctx(); // match set_goal_is_set_once's constructor
        let big = "word ".repeat(3000); // ~3750 est. tokens, well over the 512 cap
        c.set_goal(big);
        let g = c.goal.clone().expect("goal set");
        assert!(g.content.contains("[goal truncated"), "marker missing");
        // cap + marker + "Original goal: " prefix, small slack for overheads
        assert!(
            message_tokens(&g) <= GOAL_MAX_TOKENS + 40,
            "goal block too big: {}",
            message_tokens(&g)
        );
    }

    #[test]
    fn set_goal_under_cap_is_untouched() {
        let mut c = ctx();
        c.set_goal("ship the feature".into());
        let g = c.goal.clone().unwrap();
        assert_eq!(g.content, "Original goal: ship the feature");
        assert!(!g.content.contains("[goal truncated"));
    }

    #[test]
    fn oversized_first_paste_does_not_wedge_the_window() {
        // The audit-7.1 property: an over-window first paste must not make
        // pinned_tokens() exceed the window forever (budget saturates to 0,
        // second overflow is fatal → permanently wedged session).
        let mut c = ctx();
        let big = "x".repeat(400_000); // ~100k est. tokens >> the 8192 window below
        c.set_goal(big.clone());
        c.append(Message::user(big));
        let model_limit = 8192;
        assert!(
            c.pinned_tokens() < model_limit / 2,
            "pinned blocks must leave a real history budget, got {}",
            c.pinned_tokens()
        );
        let msgs = c.build(model_limit);
        assert!(!msgs.is_empty(), "build must still produce a request");
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
        assert_eq!(results_count(&c).await, 1);
        // Live message is now a placeholder; full content recoverable from the store.
        let built = c.build(100_000);
        let tool_msg = built.iter().find(|m| matches!(m.role, Role::Tool)).unwrap();
        assert!(tool_msg
            .content
            .starts_with("[tool_result offloaded to large_tool_results/1-"));
        assert_eq!(
            c.artifacts.results.read("1-call-1").await.unwrap(),
            big_err
        );
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
        assert_eq!(results_count(&c).await, 1);
    }

    #[tokio::test]
    async fn maintain_compacts_old_span_when_over_high_water() {
        let mut c = ctx();
        c.high_water_pct = 0.0; // force compaction regardless of size
        c.config.keep_recent = 1;
        for i in 0..6 {
            // Assistant chatter (not user instructions) is what gets summarized.
            c.append(Message::assistant(
                format!(
                    "turn {i} {}",
                    "with a fair bit of padding text here ".repeat(12)
                ),
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
                format!(
                    "ok, acknowledged {i}, {}",
                    "lots of filler chatter ".repeat(15)
                ),
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

    #[tokio::test]
    async fn fold_extracts_evicted_users_to_pinned_ledger() {
        // When user units overflow the retention budget, the OLDEST are folded:
        // their durable facts extracted into a pinned ledger block (compact,
        // always visible — pinned DATA is used even where pinned markers fail
        // to elicit actions), the verbatim originals offloaded, the units
        // removed from history. The newest user units stay verbatim.
        let mut c = ctx();
        c.config.keep_recent = 1;
        for i in 0..12 {
            c.append(Message::user(format!(
                "ledger entry number {i}: setting item_{i} is assigned value {i}{i}{i}{i} for the manifest"
            )));
        }
        c.append(Message::assistant("working on it", None));
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "item_0 = 0000\nitem_1 = 1111\nitem_2 = 2222".into(),
        )]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let mut deps = maint_deps(&model, &sink, &cancel);
        deps.model_limit = 250; // user turns alone exceed the budget
        let report = c.maintain(&deps).await;

        assert!(report.offloaded >= 1, "fold must commit");
        let built = c.build(100_000);
        let block = built
            .iter()
            .find(|m| m.content.starts_with("Ledger of earlier user instructions"))
            .expect("pinned ledger block present");
        assert!(matches!(block.role, Role::System));
        assert!(block.content.contains("item_0 = 0000"));
        assert!(
            block.content.contains("conversation_history/history.md"),
            "originals advertised: {}",
            block.content
        );
        assert!(
            block.content.contains("## folded-1"),
            "ledger cites its history section: {}",
            block.content
        );
        // Verbatim originals recoverable from the history file.
        assert!(c
            .artifacts
            .history
            .read("history.md")
            .await
            .unwrap()
            .contains("entry number 0"));
        // Oldest user turns left history; the newest survive verbatim.
        assert!(
            !c.history()
                .iter()
                .any(|m| m.content.contains("entry number 0:")),
            "folded units must leave history"
        );
        assert!(
            c.history()
                .iter()
                .any(|m| m.content.contains("entry number 11")),
            "newest user units stay verbatim"
        );
    }

    #[tokio::test]
    async fn ledger_rides_inside_the_goal_block() {
        // The goal block is the one pinned region with demonstrated per-run
        // attention; the ledger merges into it rather than standing alone.
        let mut c = ctx();
        c.set_goal("assemble the manifest".into());
        c.folded_facts = vec!["alpha_timeout = 4831".into()];
        let built = c.build(100_000);
        let goal = built
            .iter()
            .find(|m| m.content.starts_with("Original goal:"))
            .expect("goal block present");
        assert!(
            goal.content.contains("1. alpha_timeout = 4831"),
            "ledger numbered lines live inside the goal block: {}",
            goal.content
        );
        assert!(
            !built
                .iter()
                .any(|m| m.content.starts_with("Ledger of earlier user instructions")),
            "no standalone ledger block when a goal exists"
        );
    }

    #[test]
    fn snapshot_est_total_includes_the_pinned_ledger() {
        // Non-empty folded_facts, same setup as ledger_rides_inside_the_goal_block.
        let mut c = ctx();
        c.folded_facts = vec!["alpha_timeout = 4831".into(), "bravo_retries = 7207".into()];
        c.append(Message::user("do the thing"));
        c.append(Message::assistant("on it", None));
        let snap = c.snapshot(100_000, 1);
        let history_tokens: usize = c.history().iter().map(message_tokens).sum();
        assert_eq!(snap.est_total, c.pinned_tokens() + history_tokens);
        let ledger = snap
            .segments
            .iter()
            .find(|s| s.category == "ledger")
            .expect("ledger segment present");
        assert_eq!(ledger.count, c.folded_facts.len());
    }

    #[tokio::test]
    async fn fold_is_noop_when_users_fit() {
        let mut c = ctx();
        c.config.keep_recent = 1;
        for i in 0..4 {
            c.append(Message::user(format!("instruction {i}")));
        }
        c.append(Message::assistant("ok", None));
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        // Roomy window: no eviction, no fold, no ledger.
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        assert_eq!(report.offloaded, 0);
        assert_eq!(results_count(&c).await, 0);
        assert!(!c
            .build(100_000)
            .iter()
            .any(|m| m.content.starts_with("Ledger of earlier user instructions")));
        assert_eq!(
            c.history()
                .iter()
                .filter(|m| matches!(m.role, Role::User))
                .count(),
            4
        );
    }

    #[tokio::test]
    async fn fold_extraction_failure_leaves_history_intact() {
        // All-or-nothing: if the extraction call fails, nothing is folded —
        // the user units stay in history and are retried at the next maintain.
        let mut c = ctx();
        c.config.keep_recent = 1;
        for i in 0..12 {
            c.append(Message::user(format!(
                "ledger entry number {i}: setting item_{i} is assigned value {i}{i}{i}{i} for the manifest"
            )));
        }
        c.append(Message::assistant("working on it", None));
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![Scripted::Error]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let mut deps = maint_deps(&model, &sink, &cancel);
        deps.model_limit = 250;
        c.maintain(&deps).await;
        assert!(
            matches!(
                c.artifacts.history.read("history.md").await,
                Err(agent_tools::backend::FsError::NotFound(_))
            ),
            "nothing written on failure"
        );
        assert_eq!(
            c.history()
                .iter()
                .filter(|m| matches!(m.role, Role::User))
                .count(),
            12,
            "history untouched on extraction failure"
        );
        assert!(!c
            .build(100_000)
            .iter()
            .any(|m| m.content.starts_with("Ledger of earlier user instructions")));
    }

    #[tokio::test]
    async fn ledger_is_capped_dropping_oldest_lines() {
        // The pinned ledger never grows unboundedly: past the cap the OLDEST
        // lines drop (their verbatim originals remain in the offload store —
        // strictly no worse than today's silent eviction).
        let mut c = ctx();
        c.config.keep_recent = 1;
        c.folded_facts = (0..300).map(|i| format!("old_fact_{i} = {i}")).collect();
        for i in 0..12 {
            c.append(Message::user(format!(
                "ledger entry number {i}: setting item_{i} is assigned value {i}{i}{i}{i} for the manifest"
            )));
        }
        c.append(Message::assistant("working on it", None));
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "brand_new_fact = 42".into(),
        )]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let mut deps = maint_deps(&model, &sink, &cancel);
        deps.model_limit = 250;
        c.maintain(&deps).await;
        let block = c
            .build(100_000)
            .into_iter()
            .find(|m| m.content.starts_with("Ledger of earlier user instructions"))
            .expect("ledger present");
        assert!(block.content.contains("brand_new_fact = 42"), "newest kept");
        assert!(!block.content.contains("old_fact_0 = 0"), "oldest dropped");
        assert!(
            message_tokens(&block) <= FOLDED_FACTS_MAX_TOKENS,
            "ledger stays under its cap: {} tok",
            message_tokens(&block)
        );
    }

    #[tokio::test]
    async fn ledger_survives_compaction_untouched() {
        // The ledger is pinned — the summarizer never sees it, so compaction
        // cannot paraphrase it away (append-only, no generation loss).
        let mut c = ctx();
        c.high_water_pct = 0.0; // force compaction
        c.config.keep_recent = 1;
        c.folded_facts = vec!["alpha_timeout = 4831".into(), "bravo_retries = 7207".into()];
        c.append(parent("c1"));
        c.append(Message::tool(
            "c1",
            "shell",
            "a fairly long tool result with plenty of words to summarize away here",
        ));
        c.append(Message::assistant("done", None));
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "Summary:\nshell call happened and returned data of interest".into(),
        )]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        assert!(report.compacted_turns > 0, "compaction fired");
        let block = c
            .build(100_000)
            .into_iter()
            .find(|m| m.content.starts_with("Ledger of earlier user instructions"))
            .expect("ledger still pinned");
        assert!(block.content.contains("alpha_timeout = 4831"));
        assert!(block.content.contains("bravo_retries = 7207"));
    }

    #[tokio::test]
    async fn trivial_assistant_chatter_skips_the_summarizer() {
        // A prior summary plus a handful of tiny acks must NOT re-run the
        // summarizer — each pass over `prior + trivia` risks degrading the
        // prior (observed collapsing the running summary to "No new
        // information provided" under per-turn maintenance). The chatter
        // simply accumulates until the span is substantial.
        let mut c = ctx();
        c.high_water_pct = 0.0; // pressure permanently on
        c.config.keep_recent = 1;
        c.compaction_summary = Some(Message::system(
            "Summary of earlier conversation:\nledger entries 1-12 recorded",
        ));
        for i in 0..4 {
            c.append(Message::assistant(format!("ok {i}"), None));
        }
        c.append(Message::user("next instruction"));
        let model: Arc<dyn ModelClient> =
            Arc::new(ScriptedModel::new(vec![Scripted::Text("DEGRADED".into())]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        assert_eq!(report.compacted_turns, 0, "trivial span must not compact");
        assert!(
            c.compaction_summary
                .as_ref()
                .unwrap()
                .content
                .contains("entries 1-12 recorded"),
            "prior summary untouched"
        );
        // The acks stay in history, awaiting a substantial span.
        assert!(c.history().iter().any(|m| m.content == "ok 0"));
    }

    #[tokio::test]
    async fn shrinking_summary_is_rejected_keeping_prior() {
        // The compaction prompt mandates a strict superset of the prior; a
        // candidate SMALLER than the prior is by definition a degraded pass
        // (the collapse mechanism under repeated re-compaction) and must be
        // discarded — prior kept, span left in history for a later pass.
        let mut c = ctx();
        c.high_water_pct = 0.0; // force compaction
        c.config.keep_recent = 1;
        let fat_prior = format!(
            "Summary of earlier conversation:\n{}",
            "ledger entry detail ".repeat(30)
        );
        c.compaction_summary = Some(Message::system(fat_prior.clone()));
        c.append(parent("c1"));
        c.append(Message::tool("c1", "shell", "output ".repeat(60)));
        c.append(Message::assistant("done", None));
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "No new information provided".into(),
        )]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        assert_eq!(
            report.compacted_turns, 0,
            "shrinking candidate must be discarded"
        );
        assert_eq!(c.compaction_summary.as_ref().unwrap().content, fat_prior);
        // The span stays in history for a later, larger pass.
        assert!(c.history().iter().any(|m| matches!(m.role, Role::Tool)));
    }

    #[tokio::test]
    async fn explicit_request_bypasses_the_trivial_chatter_skip() {
        // request_compaction() is the overflow-recovery imperative — "shrink
        // now". The trivial-chatter skip is a cadence heuristic for routine
        // high-water passes and must not veto an explicit request.
        let mut c = ctx();
        c.high_water_pct = 2.0; // size trigger disabled; only the flag fires
        c.config.keep_recent = 1;
        for i in 0..4 {
            c.append(Message::assistant(format!("ok {i} noted, thanks"), None));
        }
        c.append(Message::user("next instruction"));
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "compact summary".into(),
        )]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        c.request_compaction();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        assert!(
            report.compacted_turns > 0,
            "an explicit request must compact even a trivial span"
        );
    }

    #[tokio::test]
    async fn tiny_tool_bearing_span_still_compacts() {
        // The degenerate-span skip must NOT throttle tool-bearing spans: a
        // flat recompaction floor did exactly that and regressed
        // locked-portmap 10/10 -> ~4/6 (delayed compaction left no single
        // complete source at write time). Tool-bearing spans of any size
        // keep the per-turn cadence the eval ceilings were measured under.
        let mut c = ctx();
        c.high_water_pct = 0.0; // force compaction
        c.config.keep_recent = 1;
        c.compaction_summary = Some(Message::system("Summary:\nbase"));
        c.append(parent("c1"));
        c.append(Message::tool(
            "c1",
            "shell",
            "a short tool result with a few extra words of output here",
        ));
        c.append(Message::assistant("done", None));
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "Summary:\nbase plus the shell call output".into(),
        )]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        assert!(
            report.compacted_turns > 0,
            "tool-bearing spans keep the per-turn cadence"
        );
    }

    #[tokio::test]
    async fn compaction_offloads_departing_tool_results_before_summarizing() {
        // A large tool result still inside the age-protection window when
        // compaction fires must NOT be destroyed by the summarizer: it is
        // lifted to the offload store at the compaction boundary and its
        // placeholder survives verbatim as a recall pointer.
        let mut c = ctx();
        c.high_water_pct = 0.0; // force compaction
        c.config.keep_recent = 2; // age pass alone would protect the result
        let secret = format!("the secret is QH7-ZEBRA {}", "x".repeat(1500));
        c.append(parent("c1"));
        c.append(Message::tool("c1", "read_file", secret.clone()));
        for i in 0..4 {
            c.append(Message::assistant(
                format!("chatter {i} padding text"),
                None,
            ));
        }
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "compact summary".into(),
        )]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;

        assert!(report.offloaded >= 1, "departing result must be offloaded");
        assert_eq!(
            c.artifacts.results.read("1-c1").await.unwrap(),
            secret,
            "full content recoverable from the store"
        );
        let built = c.build(100_000);
        assert!(
            built
                .iter()
                .any(|m| m.content.starts_with("[tool_result offloaded to large_tool_results/1-")),
            "placeholder survives compaction as a durable anchor: {:?}",
            built.iter().map(|m| &m.content).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn maintain_keeps_offload_placeholders_verbatim_through_compaction() {
        // An age-offloaded placeholder is the model's only recall pointer; the
        // summarizer must never paraphrase it away. Real chatter still compacts.
        let mut c = ctx();
        c.high_water_pct = 0.0; // force compaction
        c.config.keep_recent = 1;
        let ph = crate::offload_policy::placeholder_for(
            "large_tool_results/7-c1",
            "read_file",
            &crate::offload_policy::OffloadKind::Output,
            5000,
        );
        c.append(parent("c1"));
        c.append(Message::tool("c1", "read_file", ph.clone()));
        for i in 0..3 {
            c.append(Message::assistant(
                format!(
                    "chatter {i} {}",
                    "with plenty of padding text to summarize ".repeat(12)
                ),
                None,
            ));
        }
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "chatter summary".into(),
        )]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;

        assert!(report.compacted_turns > 0, "chatter should compact");
        let built = c.build(100_000);
        assert!(
            built.iter().any(|m| m.content == ph),
            "placeholder unit must survive compaction verbatim"
        );
        assert!(
            built.iter().any(|m| m.content.contains("chatter summary")),
            "non-placeholder chatter still summarized"
        );
        use crate::context::orphaned_tool_positions;
        assert!(orphaned_tool_positions(&built).is_empty());
    }

    #[test]
    fn build_keeps_user_instructions_under_tight_budget() {
        // The durable-user-turn contract must hold in build() itself, not just
        // through compaction: when the budget can't hold everything, user
        // instructions outrank newer assistant/tool chatter.
        let mut c = ctx();
        c.append(Message::user("the service auth listens on port 8401"));
        c.append(parent("c1"));
        c.append(Message::tool("c1", "shell", "n".repeat(600)));
        c.append(Message::user("the service cache listens on port 9213"));
        c.append(parent("c2"));
        c.append(Message::tool("c2", "shell", "n".repeat(600)));
        c.append(Message::user("now implement port_for"));
        use crate::context::message_tokens;
        let sys = message_tokens(&Message::system("SYS"));
        let users: usize = c
            .history()
            .iter()
            .filter(|m| matches!(m.role, Role::User))
            .map(message_tokens)
            .sum();
        // Window fits pinned + the user turns + slack, but not a 600-byte tool unit.
        let built = c.build(sys + users + 20);
        for want in [
            "the service auth listens on port 8401",
            "the service cache listens on port 9213",
            "now implement port_for",
        ] {
            assert!(
                built.iter().any(|m| m.content == want),
                "user turn {want:?} must survive a tight build budget"
            );
        }
        assert!(
            !built.iter().any(|m| matches!(m.role, Role::Tool)),
            "big chatter units should be the ones evicted"
        );
    }

    #[test]
    fn curated_snapshot_reports_system_recall_and_messages() {
        let mut c = CuratedContext::new(
            Message::system("SYS"),
            Arc::new(crate::SessionArtifacts::new()),
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
        use crate::context::evict_start;
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
        let artifacts = Arc::new(crate::SessionArtifacts::new());
        let flag = Arc::new(AtomicBool::new(false));
        let mut ctx = CuratedContext::new(Message::system("s"), artifacts.clone(), flag)
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
        let stored = artifacts.results.read("1-c1").await.expect("entry stored");
        assert_eq!(stored.len(), 50_000);
        // Offloaded event emitted.
        assert!(
            sink.events
                .lock()
                .unwrap()
                .iter()
                .any(|e| e == "offloaded:large_tool_results/1-c1"),
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
        let artifacts = Arc::new(crate::SessionArtifacts::new());
        let flag = Arc::new(AtomicBool::new(false));
        let mut ctx = CuratedContext::new(Message::system("s"), artifacts, flag)
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
        let artifacts = Arc::new(crate::SessionArtifacts::new());
        let flag = Arc::new(AtomicBool::new(false));
        let mut ctx = CuratedContext::new(Message::system("s"), artifacts.clone(), flag)
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
        assert!(msg
            .content
            .starts_with("[tool_result offloaded to large_tool_results/2-c1"));
        // Entry #2 is the capped preview, whose marker still points at #1.
        assert!(artifacts
            .results
            .read("2-c1")
            .await
            .unwrap()
            .contains("large_tool_results/1-c1"));
        assert_eq!(
            artifacts.results.read("1-c1").await.unwrap().len(),
            50_000
        );
    }

    #[tokio::test]
    async fn oversized_error_result_is_capped_too() {
        let artifacts = Arc::new(crate::SessionArtifacts::new());
        let flag = Arc::new(AtomicBool::new(false));
        let mut ctx = CuratedContext::new(Message::system("s"), artifacts.clone(), flag)
            .with_offload_config(OffloadConfig {
                max_result_bytes: 1024,
                ..Default::default()
            });
        let err_body = format!("ERROR: {}", "e".repeat(50_000));
        ctx.append(parent("c1")); // tool results always follow a tool_calls parent
        ctx.append(Message::tool("c1", "shell", err_body.clone()));

        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![]));
        let sink_dyn: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let mut deps = maint_deps(&model, &sink_dyn, &cancel);
        deps.model_limit = 1_000_000;

        ctx.maintain(&deps).await;

        assert!(ctx.history()[1].content.len() <= 1024);
        // The oversized ERROR body was capped and its full content preserved.
        assert_eq!(artifacts.results.read("1-c1").await.unwrap(), err_body);
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
                format!(
                    "turn {i} {}",
                    "with a fair bit of padding text here ".repeat(12)
                ),
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
