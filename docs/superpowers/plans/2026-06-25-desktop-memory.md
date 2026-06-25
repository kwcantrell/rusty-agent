# Desktop Memory Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the existing memory subsystem (tools + auto-retrieval) into the desktop app, enabled by default with a Settings toggle, scoped correctly to the current workspace.

**Architecture:** Load the expensive embedding model + DB handle **once** at bridge startup (`MemoryParts`); assemble the cheap, workspace-scoped tools + retriever **per connection**; gate registration live in `build_loop` on a new `RuntimeConfig.memory` flag.

**Tech Stack:** Rust (agent-memory/runtime-config/server crates, Tauri `src-tauri`), TypeScript/React (`web`), vitest.

## Global Constraints

- `cargo` is not on PATH — every Rust shell step runs `source ~/.cargo/env` first, from the `agent/` directory (workspace root) unless noted. The Tauri crate (`src-tauri`) is a **separate** cargo project at the repo root; build it with `cargo build --manifest-path src-tauri/Cargo.toml`.
- Web commands run from `web/`.
- Memory is best-effort: construction failure (offline / model download fail / DB unopenable) must never block boot — it degrades to no-memory, warn-logged.
- Default-on: `RuntimeConfig.memory` defaults to `true`. CLI behavior is unchanged (CLI does not use `build_loop`; it keeps its own `--memory` flag).
- Desktop + CLI share `~/.agent/memory.db` (the `MemoryConfig::default()` db path); memory is project-scoped by workspace.
- `git` runs from the repo root `/home/kalen/rust-agent-runtime`.

---

### Task 1: `agent-memory` — split load from assemble

**Files:**
- Modify: `agent/crates/agent-memory/src/lib.rs` (add `MemoryParts`, `open_memory_parts`, `assemble_memory`; reimplement `build_tools_and_retriever`)
- Test: inline `#[cfg(test)]` in `agent/crates/agent-memory/src/lib.rs`

