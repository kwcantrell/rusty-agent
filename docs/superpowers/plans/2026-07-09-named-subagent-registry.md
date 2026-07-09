# Named Sub-Agent Registry (Phase 3B-1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a config-defined, named sub-agent registry so an operator can declare purpose-built sub-agents (own system prompt, tool allowlist, routed model, runaway bound) that the model selects by name via `dispatch_agent`, while the default ad-hoc path stays byte-identical.

**Architecture:** `SubAgentSpec` is a serde config type on `RuntimeConfig.named_subagents`. Because `agent-core` cannot call `build_routed_model` (dependency direction — it lives in `agent-runtime-config`), `assemble_loop` resolves each spec into a `ResolvedSubAgent` (pre-built model/protocol/window knobs) and threads an `Arc<SubAgentRegistry>` into `DispatchDeps`. `DispatchAgentTool` reads the registry to (a) add a `subagent_type` enum to its schema **only when non-empty**, and (b) on a named selection, swap in the spec's system prompt (+ the always-composed `SUBAGENT_PREAMBLE`), tool allowlist, model/protocol/window, and an optional child `ToolCallLimit`.

**Tech Stack:** Rust (two Cargo workspaces; all work here is in the `agent/` workspace), `serde`/`serde_json`, `async_trait`, `tokio`. Crates touched: `agent-runtime-config` (config + assembly + eval/soak tests) and `agent-core` (dispatch).

## Global Constraints

- **Workspace:** all crates are in `agent/`; run cargo from `agent/`. `cargo test -p agent-core` / `-p agent-runtime-config`.
- **Conventional commits:** `type(scope): summary` (e.g. `feat(core): …`, `feat(config): …`).
- **Do NOT push.** Commit only. Branch off `main` first (see Task 0).
- **`file:line` anchors below are orientation only** — locate quoted code by content before editing (they drift as earlier tasks insert code).
- **Byte-identical invariant (spec §3 invariant 1):** the `general-purpose` path (no `subagent_type`, or `subagent_type == "general-purpose"`) must produce identical child construction to `cb6ddf0`. The `subagent_type` schema property appears ONLY when the registry is non-empty.
- **Reserved-field rejection (spec §2.7):** `permissions` / `response_format` / `middleware` / `skills` are inert `SubAgentSpec` fields; any non-null value is a config-validation error in 3B-1.
- **Constants (this plan):** `MAX_SUBAGENT_SYSTEM_PROMPT_CHARS = 20_000`; `MAX_SUBAGENT_TOOL_CALLS = 1_000`.
- **Plan-time resolutions of spec ambiguities:** (1) duplicate spec names → validation error (fail-fast), superseding the spec §2.2 "last-wins/warn" phrasing (that pattern is for the programmatic `ToolRegistry`; a config file wants fail-fast). (2) M3's "model constrained to the declared model set" is implemented concretely as: a spec's `model` may set only `model`/`protocol`/`context_limit`/`max_tokens`; `backend`/`base_url`/`claude_binary` must be `None`.

**Spec:** `docs/superpowers/specs/2026-07-09-named-subagent-registry-design.md` (PLAN-READY, gate-approved trimmed scope).

**Test-harness reference (USE THESE EXACT NAMES — verified at `cb6ddf0`; the earlier drafts used placeholder names that do not exist):**
- `dispatch.rs` test module (`agent-core`): build deps with `exec_deps(model: ScriptedModel, max_turns: usize) -> DispatchDeps` and a ctx with `exec_ctx() -> ToolCtx`. Models/protocols are inline: `ScriptedModel::new(vec![ … ])` and `Arc::new(PassthroughProtocol)`, both imported via `use crate::testkit::{AlwaysApprove, PassthroughProtocol, Scripted, ScriptedModel};`. There is NO `test_deps`/`test_ctx`/`test_model`/`test_protocol` — wherever this plan's code blocks say those, substitute `exec_deps(ScriptedModel::new(vec![]), 1)` / `exec_ctx()` / `Arc::new(ScriptedModel::new(vec![…]))` / `Arc::new(PassthroughProtocol)`. **`exec_deps` is itself a `DispatchDeps` literal — Task B1 must add the new `subagents` field to it.**
- `runtime_config.rs` test module: parse a partial config with `base().merge(serde_json::from_str::<PartialRuntimeConfig>(json).unwrap())` (see `runtime_config.rs:796-802`). `PartialRuntimeConfig` is private but the test module is in-file, so it is in scope. There is NO `from_partial_json`.
- A capturing child model for asserting the child's system prompt exists near `dispatch.rs:1024` — reuse it (or a `ScriptedModel` that records its request) rather than inventing a new harness.

---

## Task 0: Branch

- [ ] **Step 1: Create the feature branch**

Run:
```bash
cd /home/kalen/rust-agent-runtime && git checkout main && git checkout -b feature/named-subagent-registry
```
Expected: `Switched to a new branch 'feature/named-subagent-registry'`

---

## Wave A — Registry data model + config + standalone eval fix

### Task A1: `SubAgentSpec` config type + `named_subagents` field

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs` (define `SubAgentSpec` near `ModelRef` ~L22-46; add field to `RuntimeConfig` struct ~L115; add default in `RuntimeConfig::default()` impl ~L308-313; add to the partial-override struct ~L196-201 and its apply block ~L531-547)

**Interfaces:**
- Produces: `pub struct SubAgentSpec { name, description, system_prompt, tools: Option<Vec<String>>, model: Option<ModelRef>, tool_call_limit: Option<usize>, permissions/response_format: Option<serde_json::Value>, middleware/skills: Option<Vec<String>> }`; `RuntimeConfig.named_subagents: Vec<SubAgentSpec>`; consts `MAX_SUBAGENT_SYSTEM_PROMPT_CHARS`, `MAX_SUBAGENT_TOOL_CALLS`.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `runtime_config.rs`:
```rust
#[test]
fn named_subagents_default_empty_and_roundtrip() {
    let c = RuntimeConfig::default();
    assert!(c.named_subagents.is_empty());

    let json = r#"{"named_subagents":[{"name":"reviewer","description":"reviews code","system_prompt":"You review Rust.","tools":["read_file","grep"]}]}"#;
    let parsed = base().merge(
        serde_json::from_str::<PartialRuntimeConfig>(json).unwrap(),
    );
    assert_eq!(parsed.named_subagents.len(), 1);
    let s = &parsed.named_subagents[0];
    assert_eq!(s.name, "reviewer");
    assert_eq!(s.tools.as_deref(), Some(&["read_file".to_string(), "grep".to_string()][..]));
    assert!(s.model.is_none() && s.tool_call_limit.is_none() && s.permissions.is_none());
}
```
(Uses the `base().merge(from_str::<PartialRuntimeConfig>(…))` pattern from `runtime_config.rs:796-802` — see the Test-harness reference.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-runtime-config named_subagents_default_empty_and_roundtrip`
Expected: FAIL to compile (`no field named_subagents`).

