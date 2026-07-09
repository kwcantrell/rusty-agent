use agent_model::{Message, Role};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OffloadKind {
    Error,
    Output,
}

#[derive(Debug, Clone)]
pub struct OffloadEntry {
    pub tool_call_id: String,
    pub tool_name: String,
    pub kind: OffloadKind,
    pub content: String,
    pub bytes: usize,
    pub turn: usize,
}

/// The two skip literals (spec §5.5) — as narrow as the old "[tool_result#":
/// selectors and the durable-unit detector all gate on exactly these.
pub const PLACEHOLDER_PREFIXES: [&str; 2] = ["[tool_result offloaded", "[tool_result truncated"];

pub fn is_placeholder(content: &str) -> bool {
    PLACEHOLDER_PREFIXES.iter().any(|p| content.starts_with(p))
}

/// Default eager ingestion cap (`OffloadConfig::max_result_bytes`), also the
/// default for `RuntimeConfig::max_tool_result_bytes`. ~4K tokens: large
/// enough for real command output, small enough that one result cannot
/// swamp a small window.
pub const DEFAULT_MAX_TOOL_RESULT_BYTES: usize = 16 * 1024;

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
    /// Eager cap: any tool result larger than this many bytes is offloaded at
    /// ingestion (bounded preview + recall marker), regardless of age.
    pub max_result_bytes: usize,
}

