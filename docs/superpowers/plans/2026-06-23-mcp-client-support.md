# MCP Client Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the runtime consume tools from external MCP servers over stdio, surfacing each as an ordinary `Tool` in `ToolRegistry`, with zero changes to the four core crates.

**Architecture:** A new `agent-mcp` crate speaks JSON-RPC 2.0 over a child process's stdio (hand-rolled, actor-pattern client). Each discovered MCP tool is wrapped in an `McpTool: Tool` namespaced `server__tool`; per-server trust is encoded onto the existing policy's Read/Write axis so no policy change is needed. The CLI and daemon connect configured servers at startup and register the wrapped tools.

**Tech Stack:** Rust (edition 2021), tokio, serde/serde_json, async-trait, thiserror, tracing. Tests use tokio + the workspace `tempfile` dev-dep; one hermetic transport test shells out to `cat`.

**Spec:** `docs/superpowers/specs/2026-06-23-mcp-client-support-design.md`

## Global Constraints

- **Zero core change:** do not modify `agent-core`, `agent-model`, `agent-tools`, or `agent-policy`. MCP attaches only through the existing `Tool`/`ToolRegistry` seam. Allowed edits outside `agent-mcp`: `agent-runtime-config`, `agent-cli`, `agent-server` (wiring only).
- **cargo not on PATH:** run `source "$HOME/.cargo/env"` before any cargo command. Build/test from `agent/`.
- **Green bar each commit:** `cargo test --workspace` passes and `cargo clippy --all-targets -- -D warnings` is clean before every commit.
- **Workspace deps:** reuse versions already pinned in `agent/Cargo.toml [workspace.dependencies]` via `{ workspace = true }`. Do not introduce new third-party crates (no `rmcp`).
- **Transports:** stdio only. Keep the `McpTransport` trait as the seam; do not implement HTTP.
- **Surface:** MCP **tools** only. No resources, no prompts.
- **Naming:** wrapped tool names are `server__tool`, sanitized to `[a-zA-Z0-9_-]`.
- **Trust default:** `ask`. `ask` → `intent()` returns `Access::Write`; `allow` → `Access::Read` with empty `paths`. Never set `command` on an MCP intent.
- **All paths below are relative to `/home/kalen/rust-agent-runtime/agent/` unless noted.**

## File Structure

| File | Responsibility |
|---|---|
| `crates/agent-mcp/Cargo.toml` | Crate manifest; deps + dev-deps. |
| `crates/agent-mcp/src/lib.rs` | Module wiring + public re-exports. |
| `crates/agent-mcp/src/error.rs` | `McpError`. |
| `crates/agent-mcp/src/config.rs` | `McpServersConfig`, `McpServerSpec`, `Trust`, file loading. |
| `crates/agent-mcp/src/transport.rs` | `McpTransport` trait, `StdioTransport`, and (test-only) `MockTransport`. |
| `crates/agent-mcp/src/client.rs` | `McpClient` actor: handshake, `tools/list`, `tools/call`, id correlation. |
| `crates/agent-mcp/src/tool.rs` | `McpTool: Tool`, name sanitization, schema translation, result normalization. |
| `crates/agent-mcp/src/manager.rs` | `McpManager`, `ServerStatus`, concurrent connect, summary, shutdown. |
| `crates/agent-runtime-config/src/lib.rs` | add `connect_mcp(path) -> McpManager` helper. |
| `crates/agent-cli/src/main.rs` | `--mcp-config` flag; register MCP tools; print summary; hold manager. |
| `crates/agent-server/src/main.rs` | `--mcp-config` flag; connect once; pass tools into `DaemonParams`. |
| `crates/agent-server/src/daemon.rs` | thread `mcp_tools` field through `DaemonParams`. |
| `crates/agent-server/src/runtime.rs` | thread `mcp_tools` into `RuntimeState::new` + `build_loop`. |
| `mcp.example.json` (repo root or `agent/`) | example config. |
| `agent/docs/RUNNING.md` | how to run with MCP. |
| `docs/superpowers/context/follow-ups.md` | record review findings at branch finish. |

---

### Task 1: Crate skeleton, errors, and config parsing

**Files:**
- Create: `crates/agent-mcp/Cargo.toml`
- Create: `crates/agent-mcp/src/lib.rs`
- Create: `crates/agent-mcp/src/error.rs`
- Create: `crates/agent-mcp/src/config.rs`

**Interfaces:**
- Produces: `McpError` enum; `Trust { Ask, Allow }` (`Default = Ask`); `McpServerSpec { command: String, args: Vec<String>, env: BTreeMap<String,String>, trust: Trust }`; `McpServersConfig { servers: BTreeMap<String, McpServerSpec> }`; `McpServersConfig::load_or_empty(path: &Path) -> (McpServersConfig, Option<String>)`.

- [ ] **Step 1: Create the manifest**

`crates/agent-mcp/Cargo.toml`:
```toml
[package]
name = "agent-mcp"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
agent-tools = { path = "../agent-tools" }
tokio = { workspace = true }
async-trait = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
agent-policy = { path = "../agent-policy" }
tempfile = { workspace = true }
```

- [ ] **Step 2: Write the failing config tests**