- [ ] **Step 3: Define `SubAgentSpec` + constants**

Immediately after the `impl ModelRef { … }` block (~L68) in `runtime_config.rs`, add:
```rust
/// Upper bound on a named sub-agent's `system_prompt` (prepended to every child
/// turn; mirrors `MAX_ROLE_CHARS` for the ad-hoc `role` path). Panel fix M3.
pub const MAX_SUBAGENT_SYSTEM_PROMPT_CHARS: usize = 20_000;
/// Upper bound on a named sub-agent's `tool_call_limit` (panel fix M3; the field
/// exists to BOUND runaway, so it must itself be bounded).
pub const MAX_SUBAGENT_TOOL_CALLS: usize = 1_000;

/// A config-declared, named sub-agent (deepagents `SubAgent`, minimal-viable
/// subset; spec §2.1). Reserved fields are inert in 3B-1 (validation rejects
/// any non-null value; see `validate`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubAgentSpec {
    /// Unique registry key; the model selects by this via `dispatch_agent`'s
    /// `subagent_type`. Reserved: `"general-purpose"` is not overridable.
    pub name: String,
    /// "When to use me" — surfaced as the `subagent_type` enum value's doc.
    pub description: String,
    /// Replaces the parent-derived child prompt; the `SUBAGENT_PREAMBLE` is
    /// still always composed in (see assembly).
    pub system_prompt: String,
    /// Tool allowlist over the parent snapshot; `None` = full snapshot.
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    /// Routed model override; `None` = the child default (`subagent_model`).
    /// May set only `model`/`protocol`/`context_limit`/`max_tokens` (validated).
    #[serde(default)]
    pub model: Option<ModelRef>,
    /// Child `ToolCallLimit` cap (3A residual E4); bounded `1..=MAX_SUBAGENT_TOOL_CALLS`.
    #[serde(default)]
    pub tool_call_limit: Option<usize>,

    // ---- RESERVED (inert in 3B-1; validation rejects any non-null value) ----
    /// → 3B-1c (per-sub-agent permissions).
    #[serde(default)]
    pub permissions: Option<serde_json::Value>,
    /// → 3B-1b (structured-response handoff).
    #[serde(default)]
    pub response_format: Option<serde_json::Value>,
    /// Dropped (no committed follow-on); see spec §9.
    #[serde(default)]
    pub middleware: Option<Vec<String>>,
    /// Dropped (no committed follow-on); see spec §9.
    #[serde(default)]
    pub skills: Option<Vec<String>>,
}
```

- [ ] **Step 4: Add the field to `RuntimeConfig`, its default, and the partial-override path**

In the `RuntimeConfig` struct (after `subagent_max_depth` ~L115), add:
```rust
    /// Config-declared named sub-agents (spec 2026-07-09-named-subagent-registry).
    #[serde(default)]
    pub named_subagents: Vec<SubAgentSpec>,
```
In the `RuntimeConfig::default()` body (with the other `subagent_*` defaults ~L308-313), add:
```rust
            named_subagents: Vec::new(),
```
In the partial-override struct (the `Option`-field mirror ~L196-201), add:
```rust
    #[serde(default)]
    named_subagents: Option<Vec<SubAgentSpec>>,
```
In the apply block (~L531-547, where each `if let Some(v) = p.subagent_* { … }` lives), add:
```rust
        if let Some(v) = p.named_subagents {
            self.named_subagents = v;
        }
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd agent && cargo test -p agent-runtime-config named_subagents_default_empty_and_roundtrip`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-runtime-config/src/runtime_config.rs
git commit -m "feat(config): SubAgentSpec + named_subagents field (Phase 3B-1, Wave A)"
```

### Task A2: `named_subagents` validation

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs` (`RuntimeConfig::validate` ~L360-445; append the named-subagents rules before the final `Ok(())`)

**Interfaces:**
- Consumes: `SubAgentSpec`, `MAX_SUBAGENT_SYSTEM_PROMPT_CHARS`, `MAX_SUBAGENT_TOOL_CALLS` (Task A1).
- Produces: `RuntimeConfig::validate()` rejects invalid `named_subagents` with a spec-naming message. (NOTE: tool-name resolution is validated later at assembly — Task B2 — because it needs the parent tool snapshot.)

- [ ] **Step 1: Write the failing tests**

Add to the test module:
```rust
fn cfg_with(specs: Vec<SubAgentSpec>) -> RuntimeConfig {
    let mut c = RuntimeConfig::default();
    c.named_subagents = specs;
    c
}
fn spec(name: &str) -> SubAgentSpec {
    SubAgentSpec {
        name: name.into(),
        description: "d".into(),
        system_prompt: "p".into(),
        tools: None, model: None, tool_call_limit: None,
        permissions: None, response_format: None, middleware: None, skills: None,
    }
}

#[test]
fn validate_rejects_reserved_general_purpose_name() {
    assert!(cfg_with(vec![spec("general-purpose")]).validate().is_err());
}
#[test]
fn validate_rejects_duplicate_names() {
    assert!(cfg_with(vec![spec("a"), spec("a")]).validate().is_err());
}
#[test]
fn validate_rejects_empty_required_fields() {
    let mut s = spec("a"); s.description = "".into();
    assert!(cfg_with(vec![s]).validate().is_err());
    let mut s = spec("a"); s.system_prompt = "".into();
    assert!(cfg_with(vec![s]).validate().is_err());
    assert!(cfg_with(vec![spec("")]).validate().is_err());
}
#[test]
fn validate_rejects_oversize_system_prompt_and_bad_tool_call_limit() {
    let mut s = spec("a"); s.system_prompt = "x".repeat(MAX_SUBAGENT_SYSTEM_PROMPT_CHARS + 1);
    assert!(cfg_with(vec![s]).validate().is_err());
    let mut s = spec("a"); s.tool_call_limit = Some(0);
    assert!(cfg_with(vec![s]).validate().is_err());
    let mut s = spec("a"); s.tool_call_limit = Some(MAX_SUBAGENT_TOOL_CALLS + 1);
    assert!(cfg_with(vec![s]).validate().is_err());
    let mut s = spec("a"); s.tool_call_limit = Some(MAX_SUBAGENT_TOOL_CALLS);
    assert!(cfg_with(vec![s]).validate().is_ok());
}
#[test]
fn validate_rejects_reserved_fields_and_model_endpoint_override() {
    let mut s = spec("a"); s.permissions = Some(serde_json::json!({}));
    assert!(cfg_with(vec![s]).validate().is_err());
    let mut s = spec("a"); s.response_format = Some(serde_json::json!({}));
    assert!(cfg_with(vec![s]).validate().is_err());
    let mut s = spec("a"); s.middleware = Some(vec!["memory_recall".into()]);
    assert!(cfg_with(vec![s]).validate().is_err());
    let mut s = spec("a"); s.skills = Some(vec!["x".into()]);
    assert!(cfg_with(vec![s]).validate().is_err());
    // model may set model/protocol/context_limit/max_tokens, NOT the endpoint.
    let mut s = spec("a");
    s.model = Some(ModelRef { model: Some("haiku".into()), ..Default::default() });
    assert!(cfg_with(vec![s]).validate().is_ok());
    let mut s = spec("a");
    s.model = Some(ModelRef { base_url: Some("http://evil".into()), ..Default::default() });
    assert!(cfg_with(vec![s]).validate().is_err());
    let mut s = spec("a");
    s.model = Some(ModelRef { backend: Some("openai".into()), ..Default::default() });
    assert!(cfg_with(vec![s]).validate().is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-runtime-config validate_rejects_`
