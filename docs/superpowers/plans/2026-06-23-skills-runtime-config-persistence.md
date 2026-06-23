# Skills in `RuntimeConfig` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the daemon's `skills_dirs` and `active_skills` part of `RuntimeConfig` so they persist to disk, round-trip on the wire, live-apply mid-session, and are editable from a browser Settings UI.

**Architecture:** Add two `Vec<String>` fields to `RuntimeConfig`. Add one additive core seam (`ContextManager::set_system`) so a live session's system prompt can be recomposed. Make the daemon's `build_loop` the single place skills are built — it rebuilds the skill registry/tools and composes the system prompt on every reconfigure; the per-turn handler applies the freshly composed prompt under the existing "next-turn, no interrupt" discipline. Surface the daemon's discovered skills as read-only `settings_state` data for a self-validating checklist in the web UI.

**Tech Stack:** Rust (Cargo workspace under `agent/`), serde/serde_json, tokio; React + Vite + TS + Tailwind + Vitest under `web/`.

## Global Constraints

- **Cargo is not on PATH.** Run `source "$HOME/.cargo/env"` before any cargo command. Build/test from `agent/`: `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings` (must stay clean).
- **Web** commands run from `web/`: `npm test` (Vitest+jsdom).
- **Core-crate freeze, one exception:** `agent-core` gets exactly one additive method (`set_system`). `agent-model`, `agent-tools`, `agent-policy`, and the `agent-skills` crate internals stay **unchanged**. Do not add functions to `agent-skills`.
- **Additive serde:** every new `RuntimeConfig` field is `#[serde(default)]` so files written by older builds still load.
- **No new execution authority.** Nothing in this plan runs commands or scripts.
- **CLI is out of scope.** Do not touch `agent-cli`; its `--skills-dir`/`--skill` flags stay launch-only.
- Spec: `docs/superpowers/specs/2026-06-23-skills-runtime-config-persistence-design.md`.

## File Structure

- `agent/crates/agent-runtime-config/src/runtime_config.rs` — **Modify.** +2 fields, `PartialRuntimeConfig` arms, `merge`, `from_launch`, tests.
- `agent/crates/agent-core/src/context.rs` — **Modify.** `ContextManager::set_system` + `WindowContext` impl + test.
- `agent/crates/agent-server/src/wire.rs` — **Modify.** `DiscoveredSkill` type; `SettingsState` gains `discovered_skills`; tests.
- `agent/crates/agent-server/src/runtime.rs` — **Modify.** `build_loop` builds skills + composes prompt + returns dropped presets; `RuntimeState` stores/exposes the composed prompt; `state_body` populates `discovered_skills`; strict-on-wire / lenient-at-startup; tests.
- `agent/crates/agent-server/src/daemon.rs` — **Modify.** `RuntimeState::new` call passes the base prompt; per-turn `set_system`.
- `agent/crates/agent-server/src/main.rs` — **Modify.** Stop pre-building skills; `mcp_tools` MCP-only; seed `base.skills_dirs`/`base.active_skills` from flags; pass `SYSTEM_PROMPT` as the base.
- `web/src/wire.ts` — **Modify.** `RuntimeSettings` +2 fields; `DiscoveredSkill` type; `settings_state` inbound +`discovered_skills`.
- `web/src/state.ts` — **Modify.** Map `discovered_skills` into `settingsMeta`.
- `web/src/components/SettingsPanel.tsx` — **Modify.** New "Skills" section (dirs textarea + discovered-skills checklist).
- `web/src/components/SettingsPanel.test.tsx` — **Modify.** Fixture +2 fields; new tests.

---

### Task 1: `RuntimeConfig` gains `skills_dirs` + `active_skills`

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs`
- Test: same file (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `RuntimeConfig.skills_dirs: Vec<String>` and `RuntimeConfig.active_skills: Vec<String>`, both `#[serde(default)]`, seeded empty by `from_launch`, merged per-field by `load_over`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `runtime_config.rs`:

