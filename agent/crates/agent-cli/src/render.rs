use agent_core::{AgentEvent, EventSink, ToolStatus};
use agent_model::StopReason;
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

/// The ToolResult display block (pure for testability); child (attributed)
/// results get the same two-space `↳` indent as their ToolStart so the row
/// reads as nested (spec E7). Mirrors `format_tool_start`; the caller writes
/// this via a single `writeln!`.
fn format_tool_result(
    name: &str,
    status: ToolStatus,
    output: &agent_tools::ToolOutput,
    duration_ms: u64,
    parent_id: Option<&str>,
) -> String {
    let indent = if parent_id.is_some() { "  ↳ " } else { "" };
    if status != ToolStatus::Ok {
        format!(
            "{indent}\x1b[31m✗ {name} ({}, {duration_ms}ms)\x1b[0m {}",
            status.as_str(),
            output.content
        )
    } else if let Some(Display::Diff {
        path,
        before,
        after,
    }) = &output.display
    {
        format!(
            "{indent}\x1b[33m✎ {path}\x1b[0m\n{}",
            render_diff(before, after)
        )
    } else if let Some(Display::Terminal {
        exit_code,
        stdout,
        stderr,
        ..
    }) = &output.display
    {
        format!("{indent}\x1b[90m$ exit={exit_code}\x1b[0m\n{stdout}{stderr}")
    } else {
        format!("{indent}\x1b[32m✓ {name}\x1b[0m")
    }
}

/// Mirrors `wire.rs::stop_reason_str` — six variants, snake_case wire words
/// (a `{:?}.to_lowercase()` shortcut is WRONG: BudgetExhausted != budget_exhausted).
#[allow(dead_code)]
fn stop_str(r: &StopReason) -> &'static str {
    match r {
        StopReason::Stop => "stop",
        StopReason::ToolCalls => "tool_calls",
        StopReason::Length => "length",
        StopReason::BudgetExhausted => "budget_exhausted",
        StopReason::Cancelled => "cancelled",
        StopReason::Error => "error",
    }
}

/// The Subagent Start display line (pure for testability).
fn format_subagent_start(subagent_type: &str, role: Option<&str>) -> String {
    let role_note = role.map(|r| format!(" — {r}")).unwrap_or_default();
    format!("  ↳ \x1b[36magent[{subagent_type}]\x1b[0m started{role_note}")
}

