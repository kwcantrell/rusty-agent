//! Sub-agent dispatch: sub-agents-as-tools (spec 2026-07-01-subagent-dispatch-core).
use crate::{AgentEvent, EventSink};
use agent_model::StopReason;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Appended to the parent's composed system prompt for every child.
pub const SUBAGENT_PREAMBLE: &str = "You are a sub-agent dispatched by a parent \
agent to complete one self-contained task. Work autonomously: no one can answer \
questions. Your final message is returned verbatim to the parent as the task \
result, so end with a complete, standalone answer.";

static DISPATCH_ORDINAL: AtomicU64 = AtomicU64::new(1);

/// Process-wide dispatch ordinal: keeps forwarded child event ids unique across
/// parallel siblings and across the parent's own tool-call ids (spec D9).
pub fn next_dispatch_n() -> u64 {
    DISPATCH_ORDINAL.fetch_add(1, Ordering::Relaxed)
}

#[derive(Default)]
struct Capture {
    /// Token text split into segments at ToolResult boundaries; the last
    /// segment is the child's final-turn text (spec D10).
    segments: Vec<String>,
    tool_calls: u64,
    turns: usize,
    stop: Option<StopReason>,
}

pub struct CaptureSummary {
    pub final_text: String,
    pub tool_calls: u64,
    pub turns: usize,
    pub stop: Option<StopReason>,
}

/// The child loop's sink: captures the transcript for the tool result and
/// forwards ONLY ToolStart/ToolResult (ids `sub{n}:{id}`, names `sub:{name}`)
/// plus ServerUsage (real cost) to the parent sink — all existing wire frames,
/// so no wire/web/CLI changes (spec D9). Child Token/Done/Error/Context stay
/// private: the parent's streamed transcript must not be corrupted.
pub struct SubagentSink {
    parent: Arc<dyn EventSink>,
    n: u64,
    cap: Mutex<Capture>,
}

impl SubagentSink {
    pub fn new(parent: Arc<dyn EventSink>, n: u64) -> Self {
        Self {
            parent,
            n,
            cap: Mutex::new(Capture { segments: vec![String::new()], ..Capture::default() }),
        }
    }

    pub fn summary(&self) -> CaptureSummary {
        let cap = self.cap.lock().unwrap();
        let tail = cap.segments.last().cloned().unwrap_or_default();
        let final_text = if tail.trim().is_empty() {
            cap.segments.concat().trim().to_string()
        } else {
            tail.trim().to_string()
        };
        CaptureSummary {
            final_text,
            tool_calls: cap.tool_calls,
            turns: cap.turns,
            stop: cap.stop,
        }
    }
}

