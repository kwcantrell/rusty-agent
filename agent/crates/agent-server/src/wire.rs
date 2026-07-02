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
    Token {
        text: String,
    },
    Reasoning {
        text: String,
    },
    Usage {
        prompt_tokens: usize,
        context_limit: usize,
        turn: usize,
        max_turns: usize,
    },
    /// Faithful server-reported token totals for the completed turn; the web
    /// Context Explorer uses this as ground truth for the prompt-token chart.
    ServerUsage {
        prompt_tokens: u32,
        completion_tokens: u32,
        turn: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_tokens: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cached_tokens: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
        #[serde(default)]
        turn_duration_ms: u64,
    },
    ToolStart {
        id: String,
        name: String,
        args: serde_json::Value,
    },
    ToolResult {
        id: String,
        name: String,
        status: String,
        duration_ms: u64,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<Display>,
    },
    Error {
        message: String,
    },
    Done {
        reason: String,
    },
    ApprovalRequest {
        id: String,
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<Display>,
    },
    SandboxDegraded {
        mechanism: String,
        reason: String,
    },
    /// Context-curation telemetry (offload/compaction), forwarded from
    /// `AgentEvent::Context`. `kind` discriminates the payload in `detail`.
    Context {
        kind: String,
        detail: serde_json::Value,
    },
    /// Cumulative per-session counters, pushed once per completed run so an
    /// attached client needs no poll.
    SessionStats {
        stats: agent_core::SessionStats,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_degraded: Option<SandboxDegraded>,
}

/// Degraded-sandbox posture carried in `SettingsState` (connect-time) and as a
/// streamed `ServerEvent` (run-start). Present only when isolation was requested
/// but not delivered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SandboxDegraded {
    pub mechanism: String,
    pub reason: String,
}

/// Extract the degraded posture from a sandbox descriptor, if any. Pure so the
/// daemon's `settings_state()` stays trivial and this stays unit-testable.
pub fn sandbox_degraded_from(d: agent_tools::SandboxDescriptor) -> Option<SandboxDegraded> {
    d.degraded.map(|reason| SandboxDegraded {
        mechanism: d.mechanism.to_string(),
        reason,
    })
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
pub enum Decision {
    Approve,
    ApproveAlways,
    Deny,
}

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
        StopReason::Error => "error",
    }
}

