# Rust Agent Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a trait-decoupled, event-driven ReAct agent core in Rust — a CLI-runnable agent that talks to an OpenAI-compatible inference server (SGLang-first), executes fs/shell/git tools through a real permission engine, and streams its work.

**Architecture:** A Cargo workspace of 5 crates with an acyclic dependency graph. `agent-tools` holds the shared tool vocabulary and the `Tool` trait; `agent-policy` and `agent-model` build on it; `agent-core` owns the ReAct loop, context manager, and event model; `agent-cli` wires everything and supplies the terminal UI. Every cross-crate dependency is a trait, so the model backend, tool-call protocol, tools, policy, approval channel, and event sink are all swappable. The loop is UI-agnostic: it emits `AgentEvent`s to an `EventSink` and requests approvals through an `ApprovalChannel`, the two seams the future web layer taps.

**Tech Stack:** Rust (edition 2021), Tokio, `async-trait`, `serde`/`serde_json`, `reqwest` (streaming), `futures`, `tokio-util` (`CancellationToken`), `thiserror`, `tracing`, `similar` (diffs), `clap` (CLI). Dev: `wiremock`, `tempfile`.

## Global Constraints

- Rust **edition 2021**; build on stable toolchain.
- Library crates (`agent-tools`, `agent-policy`, `agent-model`, `agent-core`) **must not `panic!`/`unwrap`/`expect`** on reachable paths — return `Result`. `unwrap` is allowed only in `#[cfg(test)]` code.
- All async via **Tokio**; trait async via **`async-trait`**.
- All public domain types derive `Debug` + `Clone` where practical; wire types derive `serde::Serialize`/`Deserialize`.
- Dependency direction is fixed and **must stay acyclic**: `agent-tools` → (depended on by) `agent-policy`, `agent-model` → `agent-core` → `agent-cli`. No crate may depend "upward".
- Every task ends green: `cargo test -p <crate>` passes and `cargo clippy --all-targets -- -D warnings` is clean for touched crates.
- Workspace lives under `agent/`. All paths below are relative to the repo root `/home/kalen/rust-agent-runtime`.

---

## Shared Type Reference (authoritative signatures)

These types are defined across Tasks 2–13. Listed here once so any task can be implemented in isolation. **Do not redefine — import from the owning crate.**

```rust
// ───── agent-tools ─────
pub struct ToolSchema { pub name: String, pub description: String, pub parameters: serde_json::Value }
pub struct ToolCall   { pub id: String, pub name: String, pub args: serde_json::Value }
pub enum   Access     { Read, Write }
pub struct ToolIntent { pub tool: String, pub access: Access, pub paths: Vec<std::path::PathBuf>,
                        pub command: Option<String>, pub summary: String }
pub enum   Display    { Text(String),
                        Diff { path: String, before: String, after: String },
                        Terminal { command: String, stdout: String, stderr: String, exit_code: i32 } }
pub struct ToolOutput { pub content: String, pub display: Option<Display> }
pub enum   ToolError  { Denied(String), Timeout, NotFound(String),
                        Failed { message: String, stderr: Option<String> }, InvalidArgs(String) }
pub struct ToolCtx    { pub workspace: std::path::PathBuf, pub timeout: std::time::Duration,
                        pub cancel: tokio_util::sync::CancellationToken }
#[async_trait] pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> ToolSchema;
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError>;
    async fn execute(&self, args: serde_json::Value, ctx: &ToolCtx) -> Result<ToolOutput, ToolError>;
}
pub struct ToolRegistry { /* name -> Arc<dyn Tool> */ }
// new(), register(Arc<dyn Tool>), get(&str)->Option<Arc<dyn Tool>>, schemas()->Vec<ToolSchema>

// ───── agent-policy ─────  (depends on agent-tools)
pub enum Decision { Allow, Deny(String), Ask }
pub trait PolicyEngine: Send + Sync { fn check(&self, intent: &ToolIntent) -> Decision; }
pub struct ApprovalRequest  { pub intent: ToolIntent, pub display: Option<Display> }
pub enum   ApprovalResponse { Approve, ApproveAlways, Deny }
#[async_trait] pub trait ApprovalChannel: Send + Sync {
    async fn request(&self, req: ApprovalRequest) -> ApprovalResponse;
}
pub struct RulePolicy { pub workspace: PathBuf, pub command_allowlist: Vec<String>, pub command_denylist: Vec<String> }

// ───── agent-model ─────  (depends on agent-tools)
pub enum   Role    { System, User, Assistant, Tool }
pub struct Message { pub role: Role, pub content: String,
                     pub tool_calls: Option<Vec<ToolCall>>,   // assistant turns
                     pub tool_call_id: Option<String>,        // tool-result turns
                     pub name: Option<String> }               // tool name on results
pub struct CompletionRequest { pub messages: Vec<Message>, pub tools: Vec<ToolSchema>,
                               pub temperature: f32, pub max_tokens: Option<u32> }
pub enum   StopReason { Stop, ToolCalls, Length, BudgetExhausted }
pub struct RawToolCall { pub id: Option<String>, pub name: Option<String>, pub args_fragment: String }
pub enum   Chunk { Text(String), ToolCallDelta(RawToolCall), Done(StopReason) }
pub struct AssistantTurn { pub text: String, pub raw_tool_calls: Vec<RawToolCall>, pub stop: StopReason }
pub struct ParsedTurn { pub text: String, pub tool_calls: Vec<ToolCall> }
pub struct ProtocolError(pub String);
pub enum   ModelError { Http(String), Status(u16), Decode(String), Stream(String) }
pub trait ToolCallProtocol: Send + Sync {
    fn prepare(&self, req: &mut CompletionRequest);
    fn parse(&self, raw: &AssistantTurn) -> Result<ParsedTurn, ProtocolError>;
}
#[async_trait] pub trait ModelClient: Send + Sync {
    async fn stream(&self, req: CompletionRequest)
        -> Result<futures::stream::BoxStream<'static, Result<Chunk, ModelError>>, ModelError>;
}
pub struct NativeProtocol;
pub struct PromptedJsonProtocol;
pub struct OpenAiCompatClient { /* base_url, model, http client, api_key: Option<String> */ }

// ───── agent-core ─────  (depends on all three above)
pub enum AgentEvent {
    Token(String),
    ToolStart  { name: String, args: serde_json::Value },
    ToolResult { name: String, output: ToolOutput },
    Approval(ApprovalRequest),
    Error(String),
    Done(StopReason),
}
pub trait EventSink: Send + Sync { fn emit(&self, event: AgentEvent); }
pub trait ContextManager: Send + Sync {
    fn append(&mut self, msg: Message);
    fn build(&self, model_limit: usize) -> Vec<Message>;
}
pub struct WindowContext { /* system: Message, history: Vec<Message> */ }
pub struct LoopConfig { pub model_limit: usize, pub max_turns: usize, pub max_retries: usize,
                        pub temperature: f32, pub max_tokens: Option<u32>,
                        pub workspace: PathBuf, pub tool_timeout: Duration }
pub struct AgentLoop { /* Arc<dyn ModelClient>, Arc<dyn ToolCallProtocol>, Arc<ToolRegistry>,
                          Arc<dyn PolicyEngine>, Arc<dyn ApprovalChannel>, Arc<dyn EventSink>, LoopConfig */ }
// AgentLoop::run(&self, ctx: &mut dyn ContextManager, user_input: String) -> Result<(), AgentError>
```

> **Note — refinement vs. spec:** `Tool::intent` returns `Result<ToolIntent, ToolError>` (not bare `ToolIntent`) so argument parsing failures surface cleanly. `AgentEvent::Approval` is the variant name for the spec's `ApprovalRequest` event. These are the only deviations; both are noted in the spec's §5 as illustrative signatures.

---

## Task 1: Workspace scaffold

**Files:**
- Create: `agent/Cargo.toml` (workspace)
- Create: `agent/crates/agent-tools/Cargo.toml`, `agent/crates/agent-tools/src/lib.rs`
- Create: `agent/crates/agent-policy/Cargo.toml`, `agent/crates/agent-policy/src/lib.rs`
- Create: `agent/crates/agent-model/Cargo.toml`, `agent/crates/agent-model/src/lib.rs`
- Create: `agent/crates/agent-core/Cargo.toml`, `agent/crates/agent-core/src/lib.rs`
- Create: `agent/crates/agent-cli/Cargo.toml`, `agent/crates/agent-cli/src/main.rs`
- Create: `agent/.gitignore`

**Interfaces:**
- Consumes: nothing.
- Produces: a compiling 5-crate workspace.

- [ ] **Step 1: Write the workspace manifest**

`agent/Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
edition = "2021"
license = "MIT"

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
futures = "0.3"
tokio-util = "0.7"
thiserror = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "stream", "rustls-tls"] }
similar = "2"
clap = { version = "4", features = ["derive"] }
# dev
wiremock = "0.6"
tempfile = "3"
```

- [ ] **Step 2: Write each crate's `Cargo.toml`**

`agent/crates/agent-tools/Cargo.toml`:
```toml
[package]
name = "agent-tools"
edition.workspace = true

[dependencies]
async-trait.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio.workspace = true
tokio-util.workspace = true
similar.workspace = true
tracing.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

`agent/crates/agent-policy/Cargo.toml`:
```toml
[package]
name = "agent-policy"
edition.workspace = true

[dependencies]
agent-tools = { path = "../agent-tools" }
async-trait.workspace = true
tracing.workspace = true
```

`agent/crates/agent-model/Cargo.toml`:
```toml
[package]
name = "agent-model"
edition.workspace = true

[dependencies]
agent-tools = { path = "../agent-tools" }
async-trait.workspace = true
serde.workspace = true
serde_json.workspace = true
futures.workspace = true
reqwest.workspace = true
thiserror.workspace = true
tracing.workspace = true

[dev-dependencies]
tokio = { workspace = true }
wiremock.workspace = true
```

`agent/crates/agent-core/Cargo.toml`:
```toml
[package]
name = "agent-core"
edition.workspace = true

[dependencies]
agent-tools = { path = "../agent-tools" }
agent-policy = { path = "../agent-policy" }
agent-model = { path = "../agent-model" }
async-trait.workspace = true
serde_json.workspace = true
futures.workspace = true
tokio.workspace = true
tokio-util.workspace = true
thiserror.workspace = true
tracing.workspace = true

[dev-dependencies]
tokio = { workspace = true }
```

`agent/crates/agent-cli/Cargo.toml`:
```toml
[package]
name = "agent-cli"
edition.workspace = true

[[bin]]
name = "agent"
path = "src/main.rs"

[dependencies]
agent-core = { path = "../agent-core" }
agent-tools = { path = "../agent-tools" }
agent-policy = { path = "../agent-policy" }
agent-model = { path = "../agent-model" }
tokio.workspace = true
async-trait.workspace = true
serde.workspace = true
serde_json.workspace = true
clap.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
similar.workspace = true
```

- [ ] **Step 3: Write placeholder lib/main files**

Each library `src/lib.rs` (all four):
```rust
//! Crate root. Modules added by later tasks.
```

`agent/crates/agent-cli/src/main.rs`:
```rust
fn main() {
    println!("agent-cli scaffold");
}
```

`agent/.gitignore`:
```
/target
```

- [ ] **Step 4: Verify the workspace builds**

Run: `cd agent && cargo build`
Expected: compiles, all five crates listed, no errors.

- [ ] **Step 5: Commit**

```bash
git add agent/
git commit -m "chore: scaffold agent workspace (5 crates)"
```

---

## Task 2: Tool vocabulary types

**Files:**
- Create: `agent/crates/agent-tools/src/types.rs`
- Modify: `agent/crates/agent-tools/src/lib.rs`
- Test: inline `#[cfg(test)]` in `types.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `ToolSchema`, `ToolCall`, `Access`, `ToolIntent`, `Display`, `ToolOutput`, `ToolError` (see Shared Type Reference).

- [ ] **Step 1: Write the failing test**

In `agent/crates/agent-tools/src/types.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_schema_serializes_to_openai_function_shape() {
        let s = ToolSchema {
            name: "read_file".into(),
            description: "Read a file".into(),
            parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["name"], "read_file");
        assert_eq!(v["parameters"]["type"], "object");
    }

    #[test]
    fn tool_error_carries_context() {
        let e = ToolError::Failed { message: "boom".into(), stderr: Some("trace".into()) };
        match e {
            ToolError::Failed { message, stderr } => {
                assert_eq!(message, "boom");
                assert_eq!(stderr.as_deref(), Some("trace"));
            }
            _ => panic!("wrong variant"),
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-tools`
Expected: FAIL — `cannot find type ToolSchema`.

- [ ] **Step 3: Write the types**

