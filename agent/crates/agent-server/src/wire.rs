use agent_core::AgentEvent;
use agent_model::StopReason;
use agent_policy::ApprovalResponse;
use agent_runtime_config::RuntimeConfig;
use agent_tools::Display;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireEnvelope {
    pub v: u32,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(flatten)]
    pub body: WireBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireBody {
    Event { payload: WireEvent },
    ApprovalRequest {
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<Display>,
    },
    UserInput { text: String },
    ApprovalResponse { decision: WireDecision },
    SettingsGet,
    SettingsUpdate {
        settings: RuntimeConfig,
    },
    SettingsState {
        settings: RuntimeConfig,
        workspace: String,
        api_key_set: bool,
        hard_floor: Vec<String>,
        discovered_skills: Vec<DiscoveredSkill>,
    },
    SettingsError {
        message: String,
    },
}

/// Read-only skill info surfaced in `settings_state` for the Settings UI's
/// active-skills picker. Daemon-computed from the current `skills_dirs`; never
/// part of `RuntimeConfig`, so it cannot be edited or round-tripped back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredSkill {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireEvent {
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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireDecision { Approve, ApproveAlways, Deny }

impl From<WireDecision> for ApprovalResponse {
    fn from(d: WireDecision) -> Self {
        match d {
            WireDecision::Approve => ApprovalResponse::Approve,
            WireDecision::ApproveAlways => ApprovalResponse::ApproveAlways,
            WireDecision::Deny => ApprovalResponse::Deny,
        }
    }
}

fn stop_reason_str(r: &StopReason) -> &'static str {
    match r {
        StopReason::Stop => "stop",
        StopReason::ToolCalls => "tool_calls",
        StopReason::Length => "length",
        StopReason::BudgetExhausted => "budget_exhausted",
    }
}

