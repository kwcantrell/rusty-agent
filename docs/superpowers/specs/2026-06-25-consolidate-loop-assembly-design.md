# Consolidate Loop Assembly — Design Spec

**Date:** 2026-06-25
**Status:** Approved (design), pending spec review
**Scope:** Replace the duplicated `AgentLoop` assembly in the CLI and the server with one shared, `RuntimeConfig`-driven builder in `agent-runtime-config`. Behavior-preserving refactor (one small, safe policy tightening noted below).

## Problem

Two places independently orchestrate building an `AgentLoop`:

- `agent-cli/src/main.rs` (~lines 167–223): builds the registry, registers MCP/memory/skill tools, composes the system prompt, builds the model/sandbox, and hand-rolls a `LoopConfig` from CLI flags + hardcoded constants.
- `agent-server/src/runtime.rs::build_loop` (~lines 165–250): does the same, driven by `RuntimeConfig`, returning a `BuiltLoop`.

They share the *leaf* builders (`build_registry`, `build_skills`, `build_model`, `build_sandbox`, `pick_protocol` in `runtime-config/lib.rs`) but **duplicate the orchestration and the `LoopConfig` derivation**. This has already caused divergence: memory is wired differently on each side (CLI: `build_memory_full` + inline `.with_retriever`; server: `cfg.memory`-gated `build_loop` + `MemoryParts`), which had to be implemented twice across the last two features. The graph flags `build_loop()` as the #2 god node and a top cross-community bridge for exactly this reason.

## Goals

- One function assembles the loop for both front-ends; each caller shrinks to "build my pieces → call it → handle unknown presets."
- One `RuntimeConfig → LoopConfig` mapping (`loop_config_from`), eliminating the parallel hand-rolled `LoopConfig` in the CLI.
- No change to `RuntimeConfig` persistence, the settings wire format, or the web UI.
- Behavior-preserving: the resulting loop is functionally identical on both sides (see the one intentional delta below).

## Non-Goals

- No new features (no auto-ingestion, no UI changes).
- No change to the leaf builders' signatures or behavior.
- No change to how each front-end *sources* its inputs (CLI still parses flags; server still uses live `RuntimeConfig`) beyond the CLI populating its config fully.
- Not touching `loop_.rs` internals — only how `AgentLoop` is constructed.

## Architecture

A new shared builder in `agent-runtime-config`:

```rust
pub struct LoopParts {
    pub model: Arc<dyn ModelClient>,
    pub sink: Arc<dyn EventSink>,            // TerminalSink (CLI) | WsEventSink (server)
    pub approval: Arc<dyn ApprovalChannel>,  // TerminalApproval | WsApprovalChannel
    pub workspace: PathBuf,
    pub mcp_tools: Vec<Arc<dyn Tool>>,
    pub memory_tools: Vec<Arc<dyn Tool>>,
    pub memory_retriever: Option<Arc<dyn Retriever>>,
    pub stream_idle_timeout: Duration,       // CLI: --stream-timeout; server: DEFAULT_STREAM_IDLE_TIMEOUT
    pub base_system_prompt: String,
}

pub struct BuiltLoop {
    pub loop_: Arc<AgentLoop>,
    pub system_prompt: String,
    pub unknown_presets: Vec<String>,
    #[cfg(test)]
    pub registered_names: Vec<String>,
}

/// The one place a RuntimeConfig + per-frontend parts become a loop.
pub fn assemble_loop(cfg: &RuntimeConfig, parts: LoopParts) -> BuiltLoop;

/// The one place RuntimeConfig maps to LoopConfig.
pub fn loop_config_from(cfg: &RuntimeConfig, workspace: PathBuf, stream_idle_timeout: Duration) -> LoopConfig;
```

### `assemble_loop` body (the orchestration, lifted from the server's `build_loop`)

1. `let mut registry = build_registry(&cfg.http_allow_hosts);`
2. register `parts.mcp_tools`.
3. if `cfg.memory` { register `parts.memory_tools` }.
4. `let (skill_registry, skill_tools) = build_skills(&cfg.skills_dirs, &parts.workspace);` register `skill_tools`.
5. filter `cfg.active_skills` against `skill_registry.scan()` → `presets` (known) + `unknown_presets`; `compose_system_prompt(&parts.base_system_prompt, &skill_registry, &presets)` with the existing error-fallback-to-base.
6. `let policy = Arc::new(RulePolicy { workspace: parts.workspace.clone(), command_allowlist: cfg.command_allowlist.clone(), command_denylist: cfg.effective_denylist() });`
7. `let agent = AgentLoop::new(parts.model, pick_protocol(&cfg.protocol), Arc::new(registry), policy, parts.approval, parts.sink, loop_config_from(cfg, parts.workspace, parts.stream_idle_timeout));`
8. if `cfg.memory` { if let Some(r) = &parts.memory_retriever { agent = agent.with_retriever(r.clone()) } }.
9. return `BuiltLoop { loop_: Arc::new(agent), system_prompt, unknown_presets, #[cfg(test)] registered_names }`.

### `loop_config_from` (the one LoopConfig mapping)