Prepend to `agent/crates/agent-tools/src/types.rs`:
```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    /// JSON Schema object describing the arguments.
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Access { Read, Write }

#[derive(Debug, Clone)]
pub struct ToolIntent {
    pub tool: String,
    pub access: Access,
    pub paths: Vec<PathBuf>,
    pub command: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Display {
    Text(String),
    Diff { path: String, before: String, after: String },
    Terminal { command: String, stdout: String, stderr: String, exit_code: i32 },
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// Text returned to the model.
    pub content: String,
    /// Optional richer payload for UI rendering.
    pub display: Option<Display>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ToolError {
    #[error("denied: {0}")]
    Denied(String),
    #[error("timed out")]
    Timeout,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("failed: {message}")]
    Failed { message: String, stderr: Option<String> },
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
}

/// Execution context handed to every tool.
pub struct ToolCtx {
    pub workspace: PathBuf,
    pub timeout: Duration,
    pub cancel: CancellationToken,
}
```

- [ ] **Step 4: Export from lib and run tests**

`agent/crates/agent-tools/src/lib.rs`:
```rust
//! Shared tool vocabulary and the `Tool` trait.
mod types;
pub use types::*;
```

Run: `cd agent && cargo test -p agent-tools`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-tools/
git commit -m "feat(tools): tool vocabulary types"
```

---

## Task 3: Tool trait and ToolRegistry

**Files:**
- Create: `agent/crates/agent-tools/src/tool.rs`
- Create: `agent/crates/agent-tools/src/registry.rs`
- Modify: `agent/crates/agent-tools/src/lib.rs`
- Test: inline in `registry.rs`

**Interfaces:**
- Consumes: `ToolSchema`, `ToolIntent`, `ToolOutput`, `ToolError`, `ToolCtx` (Task 2).
- Produces: `Tool` trait; `ToolRegistry::{new, register, get, schemas}`.

- [ ] **Step 1: Write the failing test**

In `agent/crates/agent-tools/src/registry.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Arc;

    struct Echo;
    #[async_trait]
    impl Tool for Echo {
        fn name(&self) -> &str { "echo" }
        fn description(&self) -> &str { "echoes" }
        fn schema(&self) -> ToolSchema {
            ToolSchema { name: "echo".into(), description: "echoes".into(),
                         parameters: json!({"type":"object"}) }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent { tool: "echo".into(), access: Access::Read, paths: vec![],
                            command: None, summary: "echo".into() })
        }
        async fn execute(&self, args: serde_json::Value, _ctx: &ToolCtx)
            -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput { content: args.to_string(), display: None })
        }
    }

    #[test]
    fn registry_registers_and_looks_up() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(Echo));
        assert!(r.get("echo").is_some());
        assert!(r.get("missing").is_none());
        let schemas = r.schemas();
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].name, "echo");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-tools`
Expected: FAIL — `ToolRegistry`/`Tool` not found.

- [ ] **Step 3: Write the trait and registry**

`agent/crates/agent-tools/src/tool.rs`:
```rust
use crate::{ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> ToolSchema;
    /// Declare what this call will do, for the policy engine to judge before execution.
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError>;
    async fn execute(&self, args: serde_json::Value, ctx: &ToolCtx)
        -> Result<ToolOutput, ToolError>;
}
```

`agent/crates/agent-tools/src/registry.rs` (prepend above the test module):
```rust
use crate::{Tool, ToolSchema};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools.values().map(|t| t.schema()).collect()
    }
}
```

- [ ] **Step 4: Export and run tests**

`agent/crates/agent-tools/src/lib.rs`:
```rust
//! Shared tool vocabulary and the `Tool` trait.
mod types;
mod tool;
mod registry;
pub use types::*;
pub use tool::*;
pub use registry::*;
```

Run: `cd agent && cargo test -p agent-tools`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-tools/
git commit -m "feat(tools): Tool trait and ToolRegistry"
```

---

## Task 4: Path safety helper + read-only filesystem tools

**Files:**
- Create: `agent/crates/agent-tools/src/fs/mod.rs`
- Create: `agent/crates/agent-tools/src/fs/paths.rs`
- Create: `agent/crates/agent-tools/src/fs/read.rs`
- Modify: `agent/crates/agent-tools/src/lib.rs`
- Test: inline in `paths.rs` and `read.rs`

**Interfaces:**
- Consumes: `Tool`, `ToolCtx`, `ToolIntent`, `Access`, `ToolError`, `ToolOutput`, `ToolSchema`.
- Produces: `resolve_in_workspace(workspace, arg_path) -> Result<PathBuf, ToolError>`; `ReadFile`, `ListDirectory` tools.

- [ ] **Step 1: Write the failing test for path resolution**

`agent/crates/agent-tools/src/fs/paths.rs`:
```rust
use crate::ToolError;
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn resolves_relative_inside_workspace() {
        let dir = tempdir().unwrap();
        let ws = dir.path();
        std::fs::write(ws.join("a.txt"), "hi").unwrap();
        let p = resolve_in_workspace(ws, "a.txt").unwrap();
        assert!(p.starts_with(ws));
    }

    #[test]
    fn rejects_escape_with_dotdot() {
        let dir = tempdir().unwrap();
        let ws = dir.path();
        let err = resolve_in_workspace(ws, "../escape.txt").unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-tools paths`
Expected: FAIL — `resolve_in_workspace` not found.

- [ ] **Step 3: Implement path resolution**

Prepend to `paths.rs`:
```rust
/// Resolve `arg` against `workspace`, rejecting anything that escapes it.
/// Works for non-existent files (for writes) by normalizing lexically.
pub fn resolve_in_workspace(workspace: &Path, arg: &str) -> Result<PathBuf, ToolError> {
    let candidate = if Path::new(arg).is_absolute() {
        PathBuf::from(arg)
    } else {
        workspace.join(arg)
    };
    let normalized = normalize(&candidate);
    let ws_norm = normalize(workspace);
    if normalized.starts_with(&ws_norm) {
        Ok(normalized)
    } else {
        Err(ToolError::Denied(format!("path escapes workspace: {arg}")))
    }
}

/// Lexical normalization that collapses `.` and `..` without touching the filesystem.
fn normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => { out.pop(); }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}
```

- [ ] **Step 4: Run path tests**

Run: `cd agent && cargo test -p agent-tools paths`
Expected: PASS (2 tests).

- [ ] **Step 5: Write the failing test for read tools**

`agent/crates/agent-tools/src/fs/read.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use serde_json::json;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    fn ctx(ws: std::path::PathBuf) -> ToolCtx {
        ToolCtx { workspace: ws, timeout: Duration::from_secs(5), cancel: CancellationToken::new() }
    }

    #[tokio::test]
    async fn read_file_returns_contents() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let out = ReadFile.execute(json!({"path":"a.txt"}), &ctx(dir.path().into())).await.unwrap();
        assert_eq!(out.content, "hello");
    }

    #[test]
    fn read_file_intent_is_read_access() {
        let intent = ReadFile.intent(&json!({"path":"a.txt"})).unwrap();
        assert_eq!(intent.access, Access::Read);
        assert_eq!(intent.tool, "read_file");
    }

    #[tokio::test]
    async fn list_directory_lists_entries() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("x.txt"), "").unwrap();
        let out = ListDirectory.execute(json!({"path":"."}), &ctx(dir.path().into())).await.unwrap();
        assert!(out.content.contains("x.txt"));
    }
}
```

- [ ] **Step 6: Run to verify it fails**

Run: `cd agent && cargo test -p agent-tools read`
Expected: FAIL — `ReadFile`/`ListDirectory` not found.

- [ ] **Step 7: Implement the read tools**

Prepend to `read.rs`:
```rust
use crate::fs::paths::resolve_in_workspace;
use crate::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;

fn arg_path(args: &serde_json::Value) -> Result<String, ToolError> {
    args.get("path").and_then(|v| v.as_str()).map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs("missing string field `path`".into()))
}

pub struct ReadFile;

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &str { "read_file" }
    fn description(&self) -> &str { "Read the contents of a file within the workspace." }
    fn schema(&self) -> ToolSchema {
        ToolSchema { name: self.name().into(), description: self.description().into(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}) }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let path = arg_path(args)?;
        Ok(ToolIntent { tool: "read_file".into(), access: Access::Read,
            paths: vec![path.clone().into()], command: None, summary: format!("read {path}") })
    }
    async fn execute(&self, args: serde_json::Value, ctx: &ToolCtx)
        -> Result<ToolOutput, ToolError> {
        let path = arg_path(&args)?;
        let full = resolve_in_workspace(&ctx.workspace, &path)?;
        let content = tokio::fs::read_to_string(&full).await
            .map_err(|e| ToolError::NotFound(format!("{path}: {e}")))?;
        Ok(ToolOutput { content, display: None })
    }
}

pub struct ListDirectory;

#[async_trait]
impl Tool for ListDirectory {
    fn name(&self) -> &str { "list_directory" }
    fn description(&self) -> &str { "List entries of a directory within the workspace." }
    fn schema(&self) -> ToolSchema {
        ToolSchema { name: self.name().into(), description: self.description().into(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}) }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let path = arg_path(args)?;
        Ok(ToolIntent { tool: "list_directory".into(), access: Access::Read,
            paths: vec![path.clone().into()], command: None, summary: format!("list {path}") })
    }
    async fn execute(&self, args: serde_json::Value, ctx: &ToolCtx)
        -> Result<ToolOutput, ToolError> {
        let path = arg_path(&args)?;
        let full = resolve_in_workspace(&ctx.workspace, &path)?;
        let mut entries = tokio::fs::read_dir(&full).await
            .map_err(|e| ToolError::NotFound(format!("{path}: {e}")))?;
        let mut names = Vec::new();
        while let Some(e) = entries.next_entry().await
            .map_err(|e| ToolError::Failed { message: e.to_string(), stderr: None })? {
            names.push(e.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(ToolOutput { content: names.join("\n"), display: None })
    }
}
```

`agent/crates/agent-tools/src/fs/mod.rs`:
```rust
pub mod paths;
pub mod read;
pub use paths::resolve_in_workspace;
pub use read::{ListDirectory, ReadFile};
```

- [ ] **Step 8: Export and run all tool tests**

Add to `agent/crates/agent-tools/src/lib.rs`:
```rust
pub mod fs;
```

Run: `cd agent && cargo test -p agent-tools`
Expected: PASS (all tests, incl. read + paths).

- [ ] **Step 9: Commit**

```bash
git add agent/crates/agent-tools/
git commit -m "feat(tools): path safety + read_file/list_directory"
```

---

## Task 5: Write/edit filesystem tools (with diffs)

**Files:**
- Create: `agent/crates/agent-tools/src/fs/write.rs`
- Modify: `agent/crates/agent-tools/src/fs/mod.rs`
- Test: inline in `write.rs`

**Interfaces:**
- Consumes: `Tool`, `resolve_in_workspace`, `Display::Diff`, `Access::Write`.
- Produces: `WriteFile`, `EditFile` tools (both emit `display: Some(Display::Diff{..})`).

- [ ] **Step 1: Write the failing test**

`agent/crates/agent-tools/src/fs/write.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use serde_json::json;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    fn ctx(ws: std::path::PathBuf) -> ToolCtx {
        ToolCtx { workspace: ws, timeout: Duration::from_secs(5), cancel: CancellationToken::new() }
    }

    #[tokio::test]
    async fn write_file_creates_and_returns_diff() {
        let dir = tempdir().unwrap();
        let out = WriteFile.execute(json!({"path":"new.txt","content":"hi\n"}),
            &ctx(dir.path().into())).await.unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("new.txt")).unwrap(), "hi\n");
        assert!(matches!(out.display, Some(Display::Diff { .. })));
    }

    #[test]
    fn write_file_intent_is_write() {
        let i = WriteFile.intent(&json!({"path":"a","content":"b"})).unwrap();
        assert_eq!(i.access, Access::Write);
    }

    #[tokio::test]
    async fn edit_file_replaces_unique_substring() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "foo bar baz").unwrap();
        EditFile.execute(json!({"path":"a.txt","old":"bar","new":"QUX"}),
            &ctx(dir.path().into())).await.unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(), "foo QUX baz");
    }

    #[tokio::test]
    async fn edit_file_errors_when_old_not_unique() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "x x").unwrap();
        let err = EditFile.execute(json!({"path":"a.txt","old":"x","new":"y"}),
            &ctx(dir.path().into())).await.unwrap_err();
        assert!(matches!(err, ToolError::Failed { .. }));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-tools write`
Expected: FAIL — `WriteFile`/`EditFile` not found.

- [ ] **Step 3: Implement write/edit tools**

