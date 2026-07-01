use agent_core::AgentEvent;
use agent_model::StopReason;

pub use agent_core::{ContextSegment, ContextSnapshot};
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
    /// Faithful server-reported token totals for the completed turn; the web
    /// Context Explorer uses this as ground truth for the prompt-token chart.
    ServerUsage { prompt_tokens: u32, completion_tokens: u32, turn: usize },
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
    SandboxDegraded { mechanism: String, reason: String },
}

/// Settings snapshot returned by the `settings_get` command (was `WireBody::SettingsState`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsState {
    pub settings: RuntimeConfig,
    pub workspace: String,
    pub api_key_set: bool,
    pub hard_floor: Vec<String>,
    pub discovered_skills: Vec<DiscoveredSkill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_degraded: Option<SandboxDegraded>,
}

/// Degraded-sandbox posture carried in `SettingsState` (connect-time) and as a
/// streamed `ServerEvent` (run-start). Present only when isolation was requested
/// but not delivered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SandboxDegraded { pub mechanism: String, pub reason: String }

/// Extract the degraded posture from a sandbox descriptor, if any. Pure so the
/// daemon's `settings_state()` stays trivial and this stays unit-testable.
pub fn sandbox_degraded_from(desc: Option<agent_tools::SandboxDescriptor>) -> Option<SandboxDegraded> {
    desc.and_then(|d| d.degraded.map(|reason| SandboxDegraded {
        mechanism: d.mechanism.to_string(), reason }))
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
        AgentEvent::Context(_) => return None, // curation telemetry; not forwarded to clients in v1
        AgentEvent::ServerUsage { prompt_tokens, completion_tokens, turn } =>
            ServerEvent::ServerUsage { prompt_tokens, completion_tokens, turn },
        AgentEvent::SandboxDegraded { mechanism, reason } =>
            ServerEvent::SandboxDegraded { mechanism: mechanism.to_string(), reason },
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
    fn server_usage_serializes_with_type_tag() {
        let ev = server_event_from(AgentEvent::ServerUsage {
            prompt_tokens: 42,
            completion_tokens: 7,
            turn: 3,
        }).unwrap();
        let j = serde_json::to_string(&ev).unwrap();
        assert!(j.contains(r#""type":"server_usage""#), "missing type tag: {j}");
        assert!(j.contains(r#""prompt_tokens":42"#), "missing prompt_tokens: {j}");
        assert!(j.contains(r#""completion_tokens":7"#), "missing completion_tokens: {j}");
        assert!(j.contains(r#""turn":3"#), "missing turn: {j}");
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

    #[test]
    fn sandbox_degraded_event_serializes_with_type_tag() {
        let ev = server_event_from(AgentEvent::SandboxDegraded {
            mechanism: "docker", reason: "no daemon".into() }).unwrap();
        let j = serde_json::to_string(&ev).unwrap();
        assert!(j.contains(r#""type":"sandbox_degraded""#), "missing type tag: {j}");
        assert!(j.contains(r#""mechanism":"docker""#), "missing mechanism: {j}");
        assert!(j.contains(r#""reason":"no daemon""#), "missing reason: {j}");
    }

    #[test]
    fn sandbox_degraded_from_maps_only_when_degraded() {
        use agent_tools::{SandboxDescriptor, Mode};
        let degraded = SandboxDescriptor { mode: Mode::Auto, mechanism: "docker",
            image: None, network: false, degraded: Some("no daemon".into()) };
        assert_eq!(sandbox_degraded_from(Some(degraded)),
            Some(SandboxDegraded { mechanism: "docker".into(), reason: "no daemon".into() }));

        let healthy = SandboxDescriptor { mode: Mode::Off, mechanism: "host",
            image: None, network: true, degraded: None };
        assert_eq!(sandbox_degraded_from(Some(healthy)), None);
        assert_eq!(sandbox_degraded_from(None), None);
    }
}
