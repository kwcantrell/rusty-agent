# Settings Capability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a paired browser reconfigure the running local daemon (model/inference, command policy, loop tuning) live, without restarting the daemon or dropping the WebSocket.

**Architecture:** A new `RuntimeConfig` (in the shared `agent-runtime-config` crate) is the editable surface + on-disk persistence. The daemon stops holding one immutable `Arc<AgentLoop>` and instead holds a `RuntimeState` with a `Mutex<Arc<AgentLoop>>` it atomically rebuilds-and-swaps from the existing seams on each settings change. New additive `settings_*` wire frames flow browser↔daemon through the transparent Durable Object relay (zero `cloud/` changes). A `SettingsPanel` in `web/` reads/writes them.

**Tech Stack:** Rust (tokio, serde, std::sync::Mutex), the existing `agent-core`/`agent-policy`/`agent-model` seams, React + Vite + TypeScript + Tailwind v4, Vitest + React Testing Library.

**Spec:** [`../specs/2026-06-23-settings-capability-design.md`](../specs/2026-06-23-settings-capability-design.md)

## Global Constraints

- **cargo is NOT on PATH** — run `source "$HOME/.cargo/env"` before any cargo command. Build/test from `agent/`.
- **Rust gate:** `cargo test --workspace` green AND `cargo clippy --all-targets -- -D warnings` clean.
- **Zero `cloud/` changes** — the `AgentSession` DO relays new frame kinds transparently; do not touch `cloud/`.
- **Core crates untouched** — no edits to `agent-core`, `agent-model`, `agent-tools`, `agent-policy`. All work lands in `agent-runtime-config`, `agent-server`, and `web/`.
- **Protocol version stays `1`** — additions are purely additive; older browsers simply never send the new kinds.
- **Secrets never on the wire** — the API key stays `AGENT_API_KEY`/launch-only; `settings_*` frames carry only an `api_key_set: bool` indicator.
- **Hard-floor denylist always enforced** — `rm -rf /`, `sudo`, `:(){`, `mkfs`, `dd if=` are intersected into the effective denylist regardless of user settings.
- **Web gate:** `cd web && npx vitest run` green.

---

### Task 1: `RuntimeConfig` data model, validation, and hard floor

**Files:**
- Create: `agent/crates/agent-runtime-config/src/runtime_config.rs`
- Modify: `agent/crates/agent-runtime-config/src/lib.rs` (add `mod runtime_config;` + re-export)
- Test: inline `#[cfg(test)]` in `runtime_config.rs`

**Interfaces:**
- Consumes: existing `default_allowlist()`, `backend_name_is_valid()`, `protocol_name_is_valid()` from `lib.rs`.
- Produces:
  - `pub struct RuntimeConfig { backend: String, base_url: String, model: String, protocol: String, command_allowlist: Vec<String>, command_denylist: Vec<String>, temperature: f32, max_tokens: u32, max_turns: usize, context_limit: usize }` — derives `Debug, Clone, PartialEq, Serialize, Deserialize`.
  - `RuntimeConfig::from_launch(backend, base_url, model, protocol, context_limit) -> RuntimeConfig`
  - `RuntimeConfig::normalized(self) -> RuntimeConfig`
  - `RuntimeConfig::validate(&self) -> Result<(), String>`
  - `RuntimeConfig::effective_denylist(&self) -> Vec<String>`
  - `pub const HARD_FLOOR_DENYLIST: &[&str]`

- [ ] **Step 1: Add the module to `lib.rs`**

At the top of `agent/crates/agent-runtime-config/src/lib.rs`, under the `//!` doc comment, add:

```rust
mod runtime_config;
pub use runtime_config::{RuntimeConfig, HARD_FLOOR_DENYLIST};
```

Add `use serde::{Deserialize, Serialize};` is NOT needed in lib.rs (it stays in the new file).

- [ ] **Step 2: Write the failing tests**

Create `agent/crates/agent-runtime-config/src/runtime_config.rs` with ONLY the test module first (the types come in Step 4 — the file will not compile yet, which is the point):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> RuntimeConfig {
        RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192)
    }

    #[test]
    fn from_launch_seeds_defaults() {
        let c = base();
        assert_eq!(c.temperature, 0.2);
        assert_eq!(c.max_tokens, 2048);
        assert_eq!(c.max_turns, 25);
        assert!(c.command_denylist.is_empty()); // floor is added by effective_denylist, not stored
        assert!(!c.command_allowlist.is_empty());
    }

    #[test]
    fn validate_accepts_a_good_config() {
        assert!(base().validate().is_ok());
    }

    #[test]
    fn validate_rejects_unknown_backend_and_protocol() {
        let mut c = base();
        c.backend = "bogus".into();
        assert!(c.validate().unwrap_err().contains("backend"));
        let mut c = base();
        c.protocol = "bogus".into();
        assert!(c.validate().unwrap_err().contains("protocol"));
    }

    #[test]
    fn validate_requires_base_url_for_openai() {
        let mut c = base();
        c.base_url = "  ".into();
        assert!(c.validate().unwrap_err().contains("base_url"));
    }

    #[test]
    fn validate_enforces_numeric_bounds() {
        let mut c = base();
        c.temperature = 3.0;
        assert!(c.validate().is_err());
        let mut c = base();
        c.max_tokens = 0;
        assert!(c.validate().is_err());
        let mut c = base();
        c.max_turns = 0;
        assert!(c.validate().is_err());
        let mut c = base();
        c.context_limit = 16;
        assert!(c.validate().is_err());
    }

    #[test]
    fn claude_cli_normalizes_protocol_to_prompted() {
        let mut c = base();
        c.backend = "claude-cli".into();
        c.protocol = "native".into();
        let n = c.normalized();
        assert_eq!(n.protocol, "prompted");
        assert!(n.validate().is_ok());
    }

    #[test]
    fn effective_denylist_always_contains_the_hard_floor() {
        let mut c = base();
        c.command_denylist = vec!["custom-bad".into()];
        let eff = c.effective_denylist();
        for floor in HARD_FLOOR_DENYLIST {
            assert!(eff.iter().any(|d| d == floor), "missing floor {floor}");
        }
        assert!(eff.iter().any(|d| d == "custom-bad"));
    }

    #[test]
    fn effective_denylist_floor_survives_empty_user_list() {
        let c = base(); // command_denylist empty
        assert!(c.effective_denylist().iter().any(|d| d == "sudo"));
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail (compile error)**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-runtime-config runtime_config`
Expected: FAIL — `cannot find type RuntimeConfig` / `HARD_FLOOR_DENYLIST`.

- [ ] **Step 4: Write the implementation**

At the TOP of `agent/crates/agent-runtime-config/src/runtime_config.rs` (above the test module), add:

```rust
use crate::{default_allowlist, backend_name_is_valid, protocol_name_is_valid};
use serde::{Deserialize, Serialize};

/// Commands ALWAYS denied regardless of user settings — defense-in-depth against
/// the model (or an injected settings frame), not against the operator. Intersected
/// into the effective denylist by `RuntimeConfig::effective_denylist`.
pub const HARD_FLOOR_DENYLIST: &[&str] = &["rm -rf /", "sudo", ":(){", "mkfs", "dd if="];

/// The editable runtime surface, persisted to disk and mirrored on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub backend: String,        // "openai" | "claude-cli"
    pub base_url: String,
    pub model: String,
    pub protocol: String,       // "native" | "prompted"
    pub command_allowlist: Vec<String>,
    pub command_denylist: Vec<String>, // user-editable portion ONLY (floor added separately)
    pub temperature: f32,
    pub max_tokens: u32,
    pub max_turns: usize,
    pub context_limit: usize,
}

