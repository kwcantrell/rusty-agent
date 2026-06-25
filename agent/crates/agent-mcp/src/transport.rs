use crate::config::McpServerSpec;
use crate::error::McpError;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::mpsc;
use tokio::sync::Mutex as AsyncMutex;

/// One JSON-RPC message in / out. `recv` yields `None` when the peer closes.
#[async_trait]
pub trait McpTransport: Send + Sync {
    async fn send(&self, msg: Value) -> Result<(), McpError>;
    async fn recv(&self) -> Option<Value>;
    /// Terminate the underlying process/connection. Idempotent.
    async fn close(&self);
}

/// stdio transport: spawn a child and speak newline-delimited JSON over its
/// stdin/stdout. A reader task parses stdout lines onto an mpsc; `recv` drains it.
pub struct StdioTransport {
    stdin: AsyncMutex<ChildStdin>,
    inbound: AsyncMutex<mpsc::UnboundedReceiver<Value>>,
    child: Mutex<Option<agent_tools::SandboxedChild>>,
    reader: Mutex<Option<tokio::task::JoinHandle<()>>>,
    stderr: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl StdioTransport {
    pub fn spawn(
        spec: &McpServerSpec,
        sandbox: &std::sync::Arc<dyn agent_tools::SandboxStrategy>,
    ) -> Result<Self, McpError> {
        let cspec = agent_tools::CommandSpec {
            program: spec.command.clone(),
            args: spec.args.clone(),
            cwd: std::env::current_dir().unwrap_or_else(|_| ".".into()),
            env: spec.env.clone().into_iter().collect(),
            kind: agent_tools::ProcKind::Service,
        };
        let mut child = sandbox.launch(cspec).map_err(|e| McpError::Io(e.to_string()))?;
        let stdin = child.take_stdin().ok_or_else(|| McpError::Io("no stdin".into()))?;
        let stdout = child.take_stdout().ok_or_else(|| McpError::Io("no stdout".into()))?;
        let stderr_handle = child.take_stderr().map(|stderr| {
            // Drain server diagnostics to tracing so they never block the pipe.
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    tracing::debug!(target: "mcp.server", "{l}");
                }
            })
        });
        let (tx, rx) = mpsc::unbounded_channel();
        let reader_handle = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(&line) {
                    Ok(v) => {
                        if tx.send(v).is_err() {
                            break;
                        }
                    }
                    Err(e) => tracing::warn!(target: "mcp", error=%e, "non-JSON line from server"),
                }
            }
        });
        Ok(Self {
            stdin: AsyncMutex::new(stdin),
            inbound: AsyncMutex::new(rx),
            child: Mutex::new(Some(child)),
            reader: Mutex::new(Some(reader_handle)),
            stderr: Mutex::new(stderr_handle),
        })
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn send(&self, msg: Value) -> Result<(), McpError> {
        let mut line = serde_json::to_string(&msg).map_err(|e| McpError::Protocol(e.to_string()))?;
        line.push('\n');
        let mut w = self.stdin.lock().await;
        w.write_all(line.as_bytes()).await.map_err(|e| McpError::Io(e.to_string()))?;
        w.flush().await.map_err(|e| McpError::Io(e.to_string()))
    }

    async fn recv(&self) -> Option<Value> {
        self.inbound.lock().await.recv().await
    }

    async fn close(&self) {
        // Take child out of the Mutex first so the guard drops before the await.
        let child = self.child.lock().unwrap().take();
        if let Some(mut c) = child {
            c.kill().await;
        }
        // Deterministically tear down the reader/stderr tasks (don't wait for EOF).
        if let Some(h) = self.reader.lock().unwrap().take() { h.abort(); }
        if let Some(h) = self.stderr.lock().unwrap().take() { h.abort(); }
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // SandboxedChild's own Drop handles teardown — just drop it.
        let _ = self.child.lock().unwrap().take();
        // Abort the reader/stderr tasks (abort is sync — safe in Drop).
        if let Some(h) = self.reader.lock().unwrap().take() { h.abort(); }
        if let Some(h) = self.stderr.lock().unwrap().take() { h.abort(); }
    }
}

/// A scripted in-memory transport for hermetic client tests. The `responder`
/// closure is called with each outbound message and returns zero or more reply
/// messages to enqueue (it can echo the request `id`).
#[cfg(test)]
type Responder = Box<dyn Fn(&Value) -> Vec<Value> + Send + Sync>;

#[cfg(test)]
#[allow(dead_code)]
pub(crate) struct MockTransport {
    responder: Responder,
    inbound: AsyncMutex<mpsc::UnboundedReceiver<Value>>,
    tx: mpsc::UnboundedSender<Value>,
}

#[cfg(test)]
impl MockTransport {
    #[allow(dead_code)]
    pub(crate) fn scripted(
        responder: impl Fn(&Value) -> Vec<Value> + Send + Sync + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { responder: Box::new(responder), inbound: AsyncMutex::new(rx), tx }
    }
}

#[cfg(test)]
#[async_trait]
impl McpTransport for MockTransport {
    async fn send(&self, msg: Value) -> Result<(), McpError> {
        for reply in (self.responder)(&msg) {
            let _ = self.tx.send(reply);
        }
        Ok(())
    }
    async fn recv(&self) -> Option<Value> {
        self.inbound.lock().await.recv().await
    }
    async fn close(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::McpServerSpec;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn cat_spec() -> McpServerSpec {
        McpServerSpec {
            command: "cat".into(),
            args: vec![],
            env: BTreeMap::new(),
            trust: crate::config::Trust::Ask,
        }
    }

    fn host_sandbox() -> std::sync::Arc<dyn agent_tools::SandboxStrategy> {
        std::sync::Arc::new(agent_tools::HostExecutor)
    }

    #[tokio::test]
    async fn stdio_roundtrips_newline_delimited_json_via_cat() {
        let sandbox = host_sandbox();
        let t = StdioTransport::spawn(&cat_spec(), &sandbox).expect("spawn cat");
        t.send(json!({"jsonrpc":"2.0","id":1,"method":"ping"})).await.unwrap();
        let got = t.recv().await.expect("a message");
        assert_eq!(got["id"], 1);
        assert_eq!(got["method"], "ping");
        t.close().await;
    }

    #[tokio::test]
    async fn close_tears_down_the_reader_task() {
        let sandbox = host_sandbox();
        let t = StdioTransport::spawn(&cat_spec(), &sandbox).expect("spawn cat");
        // Sanity: the transport works before teardown.
        t.send(json!({"jsonrpc":"2.0","id":1,"method":"ping"})).await.unwrap();
        let _ = t.recv().await.expect("a message");

        t.close().await;

        // After close, the reader task is gone (its tx dropped) -> recv yields None.
        let after = tokio::time::timeout(std::time::Duration::from_secs(5), t.recv())
            .await
            .expect("recv must resolve promptly after close, not hang");
        assert!(after.is_none(), "recv after close should be None, got: {after:?}");
    }

    #[tokio::test]
    async fn close_is_idempotent() {
        let sandbox = host_sandbox();
        let t = StdioTransport::spawn(&cat_spec(), &sandbox).expect("spawn cat");
        t.close().await;
        t.close().await; // must not panic
    }
}