/// Map a core `AgentEvent` to its wire form. Returns `None` for `Approval`,
/// which the `WsApprovalChannel` sends as its own `approval_request` frame
/// (so it is not also relayed as an event — mirrors the CLI sink).
pub fn wire_event_from(event: AgentEvent) -> Option<WireEvent> {
    Some(match event {
        AgentEvent::Token(t) => WireEvent::Token { text: t },
        AgentEvent::Reasoning(t) => WireEvent::Reasoning { text: t },
        AgentEvent::Usage { prompt_tokens, context_limit, turn, max_turns } =>
            WireEvent::Usage { prompt_tokens, context_limit, turn, max_turns },
        AgentEvent::ToolStart { name, args } => WireEvent::ToolStart { name, args },
        AgentEvent::ToolResult { name, output } => WireEvent::ToolResult {
            name,
            content: output.content,
            display: output.display,
        },
        AgentEvent::Error(m) => WireEvent::Error { message: m },
        AgentEvent::Done(r) => WireEvent::Done { reason: stop_reason_str(&r).into() },
        AgentEvent::Approval(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::AgentEvent;

    #[test]
    fn tool_result_with_markdown_display_round_trips() {
        use agent_tools::Display;
        let payload = WireEvent::ToolResult {
            name: "render".into(),
            content: "rendered markdown".into(),
            display: Some(Display::Markdown { text: "# Hi".into(), title: Some("Plan".into()), id: None }),
        };
        let env = WireEnvelope { v: PROTOCOL_VERSION, session_id: "s".into(), id: None,
            body: WireBody::Event { payload } };
        let j = serde_json::to_string(&env).unwrap();
        assert!(j.contains("\"kind\":\"event\""));
        assert!(j.contains("\"type\":\"tool_result\""));
        assert!(j.contains("\"Markdown\""));
        let back: WireEnvelope = serde_json::from_str(&j).unwrap();
        assert!(matches!(back.body, WireBody::Event { .. }));
    }

    #[test]
    fn event_envelope_round_trips() {
        let payload = wire_event_from(AgentEvent::Token("hi".into())).unwrap();
        let env = WireEnvelope {
            v: PROTOCOL_VERSION,
            session_id: "s1".into(),
            id: None,
            body: WireBody::Event { payload },
        };
        let json = serde_json::to_string(&env).unwrap();
        assert!(json.contains("\"kind\":\"event\""));
        assert!(json.contains("\"type\":\"token\""));
        let back: WireEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back.session_id, "s1");
    }

    #[test]
    fn approval_response_deserializes() {
        let json = r#"{"v":1,"session_id":"s1","id":"c1","kind":"approval_response","decision":"approve"}"#;
        let env: WireEnvelope = serde_json::from_str(json).unwrap();
        match env.body {
            WireBody::ApprovalResponse { decision } => {
                assert!(matches!(decision, WireDecision::Approve));
            }
            _ => panic!("wrong body"),
        }
        assert_eq!(env.id.as_deref(), Some("c1"));
    }

    #[test]
    fn approval_event_maps_to_none() {
        use agent_policy::ApprovalRequest;
        use agent_tools::{Access, ToolIntent};
        let req = ApprovalRequest {
            intent: ToolIntent { tool: "x".into(), access: Access::Write, paths: vec![],
                command: None, summary: "s".into() },
            display: None,
        };
        assert!(wire_event_from(AgentEvent::Approval(req)).is_none());
    }

    #[test]
    fn settings_get_round_trips() {
        let env = WireEnvelope {
            v: PROTOCOL_VERSION, session_id: "s".into(), id: None,
            body: WireBody::SettingsGet,
        };
        let json = serde_json::to_string(&env).unwrap();
        assert!(json.contains("\"kind\":\"settings_get\""));
        let back: WireEnvelope = serde_json::from_str(&json).unwrap();
        assert!(matches!(back.body, WireBody::SettingsGet));
    }

    #[test]
    fn settings_update_carries_a_config() {
        use agent_runtime_config::RuntimeConfig;
        let cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://x".into(), "m".into(), "native".into(), 8192);
        let env = WireEnvelope {
            v: PROTOCOL_VERSION, session_id: "s".into(), id: None,
            body: WireBody::SettingsUpdate { settings: cfg.clone() },
        };
        let json = serde_json::to_string(&env).unwrap();
        assert!(json.contains("\"kind\":\"settings_update\""));
        let back: WireEnvelope = serde_json::from_str(&json).unwrap();
        match back.body {
            WireBody::SettingsUpdate { settings } => assert_eq!(settings, cfg),
            _ => panic!("wrong body"),
        }
    }

    #[test]
    fn reasoning_event_maps_to_wire() {
        let payload = wire_event_from(AgentEvent::Reasoning("thinking".into())).unwrap();
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"type\":\"reasoning\""));
        assert!(json.contains("thinking"));
    }

    #[test]
    fn usage_event_maps_to_wire_and_serializes() {
        let payload = wire_event_from(AgentEvent::Usage {
            prompt_tokens: 1234, context_limit: 128_000, turn: 2, max_turns: 20,
        }).unwrap();
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"type\":\"usage\""));
        assert!(json.contains("\"prompt_tokens\":1234"));
        assert!(json.contains("\"context_limit\":128000"));
        assert!(json.contains("\"turn\":2"));
        assert!(json.contains("\"max_turns\":20"));
    }

    #[test]
    fn settings_state_and_error_serialize() {
        use agent_runtime_config::RuntimeConfig;
        let cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://x".into(), "m".into(), "native".into(), 8192);
        let state = WireEnvelope {
            v: PROTOCOL_VERSION, session_id: "s".into(), id: None,
            body: WireBody::SettingsState {
                settings: cfg, workspace: "/w".into(), api_key_set: true,
                hard_floor: vec!["sudo".into()],
                discovered_skills: vec![crate::wire::DiscoveredSkill {
                    name: "greeter".into(), description: "says hi".into() }] },
        };
        let j = serde_json::to_string(&state).unwrap();
        assert!(j.contains("\"kind\":\"settings_state\""));
        assert!(j.contains("\"api_key_set\":true"));
        assert!(j.contains("\"discovered_skills\""));
        assert!(j.contains("greeter"));
        let back: WireEnvelope = serde_json::from_str(&j).unwrap();
        assert!(matches!(back.body, WireBody::SettingsState { .. }));

        let err = WireEnvelope {
            v: PROTOCOL_VERSION, session_id: "s".into(), id: None,
            body: WireBody::SettingsError { message: "bad".into() },
        };
        let j = serde_json::to_string(&err).unwrap();
        assert!(j.contains("\"kind\":\"settings_error\""));
        let back: WireEnvelope = serde_json::from_str(&j).unwrap();
        assert!(matches!(back.body, WireBody::SettingsError { .. }));
    }
}