Expected: the new `validate_rejects_*` tests FAIL (validation currently accepts everything).

- [ ] **Step 3: Add the validation rules**

In `RuntimeConfig::validate`, immediately before the final `Ok(())`, insert:
```rust
        // ---- Named sub-agents (spec §2.7) ----
        let mut seen = std::collections::HashSet::new();
        for s in &self.named_subagents {
            if s.name.trim().is_empty() {
                return Err("named_subagents: name must be non-empty".into());
            }
            if s.name == "general-purpose" {
                return Err(
                    "named_subagents: 'general-purpose' is a reserved name and cannot be redefined"
                        .into(),
                );
            }
            if !seen.insert(s.name.clone()) {
                return Err(format!("named_subagents: duplicate name '{}'", s.name));
            }
            if s.description.trim().is_empty() {
                return Err(format!("named_subagents['{}']: description must be non-empty", s.name));
            }
            if s.system_prompt.trim().is_empty() {
                return Err(format!("named_subagents['{}']: system_prompt must be non-empty", s.name));
            }
            if s.system_prompt.chars().count() > MAX_SUBAGENT_SYSTEM_PROMPT_CHARS {
                return Err(format!(
                    "named_subagents['{}']: system_prompt exceeds {MAX_SUBAGENT_SYSTEM_PROMPT_CHARS} chars",
                    s.name
                ));
            }
            if let Some(n) = s.tool_call_limit {
                if !(1..=MAX_SUBAGENT_TOOL_CALLS).contains(&n) {
                    return Err(format!(
                        "named_subagents['{}']: tool_call_limit must be 1..={MAX_SUBAGENT_TOOL_CALLS}",
                        s.name
                    ));
                }
            }
            if let Some(cl) = s.model.as_ref().and_then(|m| m.context_limit) {
                if cl < 1024 {
                    return Err(format!("named_subagents['{}']: model.context_limit must be >= 1024", s.name));
                }
            }
            // M3: a spec may re-point the model NAME/protocol/window, never the
            // endpoint — child inference stays on the config's declared backend.
            if let Some(m) = &s.model {
                if m.backend.is_some() || m.base_url.is_some() || m.claude_binary.is_some() {
                    return Err(format!(
                        "named_subagents['{}']: model may set only model/protocol/context_limit/max_tokens (not backend/base_url/claude_binary)",
                        s.name
                    ));
                }
            }
            // Reserved fields are inert in 3B-1 (spec §2.7 item 6).
            if s.permissions.is_some() || s.response_format.is_some()
                || s.middleware.is_some() || s.skills.is_some()
            {
                return Err(format!(
                    "named_subagents['{}']: permissions/response_format/middleware/skills are not supported in 3B-1 (see 3B-1b/3B-1c)",
                    s.name
                ));
            }
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-runtime-config validate_`
Expected: PASS (new tests + the existing `validate_*` tests still green).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/runtime_config.rs
git commit -m "feat(config): validate named_subagents (reserved name/fields, ceilings, endpoint lock) (3B-1, Wave A)"
```

### Task A3: eval/soak `.with_todos` standalone fix (3A residual)

**Files:**
- Modify: `agent/crates/agent-runtime-config/tests/eval_context.rs:305-312` (the `CuratedContext::new(...)` chain)
- Modify: `agent/crates/agent-runtime-config/tests/soak_live.rs:267-272` (the `CuratedContext::new(...)` chain)

**Interfaces:**
- Consumes: the `todos` handle already minted in both files (`eval_context.rs:277`, `soak_live.rs:226`) and passed to `LoopParts.todos`.
- Produces: the harness-local `CuratedContext` now renders the pinned todos block (the assembled loop already receives the handle; this closes the harness-context gap).

- [ ] **Step 1: Add `.with_todos(...)` to the eval harness context chain**

In `eval_context.rs`, change the chain at ~L305-312 to add `.with_todos(todos.clone())` (place it right after `CuratedContext::new(...)` and before the other `.with_*` calls):
```rust
        let mut ctx = CuratedContext::new(
            Message::system(built.system_prompt),
            artifacts.clone(),
            flag,
        )
        .with_todos(todos.clone())
        .with_recall_budget(cc.recall_budget)
        .with_offload_config(cc.offload_config())
        .with_high_water_pct(cc.high_water_pct);
```

- [ ] **Step 2: Add `.with_todos(...)` to the soak harness context chain**

In `soak_live.rs`, change the chain at ~L267-272:
```rust
    let mut ctx = CuratedContext::new(
        Message::system(built.system_prompt),
        artifacts.clone(),
        flag,
    )
    .with_todos(todos.clone())
    .with_recall_budget(256);