Prepend to `write.rs`:
```rust
use crate::fs::paths::resolve_in_workspace;
use crate::{Access, Display, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;

fn str_arg(args: &serde_json::Value, key: &str) -> Result<String, ToolError> {
    args.get(key).and_then(|v| v.as_str()).map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing string field `{key}`")))
}

fn diff(path: &str, before: &str, after: &str) -> Display {
    Display::Diff { path: path.into(), before: before.into(), after: after.into() }
}

pub struct WriteFile;

#[async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &str { "write_file" }
    fn description(&self) -> &str { "Create or overwrite a file within the workspace." }
    fn schema(&self) -> ToolSchema {
        ToolSchema { name: self.name().into(), description: self.description().into(),
            parameters: json!({"type":"object","properties":{
                "path":{"type":"string"},"content":{"type":"string"}},
                "required":["path","content"]}) }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let path = str_arg(args, "path")?;
        Ok(ToolIntent { tool: "write_file".into(), access: Access::Write,
            paths: vec![path.clone().into()], command: None, summary: format!("write {path}") })
    }
    async fn execute(&self, args: serde_json::Value, ctx: &ToolCtx)
        -> Result<ToolOutput, ToolError> {
        let path = str_arg(&args, "path")?;
        let content = str_arg(&args, "content")?;
        let full = resolve_in_workspace(&ctx.workspace, &path)?;
        let before = tokio::fs::read_to_string(&full).await.unwrap_or_default();
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent).await
                .map_err(|e| ToolError::Failed { message: e.to_string(), stderr: None })?;
        }
        tokio::fs::write(&full, &content).await
            .map_err(|e| ToolError::Failed { message: e.to_string(), stderr: None })?;
        Ok(ToolOutput { content: format!("wrote {} bytes to {path}", content.len()),
            display: Some(diff(&path, &before, &content)) })
    }
}

pub struct EditFile;

#[async_trait]
impl Tool for EditFile {
    fn name(&self) -> &str { "edit_file" }
    fn description(&self) -> &str { "Replace a unique substring in a workspace file." }
    fn schema(&self) -> ToolSchema {
        ToolSchema { name: self.name().into(), description: self.description().into(),
            parameters: json!({"type":"object","properties":{
                "path":{"type":"string"},"old":{"type":"string"},"new":{"type":"string"}},
                "required":["path","old","new"]}) }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let path = str_arg(args, "path")?;
        Ok(ToolIntent { tool: "edit_file".into(), access: Access::Write,
            paths: vec![path.clone().into()], command: None, summary: format!("edit {path}") })
    }
    async fn execute(&self, args: serde_json::Value, ctx: &ToolCtx)
        -> Result<ToolOutput, ToolError> {
        let path = str_arg(&args, "path")?;
        let old = str_arg(&args, "old")?;
        let new = str_arg(&args, "new")?;
        let full = resolve_in_workspace(&ctx.workspace, &path)?;
        let before = tokio::fs::read_to_string(&full).await
            .map_err(|e| ToolError::NotFound(format!("{path}: {e}")))?;
        let count = before.matches(&old).count();
        if count != 1 {
            return Err(ToolError::Failed {
                message: format!("`old` matched {count} times; must match exactly once"),
                stderr: None });
        }
        let after = before.replacen(&old, &new, 1);
        tokio::fs::write(&full, &after).await
            .map_err(|e| ToolError::Failed { message: e.to_string(), stderr: None })?;
        Ok(ToolOutput { content: format!("edited {path}"),
            display: Some(diff(&path, &before, &after)) })
    }
}
```

Update `agent/crates/agent-tools/src/fs/mod.rs`:
```rust
pub mod paths;
pub mod read;
pub mod write;
pub use paths::resolve_in_workspace;
pub use read::{ListDirectory, ReadFile};
pub use write::{EditFile, WriteFile};
```

- [ ] **Step 4: Run tests**

