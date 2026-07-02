use crate::EventSink;
use agent_model::{Message, ModelClient, Role};
use async_trait::async_trait;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Cheap, tokenizer-agnostic estimate (~4 chars/token). Swap for a real
/// tokenizer later behind the same call site.
pub fn estimate_tokens(s: &str) -> usize {
    (s.chars().count() / 4).max(1)
}

pub(crate) fn message_tokens(m: &Message) -> usize {
    let mut t = estimate_tokens(&m.content) + 4; // per-message overhead
    if let Some(r) = &m.reasoning {
        t += estimate_tokens(r);
    }
    if let Some(calls) = &m.tool_calls {
        for c in calls {
            t += estimate_tokens(&c.name) + estimate_tokens(&c.args.to_string());
        }
    }
    t
}

/// Total estimated tokens for a built context (system + kept history),
/// using the same per-message estimate the window manager evicts against.
pub fn built_tokens(messages: &[Message]) -> usize {
    messages.iter().map(message_tokens).sum()
}

/// Chronological turn-unit grouping: a message with non-empty `tool_calls`
/// absorbs the consecutive `Role::Tool` messages that follow it; every other
/// message is a singleton unit. Curation (eviction, compaction splits) must
/// keep or drop a unit whole — a `Role::Tool` result serialized without its
/// parent `tool_calls` message 400s on OpenAI-compatible servers.
pub(crate) fn turn_unit_ranges(history: &[Message]) -> Vec<std::ops::Range<usize>> {
    let mut units = Vec::new();
    let mut i = 0;
    while i < history.len() {
        let start = i;
        let is_parent = history[i]
            .tool_calls
            .as_ref()
            .is_some_and(|c| !c.is_empty());
        i += 1;
        if is_parent {
            while i < history.len() && matches!(history[i].role, Role::Tool) {
                i += 1;
            }
        }
        units.push(start..i);
    }
    units
}

/// Index into `history` where the kept window begins for `budget`: walk turn
/// units newest-first, keep whole units while they fit, always keeping at
/// least the newest unit (even if it alone exceeds budget — the keep-≥1
/// floor, unit-shaped).
pub(crate) fn evict_start(history: &[Message], budget: usize) -> usize {
    let units = turn_unit_ranges(history);
    let mut start = history.len();
    let mut used = 0usize;
    for r in units.iter().rev() {
        let t: usize = history[r.clone()].iter().map(message_tokens).sum();
        if used + t > budget && start < history.len() {
            break;
        }
        used += t;
        start = r.start;
    }
    start
}

/// Largest unit boundary `<= split`. Snapping only moves left (keeps more in
/// the recent window), never right.
#[allow(dead_code)] // wired in by the next task
pub(crate) fn snap_split_to_unit_boundary(history: &[Message], split: usize) -> usize {
    let mut boundary = 0;
    for r in turn_unit_ranges(history) {
        if r.end <= split {
            boundary = r.end;
        } else {
            break;
        }
    }
    boundary
}

/// Positions of `Role::Tool` messages whose `tool_call_id` is not covered by
/// the nearest preceding assistant `tool_calls` block with only `Role::Tool`
/// messages in between — the exact shape OpenAI-compatible servers reject.
pub(crate) fn orphaned_tool_positions(messages: &[Message]) -> Vec<usize> {
    let mut orphans = Vec::new();
    let mut live_ids: std::collections::HashSet<&str> = Default::default();
    for (i, m) in messages.iter().enumerate() {
        if matches!(m.role, Role::Tool) {
            match m.tool_call_id.as_deref() {
                Some(id) if live_ids.contains(id) => {}
                _ => orphans.push(i),
            }
        } else {
            live_ids.clear();
            if let Some(calls) = &m.tool_calls {
                live_ids.extend(calls.iter().map(|c| c.id.as_str()));
            }
        }
    }
    orphans
}