```rust
#[test]
fn skills_fields_default_empty_and_round_trip() {
    let c = base();
    assert!(c.skills_dirs.is_empty());
    assert!(c.active_skills.is_empty());

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rt.json");
    let mut c = base();
    c.skills_dirs = vec!["/ws/.agent/skills".into()];
    c.active_skills = vec!["greeter".into()];
    c.save(&path).unwrap();

    // A different base proves the file wins; a partial file falls back per-field.
    let loaded = RuntimeConfig::load_over(base(), &path);
    assert_eq!(loaded.skills_dirs, vec!["/ws/.agent/skills".to_string()]);
    assert_eq!(loaded.active_skills, vec!["greeter".to_string()]);

    std::fs::write(&path, r#"{"model":"only-model"}"#).unwrap();
    let loaded = RuntimeConfig::load_over(base(), &path);
    assert_eq!(loaded.model, "only-model");
    assert!(loaded.skills_dirs.is_empty());   // absent field falls back to base
    assert!(loaded.active_skills.is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-runtime-config skills_fields_default_empty_and_round_trip`
Expected: FAIL — `no field skills_dirs on type RuntimeConfig` (compile error).

- [ ] **Step 3: Add the fields, partial arms, and merge**

In `struct RuntimeConfig`, after the `preserve_thinking` field (line ~37), add:

```rust
    #[serde(default)]
    pub skills_dirs: Vec<String>,
    #[serde(default)]
    pub active_skills: Vec<String>,
```

In `struct PartialRuntimeConfig`, after `preserve_thinking: Option<bool>,` add:

```rust
    skills_dirs: Option<Vec<String>>,
    active_skills: Option<Vec<String>>,
```

In `from_launch`, inside the `Self { ... }` literal (after `preserve_thinking: false,`), add:

```rust
            skills_dirs: Vec::new(),
            active_skills: Vec::new(),
```

In `merge`, after `if let Some(v) = p.preserve_thinking { self.preserve_thinking = v; }`, add:

```rust
        if let Some(v) = p.skills_dirs { self.skills_dirs = v; }
        if let Some(v) = p.active_skills { self.active_skills = v; }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-runtime-config`
Expected: PASS (all tests, including the new one).

- [ ] **Step 5: Clippy + commit**

```bash
source "$HOME/.cargo/env" && cd agent && cargo clippy -p agent-runtime-config --all-targets -- -D warnings
git add agent/crates/agent-runtime-config/src/runtime_config.rs
git commit -m "feat(runtime-config): persist skills_dirs/active_skills in RuntimeConfig"
```

---

### Task 2: Core seam — `ContextManager::set_system`

**Files:**
- Modify: `agent/crates/agent-core/src/context.rs`
- Test: same file (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `ContextManager::set_system(&mut self, system: Message)`, implemented by `WindowContext` to replace its system message while preserving history.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `context.rs`:

```rust
#[test]
fn set_system_replaces_prompt_and_keeps_history() {
    let mut ctx = WindowContext::new(Message::system("OLD"));
    ctx.append(Message::user("u1"));
    ctx.set_system(Message::system("NEW"));
    let built = ctx.build(100_000);
    assert!(matches!(built[0].role, Role::System)); // system still first
    assert_eq!(built[0].content, "NEW");            // and replaced
    assert!(built.iter().any(|m| m.content == "u1")); // history intact
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-core set_system_replaces_prompt_and_keeps_history`
Expected: FAIL — `no method named set_system found` (compile error).

- [ ] **Step 3: Add the trait method and impl**

In the `ContextManager` trait (lines 13-16), add a third method:

```rust
pub trait ContextManager: Send + Sync {
    fn append(&mut self, msg: Message);
    fn build(&self, model_limit: usize) -> Vec<Message>;
    fn set_system(&mut self, system: Message);
}
```

In `impl ContextManager for WindowContext`, add the implementation (after `append`):

```rust
    fn set_system(&mut self, system: Message) {
        self.system = system;
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-core`
Expected: PASS. (Only `WindowContext` implements `ContextManager`, so no other impl breaks.)

- [ ] **Step 5: Clippy + commit**

```bash
source "$HOME/.cargo/env" && cd agent && cargo clippy -p agent-core --all-targets -- -D warnings
git add agent/crates/agent-core/src/context.rs
git commit -m "feat(core): add ContextManager::set_system to re-system a live context"
```

---

### Task 3: `discovered_skills` in the `settings_state` wire frame

**Files:**
- Modify: `agent/crates/agent-server/src/wire.rs`
- Modify: `agent/crates/agent-server/src/runtime.rs` (only `state_body` + one test)
- Test: both files' `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `RuntimeConfig.skills_dirs` (Task 1); `agent_skills::SkillRegistry::from_config(&[String], &Path) -> SkillRegistry` and its `.scan() -> Vec<Skill>` where `Skill { name: String, description: String, .. }`.
- Produces: `wire::DiscoveredSkill { name: String, description: String }`; `WireBody::SettingsState` gains `discovered_skills: Vec<DiscoveredSkill>`.

- [ ] **Step 1: Write the failing test (wire.rs)**

In `wire.rs`, replace the existing `settings_state_and_error_serialize` test's `SettingsState { .. }` construction so it includes the new field, and assert it serializes. Change the body block to:

```rust
            body: WireBody::SettingsState {
                settings: cfg, workspace: "/w".into(), api_key_set: true,
                hard_floor: vec!["sudo".into()],
                discovered_skills: vec![crate::wire::DiscoveredSkill {
                    name: "greeter".into(), description: "says hi".into() }] },
```

And after `assert!(j.contains("\"api_key_set\":true"));` add:

```rust
        assert!(j.contains("\"discovered_skills\""));
        assert!(j.contains("greeter"));
```

- [ ] **Step 2: Run test to verify it fails**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-server --lib settings_state_and_error_serialize`
Expected: FAIL — `DiscoveredSkill` not found / missing field `discovered_skills` (compile error).

- [ ] **Step 3: Add the type and field**

In `wire.rs`, after the `WireBody` enum (after line 47), add:

```rust
/// Read-only skill info surfaced in `settings_state` for the Settings UI's
/// active-skills picker. Daemon-computed from the current `skills_dirs`; never
/// part of `RuntimeConfig`, so it cannot be edited or round-tripped back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredSkill {
    pub name: String,
    pub description: String,
}
```

In the `SettingsState` variant, add the field after `hard_floor: Vec<String>,`:

```rust
        discovered_skills: Vec<DiscoveredSkill>,
