use crate::context::{estimate_tokens, message_tokens};
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
pub(crate) fn build_snapshot(
    turn: usize,
    model_limit: usize,
    system: &Message,
    goal: Option<&Message>,
    recall: &[String],
    compaction_summary: Option<&Message>,
    history: &[Message],
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

    if !recall.is_empty() {
        let est = recall.iter().map(|l| estimate_tokens(l)).sum();
        segments.push(ContextSegment {
            category: "memory".into(),
            est_tokens: est,
            items: recall.iter().map(|l| preview(l, 100)).collect(),
            count: recall.len(),
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
    use agent_model::Message;

    #[test]
    fn snapshot_has_system_and_messages_and_sums_total() {
        let snap = build_snapshot(
            3,
            1000,
            &Message::system("SYSTEM PROMPT"),
            None,
            &[],
            None,
            &[Message::user("hello"), Message::assistant("hi", None)],
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
    fn preview_collapses_newlines_and_truncates() {
        assert_eq!(preview("a\nb\nc", 100), "a b c"); // newlines → single spaces
        assert_eq!(preview("hello world", 5), "hello"); // truncates to n chars
        assert_eq!(preview("anything", 0), ""); // n = 0 → empty
        assert_eq!(preview("", 10), ""); // empty input → empty
    }

    #[test]
    fn recall_block_becomes_memory_segment_with_previews() {
        let snap = build_snapshot(
            1,
            1000,
            &Message::system("S"),
            None,
            &[
                "user likes rust".to_string(),
                "deploys on friday".to_string(),
            ],
            None,
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
}
