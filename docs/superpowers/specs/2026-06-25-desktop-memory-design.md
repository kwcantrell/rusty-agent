# Desktop Memory Integration — Design Spec

**Date:** 2026-06-25
**Status:** Approved (design), pending spec review
**Scope:** Wire the existing memory subsystem (the `remember`/`recall`/`forget` tools **and** auto-retrieval) into the fully-local desktop app. Make it enabled by default with a Settings toggle.

## Problem

The memory subsystem is fully built and the CLI uses it, but the desktop app — now the primary product — has **no memory at all**: `agent-server/src/setup.rs` hardcodes an empty `memory_tools` vec and there is no retriever. So everything we shipped (memory tools + semantic auto-retrieval) is invisible to desktop users.

This spec activates that existing value in the desktop, in one well-scoped wiring change, with no new memory capability invented.

## Goals

- Desktop app gets the three memory tools and auto-retrieval, **enabled by default**.
- A Settings UI toggle (`memory`) turns it on/off, taking effect live on the next turn (the existing config-apply path).
- The expensive parts (embedding model + DB handle) are loaded **once** at bridge startup and shared across reconnects (no per-connection model reload).
- Memory stays correctly **project-scoped to the current workspace**, even after the user switches workspaces at runtime.
- Desktop and CLI share one store (`~/.agent/memory.db`), so the same project sees the same memories in both.
- Memory remains best-effort: a model-load failure (offline/first run) must never block the desktop from booting.

## Non-Goals

- Auto-ingestion (auto-writing memories from turns) — separate future spec.
- HTTP remote embeddings — deferred.
- Exposing DB path / embedding model / `k` / threshold in the desktop UI — defaults only (YAGNI).
- Any change to memory's internal behavior (store, embedder, scope, retrieval) — pure integration.

## Architecture

```
Bridge startup ── open_memory_parts(cfg) ──► MemoryParts { embedder, store, cfg }   (model load + DB open ONCE)
   (held on Bridge as Option<MemoryParts>, gated on effective memory flag; None on failure or off)
        │
        ▼  (per connection — cheap, no model reload)
   local_params(parts, current_workspace) ──► assemble_memory(parts, workspace):
        scope = project_scope(workspace);  tools = build_tools_with(...);  retriever = MemoryRetriever{...}
        ──► DaemonParams { memory_tools, memory_retriever, recall_token_budget }
        │
        ▼
   RuntimeState::{new,apply} ──► build_loop(cfg, ...):
        if cfg.memory { register the 3 tools; AgentLoop::with_retriever(retriever) }
        WindowContext::new(system).with_recall_budget(recall_token_budget)
        │
        ▼
   apply() rebuilds the loop on every Settings change ── toggle is live (parts pre-loaded at startup)
```

### Construction lifecycle (load once, scope per connection, gate live)

Three concerns, separated by cost and by what they depend on:

1. **Expensive, workspace-independent — load once at startup.** The embedding model and SQLite handle are loaded once when the bridge starts, gated on the effective (persisted-overlaid) `memory` flag. Held on the `Bridge` as `Option<MemoryParts>` (Arc-shared `embedder` + `store` + `cfg`). `None` if memory is off at launch or construction fails.
2. **Cheap, workspace-dependent — assemble per connection.** The bridge's accept loop already reads the *current* workspace per connection. From the shared `MemoryParts` plus that workspace it assembles the three tools and the retriever (just deriving the project scope + wrapping the Arcs — no model reload). This keeps memory correctly scoped after a `pick_workspace` switch.
3. **Live — gate in `build_loop`.** `RuntimeState::apply()` already rebuilds the loop on every config change, reusing the `memory_tools`/retriever from `DaemonParams`. `build_loop` registers/attaches them only when `cfg.memory` is true, so the toggle takes effect on the next turn.

### Live toggle semantics