/// Default cap on the auto-retrieval recall block, in estimated tokens. Keeps
/// recall from crowding out conversation history. Override per-context with
/// `WindowContext::with_recall_budget`.
pub const DEFAULT_RECALL_TOKEN_BUDGET: usize = 512;

/// Build a capped recall/notes block: greedily keep lines under `budget` tokens,
/// always including at least the first line if any are present (soft cap).
/// Shared by `WindowContext` and `CuratedContext`.
pub(crate) fn recall_block(lines: &[String], budget: usize) -> Option<Message> {
    if lines.is_empty() {
        return None;
    }
    const HEADER: &str = "Relevant memories from past sessions:";
    let mut body = String::from(HEADER);
    for line in lines {
        let candidate = format!("{body}\n- {line}");
        if estimate_tokens(&candidate) > budget && body != HEADER {
            break;
        }
        body = candidate;
    }
    Some(Message::system(body))
}

/// What one `maintain` pass did, for telemetry/tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaintReport {
    pub offloaded: usize,
    pub offloaded_bytes: usize,
    pub compacted_turns: usize,
}

/// Dependencies a context manager needs to run maintenance (compaction needs a
/// model; offload does not). Borrowed for the duration of the call.
pub struct MaintCtx<'a> {
    pub model_limit: usize,
    pub model: &'a Arc<dyn ModelClient>,
    pub sink: &'a Arc<dyn EventSink>,
    pub cancel: &'a CancellationToken,
}

#[async_trait]
pub trait ContextManager: Send + Sync {
    fn append(&mut self, msg: Message);
    fn build(&self, model_limit: usize) -> Vec<Message>;
    fn set_system(&mut self, system: Message);
    /// Replace the auto-retrieved recall lines surfaced this turn. Default no-op
    /// so non-memory implementations are unaffected.
    fn set_recall(&mut self, _items: Vec<String>) {}
    /// Record the original goal for re-grounding. Default no-op; set-once impls.
    fn set_goal(&mut self, _goal: String) {}
    /// Best-effort per-turn curation (offload + compaction). Default no-op so
    /// `WindowContext` and other simple impls are unaffected.
    async fn maintain(&mut self, _deps: &MaintCtx<'_>) -> MaintReport {
        MaintReport::default()
    }
}

/// Sliding-window context: always keeps the system prompt; evicts oldest
/// history turns until the estimate fits `model_limit`. An optional recall block
/// (auto-retrieved memories) sits right after the system prompt, capped at
/// `recall_budget` tokens so it can never starve history.
pub struct WindowContext {
    system: Message,
    history: Vec<Message>,
    recall: Vec<String>,
    recall_budget: usize,
}

impl WindowContext {
    pub fn new(system: Message) -> Self {
        Self {
            system,
            history: Vec::new(),
            recall: Vec::new(),
            recall_budget: DEFAULT_RECALL_TOKEN_BUDGET,
        }
    }

    /// Override the recall-block token cap (default `DEFAULT_RECALL_TOKEN_BUDGET`).
    pub fn with_recall_budget(mut self, budget: usize) -> Self {
        self.recall_budget = budget;
        self
    }

    /// Build the recall block message, greedily keeping lines under `recall_budget`.
    /// Always includes at least the first line if any are present (soft cap).
    fn recall_message(&self) -> Option<Message> {
        recall_block(&self.recall, self.recall_budget)
    }
}

impl ContextManager for WindowContext {
    fn append(&mut self, msg: Message) {
        self.history.push(msg);
    }

    fn set_system(&mut self, system: Message) {
        self.system = system;
    }

    fn set_recall(&mut self, items: Vec<String>) {
        self.recall = items;
    }

