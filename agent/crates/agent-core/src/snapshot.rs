use crate::context::{estimate_tokens, memory_block, message_tokens};
use agent_model::Message;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextSegment {
    pub category: String,
    pub est_tokens: usize,
    pub items: Vec<String>,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextSnapshot {
    pub turn: usize,
    pub model_limit: usize,
    pub est_total: usize,
    pub segments: Vec<ContextSegment>,
}

/// First `n` chars of a single-line preview of `s`.
pub(crate) fn preview(s: &str, n: usize) -> String {
    let one_line = s.replace('\n', " ");
    one_line.chars().take(n).collect()
}

/// Build a snapshot from already-separated context blocks. Pure so it is unit
/// testable without a full CuratedContext.
// One positional param per pinned context block plus the memory index cap — a flat
// fan-in of already-separated pieces, not state worth bundling into a struct.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_snapshot(
    turn: usize,
    model_limit: usize,
    system: &Message,
    goal: Option<&Message>,
    ledger: Option<&Message>,
    ledger_items: &[String],
    memory_index: &[String],
    memory_index_budget: usize,
    compaction_summary: Option<&Message>,
    history: &[Message],
    todos: &[crate::TodoItem],
) -> ContextSnapshot {
    let mut segments = Vec::new();

    let sys_tokens = message_tokens(system);
    segments.push(ContextSegment {
        category: "system".into(),
        est_tokens: sys_tokens,
        items: vec![preview(&system.content, 120)],
        count: 1,
    });

    if let Some(g) = goal {
        segments.push(ContextSegment {
            category: "goal".into(),
            est_tokens: message_tokens(g),
            items: vec![preview(&g.content, 120)],
            count: 1,
        });
    }

    // The folded-facts ledger is pinned (it rides inside the goal block in
    // pinned()) and charged in pinned_tokens() as its OWN message — a separate
    // segment here keeps est_total equal to the budget math (audit 7.3).
    if let Some(l) = ledger {
        segments.push(ContextSegment {
            category: "ledger".into(),
            est_tokens: message_tokens(l),
            items: ledger_items.iter().map(|f| preview(f, 100)).collect(),
            count: ledger_items.len(),
        });
    }

    // The context injects only the capped `memory_block(memory_index, budget)` — a
    // greedy whole-entry prefix under the token cap — so the snapshot sizes
    // the memory segment from that SAME block, never the raw line sum, or the
    // dashboard over-reports memory pressure. `kept` is recovered by counting
    // how many non-empty input lines survived as a prefix of the rendered
    // entries (raw, one per line, after the fixed header+framing preamble).
    if let Some(block) = memory_block(memory_index, memory_index_budget) {
        let entries: Vec<&String> = memory_index
            .iter()
            .filter(|l| !l.trim().is_empty())
            .collect();
        let kept = entries
            .iter()
            .take_while(|l| block.content.contains(l.as_str()))
            .count();
        segments.push(ContextSegment {
            category: "memory".into(),
            est_tokens: estimate_tokens(&block.content),
            items: entries[..kept].iter().map(|l| preview(l, 100)).collect(),
            count: kept,
        });
    }

    if let Some(c) = compaction_summary {
        segments.push(ContextSegment {
            category: "summary".into(),
            est_tokens: message_tokens(c),
            items: vec![preview(&c.content, 120)],
            count: 1,
        });
    }

    // Pinned todos plan (spec §5.4 / S5): its own segment keeps est_total equal
    // to the budget math, exactly like the ledger segment (audit 7.3). Positioned
    // after `summary` and before `messages`, mirroring pinned()'s last-pinned slot.
    if let Some(block) = crate::render_todos_block(todos) {
        segments.push(ContextSegment {
            category: "todos".into(),
            est_tokens: message_tokens(&block),
            items: todos.iter().map(|t| preview(&t.content, 100)).collect(),
            count: todos.len(),
        });
    }

    let msg_tokens: usize = history.iter().map(message_tokens).sum();
    segments.push(ContextSegment {
        category: "messages".into(),
        est_tokens: msg_tokens,
        // Intentionally empty: message bodies are rendered in the main transcript,
        // not the explorer drill-in. Only the count/token total is surfaced here.
        items: Vec::new(),
        count: history.len(),
    });

    let est_total = segments.iter().map(|s| s.est_tokens).sum();
    ContextSnapshot {
        turn,
        model_limit,
        est_total,
        segments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::DEFAULT_MEMORY_INDEX_BUDGET;
    use agent_model::Message;

    #[test]
    fn snapshot_has_system_and_messages_and_sums_total() {
        let snap = build_snapshot(
            3,
            1000,
            &Message::system("SYSTEM PROMPT"),
            None,
            None,
            &[],
            &[],
            DEFAULT_MEMORY_INDEX_BUDGET,
            None,
            &[Message::user("hello"), Message::assistant("hi", None)],
            &[],
        );
        assert_eq!(snap.turn, 3);
        let cats: Vec<&str> = snap.segments.iter().map(|s| s.category.as_str()).collect();
        assert_eq!(cats, vec!["system", "messages"]);
        let messages = snap
            .segments
            .iter()
            .find(|s| s.category == "messages")
            .unwrap();
        assert_eq!(messages.count, 2);
        assert_eq!(
            snap.est_total,
            snap.segments.iter().map(|s| s.est_tokens).sum::<usize>()
        );
    }

    #[test]
    fn ledger_block_becomes_ledger_segment() {
        let ledger = Message::system(
            "Ledger of earlier user instructions…\n1. port = 8080\n2. name = zephyr",
        );
        let facts = vec!["port = 8080".to_string(), "name = zephyr".to_string()];
        let snap = build_snapshot(
            1,
            1000,
            &Message::system("S"),
            Some(&Message::system("Original goal: g")),
            Some(&ledger),
            &facts,
            &[],
            DEFAULT_MEMORY_INDEX_BUDGET,
            None,
            &[],
            &[],
        );
        let cats: Vec<&str> = snap.segments.iter().map(|s| s.category.as_str()).collect();
        assert_eq!(cats, vec!["system", "goal", "ledger", "messages"]);
        let ledger_seg = snap
            .segments
            .iter()
            .find(|s| s.category == "ledger")
            .unwrap();
        assert_eq!(ledger_seg.est_tokens, message_tokens(&ledger));
        assert_eq!(ledger_seg.count, 2);
        assert!(ledger_seg.items[0].contains("port = 8080"));
        assert_eq!(
            snap.est_total,
            snap.segments.iter().map(|s| s.est_tokens).sum::<usize>()
        );
    }

    #[test]
    fn no_ledger_segment_without_folded_facts() {
        let snap = build_snapshot(
            1,
            1000,
            &Message::system("S"),
            None,
            None,
            &[],
            &[],
            DEFAULT_MEMORY_INDEX_BUDGET,
            None,
            &[],
            &[],
        );
        assert!(snap.segments.iter().all(|s| s.category != "ledger"));
    }

    #[test]
    fn preview_collapses_newlines_and_truncates() {
        assert_eq!(preview("a\nb\nc", 100), "a b c"); // newlines → single spaces
        assert_eq!(preview("hello world", 5), "hello"); // truncates to n chars
        assert_eq!(preview("anything", 0), ""); // n = 0 → empty
        assert_eq!(preview("", 10), ""); // empty input → empty
    }

    #[test]
    fn memory_index_block_becomes_memory_segment_with_previews() {
        let snap = build_snapshot(
            1,
            1000,
            &Message::system("S"),
            None,
            None,
            &[],
            &[
                "user likes rust".to_string(),
                "deploys on friday".to_string(),
            ],
            DEFAULT_MEMORY_INDEX_BUDGET,
            None,
            &[],
            &[],
        );
        let mem = snap
            .segments
            .iter()
            .find(|s| s.category == "memory")
            .unwrap();
        assert_eq!(mem.count, 2);
        assert!(mem.items[0].contains("rust"));
    }

    #[test]
    fn memory_segment_uses_capped_memory_block_not_raw_sum() {
        // Many long memory index lines vastly exceed a tiny budget. The context only
        // injects the capped `memory_block` (a short whole-entry prefix), so the
        // snapshot's memory segment must be sized from that block — est ≤ the
        // block's own estimate — never the raw sum of all lines.
        let budget = 32;
        let lines: Vec<String> = (0..40)
            .map(|i| format!("memory fact number {i} with a fair amount of padding text here"))
            .collect();
        let raw_sum: usize = lines.iter().map(|l| estimate_tokens(l)).sum();
        let snap = build_snapshot(
            1,
            100_000,
            &Message::system("S"),
            None,
            None,
            &[],
            &lines,
            budget,
            None,
            &[],
            &[],
        );
        let mem = snap
            .segments
            .iter()
            .find(|s| s.category == "memory")
            .unwrap();
        // The capped block, not all 40 lines.
        let block = memory_block(&lines, budget).unwrap();
        let kept = lines
            .iter()
            .take_while(|l| block.content.contains(l.as_str()))
            .count();
        assert!(kept < lines.len(), "block must be capped below all lines");
        assert_eq!(mem.count, kept);
        assert_eq!(mem.items.len(), kept);
        let block_est = estimate_tokens(&block.content);
        assert_eq!(mem.est_tokens, block_est);
        assert!(
            mem.est_tokens < raw_sum,
            "capped est {} must be far below raw sum {}",
            mem.est_tokens,
            raw_sum
        );
    }
}
