use agent_core::{AgentEvent, EventSink, ToolStatus};
use agent_tools::Display;
use similar::{ChangeTag, TextDiff};
use std::io::Write;
use std::sync::Mutex;

#[allow(dead_code)]
pub fn render_diff(before: &str, after: &str) -> String {
    let diff = TextDiff::from_lines(before, after);
    let mut out = String::new();
    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        out.push_str(sign);
        out.push_str(change.value());
    }
    out
}

fn fmt_k(n: u64) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

/// One dim summary line printed after each run (pure for testability).
pub fn format_stats_line(s: &agent_core::SessionStats) -> String {
    let failed = s.tools_denied + s.tools_error + s.tools_timeout + s.tools_panic;
    let mut line = format!(
        "— session: {} turns · {} in / {} out tokens · {} tools ({} failed) · {:.1}s in tools",
        s.turns,
        fmt_k(s.prompt_tokens),
        fmt_k(s.completion_tokens),
        s.tool_calls,
        failed,
        s.tool_time_ms as f64 / 1000.0
    );
    if s.cost_usd > 0.0 {
        line.push_str(&format!(" · ${:.2}", s.cost_usd));
    }
    if s.subagent_tool_calls > 0 || s.subagent_turns > 0 {
        line.push_str(&format!(
            " · sub-agent: {} calls/{} turns",
            s.subagent_tool_calls, s.subagent_turns
        ));
    }
    line
}

/// The ToolStart display line (pure for testability); child (attributed)
/// calls get a two-space `↳` indent so nested activity reads as nested
/// (spec E7). The caller prepends the leading blank line via `writeln!`.
fn format_tool_start(name: &str, args: &serde_json::Value, parent_id: Option<&str>) -> String {
    let indent = if parent_id.is_some() { "  ↳ " } else { "" };
    format!("{indent}\x1b[36m⚙ {name}\x1b[0m {args}")
}

/// Renders agent events to stdout/stderr. Buffers streamed tokens inline.
#[allow(dead_code)]
pub struct TerminalSink {
    out: Mutex<std::io::Stdout>,
}

impl Default for TerminalSink {
    fn default() -> Self {
        Self {
            out: Mutex::new(std::io::stdout()),
        }
    }
}

