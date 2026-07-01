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
    if n >= 1000 { format!("{:.1}k", n as f64 / 1000.0) } else { n.to_string() }
}

/// One dim summary line printed after each run (pure for testability).
pub fn format_stats_line(s: &agent_core::SessionStats) -> String {
    let failed = s.tools_denied + s.tools_error + s.tools_timeout + s.tools_panic;
    let mut line = format!(
        "— session: {} turns · {} in / {} out tokens · {} tools ({} failed) · {:.1}s in tools",
        s.turns, fmt_k(s.prompt_tokens), fmt_k(s.completion_tokens),
        s.tool_calls, failed, s.tool_time_ms as f64 / 1000.0);
    if s.cost_usd > 0.0 { line.push_str(&format!(" · ${:.2}", s.cost_usd)); }
    line
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
            AgentEvent::ToolStart { name, args, .. } => {
                let _ = writeln!(out, "\n\x1b[36m⚙ {name}\x1b[0m {args}");
            }
            AgentEvent::ToolResult { name, status, output, duration_ms, .. } => {
                if status != ToolStatus::Ok {
                    let _ = writeln!(out, "\x1b[31m✗ {name} ({}, {duration_ms}ms)\x1b[0m {}",
                        status.as_str(), output.content);
                } else if let Some(Display::Diff {
                    path,
                    before,
                    after,
                }) = &output.display
                {
                    let _ = writeln!(
                        out,
                        "\x1b[33m✎ {path}\x1b[0m\n{}",
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
                        "\x1b[90m$ exit={exit_code}\x1b[0m\n{stdout}{stderr}"
                    );
                } else {
                    let _ = writeln!(out, "\x1b[32m✓ {name}\x1b[0m");
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
                };
                let _ = writeln!(out, "\x1b[2m{note}\x1b[0m");
            }
            AgentEvent::Approval(_) => {} // the TerminalApproval channel prints its own prompt
            AgentEvent::SandboxDegraded { mechanism, reason } => {
                let _ = writeln!(out,
                    "\n\x1b[33m⚠ sandbox degraded: {mechanism} unavailable ({reason}); \
                     tools run UNSANDBOXED on the host\x1b[0m");
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

    #[test]
    fn stats_line_summarizes_session() {
        let mut s = agent_core::SessionStats::default();
        s.turns = 3; s.prompt_tokens = 12_400; s.completion_tokens = 2_100;
        s.tool_calls = 7; s.tools_error = 1; s.tools_timeout = 1;
        s.tool_time_ms = 4_200; s.cost_usd = 0.05;
        let line = format_stats_line(&s);
        assert!(line.contains("3 turns"));
        assert!(line.contains("12.4k in"));
        assert!(line.contains("2.1k out"));
        assert!(line.contains("7 tools"));
        assert!(line.contains("2 failed"));
        assert!(line.contains("$0.05"));
    }

    #[test]
    fn stats_line_omits_zero_cost() {
        let s = agent_core::SessionStats::default();
        assert!(!format_stats_line(&s).contains('$'));
    }
}
