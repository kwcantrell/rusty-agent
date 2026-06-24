# Tauri Desktop Conversion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the existing `web/` React UI + the `agent/` Rust runtime as a single fully-local Tauri v2 desktop app that talks to itself over a localhost WebSocket and to a fixed local `llama-server` on `http://localhost:8080`.

**Architecture:** A new `src-tauri/` crate embeds the agent runtime. On startup it binds a `127.0.0.1` WebSocket server and hands each accepted connection to a transport-agnostic `agent_server::daemon::serve()` (extracted from the existing `run()`). The webview loads `web/`, detects Tauri, fetches the local WS URL via an `invoke` command, and connects with the unchanged `WireEnvelope` protocol — no Cloudflare Worker, no pairing.

**Tech Stack:** Rust, Tauri v2, tokio, tokio-tungstenite 0.24, reqwest; React 19 + Vite + TypeScript + Vitest; `@tauri-apps/api` v2, `tauri-plugin-dialog` v2.

## Global Constraints

- `tokio-tungstenite` version in `src-tauri` MUST be `0.24` (match `agent/` so `WebSocketStream<S>` types unify).
- Tauri v2; app `identifier` = `dev.rust-agent-runtime.desktop` (placeholder).
- Llama defaults seeded into `RuntimeConfig`: `backend="openai"`, `base_url="http://localhost:8080"`, `model="qwen3.6-35b-a3b"`, `protocol="native"`, `context_limit=262144`, `preserve_thinking=true`.
- Detect Tauri in the webview via `"__TAURI_INTERNALS__" in window` (NOT `window.__TAURI__`; `withGlobalTauri` stays false).
- The Cloudflare Worker, enroll/pairing/token path, and `daemon::run()` cloud behavior MUST remain unchanged and green.
- Bundle targets: Linux `deb` + `appimage` only (desktop-focused; macOS/Windows deferred).
- Webview CSP MUST allow `connect-src` to `ws://127.0.0.1:*` and `ws://localhost:*`; `:8080` is reached only from Rust (`llama_health`), never the webview.

---

### Task 1: Extract transport-agnostic `serve()` in `agent-server`

**Files:**
- Modify: `agent/crates/agent-server/src/daemon.rs` (split `run()`)
- Test: `agent/crates/agent-server/tests/serve_inbound.rs` (new)

**Interfaces:**
- Consumes: existing `DaemonParams`, `RuntimeState`, `WsEventSink`, `WsApprovalChannel`, `WireEnvelope`.
- Produces: `pub async fn serve<S>(ws: tokio_tungstenite::WebSocketStream<S>, params: DaemonParams) -> Result<(), DynErr> where S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static`. `run()` keeps its signature and delegates to `serve()`.

- [ ] **Step 1: Write the failing test**

Create `agent/crates/agent-server/tests/serve_inbound.rs`:

```rust
use agent_runtime_config::RuntimeConfig;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// The Tauri bridge's path: the agent ACCEPTS an inbound socket and runs
/// `serve()` on it (no outbound dial). A client connects, sends `settings_get`,
/// and must receive a `settings_state` frame — proving `serve()` drives the
/// runtime over an accepted connection.
#[tokio::test]
async fn serve_answers_settings_get_over_accepted_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let workspace = tempfile::tempdir().unwrap();
    let config_path = workspace.path().join("agent-runtime.json");
    let params = agent_server::daemon::DaemonParams {
        ws_url: String::new(),       // unused by serve()
        agent_token: String::new(),  // unused by serve()
        config: RuntimeConfig::from_launch(
            "openai".into(), "http://127.0.0.1:1".into(),
            "default".into(), "native".into(), 8192),
        api_key: None,
        claude_binary: "claude".into(),
        config_path,
        workspace: workspace.path().to_path_buf(),
        system_prompt: agent_server::daemon::SYSTEM_PROMPT.to_string(),
        mcp_tools: Arc::from(Vec::<Arc<dyn agent_tools::Tool>>::new()),
        memory_tools: Arc::from(Vec::<Arc<dyn agent_tools::Tool>>::new()),
    };

    let agent = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _ = tokio::time::timeout(
            Duration::from_secs(10),
            agent_server::daemon::serve(ws, params),
        ).await;
    });

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/agent"))
        .await.unwrap();
    ws.send(WsMessage::Text(
        serde_json::json!({"v":1,"session_id":"s1","kind":"settings_get"}).to_string(),
    )).await.unwrap();

    let mut saw_state = false;
    while let Some(Ok(msg)) = ws.next().await {
        let WsMessage::Text(t) = msg else { continue };
        let v: serde_json::Value = serde_json::from_str(t.as_str()).unwrap();
        if v["kind"] == "settings_state" { saw_state = true; break; }
    }
    assert!(saw_state, "expected a settings_state response from serve()");
    agent.abort();
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-server --test serve_inbound`
Expected: FAIL to compile — `function 'serve' not found in module 'daemon'`.

- [ ] **Step 3: Refactor `run()` to delegate to a new `serve()`**

In `agent/crates/agent-server/src/daemon.rs`, add imports near the top (after the existing `use` lines):

```rust
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
```

Replace the body of `pub async fn run(params: DaemonParams) -> Result<(), DynErr>` so it only dials and delegates:

```rust
pub async fn run(params: DaemonParams) -> Result<(), DynErr> {
    let mut req = params.ws_url.clone().into_client_request()?;
    req.headers_mut().insert("Authorization",
        format!("Bearer {}", params.agent_token).parse()?);
    let (ws, _resp) = tokio_tungstenite::connect_async(req).await?;
    serve(ws, params).await
}

/// Drive the runtime over an already-established WebSocket. Transport-agnostic:
/// the cloud path (`run`) dials the Worker; the desktop bridge accepts a local
/// connection. Everything from here down is identical to the old `run()` body.
pub async fn serve<S>(ws: WebSocketStream<S>, params: DaemonParams) -> Result<(), DynErr>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let session = Arc::new(Mutex::new(String::new()));
    let (tx, mut rx) = mpsc::unbounded_channel::<WireEnvelope>();

    let sink = Arc::new(WsEventSink::new(tx.clone(), session.clone()));
    let approval = Arc::new(WsApprovalChannel::new(tx.clone(), session.clone(),
        Duration::from_secs(300)));

    let config = RuntimeConfig::load_over(params.config.clone(), &params.config_path);
    let runtime = Arc::new(RuntimeState::new(
        config,
        sink,
        approval.clone(),
        params.workspace.clone(),
        params.api_key.clone(),
        params.claude_binary.clone(),
        params.config_path.clone(),
        session.clone(),
        tx.clone(),
        params.mcp_tools.clone(),
        params.memory_tools.clone(),
        params.system_prompt.clone(),
    ));
    let ctx = Arc::new(tokio::sync::Mutex::new(
        WindowContext::new(Message::system(params.system_prompt.clone()))));

    let (mut write, mut read) = ws.split();

    // Writer task: drain the channel to the socket; ping periodically.
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(Duration::from_secs(25));
        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(env) => {
                        let txt = serde_json::to_string(&env).unwrap_or_default();
                        if write.send(WsMessage::Text(txt)).await.is_err() { break; }
                    }
                    None => break,
                },
                _ = ping.tick() => {
                    if write.send(WsMessage::Ping(Vec::new())).await.is_err() { break; }
                }
            }
        }
    });

    // Read loop: dispatch inbound frames.
    while let Some(msg) = read.next().await {
        let msg = match msg { Ok(m) => m, Err(_) => break };
        let WsMessage::Text(t) = msg else { continue };
        let env: WireEnvelope = match serde_json::from_str(t.as_str()) {
            Ok(e) => e,
            Err(e) => { tracing::warn!(error=%e, "bad frame"); continue }
        };
        match env.body {
            WireBody::UserInput { text } => {
                *session.lock().unwrap() = env.session_id.clone();
                let agent = runtime.current_loop();
                let system_prompt = runtime.current_system_prompt();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    let mut guard = ctx.lock().await;
                    guard.set_system(Message::system(system_prompt));
                    if let Err(e) = agent.run(&mut *guard, text).await {
                        tracing::error!(error=%e, "run failed");
                    }
                });
            }
            WireBody::ApprovalResponse { decision } => {
                if let Some(id) = env.id {
                    approval.resolve(&id, decision.into());
                }
            }
            other @ (WireBody::SettingsGet | WireBody::SettingsUpdate { .. }) => {
                *session.lock().unwrap() = env.session_id.clone();
                runtime.handle(&other);
            }
            _ => {}
        }
    }
    writer.abort();
    Ok(())
}
```

(Delete the old inline `req`/`connect_async`/`split` + writer + read-loop that previously lived in `run()` — it now lives in `serve()`.)

- [ ] **Step 4: Run the new test and the existing cloud test**

Run: `cd agent && cargo test -p agent-server`
Expected: PASS — both `serve_answers_settings_get_over_accepted_socket` and the existing `settings_get_round_trips_over_websocket` (cloud `run()`) pass.

- [ ] **Step 5: Commit**

```bash
cd agent && git add crates/agent-server/src/daemon.rs crates/agent-server/tests/serve_inbound.rs
git commit -m "refactor(agent-server): extract transport-agnostic serve() from run()"
```

---

### Task 2: Add `setup::local_params()` desktop runtime builder

**Files:**
- Create: `agent/crates/agent-server/src/setup.rs`
- Modify: `agent/crates/agent-server/src/lib.rs` (add `pub mod setup;`)
- Test: in `setup.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `DaemonParams` (from `crate::daemon`), `RuntimeConfig::from_launch`, `daemon::SYSTEM_PROMPT`.
- Produces: `pub fn local_params(workspace: std::path::PathBuf, config_path: std::path::PathBuf, base_url: String, model: String) -> crate::daemon::DaemonParams` — builds the desktop runtime with llama defaults, no MCP/memory tools.

- [ ] **Step 1: Write the failing test**

Create `agent/crates/agent-server/src/setup.rs`:

```rust
//! Build a `DaemonParams` for the fully-local desktop bridge (no Worker, no
//! pairing, no MCP/memory). Llama defaults are seeded here; the Settings UI can
//! still edit them live via the persisted `config_path`.
use crate::daemon::{DaemonParams, SYSTEM_PROMPT};
use agent_runtime_config::RuntimeConfig;
use std::path::PathBuf;
use std::sync::Arc;

