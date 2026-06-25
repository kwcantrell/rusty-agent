use agent_core::AgentEvent;
use agent_model::StopReason;
use agent_policy::ApprovalResponse;
use agent_runtime_config::RuntimeConfig;
use agent_tools::Display;
use serde::{Deserialize, Serialize};

/// Outbound streaming event sent over the Tauri channel. Mirrors the legacy
/// `WireEvent` tagged shape so the frontend reducer is unchanged, plus the
/// `approval_request` case (was a sibling `WireBody`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Token { text: String },
    Reasoning { text: String },
    Usage { prompt_tokens: usize, context_limit: usize, turn: usize, max_turns: usize },
    ToolStart { name: String, args: serde_json::Value },
    ToolResult {
        name: String,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<Display>,
    },
    Error { message: String },
    Done { reason: String },
    ApprovalRequest {
        id: String,
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<Display>,
    },
}

/// Settings snapshot returned by the `settings_get` command (was `WireBody::SettingsState`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsState {
    pub settings: RuntimeConfig,
    pub workspace: String,
    pub api_key_set: bool,
    pub hard_floor: Vec<String>,
    pub discovered_skills: Vec<DiscoveredSkill>,
}

/// Read-only skill info surfaced in `settings_state` for the Settings UI's
/// active-skills picker. Daemon-computed from the current `skills_dirs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredSkill {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision { Approve, ApproveAlways, Deny }

impl From<Decision> for ApprovalResponse {
    fn from(d: Decision) -> Self {
        match d {
            Decision::Approve => ApprovalResponse::Approve,
            Decision::ApproveAlways => ApprovalResponse::ApproveAlways,
            Decision::Deny => ApprovalResponse::Deny,
        }
    }
}

/// Transport-agnostic outbound sink. `src-tauri` implements this over an
/// `ipc::Channel<ServerEvent>`; `agent-server` never sees Tauri.
pub trait EventOut: Send + Sync {
    fn send(&self, ev: ServerEvent);
}

fn stop_reason_str(r: &StopReason) -> &'static str {
    match r {
        StopReason::Stop => "stop",
        StopReason::ToolCalls => "tool_calls",
        StopReason::Length => "length",
        StopReason::BudgetExhausted => "budget_exhausted",
        StopReason::Cancelled => "cancelled",
    }
}

/// Map a core `AgentEvent` to its wire form. `Approval` returns `None` — the
/// approval channel emits its own `ApprovalRequest` (mirrors the CLI sink).
pub fn server_event_from(event: AgentEvent) -> Option<ServerEvent> {
    Some(match event {
        AgentEvent::Token(t) => ServerEvent::Token { text: t },
        AgentEvent::Reasoning(t) => ServerEvent::Reasoning { text: t },
        AgentEvent::Usage { prompt_tokens, context_limit, turn, max_turns } =>
            ServerEvent::Usage { prompt_tokens, context_limit, turn, max_turns },
        AgentEvent::ToolStart { name, args } => ServerEvent::ToolStart { name, args },
        AgentEvent::ToolResult { name, output } => ServerEvent::ToolResult {
            name, content: output.content, display: output.display },
        AgentEvent::Error(m) => ServerEvent::Error { message: m },
        AgentEvent::Done(r) => ServerEvent::Done { reason: stop_reason_str(&r).into() },
        AgentEvent::Approval(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::AgentEvent;

    #[test]
    fn token_serializes_with_type_tag() {
        let ev = server_event_from(AgentEvent::Token("hi".into())).unwrap();
        let j = serde_json::to_string(&ev).unwrap();
        assert_eq!(j, r#"{"type":"token","text":"hi"}"#);
    }

    #[test]
    fn approval_event_maps_to_none_but_variant_exists() {
        use agent_policy::ApprovalRequest;
        use agent_tools::{Access, ToolIntent};
        let req = ApprovalRequest {
            intent: ToolIntent { tool: "x".into(), access: Access::Write, paths: vec![],
                command: None, summary: "s".into() },
            display: None,
        };
        assert!(server_event_from(AgentEvent::Approval(req)).is_none());
        let ar = ServerEvent::ApprovalRequest { id: "c0".into(), summary: "s".into(),
            command: None, display: None };
        let j = serde_json::to_string(&ar).unwrap();
        assert!(j.contains(r#""type":"approval_request""#));
        assert!(j.contains(r#""id":"c0""#));
    }

    #[test]
    fn done_uses_stop_reason_string() {
        let ev = server_event_from(AgentEvent::Done(StopReason::Cancelled)).unwrap();
        assert_eq!(serde_json::to_string(&ev).unwrap(), r#"{"type":"done","reason":"cancelled"}"#);
    }

    #[test]
    fn decision_into_response() {
        assert_eq!(ApprovalResponse::from(Decision::ApproveAlways), ApprovalResponse::ApproveAlways);
    }
}