- `build_loop` registers the three tools and calls `.with_retriever(...)` **only when `cfg.memory` is true**. Disabling via Settings drops both on the next turn; re-enabling restores them — because the parts were pre-loaded at startup and the per-connection assembly already produced the tools/retriever in `DaemonParams`.
- **Documented edge:** if `memory` is *persisted-off at launch*, `MemoryParts` is not loaded, so enabling it via Settings requires an app restart. Rare, since the default is on.

## Components & Changes

### 0. `agent-memory` — split load from assemble
- Add `pub struct MemoryParts { embedder: Arc<dyn Embedder>, store: Arc<dyn MemoryStore>, cfg: Arc<MemoryConfig> }` (clonable; the fields are already `Arc`).
- Add `pub fn open_memory_parts(cfg: MemoryConfig) -> Result<MemoryParts, MemoryInitError>` — does the expensive, workspace-independent work (open `SqliteStore`, construct the embedder), exactly as the front half of today's `build_tools_and_retriever`.
- Add `pub fn assemble_memory(parts: &MemoryParts, workspace: &Path) -> (Vec<Arc<dyn Tool>>, Arc<dyn agent_core::Retriever>)` — cheap: derive `project_scope(workspace)`, call `build_tools_with(...)`, and build `MemoryRetriever`. No model load.
- Reimplement `build_tools_and_retriever(cfg, workspace)` as `open_memory_parts(cfg)?` then `assemble_memory(&parts, workspace)` so the CLI path (fixed workspace) is unchanged.

### 1. `agent-runtime-config` — config field
- Add `RuntimeConfig.memory: bool` with `#[serde(default = "default_true")]`.
- `RuntimeConfig::from_launch(...)` leaves it at the serde default (true). Add to the params-overlay path (`apply_params`/settings merge) so the Settings UI can change it.

### 2. `agent-server` — thread the bundle
- `DaemonParams` (in `daemon.rs`) gains:
  - `memory_retriever: Option<Arc<dyn agent_core::Retriever>>`
  - `recall_token_budget: usize`
  (alongside the existing `memory_tools`).
- `RuntimeState` (in `runtime.rs`) stores `memory_retriever` and `recall_token_budget`; `new()` and `apply()` pass them to `build_loop`.
- `build_loop(...)` gains the two params and, **only when `cfg.memory`**, registers `memory_tools` and attaches the retriever via `AgentLoop::with_retriever`. When `cfg.memory` is false, it registers neither (tools stay unused).
- `daemon.rs::serve` builds the context with `WindowContext::new(Message::system(system_prompt)).with_recall_budget(params.recall_token_budget)`.