```

- [ ] **Step 4: Populate it in `state_body` (runtime.rs)**

In `runtime.rs`, add the import near the top (after the `agent_runtime_config` use line):

```rust
use crate::wire::DiscoveredSkill;
use agent_skills::SkillRegistry;
```

Replace `state_body` (lines ~71-78) with:

```rust
    fn state_body(&self) -> WireBody {
        let cfg = self.config.lock().unwrap().clone();
        let discovered = SkillRegistry::from_config(&cfg.skills_dirs, &self.workspace)
            .scan()
            .into_iter()
            .map(|s| DiscoveredSkill { name: s.name, description: s.description })
            .collect();
        WireBody::SettingsState {
            settings: cfg,
            workspace: self.workspace.display().to_string(),
            api_key_set: self.api_key.is_some(),
            hard_floor: HARD_FLOOR_DENYLIST.iter().map(|s| s.to_string()).collect(),
            discovered_skills: discovered,
        }
    }
```

- [ ] **Step 5: Add a `state_body` test (runtime.rs)**

Add to the `tests` module in `runtime.rs`:

```rust
#[test]
fn settings_state_includes_discovered_skills() {
    use std::fs;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let session = Arc::new(Mutex::new(String::new()));
    let sink = Arc::new(WsEventSink::new(tx.clone(), session.clone()));
    let approval = Arc::new(WsApprovalChannel::new(tx.clone(), session.clone(), Duration::from_secs(1)));
    let dir = tempfile::tempdir().unwrap();
    // Put a skill in <workspace>/.agent/skills/greeter (the default writable root).
    let sdir = dir.path().join(".agent").join("skills").join("greeter");
    fs::create_dir_all(&sdir).unwrap();
    fs::write(sdir.join("SKILL.md"), "---\nname: greeter\ndescription: says hi\n---\nbody").unwrap();
    let path = dir.path().join("rt.json");
    let cfg = RuntimeConfig::from_launch(
        "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
    let rs = RuntimeState::new(cfg, sink, approval, dir.path().to_path_buf(), None,
        "claude".into(), path, session, tx, Arc::from(Vec::<Arc<dyn Tool>>::new()));
    assert!(rs.handle(&WireBody::SettingsGet));
    let env = rx.try_recv().expect("a frame");
    match env.body {
        WireBody::SettingsState { discovered_skills, .. } => {
            assert!(discovered_skills.iter().any(|s| s.name == "greeter" && s.description == "says hi"));
        }
        _ => panic!("expected settings_state"),
    }
}
```

> Note: `RuntimeState::new`'s signature is unchanged in this task. If Task 4 (which adds a `base_system_prompt` parameter) has already landed, add `daemon::SYSTEM_PROMPT.to_string()` as that argument here too.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-server`
Expected: PASS (lib unit tests, including the two new ones; the `daemon_roundtrip` integration test still passes — it only checks `kind == "settings_state"`).

- [ ] **Step 7: Clippy + commit**

```bash
source "$HOME/.cargo/env" && cd agent && cargo clippy -p agent-server --all-targets -- -D warnings
git add agent/crates/agent-server/src/wire.rs agent/crates/agent-server/src/runtime.rs
git commit -m "feat(server): surface discovered_skills in settings_state for the UI picker"
```

---

### Task 4: Daemon live-apply — build skills + compose prompt in `build_loop`

**Files:**
- Modify: `agent/crates/agent-server/src/runtime.rs`
- Modify: `agent/crates/agent-server/src/daemon.rs`
- Modify: `agent/crates/agent-server/src/main.rs`
- Test: `runtime.rs` `#[cfg(test)] mod tests`

This is one cohesive deliverable: the four files must change together to compile. Steps 1-3 change `runtime.rs`; steps 4-5 rewire `daemon.rs` + `main.rs`; step 6 tests.

**Interfaces:**
- Consumes: `RuntimeConfig.skills_dirs`/`active_skills` (Task 1); `ContextManager::set_system` (Task 2); `agent_runtime_config::build_skills(&[String], &Path) -> (Arc<SkillRegistry>, Vec<Arc<dyn Tool>>)`; `agent_skills::compose_system_prompt(&str, &SkillRegistry, &[String]) -> Result<String, String>`.
- Produces: `RuntimeState::new(.., base_system_prompt: String)` (new last-ish param — see exact order below); `RuntimeState::current_system_prompt(&self) -> String`; `build_loop(..) -> BuiltLoop` where `struct BuiltLoop { loop_: Arc<AgentLoop>, system_prompt: String, unknown_presets: Vec<String> }`.

- [ ] **Step 1: Write the failing tests (runtime.rs)**

Add to the `tests` module in `runtime.rs`. (The existing `make()` helper builds a `RuntimeState`; you will update `make()` in Step 3 to pass the new `base_system_prompt` arg, so these tests compile once the impl lands.)

```rust
#[test]
fn apply_with_valid_active_skill_updates_system_prompt() {
    use std::fs;
    let (rs, _rx, dir) = make();
    // Author a skill under the workspace default writable root.
    let sdir = dir.path().join(".agent").join("skills").join("greeter");
    fs::create_dir_all(&sdir).unwrap();
    fs::write(sdir.join("SKILL.md"),
        "---\nname: greeter\ndescription: d\n---\nSay hi politely.").unwrap();

    let before = rs.current_loop();
    let mut next = RuntimeConfig::from_launch(
        "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
    next.active_skills = vec!["greeter".into()];
    rs.apply(next).unwrap();

    assert!(!Arc::ptr_eq(&before, &rs.current_loop()), "loop swapped");
    assert!(rs.current_system_prompt().contains("Say hi politely."),
        "preset body folded into the live system prompt");
}

#[test]
fn apply_rejects_unknown_active_skill_without_swapping() {
    let (rs, _rx, _dir) = make();
    let before = rs.current_loop();
    let mut bad = RuntimeConfig::from_launch(
        "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
    bad.active_skills = vec!["does-not-exist".into()];
    let err = rs.apply(bad).unwrap_err();
    assert!(err.contains("does-not-exist"), "error names the missing skill: {err}");
    assert!(Arc::ptr_eq(&before, &rs.current_loop()), "loop unchanged on rejection");
}

#[test]
fn startup_drops_unknown_persisted_preset_without_panicking() {
    // A persisted config naming a non-existent preset must still boot (lenient).
    let (tx, _rx) = mpsc::unbounded_channel();
    let session = Arc::new(Mutex::new(String::new()));
    let sink = Arc::new(WsEventSink::new(tx.clone(), session.clone()));
    let approval = Arc::new(WsApprovalChannel::new(tx.clone(), session.clone(), Duration::from_secs(1)));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rt.json");
    let mut cfg = RuntimeConfig::from_launch(
        "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
    cfg.active_skills = vec!["ghost".into()];
    let rs = RuntimeState::new(cfg, sink, approval, dir.path().to_path_buf(), None,
        "claude".into(), path, session, tx, Arc::from(Vec::<Arc<dyn Tool>>::new()),
        crate::daemon::SYSTEM_PROMPT.to_string());
    // Booted: base prompt present, the ghost preset silently dropped.
    assert!(rs.current_system_prompt().contains("local coding agent"));
    assert!(!rs.current_system_prompt().contains("ghost"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-server --lib apply_with_valid_active_skill_updates_system_prompt`
Expected: FAIL — `no method named current_system_prompt` / arity mismatch on `RuntimeState::new` (compile error).

- [ ] **Step 3: Refactor `build_loop` + `RuntimeState` (runtime.rs)**

Add imports near the top of `runtime.rs` (alongside the existing `agent_runtime_config` use):

```rust
use agent_runtime_config::build_skills;
use agent_skills::compose_system_prompt;
use std::collections::HashSet;
```

Add a `base_system_prompt: String` field and a `system_prompt: Mutex<String>` field to `struct RuntimeState` (after `mcp_tools`):

```rust
    base_system_prompt: String,
    system_prompt: Mutex<String>,
```

Change `RuntimeState::new` to accept `base_system_prompt: String` as a **new final parameter** and use the new `build_loop`:

```rust
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
        mcp_tools: Arc<[Arc<dyn Tool>]>,
        base_system_prompt: String,
    ) -> Self {
        let config = config.normalized();
        let built = build_loop(
            &config, &sink, &approval, &workspace, &api_key, &claude_binary, &mcp_tools,
            &base_system_prompt);
        // Startup is lenient: an unknown persisted preset is already warned + dropped
        // inside build_loop, so the daemon always boots.
        Self {
            loop_cell: Mutex::new(built.loop_),
            config: Mutex::new(config),
            system_prompt: Mutex::new(built.system_prompt),
            sink, approval, workspace, api_key, claude_binary, config_path, session, tx,
            mcp_tools, base_system_prompt,
        }
    }
```

Add the accessor (next to `current_loop`):

```rust
    /// The composed system prompt for the live config (base + awareness + presets).
    /// Applied to the session context at the start of each turn (next-turn discipline).
    pub fn current_system_prompt(&self) -> String {
        self.system_prompt.lock().unwrap().clone()
    }
```

Rewrite `apply` to build first, reject unknown presets, then persist + swap:

```rust
    /// Validate+normalize, build (rejecting unknown presets), persist, then swap.
    /// On any failure, nothing changes.
    pub fn apply(&self, incoming: RuntimeConfig) -> Result<(), String> {
        let cfg = incoming.normalized();
        cfg.validate()?;
        let built = build_loop(
            &cfg, &self.sink, &self.approval, &self.workspace, &self.api_key,
            &self.claude_binary, &self.mcp_tools, &self.base_system_prompt);
        if !built.unknown_presets.is_empty() {
            // Strict on the wire: a typo'd / missing active skill is a hard error.
            return Err(format!("unknown active skill(s): {}", built.unknown_presets.join(", ")));
        }
        cfg.save(&self.config_path).map_err(|e| format!("persist failed: {e}"))?;
        *self.loop_cell.lock().unwrap() = built.loop_;
        *self.system_prompt.lock().unwrap() = built.system_prompt;
        *self.config.lock().unwrap() = cfg;
        Ok(())
    }
```

Add the `BuiltLoop` struct and rewrite `build_loop` to build skills + compose the prompt. Replace the existing `fn build_loop(...)` (lines ~113-159) with:

```rust
/// The result of (re)building the loop: the loop itself, the composed system
/// prompt, and any `active_skills` names that were not found (dropped from the
/// prompt). Callers decide whether unknown presets are fatal (wire) or tolerated
/// (startup).
struct BuiltLoop {
    loop_: Arc<AgentLoop>,
    system_prompt: String,
    unknown_presets: Vec<String>,
}

/// Assemble an `AgentLoop` from a config + the persistent seams, building the
/// skill registry/tools from `cfg.skills_dirs` and composing the system prompt
/// from `cfg.active_skills`. The one place a `RuntimeConfig` becomes a loop.
#[allow(clippy::too_many_arguments)]
fn build_loop(
    cfg: &RuntimeConfig,
    sink: &Arc<WsEventSink>,
    approval: &Arc<WsApprovalChannel>,
    workspace: &Path,
    api_key: &Option<String>,
    claude_binary: &str,
    mcp_tools: &[Arc<dyn Tool>],
    base_system_prompt: &str,
) -> BuiltLoop {
    let model = build_model(&cfg.backend, &cfg.base_url, &cfg.model, claude_binary, api_key.clone());
    let policy = Arc::new(RulePolicy {
        workspace: workspace.to_path_buf(),
        command_allowlist: cfg.command_allowlist.clone(),
        command_denylist: cfg.effective_denylist(),
    });
    let mut registry = build_registry(&cfg.http_allow_hosts);
    for t in mcp_tools {
        registry.register(t.clone());
    }
    // Skills: build the registry + the 4 tools from the configured roots.
    let (skill_registry, skill_tools) = build_skills(&cfg.skills_dirs, workspace);
    for t in skill_tools {
        registry.register(t);
    }
    // Compose the prompt: keep only presets that actually exist; warn + drop the rest.
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
    // All names in `presets` are known, so compose cannot error here.
    let system_prompt = compose_system_prompt(base_system_prompt, &skill_registry, &presets)
        .unwrap_or_else(|_| base_system_prompt.to_string());

    let loop_ = Arc::new(AgentLoop::new(
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
        },
    ));
    BuiltLoop { loop_, system_prompt, unknown_presets }
}
```

Then update the `make()` test helper (in the `tests` module) to pass the new arg — change its `RuntimeState::new(...)` call to end with `tx, Arc::from(Vec::<Arc<dyn Tool>>::new()), crate::daemon::SYSTEM_PROMPT.to_string())`. Also update the `settings_state_includes_discovered_skills` test from Task 3 the same way (it calls `RuntimeState::new` directly).

- [ ] **Step 4: Rewire `daemon.rs` — pass base prompt + apply prompt per turn**

In `daemon.rs`, add the trait import (with the existing `agent_core` use):

```rust
use agent_core::{ContextManager, WindowContext};
```

(Replace the existing `use agent_core::WindowContext;` line with the above.)

In `run`, the `WindowContext::new(...)` ctx still starts from `params.system_prompt` (now the *base* prompt) — that is fine; the first turn overwrites it with the composed prompt. Change the `RuntimeState::new(...)` call to pass the base prompt as the new final argument:

```rust
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
        params.system_prompt.clone(),
    ));
```

In the `WireBody::UserInput` arm, apply the current composed prompt before running:

```rust
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
```

- [ ] **Step 5: Rewire `main.rs` — seed flags, drop pre-built skills**

In `main.rs`, in the `Cmd::Run` arm: after `base.http_allow_hosts = allow_host;` (line ~93), seed the skill fields from the flags:

```rust
            base.skills_dirs = skills_dir;
            base.active_skills = skill;
```

Delete the skills pre-build block (lines ~112-127: the `build_skills` call, the `all_tools`/`extra_tools` construction, and the `compose_system_prompt` block). Replace the `mcp_tools` → `extra_tools` plumbing so `DaemonParams.mcp_tools` is the **MCP-only** slice and `system_prompt` is the **base** prompt. The resulting `DaemonParams` construction becomes:

```rust
            let params = daemon::DaemonParams {
                ws_url: ws_url(&cfg.worker_url),
                agent_token: cfg.agent_token,
                config: base,
                api_key,
                claude_binary,
                config_path: runtime_config,
                workspace,
                system_prompt: daemon::SYSTEM_PROMPT.to_string(),
                mcp_tools,
            };
```

(`mcp_tools` is the `Arc<[Arc<dyn Tool>]>` already built at lines ~109-111. The `skills_dir` and `skill` bindings are now consumed by the seeding lines above; if the compiler warns either is unused, that means a seeding line is missing.)

- [ ] **Step 6: Build, test, clippy**

Run:
```
source "$HOME/.cargo/env" && cd agent && cargo test -p agent-server && cargo clippy -p agent-server --all-targets -- -D warnings
```
Expected: PASS — the three new unit tests, the Task 3 tests, the existing `apply_*`/`handle_*` tests, and `daemon_roundtrip` all pass; clippy clean.

- [ ] **Step 7: Whole-workspace regression check**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test --workspace && cargo clippy --all-targets -- -D warnings`
Expected: PASS (all crates).

- [ ] **Step 8: Commit**

```bash
git add agent/crates/agent-server/src/runtime.rs agent/crates/agent-server/src/daemon.rs agent/crates/agent-server/src/main.rs
git commit -m "feat(server): live-apply skills_dirs/active_skills via build_loop + per-turn re-system"
```

---

### Task 5: Web Settings UI — skill dirs textarea + discovered-skills checklist

**Files:**
- Modify: `web/src/wire.ts`
- Modify: `web/src/state.ts`
- Modify: `web/src/components/SettingsPanel.tsx`
- Test: `web/src/components/SettingsPanel.test.tsx`

**Interfaces:**
- Consumes: the `settings_state` frame now carries `discovered_skills: { name, description }[]` and `settings.skills_dirs`/`settings.active_skills` (Tasks 1, 3, 4).
- Produces: `RuntimeSettings.skills_dirs`/`active_skills`; `settingsMeta.discoveredSkills`; a "Skills" section in `SettingsPanel`.

- [ ] **Step 1: Write the failing tests (SettingsPanel.test.tsx)**

Update the `base` fixture to include the two new fields (after `preserve_thinking: false,`):

```ts
  skills_dirs: [], active_skills: [],
```

Add a new test block:

```tsx
describe("SettingsPanel skills", () => {
  const meta = { workspace: "/w", apiKeySet: false, hardFloor: [],
    discoveredSkills: [{ name: "greeter", description: "says hi" }] };

  it("checks an active skill and saves it in active_skills", () => {
    const onSave = vi.fn();
    render(<SettingsPanel settings={base} meta={meta} error={null} disabled={false}
      onSave={onSave} onClose={() => {}} />);
    fireEvent.click(screen.getByLabelText(/greeter/));
    fireEvent.click(screen.getByText("Save"));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ active_skills: ["greeter"] }));
  });

  it("round-trips edited skill directories", () => {
    const onSave = vi.fn();
    render(<SettingsPanel settings={base} meta={meta} error={null} disabled={false}
      onSave={onSave} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("Skill directories (one per line)"),
      { target: { value: "/a\n/b" } });
    fireEvent.click(screen.getByText("Save"));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ skills_dirs: ["/a", "/b"] }));
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd web && npx vitest run src/components/SettingsPanel.test.tsx`
Expected: FAIL — `discoveredSkills`/`skills_dirs` type errors, and the "Skill directories" label not found.

- [ ] **Step 3: Extend the wire types (wire.ts)**

In `RuntimeSettings` (after `preserve_thinking: boolean;`), add:

```ts
  skills_dirs: string[];
  active_skills: string[];