impl EventSink for TerminalSink {
    fn emit(&self, event: AgentEvent) {
        let mut out = self.out.lock().unwrap();
        match event {
            AgentEvent::Token(t) => {
                let _ = write!(out, "{t}");
                let _ = out.flush();
            }
            AgentEvent::Reasoning(r) => {
                let _ = write!(out, "\x1b[2m{r}\x1b[0m");
                let _ = out.flush();
            }
            AgentEvent::ToolStart {
                name,
                args,
                parent_id,
                ..
            } => {
                let _ = writeln!(
                    out,
                    "\n{}",
                    format_tool_start(&name, &args, parent_id.as_deref())
                );
            }
            AgentEvent::ToolResult {
                name,
                status,
                output,
                duration_ms,
                parent_id,
                ..
            } => {
                // Attributed (sub-agent) results get the same `↳` indent as their
                // ToolStart so the child row reads as nested (spec E7).
                let indent = if parent_id.is_some() { "  ↳ " } else { "" };
                if status != ToolStatus::Ok {
                    let _ = writeln!(
                        out,
                        "{indent}\x1b[31m✗ {name} ({}, {duration_ms}ms)\x1b[0m {}",
                        status.as_str(),
                        output.content
                    );
                } else if let Some(Display::Diff {
                    path,
                    before,
                    after,
                }) = &output.display
                {
                    let _ = writeln!(
                        out,
                        "{indent}\x1b[33m✎ {path}\x1b[0m\n{}",
                        render_diff(before, after)
                    );
                } else if let Some(Display::Terminal {
                    exit_code,
                    stdout,
                    stderr,
                    ..
                }) = &output.display
                {
                    let _ = writeln!(
                        out,
                        "{indent}\x1b[90m$ exit={exit_code}\x1b[0m\n{stdout}{stderr}"
                    );
                } else {
                    let _ = writeln!(out, "{indent}\x1b[32m✓ {name}\x1b[0m");
                }
            }
            AgentEvent::Usage { .. } => {} // telemetry for the web context dashboard; not shown in the CLI
            AgentEvent::ServerUsage { .. } => {} // server-reported usage telemetry; not shown in the CLI
            AgentEvent::Context(c) => {
                use agent_core::ContextEvent as CE;
                let note = match c {
                    CE::Offloaded { id, bytes, tool } =>
                        format!("⟲ offloaded {tool} result #{id} ({} KB)", bytes / 1024),
                    CE::Compacted { turns_replaced, tokens_before, tokens_after } =>
                        format!("⟲ compacted {turns_replaced} turns: {tokens_before} → {tokens_after} tokens"),
                    CE::CompactionFailed { reason } => format!("⚠ compaction failed: {reason}"),
                    CE::Evicted { messages, est_tokens } =>
                        format!("⟲ evicted {messages} messages (~{est_tokens} tokens)"),
                    CE::OverflowRecovery =>
                        "⟲ context overflow: compacted and retried".to_string(),
                };
                let _ = writeln!(out, "\x1b[2m{note}\x1b[0m");
            }
            AgentEvent::Approval(_) => {} // the TerminalApproval channel prints its own prompt
            AgentEvent::SandboxDegraded { mechanism, reason } => {
                let _ = writeln!(
                    out,
                    "\n\x1b[33m⚠ sandbox degraded: {mechanism} unavailable ({reason}); \
                     exec-capable tools are DISABLED until it is available\x1b[0m"
                );
            }
            AgentEvent::Error(e) => {
                let _ = writeln!(out, "\n\x1b[31m✗ {e}\x1b[0m");
            }
            AgentEvent::Done(_) => {
                let _ = writeln!(out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_diff_marks_added_and_removed_lines() {
        let out = render_diff("foo\nbar\n", "foo\nbaz\n");
        assert!(out.contains("-bar"));
        assert!(out.contains("+baz"));
        assert!(out.contains(" foo")); // unchanged context kept
    }

    // Legacy lint, unrelated to this branch: field-by-field build reads clearly in a test.
    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn stats_line_summarizes_session() {
        let mut s = agent_core::SessionStats::default();
        s.turns = 3;
        s.prompt_tokens = 12_400;
        s.completion_tokens = 2_100;
        s.tool_calls = 7;
        s.tools_error = 1;
        s.tools_timeout = 1;
        s.tool_time_ms = 4_200;
        s.cost_usd = 0.05;
        let line = format_stats_line(&s);
        assert!(line.contains("3 turns"));
        assert!(line.contains("12.4k in"));
        assert!(line.contains("2.1k out"));
        assert!(line.contains("7 tools"));
        assert!(line.contains("2 failed"));
        assert!(line.contains("$0.05"));
    }

    #[test]
    fn child_tool_rows_are_indented() {
        let args = serde_json::json!({});
        let top = format_tool_start("read_file", &args, None);
        let child = format_tool_start("sub:read_file", &args, Some("d1"));
        assert!(!top.contains('↳'));
        assert!(child.starts_with("  ↳"), "{child:?}");
        assert!(child.contains("sub:read_file"));
    }

    #[test]
    fn stats_line_omits_zero_cost() {
        let s = agent_core::SessionStats::default();
        assert!(!format_stats_line(&s).contains('$'));
    }

    #[test]
    fn stats_line_mentions_subagents_only_when_present() {
        let mut s = agent_core::SessionStats::default();
        assert!(!format_stats_line(&s).contains("sub-agent"));
        s.subagent_tool_calls = 3;
        s.subagent_turns = 2;
        let line = format_stats_line(&s);
        assert!(line.contains("sub-agent: 3 calls/2 turns"), "{line}");
    }
}