### 3. `agent-server::setup::local_params` — assemble per connection
- Change signature to accept `Option<&agent_memory::MemoryParts>` (plus the existing args). When `Some`, call `assemble_memory(parts, &workspace)` to get the tools + retriever, read `recall_token_budget` from `parts.cfg`, and place all three into `DaemonParams`. When `None`, fill empties + `None` (today's behavior). It no longer constructs the embedder/store itself.

### 4. `src-tauri` — load parts once, assemble per connection
- In `bridge.rs`, when the bridge is created, compute the effective `memory` flag (launch default overlaid with the persisted config at `config_path`); if on, call `agent_memory::open_memory_parts(MemoryConfig::default())` once and store the resulting `Option<MemoryParts>` on the `Bridge`. Construction failure → `None` (warn-logged), memory simply absent.
- The accept loop (which already reads the current workspace per connection) passes `bridge.memory_parts.as_ref()` into `local_params`, so each connection assembles tools/retriever scoped to the current workspace.
- `Cargo.toml`: `src-tauri` already depends on `agent-server`; add `agent-memory` (for `MemoryParts`/`open_memory_parts`) and `agent-core` (for the `Retriever` type). Confirm `agent-memory`'s `onnx` feature is enabled in the desktop build so the real embedder is present.

### 5. `web` — Settings toggle
- Add `memory: boolean` to the `RuntimeSettings` type (`web/src/wire.ts`).
- Add a checkbox in `SettingsPanel.tsx` mirroring the existing `preserve_thinking` control (label e.g. "Long-term memory (remember/recall across sessions)").
- The server already round-trips `RuntimeConfig` ↔ settings frames; the new bool flows through the existing serialization.

## Error Handling / Degradation

- `open_memory_parts` returns `Err(MemoryInitError)` on any construction failure (model download failure, DB unopenable). The bridge maps `Err` to `None` (warn-logged) and stores no parts; the desktop boots normally with memory absent — `local_params` sees `None` and fills empties.
- When `cfg.memory` is true but parts are `None` (build failed), every connection assembles empty tools + `None` retriever, so `build_loop` registers nothing — identical to the disabled path. The Settings toggle still shows on, but memory is inert until the underlying issue (e.g. network for first-run model download) is resolved and the app restarts.

## Testing

**`agent-memory`:**
- `assemble_memory` from a `MemoryParts` built with `StubEmbedder` + `InMemoryStore` returns the three named tools and a working retriever scoped to the given workspace (two different workspaces → different project scopes). Pure, no model load.
- `open_memory_parts` (real model) — covered by the existing `#[ignore]` model-download tests; add one asserting it returns parts whose `assemble_memory` recalls a seeded memory (mirror the existing real-embedder test).

**`agent-runtime-config`:**
- `RuntimeConfig` default has `memory == true`.
- A settings frame with `memory: false` overlays to `memory == false` (round-trip through the params/apply path).

**`agent-server` (`runtime.rs`):**
- `build_loop` with `cfg.memory = true` and a non-empty `memory_tools` + `Some(retriever)` registers the tools (assert tool names present, as the existing `build_loop_registers_injected_memory_tools` test does) — extend/mirror that test.
- `build_loop` with `cfg.memory = false` registers **no** memory tools even when `memory_tools` is non-empty (new test).
- A fake `Retriever` is attached only when `cfg.memory` is true (assert via behavior or a visible registration signal consistent with existing test style).

**`agent-server` (`setup.rs`):**
- `local_params` with `None` parts yields empty `memory_tools` + `None` retriever (update `local_params_seeds_llama_defaults` to pass `None`).
- `local_params` with `Some(parts)` (Stub-backed) populates `memory_tools` (3) + `Some(retriever)` + the budget in `DaemonParams` (new test).

**`web`:**
- `SettingsPanel` renders the memory checkbox and emits `memory` in the saved `RuntimeSettings` (mirror an existing `preserve_thinking` test if present; otherwise a minimal render+save test).

**Manual smoke (desktop):**
- Launch the desktop app; confirm memory tools are available and a fact remembered in one session is auto-recalled in a later session.
- Toggle memory off in Settings → confirm recall/tools stop on the next turn; toggle on → confirm they return.

## Files Touched

- `agent/crates/agent-memory/src/lib.rs` — `MemoryParts`, `open_memory_parts`, `assemble_memory`; reimplement `build_tools_and_retriever` on top.
- `agent/crates/agent-runtime-config/src/runtime_config.rs` — `memory` field + overlay.
- `agent/crates/agent-server/src/daemon.rs` — `DaemonParams` fields; `WindowContext::with_recall_budget`.
- `agent/crates/agent-server/src/runtime.rs` — `RuntimeState` fields; `build_loop` gating + retriever attach.
- `agent/crates/agent-server/src/setup.rs` — `local_params` assembles from `Option<&MemoryParts>`.
- `src-tauri/src/bridge.rs` (+ `Cargo.toml`) — load `MemoryParts` once at startup, hold on `Bridge`, assemble per connection.
- `web/src/wire.ts`, `web/src/components/SettingsPanel.tsx` — `memory` toggle.

## Rollout

No migration. Default-on means existing desktop installs gain memory on next launch (one-time ~128MB model download, best-effort). Users can disable it in Settings. CLI behavior is unchanged; both now share `~/.agent/memory.db`.
