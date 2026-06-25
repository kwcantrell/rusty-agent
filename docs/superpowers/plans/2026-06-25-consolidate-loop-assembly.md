# Consolidate Loop Assembly Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the duplicated `AgentLoop` assembly in the CLI and server with one shared `assemble_loop(cfg, parts)` + `loop_config_from(cfg)` in `agent-runtime-config`.

**Architecture:** A new `agent-runtime-config::assemble` module holds `LoopParts` (per-frontend pieces), `BuiltLoop` (result), `assemble_loop` (the orchestration lifted from the server's `build_loop`), and `loop_config_from` (the single `RuntimeConfig → LoopConfig` mapping). The server's `build_loop` shrinks to a thin wrapper; the CLI populates its `RuntimeConfig` fully and calls `assemble_loop`.

**Tech Stack:** Rust (agent-runtime-config, agent-server, agent-cli).

## Global Constraints

- `cargo` is not on PATH — every Rust shell step runs `source ~/.cargo/env` first, from the `agent/` directory.
- `git` runs from repo root `/home/kalen/rust-agent-runtime`.
- Behavior-preserving refactor. The ONE intended behavioral delta: the CLI's denylist becomes `cfg.effective_denylist()` = `HARD_FLOOR_DENYLIST ∪ default_denylist()` (a safe superset of today's CLI denylist).
- No changes to `RuntimeConfig` persistence, the settings wire format, or the web UI. `stream_idle_timeout` is carried on `LoopParts`, not `RuntimeConfig`.
- Loop constants that are identical on both sides today stay as literals in `loop_config_from`: `max_retries: 3`, `tool_timeout: Duration::from_secs(120)`, `max_parallel_tools: 8`.

---

### Task 1: `assemble_loop` + `loop_config_from` in `agent-runtime-config`

**Files:**
- Modify: `agent/crates/agent-runtime-config/Cargo.toml` (add `agent-policy` dep)
- Create: `agent/crates/agent-runtime-config/src/assemble.rs`
- Modify: `agent/crates/agent-runtime-config/src/lib.rs` (declare module + re-export)
- Test: inline `#[cfg(test)]` in `assemble.rs`

**Interfaces:**
- Consumes: `RuntimeConfig` (fields + `effective_denylist()`), leaf builders `build_registry`/`build_skills`/`build_sandbox`/`pick_protocol`, `agent_core::{AgentLoop, EventSink, LoopConfig, Retriever}`, `agent_model::ModelClient`, `agent_policy::{ApprovalChannel, RulePolicy}`, `agent_tools::Tool`, `agent_skills::{compose_system_prompt, SkillRegistry}`.
- Produces:
  - `pub struct LoopParts { pub model: Arc<dyn ModelClient>, pub sink: Arc<dyn EventSink>, pub approval: Arc<dyn ApprovalChannel>, pub workspace: PathBuf, pub mcp_tools: Vec<Arc<dyn Tool>>, pub memory_tools: Vec<Arc<dyn Tool>>, pub memory_retriever: Option<Arc<dyn Retriever>>, pub stream_idle_timeout: Duration, pub base_system_prompt: String }`
  - `pub struct BuiltLoop { pub loop_: Arc<AgentLoop>, pub system_prompt: String, pub unknown_presets: Vec<String>, #[cfg(test)] pub registered_names: Vec<String> }`
  - `pub fn loop_config_from(cfg: &RuntimeConfig, workspace: PathBuf, stream_idle_timeout: Duration) -> LoopConfig`
  - `pub fn assemble_loop(cfg: &RuntimeConfig, parts: LoopParts) -> BuiltLoop`

- [ ] **Step 1: Add the `agent-policy` dependency**

In `agent/crates/agent-runtime-config/Cargo.toml`, under `[dependencies]` (after `agent-core = ...`):

```toml
agent-policy = { path = "../agent-policy" }
```

- [ ] **Step 2: Write the failing tests**

Create `agent/crates/agent-runtime-config/src/assemble.rs` with the imports, a placeholder, and the test module (implementation added in Step 4). Start with ONLY this content so the test compiles against the to-be-written API:

```rust
use crate::{build_registry, build_sandbox, build_skills, pick_protocol, RuntimeConfig};
use agent_core::{AgentLoop, EventSink, LoopConfig, Retriever};
use agent_model::ModelClient;
use agent_policy::{ApprovalChannel, RulePolicy};
use agent_skills::{compose_system_prompt, SkillRegistry};
use agent_tools::Tool;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub struct LoopParts {
    pub model: Arc<dyn ModelClient>,
    pub sink: Arc<dyn EventSink>,
    pub approval: Arc<dyn ApprovalChannel>,
    pub workspace: PathBuf,
    pub mcp_tools: Vec<Arc<dyn Tool>>,
    pub memory_tools: Vec<Arc<dyn Tool>>,
    pub memory_retriever: Option<Arc<dyn Retriever>>,
    pub stream_idle_timeout: Duration,
    pub base_system_prompt: String,
}

pub struct BuiltLoop {
    pub loop_: Arc<AgentLoop>,
    pub system_prompt: String,
    pub unknown_presets: Vec<String>,
    #[cfg(test)]
    pub registered_names: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{AgentEvent, EventSink as _};
    use agent_policy::{ApprovalRequest, ApprovalResponse};
    use agent_model::{BoxStream, CompletionRequest, ModelClient, ModelError};

    // Minimal fakes (no network / no real model).
    struct NoModel;
    #[async_trait::async_trait]
    impl ModelClient for NoModel {
        async fn stream(&self, _r: CompletionRequest) -> Result<BoxStream, ModelError> {
            Err(ModelError::Http("unused".into()))
        }
    }
    struct NoSink;
    impl EventSink for NoSink { fn emit(&self, _e: AgentEvent) {} }
    struct NoApproval;
    #[async_trait::async_trait]
    impl ApprovalChannel for NoApproval {
        async fn request(&self, _r: ApprovalRequest) -> ApprovalResponse { ApprovalResponse::Approve }
    }

    fn fake_mem(name: &'static str) -> Arc<dyn Tool> {
        use agent_tools::{Access, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
        struct M(&'static str);
        #[async_trait::async_trait]
        impl Tool for M {
            fn name(&self) -> &str { self.0 }
            fn description(&self) -> &str { "fake" }
            fn schema(&self) -> ToolSchema {
                ToolSchema { name: self.0.into(), description: "fake".into(),
                    parameters: serde_json::json!({"type":"object"}) }
            }
            fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
                Ok(ToolIntent { tool: self.0.into(), access: Access::Read, paths: vec![],
                    command: None, summary: "x".into() })
            }
            async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx)
                -> Result<ToolOutput, ToolError> {
                Ok(ToolOutput { content: "ok".into(), display: None })
            }
        }
        Arc::new(M(name))
    }

    fn parts(workspace: PathBuf, mem: Vec<Arc<dyn Tool>>) -> LoopParts {
        LoopParts {
            model: Arc::new(NoModel), sink: Arc::new(NoSink), approval: Arc::new(NoApproval),
            workspace, mcp_tools: vec![], memory_tools: mem, memory_retriever: None,
            stream_idle_timeout: Duration::from_secs(99), base_system_prompt: "BASE".into(),
        }
    }

    fn cfg() -> RuntimeConfig {
        RuntimeConfig::from_launch("openai".into(), "http://x".into(), "m".into(), "native".into(), 8192)
    }

    #[test]
    fn registers_memory_tools_when_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg(); c.memory = true;
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![fake_mem("remember")]));
        assert!(built.registered_names.iter().any(|n| n == "remember"));
    }

    #[test]
    fn skips_memory_tools_when_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg(); c.memory = false;
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![fake_mem("remember")]));
        assert!(!built.registered_names.iter().any(|n| n == "remember"));
    }

    #[test]
    fn unknown_active_skill_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg(); c.active_skills = vec!["definitely-not-a-real-skill".into()];
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        assert!(built.unknown_presets.iter().any(|n| n == "definitely-not-a-real-skill"));
    }

    #[test]
    fn loop_config_maps_runtime_config() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.temperature = 0.7; c.max_turns = 9; c.max_tokens = 1234; c.context_limit = 5000;
        c.top_p = Some(0.5); c.enable_thinking = false; c.preserve_thinking = true;
        let lc = loop_config_from(&c, dir.path().to_path_buf(), Duration::from_secs(77));
        assert_eq!(lc.model_limit, 5000);
        assert_eq!(lc.max_turns, 9);
        assert_eq!(lc.max_retries, 3);
        assert_eq!(lc.max_tokens, Some(1234));
        assert_eq!(lc.tool_timeout, Duration::from_secs(120));
        assert_eq!(lc.stream_idle_timeout, Duration::from_secs(77));
        assert_eq!(lc.max_parallel_tools, 8);
        assert_eq!(lc.top_p, Some(0.5));
        assert!(!lc.enable_thinking);
        assert!(lc.preserve_thinking);
        assert!((lc.temperature - 0.7).abs() < 1e-6);
        assert!(lc.sandbox.is_some());
    }
}
```

