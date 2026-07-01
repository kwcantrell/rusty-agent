use crate::offload::{OffloadEntry, OffloadId, OffloadKind};
use agent_model::{Message, Role};

const PLACEHOLDER_PREFIX: &str = "[tool_result#";

#[derive(Debug, Clone)]
pub struct OffloadConfig {
    /// Tool ERROR results at or above this many bytes are eligible.
    pub error_min_bytes: usize,
    /// Successful tool outputs at or above this many bytes are eligible.
    pub output_min_bytes: usize,
    /// The N most-recent tool results are always kept verbatim.
    pub keep_recent: usize,
    /// Tool names never offloaded.
    pub exclude_tools: Vec<String>,
}

impl Default for OffloadConfig {
    fn default() -> Self {
        Self {
            error_min_bytes: 200,
            output_min_bytes: 1024,
            keep_recent: 3,
            exclude_tools: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OffloadHit {
    pub history_index: usize,
    pub entry: OffloadEntry,
}

fn classify(content: &str) -> OffloadKind {
    if content.starts_with("ERROR: ") {
        OffloadKind::Error
    } else {
        OffloadKind::Output
    }
}

fn qualifies(kind: &OffloadKind, bytes: usize, config: &OffloadConfig) -> bool {
    match kind {
        OffloadKind::Error => bytes >= config.error_min_bytes,
        OffloadKind::Output => bytes >= config.output_min_bytes,
    }
}

/// Select tool-result messages eligible to be lifted out of the live window.
/// Pure: no I/O, deterministic. Skips the `keep_recent` most-recent tool results,
/// excluded tools, and already-offloaded placeholders.
pub fn select_offloads(history: &[Message], config: &OffloadConfig) -> Vec<OffloadHit> {
    // Indices of all tool-result messages, oldest-first.
    let tool_indices: Vec<usize> = history
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m.role, Role::Tool))
        .map(|(i, _)| i)
        .collect();
    // Protect the most-recent `keep_recent` tool results.
    let protected_from = tool_indices.len().saturating_sub(config.keep_recent);
    let eligible_indices = &tool_indices[..protected_from.min(tool_indices.len())];

    let mut hits = Vec::new();
    for &i in eligible_indices {
        let m = &history[i];
        if m.content.starts_with(PLACEHOLDER_PREFIX) {
            continue; // already offloaded
        }
        let tool_name = m.name.clone().unwrap_or_default();
        if config.exclude_tools.iter().any(|t| t == &tool_name) {
            continue;
        }
        let bytes = m.content.len();
        let kind = classify(&m.content);
        if !qualifies(&kind, bytes, config) {
            continue;
        }
        hits.push(OffloadHit {
            history_index: i,
            entry: OffloadEntry {
                id: 0,
                tool_call_id: m.tool_call_id.clone().unwrap_or_default(),
                tool_name,
                kind,
                content: m.content.clone(),
                bytes,
                turn: i,
            },
        });
    }
    hits
}

/// The compact stub left in the live window in place of offloaded content.
pub fn placeholder_for(id: OffloadId, tool_name: &str, kind: &OffloadKind, bytes: usize) -> String {
    let kind_str = match kind {
        OffloadKind::Error => "error",
        OffloadKind::Output => "output",
    };
    format!(
        "[tool_result#{id} offloaded: {bytes}B {kind_str} from \"{tool_name}\" \
         — recall with context_recall({id})]"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_model::Message;

    fn tool_msg(name: &str, content: &str) -> Message {
        Message::tool("call-x", name, content)
    }

    #[test]
    fn large_error_is_selected() {
        let history = vec![tool_msg("shell", &format!("ERROR: {}", "x".repeat(300)))];
        let hits = select_offloads(
            &history,
            &OffloadConfig {
                keep_recent: 0,
                ..Default::default()
            },
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry.kind, OffloadKind::Error);
        assert_eq!(hits[0].history_index, 0);
    }

    #[test]
    fn small_error_under_threshold_is_kept() {
        let history = vec![tool_msg("shell", "ERROR: nope")];
        let hits = select_offloads(
            &history,
            &OffloadConfig {
                keep_recent: 0,
                ..Default::default()
            },
        );
        assert!(hits.is_empty());
    }

    #[test]
    fn large_success_output_is_selected() {
        let history = vec![tool_msg("read_file", &"y".repeat(2000))];
        let hits = select_offloads(
            &history,
            &OffloadConfig {
                keep_recent: 0,
                ..Default::default()
            },
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry.kind, OffloadKind::Output);
    }

    #[test]
    fn keep_recent_protects_newest_tool_results() {
        let history = vec![
            tool_msg("shell", &format!("ERROR: {}", "a".repeat(300))),
            tool_msg("shell", &format!("ERROR: {}", "b".repeat(300))),
            tool_msg("shell", &format!("ERROR: {}", "c".repeat(300))),
        ];
        let hits = select_offloads(
            &history,
            &OffloadConfig {
                keep_recent: 2,
                ..Default::default()
            },
        );
        // Only the oldest of three is eligible.
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].history_index, 0);
    }

    #[test]
    fn exclude_tools_is_honored() {
        let history = vec![tool_msg("shell", &format!("ERROR: {}", "a".repeat(300)))];
        let config = OffloadConfig {
            keep_recent: 0,
            exclude_tools: vec!["shell".into()],
            ..Default::default()
        };
        assert!(select_offloads(&history, &config).is_empty());
    }

    #[test]
    fn already_offloaded_placeholder_is_skipped() {
        let history = vec![tool_msg(
            "shell",
            &placeholder_for(7, "shell", &OffloadKind::Error, 300),
        )];
        let hits = select_offloads(
            &history,
            &OffloadConfig {
                keep_recent: 0,
                ..Default::default()
            },
        );
        assert!(
            hits.is_empty(),
            "must be idempotent — never re-offload a placeholder"
        );
    }

    // Legacy lint, unrelated to this branch.
    #[allow(clippy::needless_borrows_for_generic_args)]
    #[test]
    fn non_tool_messages_are_ignored() {
        let history = vec![
            Message::user(&"u".repeat(3000)),
            Message::assistant(&"a".repeat(3000), None),
        ];
        let hits = select_offloads(
            &history,
            &OffloadConfig {
                keep_recent: 0,
                ..Default::default()
            },
        );
        assert!(hits.is_empty());
    }

    #[test]
    fn placeholder_preserves_id_and_tool() {
        let p = placeholder_for(7, "shell", &OffloadKind::Error, 3100);
        assert!(p.contains("tool_result#7"));
        assert!(p.contains("context_recall(7)"));
        assert!(p.contains("shell"));
        assert!(p.contains("3100B"));
    }
}