pub fn local_params(
    workspace: PathBuf,
    config_path: PathBuf,
    base_url: String,
    model: String,
) -> DaemonParams {
    let mut config = RuntimeConfig::from_launch(
        "openai".into(), base_url, model, "native".into(), 262_144);
    config.preserve_thinking = true;
    config.enable_thinking = true;
    DaemonParams {
        ws_url: String::new(),
        agent_token: String::new(),
        config,
        api_key: std::env::var("AGENT_API_KEY").ok(),
        claude_binary: "claude".into(),
        config_path,
        workspace,
        system_prompt: SYSTEM_PROMPT.to_string(),
        mcp_tools: Arc::from(Vec::<Arc<dyn agent_tools::Tool>>::new()),
        memory_tools: Arc::from(Vec::<Arc<dyn agent_tools::Tool>>::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_params_seeds_llama_defaults() {
        let p = local_params(
            PathBuf::from("/tmp/ws"),
            PathBuf::from("/tmp/agent-runtime.json"),
            "http://localhost:8080".into(),
            "qwen3.6-35b-a3b".into(),
        );
        assert_eq!(p.config.backend, "openai");
        assert_eq!(p.config.base_url, "http://localhost:8080");
        assert_eq!(p.config.model, "qwen3.6-35b-a3b");
        assert_eq!(p.config.protocol, "native");
        assert!(p.config.preserve_thinking);
        assert_eq!(p.workspace, PathBuf::from("/tmp/ws"));
        assert!(p.mcp_tools.is_empty());
    }
}
```

- [ ] **Step 2: Register the module and run the failing test**

In `agent/crates/agent-server/src/lib.rs`, add after `pub mod runtime;`:

```rust
pub mod setup;
```

Run: `cd agent && cargo test -p agent-server setup::tests::local_params_seeds_llama_defaults`
Expected: PASS (the module + test compile and the assertions hold).

> If it fails to compile because a field name differs, the canonical fields are
> in `agent/crates/agent-runtime-config/src/runtime_config.rs` — `backend`,
> `base_url`, `model`, `protocol`, `context_limit`, `enable_thinking`,
> `preserve_thinking`.

- [ ] **Step 3: Commit**

```bash
cd agent && git add crates/agent-server/src/setup.rs crates/agent-server/src/lib.rs
git commit -m "feat(agent-server): add setup::local_params for the desktop bridge"
```

---

### Task 3: Scaffold the `src-tauri` crate wrapping `web/`

**Files:**
- Create: `src-tauri/Cargo.toml`, `src-tauri/build.rs`, `src-tauri/tauri.conf.json`, `src-tauri/capabilities/default.json`, `src-tauri/src/main.rs`, `src-tauri/src/lib.rs`, `src-tauri/icons/icon.png`
- Modify: repo root `package.json` (add `@tauri-apps/cli`), `.gitignore` (ignore `src-tauri/target`, `src-tauri/gen`)

**Interfaces:**
- Produces: a compilable Tauri app whose `run()` is `rust_agent_runtime_desktop_lib::run`; `tauri dev` shows the existing `web/` UI (still in cloud/pairing mode until Task 8).

- [ ] **Step 1: Create `src-tauri/Cargo.toml`**

```toml
[package]
name = "rust-agent-runtime-desktop"
version = "0.1.0"
edition = "2021"

[lib]
name = "rust_agent_runtime_desktop_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-dialog = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tokio-tungstenite = "0.24"
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
futures = "0.3"
agent-server = { path = "../agent/crates/agent-server" }
agent-runtime-config = { path = "../agent/crates/agent-runtime-config" }
agent-tools = { path = "../agent/crates/agent-tools" }

[dev-dependencies]
wiremock = "0.6"
tempfile = "3"
```

- [ ] **Step 2: Create `src-tauri/build.rs`**

```rust
fn main() {
    tauri_build::build()
}
```

- [ ] **Step 3: Create `src-tauri/tauri.conf.json`**

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "rust-agent-runtime",
  "version": "0.1.0",
  "identifier": "dev.rust-agent-runtime.desktop",
  "build": {
    "frontendDist": "../web/dist",
    "devUrl": "http://localhost:5173",
    "beforeDevCommand": "npm --prefix web run dev",
    "beforeBuildCommand": "npm --prefix web run build"
  },
  "app": {
    "windows": [
      { "label": "main", "title": "rust-agent-runtime", "width": 1280, "height": 820 }
    ],
    "security": {
      "csp": "default-src 'self'; connect-src 'self' ws://127.0.0.1:* ws://localhost:* http://localhost:*; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:"
    }
  },
  "bundle": {
    "active": true,
    "targets": ["deb", "appimage"],
    "icon": ["icons/icon.png"]
  }
}
```

- [ ] **Step 4: Create `src-tauri/capabilities/default.json`**

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Capabilities for the main window",
  "windows": ["main"],
  "permissions": ["core:default", "dialog:default"]
}
```

- [ ] **Step 5: Create `src-tauri/src/main.rs`**

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    rust_agent_runtime_desktop_lib::run()
}
```

- [ ] **Step 6: Create `src-tauri/src/lib.rs` (minimal; grown in later tasks)**

```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 7: Provide an app icon**

Tauri's codegen requires `icons/icon.png`. If you have a logo PNG, generate the full set:

```bash
cd /home/kalen/rust-agent-runtime
npx @tauri-apps/cli icon path/to/logo.png   # writes src-tauri/icons/*
```

If you have no logo, create a 512×512 placeholder:

```bash
cd /home/kalen/rust-agent-runtime
mkdir -p src-tauri/icons
python3 -c "import struct,zlib;\
w=h=512;\
raw=b''.join(b'\x00'+bytes((30,30,46))*w for _ in range(h));\
c=lambda t,d:struct.pack('>I',len(d))+t+d+struct.pack('>I',zlib.crc32(t+d)&0xffffffff);\
png=b'\x89PNG\r\n\x1a\n'+c(b'IHDR',struct.pack('>IIBBBBB',w,h,8,2,0,0,0))+c(b'IDAT',zlib.compress(raw,9))+c(b'IEND',b'');\
open('src-tauri/icons/icon.png','wb').write(png)"
```

- [ ] **Step 8: Add the Tauri CLI and ignore build artifacts**

```bash
cd /home/kalen/rust-agent-runtime
npm install -D @tauri-apps/cli@latest
printf '\nsrc-tauri/target/\nsrc-tauri/gen/\n' >> .gitignore
```

- [ ] **Step 9: Verify it compiles**

Run:
```bash
cd /home/kalen/rust-agent-runtime/src-tauri && cargo check
```
Expected: PASS — `tauri-build` runs, the crate compiles, schemas are generated under `src-tauri/gen/`.

- [ ] **Step 10: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add src-tauri package.json package-lock.json .gitignore
git commit -m "feat(desktop): scaffold src-tauri Tauri v2 app wrapping web/"
```

---

### Task 4: Local WS bridge + `get_local_ws_url` command

**Files:**
- Create: `src-tauri/src/bridge.rs`
- Modify: `src-tauri/src/lib.rs` (start the bridge, register the command, manage state)
- Test: `src-tauri/tests/bridge.rs` (new)

**Interfaces:**
- Consumes: `agent_server::daemon::serve`, `agent_server::setup::local_params`.
- Produces:
  - `pub struct Bridge { pub port: u16, current: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>, workspace: std::sync::Arc<tokio::sync::Mutex<std::path::PathBuf>>, config_path: std::path::PathBuf, base_url: String, model: String }`
  - `pub async fn start(workspace: PathBuf, config_path: PathBuf, base_url: String, model: String) -> std::io::Result<std::sync::Arc<Bridge>>`
  - `impl Bridge { pub async fn set_workspace(&self, dir: PathBuf); pub fn ws_url(&self) -> String }`
  - Tauri command `get_local_ws_url(state) -> String`.

- [ ] **Step 1: Write the failing test**

Create `src-tauri/tests/bridge.rs`:

```rust
use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// Start the bridge, connect to its advertised ws_url, send settings_get, and
/// expect a settings_state frame back — proving the bridge wires an accepted
/// connection into agent_server::serve() with a desktop runtime.
#[tokio::test]
async fn bridge_serves_local_runtime() {
    let ws_dir = tempfile::tempdir().unwrap();
    let cfg = ws_dir.path().join("agent-runtime.json");
    let bridge = rust_agent_runtime_desktop_lib::bridge::start(
        ws_dir.path().to_path_buf(),
        cfg,
        "http://127.0.0.1:1".into(), // closed port: agent.run fails fast, loop survives
        "default".into(),
    ).await.unwrap();

    let url = bridge.ws_url();
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    ws.send(WsMessage::Text(
        serde_json::json!({"v":1,"session_id":"s1","kind":"settings_get"}).to_string(),
    )).await.unwrap();

    let saw = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(Ok(msg)) = ws.next().await {
            if let WsMessage::Text(t) = msg {
                let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                if v["kind"] == "settings_state" { return true; }
            }
        }
        false
    }).await.unwrap();
    assert!(saw, "expected settings_state from the bridge");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --test bridge`
Expected: FAIL to compile — `module 'bridge' not found`.

- [ ] **Step 3: Implement `src-tauri/src/bridge.rs`**

```rust
//! Localhost WebSocket bridge: accepts the webview's connection and drives the
//! embedded agent runtime via `agent_server::daemon::serve`.
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

pub struct Bridge {
    pub port: u16,
    current: Mutex<Option<tokio::task::JoinHandle<()>>>,
    workspace: Arc<Mutex<PathBuf>>,
    config_path: PathBuf,
    base_url: String,
    model: String,
}

impl Bridge {
    pub fn ws_url(&self) -> String {
        format!("ws://127.0.0.1:{}/agent", self.port)
    }

    /// Point the runtime at a new workspace: drop the active connection so the
    /// webview auto-reconnects into a fresh `serve()` bound to `dir`.
    pub async fn set_workspace(&self, dir: PathBuf) {
        *self.workspace.lock().await = dir;
        if let Some(task) = self.current.lock().await.take() {
            task.abort();
        }
    }
}

pub async fn start(
    workspace: PathBuf,
    config_path: PathBuf,
    base_url: String,
    model: String,
) -> std::io::Result<Arc<Bridge>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let bridge = Arc::new(Bridge {
        port,
        current: Mutex::new(None),
        workspace: Arc::new(Mutex::new(workspace)),
        config_path,
        base_url,
        model,
    });

    let b = bridge.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { continue };
            let ws = match tokio_tungstenite::accept_async(stream).await {
                Ok(ws) => ws,
                Err(_) => continue,
            };
            let dir = b.workspace.lock().await.clone();
            let params = agent_server::setup::local_params(
                dir, b.config_path.clone(), b.base_url.clone(), b.model.clone());
            let task = tokio::spawn(async move {
                let _ = agent_server::daemon::serve(ws, params).await;
            });
            *b.current.lock().await = Some(task);
        }
    });

    Ok(bridge)
}
```

- [ ] **Step 4: Expose the `bridge` module**

In `src-tauri/src/lib.rs`, add at the top:

```rust
pub mod bridge;
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd src-tauri && cargo test --test bridge`
Expected: PASS.

- [ ] **Step 6: Start the bridge in the app and add `get_local_ws_url`**

Replace `src-tauri/src/lib.rs` with:

```rust
pub mod bridge;

use std::sync::Arc;
use tauri::Manager;

struct AppState {
    bridge: Arc<bridge::Bridge>,
}

#[tauri::command]
fn get_local_ws_url(state: tauri::State<'_, AppState>) -> String {
    state.bridge.ws_url()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Workspace defaults to home for now; Task 5 adds the picker + persistence.
            let workspace = dirs_home().unwrap_or_else(|| std::path::PathBuf::from("."));
            let config_path = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join("agent-runtime.json");
            let bridge = tauri::async_runtime::block_on(bridge::start(
                workspace,
                config_path,
                "http://localhost:8080".into(),
                "qwen3.6-35b-a3b".into(),
            ))?;
            app.manage(AppState { bridge });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_local_ws_url])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}
```

- [ ] **Step 7: Verify it compiles**

Run: `cd src-tauri && cargo check`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add src-tauri/src/bridge.rs src-tauri/src/lib.rs src-tauri/tests/bridge.rs
git commit -m "feat(desktop): local WS bridge embedding the agent runtime + get_local_ws_url"
```

---

### Task 5: Workspace persistence + `pick_workspace`/`get_workspace`

**Files:**
- Create: `src-tauri/src/workspace.rs`
- Modify: `src-tauri/src/lib.rs` (load persisted workspace at startup, add commands)
- Test: in `workspace.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `bridge::Bridge::set_workspace`.
- Produces:
  - `pub struct AppConfig { pub workspace: Option<std::path::PathBuf> }` with `load(&Path) -> AppConfig` and `save(&self, &Path) -> std::io::Result<()>`.
  - Commands `get_workspace(state) -> Option<String>` and `pick_workspace(app, state) -> Option<String>`.

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/workspace.rs`:

```rust
//! Persisted desktop app config (currently just the chosen workspace dir).
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub workspace: Option<PathBuf>,
}