`crates/agent-mcp/src/config.rs`:
```rust
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// Per-server trust posture. Drives the policy decision via `McpTool::intent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Trust {
    /// Require approval on every call (third-party code is untrusted).
    #[default]
    Ask,
    /// Auto-allow this server's tools (operator vouches for it).
    Allow,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerSpec {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub trust: Trust,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpServersConfig {
    #[serde(rename = "mcpServers", default)]
    pub servers: BTreeMap<String, McpServerSpec>,
}

impl McpServersConfig {
    /// Load the config file. A missing file yields an empty config with no warning
    /// (MCP is simply not enabled); an unreadable or malformed file yields an empty
    /// config plus an operator warning, never an abort.
    pub fn load_or_empty(path: &Path) -> (Self, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Self>(&text) {
                Ok(cfg) => (cfg, None),
                Err(e) => (Self::default(), Some(format!("malformed mcp config ({e})"))),
            },
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(e) => (Self::default(), Some(format!("mcp config unreadable ({e})"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_server_block_and_defaults_trust_to_ask() {
        let json = r#"{ "mcpServers": {
            "filesystem": { "command": "npx", "args": ["-y", "srv", "/w"], "env": {"K":"V"} },
            "trusted":    { "command": "x", "trust": "allow" }
        }}"#;
        let cfg: McpServersConfig = serde_json::from_str(json).unwrap();
        let fs = &cfg.servers["filesystem"];
        assert_eq!(fs.command, "npx");
        assert_eq!(fs.args, vec!["-y", "srv", "/w"]);
        assert_eq!(fs.env["K"], "V");
        assert_eq!(fs.trust, Trust::Ask, "absent trust defaults to ask");
        assert_eq!(cfg.servers["trusted"].trust, Trust::Allow);
    }

    #[test]
    fn missing_file_is_empty_and_silent() {
        let dir = tempfile::tempdir().unwrap();
        let (cfg, warn) = McpServersConfig::load_or_empty(&dir.path().join("nope.json"));
        assert!(cfg.servers.is_empty());
        assert!(warn.is_none());
    }

    #[test]
    fn malformed_file_is_empty_with_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        let (cfg, warn) = McpServersConfig::load_or_empty(&path);
        assert!(cfg.servers.is_empty());
        assert!(warn.unwrap().contains("malformed"));
    }
}
```

- [ ] **Step 3: Write the error type and lib wiring**

`crates/agent-mcp/src/error.rs`:
```rust
#[derive(Debug, Clone, thiserror::Error)]
pub enum McpError {
    #[error("io error: {0}")]
    Io(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("request timed out")]
    Timeout,
    #[error("server returned error: {0}")]
    Server(String),
    #[error("transport closed")]
    Closed,
}
```

`crates/agent-mcp/src/lib.rs`:
```rust
//! Model Context Protocol (MCP) client: connect to external MCP servers over
//! stdio and surface their tools through the agent's `Tool`/`ToolRegistry` seam.

mod client;
mod config;
mod error;
mod manager;
mod tool;
mod transport;

pub use config::{McpServerSpec, McpServersConfig, Trust};
pub use error::McpError;
pub use manager::{McpManager, ServerStatus};
pub use tool::McpTool;
pub use transport::{McpTransport, StdioTransport};
```

> Note: `lib.rs` references modules created in later tasks. To keep the crate compiling after Task 1, temporarily comment out the `mod client; mod manager; mod tool; mod transport;` lines and their `pub use`s, leaving only `mod config; mod error;` plus their re-exports. Re-enable each as its task lands. (The subagent executing a later task re-enables that module.)

For Task 1, `lib.rs` is:
```rust
//! Model Context Protocol (MCP) client: connect to external MCP servers over
//! stdio and surface their tools through the agent's `Tool`/`ToolRegistry` seam.

mod config;
mod error;

pub use config::{McpServerSpec, McpServersConfig, Trust};
pub use error::McpError;
```

- [ ] **Step 4: Run tests and clippy**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-mcp && cargo clippy -p agent-mcp --all-targets -- -D warnings`
Expected: 3 tests pass; clippy clean.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-mcp/
git commit -m "feat(mcp): agent-mcp crate skeleton with error types and config parsing"
```

---

### Task 2: Transport trait, StdioTransport, and a mock transport

**Files:**
- Create: `crates/agent-mcp/src/transport.rs`
- Modify: `crates/agent-mcp/src/lib.rs` (re-enable `mod transport;` + re-exports)

**Interfaces:**
- Consumes: `McpError` (Task 1), `McpServerSpec` (Task 1).
- Produces: `trait McpTransport: Send + Sync { async fn send(&self, msg: Value) -> Result<(), McpError>; async fn recv(&self) -> Option<Value>; async fn close(&self); }`; `StdioTransport::spawn(spec: &McpServerSpec) -> Result<StdioTransport, McpError>`; `pub(crate) MockTransport` (cfg(test)) with `MockTransport::scripted(responder)`.

- [ ] **Step 1: Write the failing StdioTransport test (hermetic, via `cat`)**

`cat` echoes each stdin line to stdout, so a newline-delimited JSON message sent in comes straight back — a deterministic stand-in for a server, no MCP needed.

`crates/agent-mcp/src/transport.rs` (test module):
```rust
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

    #[tokio::test]
    async fn stdio_roundtrips_newline_delimited_json_via_cat() {
        let t = StdioTransport::spawn(&cat_spec()).expect("spawn cat");
        t.send(json!({"jsonrpc":"2.0","id":1,"method":"ping"})).await.unwrap();
        let got = t.recv().await.expect("a message");
        assert_eq!(got["id"], 1);
        assert_eq!(got["method"], "ping");
        t.close().await;
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-mcp stdio_roundtrips 2>&1 | tail -5`
Expected: FAIL — `StdioTransport` not found.

- [ ] **Step 3: Implement the trait and StdioTransport**

Top of `crates/agent-mcp/src/transport.rs`:
```rust
use crate::config::McpServerSpec;
use crate::error::McpError;
use async_trait::async_trait;
use serde_json::Value;
use std::process::Stdio;
use std::sync::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
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
    child: Mutex<Option<Child>>,
}

impl StdioTransport {
    pub fn spawn(spec: &McpServerSpec) -> Result<Self, McpError> {
        let mut cmd = tokio::process::Command::new(&spec.command);
        cmd.args(&spec.args)
            .envs(&spec.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| McpError::Io(e.to_string()))?;
        let stdin = child.stdin.take().ok_or_else(|| McpError::Io("no stdin".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| McpError::Io("no stdout".into()))?;
        if let Some(stderr) = child.stderr.take() {
            // Drain server diagnostics to tracing so they never block the pipe.
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    tracing::debug!(target: "mcp.server", "{l}");
                }
            });
        }
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
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
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.start_kill();
        }
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.start_kill();
        }
    }
}
```

- [ ] **Step 4: Add the test-only MockTransport**

Append to `crates/agent-mcp/src/transport.rs` (above the `#[cfg(test)] mod tests`):
```rust
/// A scripted in-memory transport for hermetic client tests. The `responder`
/// closure is called with each outbound message and returns zero or more reply
/// messages to enqueue (it can echo the request `id`).
#[cfg(test)]
pub(crate) struct MockTransport {
    responder: Box<dyn Fn(&Value) -> Vec<Value> + Send + Sync>,
    inbound: AsyncMutex<mpsc::UnboundedReceiver<Value>>,
    tx: mpsc::UnboundedSender<Value>,
}

#[cfg(test)]
impl MockTransport {
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
```

- [ ] **Step 5: Re-enable the module and re-exports**

In `crates/agent-mcp/src/lib.rs` add `mod transport;` and `pub use transport::{McpTransport, StdioTransport};`.