impl RuntimeConfig {
    /// Seed a config from launch flags + sensible defaults for the rest.
    pub fn from_launch(
        backend: String, base_url: String, model: String, protocol: String, context_limit: usize,
    ) -> Self {
        Self {
            backend, base_url, model, protocol,
            command_allowlist: default_allowlist(),
            command_denylist: Vec::new(),
            temperature: 0.2,
            max_tokens: 2048,
            max_turns: 25,
            context_limit,
        }
    }

    /// Apply invariants that are corrections rather than errors (claude-cli is prompted-only).
    pub fn normalized(mut self) -> Self {
        if self.backend == "claude-cli" {
            self.protocol = "prompted".into();
        }
        self
    }

    /// Reject structurally invalid configs. Call `normalized()` first so the
    /// claude-cli protocol correction is not flagged as an error.
    pub fn validate(&self) -> Result<(), String> {
        if !backend_name_is_valid(&self.backend) {
            return Err(format!("unknown backend '{}': use openai | claude-cli", self.backend));
        }
        if !protocol_name_is_valid(&self.protocol) {
            return Err(format!("unknown protocol '{}': use native | prompted", self.protocol));
        }
        if self.backend == "openai" && self.base_url.trim().is_empty() {
            return Err("base_url is required for the openai backend".into());
        }
        if !(0.0..=2.0).contains(&self.temperature) {
            return Err("temperature must be between 0.0 and 2.0".into());
        }
        if self.max_tokens == 0 {
            return Err("max_tokens must be > 0".into());
        }
        if self.max_turns == 0 {
            return Err("max_turns must be >= 1".into());
        }
        if self.context_limit < 1024 {
            return Err("context_limit must be >= 1024".into());
        }
        Ok(())
    }

    /// The denylist actually handed to `RulePolicy`: the immutable hard floor unioned
    /// with the user's editable entries (deduped, floor first).
    pub fn effective_denylist(&self) -> Vec<String> {
        let mut out: Vec<String> = HARD_FLOOR_DENYLIST.iter().map(|s| s.to_string()).collect();
        for d in &self.command_denylist {
            if !out.contains(d) {
                out.push(d.clone());
            }
        }
        out
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-runtime-config runtime_config`
Expected: PASS (8 tests).

- [ ] **Step 6: Commit**

```bash
cd agent && source "$HOME/.cargo/env" && cargo clippy -p agent-runtime-config --all-targets -- -D warnings
cd .. && git add agent/crates/agent-runtime-config && git commit -m "feat(runtime-config): RuntimeConfig model, validation, hard-floor denylist"
```

---

### Task 2: `RuntimeConfig` persistence (file wins per-field over flag base)

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs`
- Modify: `agent/crates/agent-runtime-config/Cargo.toml` (add `tempfile` dev-dep)
- Test: inline `#[cfg(test)]` in `runtime_config.rs`

**Interfaces:**
- Consumes: `RuntimeConfig` (Task 1).
- Produces:
  - `RuntimeConfig::save(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>>`
  - `RuntimeConfig::load_over(base: RuntimeConfig, path: &std::path::Path) -> RuntimeConfig` — start from `base` (flag-derived), then merge a partial file over it per-field (file wins; missing/malformed → keep base).

- [ ] **Step 1: Add the `tempfile` dev-dependency**

In `agent/crates/agent-runtime-config/Cargo.toml`, add a dev-dependencies section (after `[dependencies]`):

```toml
[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 2: Write the failing tests**

Append to the `mod tests` block in `runtime_config.rs`:

```rust
    #[test]
    fn save_then_load_over_round_trips_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.json");
        let mut c = base();
        c.model = "saved-model".into();
        c.temperature = 0.7;
        c.command_denylist = vec!["nope".into()];
        c.save(&path).unwrap();

        // A different base proves the file wins.
        let other = RuntimeConfig::from_launch(
            "openai".into(), "http://x".into(), "flag-model".into(), "native".into(), 4096);
        let loaded = RuntimeConfig::load_over(other, &path);
        assert_eq!(loaded.model, "saved-model");
        assert_eq!(loaded.temperature, 0.7);
        assert_eq!(loaded.command_denylist, vec!["nope".to_string()]);
    }

    #[test]
    fn load_over_returns_base_when_file_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.json");
        let b = base();
        assert_eq!(RuntimeConfig::load_over(b.clone(), &path), b);
    }

    #[test]
    fn load_over_falls_back_per_field_for_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.json");
        std::fs::write(&path, r#"{"model":"only-model"}"#).unwrap();
        let b = base();
        let loaded = RuntimeConfig::load_over(b.clone(), &path);
        assert_eq!(loaded.model, "only-model"); // file wins
        assert_eq!(loaded.backend, b.backend);   // absent field falls back to base
        assert_eq!(loaded.max_tokens, b.max_tokens);
    }

    #[test]
    fn load_over_ignores_a_malformed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        let b = base();
        assert_eq!(RuntimeConfig::load_over(b.clone(), &path), b);
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-runtime-config runtime_config`
Expected: FAIL — `no method named save` / `load_over`.

- [ ] **Step 4: Write the implementation**

In `runtime_config.rs`, add the `Path` import to the existing `use` block:

```rust
use std::path::Path;
```

Add a partial struct above `impl RuntimeConfig` (after the `RuntimeConfig` struct definition):

```rust
/// All-optional mirror used only for on-disk merge: a file written by an older
/// build is missing newer fields, which then fall back to the flag-derived base.
#[derive(Debug, Default, Deserialize)]
struct PartialRuntimeConfig {
    backend: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    protocol: Option<String>,
    command_allowlist: Option<Vec<String>>,
    command_denylist: Option<Vec<String>>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    max_turns: Option<usize>,
    context_limit: Option<usize>,
}
```

Add these methods to `impl RuntimeConfig`:

```rust
    fn merge(mut self, p: PartialRuntimeConfig) -> Self {
        if let Some(v) = p.backend { self.backend = v; }
        if let Some(v) = p.base_url { self.base_url = v; }
        if let Some(v) = p.model { self.model = v; }
        if let Some(v) = p.protocol { self.protocol = v; }
        if let Some(v) = p.command_allowlist { self.command_allowlist = v; }
        if let Some(v) = p.command_denylist { self.command_denylist = v; }
        if let Some(v) = p.temperature { self.temperature = v; }
        if let Some(v) = p.max_tokens { self.max_tokens = v; }
        if let Some(v) = p.max_turns { self.max_turns = v; }
        if let Some(v) = p.context_limit { self.context_limit = v; }
        self
    }

    /// Persist the full config (pretty JSON).
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Flag-derived `base`, overlaid per-field by the file at `path` if it parses;
    /// a missing or malformed file leaves `base` unchanged.
    pub fn load_over(base: RuntimeConfig, path: &Path) -> RuntimeConfig {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<PartialRuntimeConfig>(&text) {
                Ok(p) => base.merge(p),
                Err(_) => base,
            },
            Err(_) => base,
        }
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-runtime-config`
Expected: PASS (all Task 1 + Task 2 tests).

- [ ] **Step 6: Commit**

```bash
cd agent && source "$HOME/.cargo/env" && cargo clippy -p agent-runtime-config --all-targets -- -D warnings
cd .. && git add agent/crates/agent-runtime-config && git commit -m "feat(runtime-config): persist + load_over (file wins per-field over flag base)"
```

---

### Task 3: Wire frames for settings (`agent-server`)

**Files:**
- Modify: `agent/crates/agent-server/src/wire.rs`
- Test: inline `#[cfg(test)]` in `wire.rs`