**Interfaces:**
- Consumes: `Embedder`, `MemoryStore`, `MemoryConfig`, `SqliteStore`, `StubEmbedder`, `InMemoryStore`, `project_scope`, `build_tools_with`, `retriever::MemoryRetriever`, `agent_core::Retriever`.
- Produces:
  - `pub struct MemoryParts { pub embedder: Arc<dyn Embedder>, pub store: Arc<dyn MemoryStore>, pub cfg: Arc<MemoryConfig> }` (derives `Clone`).
  - `pub fn open_memory_parts(cfg: MemoryConfig) -> Result<MemoryParts, MemoryInitError>`
  - `pub fn assemble_memory(parts: &MemoryParts, workspace: &Path) -> (Vec<Arc<dyn agent_tools::Tool>>, Arc<dyn agent_core::Retriever>)`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod build_tests` block in `agent/crates/agent-memory/src/lib.rs`:

```rust
    #[tokio::test]
    async fn assemble_memory_scopes_to_workspace() {
        use crate::record::{MemoryRecord, MemoryScope, now_secs};
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
        let store: Arc<dyn MemoryStore> = Arc::new(InMemoryStore::new());
        // Seed a memory scoped to workspace A's project key.
        let key_a = match project_scope(Path::new("/tmp/ws-a")) {
            MemoryScope::Project(k) => k, MemoryScope::Global => unreachable!(),
        };
        let v = embedder.embed(&["alpha fact".to_string()]).await.unwrap().remove(0);
        store.upsert(MemoryRecord { id: "1".into(), text: "alpha fact".into(),
            scope: MemoryScope::Project(key_a), tags: vec![], vector: v,
            created_at: now_secs(), updated_at: now_secs(), source: "t".into() }).await.unwrap();
        let parts = MemoryParts { embedder, store, cfg: Arc::new(MemoryConfig::default()) };

        // Workspace A assembles three tools and a retriever that finds the memory.
        let (tools_a, retr_a) = assemble_memory(&parts, Path::new("/tmp/ws-a"));
        let names: Vec<&str> = tools_a.iter().map(|t| t.name()).collect();
        for n in ["remember", "recall", "forget"] { assert!(names.contains(&n), "missing {n}"); }
        assert!(retr_a.retrieve("alpha fact").await.iter().any(|l| l == "alpha fact"));

        // Workspace B has a different project scope → does not see A's memory.
        let (_tools_b, retr_b) = assemble_memory(&parts, Path::new("/tmp/ws-b"));
        assert!(retr_b.retrieve("alpha fact").await.is_empty(), "cross-workspace leak");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-memory assemble_memory_scopes_to_workspace 2>&1 | tail -5`
Expected: FAIL to compile — `MemoryParts` / `assemble_memory` not found.

- [ ] **Step 3: Implement the split**

In `agent/crates/agent-memory/src/lib.rs`, add after the existing `build_tools_and_retriever` function:

```rust
/// The expensive, workspace-independent half of memory construction: the embedding
/// model and the store handle. Build once; assemble per workspace via `assemble_memory`.
#[derive(Clone)]
pub struct MemoryParts {
    pub embedder: Arc<dyn Embedder>,
    pub store: Arc<dyn MemoryStore>,
    pub cfg: Arc<MemoryConfig>,
}

/// Open the store + load the embedder once (network on first run for the model).
/// Errors mean "disable memory" — never fatal.
pub fn open_memory_parts(cfg: MemoryConfig) -> Result<MemoryParts, MemoryInitError> {
    let store: Arc<dyn MemoryStore> =
        Arc::new(SqliteStore::open(&cfg.db_path).map_err(|e| MemoryInitError::Store(e.to_string()))?);
    #[cfg(feature = "onnx")]
    let embedder: Arc<dyn Embedder> = Arc::new(
        embedder::FastEmbedEmbedder::new(&cfg).map_err(|e| MemoryInitError::Embedder(e.to_string()))?);
    #[cfg(not(feature = "onnx"))]
    let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
    Ok(MemoryParts { embedder, store, cfg: Arc::new(cfg) })
}

/// Cheap, workspace-scoped assembly: derive the project scope from `workspace`,
/// then build the three tools and the auto-retrieval retriever. No model load.
pub fn assemble_memory(
    parts: &MemoryParts,
    workspace: &Path,
) -> (Vec<Arc<dyn agent_tools::Tool>>, Arc<dyn agent_core::Retriever>) {
    let scope = project_scope(workspace);
    let key = match &scope {
        MemoryScope::Project(k) => k.clone(),
        MemoryScope::Global => String::new(),
    };
    let tools = build_tools_with(parts.embedder.clone(), parts.store.clone(), parts.cfg.clone(), scope);
    let retriever: Arc<dyn agent_core::Retriever> = Arc::new(retriever::MemoryRetriever {
        embedder: parts.embedder.clone(),
        store: parts.store.clone(),
        cfg: parts.cfg.clone(),
        project_key: key,
    });
    (tools, retriever)
}
```

Then reimplement `build_tools_and_retriever` on top of the split (replace its body):

```rust
pub fn build_tools_and_retriever(
    cfg: MemoryConfig,
    workspace: &Path,
) -> Result<(Vec<Arc<dyn agent_tools::Tool>>, Arc<dyn agent_core::Retriever>), MemoryInitError> {
    let parts = open_memory_parts(cfg)?;
    Ok(assemble_memory(&parts, workspace))
}
```

> The test module needs `use std::path::Path;` — `lib.rs` already imports `std::path::Path` at module scope, but the `#[cfg(test)] mod build_tests` block uses `use super::*;` so it is in scope. The `MemoryScope` import comes via `use super::*;` too (re-exported at crate root).

- [ ] **Step 4: Run test to verify it passes**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-memory 2>&1 | grep -E "test result|error" | head`
Expected: PASS — new `assemble_memory_scopes_to_workspace` plus all existing agent-memory tests (the `retriever` tests and `build_tools_with_returns_three_named_tools` still pass; `build_tools_and_retriever` is unchanged in behavior).

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-memory/src/lib.rs
git commit -m "feat(agent-memory): split open_memory_parts from assemble_memory"
```

---

### Task 2: `agent-runtime-config` — `memory` config field

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs` (struct field, `from_launch`)
- Test: inline `#[cfg(test)]` in the same file

**Interfaces:**
- Produces: `RuntimeConfig.memory: bool` (serde default `true`), set to `true` by `from_launch`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `agent/crates/agent-runtime-config/src/runtime_config.rs`:

```rust
    #[test]
    fn memory_defaults_on() {
        let c = RuntimeConfig::from_launch(
            "openai".into(), "http://x".into(), "m".into(), "native".into(), 8192);
        assert!(c.memory, "memory should default on");
    }

    #[test]
    fn memory_absent_in_json_defaults_true() {
        // A persisted config written before the field existed must deserialize to memory=true.
        let json = r#"{"backend":"openai","base_url":"http://x","model":"m","protocol":"native",
            "command_allowlist":[],"command_denylist":[],"http_allow_hosts":[],
            "temperature":0.2,"max_tokens":2048,"max_turns":25,"context_limit":8192}"#;
        let c: RuntimeConfig = serde_json::from_str(json).unwrap();
        assert!(c.memory);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config memory_ 2>&1 | tail -5`
Expected: FAIL to compile — no field `memory`.

- [ ] **Step 3: Add the field and default**

In `agent/crates/agent-runtime-config/src/runtime_config.rs`, add to the `RuntimeConfig` struct (after `pub context_limit: usize,`):

```rust
    #[serde(default = "default_true")]
    pub memory: bool,
```

And in `from_launch`, add to the returned `Self { ... }` (after `context_limit,`):

```rust
            memory: true,
```

- [ ] **Step 4: Run to verify it passes**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config 2>&1 | grep -E "test result|error" | head`
Expected: PASS — new memory tests plus all existing runtime-config tests.

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-runtime-config/src/runtime_config.rs
git commit -m "feat(runtime-config): add memory flag (default on)"
```

---

### Task 3: `agent-server` — gate memory in `build_loop`, thread retriever + budget

**Files:**
- Modify: `agent/crates/agent-server/src/runtime.rs` (imports, `build_loop`, `RuntimeState`)
- Modify: `agent/crates/agent-server/src/daemon.rs` (`DaemonParams`, `serve` WindowContext + RuntimeState::new call)
- Test: inline `#[cfg(test)]` in `runtime.rs`

**Interfaces:**
- Consumes: `RuntimeConfig.memory` (Task 2), `agent_core::Retriever`, `AgentLoop::with_retriever`, `WindowContext::with_recall_budget`.
- Produces:
  - `DaemonParams.memory_retriever: Option<Arc<dyn agent_core::Retriever>>`, `DaemonParams.recall_token_budget: usize`.
  - `RuntimeState.memory_retriever` field; `build_loop(..., memory_retriever: Option<&Arc<dyn Retriever>>)` gating on `cfg.memory`.

- [ ] **Step 1: Write the failing test**

In `agent/crates/agent-server/src/runtime.rs`, find the existing test `build_loop_registers_injected_memory_tools` and add a sibling test right after it. First, locate the existing test's `build_loop(...)` call to copy its argument shape. Then add:

```rust
    #[test]
    fn build_loop_skips_memory_tools_when_disabled() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let session = Arc::new(Mutex::new(String::new()));
        let sink = Arc::new(WsEventSink::new(tx.clone(), session.clone()));
        let approval = Arc::new(WsApprovalChannel::new(tx, session, Duration::from_secs(1)));
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
        cfg.memory = false; // disabled
        let mem: Arc<[Arc<dyn Tool>]> = Arc::from(vec![
            Arc::new(agent_memory::Recall {
                embedder: Arc::new(agent_memory::StubEmbedder::d384()),
                store: Arc::new(agent_memory::InMemoryStore::new()),
                cfg: Arc::new(agent_memory::MemoryConfig::default()),
                project_key: "k".into(),
            }) as Arc<dyn Tool>,
        ]);
        let built = build_loop(&cfg, &sink, &approval, dir.path(), &None, "claude",
            &[], &mem, None, SYSTEM_PROMPT);
        assert!(!built.registered_names.iter().any(|n| n == "recall"),
            "memory tools must not register when cfg.memory is false");
    }
```

> Note: this assumes the existing `build_loop_registers_injected_memory_tools` test injects a `recall` tool the same way and asserts on `built.registered_names`. Match its construction style exactly; if it uses a different fake `Tool`, reuse that. Also update the **existing** test's `build_loop(...)` call to pass the two new args (`None` for the retriever, and keep `cfg.memory` at its default `true`).

- [ ] **Step 2: Run to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-server build_loop 2>&1 | tail -8`
Expected: FAIL to compile — `build_loop` takes fewer args / signature mismatch.

- [ ] **Step 3: Add the import and update `build_loop`**

In `agent/crates/agent-server/src/runtime.rs`, add to the `agent_core` import (it already imports `AgentLoop` etc.):

```rust
use agent_core::Retriever;
```

Change the `build_loop` signature to add the retriever param (after `memory_tools: &[Arc<dyn Tool>],`):

```rust
    memory_tools: &[Arc<dyn Tool>],
    memory_retriever: Option<&Arc<dyn Retriever>>,
    base_system_prompt: &str,
```

Replace the unconditional memory-tools registration:

```rust
    for t in memory_tools {
        registry.register(t.clone());
    }
```

with a gated block:

```rust
    if cfg.memory {
        for t in memory_tools {
            registry.register(t.clone());
        }
    }
```

Then, at the `let loop_ = Arc::new(AgentLoop::new(...))` construction, build it mutably and conditionally attach the retriever. Replace `let loop_ = Arc::new(AgentLoop::new(` ... `));` so it reads:

```rust
    let agent = AgentLoop::new(
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
            top_p: cfg.top_p,
            top_k: cfg.top_k,
            min_p: cfg.min_p,
            presence_penalty: cfg.presence_penalty,
            repeat_penalty: cfg.repeat_penalty,
            enable_thinking: cfg.enable_thinking,
            preserve_thinking: cfg.preserve_thinking,
            sandbox: Some(build_sandbox(cfg)),
            max_parallel_tools: 8,
        },
    );
    let agent = match (cfg.memory, memory_retriever) {
        (true, Some(r)) => agent.with_retriever(r.clone()),
        _ => agent,
    };
    let loop_ = Arc::new(agent);