```

- [ ] **Step 3: Verify both test files still compile**

Run: `cd agent && cargo test -p agent-runtime-config --test eval_context --no-run && cargo test -p agent-runtime-config --test soak_live --no-run`
Expected: both compile (these are live-model harnesses, gated by env vars; compiling is the check). If `todos` was moved rather than cloned into `LoopParts`, adjust the earlier `LoopParts { todos: todos.clone(), … }` to clone so the handle is still owned here — verify by content.

- [ ] **Step 4: Commit**

```bash
git add agent/crates/agent-runtime-config/tests/eval_context.rs agent/crates/agent-runtime-config/tests/soak_live.rs
git commit -m "fix(config): wire .with_todos in eval/soak harness contexts (3A residual; 3B-1 Wave A)"
```

---

## Wave B — Dispatch integration

### Task B1: `ResolvedSubAgent` + `SubAgentRegistry` in agent-core; thread into `DispatchDeps` (empty ⇒ byte-identical)

**Files:**
- Modify: `agent/crates/agent-core/src/dispatch.rs` (add the two types near the top after the constants ~L30; add a field to `DispatchDeps` ~L262)
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (add `subagents: Arc::new(SubAgentRegistry::default())` to the `DispatchDeps` literal ~L373-396)
- Modify: any test that constructs a `DispatchDeps` literal (grep in Step 3)

**Interfaces:**
- Produces: `pub struct ResolvedSubAgent { description: String, system_prompt: String, tools: Option<Vec<String>>, model: Arc<dyn ModelClient>, protocol: Arc<dyn ToolCallProtocol>, model_limit: Option<usize>, max_tokens: Option<u32>, tool_call_limit: Option<usize> }`; `pub struct SubAgentRegistry { map: HashMap<String, ResolvedSubAgent> }` with `pub fn get(&self, name: &str) -> Option<&ResolvedSubAgent>`, `pub fn is_empty(&self) -> bool`, `pub fn names(&self) -> Vec<&str>`, `pub fn schema_hints(&self) -> Vec<(String, String)>` (name, description), `Default`, and `pub fn from_map(map: HashMap<String, ResolvedSubAgent>) -> Self`.
- Produces: `DispatchDeps.subagents: Arc<SubAgentRegistry>`.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `dispatch.rs`:
```rust
#[test]
fn empty_registry_omits_subagent_type_from_schema() {
    // A DispatchAgentTool built with an empty registry has a schema byte-identical
    // to 3A (no `subagent_type` property).
    let tool = DispatchAgentTool::new(exec_deps(ScriptedModel::new(vec![]), 1));
    let schema = tool.schema();
    let props = schema.parameters.get("properties").unwrap();
    assert!(props.get("subagent_type").is_none(), "empty registry must not add subagent_type");
    assert!(props.get("prompt").is_some());
}
```
NOTE: `exec_deps(model, max_turns)` is the real `DispatchDeps` constructor (see the Test-harness reference). Step 3/4 below adds the new `subagents` field to it. `ScriptedModel` is imported via `use crate::testkit::{…, ScriptedModel};` already present in the test module.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-core empty_registry_omits_subagent_type_from_schema`
Expected: FAIL to compile (`no field subagents` / `SubAgentRegistry` undefined).

- [ ] **Step 3: Define the types and thread the field**

In `dispatch.rs`, after `next_dispatch_n` (~L30), add:
```rust
/// A `SubAgentSpec` resolved at assembly into everything the dispatch tool needs
/// to spawn a named child (models are pre-built here because agent-core cannot
/// call `build_routed_model`). Spec §2.1/§2.4/§2.5.
pub struct ResolvedSubAgent {
    pub description: String,
    /// Already includes `SUBAGENT_PREAMBLE` (composed at assembly).
    pub system_prompt: String,
    pub tools: Option<Vec<String>>,
    pub model: Arc<dyn ModelClient>,
    pub protocol: Arc<dyn ToolCallProtocol>,
    pub model_limit: Option<usize>,
    pub max_tokens: Option<u32>,
    pub tool_call_limit: Option<usize>,
}

/// Dispatch-facing named sub-agent registry (spec §2.2). `general-purpose` is
/// implicit (the default ad-hoc path) and never stored here.
#[derive(Default)]
pub struct SubAgentRegistry {
    map: std::collections::HashMap<String, ResolvedSubAgent>,
}

impl SubAgentRegistry {
    pub fn from_map(map: std::collections::HashMap<String, ResolvedSubAgent>) -> Self {
        Self { map }
    }
    pub fn get(&self, name: &str) -> Option<&ResolvedSubAgent> {
        self.map.get(name)
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    pub fn names(&self) -> Vec<&str> {
        self.map.keys().map(String::as_str).collect()
    }
    /// `(name, description)` pairs for the `subagent_type` enum docs.
    pub fn schema_hints(&self) -> Vec<(String, String)> {
        self.map
            .iter()
            .map(|(n, r)| (n.clone(), r.description.clone()))
            .collect()
    }
}
```
Add `ModelClient` and `ToolCallProtocol` to the `agent_model::{…}` use at the top of the file if not already imported (they are: `use agent_model::{Message, ModelClient, StopReason, ToolCallProtocol};` — already present).

Add the field to `DispatchDeps` (after `description_overrides` ~L262):
```rust
    /// Named sub-agent registry (spec §2.2). Empty ⇒ only `general-purpose`
    /// exists and the tool schema is byte-identical to 3A.
    pub subagents: Arc<SubAgentRegistry>,
```
The field MUST stay `Arc<SubAgentRegistry>` (not a by-value `SubAgentRegistry`) so `DispatchDeps` keeps its `#[derive(Clone)]` — `ResolvedSubAgent`/`SubAgentRegistry` are intentionally NOT `Clone`. The nested-dispatch clone (`dispatch.rs:464`) then shares the same immutable registry Arc with grandchildren (intended transitive semantics).

- [ ] **Step 4: Fix every `DispatchDeps` construction site**