**Interfaces:**
- Consumes: `RuntimeConfig` from `agent-runtime-config` (Task 1).
- Produces four new `WireBody` variants (snake_case tags via the existing `#[serde(tag = "kind", rename_all = "snake_case")]`):
  - `SettingsGet` → `"settings_get"`
  - `SettingsUpdate { settings: RuntimeConfig }` → `"settings_update"`
  - `SettingsState { settings: RuntimeConfig, workspace: String, api_key_set: bool, hard_floor: Vec<String> }` → `"settings_state"`
  - `SettingsError { message: String }` → `"settings_error"`

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block in `wire.rs`:

```rust
    #[test]
    fn settings_get_round_trips() {
        let env = WireEnvelope {
            v: PROTOCOL_VERSION, session_id: "s".into(), id: None,
            body: WireBody::SettingsGet,
        };
        let json = serde_json::to_string(&env).unwrap();
        assert!(json.contains("\"kind\":\"settings_get\""));
        let back: WireEnvelope = serde_json::from_str(&json).unwrap();
        assert!(matches!(back.body, WireBody::SettingsGet));
    }

    #[test]
    fn settings_update_carries_a_config() {
        use agent_runtime_config::RuntimeConfig;
        let cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://x".into(), "m".into(), "native".into(), 8192);
        let env = WireEnvelope {
            v: PROTOCOL_VERSION, session_id: "s".into(), id: None,
            body: WireBody::SettingsUpdate { settings: cfg.clone() },
        };
        let json = serde_json::to_string(&env).unwrap();
        assert!(json.contains("\"kind\":\"settings_update\""));
        let back: WireEnvelope = serde_json::from_str(&json).unwrap();
        match back.body {
            WireBody::SettingsUpdate { settings } => assert_eq!(settings, cfg),
            _ => panic!("wrong body"),
        }
    }

    #[test]
    fn settings_state_and_error_serialize() {
        use agent_runtime_config::RuntimeConfig;
        let cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://x".into(), "m".into(), "native".into(), 8192);
        let state = WireEnvelope {
            v: PROTOCOL_VERSION, session_id: "s".into(), id: None,
            body: WireBody::SettingsState {
                settings: cfg, workspace: "/w".into(), api_key_set: true,
                hard_floor: vec!["sudo".into()] },
        };
        let j = serde_json::to_string(&state).unwrap();
        assert!(j.contains("\"kind\":\"settings_state\""));
        assert!(j.contains("\"api_key_set\":true"));

        let err = WireEnvelope {
            v: PROTOCOL_VERSION, session_id: "s".into(), id: None,
            body: WireBody::SettingsError { message: "bad".into() },
        };
        let j = serde_json::to_string(&err).unwrap();
        assert!(j.contains("\"kind\":\"settings_error\""));
        let back: WireEnvelope = serde_json::from_str(&j).unwrap();
        assert!(matches!(back.body, WireBody::SettingsError { .. }));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-server wire`
Expected: FAIL — `no variant SettingsGet`.

- [ ] **Step 3: Write the implementation**

In `wire.rs`, add the import at the top (after the existing `use agent_tools::Display;`):

```rust
use agent_runtime_config::RuntimeConfig;
```

Add these variants to the `WireBody` enum (after `ApprovalResponse { decision: WireDecision }`):

```rust
    SettingsGet,
    SettingsUpdate {
        settings: RuntimeConfig,
    },
    SettingsState {
        settings: RuntimeConfig,
        workspace: String,
        api_key_set: bool,
        hard_floor: Vec<String>,
    },
    SettingsError {
        message: String,
    },
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-server wire`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cd agent && source "$HOME/.cargo/env" && cargo clippy -p agent-server --all-targets -- -D warnings
cd .. && git add agent/crates/agent-server/src/wire.rs && git commit -m "feat(agent-server): additive settings_* wire frames"
```

---

### Task 4: `RuntimeState` — build, swap, and handle settings

**Files:**
- Create: `agent/crates/agent-server/src/runtime.rs`
- Modify: `agent/crates/agent-server/src/lib.rs` (add `pub mod runtime;`)
- Test: inline `#[cfg(test)]` in `runtime.rs`

**Interfaces:**
- Consumes: `RuntimeConfig`/`HARD_FLOOR_DENYLIST` (Task 1), `WsEventSink` (`sink.rs`), `WsApprovalChannel` (`approval.rs`), `WireBody`/`WireEnvelope`/`PROTOCOL_VERSION` (Task 3), `build_model`/`build_registry`/`pick_protocol`.
- Produces:
  - `pub struct RuntimeState`
  - `RuntimeState::new(config, sink, approval, workspace, api_key, claude_binary, config_path, session, tx) -> RuntimeState` (normalizes `config` and builds the initial loop)
  - `RuntimeState::current_loop(&self) -> Arc<AgentLoop>`
  - `RuntimeState::apply(&self, incoming: RuntimeConfig) -> Result<(), String>`
  - `RuntimeState::handle(&self, body: &WireBody) -> bool` (true if it was a settings frame; sends `settings_state`/`settings_error`)

- [ ] **Step 1: Register the module in `lib.rs`**

In `agent/crates/agent-server/src/lib.rs`, add (next to the other `pub mod` lines):

```rust
pub mod runtime;
```

- [ ] **Step 2: Write the failing tests**