Run: `cd agent && cargo test -p agent-tools`
Expected: PASS (all, incl. 4 new write/edit tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-tools/
git commit -m "feat(tools): write_file/edit_file with diffs"
```

---

## Task 6: Shell tool (`execute_command`)

**Files:**
- Create: `agent/crates/agent-tools/src/shell.rs`
- Modify: `agent/crates/agent-tools/src/lib.rs`
- Test: inline in `shell.rs`

**Interfaces:**
- Consumes: `Tool`, `ToolCtx`, `Display::Terminal`, `Access::Write`.
- Produces: `ExecuteCommand` tool. Intent sets `command: Some(<cmdline>)` so policy can allowlist it. Runs under `ctx.timeout` with cancellation.

- [ ] **Step 1: Write the failing test**

`agent/crates/agent-tools/src/shell.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use serde_json::json;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn ctx(timeout: Duration) -> ToolCtx {
        ToolCtx { workspace: std::env::temp_dir(), timeout, cancel: CancellationToken::new() }
    }

    #[tokio::test]
    async fn runs_command_and_captures_stdout() {
        let out = ExecuteCommand.execute(json!({"command":"echo hello"}),
            &ctx(Duration::from_secs(5))).await.unwrap();
        assert!(out.content.contains("hello"));
        assert!(matches!(out.display, Some(Display::Terminal { exit_code: 0, .. })));
    }

    #[test]
    fn intent_carries_command_string() {
        let i = ExecuteCommand.intent(&json!({"command":"ls -la"})).unwrap();
        assert_eq!(i.command.as_deref(), Some("ls -la"));
        assert_eq!(i.access, Access::Write);
    }

    #[tokio::test]
    async fn times_out_long_command() {
        let err = ExecuteCommand.execute(json!({"command":"sleep 5"}),
            &ctx(Duration::from_millis(200))).await.unwrap_err();
        assert!(matches!(err, ToolError::Timeout));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-tools shell`
Expected: FAIL — `ExecuteCommand` not found.

- [ ] **Step 3: Implement the shell tool**

Prepend to `shell.rs`:
```rust
use crate::{Access, Display, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;

pub struct ExecuteCommand;

fn cmd_arg(args: &serde_json::Value) -> Result<String, ToolError> {
    args.get("command").and_then(|v| v.as_str()).map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs("missing string field `command`".into()))
}

#[async_trait]
impl Tool for ExecuteCommand {
    fn name(&self) -> &str { "execute_command" }
    fn description(&self) -> &str { "Run a shell command in the workspace directory." }
    fn schema(&self) -> ToolSchema {
        ToolSchema { name: self.name().into(), description: self.description().into(),
            parameters: json!({"type":"object","properties":{
                "command":{"type":"string"}},"required":["command"]}) }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let command = cmd_arg(args)?;
        Ok(ToolIntent { tool: "execute_command".into(), access: Access::Write, paths: vec![],
            command: Some(command.clone()), summary: format!("run `{command}`") })
    }
    async fn execute(&self, args: serde_json::Value, ctx: &ToolCtx)
        -> Result<ToolOutput, ToolError> {
        let command = cmd_arg(&args)?;
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(&command).current_dir(&ctx.workspace)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let run = async {
            cmd.output().await
                .map_err(|e| ToolError::Failed { message: e.to_string(), stderr: None })
        };

        let output = tokio::select! {
            _ = ctx.cancel.cancelled() => return Err(ToolError::Denied("cancelled".into())),
            r = tokio::time::timeout(ctx.timeout, run) => match r {
                Err(_elapsed) => return Err(ToolError::Timeout),
                Ok(inner) => inner?,
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let exit_code = output.status.code().unwrap_or(-1);
        let content = format!("exit={exit_code}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}");
        Ok(ToolOutput { content, display: Some(Display::Terminal {
            command, stdout, stderr, exit_code }) })
    }
}
```

- [ ] **Step 4: Export and run tests**

Add to `agent/crates/agent-tools/src/lib.rs`:
```rust
pub mod shell;
```

Run: `cd agent && cargo test -p agent-tools shell`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-tools/
git commit -m "feat(tools): execute_command with timeout + cancellation"
```

---

## Task 7: Git tools

**Files:**
- Create: `agent/crates/agent-tools/src/git.rs`
- Modify: `agent/crates/agent-tools/src/lib.rs`
- Test: inline in `git.rs`

**Interfaces:**
- Consumes: `Tool`, `ToolCtx`, `Access`.
- Produces: `GitStatus`, `GitDiff` (read-only), `GitCommit` (write). All shell out to `git` in `ctx.workspace`.

- [ ] **Step 1: Write the failing test**

`agent/crates/agent-tools/src/git.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use serde_json::json;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git").args(args).current_dir(dir.path())
                .output().unwrap();
        };
        run(&["init"]);
        run(&["config", "user.email", "t@t.com"]);
        run(&["config", "user.name", "t"]);
        dir
    }
    fn ctx(ws: std::path::PathBuf) -> ToolCtx {
        ToolCtx { workspace: ws, timeout: Duration::from_secs(10), cancel: CancellationToken::new() }
    }

    #[tokio::test]
    async fn git_status_reports_untracked() {
        let dir = init_repo();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        let out = GitStatus.execute(json!({}), &ctx(dir.path().into())).await.unwrap();
        assert!(out.content.contains("a.txt"));
    }

    #[test]
    fn git_commit_intent_is_write() {
        assert_eq!(GitCommit.intent(&json!({"message":"m"})).unwrap().access, Access::Write);
        assert_eq!(GitStatus.intent(&json!({})).unwrap().access, Access::Read);
    }

    #[tokio::test]
    async fn git_commit_commits_staged_changes() {
        let dir = init_repo();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        let out = GitCommit.execute(json!({"message":"init"}), &ctx(dir.path().into())).await.unwrap();
        assert!(out.content.to_lowercase().contains("init") || out.content.contains("master")
                || out.content.contains("main"));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-tools git`
Expected: FAIL — `GitStatus` etc. not found.

- [ ] **Step 3: Implement git tools**

Prepend to `git.rs`:
```rust
use crate::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;

async fn git(ctx: &ToolCtx, args: &[&str]) -> Result<String, ToolError> {
    let out = tokio::process::Command::new("git")
        .args(args).current_dir(&ctx.workspace).output().await
        .map_err(|e| ToolError::Failed { message: e.to_string(), stderr: None })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(ToolError::Failed {
            message: format!("git {} failed", args.join(" ")),
            stderr: Some(String::from_utf8_lossy(&out.stderr).into_owned()) })
    }
}

fn empty_schema(name: &str, desc: &str) -> ToolSchema {
    ToolSchema { name: name.into(), description: desc.into(),
        parameters: json!({"type":"object","properties":{}}) }
}

pub struct GitStatus;
#[async_trait]
impl Tool for GitStatus {
    fn name(&self) -> &str { "git_status" }
    fn description(&self) -> &str { "Show working-tree status." }
    fn schema(&self) -> ToolSchema { empty_schema(self.name(), self.description()) }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent { tool: "git_status".into(), access: Access::Read, paths: vec![],
            command: None, summary: "git status".into() })
    }
    async fn execute(&self, _a: serde_json::Value, ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput { content: git(ctx, &["status", "--short", "--branch"]).await?, display: None })
    }
}

pub struct GitDiff;
#[async_trait]
impl Tool for GitDiff {
    fn name(&self) -> &str { "git_diff" }
    fn description(&self) -> &str { "Show unstaged changes." }
    fn schema(&self) -> ToolSchema { empty_schema(self.name(), self.description()) }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent { tool: "git_diff".into(), access: Access::Read, paths: vec![],
            command: None, summary: "git diff".into() })
    }
    async fn execute(&self, _a: serde_json::Value, ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput { content: git(ctx, &["diff"]).await?, display: None })
    }
}

pub struct GitCommit;
#[async_trait]
impl Tool for GitCommit {
    fn name(&self) -> &str { "git_commit" }
    fn description(&self) -> &str { "Stage all changes and commit with a message." }
    fn schema(&self) -> ToolSchema {
        ToolSchema { name: self.name().into(), description: self.description().into(),
            parameters: json!({"type":"object","properties":{
                "message":{"type":"string"}},"required":["message"]}) }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let msg = args.get("message").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing `message`".into()))?;
        Ok(ToolIntent { tool: "git_commit".into(), access: Access::Write, paths: vec![],
            command: None, summary: format!("git commit -m {msg:?}") })
    }
    async fn execute(&self, args: serde_json::Value, ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let msg = args.get("message").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing `message`".into()))?;
        git(ctx, &["add", "-A"]).await?;
        let out = git(ctx, &["commit", "-m", msg]).await?;
        Ok(ToolOutput { content: out, display: None })
    }
}
```

- [ ] **Step 4: Export and run tests**

Add to `agent/crates/agent-tools/src/lib.rs`:
```rust
pub mod git;
```

Run: `cd agent && cargo test -p agent-tools git`
Expected: PASS (3 tests). *(Requires `git` on PATH.)*

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-tools/
git commit -m "feat(tools): git status/diff/commit"
```

---

## Task 8: Policy engine and approval channel

**Files:**
- Create: `agent/crates/agent-policy/src/lib.rs` (replace placeholder)
- Create: `agent/crates/agent-policy/src/engine.rs`
- Test: inline in `engine.rs`

**Interfaces:**
- Consumes: `agent_tools::{ToolIntent, Access, Display}`.
- Produces: `Decision`, `PolicyEngine`, `ApprovalRequest`, `ApprovalResponse`, `ApprovalChannel`, `RulePolicy`.

- [ ] **Step 1: Write the failing test**

`agent/crates/agent-policy/src/engine.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use agent_tools::{Access, ToolIntent};
    use std::path::PathBuf;

    fn policy() -> RulePolicy {
        RulePolicy {
            workspace: PathBuf::from("/work"),
            command_allowlist: vec!["ls".into(), "cat".into(), "git".into()],
            command_denylist: vec!["rm -rf /".into(), "sudo".into()],
        }
    }
    fn intent(access: Access, paths: Vec<&str>, command: Option<&str>) -> ToolIntent {
        ToolIntent { tool: "t".into(), access, paths: paths.into_iter().map(PathBuf::from).collect(),
            command: command.map(str::to_string), summary: "s".into() }
    }

    #[test]
    fn read_inside_workspace_allowed() {
        assert!(matches!(policy().check(&intent(Access::Read, vec!["/work/a.txt"], None)),
            Decision::Allow));
    }
    #[test]
    fn read_outside_workspace_asks() {
        assert!(matches!(policy().check(&intent(Access::Read, vec!["/etc/passwd"], None)),
            Decision::Ask));
    }
    #[test]
    fn write_always_asks() {
        assert!(matches!(policy().check(&intent(Access::Write, vec!["/work/a.txt"], None)),
            Decision::Ask));
    }
    #[test]
    fn allowlisted_command_allowed() {
        assert!(matches!(policy().check(&intent(Access::Write, vec![], Some("ls -la"))),
            Decision::Allow));
    }
    #[test]
    fn denylisted_command_denied() {
        assert!(matches!(policy().check(&intent(Access::Write, vec![], Some("sudo reboot"))),
            Decision::Deny(_)));
    }
    #[test]
    fn unknown_command_asks() {
        assert!(matches!(policy().check(&intent(Access::Write, vec![], Some("curl evil.com"))),
            Decision::Ask));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-policy`
Expected: FAIL — types not found.

- [ ] **Step 3: Implement the engine**

Prepend to `engine.rs`:
```rust
use agent_tools::{Access, Display, ToolIntent};
use async_trait::async_trait;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum Decision { Allow, Deny(String), Ask }

pub trait PolicyEngine: Send + Sync {
    fn check(&self, intent: &ToolIntent) -> Decision;
}

#[derive(Clone)]
pub struct ApprovalRequest { pub intent: ToolIntent, pub display: Option<Display> }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalResponse { Approve, ApproveAlways, Deny }

#[async_trait]
pub trait ApprovalChannel: Send + Sync {
    async fn request(&self, req: ApprovalRequest) -> ApprovalResponse;
}

pub struct RulePolicy {
    pub workspace: PathBuf,
    pub command_allowlist: Vec<String>,
    pub command_denylist: Vec<String>,
}

impl PolicyEngine for RulePolicy {
    fn check(&self, intent: &ToolIntent) -> Decision {
        // Commands are judged by allow/deny lists first.
        if let Some(cmd) = &intent.command {
            if self.command_denylist.iter().any(|d| cmd.contains(d.as_str())) {
                return Decision::Deny(format!("command matches denylist: {cmd}"));
            }
            let first = cmd.split_whitespace().next().unwrap_or("");
            if self.command_allowlist.iter().any(|a| a == first) {
                return Decision::Allow;
            }
            return Decision::Ask;
        }
        // Otherwise judge by access + path boundary.
        match intent.access {
            Access::Read => {
                let all_inside = intent.paths.iter().all(|p| {
                    let abs = if p.is_absolute() { p.clone() } else { self.workspace.join(p) };
                    abs.starts_with(&self.workspace)
                });
                if all_inside { Decision::Allow } else { Decision::Ask }
            }
            Access::Write => Decision::Ask,
        }
    }
}
```

`agent/crates/agent-policy/src/lib.rs`:
```rust
//! Permission policy engine and approval channel abstraction.
mod engine;
pub use engine::*;
```

- [ ] **Step 4: Run tests**

Run: `cd agent && cargo test -p agent-policy`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-policy/
git commit -m "feat(policy): rule-based engine + approval channel trait"
```

---

## Task 9: Model domain types

**Files:**
- Create: `agent/crates/agent-model/src/lib.rs` (replace placeholder)
- Create: `agent/crates/agent-model/src/types.rs`
- Test: inline in `types.rs`

**Interfaces:**
- Consumes: `agent_tools::{ToolCall, ToolSchema}`.
- Produces: `Role`, `Message`, `CompletionRequest`, `StopReason`, `RawToolCall`, `Chunk`, `AssistantTurn`, `ParsedTurn`, `ProtocolError`, `ModelError` + constructor helpers `Message::{system, user, assistant, tool}`.

- [ ] **Step 1: Write the failing test**

`agent/crates/agent-model/src/types.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_constructors_set_role() {
        assert!(matches!(Message::system("s").role, Role::System));
        assert!(matches!(Message::user("u").role, Role::User));
        let t = Message::tool("call-1", "read_file", "contents");
        assert!(matches!(t.role, Role::Tool));
        assert_eq!(t.tool_call_id.as_deref(), Some("call-1"));
        assert_eq!(t.name.as_deref(), Some("read_file"));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-model`
Expected: FAIL — types not found.

- [ ] **Step 3: Implement the types**

Prepend to `types.rs`:
```rust
use agent_tools::{ToolCall, ToolSchema};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role { System, User, Assistant, Tool }

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
}

impl Message {
    pub fn system(c: impl Into<String>) -> Self { Self::plain(Role::System, c) }
    pub fn user(c: impl Into<String>) -> Self { Self::plain(Role::User, c) }
    pub fn assistant(c: impl Into<String>, calls: Option<Vec<ToolCall>>) -> Self {
        Self { role: Role::Assistant, content: c.into(), tool_calls: calls,
               tool_call_id: None, name: None }
    }
    pub fn tool(call_id: impl Into<String>, name: impl Into<String>, c: impl Into<String>) -> Self {
        Self { role: Role::Tool, content: c.into(), tool_calls: None,
               tool_call_id: Some(call_id.into()), name: Some(name.into()) }
    }
    fn plain(role: Role, c: impl Into<String>) -> Self {
        Self { role, content: c.into(), tool_calls: None, tool_call_id: None, name: None }
    }
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSchema>,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason { Stop, ToolCalls, Length, BudgetExhausted }

#[derive(Debug, Clone, Default)]
pub struct RawToolCall {
    pub id: Option<String>,
    pub name: Option<String>,
    pub args_fragment: String,
}

#[derive(Debug, Clone)]
pub enum Chunk { Text(String), ToolCallDelta(RawToolCall), Done(StopReason) }

#[derive(Debug, Clone)]
pub struct AssistantTurn {
    pub text: String,
    pub raw_tool_calls: Vec<RawToolCall>,
    pub stop: StopReason,
}

#[derive(Debug, Clone)]
pub struct ParsedTurn { pub text: String, pub tool_calls: Vec<ToolCall> }

#[derive(Debug, Clone, thiserror::Error)]
#[error("protocol error: {0}")]
pub struct ProtocolError(pub String);

#[derive(Debug, Clone, thiserror::Error)]
pub enum ModelError {
    #[error("http error: {0}")]
    Http(String),
    #[error("status {0}")]
    Status(u16),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("stream error: {0}")]
    Stream(String),
}
```

`agent/crates/agent-model/src/lib.rs`:
```rust
//! Model client, tool-call protocols, and inference domain types.
mod types;
pub use types::*;
```

- [ ] **Step 4: Run tests**

Run: `cd agent && cargo test -p agent-model`
Expected: PASS (1 test).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-model/
git commit -m "feat(model): inference domain types"
```

---

## Task 10: ToolCallProtocol + NativeProtocol

**Files:**
- Create: `agent/crates/agent-model/src/protocol.rs`
- Modify: `agent/crates/agent-model/src/lib.rs`
- Test: inline in `protocol.rs`

**Interfaces:**
- Consumes: `AssistantTurn`, `RawToolCall`, `ParsedTurn`, `ProtocolError`, `CompletionRequest`, `agent_tools::ToolCall`.
- Produces: `ToolCallProtocol` trait; `NativeProtocol`.

- [ ] **Step 1: Write the failing test**

`agent/crates/agent-model/src/protocol.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    #[test]
    fn native_parses_raw_tool_calls_into_structured() {
        let turn = AssistantTurn {
            text: "ok".into(),
            raw_tool_calls: vec![RawToolCall {
                id: Some("c1".into()), name: Some("read_file".into()),
                args_fragment: r#"{"path":"a.txt"}"#.into() }],
            stop: StopReason::ToolCalls,
        };
        let parsed = NativeProtocol.parse(&turn).unwrap();
        assert_eq!(parsed.text, "ok");
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "read_file");
        assert_eq!(parsed.tool_calls[0].args["path"], "a.txt");
    }

    #[test]
    fn native_rejects_malformed_args() {
        let turn = AssistantTurn { text: "".into(),
            raw_tool_calls: vec![RawToolCall { id: Some("c1".into()),
                name: Some("x".into()), args_fragment: "{not json".into() }],
            stop: StopReason::ToolCalls };
        assert!(NativeProtocol.parse(&turn).is_err());
    }

    #[test]
    fn native_prepare_keeps_tools_field() {
        let mut req = CompletionRequest { messages: vec![], tools: vec![
            agent_tools::ToolSchema { name: "t".into(), description: "d".into(),
                parameters: serde_json::json!({}) }],
            temperature: 0.0, max_tokens: None };
        NativeProtocol.prepare(&mut req);
        assert_eq!(req.tools.len(), 1); // native leaves tools for the client to send
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-model protocol`
Expected: FAIL — `NativeProtocol` not found.

- [ ] **Step 3: Implement trait + NativeProtocol**

Prepend to `protocol.rs`:
```rust
use crate::{AssistantTurn, CompletionRequest, ParsedTurn, ProtocolError};
use agent_tools::ToolCall;

pub trait ToolCallProtocol: Send + Sync {
    /// Adjust the outbound request (e.g. inject tool schemas into the prompt).
    fn prepare(&self, req: &mut CompletionRequest);
    /// Convert a finished assistant turn into clean text + structured tool calls.
    fn parse(&self, raw: &AssistantTurn) -> Result<ParsedTurn, ProtocolError>;
}

/// Uses the server's native OpenAI-style `tool_calls`.
pub struct NativeProtocol;

impl ToolCallProtocol for NativeProtocol {
    fn prepare(&self, _req: &mut CompletionRequest) {
        // No-op: the client serializes `req.tools` into the `tools` field directly.
    }
    fn parse(&self, raw: &AssistantTurn) -> Result<ParsedTurn, ProtocolError> {
        let mut tool_calls = Vec::new();
        for (i, rc) in raw.raw_tool_calls.iter().enumerate() {
            let name = rc.name.clone()
                .ok_or_else(|| ProtocolError(format!("tool call {i} missing name")))?;
            let args: serde_json::Value = if rc.args_fragment.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&rc.args_fragment)
                    .map_err(|e| ProtocolError(format!("tool call {i} bad args: {e}")))?
            };
            let id = rc.id.clone().unwrap_or_else(|| format!("call_{i}"));
            tool_calls.push(ToolCall { id, name, args });
        }
        Ok(ParsedTurn { text: raw.text.clone(), tool_calls })
    }
}
```

Add to `agent/crates/agent-model/src/lib.rs`:
```rust
mod protocol;
pub use protocol::*;
```

- [ ] **Step 4: Run tests**

Run: `cd agent && cargo test -p agent-model protocol`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-model/
git commit -m "feat(model): ToolCallProtocol + NativeProtocol"
```

---

## Task 11: PromptedJsonProtocol

**Files:**
- Create: `agent/crates/agent-model/src/prompted.rs`
- Modify: `agent/crates/agent-model/src/lib.rs`
- Test: inline in `prompted.rs`

**Interfaces:**
- Consumes: `ToolCallProtocol`, `CompletionRequest`, `AssistantTurn`, `Message`, `Role`.
- Produces: `PromptedJsonProtocol`. `prepare` injects tool schemas into a system message and clears `req.tools`; `parse` extracts a fenced ```` ```tool_call ```` JSON block from assistant text.

- [ ] **Step 1: Write the failing test**

`agent/crates/agent-model/src/prompted.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    #[test]
    fn prepare_moves_schemas_into_system_prompt_and_clears_tools() {
        let mut req = CompletionRequest {
            messages: vec![Message::user("hi")],
            tools: vec![agent_tools::ToolSchema { name: "read_file".into(),
                description: "read".into(), parameters: serde_json::json!({"type":"object"}) }],
            temperature: 0.0, max_tokens: None };
        PromptedJsonProtocol.prepare(&mut req);
        assert!(req.tools.is_empty());
        let sys = req.messages.iter().find(|m| matches!(m.role, Role::System)).unwrap();
        assert!(sys.content.contains("read_file"));
    }

    #[test]
    fn parse_extracts_fenced_tool_call_block() {
        let text = "Let me read it.\n```tool_call\n{\"name\":\"read_file\",\
                    \"arguments\":{\"path\":\"a.txt\"}}\n```";
        let turn = AssistantTurn { text: text.into(), raw_tool_calls: vec![],
            stop: StopReason::Stop };
        let parsed = PromptedJsonProtocol.parse(&turn).unwrap();
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "read_file");
        assert_eq!(parsed.tool_calls[0].args["path"], "a.txt");
        assert!(parsed.text.contains("Let me read it"));
        assert!(!parsed.text.contains("```")); // block stripped from visible text
    }

    #[test]
    fn parse_returns_plain_text_when_no_block() {
        let turn = AssistantTurn { text: "all done".into(), raw_tool_calls: vec![],
            stop: StopReason::Stop };
        let parsed = PromptedJsonProtocol.parse(&turn).unwrap();
        assert!(parsed.tool_calls.is_empty());
        assert_eq!(parsed.text, "all done");
    }

    #[test]
    fn parse_errors_on_malformed_block() {
        let turn = AssistantTurn { text: "```tool_call\n{bad}\n```".into(),
            raw_tool_calls: vec![], stop: StopReason::Stop };
        assert!(PromptedJsonProtocol.parse(&turn).is_err());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-model prompted`
Expected: FAIL — `PromptedJsonProtocol` not found.

- [ ] **Step 3: Implement the prompted protocol**

Prepend to `prompted.rs`:
```rust
use crate::{AssistantTurn, CompletionRequest, Message, ParsedTurn, ProtocolError, Role, ToolCallProtocol};
use agent_tools::ToolCall;

const FENCE: &str = "```tool_call";

pub struct PromptedJsonProtocol;

impl PromptedJsonProtocol {
    fn system_preamble(req: &CompletionRequest) -> String {
        let mut s = String::from(
            "You can call tools. To call one, emit a fenced block exactly like:\n\
             ```tool_call\n{\"name\":\"<tool>\",\"arguments\":{...}}\n```\n\
             Emit at most one tool_call block per reply. Available tools:\n");
        for t in &req.tools {
            s.push_str(&format!("- {}: {} | schema: {}\n", t.name, t.description, t.parameters));
        }
        s
    }
}

impl ToolCallProtocol for PromptedJsonProtocol {
    fn prepare(&self, req: &mut CompletionRequest) {
        let preamble = Self::system_preamble(req);
        // Merge into an existing leading system message, or insert one.
        if let Some(first) = req.messages.first_mut() {
            if matches!(first.role, Role::System) {
                first.content = format!("{preamble}\n{}", first.content);
                req.tools.clear();
                return;
            }
        }
        req.messages.insert(0, Message::system(preamble));
        req.tools.clear();
    }

    fn parse(&self, raw: &AssistantTurn) -> Result<ParsedTurn, ProtocolError> {
        let text = &raw.text;
        let Some(start) = text.find(FENCE) else {
            return Ok(ParsedTurn { text: text.clone(), tool_calls: vec![] });
        };
        let after = &text[start + FENCE.len()..];
        let Some(end_rel) = after.find("```") else {
            return Err(ProtocolError("unterminated tool_call block".into()));
        };
        let body = after[..end_rel].trim();
        let v: serde_json::Value = serde_json::from_str(body)
            .map_err(|e| ProtocolError(format!("bad tool_call JSON: {e}")))?;
        let name = v.get("name").and_then(|n| n.as_str())
            .ok_or_else(|| ProtocolError("tool_call missing `name`".into()))?;
        let args = v.get("arguments").cloned().unwrap_or_else(|| serde_json::json!({}));
        let visible = format!("{}{}", &text[..start], &after[end_rel + 3..]).trim().to_string();
        Ok(ParsedTurn {
            text: visible,
            tool_calls: vec![ToolCall { id: "call_0".into(), name: name.into(), args }],
        })
    }
}
```

Add to `agent/crates/agent-model/src/lib.rs`:
```rust
mod prompted;
pub use prompted::*;
```

- [ ] **Step 4: Run tests**

Run: `cd agent && cargo test -p agent-model prompted`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-model/
git commit -m "feat(model): PromptedJsonProtocol"
```

---

## Task 12: OpenAI-compatible streaming client

**Files:**
- Create: `agent/crates/agent-model/src/openai.rs`
- Create: `agent/crates/agent-model/src/wire.rs`
- Modify: `agent/crates/agent-model/src/lib.rs`
- Test: inline in `openai.rs` (uses `wiremock`)

**Interfaces:**
- Consumes: `ModelClient`, `CompletionRequest`, `Chunk`, `ModelError`, `Message`, `Role`, `StopReason`.
- Produces: `OpenAiCompatClient::new(base_url, model, api_key: Option<String>)` implementing `ModelClient` by parsing SSE (`data: {json}\n\n`, terminated by `data: [DONE]`) into a `Chunk` stream.

- [ ] **Step 1: Write the wire-serialization unit test**

`agent/crates/agent-model/src/wire.rs`:
```rust
use crate::{Message, Role};
use serde_json::{json, Value};

/// Serialize our `Message` list into OpenAI chat-completions JSON.
pub fn messages_to_json(messages: &[Message]) -> Vec<Value> {
    messages.iter().map(|m| {
        let role = match m.role {
            Role::System => "system", Role::User => "user",
            Role::Assistant => "assistant", Role::Tool => "tool",
        };
        let mut obj = json!({ "role": role, "content": m.content });
        if let Some(id) = &m.tool_call_id { obj["tool_call_id"] = json!(id); }
        if let Some(calls) = &m.tool_calls {
            obj["tool_calls"] = json!(calls.iter().map(|c| json!({
                "id": c.id, "type": "function",
                "function": { "name": c.name, "arguments": c.args.to_string() }
            })).collect::<Vec<_>>());
        }
        obj
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serializes_tool_result_message() {
        let m = Message::tool("c1", "read_file", "data");
        let v = &messages_to_json(&[m])[0];
        assert_eq!(v["role"], "tool");
        assert_eq!(v["tool_call_id"], "c1");
        assert_eq!(v["content"], "data");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-model wire`
Expected: FAIL — `wire` module not declared. (Add `mod wire;` in lib in Step 5; for now it fails to compile — that's the failing state.)

- [ ] **Step 3: Write the SSE-streaming integration test**

`agent/crates/agent-model/src/openai.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use futures::StreamExt;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn streams_text_chunks_then_done() {
        let server = MockServer::start().await;
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
                    data: [DONE]\n\n";
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body))
            .mount(&server).await;

        let client = OpenAiCompatClient::new(server.uri(), "test-model".into(), None);
        let req = CompletionRequest { messages: vec![Message::user("hi")], tools: vec![],
            temperature: 0.0, max_tokens: None };
        let mut stream = client.stream(req).await.unwrap();

        let mut text = String::new();
        let mut done = None;
        while let Some(item) = stream.next().await {
            match item.unwrap() {
                Chunk::Text(t) => text.push_str(&t),
                Chunk::Done(r) => done = Some(r),
                Chunk::ToolCallDelta(_) => {}
            }
        }
        assert_eq!(text, "Hello");
        assert_eq!(done, Some(StopReason::Stop));
    }

    #[tokio::test]
    async fn surfaces_http_error_status() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500)).mount(&server).await;
        let client = OpenAiCompatClient::new(server.uri(), "m".into(), None);
        let req = CompletionRequest { messages: vec![], tools: vec![],
            temperature: 0.0, max_tokens: None };
        let err = client.stream(req).await.err().unwrap();
        assert!(matches!(err, ModelError::Status(500)));
    }
}
```

- [ ] **Step 4: Run to verify it fails**

Run: `cd agent && cargo test -p agent-model openai`
Expected: FAIL — `OpenAiCompatClient` not found.

- [ ] **Step 5: Implement the client**

Prepend to `openai.rs`:
```rust
use crate::wire::messages_to_json;
use crate::{Chunk, CompletionRequest, ModelClient, ModelError, RawToolCall, StopReason};
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use serde_json::{json, Value};

pub struct OpenAiCompatClient {
    base_url: String,
    model: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl OpenAiCompatClient {
    pub fn new(base_url: String, model: String, api_key: Option<String>) -> Self {
        Self { base_url: base_url.trim_end_matches('/').to_string(), model, api_key,
               http: reqwest::Client::new() }
    }

    fn body(&self, req: &CompletionRequest) -> Value {
        let mut b = json!({
            "model": self.model,
            "messages": messages_to_json(&req.messages),
            "stream": true,
            "temperature": req.temperature,
        });
        if let Some(mt) = req.max_tokens { b["max_tokens"] = json!(mt); }
        if !req.tools.is_empty() {
            b["tools"] = json!(req.tools.iter().map(|t| json!({
                "type": "function",
                "function": { "name": t.name, "description": t.description,
                              "parameters": t.parameters }
            })).collect::<Vec<_>>());
        }
        b
    }
}

fn parse_sse_line(line: &str) -> Option<Result<Vec<Chunk>, ModelError>> {
    let data = line.strip_prefix("data:")?.trim();
    if data == "[DONE]" { return Some(Ok(vec![])); }
    let v: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => return Some(Err(ModelError::Decode(e.to_string()))),
    };
    let choice = &v["choices"][0];
    let mut out = Vec::new();
    if let Some(content) = choice["delta"]["content"].as_str() {
        if !content.is_empty() { out.push(Chunk::Text(content.to_string())); }
    }
    if let Some(calls) = choice["delta"]["tool_calls"].as_array() {
        for c in calls {
            out.push(Chunk::ToolCallDelta(RawToolCall {
                id: c["id"].as_str().map(str::to_string),
                name: c["function"]["name"].as_str().map(str::to_string),
                args_fragment: c["function"]["arguments"].as_str().unwrap_or("").to_string(),
            }));
        }
    }
    if let Some(reason) = choice["finish_reason"].as_str() {
        let stop = match reason {
            "tool_calls" => StopReason::ToolCalls,
            "length" => StopReason::Length,
            _ => StopReason::Stop,
        };
        out.push(Chunk::Done(stop));
    }
    Some(Ok(out))
}

#[async_trait]
impl ModelClient for OpenAiCompatClient {
    async fn stream(&self, req: CompletionRequest)
        -> Result<BoxStream<'static, Result<Chunk, ModelError>>, ModelError> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let mut builder = self.http.post(&url).json(&self.body(&req));
        if let Some(key) = &self.api_key { builder = builder.bearer_auth(key); }
        let resp = builder.send().await.map_err(|e| ModelError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ModelError::Status(resp.status().as_u16()));
        }

        // Accumulate raw bytes, split on newlines, parse SSE `data:` lines.
        let byte_stream = resp.bytes_stream();
        let stream = futures::stream::unfold(
            (byte_stream, String::new(), false),
            |(mut bytes, mut buf, mut done)| async move {
                if done { return None; }
                loop {
                    if let Some(idx) = buf.find('\n') {
                        let line = buf[..idx].trim().to_string();
                        buf.drain(..=idx);
                        if line.is_empty() { continue; }
                        match parse_sse_line(&line) {
                            None => continue,
                            Some(Err(e)) => return Some((Err(e), (bytes, buf, true))),
                            Some(Ok(chunks)) => {
                                if line.contains("[DONE]") { done = true; }
                                if chunks.is_empty() { if done { return None; } continue; }
                                // Emit chunks one at a time by stuffing the rest back into buf
                                // is complex; instead return the first and re-queue remainder.
                                let mut iter = chunks.into_iter();
                                let first = iter.next().unwrap();
                                let remainder: String = iter.map(|c| match c {
                                    Chunk::Text(t) => format!("data: {{\"choices\":[{{\"delta\":{{\"content\":{}}}}}]}}\n",
                                        serde_json::Value::String(t)),
                                    Chunk::Done(_) => "data: [DONE]\n".to_string(),
                                    Chunk::ToolCallDelta(_) => String::new(),
                                }).collect();
                                let new_buf = format!("{remainder}{buf}");
                                return Some((Ok(first), (bytes, new_buf, done)));
                            }
                        }
                    }
                    match bytes.next().await {
                        Some(Ok(b)) => buf.push_str(&String::from_utf8_lossy(&b)),
                        Some(Err(e)) => return Some((Err(ModelError::Stream(e.to_string())),
                            (bytes, buf, true))),
                        None => return None,
                    }
                }
            });
        Ok(stream.boxed())
    }
}
```

> **Implementer note:** the re-queue-remainder trick keeps the `unfold` yielding one `Chunk` per poll. If you prefer, replace the whole `stream` construction with an `async_stream::stream!` block (add `async-stream = "0.3"`) that `yield`s each chunk directly — cleaner. Either satisfies the tests; pick one and keep it.

Add to `agent/crates/agent-model/src/lib.rs`:
```rust
mod wire;
mod openai;
pub use openai::*;
```

- [ ] **Step 6: Run tests**

Run: `cd agent && cargo test -p agent-model`
Expected: PASS (all model tests, incl. wire + 2 openai).

- [ ] **Step 7: Commit**

```bash
git add agent/crates/agent-model/
git commit -m "feat(model): OpenAI-compatible streaming client"
```

---

## Task 13: Context manager + event model

**Files:**
- Create: `agent/crates/agent-core/src/lib.rs` (replace placeholder)
- Create: `agent/crates/agent-core/src/event.rs`
- Create: `agent/crates/agent-core/src/context.rs`
- Test: inline in `context.rs`

**Interfaces:**
- Consumes: `agent_model::Message`, `agent_model::StopReason`, `agent_tools::ToolOutput`, `agent_policy::ApprovalRequest`.
- Produces: `AgentEvent`, `EventSink`, `ContextManager` trait, `WindowContext` (token-counted sliding window), `estimate_tokens`.

- [ ] **Step 1: Write the failing test**

`agent/crates/agent-core/src/context.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use agent_model::{Message, Role};

    #[test]
    fn estimate_tokens_is_roughly_quarter_of_chars() {
        assert!(estimate_tokens("abcd") >= 1);
        assert!(estimate_tokens(&"x".repeat(400)) >= 90);
    }

    #[test]
    fn build_keeps_system_and_drops_oldest_when_over_limit() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        for i in 0..50 {
            ctx.append(Message::user(format!("message number {i} with some padding text")));
        }
        // Tiny limit forces eviction.
        let built = ctx.build(40);
        assert!(matches!(built[0].role, Role::System)); // system always first
        assert!(built.len() < 51);                       // some history evicted
        // The most recent user message survives.
        let last = built.last().unwrap();
        assert!(last.content.contains("49"));
    }

    #[test]
    fn build_returns_all_when_under_limit() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        ctx.append(Message::user("hello"));
        let built = ctx.build(100_000);
        assert_eq!(built.len(), 2);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-core context`
Expected: FAIL — `WindowContext` not found.

- [ ] **Step 3: Implement event model + context manager**

`agent/crates/agent-core/src/event.rs`:
```rust
use agent_model::StopReason;
use agent_policy::ApprovalRequest;
use agent_tools::ToolOutput;

pub enum AgentEvent {
    Token(String),
    ToolStart { name: String, args: serde_json::Value },
    ToolResult { name: String, output: ToolOutput },
    Approval(ApprovalRequest),
    Error(String),
    Done(StopReason),
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: AgentEvent);
}
```

`agent/crates/agent-core/src/context.rs` (prepend above tests):
```rust
use agent_model::{Message, Role};

/// Cheap, tokenizer-agnostic estimate (~4 chars/token). Swap for a real
/// tokenizer later behind the same call site.
pub fn estimate_tokens(s: &str) -> usize {
    (s.chars().count() / 4).max(1)
}

fn message_tokens(m: &Message) -> usize {
    estimate_tokens(&m.content) + 4 // per-message overhead
}

pub trait ContextManager: Send + Sync {
    fn append(&mut self, msg: Message);
    fn build(&self, model_limit: usize) -> Vec<Message>;
}

/// Sliding-window context: always keeps the system prompt; evicts oldest
/// history turns until the estimate fits `model_limit`.
pub struct WindowContext {
    system: Message,
    history: Vec<Message>,
}

impl WindowContext {
    pub fn new(system: Message) -> Self {
        Self { system, history: Vec::new() }
    }
}

impl ContextManager for WindowContext {
    fn append(&mut self, msg: Message) {
        self.history.push(msg);
    }

    fn build(&self, model_limit: usize) -> Vec<Message> {
        let sys_tokens = message_tokens(&self.system);
        let budget = model_limit.saturating_sub(sys_tokens);
        // Walk history newest-first, keep while it fits.
        let mut kept_rev: Vec<Message> = Vec::new();
        let mut used = 0usize;
        for m in self.history.iter().rev() {
            let t = message_tokens(m);
            if used + t > budget && !kept_rev.is_empty() {
                break;
            }
            used += t;
            kept_rev.push(m.clone());
        }
        kept_rev.reverse();
        let mut out = Vec::with_capacity(kept_rev.len() + 1);
        out.push(self.system.clone());
        out.extend(kept_rev);
        out
    }
}

impl Role {}
```

> Remove the trailing `impl Role {}` line — it's a stray; included here only to flag: do **not** add it. The file ends after the `ContextManager` impl.

`agent/crates/agent-core/src/lib.rs`:
```rust
//! Agent loop, context manager, and event model.
mod event;
mod context;
pub use context::*;
pub use event::*;
```

- [ ] **Step 4: Run tests**

Run: `cd agent && cargo test -p agent-core context`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/
git commit -m "feat(core): context manager + event model"
```

---

## Task 14: Agent loop — happy path (with MockModelClient)

**Files:**
- Create: `agent/crates/agent-core/src/loop_.rs`
- Create: `agent/crates/agent-core/src/testkit.rs` (test doubles, `#[cfg(any(test, feature = "testkit"))]`)
- Modify: `agent/crates/agent-core/src/lib.rs`
- Modify: `agent/crates/agent-core/Cargo.toml` (add `testkit` feature)
- Test: inline in `loop_.rs`

**Interfaces:**
- Consumes: every trait so far (`ModelClient`, `ToolCallProtocol`, `ToolRegistry`, `PolicyEngine`, `ApprovalChannel`, `EventSink`, `ContextManager`).
- Produces: `AgentLoop::new(...)`, `AgentLoop::run(&self, &mut dyn ContextManager, String) -> Result<(), AgentError>`; `AgentError`; test doubles `ScriptedModel`, `CollectingSink`, `AlwaysApprove`.

- [ ] **Step 1: Add the `testkit` feature**

In `agent/crates/agent-core/Cargo.toml`, add:
```toml
[features]
testkit = []
```

- [ ] **Step 2: Write test doubles**

`agent/crates/agent-core/src/testkit.rs`:
```rust
//! Test doubles for driving the loop deterministically.
use crate::{AgentEvent, EventSink};
use agent_model::{AssistantTurn, Chunk, CompletionRequest, ModelClient, ModelError,
                  ParsedTurn, ProtocolError, RawToolCall, StopReason, ToolCallProtocol};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use std::sync::Mutex;

/// One scripted assistant turn the mock will emit, in order.
#[derive(Clone)]
pub enum Scripted {
    /// Final assistant text, no tool calls.
    Text(String),
    /// A native tool call: (id, name, json-args-string).
    Call(String, String, String),
    /// Force a transport error this turn.
    Error,
}

pub struct ScriptedModel { turns: Mutex<std::collections::VecDeque<Scripted>> }
impl ScriptedModel {
    pub fn new(turns: Vec<Scripted>) -> Self {
        Self { turns: Mutex::new(turns.into()) }
    }
}

#[async_trait]
impl ModelClient for ScriptedModel {
    async fn stream(&self, _req: CompletionRequest)
        -> Result<BoxStream<'static, Result<Chunk, ModelError>>, ModelError> {
        let next = self.turns.lock().unwrap().pop_front()
            .unwrap_or(Scripted::Text(String::new()));
        match next {
            Scripted::Error => Err(ModelError::Http("scripted error".into())),
            Scripted::Text(t) => Ok(stream::iter(vec![
                Ok(Chunk::Text(t)), Ok(Chunk::Done(StopReason::Stop))]).boxed()),
            Scripted::Call(id, name, args) => Ok(stream::iter(vec![
                Ok(Chunk::ToolCallDelta(RawToolCall { id: Some(id), name: Some(name),
                    args_fragment: args })),
                Ok(Chunk::Done(StopReason::ToolCalls))]).boxed()),
        }
    }
}

/// Trivial protocol for tests: reads native deltas, no prompt injection.
pub struct PassthroughProtocol;
impl ToolCallProtocol for PassthroughProtocol {
    fn prepare(&self, _req: &mut CompletionRequest) {}
    fn parse(&self, raw: &AssistantTurn) -> Result<ParsedTurn, ProtocolError> {
        let mut calls = Vec::new();
        for rc in &raw.raw_tool_calls {
            let name = rc.name.clone().ok_or_else(|| ProtocolError("no name".into()))?;
            let args = if rc.args_fragment.is_empty() { serde_json::json!({}) }
                else { serde_json::from_str(&rc.args_fragment)
                    .map_err(|e| ProtocolError(e.to_string()))? };
            calls.push(agent_tools::ToolCall {
                id: rc.id.clone().unwrap_or_else(|| "c".into()), name, args });
        }
        Ok(ParsedTurn { text: raw.text.clone(), tool_calls: calls })
    }
}

#[derive(Default)]
pub struct CollectingSink { pub events: Mutex<Vec<String>> }
impl EventSink for CollectingSink {
    fn emit(&self, event: AgentEvent) {
        let label = match event {
            AgentEvent::Token(t) => format!("token:{t}"),
            AgentEvent::ToolStart { name, .. } => format!("tool_start:{name}"),
            AgentEvent::ToolResult { name, .. } => format!("tool_result:{name}"),
            AgentEvent::Approval(_) => "approval".into(),
            AgentEvent::Error(e) => format!("error:{e}"),
            AgentEvent::Done(_) => "done".into(),
        };
        self.events.lock().unwrap().push(label);
    }
}

pub struct AlwaysApprove;
#[async_trait]
impl ApprovalChannel for AlwaysApprove {
    async fn request(&self, _req: ApprovalRequest) -> ApprovalResponse { ApprovalResponse::Approve }
}
```

- [ ] **Step 3: Write the failing happy-path test**

`agent/crates/agent-core/src/loop_.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::*;
    use crate::{WindowContext};
    use agent_model::Message;
    use agent_policy::RulePolicy;
    use agent_tools::{fs::ReadFile, ToolRegistry};
    use std::sync::Arc;

    fn registry() -> Arc<ToolRegistry> {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(ReadFile));
        Arc::new(r)
    }
    fn policy(ws: std::path::PathBuf) -> Arc<RulePolicy> {
        Arc::new(RulePolicy { workspace: ws, command_allowlist: vec![], command_denylist: vec![] })
    }

    #[tokio::test]
    async fn runs_tool_then_finishes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "FILEBODY").unwrap();
        let ws = dir.path().to_path_buf();

        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "read_file".into(), r#"{"path":"a.txt"}"#.into()),
            Scripted::Text("The file says FILEBODY".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 2,
                temperature: 0.0, max_tokens: None, workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5) });

        let mut ctx = WindowContext::new(Message::system("you are a test agent"));
        agent.run(&mut ctx, "read a.txt".into()).await.unwrap();

        let events = sink.events.lock().unwrap().clone();
        assert!(events.iter().any(|e| e == "tool_start:read_file"));
        assert!(events.iter().any(|e| e == "tool_result:read_file"));
        assert!(events.last().unwrap() == "done");
    }
}
```

- [ ] **Step 4: Run to verify it fails**

Run: `cd agent && cargo test -p agent-core loop_`
Expected: FAIL — `AgentLoop` not found.

- [ ] **Step 5: Implement the loop (happy path + accumulation)**

Prepend to `agent/crates/agent-core/src/loop_.rs`:
```rust
use crate::{AgentEvent, ContextManager, EventSink};
use agent_model::{AssistantTurn, Chunk, CompletionRequest, Message, ModelClient, ModelError,
                  RawToolCall, StopReason, ToolCallProtocol};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse, Decision, PolicyEngine};
use agent_tools::{ToolCall, ToolCtx, ToolError, ToolRegistry};
use futures::StreamExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("model error after retries: {0}")]
    Model(String),
}

pub struct LoopConfig {
    pub model_limit: usize,
    pub max_turns: usize,
    pub max_retries: usize,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    pub workspace: PathBuf,
    pub tool_timeout: Duration,
}

pub struct AgentLoop {
    model: Arc<dyn ModelClient>,
    protocol: Arc<dyn ToolCallProtocol>,
    tools: Arc<ToolRegistry>,
    policy: Arc<dyn PolicyEngine>,
    approval: Arc<dyn ApprovalChannel>,
    sink: Arc<dyn EventSink>,
    config: LoopConfig,
}

impl AgentLoop {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: Arc<dyn ModelClient>,
        protocol: Arc<dyn ToolCallProtocol>,
        tools: Arc<ToolRegistry>,
        policy: Arc<dyn PolicyEngine>,
        approval: Arc<dyn ApprovalChannel>,
        sink: Arc<dyn EventSink>,
        config: LoopConfig,
    ) -> Self {
        Self { model, protocol, tools, policy, approval, sink, config }
    }

    /// Drive one streamed completion to an `AssistantTurn`, emitting tokens as they arrive.
    async fn one_completion(&self, req: CompletionRequest) -> Result<AssistantTurn, ModelError> {
        let mut stream = self.model.stream(req).await?;
        let mut text = String::new();
        let mut raw_tool_calls: Vec<RawToolCall> = Vec::new();
        let mut stop = StopReason::Stop;
        while let Some(item) = stream.next().await {
            match item? {
                Chunk::Text(t) => { self.sink.emit(AgentEvent::Token(t.clone())); text.push_str(&t); }
                Chunk::ToolCallDelta(rc) => merge_tool_call(&mut raw_tool_calls, rc),
                Chunk::Done(r) => stop = r,
            }
        }
        Ok(AssistantTurn { text, raw_tool_calls, stop })
    }

    /// Stream with retry/backoff on transport errors.
    async fn completion_with_retry(&self, base: &CompletionRequest)
        -> Result<AssistantTurn, AgentError> {
        let mut attempt = 0;
        loop {
            let mut req = base.clone();
            self.protocol.prepare(&mut req);
            match self.one_completion(req).await {
                Ok(turn) => return Ok(turn),
                Err(e) => {
                    attempt += 1;
                    if attempt > self.config.max_retries {
                        self.sink.emit(AgentEvent::Error(e.to_string()));
                        return Err(AgentError::Model(e.to_string()));
                    }
                    tracing::warn!(attempt, error = %e, "model error, retrying");
                    tokio::time::sleep(Duration::from_millis(100 * attempt as u64)).await;
                }
            }
        }
    }

    pub async fn run(&self, ctx: &mut dyn ContextManager, user_input: String)
        -> Result<(), AgentError> {
        ctx.append(Message::user(user_input));
        let mut protocol_repairs = 0;

        for _turn in 0..self.config.max_turns {
            let base = CompletionRequest {
                messages: ctx.build(self.config.model_limit),
                tools: self.tools.schemas(),
                temperature: self.config.temperature,
                max_tokens: self.config.max_tokens,
            };
            let assistant = self.completion_with_retry(&base).await?;

            let parsed = match self.protocol.parse(&assistant) {
                Ok(p) => { protocol_repairs = 0; p }
                Err(e) if protocol_repairs < 1 => {
                    protocol_repairs += 1;
                    ctx.append(Message::assistant(assistant.text.clone(), None));
                    ctx.append(Message::user(format!(
                        "Your tool call could not be parsed: {e}. Re-emit it correctly.")));
                    continue;
                }
                Err(e) => {
                    self.sink.emit(AgentEvent::Error(e.to_string()));
                    return Ok(());
                }
            };

            ctx.append(Message::assistant(parsed.text.clone(),
                if parsed.tool_calls.is_empty() { None } else { Some(parsed.tool_calls.clone()) }));

            if parsed.tool_calls.is_empty() {
                self.sink.emit(AgentEvent::Done(assistant.stop));
                return Ok(());
            }

            for call in parsed.tool_calls {
                let result = self.run_tool(call.clone()).await;
                let content = match result {
                    Ok(output) => {
                        self.sink.emit(AgentEvent::ToolResult {
                            name: call.name.clone(), output: output.clone() });
                        output.content
                    }
                    Err(e) => format!("ERROR: {e}"),
                };
                ctx.append(Message::tool(call.id, call.name, content));
            }
        }
        self.sink.emit(AgentEvent::Done(StopReason::BudgetExhausted));
        Ok(())
    }

    async fn run_tool(&self, call: ToolCall) -> Result<agent_tools::ToolOutput, ToolError> {
        self.sink.emit(AgentEvent::ToolStart { name: call.name.clone(), args: call.args.clone() });
        let tool = self.tools.get(&call.name)
            .ok_or_else(|| ToolError::NotFound(format!("unknown tool {}", call.name)))?;
        let intent = tool.intent(&call.args)?;
        let allowed = match self.policy.check(&intent) {
            Decision::Allow => true,
            Decision::Deny(reason) => return Err(ToolError::Denied(reason)),
            Decision::Ask => {
                // diff preview is produced by execute(); the approval prompt shows the summary.
                let req = ApprovalRequest { intent, display: None };
                self.sink.emit(AgentEvent::Approval(req.clone()));
                matches!(self.approval.request(req).await,
                    ApprovalResponse::Approve | ApprovalResponse::ApproveAlways)
            }
        };
        if !allowed {
            return Err(ToolError::Denied("user declined".into()));
        }
        let ctx = ToolCtx { workspace: self.config.workspace.clone(),
            timeout: self.config.tool_timeout, cancel: CancellationToken::new() };
        tool.execute(call.args, &ctx).await
    }
}

/// Merge a streamed tool-call delta into the accumulator (handles fragmented args).
fn merge_tool_call(acc: &mut Vec<RawToolCall>, delta: RawToolCall) {
    if delta.id.is_some() || acc.is_empty() {
        acc.push(delta);
    } else if let Some(last) = acc.last_mut() {
        if last.name.is_none() { last.name = delta.name; }
        last.args_fragment.push_str(&delta.args_fragment);
    }
}
```

Add to `agent/crates/agent-core/src/lib.rs`:
```rust
mod loop_;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
pub use loop_::*;
```

- [ ] **Step 6: Run the happy-path test**

Run: `cd agent && cargo test -p agent-core loop_`
Expected: PASS — events include `tool_start:read_file`, `tool_result:read_file`, ending `done`.

- [ ] **Step 7: Commit**

```bash
git add agent/crates/agent-core/ 
git commit -m "feat(core): ReAct agent loop (happy path) + test doubles"
```

---

## Task 15: Agent loop — resilience behaviors

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` (tests only; impl already supports these)
- Test: inline in `loop_.rs`

**Interfaces:**
- Consumes: Task 14 loop + test doubles.
- Produces: regression tests proving denial-to-model, retry, self-repair, and budget behaviors.

- [ ] **Step 1: Write the denial + budget + retry tests**

Append to the `tests` module in `loop_.rs`:
```rust
    use agent_policy::PolicyEngine;
    use agent_tools::{Access, ToolIntent};

    struct DenyAll;
    impl PolicyEngine for DenyAll {
        fn check(&self, _i: &ToolIntent) -> Decision { Decision::Deny("nope".into()) }
    }

    #[tokio::test]
    async fn denied_tool_feeds_error_back_and_continues() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "X").unwrap();
        let ws = dir.path().to_path_buf();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "read_file".into(), r#"{"path":"a.txt"}"#.into()),
            Scripted::Text("Understood, it was denied.".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), Arc::new(DenyAll),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 2, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: std::time::Duration::from_secs(5) });
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let events = sink.events.lock().unwrap().clone();
        // No tool_result (it was denied), but the loop still reached done.
        assert!(!events.iter().any(|e| e == "tool_result:read_file"));
        assert_eq!(events.last().unwrap(), "done");
    }

    #[tokio::test]
    async fn transport_error_then_success_via_retry() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Error,
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 3, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: std::time::Duration::from_secs(5) });
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert_eq!(sink.events.lock().unwrap().last().unwrap(), "done");
    }

    #[tokio::test]
    async fn budget_exhaustion_stops_the_loop() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "X").unwrap();
        let ws = dir.path().to_path_buf();
        // Model always calls a tool, never finishes -> must hit max_turns.
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call("c".into(), "read_file".into(), r#"{"path":"a.txt"}"#.into()); 100
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 3, max_retries: 1, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: std::time::Duration::from_secs(5) });
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "loop forever".into()).await.unwrap();
        // 3 turns, each a tool call, then done (BudgetExhausted).
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(events.iter().filter(|e| *e == "tool_start:read_file").count(), 3);
        assert_eq!(events.last().unwrap(), "done");
    }