Run: `cd agent && grep -rn "DispatchDeps {" crates/`
For EACH literal found (the real one in `assemble.rs` ~L374, plus every test helper in `dispatch.rs`), add:
```rust
                subagents: Arc::new(SubAgentRegistry::default()),
```
In `assemble.rs`, import the type: ensure `use agent_core::{…, SubAgentRegistry};` (or reference it as `agent_core::SubAgentRegistry` inline, matching the file's existing `agent_core::` usage style).

- [ ] **Step 5: Add the empty-registry guard to `schema()`**

The `schema()` method (~L312) currently returns a static `parameters` object. Wrap the `subagent_type` addition (added in Task B3) behind `if !self.deps.subagents.is_empty()`. For THIS task, just make the test pass by leaving `schema()` unchanged (empty registry already omits `subagent_type` because the property doesn't exist yet). The guard itself is added in B3.

- [ ] **Step 6: Run the test + full crate tests**

Run: `cd agent && cargo test -p agent-core empty_registry_omits_subagent_type_from_schema && cargo test -p agent-core dispatch`
Expected: PASS; all existing dispatch tests still green (field is additive, empty default).

- [ ] **Step 7: Commit**

```bash
git add agent/crates/agent-core/src/dispatch.rs agent/crates/agent-runtime-config/src/assemble.rs
git commit -m "feat(core): ResolvedSubAgent + SubAgentRegistry threaded into DispatchDeps (empty=byte-identical) (3B-1 Wave B)"
```

### Task B2: Assembly resolves `named_subagents` → `SubAgentRegistry`

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (inside the `if let Some(child_base) = child_base { … }` block ~L335-397: build the registry from `cfg.named_subagents` before constructing `DispatchDeps`, and pass it in place of the empty default)

**Interfaces:**
- Consumes: `cfg.named_subagents: Vec<SubAgentSpec>`, `child_base: Vec<Arc<dyn Tool>>`, `crate::build_routed_model`, `child_protocol` / `child_model` / `child_config` already computed in this block; `agent_core::{ResolvedSubAgent, SubAgentRegistry, SUBAGENT_PREAMBLE}`.
- Produces: a populated `Arc<SubAgentRegistry>` in the `DispatchDeps.subagents` field; `#[cfg(test)] BuiltLoop.subagent_registry`.

> **DESIGN NOTE (plan-review BLOCKER resolution):** `assemble_loop` stays **infallible** — do NOT add a `_checked`/`Result` variant. An earlier draft validated a spec's `tools` against the child snapshot here and returned `Err`, which would `.expect()`-panic the *lenient-boot* server daemon (`agent-server/src/runtime.rs:71-72` deliberately skips `validate()` so it always boots). It is unnecessary: a named spec's `tools` becomes the child `allow` allowlist, and the **existing execute-time allowlist check** (`dispatch.rs:410-425`) already rejects an unknown tool with a clean `ToolError::InvalidArgs` — symmetric with the ad-hoc `tools` arg today. So there is NO assembly-time tool validation; the unknown-tool case is pinned as a dispatch-time test in Task B4.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `assemble.rs` (mirror the existing `parts(...)`/config helpers used by e.g. `child_base_snapshot_includes_memory_tools_when_enabled`):
```rust
#[test]
fn named_subagent_resolves_into_dispatch_registry() {
    let ws = tempdir();
    let mut c = RuntimeConfig::default();
    c.named_subagents = vec![agent_runtime_config::SubAgentSpec {
        name: "reviewer".into(),
        description: "reviews code".into(),
        system_prompt: "You review Rust.".into(),
        tools: None, model: None, tool_call_limit: Some(5),
        permissions: None, response_format: None, middleware: None, skills: None,
    }];
    let built = assemble_loop(&c, parts(ws.path().into(), vec![]));
    let reg = built.subagent_registry.expect("registry present when subagents on");
    let r = reg.get("reviewer").expect("reviewer resolved");
    assert!(r.system_prompt.contains("You review Rust."));
    assert!(r.system_prompt.contains(agent_core::SUBAGENT_PREAMBLE));
    assert_eq!(r.tool_call_limit, Some(5));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-runtime-config named_subagent_resolves`
Expected: FAIL to compile (`subagent_registry` field missing on `BuiltLoop`).

- [ ] **Step 3a: Add the `#[cfg(test)]` registry field + mut binding (mirror `subagent_model_routed`)**

Add to the `BuiltLoop` struct (near `assemble.rs:60-79`, with the other `#[cfg(test)]` fields):
```rust
    #[cfg(test)]
    pub subagent_registry: Option<std::sync::Arc<agent_core::SubAgentRegistry>>,
```
Before the `if let Some(child_base)` block (next to `#[cfg(test)] let mut subagent_model_routed` ~L332), add:
```rust
    #[cfg(test)]
    let mut subagent_registry: Option<std::sync::Arc<agent_core::SubAgentRegistry>> = None;
```
And add `subagent_registry,` to the `BuiltLoop { ... }` struct literal at the bottom (~L449-464), alongside the other `#[cfg(test)]` fields (stays `None` when subagents are disabled).

- [ ] **Step 3b: Build the registry in the dispatch block (infallible)**

Inside the `if let Some(child_base) = child_base` block, AFTER `child_model`/`child_protocol`/`child_config` are computed and BEFORE `registry.register(Arc::new(agent_core::DispatchAgentTool::new(...)))`, insert:
```rust
        // Resolve config specs into dispatch-facing ResolvedSubAgent (spec 2.2/2.4).
        // Infallible: unknown-tool refs surface at dispatch time (see design note).
        let mut resolved: std::collections::HashMap<String, agent_core::ResolvedSubAgent> =
            std::collections::HashMap::new();
        for spec in &cfg.named_subagents {
            // Model: None → inherit the child default (child_model/child_protocol
            // + child_config knobs). Some → build a routed model on the SAME
            // endpoint (validation guarantees no backend/base_url override).
            let (s_model, s_protocol, s_model_limit, s_max_tokens) = match &spec.model {
                None => (child_model.clone(), child_protocol.clone(), None, None),
                Some(r) => (
                    crate::build_routed_model(cfg, r, &parts.claude_binary, parts.api_key.clone()),
                    pick_protocol(&child_protocol_name(cfg, Some(r))),
                    r.context_limit,
                    r.max_tokens,
                ),
            };
            resolved.insert(
                spec.name.clone(),
                agent_core::ResolvedSubAgent {
                    description: spec.description.clone(),
                    system_prompt: format!("{}\n\n{}", spec.system_prompt, agent_core::SUBAGENT_PREAMBLE),
                    tools: spec.tools.clone(),
                    model: s_model,
                    protocol: s_protocol,
                    model_limit: s_model_limit,
                    max_tokens: s_max_tokens,
                    tool_call_limit: spec.tool_call_limit,
                },
            );
        }
        let subagents_reg = Arc::new(agent_core::SubAgentRegistry::from_map(resolved));
        #[cfg(test)]
        {
            subagent_registry = Some(subagents_reg.clone());
        }
```
Then in the `DispatchDeps { ... }` literal, replace the empty default from Task B1 with:
```rust
                subagents: subagents_reg.clone(),
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd agent && cargo test -p agent-runtime-config named_subagent_resolves`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/assemble.rs
git commit -m "feat(config): resolve named_subagents into dispatch SubAgentRegistry (per-spec model) (3B-1 Wave B)"
```

### Task B3: `subagent_type` schema enum (fix M1) + arg parse + unknown-type error

**Files:**
- Modify: `agent/crates/agent-core/src/dispatch.rs` (`schema()` ~L312-336: add the `subagent_type` enum when the registry is non-empty; `execute()` ~L364: parse `subagent_type`, resolve or error)

**Interfaces:**
- Consumes: `SubAgentRegistry::{is_empty, schema_hints, names, get}` (Task B1).
- Produces: a `subagent_type` enum in the schema (present only when the registry is non-empty); a resolved `Option<&ResolvedSubAgent>` local in `execute` (consumed by Task B4); an `InvalidArgs` error for unknown types.

- [ ] **Step 1: Write the failing tests**

Add to the dispatch test module. `one_agent_registry()` builds a one-entry registry using the real testkit types (`ScriptedModel`/`PassthroughProtocol` — see the Test-harness reference):
```rust
fn one_agent_registry() -> Arc<SubAgentRegistry> {
    let mut m = std::collections::HashMap::new();
    m.insert("reviewer".to_string(), ResolvedSubAgent {
        description: "reviews code".into(),
        system_prompt: format!("You review.\n\n{}", SUBAGENT_PREAMBLE),
        tools: None,
        model: Arc::new(ScriptedModel::new(vec![])),
        protocol: Arc::new(PassthroughProtocol),
        model_limit: None, max_tokens: None, tool_call_limit: Some(3),
    });
    Arc::new(SubAgentRegistry::from_map(m))
}

#[test]
fn nonempty_registry_adds_subagent_type_enum_with_descriptions() {
    let mut deps = exec_deps(ScriptedModel::new(vec![]), 1);
    deps.subagents = one_agent_registry();
    let tool = DispatchAgentTool::new(deps);
    let schema = tool.schema();
    let st = schema.parameters["properties"]["subagent_type"].clone();
    let variants: Vec<String> = serde_json::from_value(st["enum"].clone()).unwrap();
    assert!(variants.contains(&"general-purpose".to_string()));
    assert!(variants.contains(&"reviewer".to_string()));
    // description mentions the registered agent's purpose (M1 discovery)
    assert!(st["description"].as_str().unwrap().contains("reviews code"));
}

#[tokio::test]
async fn unknown_subagent_type_is_invalid_args() {
    let mut deps = exec_deps(ScriptedModel::new(vec![]), 1);
    deps.subagents = one_agent_registry();
    let tool = DispatchAgentTool::new(deps);
    let err = tool.execute(
        serde_json::json!({"prompt":"hi","subagent_type":"nope"}),
        &exec_ctx(),
    ).await.unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(_)));
}
```
(Uses the real `exec_deps`/`exec_ctx`/`ScriptedModel`/`PassthroughProtocol` helpers — see the Test-harness reference.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-core subagent_type`
Expected: FAIL (`subagent_type` absent from schema; unknown type not yet handled).