Create `agent/crates/agent-server/src/runtime.rs` with the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::WsApprovalChannel;
    use crate::sink::WsEventSink;
    use crate::wire::WireBody;
    use agent_runtime_config::RuntimeConfig;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::sync::mpsc;

    fn make() -> (RuntimeState, mpsc::UnboundedReceiver<crate::wire::WireEnvelope>, tempfile::TempDir) {
        let (tx, rx) = mpsc::unbounded_channel();
        let session = Arc::new(Mutex::new(String::new()));
        let sink = Arc::new(WsEventSink::new(tx.clone(), session.clone()));
        let approval = Arc::new(WsApprovalChannel::new(tx.clone(), session.clone(), Duration::from_secs(1)));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.json");
        let cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
        let rs = RuntimeState::new(cfg, sink, approval, dir.path().to_path_buf(), None,
            "claude".into(), path, session, tx);
        (rs, rx, dir)
    }

    #[test]
    fn apply_swaps_the_loop_and_persists() {
        let (rs, _rx, dir) = make();
        let before = rs.current_loop();
        let mut next = RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(), "m2".into(), "native".into(), 8192);
        next.temperature = 0.9;
        rs.apply(next).unwrap();
        let after = rs.current_loop();
        assert!(!Arc::ptr_eq(&before, &after), "loop should be a new Arc");
        assert!(dir.path().join("rt.json").exists(), "config persisted");
    }

    #[test]
    fn apply_rejects_invalid_without_swapping() {
        let (rs, _rx, _dir) = make();
        let before = rs.current_loop();
        let mut bad = RuntimeConfig::from_launch(
            "openai".into(), "   ".into(), "m".into(), "native".into(), 8192); // empty base_url
        bad.base_url = "  ".into();
        let err = rs.apply(bad).unwrap_err();
        assert!(err.contains("base_url"));
        assert!(Arc::ptr_eq(&before, &rs.current_loop()), "loop unchanged on rejection");
    }

    #[test]
    fn handle_settings_get_emits_state() {
        let (rs, mut rx, _dir) = make();
        assert!(rs.handle(&WireBody::SettingsGet));
        let env = rx.try_recv().expect("a frame");
        match env.body {
            WireBody::SettingsState { api_key_set, hard_floor, .. } => {
                assert!(!api_key_set);
                assert!(hard_floor.iter().any(|d| d == "sudo"));
            }
            _ => panic!("expected settings_state"),
        }
    }

    #[test]
    fn handle_invalid_update_emits_error() {
        let (rs, mut rx, _dir) = make();
        let mut bad = RuntimeConfig::from_launch(
            "openai".into(), "".into(), "m".into(), "native".into(), 8192);
        bad.base_url = "".into();
        assert!(rs.handle(&WireBody::SettingsUpdate { settings: bad }));
        let env = rx.try_recv().expect("a frame");
        assert!(matches!(env.body, WireBody::SettingsError { .. }));
    }

    #[test]
    fn handle_ignores_non_settings_frames() {
        let (rs, _rx, _dir) = make();
        assert!(!rs.handle(&WireBody::UserInput { text: "hi".into() }));
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-server runtime`
Expected: FAIL — `cannot find type RuntimeState`.

- [ ] **Step 4: Write the implementation**

At the TOP of `runtime.rs` (above the test module):

```rust
use crate::approval::WsApprovalChannel;
use crate::sink::WsEventSink;
use crate::wire::{WireBody, WireEnvelope, PROTOCOL_VERSION};
use agent_core::{AgentLoop, LoopConfig, DEFAULT_STREAM_IDLE_TIMEOUT};
use agent_policy::RulePolicy;
use agent_runtime_config::{build_model, build_registry, pick_protocol, RuntimeConfig, HARD_FLOOR_DENYLIST};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// Holds the live `AgentLoop` plus everything needed to rebuild it on a settings
/// change. The loop is swapped atomically; an in-flight run keeps the `Arc` it
/// already cloned, so it finishes on its old config (next-turn apply, no interrupt).
pub struct RuntimeState {
    loop_cell: Mutex<Arc<AgentLoop>>,
    config: Mutex<RuntimeConfig>,
    sink: Arc<WsEventSink>,
    approval: Arc<WsApprovalChannel>,
    workspace: PathBuf,
    api_key: Option<String>,
    claude_binary: String,
    config_path: PathBuf,
    session: Arc<Mutex<String>>,
    tx: mpsc::UnboundedSender<WireEnvelope>,
}

impl RuntimeState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: RuntimeConfig,
        sink: Arc<WsEventSink>,
        approval: Arc<WsApprovalChannel>,
        workspace: PathBuf,
        api_key: Option<String>,
        claude_binary: String,
        config_path: PathBuf,
        session: Arc<Mutex<String>>,
        tx: mpsc::UnboundedSender<WireEnvelope>,
    ) -> Self {
        let config = config.normalized();
        let initial = build_loop(&config, &sink, &approval, &workspace, &api_key, &claude_binary);
        Self {
            loop_cell: Mutex::new(initial),
            config: Mutex::new(config),
            sink, approval, workspace, api_key, claude_binary, config_path, session, tx,
        }
    }

    /// Clone the current loop `Arc` (lock held only for the clone, never across await).
    pub fn current_loop(&self) -> Arc<AgentLoop> {
        self.loop_cell.lock().unwrap().clone()
    }

    /// Validate+normalize, persist, then swap. On any failure, nothing changes.
    pub fn apply(&self, incoming: RuntimeConfig) -> Result<(), String> {
        let cfg = incoming.normalized();
        cfg.validate()?;
        cfg.save(&self.config_path).map_err(|e| format!("persist failed: {e}"))?;
        let new_loop = build_loop(
            &cfg, &self.sink, &self.approval, &self.workspace, &self.api_key, &self.claude_binary);
        *self.loop_cell.lock().unwrap() = new_loop;
        *self.config.lock().unwrap() = cfg;
        Ok(())
    }

    fn state_body(&self) -> WireBody {
        WireBody::SettingsState {
            settings: self.config.lock().unwrap().clone(),
            workspace: self.workspace.display().to_string(),
            api_key_set: self.api_key.is_some(),
            hard_floor: HARD_FLOOR_DENYLIST.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn send(&self, body: WireBody) {
        let env = WireEnvelope {
            v: PROTOCOL_VERSION,
            session_id: self.session.lock().unwrap().clone(),
            id: None,
            body,
        };
        let _ = self.tx.send(env);
    }

    /// Dispatch a settings_* frame. Returns true if it was handled (a settings frame).
    pub fn handle(&self, body: &WireBody) -> bool {
        match body {
            WireBody::SettingsGet => {
                let s = self.state_body();
                self.send(s);
                true
            }
            WireBody::SettingsUpdate { settings } => {
                match self.apply(settings.clone()) {
                    Ok(()) => {
                        let s = self.state_body();
                        self.send(s);
                    }
                    Err(message) => self.send(WireBody::SettingsError { message }),
                }
                true
            }
            _ => false,
        }
    }
}

/// Assemble an `AgentLoop` from a config + the persistent seams. The one place that
/// turns a `RuntimeConfig` into a loop (initial build + every reconfigure).
fn build_loop(
    cfg: &RuntimeConfig,
    sink: &Arc<WsEventSink>,
    approval: &Arc<WsApprovalChannel>,
    workspace: &PathBuf,
    api_key: &Option<String>,
    claude_binary: &str,
) -> Arc<AgentLoop> {
    let model = build_model(&cfg.backend, &cfg.base_url, &cfg.model, claude_binary, api_key.clone());
    let policy = Arc::new(RulePolicy {
        workspace: workspace.clone(),
        command_allowlist: cfg.command_allowlist.clone(),
        command_denylist: cfg.effective_denylist(),
    });
    Arc::new(AgentLoop::new(
        model,
        pick_protocol(&cfg.protocol),
        Arc::new(build_registry()),
        policy,
        approval.clone(),
        sink.clone(),
        LoopConfig {
            model_limit: cfg.context_limit,
            max_turns: cfg.max_turns,
            max_retries: 3,
            temperature: cfg.temperature,
            max_tokens: Some(cfg.max_tokens),
            workspace: workspace.clone(),
            tool_timeout: Duration::from_secs(120),
            stream_idle_timeout: DEFAULT_STREAM_IDLE_TIMEOUT,
        },
    ))
}
```

Note: `Arc<WsEventSink>` / `Arc<WsApprovalChannel>` coerce to `Arc<dyn EventSink>` / `Arc<dyn ApprovalChannel>` at the `AgentLoop::new` call site automatically.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-server runtime`
Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
cd agent && source "$HOME/.cargo/env" && cargo clippy -p agent-server --all-targets -- -D warnings
cd .. && git add agent/crates/agent-server/src/lib.rs agent/crates/agent-server/src/runtime.rs && git commit -m "feat(agent-server): RuntimeState — atomic loop rebuild/swap + settings handler"
```

---

### Task 5: Wire `RuntimeState` into the daemon read loop + CLI

**Files:**
- Modify: `agent/crates/agent-server/src/daemon.rs`
- Modify: `agent/crates/agent-server/src/main.rs`
- Test: regression — whole-workspace test + clippy (logic is covered by Task 4's unit tests)

**Interfaces:**
- Consumes: `RuntimeState` (Task 4), `RuntimeConfig` (Task 1).
- Produces: a `DaemonParams` whose fields are `{ ws_url: String, agent_token: String, config: RuntimeConfig, api_key: Option<String>, claude_binary: String, config_path: PathBuf, workspace: PathBuf }`.

- [ ] **Step 1: Rewrite `daemon.rs`**

Replace the entire contents of `agent/crates/agent-server/src/daemon.rs` with:

```rust
use crate::approval::WsApprovalChannel;
use crate::runtime::RuntimeState;
use crate::sink::WsEventSink;
use crate::wire::{WireBody, WireEnvelope};
use agent_core::WindowContext;
use agent_model::Message;
use agent_runtime_config::RuntimeConfig;
use futures::{SinkExt, StreamExt};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WsMessage;

type DynErr = Box<dyn std::error::Error + Send + Sync>;

pub struct DaemonParams {
    pub ws_url: String, // ws://host/agent
    pub agent_token: String,
    pub config: RuntimeConfig, // flag-derived base; the file at config_path overlays it
    pub api_key: Option<String>,
    pub claude_binary: String,
    pub config_path: PathBuf,
    pub workspace: PathBuf,
}

const SYSTEM_PROMPT: &str = "You are a local coding agent. Use the provided tools to inspect \
and modify the workspace. Think step by step. When the task is complete, reply with a summary \
and no tool call.";

pub async fn run(params: DaemonParams) -> Result<(), DynErr> {
    // Shared session id (MVP: one active session per agent). The read loop sets it
    // on each user_input; the sink, approval channel, and settings replies stamp it.
    let session = Arc::new(Mutex::new(String::new()));
    let (tx, mut rx) = mpsc::unbounded_channel::<WireEnvelope>();

    let sink = Arc::new(WsEventSink::new(tx.clone(), session.clone()));
    let approval = Arc::new(WsApprovalChannel::new(tx.clone(), session.clone(),
        Duration::from_secs(300)));

    // Live settings survive reconnect/restart: overlay the persisted file on the flag base.
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
    ));
    let ctx = Arc::new(tokio::sync::Mutex::new(
        WindowContext::new(Message::system(SYSTEM_PROMPT))));

    let mut req = params.ws_url.clone().into_client_request()?;
    req.headers_mut().insert("Authorization",
        format!("Bearer {}", params.agent_token).parse()?);
    let (ws, _resp) = tokio_tungstenite::connect_async(req).await?;
    let (mut write, mut read) = ws.split();

    // Writer task: drain the channel to the socket; ping periodically to stay alive.
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
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    let mut guard = ctx.lock().await;
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
            other => {
                // settings_get / settings_update handled here; anything else is ignored.
                *session.lock().unwrap() = env.session_id.clone();
                runtime.handle(&other);
            }
        }
    }
    writer.abort();
    Ok(())
}
```

- [ ] **Step 2: Update `main.rs`**

In `agent/crates/agent-server/src/main.rs`, change the `Run` variant to add the `--runtime-config` flag (add this field inside the `Run { ... }` struct variant):

```rust
        /// Path to the persisted runtime config (live settings). Seeded from the flags
        /// above; overlaid by this file if present.
        #[arg(long, default_value = "agent-runtime.json")]
        runtime_config: PathBuf,