impl EventSink for SubagentSink {
    fn emit(&self, event: AgentEvent) {
        let mut cap = self.cap.lock().unwrap();
        match event {
            AgentEvent::Token(t) => {
                cap.segments.last_mut().expect("segments never empty").push_str(&t);
            }
            AgentEvent::ToolStart { id, name, args } => {
                cap.tool_calls += 1;
                drop(cap);
                self.parent.emit(AgentEvent::ToolStart {
                    id: format!("sub{}:{}", self.n, id),
                    name: format!("sub:{name}"),
                    args,
                });
            }
            AgentEvent::ToolResult { id, name, status, output, duration_ms } => {
                cap.segments.push(String::new());
                drop(cap);
                self.parent.emit(AgentEvent::ToolResult {
                    id: format!("sub{}:{}", self.n, id),
                    name: format!("sub:{name}"),
                    status,
                    output,
                    duration_ms,
                });
            }
            e @ AgentEvent::ServerUsage { .. } => {
                drop(cap);
                self.parent.emit(e);
            }
            AgentEvent::Usage { turn, .. } => {
                cap.turns = cap.turns.max(turn);
            }
            AgentEvent::Done(reason) => {
                cap.stop = Some(reason);
            }
            // Suppressed: Reasoning, Approval, Error, Context, SandboxDegraded
            // (spec D9 — child terminal/context events are the tool result's
            // business, not the parent transcript's).
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentEvent, ContextEvent, EventSink, ToolStatus};
    use agent_model::StopReason;
    use agent_tools::ToolOutput;
    use std::sync::{Arc, Mutex};

    /// Captures full (kind, id, name) triples — testkit's CollectingSink drops ids.
    #[derive(Default)]
    struct FullSink {
        events: Mutex<Vec<(String, String, String)>>,
    }
    impl EventSink for FullSink {
        fn emit(&self, event: AgentEvent) {
            let triple = match event {
                AgentEvent::ToolStart { id, name, .. } => ("tool_start".into(), id, name),
                AgentEvent::ToolResult { id, name, status, .. } => {
                    (format!("tool_result:{}", status.as_str()), id, name)
                }
                AgentEvent::ServerUsage { prompt_tokens, .. } => {
                    ("server_usage".into(), prompt_tokens.to_string(), String::new())
                }
                // Anything else reaching the parent is a forwarding-table bug —
                // record it so the exact-equality assertion below catches the leak.
                _ => ("unexpected".to_string(), String::new(), String::new()),
            };
            self.events.lock().unwrap().push(triple);
        }
    }

    fn tool_result(id: &str, name: &str) -> AgentEvent {
        AgentEvent::ToolResult {
            id: id.into(),
            name: name.into(),
            status: ToolStatus::Ok,
            output: ToolOutput { content: "r".into(), display: None },
            duration_ms: 1,
        }
    }

    #[test]
    fn forwards_tool_events_rewritten_and_suppresses_the_rest() {
        let parent = Arc::new(FullSink::default());
        let sink = SubagentSink::new(parent.clone(), 7);
        sink.emit(AgentEvent::Token("hi".into()));
        sink.emit(AgentEvent::Reasoning("r".into()));
        sink.emit(AgentEvent::Usage { prompt_tokens: 1, context_limit: 10, turn: 1, max_turns: 5 });
        sink.emit(AgentEvent::ToolStart { id: "c1".into(), name: "echo".into(), args: serde_json::json!({}) });
        sink.emit(tool_result("c1", "echo"));
        sink.emit(AgentEvent::ServerUsage {
            prompt_tokens: 42, completion_tokens: 1, reasoning_tokens: None,
            cached_tokens: None, cost_usd: None, turn_duration_ms: 1, turn: 1,
        });
        sink.emit(AgentEvent::Error("boom".into()));
        sink.emit(AgentEvent::Context(ContextEvent::OverflowRecovery));
        sink.emit(AgentEvent::Done(StopReason::Stop));

        let got = parent.events.lock().unwrap().clone();
        // ONLY ToolStart/ToolResult (rewritten) + ServerUsage (verbatim) forwarded.
        assert_eq!(
            got,
            vec![
                ("tool_start".to_string(), "sub7:c1".to_string(), "sub:echo".to_string()),
                ("tool_result:ok".to_string(), "sub7:c1".to_string(), "sub:echo".to_string()),
                ("server_usage".to_string(), "42".to_string(), String::new()),
            ]
        );
    }

    #[test]
    fn summary_final_text_is_tail_after_last_tool_result() {
        let sink = SubagentSink::new(Arc::new(FullSink::default()), 1);
        sink.emit(AgentEvent::Token("thinking...".into()));
        sink.emit(tool_result("c1", "echo"));
        sink.emit(AgentEvent::Token("final ".into()));
        sink.emit(AgentEvent::Token("answer".into()));
        sink.emit(AgentEvent::Done(StopReason::Stop));
        let s = sink.summary();
        assert_eq!(s.final_text, "final answer");
        assert_eq!(s.tool_calls, 0); // no ToolStart was emitted
        assert_eq!(s.stop, Some(StopReason::Stop));
    }

    #[test]
    fn summary_falls_back_to_all_text_when_tail_is_blank() {
        let sink = SubagentSink::new(Arc::new(FullSink::default()), 1);
        sink.emit(AgentEvent::Token("early words".into()));
        sink.emit(tool_result("c1", "echo"));
        // no tokens after the last tool result
        let s = sink.summary();
        assert_eq!(s.final_text, "early words");
    }

    #[test]
    fn summary_counts_tool_calls_and_turns() {
        let sink = SubagentSink::new(Arc::new(FullSink::default()), 1);
        sink.emit(AgentEvent::Usage { prompt_tokens: 1, context_limit: 10, turn: 1, max_turns: 5 });
        sink.emit(AgentEvent::ToolStart { id: "c1".into(), name: "a".into(), args: serde_json::json!({}) });
        sink.emit(AgentEvent::ToolStart { id: "c2".into(), name: "b".into(), args: serde_json::json!({}) });
        sink.emit(AgentEvent::Usage { prompt_tokens: 2, context_limit: 10, turn: 2, max_turns: 5 });
        let s = sink.summary();
        assert_eq!(s.tool_calls, 2);
        assert_eq!(s.turns, 2);
    }

    #[test]
    fn dispatch_ordinals_are_unique() {
        let a = next_dispatch_n();
        let b = next_dispatch_n();
        assert_ne!(a, b);
    }
}