    fn build(&self, model_limit: usize) -> Vec<Message> {
        let sys_tokens = message_tokens(&self.system);
        let recall_msg = self.recall_message();
        let recall_tokens = recall_msg.as_ref().map(message_tokens).unwrap_or(0);
        let budget = model_limit
            .saturating_sub(sys_tokens)
            .saturating_sub(recall_tokens);
        // Walk history newest-first in turn units, keep whole units while they
        // fit — never split a tool_calls parent from its Role::Tool results.
        let start = evict_start(&self.history, budget);
        let mut out = Vec::with_capacity(self.history.len() - start + 2);
        out.push(self.system.clone());
        if let Some(m) = recall_msg {
            out.push(m);
        }
        out.extend(self.history[start..].iter().cloned());
        debug_assert!(
            orphaned_tool_positions(&out).is_empty(),
            "WindowContext::build produced an orphaned tool message"
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_model::{Message, Role};

    #[test]
    fn built_tokens_sums_per_message_estimate() {
        let msgs = vec![Message::system("SYS"), Message::user("hello world")];
        let expected = message_tokens(&msgs[0]) + message_tokens(&msgs[1]);
        assert_eq!(built_tokens(&msgs), expected);
    }

    #[test]
    fn message_tokens_counts_tool_calls_and_reasoning() {
        let plain = Message::assistant("hi", None);
        let heavy = Message::assistant(
            "hi",
            Some(vec![agent_tools::ToolCall {
                id: "c1".into(),
                name: "read_file".into(),
                args: serde_json::json!({"path": "some/long/path/to/a/file/name.txt"}),
            }]),
        )
        .with_reasoning("a fairly long chain of reasoning that should add tokens");
        assert!(
            message_tokens(&heavy) > message_tokens(&plain),
            "tool_calls + reasoning must increase the estimate"
        );
    }

    #[test]
    fn estimate_tokens_is_roughly_quarter_of_chars() {
        assert!(estimate_tokens("abcd") >= 1);
        assert!(estimate_tokens(&"x".repeat(400)) >= 90);
    }

    #[test]
    fn build_keeps_system_and_drops_oldest_when_over_limit() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        for i in 0..50 {
            ctx.append(Message::user(format!(
                "message number {i} with some padding text"
            )));
        }
        // Tiny limit forces eviction.
        let built = ctx.build(40);
        assert!(matches!(built[0].role, Role::System)); // system always first
        assert!(built.len() < 51); // some history evicted
                                   // The most recent user message survives.
        let last = built.last().unwrap();
        assert!(last.content.contains("49"));
    }

    #[test]
    fn build_returns_all_when_under_limit() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        ctx.append(Message::user("hello"));
        let built = ctx.build(100_000);
        assert_eq!(built.len(), 2);
    }