```

Replace the body of the `Cmd::Run { .. }` match arm with:

```rust
        Cmd::Run { base_url, model, protocol, workspace, context_limit, backend, claude_binary,
                   runtime_config } => {
            let cfg = DaemonConfig::load(&cli.config)
                .expect("load config (run `enroll` first)");
            println!("pairing code: {}", cfg.pairing_code);
            let workspace = std::fs::canonicalize(&workspace)
                .unwrap_or_else(|_| PathBuf::from(&workspace));
            if !backend_name_is_valid(&backend) {
                eprintln!("unknown --backend '{backend}': use openai | claude-cli");
                std::process::exit(2);
            }
            let api_key = std::env::var("AGENT_API_KEY").ok();
            let base = RuntimeConfig::from_launch(backend, base_url, model, protocol, context_limit);
            // Surface bad flags early (the persisted file is only ever written post-validation).
            if let Err(e) = base.clone().normalized().validate() {
                eprintln!("invalid launch config: {e}");
                std::process::exit(2);
            }
            let params = daemon::DaemonParams {
                ws_url: ws_url(&cfg.worker_url),
                agent_token: cfg.agent_token,
                config: base,
                api_key,
                claude_binary,
                config_path: runtime_config,
                workspace,
            };
            // Reconnect with simple backoff.
            let mut backoff = 1u64;
            loop {
                match daemon::run(params_clone(&params)).await {
                    Ok(()) => {
                        backoff = 1;
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "daemon disconnected");
                        tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                        backoff = (backoff * 2).min(30);
                    }
                }
            }
        }
```

Replace the imports at the top of `main.rs`:

```rust
use agent_runtime_config::{backend_name_is_valid, RuntimeConfig};
use agent_server::config::{ws_url, DaemonConfig};
use agent_server::{config, daemon};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
```

Replace `params_clone` at the bottom of `main.rs` with:

```rust
// DaemonParams holds a RuntimeConfig + plain fields; clone by hand for reconnect.
fn params_clone(p: &daemon::DaemonParams) -> daemon::DaemonParams {
    daemon::DaemonParams {
        ws_url: p.ws_url.clone(),
        agent_token: p.agent_token.clone(),
        config: p.config.clone(),
        api_key: p.api_key.clone(),
        claude_binary: p.claude_binary.clone(),
        config_path: p.config_path.clone(),
        workspace: p.workspace.clone(),
    }
}
```

(The old `build_model` import and the `let client = build_model(...)` / protocol-forcing block are now gone — `normalized()`/`build_loop` own that.)

- [ ] **Step 3: Build and run the full workspace test suite**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test --workspace`
Expected: PASS — all crates, including the existing daemon/wire/runtime-config tests.

- [ ] **Step 4: Clippy gate**

