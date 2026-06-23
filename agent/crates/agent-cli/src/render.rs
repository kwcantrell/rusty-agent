use agent_core::{AgentEvent, EventSink};
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
            AgentEvent::ToolStart { name, args } => {
                let _ = writeln!(out, "\n\x1b[36m⚙ {name}\x1b[0m {args}");
            }
            AgentEvent::ToolResult { name, output } => {
                if let Some(Display::Diff {
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
            AgentEvent::Approval(_) => {} // the TerminalApproval channel prints its own prompt
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
}
