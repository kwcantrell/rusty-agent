use crate::wire::{WireBody, WireEnvelope, PROTOCOL_VERSION};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// `ApprovalChannel` that sends an `approval_request` frame and awaits an
/// `approval_response` matched by correlation id. A disconnect/timeout
/// resolves to `Deny` (safe default).
pub struct WsApprovalChannel {
    tx: mpsc::UnboundedSender<WireEnvelope>,
    session: Arc<Mutex<String>>,
    pending: Mutex<HashMap<String, oneshot::Sender<ApprovalResponse>>>,
    counter: AtomicU64,
    timeout: Duration,
}

impl WsApprovalChannel {
    pub fn new(
        tx: mpsc::UnboundedSender<WireEnvelope>,
        session: Arc<Mutex<String>>,
        timeout: Duration,
    ) -> Self {
        Self { tx, session, pending: Mutex::new(HashMap::new()), counter: AtomicU64::new(0), timeout }
    }

    /// Complete a pending approval, called by the daemon read loop.
    pub fn resolve(&self, id: &str, decision: ApprovalResponse) {
        if let Some(tx) = self.pending.lock().unwrap().remove(id) {
            let _ = tx.send(decision);
        }
    }
}

#[async_trait]
impl ApprovalChannel for WsApprovalChannel {
    async fn request(&self, req: ApprovalRequest) -> ApprovalResponse {
        let id = format!("c{}", self.counter.fetch_add(1, Ordering::Relaxed));
        let (otx, orx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), otx);
        let env = WireEnvelope {
            v: PROTOCOL_VERSION,
            session_id: self.session.lock().unwrap().clone(),
            id: Some(id.clone()),
            body: WireBody::ApprovalRequest {
                summary: req.intent.summary.clone(),
                command: req.intent.command.clone(),
                display: req.display.clone(),
            },
        };
        if self.tx.send(env).is_err() {
            self.pending.lock().unwrap().remove(&id);
            return ApprovalResponse::Deny;
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
    use agent_tools::{Access, ToolIntent};

    fn req() -> ApprovalRequest {
        ApprovalRequest {
            intent: ToolIntent { tool: "execute_command".into(), access: Access::Write,
                paths: vec![], command: Some("touch x".into()), summary: "run touch x".into() },
            display: None,
        }
    }

    #[tokio::test]
    async fn resolves_when_response_arrives() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let ch = Arc::new(WsApprovalChannel::new(tx, Arc::new(Mutex::new("s".into())),
            Duration::from_secs(5)));
        let ch2 = ch.clone();
        let h = tokio::spawn(async move { ch2.request(req()).await });
        // The channel sent an approval_request; pull its correlation id.
        let env = rx.recv().await.unwrap();
        let id = env.id.clone().unwrap();
        ch.resolve(&id, ApprovalResponse::ApproveAlways);
        assert_eq!(h.await.unwrap(), ApprovalResponse::ApproveAlways);
    }

    #[tokio::test]
    async fn times_out_to_deny() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let ch = WsApprovalChannel::new(tx, Arc::new(Mutex::new("s".into())),
            Duration::from_millis(20));
        assert_eq!(ch.request(req()).await, ApprovalResponse::Deny);
    }
}