```

- [ ] **Step 2: Run the tests**

Run: `cd agent && cargo test -p agent-core loop_`
Expected: PASS (happy path + 3 resilience tests).

> If `denied_tool_feeds_error_back_and_continues` fails because the deny path doesn't emit a tool result (correct) but the model's 2nd turn isn't reached, confirm the loop appends a `Message::tool(... "ERROR: ...")` on `Err` — it does in Task 14's `run`. No code change expected.

- [ ] **Step 3: Commit**

```bash
git add agent/crates/agent-core/
git commit -m "test(core): denial, retry, and budget regression tests"
```

---

## Task 16: CLI — terminal approval + renderer

**Files:**
- Create: `agent/crates/agent-cli/src/approval.rs`
- Create: `agent/crates/agent-cli/src/render.rs`
- Modify: `agent/crates/agent-cli/src/main.rs`
- Test: inline in `render.rs`

**Interfaces:**
- Consumes: `agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse}`, `agent_core::{AgentEvent, EventSink}`, `agent_tools::Display`, `similar`.
- Produces: `TerminalApproval` (reads stdin y/n/a), `TerminalSink` (renders events), `render_diff(before, after) -> String`.

- [ ] **Step 1: Write the failing diff-render test**

`agent/crates/agent-cli/src/render.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn render_diff_marks_added_and_removed_lines() {
        let out = render_diff("foo\nbar\n", "foo\nbaz\n");
        assert!(out.contains("-bar"));
        assert!(out.contains("+baz"));
        assert!(out.contains(" foo")); // unchanged context kept
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-cli render`
Expected: FAIL — `render_diff` not found.

- [ ] **Step 3: Implement renderer + approval + sink**

`agent/crates/agent-cli/src/render.rs` (prepend):
```rust
use agent_core::{AgentEvent, EventSink};
use agent_tools::Display;
use similar::{ChangeTag, TextDiff};
use std::io::Write;
use std::sync::Mutex;

pub fn render_diff(before: &str, after: &str) -> String {
    let diff = TextDiff::from_lines(before, after);
    let mut out = String::new();
    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        out.push_str(sign);
        out.push_str(change.value());
    }
    out
}

