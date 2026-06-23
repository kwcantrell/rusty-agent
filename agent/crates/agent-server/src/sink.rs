use crate::wire::{wire_event_from, WireBody, WireEnvelope, PROTOCOL_VERSION};
use agent_core::{AgentEvent, EventSink};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// `EventSink` that serialises events as `WireEnvelope`s onto a channel.
/// `emit` is synchronous (core requirement); a writer task drains the channel.
pub struct WsEventSink {
    tx: mpsc::UnboundedSender<WireEnvelope>,
    session: Arc<Mutex<String>>,
}

impl WsEventSink {
    pub fn new(tx: mpsc::UnboundedSender<WireEnvelope>, session: Arc<Mutex<String>>) -> Self {
        Self { tx, session }
    }
}

impl EventSink for WsEventSink {
    fn emit(&self, event: AgentEvent) {
        let Some(payload) = wire_event_from(event) else { return };
        let env = WireEnvelope {
            v: PROTOCOL_VERSION,
            session_id: self.session.lock().unwrap().clone(),
            id: None,
            body: WireBody::Event { payload },
        };
        let _ = self.tx.send(env);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_token_envelope_with_session() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let session = Arc::new(Mutex::new("sess-1".to_string()));
        let sink = WsEventSink::new(tx, session);
        sink.emit(AgentEvent::Token("hello".into()));
        let env = rx.try_recv().expect("one envelope");
        assert_eq!(env.session_id, "sess-1");
        assert!(matches!(env.body, WireBody::Event { .. }));
    }

    #[test]
    fn approval_event_is_not_emitted() {
        use agent_policy::ApprovalRequest;
        use agent_tools::{Access, ToolIntent};
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sink = WsEventSink::new(tx, Arc::new(Mutex::new("s".into())));
        sink.emit(AgentEvent::Approval(ApprovalRequest {
            intent: ToolIntent { tool: "x".into(), access: Access::Read, paths: vec![],
                command: None, summary: "s".into() },
            display: None,
        }));
        assert!(rx.try_recv().is_err());
    }
}