impl Default for OffloadConfig {
    fn default() -> Self {
        Self {
            error_min_bytes: 200,
            output_min_bytes: 1024,
            keep_recent: 3,
            exclude_tools: Vec::new(),
            max_result_bytes: DEFAULT_MAX_TOOL_RESULT_BYTES,
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
        if is_placeholder(&m.content) {
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

/// The compact stub left in the live window (spec §5.5 grammar, verbatim).
pub fn placeholder_for(vpath: &str, tool_name: &str, kind: &OffloadKind, bytes: usize) -> String {
    let kind_str = match kind {
        OffloadKind::Error => "error",
        OffloadKind::Output => "output",
    };
    format!(
        "[tool_result offloaded to {vpath}: {bytes}B {kind_str} from \"{tool_name}\" \
         — read_file the path, or grep large_tool_results/ to search]"
    )
}

/// Marker appended to an ingestion-capped preview; continuation is read_file
/// byte mode against the artifact path (spec §5.4/§5.5).
pub fn truncation_marker(vpath: &str, tool_name: &str, shown: usize, total: usize) -> String {
    format!(
        "\n[tool_result truncated: showing first {shown}B of {total}B from \"{tool_name}\" \
         — full content at {vpath}; continue with read_file(path: \"{vpath}\", byte_offset: {shown})]"
    )
}

/// Truncate `content` so preview + marker fit within `cap` bytes (char-boundary
/// safe). When `cap` cannot even hold the marker, degrades to a marker-only
/// string with no leading newline, which starts with `[tool_result truncated`
/// and is therefore never re-selected.
pub fn capped_preview(content: &str, cap: usize, vpath: &str, tool_name: &str) -> String {
    let total = content.len();
    // Budget against the widest the marker can render (`shown = total`), so the
    // final string can only come in under `cap`.
    let worst = truncation_marker(vpath, tool_name, total, total);
    let mut cut = cap.saturating_sub(worst.len()).min(total);
    while !content.is_char_boundary(cut) {
        cut -= 1;
    }
    if cut == 0 {
        return truncation_marker(vpath, tool_name, 0, total)
            .trim_start()
            .to_string();
    }
    format!(
        "{}{}",
        &content[..cut],
        truncation_marker(vpath, tool_name, cut, total)
    )
}

/// Select tool-result messages exceeding the eager ingestion cap, regardless
/// of age. Pure: no I/O, deterministic. Skips excluded tools and placeholders.
pub fn select_oversized(history: &[Message], config: &OffloadConfig) -> Vec<OffloadHit> {
    let mut hits = Vec::new();
    for (i, m) in history.iter().enumerate() {
        if !matches!(m.role, Role::Tool)
            || is_placeholder(&m.content)
            || m.content.len() <= config.max_result_bytes
        {
            continue;
        }
        let tool_name = m.name.clone().unwrap_or_default();
        if config.exclude_tools.iter().any(|t| t == &tool_name) {
            continue;
        }
        hits.push(OffloadHit {
            history_index: i,
            entry: OffloadEntry {
                tool_call_id: m.tool_call_id.clone().unwrap_or_default(),
                tool_name,
                kind: classify(&m.content),
                content: m.content.clone(),
                bytes: m.content.len(),
                turn: i,
            },
        });
    }
    hits
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
            &placeholder_for("large_tool_results/7-c1", "shell", &OffloadKind::Error, 300),
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
    fn placeholder_preserves_path_and_tool() {
        let p = placeholder_for(
            "large_tool_results/7-c1",
            "shell",
            &OffloadKind::Error,
            3100,
        );
        assert!(p.contains("offloaded to large_tool_results/7-c1"));
        assert!(p.contains("read_file the path"));
        assert!(p.contains("shell"));
        assert!(p.contains("3100B"));
    }

    fn cap_cfg(max: usize) -> OffloadConfig {
        OffloadConfig {
            max_result_bytes: max,
            ..Default::default()
        }
    }

    #[test]
    fn oversized_fresh_result_is_selected_despite_keep_recent() {
        // keep_recent protects by AGE; size-based selection must ignore it.
        let history = vec![tool_msg("shell", &"x".repeat(5000))];
        let hits = select_oversized(&history, &cap_cfg(1024));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].history_index, 0);
        assert_eq!(hits[0].entry.bytes, 5000);
        assert_eq!(hits[0].entry.kind, OffloadKind::Output);
    }

    #[test]
    fn at_cap_result_is_not_selected() {
        let history = vec![tool_msg("shell", &"x".repeat(1024))];
        assert!(select_oversized(&history, &cap_cfg(1024)).is_empty());
    }

    #[test]
    fn oversized_error_is_classified_error() {
        let history = vec![tool_msg("shell", &format!("ERROR: {}", "x".repeat(5000)))];
        let hits = select_oversized(&history, &cap_cfg(1024));
        assert_eq!(hits[0].entry.kind, OffloadKind::Error);
    }

    #[test]
    fn placeholders_and_excluded_tools_are_not_selected() {
        let placeholder = format!(
            "[tool_result offloaded to large_tool_results/7-c1: 9000B output \
             from \"shell\" — read_file the path, or grep large_tool_results/ to search]{}",
            "x".repeat(2000)
        );
        let history = vec![
            Message::tool("c1", "shell", &placeholder),
            Message::tool("c2", "use_skill", "y".repeat(5000)),
        ];
        let cfg = OffloadConfig {
            max_result_bytes: 1024,
            exclude_tools: vec!["use_skill".into()],
            ..Default::default()
        };
        assert!(select_oversized(&history, &cfg).is_empty());
    }

    #[test]
    fn non_tool_messages_are_never_selected() {
        let history = vec![Message::user("x".repeat(5000))];
        assert!(select_oversized(&history, &cap_cfg(1024)).is_empty());
    }

    #[test]
    fn capped_preview_fits_cap_and_carries_the_recall_hint() {
        let content = "x".repeat(50_000);
        let out = capped_preview(&content, 1024, "large_tool_results/42-c1", "shell");
        assert!(out.len() <= 1024, "preview+marker {} > cap", out.len());
        assert!(out.starts_with("xxx"));
        assert!(out.contains("full content at large_tool_results/42-c1"));
        assert!(out.contains("of 50000B"));
        assert!(out.contains("byte_offset: "));
    }

    #[test]
    fn capped_preview_respects_char_boundaries() {
        // 4-byte scalars; a byte-index cut inside one would panic on slicing.
        let content = "🦀".repeat(20_000);
        let out = capped_preview(&content, 1024, "large_tool_results/1-c1", "shell");
        assert!(out.len() <= 1024);
        assert!(out.starts_with('🦀'));
    }

    #[test]
    fn capped_preview_is_idempotent_under_reselection() {
        let content = "x".repeat(50_000);
        let out = capped_preview(&content, 1024, "large_tool_results/1-c1", "shell");
        let history = vec![Message::tool("c1", "shell", &out)];
        assert!(
            select_oversized(&history, &cap_cfg(1024)).is_empty(),
            "capped output must never be re-selected"
        );
    }

    #[test]
    fn pathological_small_cap_degrades_to_placeholder_prefix() {
        // cap smaller than the marker itself: output is marker-only and must
        // start with a skip literal so both selectors skip it forever.
        let content = "x".repeat(50_000);
        let out = capped_preview(&content, 16, "large_tool_results/3-c1", "shell");
        assert!(out.starts_with("[tool_result truncated"));
        let history = vec![Message::tool("c1", "shell", &out)];
        assert!(select_oversized(&history, &cap_cfg(16)).is_empty());
    }

    #[test]
    fn result_echoing_a_placeholder_line_is_skipped_accepted_residual() {
        // A large tool result whose content STARTS with a full placeholder
        // line is skipped by the selectors — the same theoretical false
        // positive today's "[tool_result#" prefix had, accepted at the panel
        // (spec §5.5). This pin makes any future change to that behavior a
        // conscious decision.
        let echoed = format!(
            "{}\n{}",
            placeholder_for(
                "large_tool_results/9-cX",
                "shell",
                &OffloadKind::Output,
                9000
            ),
            "y".repeat(5000)
        );
        let history = vec![Message::tool("c1", "shell", &echoed)];
        assert!(select_oversized(&history, &cap_cfg(1024)).is_empty());
    }
}