/// Renders agent events to stdout/stderr. Buffers streamed tokens inline.
pub struct TerminalSink {
    out: Mutex<std::io::Stdout>,
}

impl Default for TerminalSink {
    fn default() -> Self { Self { out: Mutex::new(std::io::stdout()) } }
}

impl EventSink for TerminalSink {
    fn emit(&self, event: AgentEvent) {
        let mut out = self.out.lock().unwrap();
        match event {
            AgentEvent::Token(t) => { let _ = write!(out, "{t}"); let _ = out.flush(); }
            AgentEvent::ToolStart { name, args } =>
                { let _ = writeln!(out, "\n\x1b[36m⚙ {name}\x1b[0m {args}"); }
            AgentEvent::ToolResult { name, output } => {
                if let Some(Display::Diff { path, before, after }) = &output.display {
                    let _ = writeln!(out, "\x1b[33m✎ {path}\x1b[0m\n{}", render_diff(before, after));
                } else if let Some(Display::Terminal { exit_code, stdout, stderr, .. }) = &output.display {
                    let _ = writeln!(out, "\x1b[90m$ exit={exit_code}\x1b[0m\n{stdout}{stderr}");
                } else {
                    let _ = writeln!(out, "\x1b[32m✓ {name}\x1b[0m");
                }
            }
            AgentEvent::Approval(_) => {} // the TerminalApproval channel prints its own prompt
            AgentEvent::Error(e) => { let _ = writeln!(out, "\n\x1b[31m✗ {e}\x1b[0m"); }
            AgentEvent::Done(_) => { let _ = writeln!(out); }
        }
    }
}
```

`agent/crates/agent-cli/src/approval.rs`:
```rust
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use async_trait::async_trait;
use std::io::Write;

