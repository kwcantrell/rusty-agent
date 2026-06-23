use agent_core::AgentEvent;
use agent_model::StopReason;
use agent_policy::ApprovalResponse;
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
    Presence { online: bool },
    UserInput { text: String },
    ApprovalResponse { decision: WireDecision },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireEvent {
    Token { text: String },
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
}