> Verify the `agent_model` re-exports `BoxStream`, `CompletionRequest`, `ModelError` at crate root (they are used by `agent-core`'s loop, so they are public). If `BoxStream` is named differently, match the actual `ModelClient::stream` return type in `agent/crates/agent-model/src/lib.rs`.

- [ ] **Step 3: Run tests to verify they fail**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config --lib assemble 2>&1 | tail -8`
Expected: FAIL to compile — `assemble_loop` / `loop_config_from` not found (and module not declared).

- [ ] **Step 4: Implement `loop_config_from` and `assemble_loop`**

Append to `agent/crates/agent-runtime-config/src/assemble.rs` (above the `#[cfg(test)]` module):

```rust
/// The single RuntimeConfig → LoopConfig mapping. Constants that are identical on
/// both front-ends today stay literals here; `stream_idle_timeout` is frontend-supplied.
pub fn loop_config_from(
    cfg: &RuntimeConfig,
    workspace: PathBuf,
    stream_idle_timeout: Duration,
) -> LoopConfig {
    LoopConfig {
        model_limit: cfg.context_limit,
        max_turns: cfg.max_turns,
        max_retries: 3,
        temperature: cfg.temperature,
        max_tokens: Some(cfg.max_tokens),
        workspace,
        tool_timeout: Duration::from_secs(120),
        stream_idle_timeout,
        top_p: cfg.top_p,
        top_k: cfg.top_k,
        min_p: cfg.min_p,
        presence_penalty: cfg.presence_penalty,
        repeat_penalty: cfg.repeat_penalty,
        enable_thinking: cfg.enable_thinking,
        preserve_thinking: cfg.preserve_thinking,
        sandbox: Some(build_sandbox(cfg)),
        max_parallel_tools: 8,
    }
}

/// The one place a RuntimeConfig + per-frontend `LoopParts` become an `AgentLoop`.
/// Used by both the CLI and the server. Never panics: a `compose_system_prompt`
/// failure falls back to the base prompt.
pub fn assemble_loop(cfg: &RuntimeConfig, parts: LoopParts) -> BuiltLoop {
    let mut registry = build_registry(&cfg.http_allow_hosts);
    for t in &parts.mcp_tools {
        registry.register(t.clone());
    }
    if cfg.memory {
        for t in &parts.memory_tools {
            registry.register(t.clone());
        }
    }
    let (skill_registry, skill_tools) = build_skills(&cfg.skills_dirs, &parts.workspace);
    for t in skill_tools {
        registry.register(t);
    }

    let available: HashSet<String> =
        skill_registry.scan().into_iter().map(|s| s.name).collect();
    let mut presets = Vec::new();
    let mut unknown_presets = Vec::new();
    for name in &cfg.active_skills {
        if available.contains(name) {
            presets.push(name.clone());
        } else {
            tracing::warn!(skill = %name, "active skill not found; dropping from system prompt");
            unknown_presets.push(name.clone());
        }
    }
    let system_prompt = match compose_system_prompt(&parts.base_system_prompt, &skill_registry, &presets) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "compose_system_prompt failed unexpectedly; using base prompt");
            parts.base_system_prompt.clone()
        }
    };

    #[cfg(test)]
    let registered_names: Vec<String> = registry.schemas().into_iter().map(|s| s.name).collect();

    let policy = Arc::new(RulePolicy {
        workspace: parts.workspace.clone(),
        command_allowlist: cfg.command_allowlist.clone(),
        command_denylist: cfg.effective_denylist(),
    });

    let agent = AgentLoop::new(
        parts.model,
        pick_protocol(&cfg.protocol),
        Arc::new(registry),
        policy,
        parts.approval,
        parts.sink,
        loop_config_from(cfg, parts.workspace.clone(), parts.stream_idle_timeout),
    );
    let agent = match (cfg.memory, &parts.memory_retriever) {
        (true, Some(r)) => agent.with_retriever(r.clone()),
        _ => agent,
    };

    BuiltLoop {
        loop_: Arc::new(agent),
        system_prompt,
        unknown_presets,
        #[cfg(test)]
        registered_names,
    }
}
```

- [ ] **Step 5: Declare the module and re-export**

In `agent/crates/agent-runtime-config/src/lib.rs`, add after `pub use runtime_config::{...};` (top of the file):

```rust
mod assemble;
pub use assemble::{assemble_loop, loop_config_from, BuiltLoop, LoopParts};
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config 2>&1 | grep -E "test result|error\[|error:" | head`
Expected: PASS — the four new `assemble`-module tests plus all existing runtime-config tests.

- [ ] **Step 7: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-runtime-config/Cargo.toml agent/crates/agent-runtime-config/src/assemble.rs agent/crates/agent-runtime-config/src/lib.rs
git commit -m "feat(runtime-config): shared assemble_loop + loop_config_from"
```

---

### Task 2: Reduce the server's `build_loop` to a wrapper

**Files:**
- Modify: `agent/crates/agent-server/src/runtime.rs` (`build_loop` body, delete local `BuiltLoop`, remove moved tests)
- Test: inline `#[cfg(test)]` in `runtime.rs`

**Interfaces:**
- Consumes: `agent_runtime_config::{assemble_loop, BuiltLoop, LoopParts}` (Task 1), `build_model` (existing), `agent_core::DEFAULT_STREAM_IDLE_TIMEOUT`.
- Produces: `build_loop(...) -> agent_runtime_config::BuiltLoop` with the same parameter list it has today; `RuntimeState` consumes `built.loop_` / `built.system_prompt` / `built.unknown_presets` unchanged.

- [ ] **Step 1: Replace the `build_loop` body and delete the local `BuiltLoop`**

In `agent/crates/agent-server/src/runtime.rs`:

Update imports — add to the `agent_runtime_config` use line:

```rust
use agent_runtime_config::{assemble_loop, build_model, BuiltLoop, LoopParts, pick_protocol, RuntimeConfig, HARD_FLOOR_DENYLIST};
```

(Remove `build_registry`, `build_sandbox`, `build_skills` from that use line if they become unused after this change; the compiler will warn. Keep `pick_protocol`/`HARD_FLOOR_DENYLIST` only if still referenced — remove if not.)

Delete the local `struct BuiltLoop { ... }` definition in `runtime.rs` (now provided by `agent_runtime_config`).

Replace the entire `fn build_loop(...) -> BuiltLoop { ... }` body with the wrapper:

```rust
/// Build the loop for the current config. Thin wrapper over the shared
/// `agent_runtime_config::assemble_loop`; this crate supplies the per-frontend
/// parts (WebSocket sink/approval, the model, injected tools).
#[allow(clippy::too_many_arguments)]
fn build_loop(
    cfg: &RuntimeConfig,
    sink: &Arc<WsEventSink>,
    approval: &Arc<WsApprovalChannel>,
    workspace: &Path,
    api_key: &Option<String>,
    claude_binary: &str,
    mcp_tools: &[Arc<dyn Tool>],
    memory_tools: &[Arc<dyn Tool>],
    memory_retriever: Option<&Arc<dyn agent_core::Retriever>>,
    base_system_prompt: &str,
) -> BuiltLoop {
    let model = build_model(&cfg.backend, &cfg.base_url, &cfg.model, claude_binary, api_key.clone());
    assemble_loop(cfg, LoopParts {
        model,
        sink: sink.clone(),
        approval: approval.clone(),
        workspace: workspace.to_path_buf(),
        mcp_tools: mcp_tools.to_vec(),
        memory_tools: memory_tools.to_vec(),
        memory_retriever: memory_retriever.cloned(),
        stream_idle_timeout: agent_core::DEFAULT_STREAM_IDLE_TIMEOUT,
        base_system_prompt: base_system_prompt.to_string(),
    })
}
```

> `sink.clone()` is `Arc<WsEventSink>` → coerces to `Arc<dyn EventSink>` at the `LoopParts` field; same for `approval`. If the compiler needs an explicit coercion, write `sink.clone() as Arc<dyn agent_core::EventSink>`.

- [ ] **Step 2: Remove the moved injection tests**

Delete the `build_loop_registers_injected_memory_tools` and `build_loop_skips_memory_tools_when_disabled` tests from `runtime.rs` (they now live in `agent-runtime-config`, which is where `assemble_loop` is). Keep the existing `build_loop` smoke test (the one that builds a loop and asserts it constructs without panic) and the `apply_*` tests (they cover the unknown-preset → wire-error path).

> If the remaining smoke test referenced `result.registered_names`, change it to assert on `result.loop_` being usable instead (e.g. `let _ = result.loop_;`), since `registered_names` is only present in the `agent-runtime-config` crate's own test builds.

- [ ] **Step 3: Build and test the crate**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-server 2>&1 | grep -E "test result|error\[|error:|warning: unused" | head -15`
Expected: PASS. Remove any now-unused imports the compiler flags.

- [ ] **Step 4: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-server/src/runtime.rs
git commit -m "refactor(agent-server): build_loop delegates to shared assemble_loop"
```

---

### Task 3: Route the CLI through `assemble_loop`

**Files:**
- Modify: `agent/crates/agent-cli/src/main.rs` (config population, loop construction, imports)
- Test: inline `#[cfg(test)]` in `main.rs`

**Interfaces:**
- Consumes: `agent_runtime_config::{assemble_loop, LoopParts}`, the existing `build_memory_full`, `build_model`, `connect_mcp`.
- Produces: `fn runtime_config_from_cli(cli: &Cli, protocol_name: &str) -> RuntimeConfig` (testable flag→config mapping).

- [ ] **Step 1: Write the failing test**

Add to a `#[cfg(test)] mod tests` block at the bottom of `agent/crates/agent-cli/src/main.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn runtime_config_from_cli_carries_loop_fields() {
        let cli = Cli::parse_from([
            "agent", "--model", "m", "--base-url", "http://x",
            "--top-p", "0.9", "--max-turns-unused-placeholder-ignore", // see note
        ].iter().filter(|a| !a.contains("placeholder")));
        let rc = runtime_config_from_cli(&cli, "native");
        assert_eq!(rc.model, "m");
        assert_eq!(rc.base_url, "http://x");
        assert_eq!(rc.protocol, "native");
        assert_eq!(rc.top_p, Some(0.9));
        assert_eq!(rc.memory, cli.memory);
        assert_eq!(rc.http_allow_hosts, cli.allow_host);
        assert_eq!(rc.skills_dirs, cli.skills_dir);
        assert_eq!(rc.active_skills, cli.skill);
        // CLI carries its default command lists into the config.
        assert!(!rc.command_allowlist.is_empty());
    }
}
```

> Drop the placeholder filter line — write the args cleanly as the real flags your `Cli` accepts. The point of the test: every loop-relevant flag lands in the returned `RuntimeConfig`. Use only flags that exist on `Cli` (see the struct: `--model`, `--base-url`, `--top-p`, etc.). `Cli::parse_from` fills the rest with their clap defaults.

- [ ] **Step 2: Run to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-cli runtime_config_from_cli 2>&1 | tail -6`
Expected: FAIL to compile — `runtime_config_from_cli` not found.

- [ ] **Step 3: Extract the flag→config mapping**

In `agent/crates/agent-cli/src/main.rs`, add this free function (near the top, after imports):

```rust
/// Map CLI flags to a complete RuntimeConfig so the loop is assembled the same way
/// as the server (via agent_runtime_config::assemble_loop).
fn runtime_config_from_cli(cli: &Cli, protocol_name: &str) -> agent_runtime_config::RuntimeConfig {
    let mut c = agent_runtime_config::RuntimeConfig::from_launch(
        cli.backend.clone(), cli.base_url.clone(), cli.model.clone(),
        protocol_name.to_string(), cli.context_limit);
    // Sandbox
    c.sandbox_mode = cli.sandbox_mode.clone();
    c.sandbox_image = cli.sandbox_image.clone();
    c.sandbox_network = cli.sandbox_network;
    c.sandbox_memory = cli.sandbox_memory.clone();
    c.sandbox_cpus = cli.sandbox_cpus.clone();
    c.sandbox_pids = cli.sandbox_pids;
    c.sandbox_fsize = cli.sandbox_fsize.clone();
    c.sandbox_tmp_size = cli.sandbox_tmp_size.clone();
    c.sandbox_extra_rw = cli.sandbox_extra_rw.clone();
    c.sandbox_extra_ro = cli.sandbox_extra_ro.clone();
    // Sampling + thinking
    c.temperature = 0.2;
    c.max_turns = 25;
    c.max_tokens = 2048;
    c.top_p = cli.top_p;
    c.top_k = cli.top_k;
    c.min_p = cli.min_p;
    c.presence_penalty = cli.presence_penalty;
    c.repeat_penalty = cli.repeat_penalty;
    c.enable_thinking = !cli.no_thinking;
    c.preserve_thinking = cli.preserve_thinking;
    // Tools / skills / memory / network
    c.http_allow_hosts = cli.allow_host.clone();
    c.skills_dirs = cli.skills_dir.clone();
    c.active_skills = cli.skill.clone();
    c.memory = cli.memory;
    c.command_allowlist = default_allowlist();
    c.command_denylist = default_denylist();
    c
}
```

- [ ] **Step 4: Run to verify the test passes**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-cli runtime_config_from_cli 2>&1 | grep -E "test result|error" | head`
Expected: PASS.

- [ ] **Step 5: Replace the inline loop construction with `assemble_loop`**

In `agent/crates/agent-cli/src/main.rs`, replace the block from `let mut sbcfg = ...` (the `RuntimeConfig::from_launch` at ~line 154) through the end of the `WindowContext` construction (~line 226) with:

```rust
    let rt = runtime_config_from_cli(&cli, protocol_name);
    let sandbox = build_sandbox(&rt);

    // MCP servers (if configured): collect tools, keep the manager alive for the session.
    let mut mcp_tools: Vec<Arc<dyn agent_tools::Tool>> = Vec::new();
    let mcp_manager = match &cli.mcp_config {
        Some(path) => {
            let mgr = agent_runtime_config::connect_mcp(path, sandbox.clone()).await;
            println!("{}", mgr.summary_line());
            mcp_tools = mgr.tools();
            Some(mgr)
        }
        None => None,
    };

    // Long-term memory: construct once (loads the embedding model); pass tools + retriever in.
    let memory = build_memory_full(cli.memory, cli.memory_db.clone(),
        cli.memory_model_dir.clone(), &workspace);

    let built = assemble_loop(&rt, LoopParts {
        model,
        sink: Arc::new(TerminalSink::default()),
        approval: Arc::new(TerminalApproval),
        workspace: workspace.clone(),
        mcp_tools,
        memory_tools: memory.tools.clone(),
        memory_retriever: memory.retriever.clone(),
        stream_idle_timeout: Duration::from_secs(cli.stream_timeout_secs),
        base_system_prompt: BASE_SYSTEM_PROMPT.to_string(),
    });
    if !built.unknown_presets.is_empty() {
        eprintln!("skills: unknown active skill(s): {}", built.unknown_presets.join(", "));
        std::process::exit(2);
    }
    let agent = built.loop_;

    let mut ctx = WindowContext::new(Message::system(built.system_prompt))
        .with_recall_budget(memory.recall_token_budget);
```

Update the `use agent_runtime_config::{...}` line at the top of `main.rs` to add `assemble_loop, LoopParts` and drop names no longer used directly (`build_registry`, `build_skills`, `pick_protocol` are now used inside `assemble_loop`; keep `build_memory_full`, `build_model`, `build_sandbox`, `backend_name_is_valid`, `default_allowlist`, `default_denylist`). The compiler will flag unused imports — remove exactly those it names. `pick_protocol` is still needed iff `protocol_name` logic uses it — it does not (it only computes the name string), so it can be dropped from the import.

> Keep the existing `model`, `protocol_name`, `api_key`, `workspace`, and `BASE_SYSTEM_PROMPT` bindings above this block exactly as they are. `RulePolicy`, `LoopConfig`, `AgentLoop` imports in `main.rs` may become unused — remove them if the compiler flags them (the loop is now built inside `assemble_loop`). `WindowContext` and `Message` stay.

- [ ] **Step 6: Build + test the CLI**

Run: `source ~/.cargo/env && cd agent && cargo build -p agent-cli 2>&1 | grep -E "error|warning: unused" | head; cargo test -p agent-cli 2>&1 | grep -E "test result|error" | head`
Expected: builds clean (remove any unused imports flagged); tests PASS.

- [ ] **Step 7: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-cli/src/main.rs
git commit -m "refactor(agent-cli): assemble the loop via shared assemble_loop"
```

---

### Task 4: Full verification

- [ ] **Step 1: Whole Rust workspace**

Run: `source ~/.cargo/env && cd agent && cargo test --workspace 2>&1 | grep -E "FAILED|panicked|error\[" | head; echo "ok-groups:"; cargo test --workspace 2>&1 | grep -cE "test result: ok"`
Expected: no FAILED/panicked.

- [ ] **Step 2: Tauri crate builds (server path compiles end-to-end)**

Run: `source ~/.cargo/env && cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | tail -1`
Expected: `Finished`.

- [ ] **Step 3: Live e2e regression (backend must be up on :8080)**

Run:
```bash
source ~/.cargo/env && cd agent
AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
  cargo test -p agent-runtime-config --test e2e_auto_retrieval -- --ignored --nocapture 2>&1 | grep -E "MODEL ANSWER|test result"
AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
  cargo test -p agent-core --test e2e_sglang -- --ignored 2>&1 | grep -E "test result"
```
Expected: both PASS (auto-retrieval still answers "BANANA-7"; loop still reads a file). If the backend is down, skip and note it.

---

## Self-Review

**1. Spec coverage:**
- Shared `assemble_loop` + `LoopParts` + `BuiltLoop` → Task 1. ✓
- One `loop_config_from` mapping → Task 1 (+ field-mapping test). ✓
- `stream_idle_timeout` on `LoopParts` (not RuntimeConfig) → Task 1 struct, Task 2 (server passes default), Task 3 (CLI passes flag). ✓
- `agent-policy` dep on runtime-config → Task 1 Step 1. ✓
- Server `build_loop` → thin wrapper → Task 2. ✓
- CLI populates RuntimeConfig fully + drops hand-rolled LoopConfig → Task 3. ✓
- CLI denylist tightening (effective_denylist) → Task 1 `assemble_loop` (uses `cfg.effective_denylist()`) + Task 3 (`command_denylist = default_denylist()`). ✓
- Move injection tests to runtime-config; server keeps smoke test → Task 1 (new tests) + Task 2 Step 2 (remove). ✓
- No RuntimeConfig/wire/web change → confirmed (no edits to runtime_config.rs fields, wire, or web). ✓
- Behavior-preserving; regression via workspace + live e2e → Task 4. ✓

**2. Placeholder scan:** No TBD/TODO. The Task 3 Step 1 test has an explicit "write the args cleanly / drop the placeholder filter" instruction — that is a deliberate authoring note (the exact flag set depends on the real `Cli`), not a shipped placeholder; the implementer resolves it against the visible `Cli` struct.

**3. Type consistency:**
- `LoopParts` / `BuiltLoop` fields identical across Tasks 1–3. ✓
- `assemble_loop(&RuntimeConfig, LoopParts) -> BuiltLoop` — Task 1 def, Tasks 2 & 3 calls. ✓
- `loop_config_from(&RuntimeConfig, PathBuf, Duration) -> LoopConfig` — Task 1 def + test. ✓
- `runtime_config_from_cli(&Cli, &str) -> RuntimeConfig` — Task 3 def + test. ✓
- `BuiltLoop.unknown_presets: Vec<String>` consumed identically by server (`apply` error) and CLI (`exit(2)`). ✓
- `#[cfg(test)] registered_names` lives only in runtime-config tests (Task 1); server stops using it (Task 2 Step 2). ✓