pub struct TerminalApproval;

#[async_trait]
impl ApprovalChannel for TerminalApproval {
    async fn request(&self, req: ApprovalRequest) -> ApprovalResponse {
        // Run the blocking stdin read off the async runtime.
        let summary = req.intent.summary.clone();
        tokio::task::spawn_blocking(move || {
            print!("\n\x1b[35mAllow:\x1b[0m {summary} ? [y]es / [n]o / [a]lways: ");
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_err() {
                return ApprovalResponse::Deny;
            }
            match line.trim().to_lowercase().as_str() {
                "y" | "yes" => ApprovalResponse::Approve,
                "a" | "always" => ApprovalResponse::ApproveAlways,
                _ => ApprovalResponse::Deny,
            }
        }).await.unwrap_or(ApprovalResponse::Deny)
    }
}
```

Update `agent/crates/agent-cli/src/main.rs` (minimal, so it compiles; real wiring in Task 17):
```rust
mod approval;
mod render;

fn main() {
    println!("agent-cli — see Task 17 for full wiring");
}
```

- [ ] **Step 4: Run tests**

Run: `cd agent && cargo test -p agent-cli render`
Expected: PASS (1 test). Crate compiles with the two new modules.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-cli/
git commit -m "feat(cli): terminal approval channel + event renderer"
```

---

## Task 17: CLI — config, wiring, and REPL

**Files:**
- Create: `agent/crates/agent-cli/src/config.rs`
- Modify: `agent/crates/agent-cli/src/main.rs`
- Create: `agent/config.example.toml`
- Test: inline in `config.rs`

**Interfaces:**
- Consumes: everything; constructs `OpenAiCompatClient`, picks protocol, builds `ToolRegistry`, `RulePolicy`, `AgentLoop`, runs a stdin REPL.
- Produces: `Cli` (clap args), `Config`, `build_registry()`, `pick_protocol(&str)`.

- [ ] **Step 1: Write the failing config/protocol test**

`agent/crates/agent-cli/src/config.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pick_protocol_selects_by_name() {
        assert_eq!(protocol_name_is_valid("native"), true);
        assert_eq!(protocol_name_is_valid("prompted"), true);
        assert_eq!(protocol_name_is_valid("bogus"), false);
    }
    #[test]
    fn registry_has_all_core_tools() {
        let r = build_registry();
        for name in ["read_file","write_file","edit_file","list_directory",
                     "execute_command","git_status","git_diff","git_commit"] {
            assert!(r.get(name).is_some(), "missing {name}");
        }
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-cli config`
Expected: FAIL — functions not found.

- [ ] **Step 3: Implement config + wiring helpers**