- [ ] **Step 6: Run tests and clippy**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-mcp && cargo clippy -p agent-mcp --all-targets -- -D warnings`
Expected: PASS (cat roundtrip + Task 1 config tests); clippy clean.

- [ ] **Step 7: Commit**

```bash
git add agent/crates/agent-mcp/
git commit -m "feat(mcp): McpTransport trait, stdio transport, and scripted mock"
```

---

### Task 3: McpClient actor (request/notify, id correlation, timeout)

**Files:**
- Create: `crates/agent-mcp/src/client.rs`
- Modify: `crates/agent-mcp/src/lib.rs` (add `mod client;`)

**Interfaces:**
- Consumes: `McpTransport` (Task 2), `McpError` (Task 1).
- Produces: `McpClient` with `McpClient::new(transport: Arc<dyn McpTransport>) -> Arc<McpClient>`; `async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, McpError>`; `async fn notify(&self, method: &str, params: Value) -> Result<(), McpError>`; `async fn close(&self)`.

- [ ] **Step 1: Write the failing tests**

`crates/agent-mcp/src/client.rs` (test module):
```rust
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
        let res = client.request("ping", json!({}), Duration::from_secs(2)).await.unwrap();
        assert_eq!(res["ok"], true);
    }

    #[tokio::test]
    async fn request_surfaces_server_error_envelope() {
        let t = MockTransport::scripted(|req| {
            let id = req["id"].clone();
            vec![json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"nope"}})]
        });
        let client = McpClient::new(Arc::new(t));
        let err = client.request("ping", json!({}), Duration::from_secs(2)).await.unwrap_err();
        assert!(matches!(err, McpError::Server(m) if m.contains("nope")));
    }

    #[tokio::test]
    async fn request_times_out_when_no_response() {
        let t = MockTransport::scripted(|_| vec![]); // never replies
        let client = McpClient::new(Arc::new(t));
        let err = client.request("ping", json!({}), Duration::from_millis(50)).await.unwrap_err();
        assert!(matches!(err, McpError::Timeout));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-mcp client:: 2>&1 | tail -5`
Expected: FAIL — `McpClient` not found.

- [ ] **Step 3: Implement the actor**

Top of `crates/agent-mcp/src/client.rs`:
```rust
use crate::error::McpError;
use crate::transport::McpTransport;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, McpError>>>>>;

/// A connected MCP server. One background task reads inbound messages off the
/// transport and routes each response to the waiter registered for its id.
pub struct McpClient {
    transport: Arc<dyn McpTransport>,
    pending: Pending,
    next_id: AtomicU64,
}

impl McpClient {
    pub fn new(transport: Arc<dyn McpTransport>) -> Arc<Self> {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let client = Arc::new(Self { transport: transport.clone(), pending: pending.clone(), next_id: AtomicU64::new(1) });
        // Reader loop: route responses by id; on close, fail all waiters.
        tokio::spawn(async move {
            while let Some(msg) = transport.recv().await {
                let Some(id) = msg.get("id").and_then(Value::as_u64) else { continue }; // notifications ignored
                if let Some(tx) = pending.lock().unwrap().remove(&id) {
                    let routed = if let Some(err) = msg.get("error") {
                        Err(McpError::Server(err.get("message").and_then(Value::as_str).unwrap_or("unknown").to_string()))
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

    pub async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, McpError> {
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
        self.transport.send(json!({"jsonrpc":"2.0","method":method,"params":params})).await
    }

    pub async fn close(&self) {
        self.transport.close().await;
    }
}
```

- [ ] **Step 4: Re-enable the module**

In `crates/agent-mcp/src/lib.rs` add `mod client;` (no public re-export yet — `McpClient` stays crate-internal; the manager exposes tools).

- [ ] **Step 5: Run tests and clippy**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-mcp && cargo clippy -p agent-mcp --all-targets -- -D warnings`
Expected: PASS (3 new client tests); clippy clean.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-mcp/
git commit -m "feat(mcp): McpClient actor with id-correlated request/notify and timeout"
```

---

### Task 4: Handshake and tool discovery

**Files:**
- Modify: `crates/agent-mcp/src/client.rs`

**Interfaces:**
- Consumes: `McpClient` (Task 3).
- Produces: `RawTool { name: String, description: String, input_schema: Value }`; `McpClient::initialize(&self, timeout) -> Result<(), McpError>`; `McpClient::list_tools(&self, timeout) -> Result<Vec<RawTool>, McpError>`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/agent-mcp/src/client.rs`:
```rust
#[tokio::test]
async fn initialize_then_list_tools_parses_descriptors() {
    let t = MockTransport::scripted(|req| {
        let id = req["id"].clone();
        match req["method"].as_str() {
            Some("initialize") => vec![json!({"jsonrpc":"2.0","id":id,
                "result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"mock"}}})],
            Some("tools/list") => vec![json!({"jsonrpc":"2.0","id":id,"result":{"tools":[
                {"name":"read_file","description":"Read a file",
                 "inputSchema":{"type":"object","properties":{"path":{"type":"string"}}}}
            ]}})],
            _ => vec![],
        }
    });
    let client = McpClient::new(Arc::new(t));
    client.initialize(Duration::from_secs(2)).await.unwrap();
    let tools = client.list_tools(Duration::from_secs(2)).await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "read_file");
    assert_eq!(tools[0].input_schema["properties"]["path"]["type"], "string");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-mcp initialize_then 2>&1 | tail -5`
Expected: FAIL — no `initialize`/`list_tools`.

- [ ] **Step 3: Implement handshake + discovery**

Add to `crates/agent-mcp/src/client.rs` (inside `impl McpClient`, and a `RawTool` struct + the protocol-version const at module top):
```rust
/// MCP protocol version we advertise. Servers negotiate down if needed.
const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Debug, Clone)]
pub struct RawTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}
```
```rust
    /// `initialize` → receive capabilities → `notifications/initialized`.
    pub async fn initialize(&self, timeout: Duration) -> Result<(), McpError> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "clientInfo": {"name": "agent-mcp", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {}
        });
        self.request("initialize", params, timeout).await?;
        self.notify("notifications/initialized", json!({})).await
    }

    /// `tools/list` → parse the tool descriptors.
    pub async fn list_tools(&self, timeout: Duration) -> Result<Vec<RawTool>, McpError> {
        let res = self.request("tools/list", json!({}), timeout).await?;
        let arr = res.get("tools").and_then(Value::as_array)
            .ok_or_else(|| McpError::Protocol("tools/list: missing 'tools' array".into()))?;
        let mut out = Vec::with_capacity(arr.len());
        for t in arr {
            let name = t.get("name").and_then(Value::as_str)
                .ok_or_else(|| McpError::Protocol("tool missing 'name'".into()))?;
            out.push(RawTool {
                name: name.to_string(),
                description: t.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
                input_schema: t.get("inputSchema").cloned()
                    .unwrap_or_else(|| json!({"type":"object"})),
            });
        }
        Ok(out)
    }
```

- [ ] **Step 4: Run tests and clippy**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-mcp && cargo clippy -p agent-mcp --all-targets -- -D warnings`
Expected: PASS; clippy clean.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-mcp/
git commit -m "feat(mcp): initialize handshake and tools/list discovery"
```

---

### Task 5: McpTool wrapper (naming, schema, intent, execute)

**Files:**
- Create: `crates/agent-mcp/src/tool.rs`
- Modify: `crates/agent-mcp/src/lib.rs` (add `mod tool;` + `pub use tool::McpTool;`)

**Interfaces:**
- Consumes: `McpClient` + `RawTool` (Tasks 3–4), `Trust` (Task 1), `agent_tools::{Tool, ToolSchema, ToolOutput, ToolIntent, ToolError, ToolCtx, Access, Display}`.
- Produces: `pub fn namespaced_name(server: &str, tool: &str) -> String`; `McpTool::new(server: &str, client: Arc<McpClient>, raw: RawTool, trust: Trust) -> McpTool`; `impl Tool for McpTool`.

- [ ] **Step 1: Write the failing tests**

`crates/agent-mcp/src/tool.rs` (test module). The intent test pulls in `agent-policy` (dev-dep) to assert the *real* `Decision`, proving the trust trick needs no policy change:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{McpClient, RawTool};
    use crate::transport::MockTransport;
    use agent_policy::{Decision, PolicyEngine, RulePolicy};
    use agent_tools::{Access, Tool};
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn raw() -> RawTool {
        RawTool { name: "create_issue".into(), description: "Create an issue".into(),
            input_schema: json!({"type":"object","properties":{"title":{"type":"string"}}}) }
    }
    fn client_that<F>(f: F) -> Arc<McpClient>
    where F: Fn(&serde_json::Value) -> Vec<serde_json::Value> + Send + Sync + 'static {
        McpClient::new(Arc::new(MockTransport::scripted(f)))
    }
    fn policy() -> RulePolicy {
        RulePolicy { workspace: PathBuf::from("/work"),
            command_allowlist: vec![], command_denylist: vec![] }
    }

    #[test]
    fn name_is_namespaced_and_sanitized() {
        assert_eq!(namespaced_name("git hub", "create.issue"), "git_hub__create_issue");
    }

    #[test]
    fn schema_carries_namespaced_name_and_input_schema() {
        let tool = McpTool::new("github", client_that(|_| vec![]), raw(), Trust::Ask);
        let s = tool.schema();
        assert_eq!(s.name, "github__create_issue");
        assert_eq!(s.parameters["properties"]["title"]["type"], "string");
    }

    #[test]
    fn ask_trust_maps_to_policy_ask() {
        let tool = McpTool::new("github", client_that(|_| vec![]), raw(), Trust::Ask);
        let intent = tool.intent(&json!({})).unwrap();
        assert_eq!(intent.access, Access::Write);
        assert!(intent.command.is_none());
        assert!(intent.paths.is_empty());
        assert!(matches!(policy().check(&intent), Decision::Ask));
    }

    #[test]
    fn allow_trust_maps_to_policy_allow() {
        let tool = McpTool::new("fs", client_that(|_| vec![]), raw(), Trust::Allow);
        let intent = tool.intent(&json!({})).unwrap();
        assert_eq!(intent.access, Access::Read);
        assert!(intent.paths.is_empty());
        assert!(matches!(policy().check(&intent), Decision::Allow));
    }

    #[tokio::test]
    async fn execute_forwards_call_and_normalizes_text_content() {
        let tool = McpTool::new("github", client_that(|req| {
            let id = req["id"].clone();
            assert_eq!(req["method"], "tools/call");
            assert_eq!(req["params"]["name"], "create_issue"); // server-local name, not namespaced
            vec![json!({"jsonrpc":"2.0","id":id,"result":{
                "content":[{"type":"text","text":"issue #1 created"}],"isError":false}})]
        }), raw(), Trust::Ask);
        let ctx = agent_tools::ToolCtx { workspace: PathBuf::from("/work"),
            timeout: std::time::Duration::from_secs(2),
            cancel: tokio_util::sync::CancellationToken::new() };
        let out = tool.execute(json!({"title":"bug"}), &ctx).await.unwrap();
        assert!(out.content.contains("issue #1 created"));
    }

    #[tokio::test]
    async fn execute_maps_is_error_to_tool_error() {
        let tool = McpTool::new("github", client_that(|req| {
            let id = req["id"].clone();
            vec![json!({"jsonrpc":"2.0","id":id,"result":{
                "content":[{"type":"text","text":"boom"}],"isError":true}})]
        }), raw(), Trust::Ask);
        let ctx = agent_tools::ToolCtx { workspace: PathBuf::from("/work"),
            timeout: std::time::Duration::from_secs(2),
            cancel: tokio_util::sync::CancellationToken::new() };
        let err = tool.execute(json!({}), &ctx).await.unwrap_err();
        assert!(matches!(err, agent_tools::ToolError::Failed { .. }));
    }
}
```

> Add `tokio-util = { workspace = true }` to `agent-mcp`'s `[dev-dependencies]` for the `CancellationToken` in these tests.

- [ ] **Step 2: Run to verify failure**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-mcp tool:: 2>&1 | tail -5`
Expected: FAIL — `McpTool`/`namespaced_name` not found.

- [ ] **Step 3: Implement McpTool**

`crates/agent-mcp/src/tool.rs` (top):
```rust
use crate::client::{McpClient, RawTool};
use crate::config::Trust;
use agent_tools::{Access, Display, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

/// `server__tool`, sanitized to the model-tool-name charset `[a-zA-Z0-9_-]`.
pub fn namespaced_name(server: &str, tool: &str) -> String {
    fn clean(s: &str) -> String {
        s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect()
    }
    format!("{}__{}", clean(server), clean(tool))
}

/// A single MCP server tool wrapped as a native `Tool`.
pub struct McpTool {
    server: String,
    client: Arc<McpClient>,
    local_name: String,   // server-local name used on the wire
    namespaced: String,   // exposed to the model + registry
    description: String,
    input_schema: Value,
    trust: Trust,
}

impl McpTool {
    pub fn new(server: &str, client: Arc<McpClient>, raw: RawTool, trust: Trust) -> Self {
        let namespaced = namespaced_name(server, &raw.name);
        Self {
            server: server.to_string(),
            client,
            local_name: raw.name,
            namespaced,
            description: raw.description,
            input_schema: raw.input_schema,
            trust,
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str { &self.namespaced }
    fn description(&self) -> &str { &self.description }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.namespaced.clone(),
            description: self.description.clone(),
            parameters: self.input_schema.clone(),
        }
    }

    fn intent(&self, _args: &Value) -> Result<ToolIntent, ToolError> {
        // Trust is encoded onto the policy's Read/Write axis (zero policy change):
        // Ask → Write (RulePolicy asks); Allow → Read with empty paths (RulePolicy allows).
        let access = match self.trust { Trust::Allow => Access::Read, Trust::Ask => Access::Write };
        Ok(ToolIntent {
            tool: self.namespaced.clone(),
            access,
            paths: vec![],
            command: None,
            summary: format!("MCP {}::{} (third-party server)", self.server, self.local_name),
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let params = json!({"name": self.local_name, "arguments": args});
        let timeout = ctx.timeout.max(Duration::from_secs(1));
        let result = self.client.request("tools/call", params, timeout).await
            .map_err(|e| ToolError::Failed { message: e.to_string(), stderr: None })?;

        let text = result.get("content").and_then(Value::as_array).map(|parts| {
            parts.iter().map(|p| match p.get("type").and_then(Value::as_str) {
                Some("text") => p.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
                Some(other) => format!("[{other} content omitted]"),
                None => String::new(),
            }).collect::<Vec<_>>().join("\n")
        }).unwrap_or_default();

        if result.get("isError").and_then(Value::as_bool).unwrap_or(false) {
            return Err(ToolError::Failed { message: text, stderr: None });
        }
        Ok(ToolOutput { content: text.clone(), display: Some(Display::Text(text)) })
    }
}
```

- [ ] **Step 4: Re-enable the module + dev-dep**

Add `mod tool;` + `pub use tool::McpTool;` to `lib.rs`. Add `tokio-util = { workspace = true }` to `[dev-dependencies]` in `agent-mcp/Cargo.toml`.

- [ ] **Step 5: Run tests and clippy**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-mcp && cargo clippy -p agent-mcp --all-targets -- -D warnings`
Expected: PASS (naming, schema, both intent→policy tests, execute success + isError); clippy clean.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-mcp/
git commit -m "feat(mcp): McpTool wrapper — namespacing, schema, trust intent, execute"
```

---

### Task 6: McpManager (concurrent connect, status, shutdown) + runtime-config helper

**Files:**
- Create: `crates/agent-mcp/src/manager.rs`
- Modify: `crates/agent-mcp/src/lib.rs` (add `mod manager;` + re-exports)
- Modify: `crates/agent-runtime-config/src/lib.rs` (add `connect_mcp`)
- Modify: `crates/agent-runtime-config/Cargo.toml` (add `agent-mcp` dep)

**Interfaces:**
- Consumes: `McpServersConfig`/`McpServerSpec`/`Trust` (Task 1), `StdioTransport` (Task 2), `McpClient` (Tasks 3–4), `McpTool` (Task 5).
- Produces: `ServerStatus { name: String, connected: bool, tool_count: usize, error: Option<String> }`; `McpManager` with `async fn connect(cfg: &McpServersConfig, connect_timeout: Duration) -> McpManager`, `fn tools(&self) -> Vec<Arc<dyn Tool>>`, `fn summary_line(&self) -> String`, `async fn shutdown(&self)`; runtime-config `async fn connect_mcp(path: &Path) -> McpManager`.

- [ ] **Step 1: Write the failing manager tests (pure parts)**

`crates/agent-mcp/src/manager.rs` (test module). Process-spawning connect is covered by the live test (Task 9); here we test the summary/empty-config behavior:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::McpServersConfig;
    use std::time::Duration;

    #[tokio::test]
    async fn empty_config_connects_nothing() {
        let mgr = McpManager::connect(&McpServersConfig::default(), Duration::from_secs(1)).await;
        assert!(mgr.tools().is_empty());
        assert_eq!(mgr.summary_line(), "mcp: no servers configured");
    }

    #[test]
    fn summary_line_formats_mixed_statuses() {
        let mgr = McpManager::from_parts(vec![], vec![
            ServerStatus { name: "filesystem".into(), connected: true, tool_count: 3, error: None },
            ServerStatus { name: "github".into(), connected: false, tool_count: 0,
                error: Some("timeout".into()) },
        ]);
        assert_eq!(mgr.summary_line(), "mcp: filesystem ✓ (3 tools), github ✗ (timeout)");
    }

    #[tokio::test]
    async fn failed_spawn_is_reported_not_fatal() {
        let mut cfg = McpServersConfig::default();
        cfg.servers.insert("broken".into(), crate::config::McpServerSpec {
            command: "definitely-not-a-real-binary-xyz".into(),
            args: vec![], env: Default::default(), trust: crate::config::Trust::Ask });
        let mgr = McpManager::connect(&cfg, Duration::from_secs(1)).await;
        assert!(mgr.tools().is_empty());
        assert!(mgr.summary_line().contains("broken ✗"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-mcp manager:: 2>&1 | tail -5`
Expected: FAIL — `McpManager` not found.

- [ ] **Step 3: Implement the manager**

`crates/agent-mcp/src/manager.rs`:
```rust
use crate::client::McpClient;
use crate::config::{McpServerSpec, McpServersConfig};
use crate::tool::McpTool;
use crate::transport::StdioTransport;
use agent_tools::Tool;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ServerStatus {
    pub name: String,
    pub connected: bool,
    pub tool_count: usize,
    pub error: Option<String>,
}

/// Owns every connected server's client (and thus child process) for the agent's
/// lifetime, plus the wrapped tools and per-server status.
pub struct McpManager {
    clients: Vec<Arc<McpClient>>,
    tools: Vec<Arc<dyn Tool>>,
    statuses: Vec<ServerStatus>,
}

impl McpManager {
    /// Connect all configured servers concurrently, each under `connect_timeout`.
    /// A server that fails to spawn or handshake is recorded and skipped.
    pub async fn connect(cfg: &McpServersConfig, connect_timeout: Duration) -> Self {
        let futs = cfg.servers.iter().map(|(name, spec)| {
            let name = name.clone();
            let spec = spec.clone();
            async move { connect_one(&name, &spec, connect_timeout).await }
        });
        let results = futures_join_all(futs).await;

        let mut clients = Vec::new();
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
        let mut statuses = Vec::new();
        for r in results {
            match r {
                Ok((name, client, server_tools)) => {
                    statuses.push(ServerStatus { name, connected: true,
                        tool_count: server_tools.len(), error: None });
                    tools.extend(server_tools);
                    clients.push(client);
                }
                Err((name, e)) => {
                    tracing::warn!(target: "mcp", server = %name, error = %e, "server failed to connect");
                    statuses.push(ServerStatus { name, connected: false, tool_count: 0,
                        error: Some(e) });
                }
            }
        }
        statuses.sort_by(|a, b| a.name.cmp(&b.name));
        Self { clients, tools, statuses }
    }

    #[cfg(test)]
    pub(crate) fn from_parts(tools: Vec<Arc<dyn Tool>>, statuses: Vec<ServerStatus>) -> Self {
        Self { clients: vec![], tools, statuses }
    }

    pub fn tools(&self) -> Vec<Arc<dyn Tool>> { self.tools.clone() }

    pub fn summary_line(&self) -> String {
        if self.statuses.is_empty() {
            return "mcp: no servers configured".to_string();
        }
        let parts: Vec<String> = self.statuses.iter().map(|s| {
            if s.connected {
                format!("{} ✓ ({} tools)", s.name, s.tool_count)
            } else {
                format!("{} ✗ ({})", s.name, s.error.as_deref().unwrap_or("error"))
            }
        }).collect();
        format!("mcp: {}", parts.join(", "))
    }

    pub async fn shutdown(&self) {
        for c in &self.clients {
            c.close().await;
        }
    }
}

/// Connect one server: spawn, handshake, discover, wrap tools.
async fn connect_one(
    name: &str, spec: &McpServerSpec, timeout: Duration,
) -> Result<(String, Arc<McpClient>, Vec<Arc<dyn Tool>>), (String, String)> {
    let attempt = async {
        let transport = StdioTransport::spawn(spec).map_err(|e| e.to_string())?;
        let client = McpClient::new(Arc::new(transport));
        client.initialize(timeout).await.map_err(|e| e.to_string())?;
        let raw = client.list_tools(timeout).await.map_err(|e| e.to_string())?;
        let tools: Vec<Arc<dyn Tool>> = raw.into_iter()
            .map(|r| Arc::new(McpTool::new(name, client.clone(), r, spec.trust)) as Arc<dyn Tool>)
            .collect();
        Ok::<_, String>((client, tools))
    };
    match tokio::time::timeout(timeout, attempt).await {
        Ok(Ok((client, tools))) => Ok((name.to_string(), client, tools)),
        Ok(Err(e)) => Err((name.to_string(), e)),
        Err(_) => Err((name.to_string(), "connect timed out".to_string())),
    }
}

/// Minimal concurrent join without pulling extra deps: spawn each future and await.
async fn futures_join_all<F, T>(futs: impl IntoIterator<Item = F>) -> Vec<T>
where F: std::future::Future<Output = T> + Send + 'static, T: Send + 'static {
    let handles: Vec<_> = futs.into_iter().map(tokio::spawn).collect();
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        if let Ok(v) = h.await {
            out.push(v);
        }
    }
    out
}
```

> `futures_join_all` requires each future to be `'static + Send`; `connect_one` is called with owned `name`/`spec` clones (see `connect`), so the spawned futures own their data. This avoids adding the `futures` crate to `agent-mcp`.

- [ ] **Step 4: Re-enable module + re-exports**

Add `mod manager;` + `pub use manager::{McpManager, ServerStatus};` to `lib.rs`.

- [ ] **Step 5: Add the runtime-config helper**

In `crates/agent-runtime-config/Cargo.toml` add under `[dependencies]`:
```toml
agent-mcp = { path = "../agent-mcp" }
```
In `crates/agent-runtime-config/src/lib.rs` add imports + helper:
```rust
use agent_mcp::{McpManager, McpServersConfig};
use std::path::Path;
use std::time::Duration;

/// Load `mcp.json` at `path` and connect its servers. A missing file yields an
/// empty manager (MCP disabled); a malformed file warns and yields empty. The
/// returned `McpManager` owns the server processes — keep it alive for the session.
pub async fn connect_mcp(path: &Path) -> McpManager {
    let (cfg, warning) = McpServersConfig::load_or_empty(path);
    if let Some(w) = warning {
        eprintln!("warning: {} ({}); MCP disabled", w, path.display());
    }
    McpManager::connect(&cfg, Duration::from_secs(15)).await
}
```
Also re-export the types for the binaries:
```rust
pub use agent_mcp::{McpManager, ServerStatus};
```

- [ ] **Step 6: Run tests and clippy (workspace)**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-mcp -p agent-runtime-config && cargo clippy --all-targets -- -D warnings`
Expected: PASS; clippy clean.

- [ ] **Step 7: Commit**

```bash
git add agent/crates/agent-mcp/ agent/crates/agent-runtime-config/
git commit -m "feat(mcp): McpManager concurrent connect/status/shutdown + connect_mcp helper"
```

---

### Task 7: CLI wiring (`--mcp-config`)

**Files:**
- Modify: `crates/agent-cli/src/main.rs`
- Modify: `crates/agent-cli/Cargo.toml` (ensure it can reach `connect_mcp` — it already depends on `agent-runtime-config`)

**Interfaces:**
- Consumes: `connect_mcp` (Task 6).

- [ ] **Step 1: Add the flag**

In `crates/agent-cli/src/main.rs`, add to `struct Cli` (after `stream_timeout_secs`):
```rust
    /// Optional MCP server config (mcp.json shape). If absent, MCP is disabled.
    #[arg(long)]
    mcp_config: Option<std::path::PathBuf>,
```

- [ ] **Step 2: Connect MCP and register tools before building the loop**

In `crates/agent-cli/src/main.rs`, replace the line `let tools = Arc::new(build_registry());` with:
```rust
    let mut registry = build_registry();
    // Connect MCP servers (if configured), register their tools, keep the manager alive.
    let mcp_manager = match &cli.mcp_config {
        Some(path) => {
            let mgr = agent_runtime_config::connect_mcp(path).await;
            println!("{}", mgr.summary_line());
            for t in mgr.tools() {
                registry.register(t);
            }
            Some(mgr)
        }
        None => None,
    };
    let tools = Arc::new(registry);
```
The `mcp_manager` binding lives until `main` returns, keeping the server processes (and their tools) alive for the whole REPL session. Add `let _ = &mcp_manager;` after the REPL loop if clippy warns about an unused binding, or name it `_mcp_manager`.

- [ ] **Step 3: Manual smoke test (no server needed)**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo run -p agent-cli -- --help 2>&1 | grep mcp-config`
Expected: the `--mcp-config` flag appears in help.

Run with a missing file (should warn + disable, not crash):
```bash
echo 'exit' | cargo run -p agent-cli -- --mcp-config /tmp/nope.json --backend openai --base-url http://localhost:8080 --model qwen3.6-35b-a3b 2>&1 | head -3
```
Expected: `mcp: no servers configured` printed; REPL starts then exits.

- [ ] **Step 4: Run workspace tests and clippy**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo test --workspace && cargo clippy --all-targets -- -D warnings`
Expected: all green; clippy clean.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-cli/
git commit -m "feat(mcp): wire --mcp-config into the CLI (register tools, print summary)"
```

---

### Task 8: Daemon wiring (thread MCP tools through rebuilds)

**Files:**
- Modify: `crates/agent-server/src/runtime.rs` (thread `mcp_tools` into `RuntimeState::new` + `build_loop`)
- Modify: `crates/agent-server/src/daemon.rs` (`DaemonParams.mcp_tools` field; pass into `RuntimeState::new`)
- Modify: `crates/agent-server/src/main.rs` (`--mcp-config`; connect once; populate params; hold manager)

**Interfaces:**
- Consumes: `connect_mcp` (Task 6), `agent_tools::Tool`.
- Produces: `DaemonParams.mcp_tools: Arc<[Arc<dyn Tool>]>`; `build_loop(..., mcp_tools: &[Arc<dyn Tool>])` registers MCP tools after `build_registry()` on every rebuild.

Rationale: the daemon rebuilds the registry on every settings change (`build_loop`). MCP servers must be connected **once** at process start; the wrapped tool handles (cheap `Arc` clones) are threaded through each rebuild so a reconfigure never respawns server processes. The owning `McpManager` lives in `main`, across WebSocket reconnects.

- [ ] **Step 1: Add `mcp_tools` to `build_loop` and register them**

In `crates/agent-server/src/runtime.rs`, change `build_loop`'s signature and body. Add `use agent_tools::Tool;` at the top. Replace the `build_loop` function's signature and the registry line:
```rust
fn build_loop(
    cfg: &RuntimeConfig,
    sink: &Arc<WsEventSink>,
    approval: &Arc<WsApprovalChannel>,
    workspace: &Path,
    api_key: &Option<String>,
    claude_binary: &str,
    mcp_tools: &[Arc<dyn Tool>],
) -> Arc<AgentLoop> {
    let model = build_model(&cfg.backend, &cfg.base_url, &cfg.model, claude_binary, api_key.clone());
    let policy = Arc::new(RulePolicy {
        workspace: workspace.to_path_buf(),
        command_allowlist: cfg.command_allowlist.clone(),
        command_denylist: cfg.effective_denylist(),
    });
    let mut registry = build_registry();
    for t in mcp_tools {
        registry.register(t.clone());
    }
    Arc::new(AgentLoop::new(
        model,
        pick_protocol(&cfg.protocol),
        Arc::new(registry),
        policy,
        approval.clone(),
        sink.clone(),
        LoopConfig {
            model_limit: cfg.context_limit,
            max_turns: cfg.max_turns,
            max_retries: 3,
            temperature: cfg.temperature,
            max_tokens: Some(cfg.max_tokens),
            workspace: workspace.to_path_buf(),
            tool_timeout: Duration::from_secs(120),
            stream_idle_timeout: DEFAULT_STREAM_IDLE_TIMEOUT,
        },
    ))
}
```

- [ ] **Step 2: Store `mcp_tools` on `RuntimeState` and pass to every `build_loop`**

In `crates/agent-server/src/runtime.rs`:
1. Add a field to `struct RuntimeState`: `mcp_tools: Arc<[Arc<dyn Tool>]>,`.
2. Add a parameter to `RuntimeState::new` (after `tx`): `mcp_tools: Arc<[Arc<dyn Tool>]>,`. (The `#[allow(clippy::too_many_arguments)]` already on `new` covers the extra arg.)
3. In `new`, change the initial build call to pass `&mcp_tools`, and store the field:
```rust
        let initial = build_loop(&config, &sink, &approval, &workspace, &api_key, &claude_binary, &mcp_tools);
        Self {
            loop_cell: Mutex::new(initial),
            config: Mutex::new(config),
            sink, approval, workspace, api_key, claude_binary, config_path, session, tx, mcp_tools,
        }
```
4. In `apply`, change the rebuild call to pass `&self.mcp_tools`:
```rust
        let new_loop = build_loop(&cfg, &self.sink, &self.approval, &self.workspace,
            &self.api_key, &self.claude_binary, &self.mcp_tools);
```
5. In `runtime.rs`'s test module, every `RuntimeState::new(...)` call must pass a final `mcp_tools` arg. Use an empty slice: `Arc::from(Vec::<Arc<dyn Tool>>::new())`. (Search the test module for `RuntimeState::new` and append the argument.)

- [ ] **Step 3: Add `mcp_tools` to `DaemonParams` and forward it**

In `crates/agent-server/src/daemon.rs`:
1. Add `use agent_tools::Tool;` and `use std::sync::Arc;` if not present.
2. Add a field to `pub struct DaemonParams`: `pub mcp_tools: Arc<[Arc<dyn Tool>]>,`.
3. Where `RuntimeState::new(...)` is constructed inside `run`, pass `params.mcp_tools.clone()` as the final argument.

In `crates/agent-server/src/main.rs`'s `params_clone`, add: `mcp_tools: p.mcp_tools.clone(),` to the constructed `DaemonParams`.

- [ ] **Step 4: Connect MCP once in `main` and populate params**

In `crates/agent-server/src/main.rs`:
1. Add the flag to the `Run` subcommand (after `runtime_config`):
```rust
        /// Optional MCP server config (mcp.json shape). If absent, MCP is disabled.
        #[arg(long)]
        mcp_config: Option<PathBuf>,
```
2. Add `mcp_config` to the `Cmd::Run { .. }` destructuring pattern.
3. After the `base.clone().normalized().validate()` block and before building `DaemonParams`, connect MCP once and hold the manager for the process lifetime:
```rust
            let mcp_manager = match &mcp_config {
                Some(path) => {
                    let mgr = agent_runtime_config::connect_mcp(path).await;
                    println!("{}", mgr.summary_line());
                    Some(mgr)
                }
                None => None,
            };
            let mcp_tools: std::sync::Arc<[std::sync::Arc<dyn agent_tools::Tool>]> =
                mcp_manager.as_ref().map(|m| std::sync::Arc::from(m.tools()))
                    .unwrap_or_else(|| std::sync::Arc::from(Vec::new()));
```
4. Add `mcp_tools,` to the `DaemonParams { .. }` initializer.
5. Ensure `agent-server` depends on `agent-tools` (it does, transitively via core, but add it explicitly to `[dependencies]` if `cargo` complains about the path import). Add to `crates/agent-server/Cargo.toml` under `[dependencies]` if missing:
```toml
agent-tools = { path = "../agent-tools" }
```
The `mcp_manager` binding lives until `main` exits (it owns the server processes across all WebSocket reconnects in the loop below it).

- [ ] **Step 5: Run workspace tests and clippy**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo test --workspace && cargo clippy --all-targets -- -D warnings`
Expected: all green (incl. the updated `runtime.rs` tests); clippy clean.

- [ ] **Step 6: Smoke-test the daemon help**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo run -p agent-server -- run --help 2>&1 | grep mcp-config`
Expected: the `--mcp-config` flag appears.

- [ ] **Step 7: Commit**

```bash
git add agent/crates/agent-server/
git commit -m "feat(mcp): wire --mcp-config into the daemon (connect once, thread through rebuilds)"
```

---

### Task 9: Live DoD test, example config, and docs

**Files:**
- Create: `crates/agent-mcp/tests/live_filesystem.rs`
- Create: `mcp.example.json` (at `agent/` root)
- Modify: `agent/docs/RUNNING.md`
- Modify: `docs/superpowers/context/follow-ups.md` (create if absent)

**Interfaces:**
- Consumes: the full public `agent-mcp` API.

- [ ] **Step 1: Write the `#[ignore]`-gated live integration test**

`crates/agent-mcp/tests/live_filesystem.rs`. Requires Node/npx + network on first run; gated so CI stays hermetic.
```rust
//! Live integration test against the official filesystem MCP server.
//! Run explicitly: `cargo test -p agent-mcp --test live_filesystem -- --ignored`
//! Requires `npx` on PATH (downloads @modelcontextprotocol/server-filesystem).

use agent_mcp::{McpManager, McpServerSpec, McpServersConfig, Trust};
use std::collections::BTreeMap;
use std::time::Duration;

#[tokio::test]
#[ignore = "requires npx + network; run manually for the DoD"]
async fn filesystem_server_tools_register_and_execute() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("hello.txt"), "hi from mcp").unwrap();

    let mut servers = BTreeMap::new();
    servers.insert("filesystem".to_string(), McpServerSpec {
        command: "npx".into(),
        args: vec!["-y".into(), "@modelcontextprotocol/server-filesystem".into(),
                   tmp.path().to_string_lossy().into_owned()],
        env: BTreeMap::new(),
        trust: Trust::Ask,
    });
    let cfg = McpServersConfig { servers };

    let mgr = McpManager::connect(&cfg, Duration::from_secs(30)).await;
    eprintln!("{}", mgr.summary_line());
    let tools = mgr.tools();
    assert!(!tools.is_empty(), "filesystem server should expose tools");
    assert!(tools.iter().any(|t| t.name().starts_with("filesystem__")),
        "tools should be namespaced by server");

    mgr.shutdown().await;
}
```

- [ ] **Step 2: Run the live test explicitly (DoD verification)**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-mcp --test live_filesystem -- --ignored --nocapture 2>&1 | tail -20`
Expected: prints `mcp: filesystem ✓ (N tools)`; test passes. If `npx` is unavailable, note it and proceed (CI does not run this).

- [ ] **Step 3: Add the example config**

`agent/mcp.example.json`:
```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "."],
      "trust": "ask"
    }
  }
}
```

- [ ] **Step 4: Document it in RUNNING.md**

Append a section to `agent/docs/RUNNING.md`:
```markdown
## MCP servers (optional)

The agent can consume tools from external MCP servers over stdio. Copy
`mcp.example.json` to `mcp.json`, edit the server list, and pass `--mcp-config`:

    cargo run -p agent-cli -- --base-url http://localhost:8080 --model qwen3.6-35b-a3b \
      --workspace . --mcp-config mcp.json

On startup the CLI prints a one-line summary, e.g.
`mcp: filesystem ✓ (11 tools)`. Each server's tools appear namespaced as
`server__tool`. By default every MCP tool requires approval on each call; set a
server's `"trust": "allow"` to auto-approve a server you operate yourself.

A server that fails to start is skipped with a warning — it never blocks the agent.
The daemon takes the same `--mcp-config` flag on its `run` subcommand.
```

- [ ] **Step 5: Run the full workspace suite + clippy one final time**

Run: `source "$HOME/.cargo/env" && cd /home/kalen/rust-agent-runtime/agent && cargo test --workspace && cargo clippy --all-targets -- -D warnings`
Expected: all green; clippy clean. (The live test is `#[ignore]`d, so it does not run here.)

- [ ] **Step 6: Record follow-ups**

Append to `docs/superpowers/context/follow-ups.md` (create with a top heading if absent) a dated section for this cycle, e.g.:
```markdown
## 2026-06-23 mcp-client

- Streamable HTTP / remote MCP transport (+ auth) — Open — deferred; `McpTransport` seam is ready.
- MCP resources & prompts — Open — no core seam yet; deferred.
- EventSink/UI MCP server-status — Open — would add an AgentEvent variant (core touch); deferred until a UI consumer exists.
- Browser-side MCP management via Settings inbound channel — Open — pairs with the deferred Settings capability.
- OS-sandboxed MCP server processes — Open — MCP servers are untrusted code; synergy with os-sandboxing primer.
```
(Add any Minor findings from the final whole-branch review here too, per the cycle's review step.)

- [ ] **Step 7: Commit**

```bash
git add agent/crates/agent-mcp/tests/ agent/mcp.example.json agent/docs/RUNNING.md docs/superpowers/context/follow-ups.md
git commit -m "test(mcp): live filesystem-server DoD test; docs + example config + follow-ups"
```

---

## Done criteria (whole plan)

- `cargo test --workspace` green; `cargo clippy --all-targets -- -D warnings` clean.
- `agent-core`, `agent-model`, `agent-tools`, `agent-policy` unchanged (`git diff --stat` touches none of them).
- CLI and daemon accept `--mcp-config`; configured stdio servers' tools register namespaced `server__tool` and run through the normal loop, gated by trust.
- The `#[ignore]`-gated live test connects the real filesystem server and registers its tools.
- After all tasks: run `/graphify . --update` once to refresh the knowledge graph (per the session brief).