```rust
LoopConfig {
    model_limit: cfg.context_limit,
    max_turns: cfg.max_turns,
    max_retries: 3,                                   // constant in both today
    temperature: cfg.temperature,
    max_tokens: Some(cfg.max_tokens),
    workspace,
    tool_timeout: Duration::from_secs(120),           // constant in both today
    stream_idle_timeout,                              // from LoopParts
    top_p: cfg.top_p, top_k: cfg.top_k, min_p: cfg.min_p,
    presence_penalty: cfg.presence_penalty, repeat_penalty: cfg.repeat_penalty,
    enable_thinking: cfg.enable_thinking,
    preserve_thinking: cfg.preserve_thinking,
    sandbox: Some(build_sandbox(cfg)),
    max_parallel_tools: 8,                            // constant in both today
}
```

### Caller changes

- **`agent-runtime-config`** — add `LoopParts`, `BuiltLoop`, `assemble_loop`, `loop_config_from`; add `agent-policy` as a dependency (for `RulePolicy` + `ApprovalChannel`); the leaf builders stay as-is.
- **`agent-server/src/runtime.rs`** — `build_loop` shrinks to: fill `LoopParts` from `RuntimeState` fields (sink/approval as `Arc<dyn _>`, `mcp_tools`/`memory_tools` to `Vec`, `memory_retriever`, `stream_idle_timeout: DEFAULT_STREAM_IDLE_TIMEOUT`, `base_system_prompt`), call `assemble_loop`, return its `BuiltLoop`. `RuntimeState::{new,apply}` keep mapping `unknown_presets` → wire error exactly as today.
- **`agent-cli/src/main.rs`** — fully populate its `RuntimeConfig` (the existing `sbcfg`) from flags: `temperature`, `max_turns`, `max_tokens`, `top_p/top_k/min_p/presence_penalty/repeat_penalty`, `enable_thinking = !cli.no_thinking`, `preserve_thinking`, `memory = cli.memory`, `http_allow_hosts = cli.allow_host`, `skills_dirs = cli.skills_dir`, `active_skills = cli.skill`, `command_allowlist = default_allowlist()`, `command_denylist = default_denylist()` (already-set: backend/url/model/protocol/context_limit/sandbox_*). Build model + MCP + memory pieces, fill `LoopParts` (sink `TerminalSink`, approval `TerminalApproval`, `stream_idle_timeout: Duration::from_secs(cli.stream_timeout_secs)`), call `assemble_loop`, exit 2 if `unknown_presets` is non-empty, then use `built.system_prompt` for the `WindowContext` (with `.with_recall_budget(memory.recall_token_budget)` as today).

## Behavior Notes (intentional deltas)

- **CLI denylist gains the hard floor.** Today the CLI passes `default_denylist()` directly to `RulePolicy`; the consolidated path uses `cfg.effective_denylist()` = `HARD_FLOOR_DENYLIST ∪ cfg.command_denylist`. With `cfg.command_denylist = default_denylist()`, the CLI's denylist becomes a **superset** of today's (it gains the hard-floor backstop the server already has). This is a safe tightening and the only intended behavioral change. Everything else (sampling, turns, tokens, sandbox, skills, memory wiring) is identical because the values map 1:1.

## Error Handling

- `assemble_loop` never panics: the `compose_system_prompt` error path keeps today's "log + fall back to base prompt" behavior.
- Strictness on unknown active skills stays caller-side: server returns a wire error (`unknown active skill(s): …`); CLI prints `skills: …` and `exit(2)` — both driven by the returned `unknown_presets`.

## Testing

- **`agent-runtime-config`** (new home for the assembly tests):
  - `assemble_loop` with `cfg.memory = true` + injected memory tools registers them (`registered_names` contains `remember`); with `cfg.memory = false` it does not. (Moved from the server's `build_loop_registers_injected_memory_tools` / `build_loop_skips_memory_tools_when_disabled`.)
  - `assemble_loop` attaches the retriever only when `cfg.memory` is true.
  - `loop_config_from` maps a fully-populated `RuntimeConfig` to the expected `LoopConfig` (assert each field, incl. the three constants and the passed-through `stream_idle_timeout`).
  - unknown active skill → present in `BuiltLoop.unknown_presets`.
- **`agent-server`** — keep a smoke test that `build_loop` returns a usable loop and still maps unknown presets to an `apply` error (the existing `apply_*` tests already cover the wire-error path).
- **`agent-cli`** — a unit test that the flag→`RuntimeConfig` population is complete (e.g. given a `Cli` with sampling + memory + skills set, the derived `RuntimeConfig` carries them), so the CLI can't silently drop a field.
- **Regression bar:** full Rust workspace suite green; the live e2e (`e2e_auto_retrieval`, `e2e_sglang`, `e2e_parallel_tools`) still pass against the backend.

## Files Touched

- `agent/crates/agent-runtime-config/Cargo.toml` — add `agent-policy` dep.
- `agent/crates/agent-runtime-config/src/lib.rs` (or a new `src/assemble.rs` module) — `LoopParts`, `BuiltLoop`, `assemble_loop`, `loop_config_from` + tests.
- `agent/crates/agent-server/src/runtime.rs` — `build_loop` becomes a thin wrapper; move injection tests out.
- `agent/crates/agent-cli/src/main.rs` — populate `RuntimeConfig` fully; call `assemble_loop`; drop the hand-rolled `LoopConfig`.

## Rollout

Pure refactor; no migration, no config/wire/UI change. The only observable difference is the CLI gaining the hard-floor denylist backstop.
