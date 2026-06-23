use crate::error::McpError;
use crate::transport::McpTransport;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

// `McpClient` and helpers are crate-internal; later tasks (manager, etc.) will
// use them via `crate::client::…`. Suppress dead_code until then.
#[allow(dead_code)]
type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, McpError>>>>>;

/// A connected MCP server. One background task reads inbound messages off the
/// transport and routes each response to the waiter registered for its id.
#[allow(dead_code)]
pub struct McpClient {
    transport: Arc<dyn McpTransport>,
    pending: Pending,
    next_id: AtomicU64,
}

#[allow(dead_code)]
impl McpClient {
    pub fn new(transport: Arc<dyn McpTransport>) -> Arc<Self> {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let client = Arc::new(Self {
            transport: transport.clone(),
            pending: pending.clone(),
            next_id: AtomicU64::new(1),
        });
        // Reader loop: route responses by id; on close, fail all waiters.
        tokio::spawn(async move {
            while let Some(msg) = transport.recv().await {
                let Some(id) = msg.get("id").and_then(Value::as_u64) else {
                    continue
                }; // notifications ignored
                if let Some(tx) = pending.lock().unwrap().remove(&id) {
                    let routed = if let Some(err) = msg.get("error") {
                        Err(McpError::Server(
                            err.get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown")
                                .to_string(),
                        ))
                    } else {
                        Ok(msg.get("result").cloned().unwrap_or(Value::Null))
                    };
                    let _ = tx.send(routed);
                }
            }
            // Transport closed: nothing more will arrive — fail everyone waiting.
            for (_, tx) in pending.lock().unwrap().drain() {
                let _ = tx.send(Err(McpError::Closed));
            }
        });
        client
    }

    pub async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let frame = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        if let Err(e) = self.transport.send(frame).await {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(McpError::Closed), // sender dropped
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(McpError::Timeout)
            }
        }
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<(), McpError> {
        self.transport
            .send(json!({"jsonrpc":"2.0","method":method,"params":params}))
            .await
    }

    pub async fn close(&self) {
        self.transport.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::MockTransport;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn request_resolves_with_matching_response_by_id() {
        // Mock echoes the request id back in a result envelope.
        let t = MockTransport::scripted(|req| {
            let id = req["id"].clone();
            vec![json!({"jsonrpc":"2.0","id":id,"result":{"ok":true}})]
        });
        let client = McpClient::new(Arc::new(t));
        let res = client
            .request("ping", json!({}), Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(res["ok"], true);
    }

    #[tokio::test]
    async fn request_surfaces_server_error_envelope() {
        let t = MockTransport::scripted(|req| {
            let id = req["id"].clone();
            vec![json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"nope"}})]
        });
        let client = McpClient::new(Arc::new(t));
        let err = client
            .request("ping", json!({}), Duration::from_secs(2))
            .await
            .unwrap_err();
        assert!(matches!(err, McpError::Server(m) if m.contains("nope")));
    }

    #[tokio::test]
    async fn request_times_out_when_no_response() {
        let t = MockTransport::scripted(|_| vec![]); // never replies
        let client = McpClient::new(Arc::new(t));
        let err = client
            .request("ping", json!({}), Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(matches!(err, McpError::Timeout));
    }
}