    #[test]
    fn set_recall_injects_block_after_system_before_history() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        ctx.append(Message::user("hello"));
        ctx.set_recall(vec!["user likes rust".into(), "project uses tokio".into()]);
        let built = ctx.build(100_000);
        assert!(matches!(built[0].role, Role::System)); // system first
        assert_eq!(
            built[1].content,
            "Relevant memories from past sessions:\n- user likes rust\n- project uses tokio"
        );
        assert!(matches!(built[1].role, Role::System)); // recall block is system-role
        assert_eq!(built.last().unwrap().content, "hello"); // history after recall
    }

    #[test]
    fn empty_recall_injects_no_block() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        ctx.append(Message::user("hello"));
        ctx.set_recall(vec![]);
        let built = ctx.build(100_000);
        assert_eq!(built.len(), 2); // system + history only
    }

    #[test]
    fn set_recall_replaces_previous_lines() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        ctx.set_recall(vec!["old".into()]);
        ctx.set_recall(vec!["new".into()]);
        let built = ctx.build(100_000);
        assert!(built[1].content.contains("new"));
        assert!(!built[1].content.contains("old"));
    }

    #[test]
    fn recall_block_is_capped_by_budget() {
        // 30 long lines vastly exceed a 64-token budget; the block must stay under it
        // (plus the soft floor of one line) — never inject all 30.
        let mut ctx = WindowContext::new(Message::system("SYS")).with_recall_budget(64);
        let lines: Vec<String> = (0..30)
            .map(|i| format!("memory fact number {i} with a fair amount of padding text"))
            .collect();
        ctx.set_recall(lines);
        let built = ctx.build(100_000);
        let block = &built[1].content;
        // Far fewer than 30 lines survived.
        assert!(block.matches("\n- ").count() < 30);
        assert!(block.starts_with("Relevant memories from past sessions:"));
    }

    #[test]
    fn history_is_evicted_before_recall_and_system() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        for i in 0..50 {
            ctx.append(Message::user(format!(
                "message number {i} with some padding text"
            )));
        }
        ctx.set_recall(vec!["pinned memory".into()]);
        let built = ctx.build(40); // tiny limit forces history eviction
        assert!(matches!(built[0].role, Role::System));
        assert!(built[1].content.contains("pinned memory")); // recall survives
        assert!(built.len() < 51); // history evicted
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
    fn parent2(id1: &str, id2: &str) -> Message {
        Message::assistant(
            "calling two",
            Some(vec![
                agent_tools::ToolCall {
                    id: id1.into(),
                    name: "shell".into(),
                    args: serde_json::json!({}),
                },
                agent_tools::ToolCall {
                    id: id2.into(),
                    name: "shell".into(),
                    args: serde_json::json!({}),
                },
            ]),
        )
    }

    #[test]
    fn turn_units_group_parent_with_consecutive_tool_results() {
        let h = vec![
            Message::user("u0"), // unit 0..1
            parent("c1"),        // unit 1..3
            Message::tool("c1", "shell", "r1"),
            Message::user("u1"), // unit 3..4
        ];
        assert_eq!(turn_unit_ranges(&h), vec![0..1, 1..3, 3..4]);
    }

    #[test]
    fn turn_units_parallel_calls_are_one_unit() {
        let h = vec![
            parent2("c1", "c2"), // unit 0..3
            Message::tool("c1", "shell", "r1"),
            Message::tool("c2", "shell", "r2"),
        ];
        assert_eq!(turn_unit_ranges(&h), vec![0..3]);
    }

    #[test]
    fn turn_units_stray_tool_is_a_singleton() {
        // Defensive: a Role::Tool with no preceding parent must not panic or
        // mis-attach; it stays a singleton unit.
        let h = vec![Message::tool("cX", "shell", "stray"), Message::user("u")];
        assert_eq!(turn_unit_ranges(&h), vec![0..1, 1..2]);
        assert_eq!(turn_unit_ranges(&[]), Vec::<std::ops::Range<usize>>::new());
    }

    #[test]
    fn evict_start_drops_whole_units_and_keeps_newest_even_over_budget() {
        let h = vec![
            Message::user("old message with padding padding padding"),
            parent("c1"),
            Message::tool("c1", "shell", &"x".repeat(200)),
            Message::user("newest"),
        ];
        // Budget 0: only the newest unit survives (keep-≥1-unit floor).
        assert_eq!(evict_start(&h, 0), 3);
        // Huge budget: everything kept.
        assert_eq!(evict_start(&h, 1_000_000), 0);
        // Budget that fits "newest" + the tool unit but not the old user msg:
        let tool_unit: usize = h[1..3].iter().map(message_tokens).sum();
        let newest = message_tokens(&h[3]);
        assert_eq!(evict_start(&h, tool_unit + newest), 1);
        // One token short of the tool unit: the cut moves to the unit start,
        // never inside it.
        assert_eq!(evict_start(&h, tool_unit + newest - 1), 3);
    }

    #[test]
    fn snap_split_moves_left_to_a_unit_boundary() {
        let h = vec![
            Message::user("u0"), // boundary at 1
            parent("c1"),        // unit 1..4
            Message::tool("c1", "shell", "r1"),
            Message::tool("c1", "shell", "r2"),
            Message::user("u1"), // boundary at 4, 5
        ];
        assert_eq!(snap_split_to_unit_boundary(&h, 4), 4); // exact boundary unchanged
        assert_eq!(snap_split_to_unit_boundary(&h, 3), 1); // mid-unit snaps left
        assert_eq!(snap_split_to_unit_boundary(&h, 2), 1);
        assert_eq!(snap_split_to_unit_boundary(&h, 0), 0); // snap to zero
        assert_eq!(snap_split_to_unit_boundary(&h, 5), 5);
    }

    #[test]
    fn orphan_checker_flags_tool_without_live_parent() {
        let clean = vec![parent("c1"), Message::tool("c1", "shell", "r")];
        assert!(orphaned_tool_positions(&clean).is_empty());
        // Parent evicted → orphan.
        let torn = vec![Message::tool("c1", "shell", "r"), Message::user("u")];
        assert_eq!(orphaned_tool_positions(&torn), vec![0]);
        // A non-tool interloper breaks adjacency → later result is orphaned.
        let interloped = vec![
            parent("c1"),
            Message::tool("c1", "shell", "r1"),
            Message::user("interloper"),
            Message::tool("c1", "shell", "r2"),
        ];
        assert_eq!(orphaned_tool_positions(&interloped), vec![3]);
        // Wrong id → orphaned.
        let wrong = vec![parent("c1"), Message::tool("c9", "shell", "r")];
        assert_eq!(orphaned_tool_positions(&wrong), vec![1]);
    }

    #[test]
    fn build_never_orphans_tool_results() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        ctx.append(Message::user(
            "old old old message with lots of padding text here",
        ));
        ctx.append(parent("c1"));
        ctx.append(Message::tool("c1", "shell", &"y".repeat(120)));
        ctx.append(Message::user("recent"));
        // Budget forces the cut inside the tool turn under the old walk:
        // recent fits, tool result fits, parent does not.
        let tool_result_t = message_tokens(&Message::tool("c1", "shell", &"y".repeat(120)));
        let recent_t = message_tokens(&Message::user("recent"));
        let sys_t = message_tokens(&Message::system("SYS"));
        let limit = sys_t + recent_t + tool_result_t + 2; // parent excluded
        let built = ctx.build(limit);
        assert!(
            orphaned_tool_positions(&built).is_empty(),
            "eviction must drop the torn tool turn whole, got: {:?}",
            built
                .iter()
                .map(|m| (&m.role, &m.content))
                .collect::<Vec<_>>()
        );
        // The torn turn was dropped whole: no c1 result without its parent.
        let has_result = built
            .iter()
            .any(|m| m.tool_call_id.as_deref() == Some("c1"));
        let has_parent = built.iter().any(|m| m.tool_calls.is_some());
        assert_eq!(has_result, has_parent);
    }

    #[test]
    fn window_build_budget_sweep_never_orphans() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        ctx.append(Message::user("intro message with padding"));
        ctx.append(parent("c1"));
        ctx.append(Message::tool("c1", "shell", &"a".repeat(100)));
        ctx.append(Message::user("middle instruction"));
        ctx.append(parent2("c2", "c3"));
        ctx.append(Message::tool("c2", "shell", &"b".repeat(80)));
        ctx.append(Message::tool("c3", "shell", "tiny"));
        ctx.append(Message::user("latest"));
        let total = built_tokens(&ctx.build(usize::MAX)) + 16;
        for limit in 1..=total {
            let built = ctx.build(limit);
            assert!(
                orphaned_tool_positions(&built).is_empty(),
                "orphan at model_limit={limit}"
            );
        }
    }

    #[test]
    fn set_system_replaces_prompt_and_keeps_history() {
        let mut ctx = WindowContext::new(Message::system("OLD"));
        ctx.append(Message::user("u1"));
        ctx.set_system(Message::system("NEW"));
        let built = ctx.build(100_000);
        assert!(matches!(built[0].role, Role::System)); // system still first
        assert_eq!(built[0].content, "NEW"); // and replaced
        assert!(built.iter().any(|m| m.content == "u1")); // history intact
    }
}