impl AppConfig {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_round_trips_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config/app.json");
        let cfg = AppConfig { workspace: Some(PathBuf::from("/home/u/proj")) };
        cfg.save(&p).unwrap();
        let back = AppConfig::load(&p);
        assert_eq!(back.workspace, Some(PathBuf::from("/home/u/proj")));
    }

    #[test]
    fn missing_file_loads_default() {
        let back = AppConfig::load(Path::new("/no/such/file.json"));
        assert!(back.workspace.is_none());
    }
}
```

- [ ] **Step 2: Register the module and run the failing test**

In `src-tauri/src/lib.rs`, add near the top:

```rust
pub mod workspace;
```

Run: `cd src-tauri && cargo test workspace::tests`
Expected: PASS (module compiles, both tests pass).

- [ ] **Step 3: Wire persistence + commands into the app**

Update `src-tauri/src/lib.rs` — extend `AppState`, load the persisted workspace before starting the bridge, and add the two commands. Replace the file with:

```rust
pub mod bridge;
pub mod workspace;

use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

struct AppState {
    bridge: Arc<bridge::Bridge>,
    config_path: PathBuf, // app.json (persisted workspace)
}

#[tauri::command]
fn get_local_ws_url(state: tauri::State<'_, AppState>) -> String {
    state.bridge.ws_url()
}