```

- [ ] **Step 4: Thread `memory_retriever` through `RuntimeState`**

Add the field to the `RuntimeState` struct (after `memory_tools: Arc<[Arc<dyn Tool>]>,`):

```rust
    memory_retriever: Option<Arc<dyn Retriever>>,
```

Add the parameter to `RuntimeState::new` (after `memory_tools: Arc<[Arc<dyn Tool>]>,`):

```rust
        memory_retriever: Option<Arc<dyn Retriever>>,
```

Update the `build_loop(...)` call inside `new` to pass `memory_retriever.as_ref()`:

```rust
        let built = build_loop(
            &config, &sink, &approval, &workspace, &api_key, &claude_binary, &mcp_tools,
            &memory_tools, memory_retriever.as_ref(), &base_system_prompt);
```

Add `memory_retriever` to the `Self { ... }` initializer (after `mcp_tools, memory_tools,`):

```rust
            mcp_tools, memory_tools, memory_retriever, base_system_prompt,
```

Update the `build_loop(...)` call inside `apply` likewise:

```rust
        let built = build_loop(
            &cfg, &self.sink, &self.approval, &self.workspace, &self.api_key,
            &self.claude_binary, &self.mcp_tools, &self.memory_tools,
            self.memory_retriever.as_ref(), &self.base_system_prompt);