Run: `source "$HOME/.cargo/env" && cd agent && cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-server/src/daemon.rs agent/crates/agent-server/src/main.rs
git commit -m "feat(agent-server): drive the loop through RuntimeState; --runtime-config flag"
```

---

### Task 6: Frontend wire types (`web/`)

**Files:**
- Modify: `web/src/wire.ts`
- Test: `web/test/wire.test.ts`

**Interfaces:**
- Produces:
  - `RuntimeSettings` interface (mirrors `RuntimeConfig`).
  - `Inbound` adds `settings_state` (`settings`, `workspace`, `api_key_set`, `hard_floor`) and `settings_error` (`message`).
  - `Outbound` adds `settings_get` and `settings_update` (`settings`).
  - `parseInbound` accepts `"settings_state"` and `"settings_error"`.

- [ ] **Step 1: Write the failing test**

Append to `web/test/wire.test.ts`:

```ts
import { parseInbound, type RuntimeSettings } from "../src/wire";

const sampleSettings: RuntimeSettings = {
  backend: "openai", base_url: "http://localhost:8080", model: "qwen", protocol: "native",
  command_allowlist: ["ls", "git"], command_denylist: [], temperature: 0.2,
  max_tokens: 2048, max_turns: 25, context_limit: 8192,
};

test("parses a settings_state frame", () => {
  const raw = JSON.stringify({
    v: 1, session_id: "s", kind: "settings_state", settings: sampleSettings,
    workspace: "/w", api_key_set: true, hard_floor: ["sudo"],
  });
  const f = parseInbound(raw);
  expect(f?.kind).toBe("settings_state");
  if (f?.kind === "settings_state") {
    expect(f.settings.model).toBe("qwen");
    expect(f.api_key_set).toBe(true);
    expect(f.hard_floor).toContain("sudo");
  }
});

test("parses a settings_error frame", () => {
  const raw = JSON.stringify({ v: 1, session_id: "s", kind: "settings_error", message: "bad" });
  const f = parseInbound(raw);
  expect(f?.kind).toBe("settings_error");
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd web && npx vitest run test/wire.test.ts`
Expected: FAIL — `parseInbound` returns null for `settings_state` (and `RuntimeSettings` is not exported).

- [ ] **Step 3: Write the implementation**

In `web/src/wire.ts`, add the `RuntimeSettings` interface after the `Display` type:

```ts
export interface RuntimeSettings {
  backend: string;
  base_url: string;
  model: string;
  protocol: string;
  command_allowlist: string[];
  command_denylist: string[];
  temperature: number;
  max_tokens: number;
  max_turns: number;
  context_limit: number;
}
```

Add to the `Inbound` union (before the closing `;`):

```ts
  | { v: number; session_id: string; kind: "settings_state"; settings: RuntimeSettings; workspace: string; api_key_set: boolean; hard_floor: string[] }
  | { v: number; session_id: string; kind: "settings_error"; message: string }
```

Add to the `Outbound` union (before the closing `;`):

```ts
  | { v: number; session_id: string; kind: "settings_get" }
  | { v: number; session_id: string; kind: "settings_update"; settings: RuntimeSettings }
```

In `parseInbound`, extend the accepted-kinds check:

```ts
  if (
    o.kind === "event" || o.kind === "approval_request" || o.kind === "presence" ||
    o.kind === "settings_state" || o.kind === "settings_error"
  ) {
    return o as unknown as Inbound;
  }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd web && npx vitest run test/wire.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src/wire.ts web/test/wire.test.ts
git commit -m "feat(web): settings_* wire types + parse"
```

---

### Task 7: Frontend state reducer for settings (`web/`)

**Files:**
- Modify: `web/src/state.ts`
- Test: `web/test/state.test.ts`

**Interfaces:**
- Consumes: `RuntimeSettings` (Task 6).
- Produces: `ConversationState` gains `settings: RuntimeSettings | null`, `settingsMeta: { workspace: string; apiKeySet: boolean; hardFloor: string[] } | null`, `settingsError: string | null`. Reducer handles `settings_state` (stores settings + meta, clears error) and `settings_error` (sets error); `reset` clears all three.

- [ ] **Step 1: Write the failing test**

Append to `web/test/state.test.ts`:

```ts
import type { RuntimeSettings } from "../src/wire";

const s: RuntimeSettings = {
  backend: "openai", base_url: "u", model: "m", protocol: "native",
  command_allowlist: [], command_denylist: [], temperature: 0.2,
  max_tokens: 2048, max_turns: 25, context_limit: 8192,
};

test("settings_state stores settings + meta and clears error", () => {
  let st = initialState([]);
  st = reduce(st, { type: "frame", frame: { v: 1, session_id: "x", kind: "settings_error", message: "old" } });
  expect(st.settingsError).toBe("old");
  st = reduce(st, { type: "frame", frame: {
    v: 1, session_id: "x", kind: "settings_state", settings: s,
    workspace: "/w", api_key_set: true, hard_floor: ["sudo"] } });
  expect(st.settings?.model).toBe("m");
  expect(st.settingsMeta?.workspace).toBe("/w");
  expect(st.settingsMeta?.apiKeySet).toBe(true);
  expect(st.settingsMeta?.hardFloor).toEqual(["sudo"]);
  expect(st.settingsError).toBeNull();
});

test("settings_error sets the error message", () => {
  let st = initialState([]);
  st = reduce(st, { type: "frame", frame: { v: 1, session_id: "x", kind: "settings_error", message: "nope" } });
  expect(st.settingsError).toBe("nope");
});
```

(If `initialState`/`reduce` are not already imported at the top of `state.test.ts`, add them: `import { initialState, reduce } from "../src/state";`.)

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd web && npx vitest run test/state.test.ts`
Expected: FAIL — `settings`/`settingsError` do not exist on the state.

- [ ] **Step 3: Write the implementation**

In `web/src/state.ts`, update the import line:

```ts
import type { Display, Inbound, RuntimeSettings } from "./wire";
```

Add to the `ConversationState` interface:

```ts
  settings: RuntimeSettings | null;
  settingsMeta: { workspace: string; apiKeySet: boolean; hardFloor: string[] } | null;
  settingsError: string | null;
```

Update `initialState` to seed them:

```ts
export function initialState(userMsgs: string[]): ConversationState {
  return { items: [], pendingApproval: null, online: false, status: "connecting",
    userMsgs, turnIndex: 0, inTurn: false,
    settings: null, settingsMeta: null, settingsError: null };
}
```

In `reduceFrame`, add these two branches at the top (right after the `presence` branch, before `approval_request`):

```ts
  if (frame.kind === "settings_state") {
    return { ...state, settings: frame.settings,
      settingsMeta: { workspace: frame.workspace, apiKeySet: frame.api_key_set, hardFloor: frame.hard_floor },
      settingsError: null };
  }
  if (frame.kind === "settings_error") {
    return { ...state, settingsError: frame.message };
  }
```

(`reset` already calls `initialState`, so it clears the new fields automatically.)

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd web && npx vitest run test/state.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src/state.ts web/test/state.test.ts
git commit -m "feat(web): reducer handles settings_state / settings_error"
```

---