```

After the `RuntimeSettings` interface, add:

```ts
export interface DiscoveredSkill { name: string; description: string }
```

In the `Inbound` union, extend the `settings_state` member to carry the new field:

```ts
  | { v: number; session_id: string; kind: "settings_state"; settings: RuntimeSettings; workspace: string; api_key_set: boolean; hard_floor: string[]; discovered_skills: DiscoveredSkill[] }
```

- [ ] **Step 4: Map it into `settingsMeta` (state.ts)**

In `state.ts`, extend the `settingsMeta` type (line ~29):

```ts
  settingsMeta: { workspace: string; apiKeySet: boolean; hardFloor: string[]; discoveredSkills: import("./wire").DiscoveredSkill[] } | null;
```

In `reduceFrame`'s `settings_state` branch (line ~72-76), include the discovered skills:

```ts
  if (frame.kind === "settings_state") {
    return { ...state, settings: frame.settings,
      settingsMeta: { workspace: frame.workspace, apiKeySet: frame.api_key_set,
        hardFloor: frame.hard_floor, discoveredSkills: frame.discovered_skills },
      settingsError: null };
  }
```

- [ ] **Step 5: Add the Skills section (SettingsPanel.tsx)**

In `SettingsPanel.tsx`, extend the `Meta` interface (line 4):

```ts
interface Meta { workspace: string; apiKeySet: boolean; hardFloor: string[]; discoveredSkills: { name: string; description: string }[] }
```

Add local state for the dirs textarea, next to `allow`/`deny` (after line 21):

```tsx
  const [skillsDirs, setSkillsDirs] = useState(toLines(settings.skills_dirs));
