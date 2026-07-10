use crate::wire::{server_event_from, EventOut};
use agent_core::{AgentEvent, EventSink};
use std::sync::{Arc, Mutex};

/// The live-read outbound slot, swapped by the `subscribe` command. Reading it
/// per-emit (not snapshotting) lets a re-subscribe redirect an in-flight run.
pub type EventSlot = Arc<Mutex<Option<Arc<dyn EventOut>>>>;

/// `EventSink` that maps core events to `ServerEvent` and forwards them to the
/// currently-registered `EventOut`. Absent slot → drop (infallible emit).
pub struct ChannelEventSink {
    slot: EventSlot,
}

impl ChannelEventSink {
    pub fn new(slot: EventSlot) -> Self {
        Self { slot }
    }
}

impl EventSink for ChannelEventSink {
    fn emit(&self, event: AgentEvent) {
        let Some(ev) = server_event_from(event) else {
            return;
        };
        if let Some(out) = self.slot.lock().unwrap().clone() {
            out.send(ev);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::ServerEvent;
    use agent_core::{AgentEvent, EventSink};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Captured(Mutex<Vec<ServerEvent>>);
    impl crate::wire::EventOut for Captured {
        fn send(&self, ev: ServerEvent) {
            self.0.lock().unwrap().push(ev);
        }
    }

    fn slot_with(out: Arc<Captured>) -> EventSlot {
        Arc::new(Mutex::new(Some(out as Arc<dyn crate::wire::EventOut>)))
    }

    #[test]
    fn token_is_forwarded_as_server_event() {
        let cap = Arc::new(Captured::default());
        let sink = ChannelEventSink::new(slot_with(cap.clone()));
        sink.emit(AgentEvent::Token("hello".into()));
        let got = cap.0.lock().unwrap();
        assert!(matches!(got.as_slice(), [ServerEvent::Token { text }] if text == "hello"));
    }

    #[test]
    fn approval_event_is_not_forwarded() {
        use agent_policy::ApprovalRequest;
        use agent_tools::{Access, ToolIntent};
        let cap = Arc::new(Captured::default());
        let sink = ChannelEventSink::new(slot_with(cap.clone()));
        sink.emit(AgentEvent::Approval(ApprovalRequest {
            intent: ToolIntent {
                tool: "x".into(),
                access: Access::Read,
                paths: vec![],
                command: None,
                summary: "s".into(),
            },
            display: None,
            origin: None,
        }));
        assert!(cap.0.lock().unwrap().is_empty());
    }

    #[test]
    fn emit_with_empty_slot_is_a_noop() {
        let slot: EventSlot = Arc::new(Mutex::new(None));
        let sink = ChannelEventSink::new(slot);
        sink.emit(AgentEvent::Token("x".into())); // must not panic
    }

    #[test]
    fn relinks_to_a_new_out_live() {
        let first = Arc::new(Captured::default());
        let slot = slot_with(first.clone());
        let sink = ChannelEventSink::new(slot.clone());
        sink.emit(AgentEvent::Token("a".into()));
        let second = Arc::new(Captured::default());
        *slot.lock().unwrap() = Some(second.clone() as Arc<dyn crate::wire::EventOut>);
        sink.emit(AgentEvent::Token("b".into()));
        assert_eq!(first.0.lock().unwrap().len(), 1);
        assert_eq!(second.0.lock().unwrap().len(), 1);
    }
}