### Task 8: `SettingsPanel` component + StatusBar gear + App wiring (`web/`)

**Files:**
- Create: `web/src/components/SettingsPanel.tsx`
- Modify: `web/src/components/StatusBar.tsx`
- Modify: `web/src/App.tsx`
- Test: `web/test/settings-panel.test.tsx`

**Interfaces:**
- Consumes: `RuntimeSettings` (Task 6), `ConversationState.settings`/`settingsMeta`/`settingsError` (Task 7), the socket `send` for `settings_get`/`settings_update` (Task 6).
- Produces: `SettingsPanel` component with props `{ settings, meta, error, disabled, onSave, onClose }`; `StatusBar` gains optional `onOpenSettings?: () => void`.

- [ ] **Step 1: Write the failing test**

Create `web/test/settings-panel.test.tsx`:

```tsx
import { render, screen, fireEvent } from "@testing-library/react";
import { SettingsPanel } from "../src/components/SettingsPanel";
import type { RuntimeSettings } from "../src/wire";

const settings: RuntimeSettings = {
  backend: "openai", base_url: "http://localhost:8080", model: "qwen", protocol: "native",
  command_allowlist: ["ls", "git"], command_denylist: ["foo"], temperature: 0.2,
  max_tokens: 2048, max_turns: 25, context_limit: 8192,
};
const meta = { workspace: "/home/me/proj", apiKeySet: true, hardFloor: ["sudo", "rm -rf /"] };

test("renders fields and read-only metadata", () => {
  render(<SettingsPanel settings={settings} meta={meta} error={null} disabled={false}
    onSave={() => {}} onClose={() => {}} />);
  expect(screen.getByLabelText(/model/i)).toHaveValue("qwen");
  expect(screen.getByText("/home/me/proj")).toBeInTheDocument();
  expect(screen.getByText(/sudo/)).toBeInTheDocument();
  expect(screen.getByText(/api key/i)).toBeInTheDocument();
});

test("editing the model and saving emits the updated settings", () => {
  const onSave = vi.fn();
  render(<SettingsPanel settings={settings} meta={meta} error={null} disabled={false}
    onSave={onSave} onClose={() => {}} />);
  fireEvent.change(screen.getByLabelText(/model/i), { target: { value: "new-model" } });
  fireEvent.click(screen.getByRole("button", { name: /save/i }));
  expect(onSave).toHaveBeenCalledTimes(1);
  expect(onSave.mock.calls[0][0].model).toBe("new-model");
});

test("textareas convert newline lists back to arrays on save", () => {
  const onSave = vi.fn();
  render(<SettingsPanel settings={settings} meta={meta} error={null} disabled={false}
    onSave={onSave} onClose={() => {}} />);
  fireEvent.change(screen.getByLabelText(/allowlist/i), { target: { value: "ls\ncat\n" } });
  fireEvent.click(screen.getByRole("button", { name: /save/i }));
  expect(onSave.mock.calls[0][0].command_allowlist).toEqual(["ls", "cat"]);
});

test("shows an error message", () => {
  render(<SettingsPanel settings={settings} meta={meta} error="bad base_url" disabled={false}
    onSave={() => {}} onClose={() => {}} />);
  expect(screen.getByText("bad base_url")).toBeInTheDocument();
});

test("save is disabled when offline", () => {
  render(<SettingsPanel settings={settings} meta={meta} error={null} disabled={true}
    onSave={() => {}} onClose={() => {}} />);
  expect(screen.getByRole("button", { name: /save/i })).toBeDisabled();
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd web && npx vitest run test/settings-panel.test.tsx`
Expected: FAIL — cannot resolve `../src/components/SettingsPanel`.

- [ ] **Step 3: Write the `SettingsPanel` component**

Create `web/src/components/SettingsPanel.tsx`:

```tsx
import { useState } from "react";
import type { RuntimeSettings } from "../wire";

interface Meta { workspace: string; apiKeySet: boolean; hardFloor: string[] }

interface Props {
  settings: RuntimeSettings;
  meta: Meta | null;
  error: string | null;
  disabled: boolean;
  onSave: (s: RuntimeSettings) => void;
  onClose: () => void;
}

const toLines = (xs: string[]) => xs.join("\n");
const fromLines = (s: string) => s.split("\n").map((l) => l.trim()).filter((l) => l.length > 0);

export function SettingsPanel({ settings, meta, error, disabled, onSave, onClose }: Props) {
  const [form, setForm] = useState<RuntimeSettings>(settings);
  const [allow, setAllow] = useState(toLines(settings.command_allowlist));
  const [deny, setDeny] = useState(toLines(settings.command_denylist));

  const set = <K extends keyof RuntimeSettings>(k: K, v: RuntimeSettings[K]) =>
    setForm((f) => ({ ...f, [k]: v }));

  const save = () => onSave({ ...form, command_allowlist: fromLines(allow), command_denylist: fromLines(deny) });

  const field = "w-full rounded bg-zinc-800 px-2 py-1 text-sm text-zinc-100";
  const label = "block text-xs uppercase tracking-wide text-zinc-400 mb-1";

  return (
    <div className="absolute inset-0 z-10 flex justify-end bg-black/50">
      <div className="h-full w-96 overflow-y-auto bg-zinc-900 p-4 text-zinc-200 shadow-xl">
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-lg font-semibold">Settings</h2>
          <button onClick={onClose} className="text-zinc-400 hover:text-zinc-200">close</button>
        </div>

        {error && <div className="mb-3 rounded bg-red-900/50 px-2 py-1 text-sm text-red-200">{error}</div>}

        <section className="mb-4 space-y-3">
          <h3 className="text-sm font-semibold text-zinc-300">Model &amp; inference</h3>
          <div>
            <label className={label} htmlFor="backend">Backend</label>
            <select id="backend" className={field} value={form.backend}
              onChange={(e) => set("backend", e.target.value)}>
              <option value="openai">openai</option>
              <option value="claude-cli">claude-cli</option>
            </select>
          </div>
          <div>
            <label className={label} htmlFor="base_url">Base URL</label>
            <input id="base_url" className={field} value={form.base_url}
              onChange={(e) => set("base_url", e.target.value)} />
          </div>
          <div>
            <label className={label} htmlFor="model">Model</label>
            <input id="model" className={field} value={form.model}
              onChange={(e) => set("model", e.target.value)} />
          </div>
          <div>
            <label className={label} htmlFor="protocol">Protocol</label>
            <select id="protocol" className={field} value={form.protocol}
              onChange={(e) => set("protocol", e.target.value)}>
              <option value="native">native</option>
              <option value="prompted">prompted</option>
            </select>
          </div>
        </section>

        <section className="mb-4 space-y-3">
          <h3 className="text-sm font-semibold text-zinc-300">Command policy</h3>
          <div>
            <label className={label} htmlFor="allowlist">Allowlist (one per line)</label>
            <textarea id="allowlist" rows={4} className={field} value={allow}
              onChange={(e) => setAllow(e.target.value)} />
          </div>
          <div>
            <label className={label} htmlFor="denylist">Denylist (one per line)</label>
            <textarea id="denylist" rows={3} className={field} value={deny}
              onChange={(e) => setDeny(e.target.value)} />
          </div>
          {meta && (
            <p className="text-xs text-zinc-500">
              Always blocked (hard floor): {meta.hardFloor.join(", ")}
            </p>
          )}
        </section>

        <section className="mb-4 space-y-3">
          <h3 className="text-sm font-semibold text-zinc-300">Loop tuning</h3>
          <div>
            <label className={label} htmlFor="temperature">Temperature</label>
            <input id="temperature" type="number" step="0.1" className={field} value={form.temperature}
              onChange={(e) => set("temperature", Number(e.target.value))} />
          </div>
          <div>
            <label className={label} htmlFor="max_tokens">Max tokens</label>
            <input id="max_tokens" type="number" className={field} value={form.max_tokens}
              onChange={(e) => set("max_tokens", Number(e.target.value))} />
          </div>
          <div>
            <label className={label} htmlFor="max_turns">Max turns</label>
            <input id="max_turns" type="number" className={field} value={form.max_turns}
              onChange={(e) => set("max_turns", Number(e.target.value))} />
          </div>
          <div>
            <label className={label} htmlFor="context_limit">Context limit</label>
            <input id="context_limit" type="number" className={field} value={form.context_limit}
              onChange={(e) => set("context_limit", Number(e.target.value))} />
          </div>
        </section>

        {meta && (
          <section className="mb-4 text-xs text-zinc-500">
            <p>Workspace: {meta.workspace}</p>
            <p>API key: {meta.apiKeySet ? "set" : "not set"}</p>
          </section>
        )}

        <button onClick={save} disabled={disabled}
          className="w-full rounded bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-500 disabled:opacity-40">
          Save
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Add the gear to `StatusBar`**

In `web/src/components/StatusBar.tsx`, change the signature and add the button:

```tsx
export function StatusBar({ online, status, onSignOut, onOpenSettings }:
  { online: boolean; status: ConnectionStatus; onSignOut: () => void; onOpenSettings?: () => void }) {
  return (
    <div className="flex items-center justify-between border-b border-zinc-800 bg-zinc-950 px-4 py-2 text-sm">
      <div className="flex items-center gap-2">
        <span className={`h-2 w-2 rounded-full ${online ? "bg-green-400" : "bg-zinc-600"}`} />
        <span className="text-zinc-300">{online ? "agent online" : "agent offline"}</span>
        <span className="text-zinc-600">· {status}</span>
      </div>
      <div className="flex items-center gap-3">
        {onOpenSettings && (
          <button onClick={onOpenSettings} className="text-zinc-400 hover:text-zinc-200" aria-label="settings">⚙</button>
        )}
        <button onClick={onSignOut} className="text-zinc-400 hover:text-zinc-200">sign out</button>
      </div>
    </div>
  );
}
```

- [ ] **Step 5: Wire the panel into `App.tsx`**

In `web/src/App.tsx`, add the import:

```tsx
import { SettingsPanel } from "./components/SettingsPanel";
```

Add panel state inside the component (next to the other `useState` calls):

```tsx
  const [showSettings, setShowSettings] = useState(false);