```

Update `save` to fold both skill fields into the outbound object:

```tsx
  const save = () => onSave({ ...form, command_allowlist: fromLines(allow),
    command_denylist: fromLines(deny), skills_dirs: fromLines(skillsDirs) });
```

Add a helper to toggle an active skill (after the `set` helper, ~line 24):

```tsx
  const toggleSkill = (name: string) =>
    setForm((f) => ({ ...f, active_skills: f.active_skills.includes(name)
      ? f.active_skills.filter((n) => n !== name)
      : [...f.active_skills, name] }));
```

Insert a new `<section>` before the closing `{meta && (...)}` workspace block (after the "Sampling & thinking" section, ~line 163):

```tsx
        <section className="mb-4 space-y-3">
          <h3 className="text-sm font-semibold text-zinc-300">Skills</h3>
          <div>
            <label className={label} htmlFor="skills_dirs">Skill directories (one per line)</label>
            <textarea id="skills_dirs" rows={3} className={field} value={skillsDirs}
              onChange={(e) => setSkillsDirs(e.target.value)} />
            <p className="mt-1 text-xs text-zinc-500">
              Save directories, then the skills they contain appear below to activate.
            </p>
          </div>
          <div>
            <span className={label}>Active skills</span>
            {(meta?.discoveredSkills ?? []).length === 0 ? (
              <p className="text-xs text-zinc-500">No skills found in the configured directories.</p>
            ) : (
              <ul className="space-y-1">
                {meta!.discoveredSkills.map((s) => (
                  <li key={s.name}>
                    <label className="flex items-start gap-2 text-sm">
                      <input type="checkbox" className="mt-1"
                        checked={form.active_skills.includes(s.name)}
                        onChange={() => toggleSkill(s.name)} />
                      <span><span className="text-zinc-200">{s.name}</span>
                        <span className="block text-xs text-zinc-500">{s.description}</span></span>
                    </label>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </section>
```

> The checkbox's accessible label is the skill `name` (+ description), so `getByLabelText(/greeter/)` matches it.

- [ ] **Step 6: Run the web tests**

Run: `cd web && npx vitest run`
Expected: PASS (the two new skills tests + all existing SettingsPanel/state tests). The existing "preserves all fields" behaviour holds because `save` spreads `...form`.

- [ ] **Step 7: Typecheck + build**

Run: `cd web && npm run build`
Expected: succeeds (tsc + vite build with no type errors).

- [ ] **Step 8: Commit**

```bash
git add web/src/wire.ts web/src/state.ts web/src/components/SettingsPanel.tsx web/src/components/SettingsPanel.test.tsx
git commit -m "feat(web): edit skill dirs + active skills in the Settings panel"
```

---

## Self-Review

**Spec coverage:**
- §3 RuntimeConfig fields + flag seeding → Task 1 (fields/merge/round-trip) + Task 4 Step 5 (daemon `main.rs` seeding). ✓
- §4 core `set_system` → Task 2. ✓
- §5 `build_loop` single skills-build site, returns composed prompt → Task 4 Step 3. ✓
- §5 next-turn `set_system` apply → Task 4 Step 4. ✓
- §5 validation split (strict wire / lenient startup) → Task 4 Step 3 (`apply` rejects `unknown_presets`; `new` tolerates) + tests in Step 1. ✓
- §6 `discovered_skills` read-only in `settings_state`, computed via `scan()` → Task 3. ✓
- §7 web types + Skills section (dirs textarea + checklist) → Task 5. ✓
- §8 testing across all four areas → tests in Tasks 1-5. ✓
- Non-goals (CLI untouched, no agent-skills changes, no new execution) → honoured by Global Constraints; no task touches `agent-cli` or `agent-skills`. ✓

**Placeholder scan:** No TBD/TODO; every code step shows full code. ✓

**Type consistency:** `BuiltLoop { loop_, system_prompt, unknown_presets }` defined in Task 4 Step 3 and consumed in `new`/`apply` in the same step; `DiscoveredSkill { name, description }` defined in Task 3 (Rust) and Task 5 (TS) with matching field names; `current_system_prompt()` defined Task 4 Step 3, used in tests (Step 1) and `daemon.rs` (Step 4); `set_system` defined Task 2, used in `daemon.rs` Task 4 Step 4. `RuntimeState::new`'s new final param `base_system_prompt: String` is added in Task 4 and back-filled at the one direct call site introduced in Task 3 (noted there). ✓

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-06-23-skills-runtime-config-persistence.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

**Which approach?**