```

- [ ] **Step 5: Add `DaemonParams` fields and wire `serve`**

In `agent/crates/agent-server/src/daemon.rs`, add to the `agent_core` import:

```rust
use agent_core::{ContextManager, WindowContext, Retriever};
```

Add to the `DaemonParams` struct (after `pub memory_tools: Arc<[Arc<dyn Tool>]>,`):

```rust
    pub memory_retriever: Option<Arc<dyn Retriever>>,
    pub recall_token_budget: usize,
```

In `serve`, pass `memory_retriever` to `RuntimeState::new` (add after `params.memory_tools.clone(),`):

```rust
        params.memory_retriever.clone(),
```

And set the recall budget on the context — replace:

```rust
    let ctx = Arc::new(tokio::sync::Mutex::new(
        WindowContext::new(Message::system(params.system_prompt.clone()))));
```

with:

```rust
    let ctx = Arc::new(tokio::sync::Mutex::new(
        WindowContext::new(Message::system(params.system_prompt.clone()))
            .with_recall_budget(params.recall_token_budget)));
```

- [ ] **Step 6: Fix the other `RuntimeState::new` / `DaemonParams` construction sites the compiler flags**

Run the build; the `make()` test helper in `runtime.rs` and any `DaemonParams { ... }` literal (e.g. `setup.rs`, handled in Task 4) will need the new fields. For `runtime.rs`'s `make()` test helper, pass `None` for the new `RuntimeState::new` retriever arg.

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-server 2>&1 | grep -E "test result|error\[|error:" | head -20`
Expected after fixing the flagged sites: PASS — both `build_loop` tests and the full agent-server suite. (`setup.rs`'s `DaemonParams` literal is updated in Task 4; if agent-server won't compile until then, complete Task 4's `setup.rs` edit before re-running — they are in the same crate.)

- [ ] **Step 7: Commit** (combined with Task 4, since they share the `agent-server` crate compile)

Proceed to Task 4; commit both together at the end of Task 4.

---

### Task 4: `agent-server::setup` — assemble per connection from `MemoryParts`

**Files:**
- Modify: `agent/crates/agent-server/src/setup.rs` (`local_params` signature + body, test)
- Modify: `agent/crates/agent-server/Cargo.toml` (ensure `agent-memory` dep is present)

**Interfaces:**
- Consumes: `agent_memory::{MemoryParts, assemble_memory}` (Task 1), `DaemonParams.memory_retriever`/`recall_token_budget` (Task 3).
- Produces: `local_params(workspace, config_path, base_url, model, memory_parts: Option<&MemoryParts>) -> DaemonParams`.

- [ ] **Step 1: Ensure the `agent-memory` dependency**

In `agent/crates/agent-server/Cargo.toml`, confirm/add under `[dependencies]`:

```toml
agent-memory = { path = "../agent-memory" }
```

(If it is already present — the daemon may already reference memory types — leave it.)

- [ ] **Step 2: Update the failing test**

In `agent/crates/agent-server/src/setup.rs`, replace the `local_params_seeds_llama_defaults` test's call and add a memory test:

```rust
    #[test]
    fn local_params_seeds_llama_defaults() {
        let p = local_params(
            PathBuf::from("/tmp/ws"),
            PathBuf::from("/tmp/agent-runtime.json"),
            "http://localhost:8080".into(),
            "qwen3.6-35b-a3b".into(),
            None,
        );
        assert_eq!(p.config.backend, "openai");
        assert_eq!(p.config.base_url, "http://localhost:8080");
        assert_eq!(p.config.model, "qwen3.6-35b-a3b");
        assert_eq!(p.config.protocol, "native");
        assert!(p.config.preserve_thinking);
        assert_eq!(p.workspace, PathBuf::from("/tmp/ws"));
        assert!(p.mcp_tools.is_empty());
        assert!(p.memory_tools.is_empty());
        assert!(p.memory_retriever.is_none());
    }

    #[test]
    fn local_params_with_parts_populates_memory() {
        use agent_memory::{MemoryParts, MemoryConfig, StubEmbedder, InMemoryStore, Embedder, MemoryStore};
        use std::sync::Arc;
        let parts = MemoryParts {
            embedder: Arc::new(StubEmbedder::d384()) as Arc<dyn Embedder>,
            store: Arc::new(InMemoryStore::new()) as Arc<dyn MemoryStore>,
            cfg: Arc::new(MemoryConfig::default()),
        };
        let p = local_params(
            PathBuf::from("/tmp/ws"), PathBuf::from("/tmp/rt.json"),
            "http://localhost:8080".into(), "m".into(), Some(&parts));
        assert_eq!(p.memory_tools.len(), 3);
        assert!(p.memory_retriever.is_some());
        assert_eq!(p.recall_token_budget, 512);
    }
```

- [ ] **Step 3: Update `local_params`**

In `agent/crates/agent-server/src/setup.rs`, change the signature and body:

```rust
use agent_memory::{assemble_memory, MemoryParts};
use std::sync::Arc;

pub fn local_params(
    workspace: PathBuf,
    config_path: PathBuf,
    base_url: String,
    model: String,
    memory_parts: Option<&MemoryParts>,
) -> DaemonParams {
    let mut config = RuntimeConfig::from_launch(
        "openai".into(), base_url, model, "native".into(), 262_144);
    config.preserve_thinking = true;
    config.enable_thinking = true;

    let (memory_tools, memory_retriever, recall_token_budget) = match memory_parts {
        Some(parts) => {
            let (tools, retriever) = assemble_memory(parts, &workspace);
            (Arc::from(tools), Some(retriever), parts.cfg.recall_token_budget)
        }
        None => (
            Arc::from(Vec::<Arc<dyn agent_tools::Tool>>::new()),
            None,
            512, // matches MemoryConfig::default().recall_token_budget
        ),
    };

    DaemonParams {
        config,
        api_key: std::env::var("AGENT_API_KEY").ok(),
        claude_binary: "claude".into(),
        config_path,
        workspace,
        system_prompt: SYSTEM_PROMPT.to_string(),
        mcp_tools: Arc::from(Vec::<Arc<dyn agent_tools::Tool>>::new()),
        memory_tools,
        memory_retriever,
        recall_token_budget,
    }
}
```

- [ ] **Step 4: Build and test the whole `agent-server` crate (Tasks 3 + 4)**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-server 2>&1 | grep -E "test result|error\[|error:" | head -20`
Expected: PASS — all agent-server tests including the new `build_loop_skips_memory_tools_when_disabled`, `local_params_with_parts_populates_memory`.

- [ ] **Step 5: Commit (Tasks 3 + 4 together)**

```bash
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-server/
git commit -m "feat(agent-server): gate memory on cfg.memory, assemble per-connection from MemoryParts"
```

---

### Task 5: `src-tauri` — load `MemoryParts` once, assemble per connection

**Files:**
- Modify: `src-tauri/Cargo.toml` (add `agent-memory`, `agent-core` deps)
- Modify: `src-tauri/src/bridge.rs` (`Bridge` field, `start`, accept loop)

**Interfaces:**
- Consumes: `agent_memory::{open_memory_parts, MemoryParts, MemoryConfig}`, `agent_runtime_config::RuntimeConfig`, `agent_server::setup::local_params` (Task 4).

- [ ] **Step 1: Add dependencies**

In `src-tauri/Cargo.toml`, under `[dependencies]`, add:

```toml
agent-memory = { path = "../agent/crates/agent-memory" }
agent-core = { path = "../agent/crates/agent-core" }
```

- [ ] **Step 2: Add the `memory_parts` field to `Bridge`**

In `src-tauri/src/bridge.rs`, add to the `Bridge` struct (after `model: String,`):

```rust
    memory_parts: Option<agent_memory::MemoryParts>,
```

- [ ] **Step 3: Load parts once in `start`**

In `start`, after computing `port` and before constructing the `Bridge`, compute the effective memory flag and load parts:

```rust
    // Memory is loaded once (model + DB), gated on the effective (persisted) flag.
    let eff = agent_runtime_config::RuntimeConfig::load_over(
        agent_runtime_config::RuntimeConfig::from_launch(
            "openai".into(), base_url.clone(), model.clone(), "native".into(), 262_144),
        &config_path);
    let memory_parts = if eff.memory {
        match agent_memory::open_memory_parts(agent_memory::MemoryConfig::default()) {
            Ok(parts) => Some(parts),
            Err(e) => { tracing::warn!(target: "memory", "desktop memory disabled: {e}"); None }
        }
    } else {
        None
    };
```

Then add `memory_parts` to the `Bridge { ... }` initializer (after `model,`):

```rust
        memory_parts,
```

- [ ] **Step 4: Pass parts into `local_params` per connection**

In the accept loop, update the `local_params(...)` call to pass the parts:

```rust
            let params = agent_server::setup::local_params(
                dir,
                b.config_path.clone(),
                b.base_url.clone(),
                b.model.clone(),
                b.memory_parts.as_ref(),
            );
```

- [ ] **Step 5: Build the Tauri crate**

Run: `source ~/.cargo/env && cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | grep -E "error|warning: unused|Finished" | head`
Expected: `Finished` with no errors. (First build compiles agent-memory with `onnx`; it may take a while.)

- [ ] **Step 6: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add src-tauri/Cargo.toml src-tauri/src/bridge.rs src-tauri/Cargo.lock
git commit -m "feat(desktop): load memory once at bridge start, assemble per connection"
```

---

### Task 6: `web` — memory toggle in Settings

**Files:**
- Modify: `web/src/wire.ts` (`RuntimeSettings.memory`)
- Modify: `web/src/components/SettingsPanel.tsx` (checkbox)
- Modify: `web/src/components/SettingsPanel.test.tsx` (fixture + toggle test)

**Interfaces:**
- Consumes: the `memory` field now present on the server `RuntimeConfig` (Task 2), round-tripped via the `settings_update` frame.

- [ ] **Step 1: Write the failing test**

In `web/src/components/SettingsPanel.test.tsx`, add `memory: true,` to the `base` fixture (after `enable_thinking: true, preserve_thinking: false,`), then add a toggle test inside the `describe("SettingsPanel sampling inputs", ...)` block:

```tsx
  it("toggles memory off and saves it", () => {
    const onSave = vi.fn();
    render(<SettingsPanel settings={base} meta={null} error={null} disabled={false}
      onSave={onSave} onClose={() => {}} />);
    fireEvent.click(screen.getByLabelText("Long-term memory (remember/recall across sessions)"));
    fireEvent.click(screen.getByText("Save"));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ memory: false }));
  });
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd web && npx vitest run SettingsPanel 2>&1 | tail -15`
Expected: FAIL — `memory` missing on `RuntimeSettings` (type error) and/or label not found.

- [ ] **Step 3: Add `memory` to the type**

In `web/src/wire.ts`, add to the `RuntimeSettings` interface (after `preserve_thinking: boolean;`):

```ts
  memory: boolean;
```

- [ ] **Step 4: Add the checkbox**

In `web/src/components/SettingsPanel.tsx`, add after the `preserve_thinking` `<label>...</label>` block:

```tsx
          <label className="flex items-center gap-2 text-sm">
            <input id="memory" type="checkbox" checked={form.memory}
              onChange={(e) => set("memory", e.target.checked)} />
            Long-term memory (remember/recall across sessions)
          </label>
```

- [ ] **Step 5: Run the test + typecheck**

Run: `cd web && npx vitest run SettingsPanel 2>&1 | tail -8 && npm run typecheck 2>&1 | tail -5`
Expected: vitest PASS; typecheck clean. If `typecheck` flags other `RuntimeSettings` literals missing `memory`, add `memory: true,` to each flagged fixture, then re-run.

- [ ] **Step 6: Full web test run**

Run: `cd web && npm test 2>&1 | tail -12`
Expected: all web tests pass.

- [ ] **Step 7: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/wire.ts web/src/components/SettingsPanel.tsx web/src/components/SettingsPanel.test.tsx
git commit -m "feat(web): long-term memory toggle in Settings"
```

---

### Task 7: Full verification + manual desktop smoke

- [ ] **Step 1: Full Rust workspace tests**

Run: `source ~/.cargo/env && cd agent && cargo test --workspace 2>&1 | grep -E "FAILED|panicked|error\[" | head; echo "ok-groups:"; cargo test --workspace 2>&1 | grep -cE "test result: ok"`
Expected: no FAILED/panicked; ok-groups count unchanged-or-higher vs. before.

- [ ] **Step 2: Tauri build + web build**

Run: `source ~/.cargo/env && cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | tail -2 && cd web && npm run build 2>&1 | tail -3`
Expected: both finish without errors.

- [ ] **Step 3: Manual desktop smoke (human)**

Launch the desktop app. Confirm: (a) memory tools are available and a fact remembered in one session is auto-recalled in a later session without an explicit `recall`; (b) toggling "Long-term memory" off in Settings stops recall/tools on the next turn, and toggling on restores them; (c) switching workspace (pick a different folder) scopes memory to that project. Note: first launch downloads the ~128MB embedding model.

---

## Self-Review

**1. Spec coverage:**
- `agent-memory` split (`MemoryParts`/`open_memory_parts`/`assemble_memory`, reimplement `build_tools_and_retriever`) → Task 1. ✓
- `RuntimeConfig.memory` default-true + overlay → Task 2 (field + serde default + from_launch). The wire path round-trips full `RuntimeConfig`, so no separate overlay struct change is needed; the web type carries it (Task 6). ✓
- `DaemonParams` retriever + budget; `RuntimeState`; `build_loop` gating; `WindowContext::with_recall_budget` → Task 3. ✓
- `local_params` assembles from `Option<&MemoryParts>` → Task 4. ✓
- `src-tauri` load-once + per-connection assemble → Task 5. ✓
- Settings UI toggle (`wire.ts` + `SettingsPanel`) → Task 6. ✓
- Degradation (best-effort, never fatal) → Task 1 (`open_memory_parts` returns Err), Task 5 (Err→None warn), Task 3 (cfg.memory gate). ✓
- Shared `~/.agent/memory.db`, project-scoped per workspace → Task 1 `assemble_memory` (scope from workspace), Task 5 default `MemoryConfig`. ✓
- Tests per crate + manual smoke → Tasks 1–7. ✓

**2. Placeholder scan:** No TBD/TODO. Two implementer notes (Task 3 Step 1: match the existing test's tool-construction style; Task 6 Step 5: add `memory` to any other flagged fixture) are verification instructions resolved deterministically by the compiler/typechecker, not vague requirements.

**3. Type consistency:**
- `MemoryParts { embedder, store, cfg }` — Task 1 def, used in Tasks 4 (`parts.cfg.recall_token_budget`) and 5. ✓
- `open_memory_parts(MemoryConfig) -> Result<MemoryParts, MemoryInitError>` — Task 1, used Task 5. ✓
- `assemble_memory(&MemoryParts, &Path) -> (Vec<Arc<dyn Tool>>, Arc<dyn Retriever>)` — Task 1, used Task 4. ✓
- `build_loop(..., memory_retriever: Option<&Arc<dyn Retriever>>)` — Task 3 def + both call sites (`new`, `apply`) + tests. ✓
- `DaemonParams.memory_retriever: Option<Arc<dyn Retriever>>`, `.recall_token_budget: usize` — Task 3 def, set in Task 4, read in Task 3 `serve`. ✓
- `RuntimeConfig.memory: bool` — Task 2 def, read in Task 3 `build_loop`, Task 5 effective-flag check, Task 6 web type. ✓
- `local_params(..., Option<&MemoryParts>)` — Task 4 def, called in Task 5. ✓
- `RuntimeSettings.memory: boolean` — Task 6, mirrors the Rust field name `memory`. ✓
