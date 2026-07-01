use crate::AgentEvent;

/// Rolling per-session telemetry, computed as a pure fold over the
/// [`AgentEvent`] stream. Serialized to the wire and mirrored in TypeScript,
/// so the field names below are binding.
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionStats {
    pub turns: usize,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
    pub cached_tokens: u64,
    pub cost_usd: f64,
    pub tool_calls: u64,
    pub tools_ok: u64,
    pub tools_denied: u64,
    pub tools_error: u64,
    pub tools_timeout: u64,
    pub tools_panic: u64,
    pub tool_time_ms: u64,
    pub turn_time_ms: u64,
    pub context_events: u64,
    pub errors: u64,
}

impl SessionStats {
    /// Pure accumulator over the event stream. Token/cost fields SUM per-turn
    /// server usage (total billed volume); `turns` tracks the highest turn seen.
    pub fn fold(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::ServerUsage { prompt_tokens, completion_tokens, reasoning_tokens,
                cached_tokens, cost_usd, turn_duration_ms, turn } => {
                self.turns = self.turns.max(*turn);
                self.prompt_tokens += *prompt_tokens as u64;
                self.completion_tokens += *completion_tokens as u64;
                self.reasoning_tokens += reasoning_tokens.unwrap_or(0) as u64;
                self.cached_tokens += cached_tokens.unwrap_or(0) as u64;
                self.cost_usd += cost_usd.unwrap_or(0.0);
                self.turn_time_ms += turn_duration_ms;
            }
            AgentEvent::ToolStart { .. } => self.tool_calls += 1,
            AgentEvent::ToolResult { status, duration_ms, .. } => {
                self.tool_time_ms += duration_ms;
                match status {
                    crate::ToolStatus::Ok => self.tools_ok += 1,
                    crate::ToolStatus::Denied => self.tools_denied += 1,
                    crate::ToolStatus::Error => self.tools_error += 1,
                    crate::ToolStatus::Timeout => self.tools_timeout += 1,
                    crate::ToolStatus::Panic => self.tools_panic += 1,
                }
            }
            AgentEvent::Context(_) => self.context_events += 1,
            AgentEvent::Error(_) => self.errors += 1,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentEvent, ContextEvent, ToolStatus};
    use agent_tools::ToolOutput;

    #[test]
    fn fold_accumulates_usage_tools_and_context() {
        let mut s = SessionStats::default();
        s.fold(&AgentEvent::ServerUsage { prompt_tokens: 100, completion_tokens: 40,
            reasoning_tokens: Some(10), cached_tokens: Some(60), cost_usd: Some(0.02),
            turn_duration_ms: 500, turn: 1 });
        s.fold(&AgentEvent::ServerUsage { prompt_tokens: 200, completion_tokens: 50,
            reasoning_tokens: None, cached_tokens: None, cost_usd: Some(0.03),
            turn_duration_ms: 700, turn: 2 });
        s.fold(&AgentEvent::ToolStart { id: "a".into(), name: "t".into(),
            args: serde_json::json!({}) });
        s.fold(&AgentEvent::ToolResult { id: "a".into(), name: "t".into(),
            status: ToolStatus::Ok,
            output: ToolOutput { content: "x".into(), display: None }, duration_ms: 30 });
        s.fold(&AgentEvent::ToolResult { id: "b".into(), name: "t".into(),
            status: ToolStatus::Timeout,
            output: ToolOutput { content: "e".into(), display: None }, duration_ms: 60000 });
        s.fold(&AgentEvent::Context(ContextEvent::CompactionFailed { reason: "r".into() }));
        s.fold(&AgentEvent::Error("boom".into()));

        assert_eq!(s.turns, 2);
        assert_eq!(s.prompt_tokens, 300);
        assert_eq!(s.completion_tokens, 90);
        assert_eq!(s.reasoning_tokens, 10);
        assert_eq!(s.cached_tokens, 60);
        assert!((s.cost_usd - 0.05).abs() < 1e-9);
        assert_eq!(s.turn_time_ms, 1200);
        assert_eq!(s.tool_calls, 1);           // counted on ToolStart
        assert_eq!(s.tools_ok, 1);
        assert_eq!(s.tools_timeout, 1);
        assert_eq!(s.tool_time_ms, 60030);
        assert_eq!(s.context_events, 1);
        assert_eq!(s.errors, 1);
    }
}