```

Add handlers (next to `send`/`decide`/`signOut`):

```tsx
  const openSettings = () => {
    setShowSettings(true);
    sock.current?.send({ v: 1, session_id: sessionId, kind: "settings_get" });
  };
  const saveSettings = (s: import("./wire").RuntimeSettings) => {
    sock.current?.send({ v: 1, session_id: sessionId, kind: "settings_update", settings: s });
  };
```

Update the `StatusBar` usage and render the panel inside the main `<div>` (after `<StatusBar .../>`):

```tsx
      <StatusBar online={state.online} status={state.status} onSignOut={signOut} onOpenSettings={openSettings} />
      {showSettings && state.settings && (
        <SettingsPanel
          settings={state.settings}
          meta={state.settingsMeta}
          error={state.settingsError}
          disabled={!connected}
          onSave={saveSettings}
          onClose={() => setShowSettings(false)}
        />
      )}
```

(Note: `connected` is defined just above the `return`; ensure the panel block is inside the returned JSX where `connected` is in scope.)

- [ ] **Step 6: Run the component test to verify it passes**

Run: `cd web && npx vitest run test/settings-panel.test.tsx`
Expected: PASS (5 tests).

- [ ] **Step 7: Run the full web suite + typecheck/build**

Run: `cd web && npx vitest run && npm run build`
Expected: all tests PASS; `tsc`/`vite build` succeeds (catches any type wiring errors in `App.tsx`).

- [ ] **Step 8: Commit**

```bash
git add web/src/components/SettingsPanel.tsx web/src/components/StatusBar.tsx web/src/App.tsx web/test/settings-panel.test.tsx
git commit -m "feat(web): SettingsPanel + gear + App wiring (settings_get/update)"
```

---

## Final verification (run after all tasks)

- [ ] **Rust:** `source "$HOME/.cargo/env" && cd agent && cargo test --workspace && cargo clippy --all-targets -- -D warnings` → green.
- [ ] **Web:** `cd web && npx vitest run && npm run build` → green.
- [ ] **Cloud untouched / still green:** `cd cloud && npm test` → green (no `cloud/` files changed).
- [ ] **Manual chrome E2E (human, optional, per `cloud/RUNNING.md`):** bring up the stack; open the app; gear → Settings; (a) switch backend `openai`↔`claude-cli`, save, confirm the *next* turn uses the new backend without a reconnect; (b) add a command to the allowlist, save, confirm a previously-gated command now auto-approves; (c) edit a loop-tuning value; (d) restart the daemon and confirm settings persisted (`agent-runtime.json` present, values retained); (e) confirm the hard-floor entries cannot be removed (they remain enforced even if the denylist is cleared).

## Self-review notes (verified against the spec)

- **Spec §2 scope** (model & inference / command policy / loop tuning; workspace + MCP + secrets out): covered by `RuntimeConfig` fields (Task 1) and `SettingsPanel` sections (Task 8); workspace is read-only metadata; no MCP/secret fields exist.
- **Spec §3 wire additions** (`settings_get`/`settings_update`/`settings_state`/`settings_error`): Task 3 (Rust) + Task 6 (TS). `settings_state` carries `workspace`/`api_key_set`/`hard_floor`.
- **Spec §4 config model & persistence** (flag-seed, file-wins-per-field, persist-then-swap, validation incl. claude-cli⇒prompted): Tasks 1, 2, 4 (`apply` persists before swapping), 5 (`--runtime-config`, `load_over` on connect).
- **Spec §5 security** (hard floor always enforced via `effective_denylist`; secrets never on wire — only `api_key_set`): Tasks 1 + 4; no key field anywhere.
- **Spec §6 daemon integration** (`Mutex<Arc<AgentLoop>>`, next-turn no-interrupt, persistent `WindowContext`): Tasks 4 + 5. The in-flight run clones its `Arc` before any swap; `ctx` is built once and reused.
- **Spec §7 frontend**: Tasks 6–8.
- **Spec §8 testing**: every listed Rust + web test is present; manual E2E in Final verification.
- **Invariants**: zero `cloud/` changes (verified in Final verification); core crates untouched (all edits are in `agent-runtime-config`/`agent-server`/`web`); protocol version stays `1`.