/// The Subagent End display line (pure for testability).
fn format_subagent_end(
    outcome: agent_core::SubagentOutcome,
    stop: Option<&str>,
    detail: Option<&str>,
    turns: usize,
    tool_calls: u64,
    duration_ms: u64,
) -> String {
    use agent_core::SubagentOutcome as O;
    let word = match outcome {
        O::Completed => "done",
        O::Timeout => "timed out",
        O::Failed => "failed",
        O::Cancelled => "cancelled",
    };
    let stop_note = stop.map(|s| format!("{s}, ")).unwrap_or_default();
    let detail_note = detail.map(|d| format!(" — {d}")).unwrap_or_default();
    let secs = duration_ms as f64 / 1000.0;
    format!(
        "  ↳ agent {word} — {stop_note}{turns} turns, {tool_calls} tools, {secs:.1}s{detail_note}"
    )
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
                let _ = writeln!(
                    out,
                    "{}",
                    format_tool_result(&name, status, &output, duration_ms, parent_id.as_deref())
                );
            }
            AgentEvent::Usage { .. } => {} // telemetry for the web context dashboard; not shown in the CLI
            AgentEvent::ServerUsage { .. } => {} // server-reported usage telemetry; not shown in the CLI
            AgentEvent::Context(c) => {
                use agent_core::ContextEvent as CE;
                let note = match c {
                    CE::Offloaded { path, bytes, tool } =>
                        format!("⟲ offloaded {tool} result → {path} ({} KB)", bytes / 1024),
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
            AgentEvent::RunStart { .. } => {} // trace-only record; nothing to render
            AgentEvent::SandboxDegraded { mechanism, reason } => {
                let _ = writeln!(
                    out,
                    "\n\x1b[33m⚠ sandbox degraded: {mechanism} unavailable ({reason}); \
                     exec-capable tools are DISABLED until it is available\x1b[0m"
                );
            }
            AgentEvent::StreamRetry { .. } => {
                let _ = writeln!(
                    out,
                    "\n\x1b[2m[stream interrupted — retrying; partial output above is discarded]\x1b[0m"
                );
            }
            AgentEvent::Error(e) => {
                let _ = writeln!(out, "\n\x1b[31m✗ {e}\x1b[0m");
            }
            AgentEvent::Done(_) => {
                let _ = writeln!(out);
            }
            AgentEvent::Subagent(se) => {
                use agent_core::SubagentEvent as SE;
                match se {
                    SE::Start {
                        subagent_type,
                        role,
                        ..
                    } => {
                        let _ = writeln!(
                            out,
                            "\n{}",
                            format_subagent_start(&subagent_type, role.as_deref())
                        );
                    }
                    SE::End {
                        outcome,
                        stop,
                        detail,
                        turns,
                        tool_calls,
                        duration_ms,
                        ..
                    } => {
                        let _ = writeln!(
                            out,
                            "{}",
                            format_subagent_end(
                                outcome,
                                stop.map(|r| stop_str(&r)),
                                detail.as_deref(),
                                turns,
                                tool_calls,
                                duration_ms
                            )
                        );
                    }
                    // Live child prose is terminal noise (spec §2.5, owner
                    // decision) — lifecycle lines only.
                    SE::Text { .. } | SE::Reasoning { .. } | SE::StreamRetry { .. } => {}
                }
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
    fn tool_result_helper_pins_ok_and_error_literals_and_indent() {
        let ok = agent_tools::ToolOutput {
            content: "unused".into(),
            display: None,
        };
        let err = agent_tools::ToolOutput {
            content: "ERROR: nope".into(),
            display: None,
        };
        // Unattributed lines are byte-identical to the pre-helper literals.
        assert_eq!(
            format_tool_result("read_file", ToolStatus::Ok, &ok, 5, None),
            "\x1b[32m✓ read_file\x1b[0m"
        );
        assert_eq!(
            format_tool_result("read_file", ToolStatus::Denied, &err, 5, None),
            "\x1b[31m✗ read_file (denied, 5ms)\x1b[0m ERROR: nope"
        );
        // Attributed (sub-agent) lines gain the two-space `↳` indent.
        let ok_child = format_tool_result("sub:read_file", ToolStatus::Ok, &ok, 5, Some("d1"));
        let err_child =
            format_tool_result("sub:read_file", ToolStatus::Denied, &err, 5, Some("d1"));
        assert!(ok_child.starts_with("  ↳ "), "{ok_child:?}");
        assert!(err_child.starts_with("  ↳ "), "{err_child:?}");
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
    fn subagent_lifecycle_lines() {
        assert!(format_subagent_start("researcher", None).contains("agent[researcher]"));
        assert!(format_subagent_start("general-purpose", Some("be brief")).contains("— be brief"));
        let end = format_subagent_end(
            agent_core::SubagentOutcome::Timeout,
            None,
            Some("sub-agent timed out after 5s"),
            2,
            3,
            5000,
        );
        assert!(end.contains("timed out"), "{end}");
        assert!(end.contains("2 turns, 3 tools, 5.0s"), "{end}");
        assert!(end.contains("— sub-agent timed out after 5s"), "{end}");
        let done = format_subagent_end(
            agent_core::SubagentOutcome::Completed,
            Some("stop"),
            None,
            4,
            7,
            12300,
        );
        assert!(
            done.contains("done — stop, 4 turns, 7 tools, 12.3s"),
            "{done}"
        );
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