- [ ] **Step 3: Add the enum to `schema()`**

Rewrite `schema()` to conditionally add the property. Replace the static `parameters` construction with:
```rust
    fn schema(&self) -> ToolSchema {
        let mut properties = serde_json::json!({
            "prompt": {
                "type": "string",
                "description": "The complete, self-contained task for the sub-agent: goal, relevant paths/facts, and what to return."
            },
            "tools": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Optional allowlist restricting which tools the sub-agent may use (default: all). For focus, not safety — permissions are inherited either way. The child's context tool (context_compact) is always available. Include dispatch_agent to let the sub-agent dispatch its own (only meaningful when nesting depth allows); the restriction applies transitively to its children. Ignored when subagent_type names a registered sub-agent."
            },
            "role": {
                "type": "string",
                "description": "Optional persona/role instructions injected into the sub-agent's system prompt (stronger steering than putting them in the prompt). Max 2000 characters. Ignored when subagent_type names a registered sub-agent."
            }
        });
        // Fix M1: expose registered sub-agents as a typed enum so the model can
        // ROUTE to the right one. Present ONLY when the registry is non-empty, so
        // an empty registry keeps the schema byte-identical to 3A.
        if !self.deps.subagents.is_empty() {
            let mut hints = self.deps.subagents.schema_hints();
            hints.sort_by(|a, b| a.0.cmp(&b.0));
            let mut variants: Vec<String> = vec!["general-purpose".into()];
            let mut doc = String::from(
                "Which sub-agent to dispatch. 'general-purpose' inherits your tools/model and honors `role`/`tools`. Registered sub-agents (use their own prompt/tools/model, and IGNORE `role`/`tools`): ",
            );
            for (i, (name, desc)) in hints.iter().enumerate() {
                variants.push(name.clone());
                if i > 0 {
                    doc.push_str("; ");
                }
                doc.push_str(&format!("{name} — {desc}"));
            }
            properties["subagent_type"] = serde_json::json!({
                "type": "string",
                "enum": variants,
                "default": "general-purpose",
                "description": doc,
            });
        }
        ToolSchema {
            name: "dispatch_agent".into(),
            description: self.description().into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": properties,
                "required": ["prompt"]
            }),
        }
    }
```

- [ ] **Step 4: Parse `subagent_type` and resolve in `execute()`**