/// Map a core `AgentEvent` to its wire form. `Approval` returns `None` — the
/// approval channel emits its own `ApprovalRequest` (mirrors the CLI sink).
pub fn server_event_from(event: AgentEvent) -> Option<ServerEvent> {
    Some(match event {
        AgentEvent::Token(t) => ServerEvent::Token { text: t },
        AgentEvent::Reasoning(t) => ServerEvent::Reasoning { text: t },
        AgentEvent::Usage {
            prompt_tokens,
            context_limit,
            turn,
            max_turns,
        } => ServerEvent::Usage {
            prompt_tokens,
            context_limit,
            turn,
            max_turns,
        },
        AgentEvent::ToolStart { id, name, args } => ServerEvent::ToolStart { id, name, args },
        AgentEvent::ToolResult {
            id,
            name,
            status,
            output,
            duration_ms,
        } => ServerEvent::ToolResult {
            id,
            name,
            status: status.as_str().into(),
            duration_ms,
            content: output.content,
            display: output.display,
        },
        AgentEvent::Error(m) => ServerEvent::Error { message: m },
        AgentEvent::Done(r) => ServerEvent::Done {
            reason: stop_reason_str(&r).into(),
        },
        AgentEvent::Approval(_) => return None,
        AgentEvent::Context(c) => {
            use agent_core::ContextEvent as CE;
            let (kind, detail) = match c {
                CE::Offloaded { id, bytes, tool } => (
                    "offloaded",
                    serde_json::json!({"id": id, "bytes": bytes, "tool": tool}),
                ),
                CE::Compacted {
                    turns_replaced,
                    tokens_before,
                    tokens_after,
                } => (
                    "compacted",
                    serde_json::json!({"turns_replaced": turns_replaced,
                        "tokens_before": tokens_before, "tokens_after": tokens_after}),
                ),
                CE::CompactionFailed { reason } => {
                    ("compaction_failed", serde_json::json!({"reason": reason}))
                }
                CE::Evicted {
                    messages,
                    est_tokens,
                } => (
                    "evicted",
                    serde_json::json!({"messages": messages, "est_tokens": est_tokens}),
                ),
                CE::OverflowRecovery => ("overflow_recovery", serde_json::json!({})),
            };
            ServerEvent::Context {
                kind: kind.into(),
                detail,
            }
        }
        AgentEvent::ServerUsage {
            prompt_tokens,
            completion_tokens,
            reasoning_tokens,
            cached_tokens,
            cost_usd,
            turn_duration_ms,
            turn,
        } => ServerEvent::ServerUsage {
            prompt_tokens,
            completion_tokens,
            turn,
            reasoning_tokens,
            cached_tokens,
            cost_usd,
            turn_duration_ms,
        },
        AgentEvent::SandboxDegraded { mechanism, reason } => ServerEvent::SandboxDegraded {
            mechanism: mechanism.to_string(),
            reason,
        },
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
            intent: ToolIntent {
                tool: "x".into(),
                access: Access::Write,
                paths: vec![],
                command: None,
                summary: "s".into(),
            },
            display: None,
        };
        assert!(server_event_from(AgentEvent::Approval(req)).is_none());
        let ar = ServerEvent::ApprovalRequest {
            id: "c0".into(),
            summary: "s".into(),
            command: None,
            display: None,
        };
        let j = serde_json::to_string(&ar).unwrap();
        assert!(j.contains(r#""type":"approval_request""#));
        assert!(j.contains(r#""id":"c0""#));
    }

    #[test]
    fn server_usage_serializes_with_type_tag() {
        let ev = server_event_from(AgentEvent::ServerUsage {
            prompt_tokens: 42,
            completion_tokens: 7,
            reasoning_tokens: None,
            cached_tokens: None,
            cost_usd: None,
            turn_duration_ms: 1234,
            turn: 3,
        })
        .unwrap();
        let j = serde_json::to_string(&ev).unwrap();
        assert!(
            j.contains(r#""type":"server_usage""#),
            "missing type tag: {j}"
        );
        assert!(
            j.contains(r#""prompt_tokens":42"#),
            "missing prompt_tokens: {j}"
        );
        assert!(
            j.contains(r#""completion_tokens":7"#),
            "missing completion_tokens: {j}"
        );
        assert!(j.contains(r#""turn":3"#), "missing turn: {j}");
        assert!(
            j.contains(r#""turn_duration_ms":1234"#),
            "missing turn_duration_ms: {j}"
        );
        // None-valued optionals are omitted from the wire form entirely.
        assert!(
            !j.contains("reasoning_tokens"),
            "None optionals must be skipped: {j}"
        );
        assert!(
            !j.contains("cached_tokens"),
            "None optionals must be skipped: {j}"
        );
        assert!(
            !j.contains("cost_usd"),
            "None optionals must be skipped: {j}"
        );
    }

    #[test]
    fn tool_result_carries_id_status_and_duration() {
        use agent_core::ToolStatus;
        use agent_tools::ToolOutput;
        let ev = server_event_from(AgentEvent::ToolResult {
            id: "c9".into(),
            name: "read_file".into(),
            status: ToolStatus::Denied,
            output: ToolOutput {
                content: "ERROR: nope".into(),
                display: None,
            },
            duration_ms: 0,
        })
        .unwrap();
        let j = serde_json::to_string(&ev).unwrap();
        assert!(
            j.contains(r#""type":"tool_result""#),
            "missing type tag: {j}"
        );
        assert!(j.contains(r#""id":"c9""#), "missing id: {j}");
        assert!(
            j.contains(r#""status":"denied""#),
            "missing snake_case status: {j}"
        );
        assert!(j.contains(r#""duration_ms":0"#), "missing duration_ms: {j}");
        assert!(
            j.contains(r#""content":"ERROR: nope""#),
            "missing content: {j}"
        );
    }

    #[test]
    fn context_events_are_forwarded() {
        use agent_core::ContextEvent;
        for (ev, kind) in [
            (
                ContextEvent::Offloaded {
                    id: 4,
                    bytes: 2048,
                    tool: "read_file".into(),
                },
                "offloaded",
            ),
            (
                ContextEvent::Compacted {
                    turns_replaced: 3,
                    tokens_before: 900,
                    tokens_after: 200,
                },
                "compacted",
            ),
            (
                ContextEvent::CompactionFailed {
                    reason: "model err".into(),
                },
                "compaction_failed",
            ),
            (ContextEvent::OverflowRecovery, "overflow_recovery"),
        ] {
            let out = server_event_from(AgentEvent::Context(ev)).expect("must forward");
            let j = serde_json::to_value(&out).unwrap();
            assert_eq!(j["type"], "context");
            assert_eq!(j["kind"], kind);
        }
    }

    #[test]
    fn evicted_context_event_maps_to_wire() {
        let ev = AgentEvent::Context(agent_core::ContextEvent::Evicted {
            messages: 7,
            est_tokens: 1234,
        });
        let se = server_event_from(ev).expect("mapped");
        let js = serde_json::to_value(&se).unwrap();
        assert_eq!(js["kind"], "evicted");
        assert_eq!(js["detail"]["messages"], 7);
        assert_eq!(js["detail"]["est_tokens"], 1234);
    }

    #[test]
    fn tool_result_wire_carries_status_and_duration() {
        let out = server_event_from(AgentEvent::ToolResult {
            id: "c1".into(),
            name: "t".into(),
            status: agent_core::ToolStatus::Timeout,
            output: agent_tools::ToolOutput {
                content: "e".into(),
                display: None,
            },
            duration_ms: 60000,
        })
        .unwrap();
        let j = serde_json::to_value(&out).unwrap();
        assert_eq!(j["type"], "tool_result");
        assert_eq!(j["id"], "c1");
        assert_eq!(j["status"], "timeout");
        assert_eq!(j["duration_ms"], 60000);
    }

    #[test]
    fn stop_reason_error_maps_to_error() {
        assert_eq!(stop_reason_str(&StopReason::Error), "error");
    }

    #[test]
    fn done_uses_stop_reason_string() {
        let ev = server_event_from(AgentEvent::Done(StopReason::Cancelled)).unwrap();
        assert_eq!(
            serde_json::to_string(&ev).unwrap(),
            r#"{"type":"done","reason":"cancelled"}"#
        );
    }

    #[test]
    fn decision_into_response() {
        assert_eq!(
            ApprovalResponse::from(Decision::ApproveAlways),
            ApprovalResponse::ApproveAlways
        );
    }

    #[test]
    fn sandbox_degraded_event_serializes_with_type_tag() {
        let ev = server_event_from(AgentEvent::SandboxDegraded {
            mechanism: "docker",
            reason: "no daemon".into(),
        })
        .unwrap();
        let j = serde_json::to_string(&ev).unwrap();
        assert!(
            j.contains(r#""type":"sandbox_degraded""#),
            "missing type tag: {j}"
        );
        assert!(
            j.contains(r#""mechanism":"docker""#),
            "missing mechanism: {j}"
        );
        assert!(j.contains(r#""reason":"no daemon""#), "missing reason: {j}");
    }

    #[test]
    fn sandbox_degraded_from_maps_only_when_degraded() {
        use agent_tools::{Mode, SandboxDescriptor};
        let degraded = SandboxDescriptor {
            mode: Mode::Auto,
            mechanism: "docker",
            image: None,
            network: false,
            degraded: Some("no daemon".into()),
        };
        assert_eq!(
            sandbox_degraded_from(degraded),
            Some(SandboxDegraded {
                mechanism: "docker".into(),
                reason: "no daemon".into()
            })
        );

        let healthy = SandboxDescriptor {
            mode: Mode::Off,
            mechanism: "host",
            image: None,
            network: true,
            degraded: None,
        };
        assert_eq!(sandbox_degraded_from(healthy), None);
    }
}