`agent/crates/agent-cli/src/config.rs` (prepend):
```rust
use agent_model::{NativeProtocol, PromptedJsonProtocol, ToolCallProtocol};
use agent_tools::fs::{EditFile, ListDirectory, ReadFile, WriteFile};
use agent_tools::{git::{GitCommit, GitDiff, GitStatus}, shell::ExecuteCommand, ToolRegistry};
use std::sync::Arc;

pub fn protocol_name_is_valid(name: &str) -> bool {
    matches!(name, "native" | "prompted")
}

pub fn pick_protocol(name: &str) -> Arc<dyn ToolCallProtocol> {
    match name {
        "prompted" => Arc::new(PromptedJsonProtocol),
        _ => Arc::new(NativeProtocol),
    }
}

pub fn build_registry() -> ToolRegistry {
    let mut r = ToolRegistry::new();
    r.register(Arc::new(ReadFile));
    r.register(Arc::new(WriteFile));
    r.register(Arc::new(EditFile));
    r.register(Arc::new(ListDirectory));
    r.register(Arc::new(ExecuteCommand));
    r.register(Arc::new(GitStatus));
    r.register(Arc::new(GitDiff));
    r.register(Arc::new(GitCommit));
    r
}

pub fn default_allowlist() -> Vec<String> {
    ["ls","cat","pwd","echo","git","grep","find","rg","cargo","head","tail","wc"]
        .into_iter().map(String::from).collect()
}
pub fn default_denylist() -> Vec<String> {
    ["rm -rf /","sudo",":(){","mkfs","dd if="].into_iter().map(String::from).collect()
}
```

- [ ] **Step 4: Run config tests**

Run: `cd agent && cargo test -p agent-cli config`
Expected: PASS (2 tests).

- [ ] **Step 5: Implement `main` REPL wiring**

`agent/crates/agent-cli/src/main.rs` (full replace):
```rust
mod approval;
mod config;
mod render;

use agent_core::{AgentLoop, LoopConfig, WindowContext};
use agent_model::{Message, OpenAiCompatClient};
use agent_policy::RulePolicy;
use approval::TerminalApproval;
use clap::Parser;
use config::{build_registry, default_allowlist, default_denylist, pick_protocol};
use render::TerminalSink;
use std::io::{BufRead, Write};
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "agent", about = "Local Rust agent core (CLI)")]
struct Cli {
    /// OpenAI-compatible base URL (e.g. http://localhost:30000 for SGLang)
    #[arg(long, default_value = "http://localhost:30000")]
    base_url: String,
    /// Model name to request
    #[arg(long, default_value = "default")]
    model: String,
    /// Tool-call protocol: native | prompted
    #[arg(long, default_value = "native")]
    protocol: String,
    /// Workspace directory the agent may operate in
    #[arg(long, default_value = ".")]
    workspace: String,
    /// Approx context token limit
    #[arg(long, default_value_t = 8192)]
    context_limit: usize,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::from_default_env()).init();
    let cli = Cli::parse();
    let workspace = std::fs::canonicalize(&cli.workspace)
        .unwrap_or_else(|_| std::path::PathBuf::from(&cli.workspace));

    let api_key = std::env::var("AGENT_API_KEY").ok();
    let model = Arc::new(OpenAiCompatClient::new(cli.base_url.clone(), cli.model.clone(), api_key));
    let protocol = pick_protocol(&cli.protocol);
    let tools = Arc::new(build_registry());
    let policy = Arc::new(RulePolicy {
        workspace: workspace.clone(),
        command_allowlist: default_allowlist(),
        command_denylist: default_denylist(),
    });
    let sink = Arc::new(TerminalSink::default());
    let agent = AgentLoop::new(model, protocol, tools, policy, Arc::new(TerminalApproval),
        sink, LoopConfig {
            model_limit: cli.context_limit, max_turns: 25, max_retries: 3, temperature: 0.2,
            max_tokens: Some(2048), workspace, tool_timeout: Duration::from_secs(120),
        });

    let mut ctx = WindowContext::new(Message::system(
        "You are a local coding agent. Use the provided tools to inspect and modify the \
         workspace. Think step by step. When the task is complete, reply with a summary and \
         no tool call."));

    println!("agent ready. Type a task, or 'exit'.");
    let stdin = std::io::stdin();
    loop {
        print!("\n\x1b[1m›\x1b[0m ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 { break; }
        let input = line.trim();
        if input.is_empty() { continue; }
        if input == "exit" || input == "quit" { break; }
        if let Err(e) = agent.run(&mut ctx, input.to_string()).await {
            eprintln!("\x1b[31mfatal: {e}\x1b[0m");
        }
    }
}
```

`agent/config.example.toml`:
```toml
# Example invocation reference (flags shown; this file is documentation only).
# agent --base-url http://localhost:30000 --model my-model --protocol native --workspace .
base_url = "http://localhost:30000"
model = "default"
protocol = "native"   # or "prompted"
workspace = "."
context_limit = 8192
```

- [ ] **Step 6: Verify the whole workspace builds and tests pass**

Run: `cd agent && cargo build && cargo test`
Expected: builds; all unit tests across all crates PASS.

- [ ] **Step 7: Commit**

```bash
git add agent/crates/agent-cli/ agent/config.example.toml
git commit -m "feat(cli): config, tool wiring, and stdin REPL"
```

---

## Task 18: End-to-end integration test (feature-flagged, real SGLang)

**Files:**
- Create: `agent/crates/agent-core/tests/e2e_sglang.rs`
- Modify: `agent/crates/agent-core/Cargo.toml` (add `e2e` feature + dev-deps)
- Create: `agent/docs/RUNNING.md`

**Interfaces:**
- Consumes: the full public API of `agent-core` (+ `testkit` feature for nothing; uses real client).
- Produces: an ignored-by-default integration test that hits a live OpenAI-compatible endpoint.

- [ ] **Step 1: Add the feature and dev-deps**

In `agent/crates/agent-core/Cargo.toml`:
```toml
[features]
testkit = []
e2e = []

[dev-dependencies]
agent-model = { path = "../agent-model" }
agent-tools = { path = "../agent-tools" }
agent-policy = { path = "../agent-policy" }
tempfile = "3"
```

- [ ] **Step 2: Write the e2e test (ignored unless env set)**

`agent/crates/agent-core/tests/e2e_sglang.rs`:
```rust
//! Live end-to-end test. Requires a running OpenAI-compatible server.
//! Run with: AGENT_E2E_URL=http://localhost:30000 AGENT_E2E_MODEL=<name> \
//!           cargo test -p agent-core --test e2e_sglang -- --ignored --nocapture

use agent_core::{AgentLoop, EventSink, AgentEvent, LoopConfig, WindowContext};
use agent_model::{Message, NativeProtocol, OpenAiCompatClient};
use agent_policy::RulePolicy;
use agent_tools::{fs::ReadFile, ToolRegistry};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Capture(Mutex<Vec<String>>);
impl EventSink for Capture {
    fn emit(&self, e: AgentEvent) {
        if let AgentEvent::ToolResult { name, .. } = e {
            self.0.lock().unwrap().push(name);
        }
    }
}

#[tokio::test]
#[ignore = "requires AGENT_E2E_URL / AGENT_E2E_MODEL and a live server"]
async fn reads_a_file_against_real_server() {
    let url = std::env::var("AGENT_E2E_URL").expect("set AGENT_E2E_URL");
    let model_name = std::env::var("AGENT_E2E_MODEL").expect("set AGENT_E2E_MODEL");

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("secret.txt"), "the password is swordfish").unwrap();
    let ws = dir.path().to_path_buf();

    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(ReadFile));
    let sink = Arc::new(Capture(Mutex::new(vec![])));
    let agent = AgentLoop::new(
        Arc::new(OpenAiCompatClient::new(url, model_name, std::env::var("AGENT_API_KEY").ok())),
        Arc::new(NativeProtocol), Arc::new(reg),
        Arc::new(RulePolicy { workspace: ws.clone(), command_allowlist: vec![],
            command_denylist: vec![] }),
        Arc::new(AutoApprove), sink.clone(),
        LoopConfig { model_limit: 8192, max_turns: 8, max_retries: 2, temperature: 0.0,
            max_tokens: Some(512), workspace: ws, tool_timeout: Duration::from_secs(60) });

    let mut ctx = WindowContext::new(Message::system(
        "You are a coding agent. Use read_file to answer questions about files."));
    agent.run(&mut ctx, "Read secret.txt and tell me the password.".into()).await.unwrap();

    let tools_used = sink.0.lock().unwrap().clone();
    assert!(tools_used.iter().any(|n| n == "read_file"),
        "model should have called read_file; got {tools_used:?}");
}

struct AutoApprove;
#[async_trait::async_trait]
impl agent_policy::ApprovalChannel for AutoApprove {
    async fn request(&self, _r: agent_policy::ApprovalRequest) -> agent_policy::ApprovalResponse {
        agent_policy::ApprovalResponse::Approve
    }
}
```

Add `async-trait` to `agent-core` dev-deps if not already present:
```toml
async-trait = "0.1"
```
(It is already a normal dependency from Task 1, so `tests/` can use it.)

- [ ] **Step 3: Verify it compiles and is skipped by default**

Run: `cd agent && cargo test -p agent-core`
Expected: PASS; the e2e test shows as `ignored`.

- [ ] **Step 4: Write the run instructions**

`agent/docs/RUNNING.md`:
```markdown
# Running the agent

## 1. Start an inference server (OpenAI-compatible)

**SGLang (primary target):**
```bash
python -m sglang.launch_server --model-path <hf-model> --port 30000
```
vLLM (`--port 8000`) and llama.cpp's `llama-server` (`--port 8080`) expose the same
`/v1/chat/completions` API and work identically — just change `--base-url`.

## 2. Run the CLI

```bash
cd agent
cargo run -p agent-cli -- \
  --base-url http://localhost:30000 \
  --model <served-model-name> \
  --protocol native \        # use `prompted` for models without native tool-calling
  --workspace /path/to/project
```
Set `AGENT_API_KEY` if your endpoint requires a bearer token. Tune log output with
`RUST_LOG=agent_core=debug`.

## 3. End-to-end test against your server

```bash
AGENT_E2E_URL=http://localhost:30000 AGENT_E2E_MODEL=<name> \
  cargo test -p agent-core --test e2e_sglang -- --ignored --nocapture
```
```

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/ agent/docs/RUNNING.md
git commit -m "test(core): feature-flagged e2e against live server + run docs"
```

---

## Final verification

- [ ] **Run the full suite**

Run: `cd agent && cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all unit/integration tests PASS (e2e ignored); clippy clean.

- [ ] **Smoke-build the binary**

Run: `cd agent && cargo build --release -p agent-cli`
Expected: produces `agent/target/release/agent`.

---

## Self-Review

**Spec coverage** (spec section → task):
- §3 workspace of 5 crates → Task 1 (with `agent-tools` as shared-vocabulary foundation, noted).
- §4 control flow / ReAct loop → Tasks 14–15.
- §5 `ModelClient` → Task 12; `ToolCallProtocol` (native + prompted) → Tasks 10–11; `Tool` + `intent()` → Tasks 3–7; `PolicyEngine` + `ApprovalChannel` → Task 8; `EventSink` → Task 13; `ContextManager` → Task 13.
- §6 tool set (read/write/edit/list/execute/git status·diff·commit) → Tasks 4–7; policy treatments → Task 8 + applied in Task 17 wiring.
- §7 error handling: ToolError-to-model → Task 14 `run`; ModelError retry/backoff → Task 14 `completion_with_retry` + Task 15 test; ProtocolError self-repair → Task 14 `run` + (covered structurally); timeout/cancel → Task 6; turn budget → Tasks 14–15.
- §8 observability (`tracing`) → wired in Task 17 `main` + warn in Task 14; JSON log option via `tracing-subscriber` json feature (Task 1).
- §9 testing: per-crate unit (all tasks), MockModelClient loop tests (Tasks 14–15), feature-flagged real-SGLang integration (Task 18).
- §10 forward-compat seams (`EventSink`, `ApprovalChannel`) → defined Tasks 8 & 13, exercised by CLI impls in Task 16.

**Placeholder scan:** No "TBD/TODO" left as work. Two "implementer note" callouts (Task 12 SSE style, Task 14 `ApprovalRequest` clone) offer a cleaner alternative but each ships complete, working code as written. The Task 13 stray `impl Role {}` is explicitly flagged for removal.

**Type consistency:** `Tool::intent -> Result<ToolIntent, ToolError>` used uniformly (Tasks 3–7, consumed Task 14). `AgentEvent::Approval` name consistent (Tasks 13, 14, 16). `AgentLoop::new` 7-arg signature matches all call sites (Tasks 14, 15, 17, 18). `ContextManager::{append, build}` consistent (Tasks 13–18). `Message::{system,user,assistant,tool}` constructors consistent (Tasks 9, 13–18). `ApprovalRequest` derives `Clone` (Task 8), so Task 14 emits the approval event and calls the channel with a clean `req.clone()`; this relies on `ToolIntent` and `Display` deriving `Clone` (both do, Task 2).