In `execute()`, after the `prompt` is parsed and before the `role`/`tools` parsing (~L371), add:
```rust
        let subagent_type = args
            .get("subagent_type")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| "general-purpose".to_string());
        let resolved: Option<&ResolvedSubAgent> = if subagent_type == "general-purpose" {
            None
        } else {
            match self.deps.subagents.get(&subagent_type) {
                Some(r) => Some(r),
                None => {
                    let mut names: Vec<&str> = self.deps.subagents.names();
                    names.sort_unstable();
                    return Err(ToolError::InvalidArgs(format!(
                        "unknown subagent_type '{subagent_type}'; registered: general-purpose, {}",
                        names.join(", ")
                    )));
                }
            }
        };
```
`resolved` is consumed by Task B4. (It compiles as an unused binding warning until B4 — acceptable within the same wave; if the reviewer runs `-D warnings`, add `let _ = &resolved;` as a temporary and remove it in B4.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-core subagent_type`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-core/src/dispatch.rs
git commit -m "feat(core): subagent_type enum discovery + unknown-type error (fix M1) (3B-1 Wave B)"
```

### Task B4: Named-type resolution — prompt+preamble, tools, model/window swap, E4 ToolCallLimit

**Files:**
- Modify: `agent/crates/agent-core/src/dispatch.rs` (`execute()` ~L387-560: apply `resolved` to the allowlist, system prompt, model/protocol/loop_config, and the child middleware stack; add `ToolCallLimit` to the crate `use`)

**Interfaces:**
- Consumes: `resolved: Option<&ResolvedSubAgent>` (Task B3), `crate::ToolCallLimit`.
- Produces: a named child whose construction uses the spec's prompt (+ preamble), tools, model/protocol/window, and (if set) a child `ToolCallLimit`; per-call `role`/`tools` are ignored for named types.

> **BORROW-SAFETY RULE (architecture review):** `resolved` is a `&`-borrow into `self.deps.subagents`. Read every field you need out of `resolved` into an **owned local** (clone `system_prompt`/`tools`, `Arc::clone` `model`/`protocol`, copy the `Option<usize>`/`Option<u32>` knobs) **before** `AgentLoop::new`, and do NOT reference `resolved` anywhere after it. As written below this holds (last use of `resolved` is Step 6, before the child `.await` at ~L570), keeping the future `Send`; the rule prevents a later edit from reintroducing a hold-across-`await`.

- [ ] **Step 1: Write the failing tests**

Add to the dispatch test module (reuse the child-construction test scaffolding the file already has for asserting child prompt / stack / model — mirror `child_stack_is_exactly_curation_and_stuck_detection_never_memory_recall`):
```rust
#[tokio::test]
async fn named_type_uses_spec_prompt_and_preamble_ignoring_role_tools() {
    // A capturing model records the child's system prompt; assert it is the
    // spec prompt + preamble, NOT the parent child_system_prompt or the role.
    let (deps, captured_system) = deps_with_capturing_child(one_agent_registry());
    let tool = DispatchAgentTool::new(deps);
    let _ = tool.execute(serde_json::json!({
        "prompt":"do it","subagent_type":"reviewer",
        "role":"IGNORED ROLE","tools":["IGNORED"]
    }), &exec_ctx()).await;
    let sys = captured_system.lock().unwrap().clone().expect("child ran");
    assert!(sys.starts_with("You review."));
    assert!(sys.contains(SUBAGENT_PREAMBLE));
    assert!(!sys.contains("IGNORED ROLE"));
}

// Pin m-3 / architecture item (b): with a NON-EMPTY registry, selecting
// general-purpose still yields the 3A child (parent prompt + role appended) —
// the headline byte-identical invariant (spec §3 invariant 1) on the `None` arm.
#[tokio::test]
async fn general_purpose_under_nonempty_registry_is_unchanged() {
    let (deps, captured_system) = deps_with_capturing_child(one_agent_registry());
    let parent_prompt = deps.child_system_prompt.clone();
    let tool = DispatchAgentTool::new(deps);
    let _ = tool.execute(serde_json::json!({
        "prompt":"do it","subagent_type":"general-purpose","role":"R"
    }), &exec_ctx()).await;
    let sys = captured_system.lock().unwrap().clone().expect("child ran");
    assert!(sys.starts_with(&parent_prompt));   // parent-derived prompt, not a spec's
    assert!(sys.contains("Role: R"));            // role honored on the general-purpose path
}

// Unknown-tool refs surface at DISPATCH time (Task B2 design note), not assembly.
#[tokio::test]
async fn named_type_unknown_tool_errors_at_dispatch() {
    let mut m = std::collections::HashMap::new();
    m.insert("bad".to_string(), ResolvedSubAgent {
        description: "d".into(),
        system_prompt: format!("p\n\n{}", SUBAGENT_PREAMBLE),
        tools: Some(vec!["no_such_tool".into()]),   // not in the (empty) base snapshot
        model: Arc::new(ScriptedModel::new(vec![])),
        protocol: Arc::new(PassthroughProtocol),
        model_limit: None, max_tokens: None, tool_call_limit: None,
    });
    let mut deps = exec_deps(ScriptedModel::new(vec![]), 1);
    deps.subagents = Arc::new(SubAgentRegistry::from_map(m));
    let tool = DispatchAgentTool::new(deps);
    let err = tool.execute(
        serde_json::json!({"prompt":"hi","subagent_type":"bad"}),
        &exec_ctx(),
    ).await.unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(_)));
}
```
NOTE: `deps_with_capturing_child(registry) -> (DispatchDeps, Arc<Mutex<Option<String>>>)` — the file already has a capturing child harness near `dispatch.rs:1024`; extend it to record the child's system message and to set `subagents = registry`. Do NOT build a parallel harness. (`exec_deps` builds `base_tools: vec![]`, so the `no_such_tool` ref is genuinely absent — the existing execute-time allowlist check at `dispatch.rs:410-425` fires.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-core named_type_ && cargo test -p agent-core general_purpose_under`
Expected: FAIL (named path not yet applied — child still gets parent prompt; the general-purpose pin fails only if the `None` arm regresses, which it must not).

- [ ] **Step 3: Apply `resolved` — allowlist**

Add `ToolCallLimit` to the crate `use` at the top of `dispatch.rs`:
```rust
use crate::{
    AgentEvent, AgentLoop, ContextCurationMiddleware, CuratedContext, EventSink, LoopConfig,
    Middleware, OffloadConfig, RepairMiddleware, SessionArtifacts, StuckDetectionMiddleware,
    TodoHandle, ToolCallLimit, WriteTodosTool,
};
```
Where `allow` is computed from the `tools` arg (~L387-403), override it for a named type. Immediately AFTER the existing `allow` binding, add:
```rust
        // Named type: its allowlist REPLACES per-call tools (deepagents replace
        // semantics). general-purpose keeps the per-call `allow` computed above.
        let allow = match resolved {
            Some(r) => r.tools.clone(),
            None => allow,
        };
```
(If `allow` is currently `let allow: Option<Vec<String>> = …;`, change it to `let mut allow` OR shadow it as above — shadowing with a new `let allow` is cleanest and compiles.)

- [ ] **Step 4: Apply `resolved` — system prompt**

Where the child system prompt is built (~L498-501):
```rust
        let system = match &role {
            Some(r) => format!("{}\n\nRole: {r}", self.deps.child_system_prompt),
            None => self.deps.child_system_prompt.clone(),
        };
```
replace with:
```rust
        // Named type replaces the parent-derived prompt (preamble already baked
        // into resolved.system_prompt) and ignores `role`. general-purpose keeps
        // the parent child_system_prompt + optional role (byte-identical to 3A).
        let system = match resolved {
            Some(r) => r.system_prompt.clone(),
            None => match &role {
                Some(rl) => format!("{}\n\nRole: {rl}", self.deps.child_system_prompt),
                None => self.deps.child_system_prompt.clone(),
            },
        };
```

- [ ] **Step 5: Apply `resolved` — model / protocol / loop_config**

Find the `AgentLoop::new(...)` child construction (~L519-527). It currently passes `self.deps.model.clone()`, `self.deps.protocol.clone()`, and later `self.deps.loop_config.clone()`. Just before it, compute the effective triple:
```rust
        // Named type may route its own model/protocol/window; general-purpose
        // uses the parent-configured child defaults (byte-identical to 3A).
        let (child_model, child_protocol, child_loop_config) = match resolved {
            Some(r) => {
                let mut lc = self.deps.loop_config.clone();
                if let Some(ml) = r.model_limit {
                    lc.model_limit = ml;
                }
                if r.max_tokens.is_some() {
                    lc.max_tokens = r.max_tokens;
                }
                (r.model.clone(), r.protocol.clone(), lc)
            }
            None => (
                self.deps.model.clone(),
                self.deps.protocol.clone(),
                self.deps.loop_config.clone(),
            ),
        };
```
Change the `AgentLoop::new(...)` call to use `child_model`, `child_protocol`, and `child_loop_config` in place of the three `self.deps.*` clones.

- [ ] **Step 6: Apply `resolved` — child ToolCallLimit (E4)**

Where the child middleware stack is set (~L554-559):
```rust
        let child = child
            .with_middleware(vec![
                curation,
                Arc::new(StuckDetectionMiddleware),
                Arc::new(RepairMiddleware),
            ])
            .with_backend(child_backend);
```
replace with:
```rust
        // 3A default child stack; a named type with tool_call_limit appends the
        // 3A ToolCallLimit guardrail (E4). Aborts via EndRun(StopReason::Error).
        let mut child_mw: Vec<Arc<dyn Middleware>> = vec![
            curation,
            Arc::new(StuckDetectionMiddleware),
            Arc::new(RepairMiddleware),
        ];
        if let Some(cap) = resolved.and_then(|r| r.tool_call_limit) {
            child_mw.push(Arc::new(ToolCallLimit::with_cap(cap)));
        }
        let child = child.with_middleware(child_mw).with_backend(child_backend);
```

- [ ] **Step 7: Run the named test + the byte-identical regression**

Run: `cd agent && cargo test -p agent-core named_type_uses_spec_prompt && cargo test -p agent-core dispatch`
Expected: PASS; the existing `general-purpose`/child-stack tests (e.g. `child_stack_is_exactly_curation_and_stuck_detection_never_memory_recall`) still green (unchanged when `resolved` is `None`).

- [ ] **Step 8: Commit**

```bash
git add agent/crates/agent-core/src/dispatch.rs
git commit -m "feat(core): named subagent_type resolution — prompt/preamble, tools, model, E4 ToolCallLimit (3B-1 Wave B)"
```

---

## Task C: config example + full CI gate

**Files:**
- Modify: `agent/config.example.toml` (document a `[[named_subagents]]` block)

- [ ] **Step 1: Add a documented example**

Append to `agent/config.example.toml`:
```toml
# Named sub-agents (Phase 3B-1). Each becomes a `subagent_type` the model can
# pick via dispatch_agent. Only name/description/system_prompt are required.
# [[named_subagents]]
# name = "reviewer"
# description = "Reviews Rust changes for correctness and style."
# system_prompt = "You are a meticulous Rust reviewer. Report concrete findings."
# tools = ["read_file", "grep", "git_status"]   # optional allowlist (default: all)
# tool_call_limit = 50                            # optional runaway bound (1..=1000)
# # model = { model = "haiku" }                   # optional; same endpoint only
```

- [ ] **Step 2: Run the full CI gate**

Run: `cd /home/kalen/rust-agent-runtime && bash scripts/ci.sh`
Expected: PASS (okf check + skills lint + fmt + clippy + `cargo test` for `agent/` + conditional src-tauri + web typecheck/vitest). Fix any `fmt`/`clippy` findings the new code introduces (e.g. remove the temporary `let _ = &resolved;` from Task B3 if added).

- [ ] **Step 3: Commit**

```bash
git add agent/config.example.toml
git commit -m "docs(config): document [[named_subagents]] example (3B-1)"
```

---

## Self-Review (run before handoff)

**1. Spec coverage** (spec §§ → task):
- §2.1 `SubAgentSpec` (core + reserved) → A1. §2.2 registry + `general-purpose` lockdown → A2 (reserved name) + B1/B2 (registry). §2.3 `subagent_type` enum (M1) + preamble (MINOR-5) + replace-semantics + merge-back → B3 + B4. §2.4 tools/model + endpoint lock (M3) → A2 (lock) + B2 (resolve) + B4 (apply); unknown-tool ref → dispatch-time check (B4 test). §2.5 E4 `tool_call_limit` + ceiling (M3) → A2 (ceiling) + B4 (apply). §2.6 system_prompt ceiling (M3) → A2. §2.7 config + validation + reserved rejection → A2. §3 invariants → B1 (empty=byte-identical) + B4 (`general_purpose_under_nonempty_registry_is_unchanged` pins the non-empty case). §5 residuals: E4 → B4; Read⇒side-effects → carried to 3B-1c (no task, correct); eval `.with_todos` → A3. §6 tests → embedded per task.
- Gap check: the `general-purpose` byte-identical regression is now pinned BOTH ways — empty registry (`empty_registry_omits_subagent_type_from_schema`, B1) and non-empty-registry-but-general-purpose-selected (`general_purpose_under_nonempty_registry_is_unchanged`, B4). No gap.

**2. Placeholder scan:** No "TBD"/"add error handling"/"similar to Task N". The remaining NOTE blocks (test-helper reuse, capturing-child harness) point at named real helpers in the Test-harness reference — codebase-matching, not deferred work.

**3. Type consistency:** `SubAgentSpec` fields (A1) match validation (A2), resolution (B2), and the reserved-rejection list. `ResolvedSubAgent`/`SubAgentRegistry` signatures (B1) match their uses in B2 (`from_map`), B3 (`is_empty`/`schema_hints`/`names`/`get`), B4 (`.tools`/`.system_prompt`/`.model`/`.protocol`/`.model_limit`/`.max_tokens`/`.tool_call_limit`). `subagents: Arc<SubAgentRegistry>` field name consistent across dispatch.rs + assemble.rs.

## Plan review log

### 2026-07-09 — Two plan reviewers (coverage/buildability + architecture)

- **Coverage/buildability: APPROVE-WITH-FIXES.** **BLOCKER** — assembly-time tool validation would have made `assemble_loop` fallible and `.expect()`-panicked the lenient-boot server. **Resolved by REMOVING assembly-time tool validation** (Task B2 design note): a named spec's `tools` becomes the child `allow` allowlist, already validated at dispatch by the existing `dispatch.rs:410-425` check; pinned by `named_type_unknown_tool_errors_at_dispatch` (B4). **MAJOR** — plan test literals named non-existent helpers. **Fixed**: added the Test-harness reference and rewrote every literal to `exec_deps`/`exec_ctx`/`ScriptedModel`/`PassthroughProtocol` and `base().merge(from_str::<PartialRuntimeConfig>(…))`. **MAJOR** — `Arc`/`Clone` note added to B1. **MINOR** — `#[cfg(test)] subagent_registry` mut-binding pattern spelled out (B2 Step 3a). Verified-sound (no action): `build_routed_model`/`pick_protocol`/`child_protocol_name` signatures, `ToolCallLimit::with_cap` re-export, `SubAgentRegistry` re-export via `pub use dispatch::*`, `.with_todos` builder, all TDD fail-reasons.
- **Architecture: SOUND-WITH-NOTES.** Confirmed the `resolved` borrow does NOT hold across the child `.await` (B4 clones fields out first) → **added the explicit BORROW-SAFETY RULE** to B4 so a later edit can't reintroduce it. Confirmed the dep-direction seam is the only correct one, the cloned-`child_config` reproduces assembly's knobs faithfully, and the `is_empty()` byte-identical keying matches the invariant's scope. **Item (b)** — added the `general_purpose_under_nonempty_registry_is_unchanged` pin (B4). Noted-residual (accepted): per-spec routed models are built eagerly at assembly even if never dispatched — precedented (the default child already does this) and negligible for a handful of specs; revisit only at dozens of rarely-used specs.

## Execution Handoff

Two execution options — **subagent-driven (recommended)**: fresh subagent per task + two-stage review, or **inline** via executing-plans with checkpoints.
