use crate::sink::EventSlot;
use crate::wire::ServerEvent;
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::oneshot;

/// `ApprovalChannel` that emits an `ApprovalRequest` over the live-read event
/// slot and awaits an `approve` command matched by correlation id. A timeout or
/// an absent channel resolves to `Deny` (safe default).
pub struct IpcApprovalChannel {
    slot: EventSlot,
    pending: Mutex<HashMap<String, oneshot::Sender<ApprovalResponse>>>,
    counter: AtomicU64,
    timeout: Duration,
}

impl IpcApprovalChannel {
    pub fn new(slot: EventSlot, timeout: Duration) -> Self {
        Self {
            slot,
            pending: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(0),
            timeout,
        }
    }

    /// Complete a pending approval, called by the `approve` command.
    pub fn resolve(&self, id: &str, decision: ApprovalResponse) {
        if let Some(tx) = self.pending.lock().unwrap().remove(id) {
            let _ = tx.send(decision);
        }
    }
}

#[async_trait]
impl ApprovalChannel for IpcApprovalChannel {
    async fn request(&self, req: ApprovalRequest) -> ApprovalResponse {
        let id = format!("c{}", self.counter.fetch_add(1, Ordering::Relaxed));
        let (otx, orx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), otx);
        let ev = ServerEvent::ApprovalRequest {
            id: id.clone(),
            summary: req.intent.summary.clone(),
            command: req.intent.command.clone(),
            display: req.display.clone(),
        };
        match self.slot.lock().unwrap().clone() {
            Some(out) => out.send(ev),
            None => {
                self.pending.lock().unwrap().remove(&id);
                return ApprovalResponse::Deny;
            }
        }
        match tokio::time::timeout(self.timeout, orx).await {
            Ok(Ok(resp)) => resp,
            _ => {
                self.pending.lock().unwrap().remove(&id);
                ApprovalResponse::Deny
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{EventOut, ServerEvent};
    use agent_tools::{Access, ToolIntent};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Captured(Mutex<Vec<ServerEvent>>);
    impl EventOut for Captured {
        fn send(&self, ev: ServerEvent) {
            self.0.lock().unwrap().push(ev);
        }
    }
    fn slot_with(out: Arc<Captured>) -> crate::sink::EventSlot {
        Arc::new(Mutex::new(Some(out as Arc<dyn EventOut>)))
    }
    fn req() -> ApprovalRequest {
        ApprovalRequest {
            intent: ToolIntent {
                tool: "execute_command".into(),
                access: Access::Write,
                paths: vec![],
                command: Some("touch x".into()),
                summary: "run touch x".into(),
            },
            display: None,
            origin: None,
        }
    }

    #[tokio::test]
    async fn emits_request_and_resolves() {
        let cap = Arc::new(Captured::default());
        let ch = Arc::new(IpcApprovalChannel::new(
            slot_with(cap.clone()),
            Duration::from_secs(5),
        ));
        let ch2 = ch.clone();
        let h = tokio::spawn(async move { ch2.request(req()).await });
        // Spin until the request frame appears, then pull its id.
        let id = loop {
            if let Some(ServerEvent::ApprovalRequest { id, .. }) = cap.0.lock().unwrap().first() {
                break id.clone();
            }
            tokio::task::yield_now().await;
        };
        ch.resolve(&id, ApprovalResponse::ApproveAlways);
        assert_eq!(h.await.unwrap(), ApprovalResponse::ApproveAlways);
    }

    #[tokio::test]
    async fn times_out_to_deny() {
        let cap = Arc::new(Captured::default());
        let ch = IpcApprovalChannel::new(slot_with(cap), Duration::from_millis(20));
        assert_eq!(ch.request(req()).await, ApprovalResponse::Deny);
    }

    #[tokio::test]
    async fn absent_slot_denies() {
        let slot: crate::sink::EventSlot = Arc::new(Mutex::new(None));
        let ch = IpcApprovalChannel::new(slot, Duration::from_secs(5));
        assert_eq!(ch.request(req()).await, ApprovalResponse::Deny);
    }
}