#[tauri::command]
fn get_workspace(state: tauri::State<'_, AppState>) -> Option<String> {
    workspace::AppConfig::load(&state.config_path)
        .workspace
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
async fn pick_workspace(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    let folder = app.dialog().file().blocking_pick_folder();
    let Some(path) = folder else { return Ok(None) };
    let dir = path
        .into_path()
        .map_err(|e| e.to_string())?;
    // Persist, then reconnect the runtime to the new dir.
    let cfg = workspace::AppConfig { workspace: Some(dir.clone()) };
    cfg.save(&state.config_path).map_err(|e| e.to_string())?;
    state.bridge.set_workspace(dir.clone()).await;
    Ok(Some(dir.to_string_lossy().into_owned()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_config_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| PathBuf::from("."));
            let config_path = app_config_dir.join("app.json");
            let runtime_config_path = app_config_dir.join("agent-runtime.json");

            // Restore the last workspace, or default to $HOME.
            let workspace = workspace::AppConfig::load(&config_path)
                .workspace
                .or_else(dirs_home)
                .unwrap_or_else(|| PathBuf::from("."));

            let bridge = tauri::async_runtime::block_on(bridge::start(
                workspace,
                runtime_config_path,
                "http://localhost:8080".into(),
                "qwen3.6-35b-a3b".into(),
            ))?;
            app.manage(AppState { bridge, config_path });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_local_ws_url,
            get_workspace,
            pick_workspace
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cd src-tauri && cargo check`
Expected: PASS.

> If `blocking_pick_folder`/`into_path` names differ in the installed
> `tauri-plugin-dialog` 2.x, check `cargo doc -p tauri-plugin-dialog --open`;
> the folder-pick API returns a `FilePath` you convert to a `PathBuf`.

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add src-tauri/src/workspace.rs src-tauri/src/lib.rs
git commit -m "feat(desktop): persist + pick the agent workspace; reconnect on change"
```

---

### Task 6: `llama_health` command

**Files:**
- Create: `src-tauri/src/llama.rs`
- Modify: `src-tauri/src/lib.rs` (register module + command)
- Test: `src-tauri/tests/llama_health.rs` (new, uses `wiremock`)

**Interfaces:**
- Produces:
  - `pub struct LlamaHealth { pub ok: bool, pub model: Option<String> }` (Serialize)
  - `pub async fn check_health(base_url: &str) -> LlamaHealth`
  - Tauri command `llama_health() -> LlamaHealth` (hardcoded `http://localhost:8080`).

- [ ] **Step 1: Write the failing test**

Create `src-tauri/tests/llama_health.rs`:

```rust
use rust_agent_runtime_desktop_lib::llama::check_health;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn reports_ok_and_model_from_props() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status":"ok"})))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/props"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "default_generation_settings": { "model": "qwen3.6-35b-a3b" }
        })))
        .mount(&server).await;

    let h = check_health(&server.uri()).await;
    assert!(h.ok);
    assert_eq!(h.model.as_deref(), Some("qwen3.6-35b-a3b"));
}

#[tokio::test]
async fn reports_not_ok_when_server_down() {
    // Nothing listening on this port.
    let h = check_health("http://127.0.0.1:1").await;
    assert!(!h.ok);
    assert!(h.model.is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --test llama_health`
Expected: FAIL to compile — `module 'llama' not found`.

- [ ] **Step 3: Implement `src-tauri/src/llama.rs`**

```rust
//! Read-only health probe for the fixed local llama-server. The webview never
//! contacts :8080 directly (CSP); it calls the `llama_health` command instead.
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct LlamaHealth {
    pub ok: bool,
    pub model: Option<String>,
}

pub async fn check_health(base_url: &str) -> LlamaHealth {
    let base = base_url.trim_end_matches('/');
    let client = reqwest::Client::new();

    let ok = matches!(
        client.get(format!("{base}/health")).send().await,
        Ok(r) if r.status().is_success()
    );

    let model = match client.get(format!("{base}/props")).send().await {
        Ok(r) => r.json::<serde_json::Value>().await.ok().and_then(|v| {
            // llama-server exposes the model under default_generation_settings.model;
            // fall back to a top-level "model" if present.
            v.get("default_generation_settings")
                .and_then(|g| g.get("model"))
                .or_else(|| v.get("model"))
                .and_then(|m| m.as_str())
                .map(|s| s.to_string())
        }),
        Err(_) => None,
    };

    LlamaHealth { ok, model }
}
```

- [ ] **Step 4: Register the module + command**

In `src-tauri/src/lib.rs`:
- add `pub mod llama;` near the other `pub mod` lines;
- add the command function:

```rust
#[tauri::command]
async fn llama_health() -> llama::LlamaHealth {
    llama::check_health("http://localhost:8080").await
}
```
- add `llama_health` to the `tauri::generate_handler![…]` list.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --test llama_health`
Expected: PASS (both cases).

- [ ] **Step 6: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add src-tauri/src/llama.rs src-tauri/src/lib.rs src-tauri/tests/llama_health.rs
git commit -m "feat(desktop): llama_health command probing :8080 /health + /props"
```

---

### Task 7: Frontend transport adapter (`web/src/transport.ts`)

**Files:**
- Create: `web/src/transport.ts`
- Create: `web/src/transport.test.ts`
- Modify: `web/package.json` (add `@tauri-apps/api`)

**Interfaces:**
- Produces:
  - `export function isTauri(): boolean`
  - `export interface Transport { wsUrl: string; sessionId: string; needsPairing: boolean }`
  - `export async function resolveTransport(): Promise<Transport>` — Tauri: `{ wsUrl: invoke('get_local_ws_url'), sessionId: persisted UUID, needsPairing: false }`; browser: `{ wsUrl: '', sessionId: '', needsPairing: true }`.

- [ ] **Step 1: Add the dependency**

```bash
cd /home/kalen/rust-agent-runtime/web && npm install @tauri-apps/api@^2
```

- [ ] **Step 2: Write the failing test**

Create `web/src/transport.test.ts`:

```ts
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

describe("resolveTransport", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    localStorage.clear();
    delete (window as Record<string, unknown>).__TAURI_INTERNALS__;
  });
  afterEach(() => {
    delete (window as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("uses the local bridge URL and skips pairing in Tauri mode", async () => {
    (window as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValue("ws://127.0.0.1:54321/agent");
    const { resolveTransport } = await import("./transport");
    const t = await resolveTransport();
    expect(invokeMock).toHaveBeenCalledWith("get_local_ws_url");
    expect(t.wsUrl).toBe("ws://127.0.0.1:54321/agent");
    expect(t.needsPairing).toBe(false);
    expect(t.sessionId).toMatch(/[0-9a-f-]{36}/);
  });

  it("requires pairing in browser mode", async () => {
    const { resolveTransport } = await import("./transport");
    const t = await resolveTransport();
    expect(t.needsPairing).toBe(true);
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd web && npx vitest run src/transport.test.ts`
Expected: FAIL — cannot resolve `./transport`.

- [ ] **Step 4: Implement `web/src/transport.ts`**

```ts
import { invoke } from "@tauri-apps/api/core";

export interface Transport {
  wsUrl: string;
  sessionId: string;
  needsPairing: boolean;
}

// Tauri v2 with withGlobalTauri=false still injects __TAURI_INTERNALS__.
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

const SESSION_KEY = "local_session_id";

function localSessionId(): string {
  let id = localStorage.getItem(SESSION_KEY);
  if (!id) {
    id = crypto.randomUUID();
    localStorage.setItem(SESSION_KEY, id);
  }
  return id;
}

export async function resolveTransport(): Promise<Transport> {
  if (isTauri()) {
    const wsUrl = await invoke<string>("get_local_ws_url");
    return { wsUrl, sessionId: localSessionId(), needsPairing: false };
  }
  return { wsUrl: "", sessionId: "", needsPairing: true };
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd web && npx vitest run src/transport.test.ts`
Expected: PASS (both cases).

- [ ] **Step 6: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/transport.ts web/src/transport.test.ts web/package.json web/package-lock.json
git commit -m "feat(web): transport adapter — local bridge in Tauri, pairing in browser"
```

---

### Task 8: Wire `App.tsx` to Tauri mode (bypass pairing)

**Files:**
- Modify: `web/src/App.tsx`
- Test: `web/src/App.tauri.test.tsx` (new)

**Interfaces:**
- Consumes: `resolveTransport` (Task 7), existing `connect` (`web/src/socket.ts`), existing `PairingScreen`.
- Produces: in Tauri mode `App` connects to the local bridge with a generated `sessionId` and never renders `PairingScreen`.

- [ ] **Step 1: Write the failing test**

Create `web/src/App.tauri.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

vi.mock("./transport", () => ({
  isTauri: () => true,
  resolveTransport: async () => ({
    wsUrl: "ws://127.0.0.1:5/agent",
    sessionId: "11111111-1111-1111-1111-111111111111",
    needsPairing: false,
  }),
}));

// A no-op socket so connect() doesn't open a real WebSocket in jsdom.
vi.mock("./socket", () => ({
  connect: () => ({ send: vi.fn(), close: vi.fn() }),
}));

describe("App in Tauri mode", () => {
  beforeEach(() => localStorage.clear());

  it("skips the pairing screen and shows the main UI", async () => {
    const App = (await import("./App")).default;
    render(<App />);
    await waitFor(() => {
      // PairingScreen renders a pairing-code prompt; it must be absent.
      expect(screen.queryByText(/pairing/i)).toBeNull();
    });
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && npx vitest run src/App.tauri.test.tsx`
Expected: FAIL — `App` still gates on `token`/`sessionId` and renders `PairingScreen` (or throws because the new wiring doesn't exist).

- [ ] **Step 3: Implement the Tauri-mode branch in `App.tsx`**

At the top of `web/src/App.tsx`, add the import:

```tsx
import { resolveTransport, isTauri } from "./transport";
```

Add a transport state and resolve it on mount. Inside `export default function App()`, after the existing `useState` hooks, add:

```tsx
  const [localUrl, setLocalUrl] = useState<string | null>(null);
  const tauri = isTauri();

  useEffect(() => {
    if (!tauri) return;
    let active = true;
    resolveTransport().then((t) => {
      if (!active) return;
      setLocalUrl(t.wsUrl);
      setSessionId(t.sessionId); // satisfies the existing sessionId gate
    });
    return () => { active = false; };
  }, [tauri]);
```

Change the connection effect so Tauri mode uses `localUrl` instead of `wsUrl(token)` and does not require a token. Replace the existing connect `useEffect` (the one keyed on `[token, sessionId]`) with:

```tsx
  useEffect(() => {
    if (!sessionId) return;
    if (tauri ? !localUrl : !token) return;
    dispatch({ type: "reset", userMsgs: loadUserMsgs(sessionId) });
    const WebSocketImpl = (window as unknown as { __WS__?: typeof WebSocket }).__WS__;
    const url = tauri ? (localUrl as string) : wsUrl(token as string);
    sock.current = connect(
      url,
      { onFrame: (f) => dispatch({ type: "frame", frame: f }), onStatus: (s) => dispatch({ type: "status", status: s }) },
      WebSocketImpl ? { WebSocketImpl } : undefined,
    );
    return () => { sock.current?.close(); sock.current = null; };
  }, [token, sessionId, tauri, localUrl]);
```

Change the pairing gate so it only applies in browser mode. Replace:

```tsx
  if (!token || !sessionId) {
```

with:

```tsx
  if (!tauri && (!token || !sessionId)) {
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd web && npx vitest run src/App.tauri.test.tsx`
Expected: PASS.

- [ ] **Step 5: Run the full web test suite + typecheck (no regressions)**

Run: `cd web && npx vitest run && npm run typecheck`
Expected: PASS — existing `App.test.tsx` (browser/pairing mode) still passes.

- [ ] **Step 6: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/App.tsx web/src/App.tauri.test.tsx
git commit -m "feat(web): connect to the local bridge and skip pairing in Tauri mode"
```

---

### Task 9: TopBar workspace control (Tauri only)

**Files:**
- Modify: `web/src/components/TopBar.tsx`
- Modify: `web/src/App.tsx` (pass workspace + change handler to TopBar)
- Test: `web/src/components/TopBar.workspace.test.tsx` (new)

**Interfaces:**
- Consumes: `invoke('pick_workspace')` from `@tauri-apps/api/core`; existing `TopBar` props (`projectLabel, online, status, theme, onToggleTheme, onSignOut` required; `onOpenSettings, settingsDisabled, onToggleWorkspace, showWorkspaceToggle` optional — at `web/src/components/TopBar.tsx`).
- Produces: two new optional `TopBar` props — `tauriWorkspace?: string` and `onWorkspaceChanged?: (path: string) => void` — rendering a path + "Change…" button only when `tauriWorkspace` is set.

- [ ] **Step 1: Write the failing test**

Create `web/src/components/TopBar.workspace.test.tsx` (required props copied verbatim from the current `TopBar` signature):

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TopBar } from "./TopBar";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

describe("TopBar workspace control", () => {
  it("shows the workspace and calls pick_workspace on click in Tauri mode", async () => {
    invokeMock.mockReset().mockResolvedValue("/home/u/new");
    const onChanged = vi.fn();
    render(
      <TopBar
        projectLabel="proj"
        online={true}
        status="open"
        theme="dark"
        onToggleTheme={() => {}}
        onSignOut={() => {}}
        tauriWorkspace="/home/u/proj"
        onWorkspaceChanged={onChanged}
      />,
    );
    expect(screen.getByText("/home/u/proj")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: /change/i }));
    expect(invokeMock).toHaveBeenCalledWith("pick_workspace");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && npx vitest run src/components/TopBar.workspace.test.tsx`
Expected: FAIL — `TopBar` doesn't accept `tauriWorkspace`/`onWorkspaceChanged` and renders no "Change" button.

- [ ] **Step 3: Add the optional props + control to `TopBar.tsx`**

Add the import at the top:

```tsx
import { invoke } from "@tauri-apps/api/core";
```

Extend the destructured params and the props type. Change the function signature's destructuring to include the two new props, and add to the inline type:

```tsx
    onToggleWorkspace?: () => void; showWorkspaceToggle?: boolean;
    tauriWorkspace?: string; onWorkspaceChanged?: (path: string) => void }) {
```

(Add `tauriWorkspace, onWorkspaceChanged` to the destructuring list at the top of `TopBar({ … })` too.)

Render the control inside the right-side cluster — the `<div className="flex items-center gap-3 text-sm">` — immediately before `<ThemeToggle … />`:

```tsx
        {tauriWorkspace !== undefined && (
          <div className="flex items-center gap-2 text-xs" style={{ color: "var(--text-muted)" }}>
            <span title={tauriWorkspace} className="max-w-[28ch] truncate">{tauriWorkspace}</span>
            <button
              type="button"
              className="rounded-full px-3 py-1 hover:opacity-80"
              style={{ border: "1px solid var(--border)", color: "var(--text)" }}
              onClick={async () => {
                const picked = await invoke<string | null>("pick_workspace");
                if (picked) onWorkspaceChanged?.(picked);
              }}
            >
              Change…
            </button>
          </div>
        )}
```

- [ ] **Step 4: Pass workspace from `App.tsx`**

In `web/src/App.tsx`, track and load the workspace in Tauri mode and pass it to `TopBar`. Add to the Tauri `useEffect` from Task 8 (after `setLocalUrl`):

```tsx
      import("@tauri-apps/api/core").then(({ invoke }) =>
        invoke<string | null>("get_workspace").then((w) => { if (active && w) setWorkspace(w); }));
```

Add the state near the other `useState`s:

```tsx
  const [workspace, setWorkspace] = useState<string | undefined>(undefined);
```

Find the existing `<TopBar ... />` render and add the two props:

```tsx
        tauriWorkspace={tauri ? workspace : undefined}
        onWorkspaceChanged={(p) => setWorkspace(p)}
```

(The bridge drops the socket on change; `socket.ts` auto-reconnects into the new workspace — no extra reconnect code needed.)

- [ ] **Step 5: Run the test + typecheck**

Run: `cd web && npx vitest run src/components/TopBar.workspace.test.tsx && npm run typecheck`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/components/TopBar.tsx web/src/App.tsx web/src/components/TopBar.workspace.test.tsx
git commit -m "feat(web): TopBar workspace path + Change… picker in Tauri mode"
```

---

### Task 10: End-to-end dev run + Linux bundle verification

**Files:**
- Modify: repo root `package.json` (add `tauri` convenience scripts)
- No new source; this task verifies the whole app.

**Interfaces:**
- Consumes: everything above.

- [ ] **Step 1: Add convenience scripts**

In the repo-root `package.json` `"scripts"`, add:

```json
"tauri": "tauri",
"desktop:dev": "tauri dev",
"desktop:build": "tauri build"
```

- [ ] **Step 2: Confirm the full Rust + web suites are green**

Run:
```bash
cd /home/kalen/rust-agent-runtime/agent && cargo test
cd /home/kalen/rust-agent-runtime/src-tauri && cargo test
cd /home/kalen/rust-agent-runtime/web && npx vitest run && npm run typecheck
```
Expected: all PASS.

- [ ] **Step 3: Start the llama-server (fixed) if not already running**

```bash
docker run -d --name llama-agent --gpus all -p 8080:8080 \
  -v /mnt/storage/models:/models:ro ghcr.io/ggml-org/llama.cpp:server-cuda \
  -m /models/qwen3.6-35b-a3b-gguf/Qwen3.6-35B-A3B-UD-IQ4_XS.gguf \
  -a qwen3.6-35b-a3b -ngl 99 -np 1 -c 262144 -fa on \
  --cache-type-k q8_0 --cache-type-v q8_0 --host 0.0.0.0 --port 8080 --jinja
# verify:
curl -s localhost:8080/health   # -> {"status":"ok"}
```

- [ ] **Step 4: Run the desktop app in dev and verify the acceptance criteria**

Run: `cd /home/kalen/rust-agent-runtime && npx tauri dev`
Verify by observation:
- A native window opens showing the existing UI with **no pairing screen**.
- On first launch the workspace defaults to `$HOME`; the TopBar shows it with a **Change…** button; clicking it opens a native folder dialog and the path updates.
- Typing a message streams tokens/tool events back; an action requiring approval prompts and resolves; the Settings panel loads and edits apply live.
- After changing the workspace, a new message's file/tool operations act on the new directory.

- [ ] **Step 5: Build the Linux bundles**

Run: `cd /home/kalen/rust-agent-runtime && npx tauri build`
Expected: success; artifacts under `src-tauri/target/release/bundle/deb/*.deb` and `.../appimage/*.AppImage`.

> If the build fails on missing system libs, install the Tauri Linux deps:
> `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev
> libayatana-appindicator3-dev librsvg2-dev` (see `.agents/skills/tauri/SKILL.md`).

- [ ] **Step 6: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add package.json
git commit -m "chore(desktop): add tauri dev/build scripts; verified local desktop app"
```

---

## Notes for the implementer

- **Cloud path is sacrosanct.** Do not edit the Worker (`cloud/`), the daemon's
  `Cmd::Enroll`/`Cmd::Run` flags, or `main.rs` assembly. Task 1 only *extracts*
  `serve()`; `run()` keeps behaving identically (the existing
  `daemon_roundtrip.rs` test is the guard).
- **Version match matters.** `tokio-tungstenite` must be `0.24` in `src-tauri`
  so `WebSocketStream<TcpStream>` from `accept_async` is the same type
  `serve()` expects. A mismatch shows up as a confusing trait error.
- **One connection at a time.** The bridge tracks a single active `serve` task
  (matches the daemon's one-session MVP). Reconnect-on-workspace-change relies
  on `socket.ts`'s existing auto-reconnect.
- **Plugin API drift.** `tauri-plugin-dialog` 2.x folder-pick returns a
  `FilePath`; if `blocking_pick_folder().into_path()` doesn't compile, consult
  `cargo doc -p tauri-plugin-dialog`.
