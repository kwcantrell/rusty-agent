# Sub-agent Advanced Dispatch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Per-child and per-compaction model routing (`ModelRef` config), a minimal `role` arg on dispatch_agent, a configurable sub-agent depth budget with correct depth-2 attribution, and a fan-out description nudge.

**Architecture:** `ModelRef` is a partial override (every None inherits the primary config); one `build_routed_model` helper constructs routed clients centrally in `assemble_loop` (frontends contribute `api_key`/`claude_binary` via `LoopParts`). `AgentLoop` gains an optional `compaction_model` consulted at both `MaintCtx` sites. `DispatchDeps` derives Clone and gains `depth`/`max_depth`/`id_prefix`/`compaction_model`; `execute()` registers a nested dispatch tool at depth+1 whose `id_prefix` is the freshly-minted `sub{n}:` — so a grandchild's `parent_id` is the child's *visible* row id.

**Tech Stack:** Rust only (serde, tokio, existing testkit). Zero wire/web changes.

**Spec:** `docs/superpowers/specs/2026-07-02-subagent-advanced-dispatch-design.md` (G1–G10).

## Global Constraints

- With all new config absent, behavior is byte-identical to today: same single model everywhere, depth 1, no role text (spec Invariant). Existing tests must pass unchanged.
- Routing never widens privilege: policy/approval/sandbox/registry-subsetting/budgets/attribution untouched by which client serves a completion.
- All new `RuntimeConfig` fields serde-default (None / 1) + PartialConfig + `merge()` arms, mirroring `subagent_max_turns`.
- `subagent_max_depth` is clamped to ≥1 at the read site (0 would contradict the top-level tool; "no sub-agents" is `subagents: false`).
- Two Cargo workspaces; all work is in `agent/` — run cargo from `/home/kalen/rust-agent-runtime/agent` (`source ~/.cargo/env` if missing). Conventional commits; TDD.
- No changes under `web/` or to `agent-server/src/wire.rs`.

---

### Task 1: `ModelRef` + config slots + `build_routed_model`

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs`
- Modify: `agent/crates/agent-runtime-config/src/lib.rs` (next to `build_model`, ~line 75)
- Test: both files' `mod tests`

**Interfaces:**
- Produces (later tasks consume verbatim):

```rust
pub struct ModelRef { pub backend: Option<String>, pub base_url: Option<String>,
    pub model: Option<String>, pub claude_binary: Option<String>, pub protocol: Option<String> }
// RuntimeConfig: pub subagent_model: Option<ModelRef>, pub compaction_model: Option<ModelRef>,
//                pub subagent_max_depth: usize  (default 1)
pub fn build_routed_model(cfg: &RuntimeConfig, r: &ModelRef, claude_binary: &str,
    api_key: Option<String>) -> Arc<dyn ModelClient>
```

- [ ] **Step 1: Write the failing tests**

In `runtime_config.rs` `mod tests` (mirror the `subagent_fields_*` tests):

```rust
    #[test]
    fn model_routing_fields_default_none_and_depth_one() {
        let c = base();
        assert!(c.subagent_model.is_none());
        assert!(c.compaction_model.is_none());
        assert_eq!(c.subagent_max_depth, 1);
        // Old on-disk file without the fields -> defaults.
        let mut v = serde_json::to_value(&c).unwrap();
        let o = v.as_object_mut().unwrap();
        o.remove("subagent_model");
        o.remove("compaction_model");
        o.remove("subagent_max_depth");
        let parsed: RuntimeConfig = serde_json::from_value(v).unwrap();
        assert!(parsed.subagent_model.is_none());
        assert_eq!(parsed.subagent_max_depth, 1);
    }

    #[test]
    fn model_routing_fields_partial_merge() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.json");
        std::fs::write(&path,
            r#"{"subagent_model": {"model": "haiku"}, "subagent_max_depth": 2}"#).unwrap();
        let b = base();
        let loaded = RuntimeConfig::load_over(b.clone(), &path);
        let r = loaded.subagent_model.expect("merged");
        assert_eq!(r.model.as_deref(), Some("haiku"));
        assert!(r.backend.is_none()); // partial ModelRef: unset fields stay None
        assert_eq!(loaded.subagent_max_depth, 2);
        assert!(loaded.compaction_model.is_none()); // untouched
        assert_eq!(loaded.model, b.model);
    }
```

In `lib.rs` `mod tests` (create if absent; construct-only — no live server):

```rust
    #[test]
    fn build_routed_model_inherits_primary_for_none_fields() {
        let cfg = RuntimeConfig::from_launch(
            "claude-cli".into(), "http://x".into(), "opus".into(), "native".into(), 8192);
        // model-only override on the primary backend: constructs a claude-cli client.
        let r = ModelRef { model: Some("haiku".into()), ..ModelRef::default() };
        let _m = build_routed_model(&cfg, &r, "claude", None);
        // backend override to openai: constructs without touching claude_binary.
        let r2 = ModelRef { backend: Some("openai".into()),
            base_url: Some("http://127.0.0.1:1".into()),
            model: Some("qwen-mini".into()), ..ModelRef::default() };
        let _m2 = build_routed_model(&cfg, &r2, "claude", None);
        // Construction is the contract here (build_model is already the tested seam);
        // resolution correctness is pinned by resolve() below.
        let (be, url, model, bin) = r2.resolve(&cfg, "claude");
        assert_eq!((be.as_str(), model.as_str()), ("openai", "qwen-mini"));
        assert_eq!(url, "http://127.0.0.1:1");
        assert_eq!(bin, "claude");
        let (be1, _, model1, _) = r.resolve(&cfg, "claude");
        assert_eq!((be1.as_str(), model1.as_str()), ("claude-cli", "haiku"));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-runtime-config model_routing ; cargo test -p agent-runtime-config build_routed`
Expected: compile errors — `ModelRef`/fields undefined.

- [ ] **Step 3: Implement**

`runtime_config.rs`:

```rust
/// Partial model override (spec 2026-07-02 sub-spec #3, G1): every `None`
/// inherits the primary config's value, so `{"model": "haiku"}` just works.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ModelRef {
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub claude_binary: Option<String>,
    /// Tool-call protocol for routed CHILD LOOPS ("native" | "prompted");
    /// compaction ignores it (plain completion).
    #[serde(default)]
    pub protocol: Option<String>,
}

impl ModelRef {
    /// Merge with the primary config: (backend, base_url, model, claude_binary).
    /// `primary_claude_binary` is a parameter because it lives on the frontends,
    /// not RuntimeConfig.
    pub fn resolve(&self, cfg: &RuntimeConfig, primary_claude_binary: &str)
        -> (String, String, String, String)
    {
        (
            self.backend.clone().unwrap_or_else(|| cfg.backend.clone()),
            self.base_url.clone().unwrap_or_else(|| cfg.base_url.clone()),
            self.model.clone().unwrap_or_else(|| cfg.model.clone()),
            self.claude_binary.clone().unwrap_or_else(|| primary_claude_binary.to_string()),
        )
    }
}
```

Fields on `RuntimeConfig` (next to the other `subagent_*` fields), default fn, `from_launch` values, `PartialConfig` fields, `merge()` arms — the exact `subagent_max_turns` pattern:

```rust
    /// Model serving sub-agent (dispatch_agent) children; None = the session model.
    #[serde(default)]
    pub subagent_model: Option<ModelRef>,
    /// Model serving context compaction; None = the session model.
    #[serde(default)]
    pub compaction_model: Option<ModelRef>,
    /// Max sub-agent nesting depth (1 = children cannot dispatch). Read sites
    /// clamp to >= 1; "no sub-agents at all" is `subagents: false`.
    #[serde(default = "default_subagent_max_depth")]
    pub subagent_max_depth: usize,
```

```rust
fn default_subagent_max_depth() -> usize {
    1
}
```

`lib.rs`, next to `build_model`:

```rust
/// Build a routed model client from a partial `ModelRef`, inheriting every
/// unset field from the primary config (spec G1).
pub fn build_routed_model(
    cfg: &RuntimeConfig,
    r: &ModelRef,
    claude_binary: &str,
    api_key: Option<String>,
) -> Arc<dyn ModelClient> {
    let (backend, base_url, model, bin) = r.resolve(cfg, claude_binary);
    build_model(&backend, &base_url, &model, &bin, api_key)
}
```

(Export `ModelRef` from the crate root alongside `RuntimeConfig`.)

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-runtime-config`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A agent/crates/agent-runtime-config
git commit -m "feat(config): ModelRef partial override + subagent/compaction model slots + depth knob"
```

---

### Task 2: Compaction model routing in the loop

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` (AgentLoop struct ~line 99, `new` ~112, the two `MaintCtx` sites ~352 and ~594)
- Test: `agent/crates/agent-core/tests/compaction_routing.rs` (new)

**Interfaces:**
- Consumes: nothing new (pure agent-core).
- Produces: `AgentLoop::with_compaction_model(self, Arc<dyn ModelClient>) -> Self` (builder, `with_retriever` pattern) and private `fn maint_model(&self) -> &Arc<dyn ModelClient>`; both `MaintCtx` sites use `model: self.maint_model()`. Task 4/5 rely on the builder name exactly.

- [ ] **Step 1: Write the failing test**

Create `agent/crates/agent-core/tests/compaction_routing.rs`:

```rust
//! Pins spec G4: a routed compaction model serves maintain()/overflow
//! compaction; the session model is untouched by the summary call.
use agent_core::testkit::{AlwaysApprove, CollectingSink, PassthroughProtocol, Scripted, ScriptedModel};
use agent_core::{AgentLoop, ContextManager, CuratedContext, InMemoryOffloadStore, LoopConfig};
use agent_model::Message;
use agent_policy::RulePolicy;
use agent_tools::ToolRegistry;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn routed_compaction_model_serves_the_summary_call() {
    let dir = tempfile::tempdir().unwrap();
    // Session model: one plain final answer. Compaction model: one summary.
    let session = Arc::new(ScriptedModel::new(vec![Scripted::Text("done".into())]));
    let compactor = Arc::new(ScriptedModel::new(vec![Scripted::Text("SUMMARY".into())]));
    let sink = Arc::new(CollectingSink::default());
    let agent = AgentLoop::new(
        session.clone(),
        Arc::new(PassthroughProtocol),
        Arc::new(ToolRegistry::new()),
        Arc::new(RulePolicy {
            workspace: dir.path().to_path_buf(),
            command_allowlist: vec![],
            command_denylist: vec![],
        }),
        Arc::new(AlwaysApprove),
        sink.clone(),
        LoopConfig {
            model_limit: 16384,
            max_turns: 3,
            max_retries: 1,
            tool_timeout: Duration::from_secs(5),
            stream_idle_timeout: Duration::from_secs(5),
            workspace: dir.path().to_path_buf(),
            ..LoopConfig::default()
        },
    )
    .with_compaction_model(compactor.clone());

    let mut ctx = CuratedContext::new(
        Message::system("s"),
        Arc::new(InMemoryOffloadStore::new()),
        Arc::new(AtomicBool::new(false)),
    );
    // Seed enough closed history that a forced compaction has a span to replace,
    // then request compaction so the post-turn maintain() runs the compactor.
    for i in 0..6 {
        ctx.append(Message::user(format!("old question {i}")));
        ctx.append(Message::assistant(format!("old answer {i}"), None));
    }
    ctx.request_compaction();

    agent.run(&mut ctx, "final question".into()).await.unwrap();

    // The compaction model was consumed; the summary call did NOT come from the
    // session model (its single scripted turn answered the user question).
    assert_eq!(compactor.remaining(), 0, "routed compaction model unused");
    assert_eq!(session.remaining(), 0);
    let events = sink.events.lock().unwrap().clone();
    assert!(events.iter().any(|e| e.starts_with("compacted:")), "{events:?}");
}
```

Note: `maintain()` runs after tool turns; a tool-less final answer ends the run at `Done` — check the loop source: if the flag-forced compaction only fires on the post-tools `maintain()` path, script a tool turn instead (register the `dispatch_tool.rs`-style `Echo` fake and script `Scripted::Call` then `Scripted::Text`, exactly like `agent-core/tests/dispatch_tool.rs` does). Adapt so the test genuinely drives one `maintain()` with the compact flag set — mirror how `curated.rs`'s own tests force compaction if the seeded-history shape needs adjusting (e.g. goal pinning). The assertion contract (compactor consumed + `compacted:` event) is the requirement; the harness shape is yours to adapt.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-core --test compaction_routing`
Expected: compile error — `with_compaction_model` undefined.

- [ ] **Step 3: Implement**

`loop_.rs`:

```rust
    // field on AgentLoop:
    compaction_model: Option<Arc<dyn ModelClient>>,
    // in new(): compaction_model: None,

    /// Route context compaction to a (typically cheaper) dedicated model
    /// (spec 2026-07-02 sub-spec #3, G4). None = the session model.
    pub fn with_compaction_model(mut self, model: Arc<dyn ModelClient>) -> Self {
        self.compaction_model = Some(model);
        self
    }

    /// The model that serves maintenance (compaction) completions.
    fn maint_model(&self) -> &Arc<dyn ModelClient> {
        self.compaction_model.as_ref().unwrap_or(&self.model)
    }
```

Both `MaintCtx` constructions (`loop_.rs` ~352 and ~594): `model: self.maint_model(),` replacing `model: &self.model,`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-core`
Expected: PASS (new test + all existing — the None path is `&self.model` exactly as before).

- [ ] **Step 5: Commit**

```bash
git add -A agent/crates/agent-core
git commit -m "feat(core): route compaction to an optional dedicated model (audit run_compaction routing)"
```

---

### Task 3: `role` arg on dispatch_agent

**Files:**
- Modify: `agent/crates/agent-core/src/dispatch.rs`
- Test: `agent/crates/agent-core/tests/dispatch_tool.rs`

**Interfaces:**
- Consumes: existing `DispatchAgentTool` (deps helper `deps(...)` in the test file).
- Produces: `dispatch_agent` accepts optional `role: string` (≤ 2000 chars); child system prompt = `{deps.child_system_prompt}\n\nRole: {role}` when present. `pub const MAX_ROLE_CHARS: usize = 2000;` exported from dispatch.rs.

- [ ] **Step 1: Write the failing tests**

Append to `dispatch_tool.rs` (a capturing model records the child's request):

```rust
/// Records every CompletionRequest's system message, then delegates to a script.
struct CapturingModel {
    inner: ScriptedModel,
    systems: Mutex<Vec<String>>,
}
#[async_trait::async_trait]
impl agent_model::ModelClient for CapturingModel {
    async fn stream(
        &self,
        req: agent_model::CompletionRequest,
    ) -> Result<
        futures::stream::BoxStream<'static, Result<agent_model::Chunk, agent_model::ModelError>>,
        agent_model::ModelError,
    > {
        if let Some(m) = req.messages.first() {
            self.systems.lock().unwrap().push(m.content.clone());
        }
        self.inner.stream(req).await
    }
}
```

(If `Message.content`/`req.messages` field names differ, adapt to `agent-model/src/types.rs` — the requirement is capturing the system message text.)

```rust
#[tokio::test]
async fn role_arg_lands_in_the_child_system_prompt() {
    let model = Arc::new(CapturingModel {
        inner: ScriptedModel::new(vec![Scripted::Text("ok".into())]),
        systems: Mutex::new(vec![]),
    });
    let mut d = deps(ScriptedModel::new(vec![]), Arc::new(FullSink::default()), vec![]);
    d.model = model.clone();
    let tool = DispatchAgentTool::new(d);
    tool.execute(serde_json::json!({"prompt": "p", "role": "You are a meticulous code reviewer."}), &tool_ctx())
        .await
        .unwrap();
    let systems = model.systems.lock().unwrap().clone();
    assert!(systems[0].contains("Role: You are a meticulous code reviewer."), "{systems:?}");
}

#[tokio::test]
async fn role_is_optional_and_bounded() {
    let mk = || {
        DispatchAgentTool::new(deps(
            ScriptedModel::new(vec![Scripted::Text("ok".into())]),
            Arc::new(FullSink::default()),
            vec![],
        ))
    };
    // Absent role: fine (no Role block asserted via the capturing test above).
    mk().execute(serde_json::json!({"prompt": "p"}), &tool_ctx()).await.unwrap();
    // Whitespace-only role: treated as absent (no error).
    mk().execute(serde_json::json!({"prompt": "p", "role": "   "}), &tool_ctx()).await.unwrap();
    // Over 2000 chars: InvalidArgs.
    let long = "r".repeat(2001);
    let err = mk()
        .execute(serde_json::json!({"prompt": "p", "role": long}), &tool_ctx())
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(ref m) if m.contains("role")), "{err:?}");
    // Non-string role: InvalidArgs.
    let err = mk()
        .execute(serde_json::json!({"prompt": "p", "role": 7}), &tool_ctx())
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(_)), "{err:?}");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-core --test dispatch_tool role`
Expected: FAIL — `Role:` block absent / no `role` validation (`InvalidArgs` not returned).

- [ ] **Step 3: Implement in `dispatch.rs`**

```rust
/// Upper bound on the `role` arg (system-prompt injection; spec G6).
pub const MAX_ROLE_CHARS: usize = 2000;
```

In `execute()`, after the `prompt` parse:

```rust
        let role: Option<String> = match args.get("role") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(s)) => {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    None
                } else if s.chars().count() > MAX_ROLE_CHARS {
                    return Err(ToolError::InvalidArgs(format!(
                        "role must be at most {MAX_ROLE_CHARS} characters"
                    )));
                } else {
                    Some(trimmed.to_string())
                }
            }
            Some(_) => return Err(ToolError::InvalidArgs("role must be a string".into())),
        };
```

Where the child `CuratedContext` is built, the system message becomes:

```rust
        let system = match &role {
            Some(r) => format!("{}\n\nRole: {r}", self.deps.child_system_prompt),
            None => self.deps.child_system_prompt.clone(),
        };
        // ... CuratedContext::new(Message::system(system), store, flag)
```

Schema `properties` gains:

```json
                    "role": {
                        "type": "string",
                        "description": "Optional persona/role instructions injected into the sub-agent's system prompt (stronger steering than putting them in the prompt). Max 2000 characters."
                    }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-core --test dispatch_tool && cargo test -p agent-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A agent/crates/agent-core
git commit -m "feat(core): dispatch_agent role arg — bounded persona block in the child system prompt"
```

---

### Task 4: Depth budget + visible-id prefix + compaction pass-through

**Files:**
- Modify: `agent/crates/agent-core/src/dispatch.rs`
- Test: `agent/crates/agent-core/tests/dispatch_tool.rs` + `dispatch.rs` unit tests

**Interfaces:**
- Consumes: `AgentLoop::with_compaction_model` (Task 2).
- Produces (Task 5 constructs these fields):

```rust
// DispatchDeps derives Clone and gains:
pub compaction_model: Option<Arc<dyn ModelClient>>, // child loops route compaction too
pub depth: usize,        // this tool's depth; top-level = 1
pub max_depth: usize,    // from cfg.subagent_max_depth (assembly clamps >= 1)
pub id_prefix: String,   // "" at top level; "sub{n}:" for a nested tool (spec G8)
```

- Behavior: `parent_id = format!("{}{}", deps.id_prefix, ctx.call_id)`; iff `depth < max_depth` the child registry gets a nested `DispatchAgentTool` (cloned deps, `depth+1`, `id_prefix = "sub{n}:"` where `n` is THIS call's freshly-minted ordinal). Description gains the fan-out sentence (G9).

- [ ] **Step 1: Write the failing tests**

Append to `dispatch_tool.rs`. The `deps(...)` helper gains the new fields (`compaction_model: None, depth: 1, max_depth: 1, id_prefix: String::new()`) — update it and `dispatch.rs::exec_deps` likewise.

```rust
#[tokio::test]
async fn depth_two_child_can_dispatch_and_grandchild_attribution_chains() {
    let sink = Arc::new(FullSink::default());
    // Child model: dispatches a grandchild, then answers.
    // Grandchild model comes from the SAME deps.model (scripted queue): its turn
    // is the "gc done" text. Order: child turn1 -> grandchild turn -> child turn2.
    let tap = Arc::new(TapSpy::default());
    let mut d = deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "dispatch_agent".into(), r#"{"prompt":"nested task"}"#.into()),
            Scripted::Text("gc done".into()),   // consumed by the GRANDCHILD loop
            Scripted::Text("child done".into()), // child's final answer
        ]),
        sink.clone(),
        vec![],
    );
    d.max_depth = 2;
    d.child_trace = Some(tap.clone());
    let tool = DispatchAgentTool::new(d);
    let out = tool.execute(serde_json::json!({"prompt": "p"}), &tool_ctx()).await.unwrap();
    assert!(out.content.starts_with("child done"), "{}", out.content);

    let events = sink.events.lock().unwrap().clone();
    // The child's dispatch call is visible as sub{n}:c1 with parent "d1";
    // the grandchild's? there are no grandchild TOOL calls here, so the pin is:
    // the child-level dispatch_agent tool_start row itself chains to d1...
    let child_dispatch_start = events.iter()
        .find(|(k, _, name, _)| k == "tool_start" && name == "sub:dispatch_agent")
        .expect("child dispatch row forwarded");
    assert_eq!(child_dispatch_start.3, "d1");
    // ...and the grandchild's ServerUsage carries parent_id == the child's VISIBLE id.
    let child_visible_id = child_dispatch_start.1.clone(); // "sub{n}:c1"
    assert!(child_visible_id.ends_with(":c1"), "{child_visible_id}");
    // Concrete grandchild pin: at least one forwarded event carries the child's
    // visible id as its parent_id (the grandchild's ServerUsage does).
    assert!(
        events.iter().any(|(_, _, _, parent)| parent == &child_visible_id),
        "no event chained to the child's visible id {child_visible_id}: {events:?}"
    );
    // Tap pin (spec Testing): grandchild suppressed events reach the tap with
    // the prefixed parent id.
    let tap_parents = tap.seen.lock().unwrap().clone();
    assert!(
        tap_parents.iter().any(|(_, p, _)| p == &child_visible_id),
        "no tap record chained to {child_visible_id}: {tap_parents:?}"
    );
}

/// Local tap spy (SubagentTrace is pub): records (ordinal, parent_id, kind).
#[derive(Default)]
struct TapSpy {
    seen: Mutex<Vec<(u64, String, &'static str)>>,
}
impl agent_core::SubagentTrace for TapSpy {
    fn record(&self, n: u64, parent_id: &str, event: &agent_core::AgentEvent) {
        let kind = match event {
            agent_core::AgentEvent::Token(_) => "token",
            agent_core::AgentEvent::Done(_) => "done",
            _ => "other",
        };
        self.seen.lock().unwrap().push((n, parent_id.to_string(), kind));
    }
}

#[tokio::test]
async fn depth_two_is_the_floor_for_the_grandchild() {
    let sink = Arc::new(FullSink::default());
    let mut d = deps(
        ScriptedModel::new(vec![
            // Child dispatches grandchild; grandchild TRIES to dispatch (rejected: unknown tool).
            Scripted::Call("c1".into(), "dispatch_agent".into(), r#"{"prompt":"nested"}"#.into()),
            Scripted::Call("g1".into(), "dispatch_agent".into(), r#"{"prompt":"三"}"#.into()),
            Scripted::Text("gc done".into()),
            Scripted::Text("child done".into()),
        ]),
        sink.clone(),
        vec![],
    );
    d.max_depth = 2;
    let tool = DispatchAgentTool::new(d);
    tool.execute(serde_json::json!({"prompt": "p"}), &tool_ctx()).await.unwrap();
    let events = sink.events.lock().unwrap().clone();
    // The grandchild's dispatch attempt is a denied tool_result at depth 3.
    assert!(
        events.iter().any(|(k, _, n, _)| k == "tool_result:denied" && n == "sub:dispatch_agent"),
        "{events:?}"
    );
}

#[tokio::test]
async fn default_depth_one_matches_v1_no_recursion() {
    // deps() defaults max_depth = 1: the existing child_cannot_recurse_into_dispatch_agent
    // test already pins this; this test pins the DEFAULT deps value explicitly.
    let d = deps(ScriptedModel::new(vec![]), Arc::new(FullSink::default()), vec![]);
    assert_eq!((d.depth, d.max_depth), (1, 1));
}

#[test]
fn description_mentions_concurrent_fanout() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    assert!(tool.description().contains("concurrently"), "{}", tool.description());
}
```

Note on the first test: FullSink is the quad sink `(kind, id, name, parent)`; the ServerUsage arm records prompt_tokens as `id` — the parent field is what matters. Remove the placeholder `|| true` line when transcribing (it is NOT part of the test; the concrete pin below it is the assertion). The scripted-queue sharing (grandchild pulls from the same ScriptedModel) is deterministic here because the child blocks awaiting the grandchild before its own next turn.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-core --test dispatch_tool depth ; cargo test -p agent-core --test dispatch_tool description_mentions`
Expected: compile error — `DispatchDeps` has no `max_depth`/`depth`/`id_prefix`.

- [ ] **Step 3: Implement in `dispatch.rs`**

`DispatchDeps`: add `#[derive(Clone)]` and the four fields (docs from Interfaces block). In `execute()`:

1. Mint the ordinal FIRST (move the existing `next_dispatch_n()` call up, before registry construction): `let n = next_dispatch_n();`
2. Visible parent id: `let parent_id = format!("{}{}", self.deps.id_prefix, ctx.call_id);` — pass to `SubagentSink::new(self.deps.sink.clone(), n, parent_id, self.deps.child_trace.clone())`.
3. Nested registration, right after the base-tools loop (the `dispatch_agent` skip-guard stays):

```rust
        // Depth budget (spec G7/G8): a child may dispatch only while under the
        // configured depth; the nested tool's id_prefix is THIS call's visible
        // prefix so a grandchild's parent_id is the child row's on-wire id.
        if self.deps.depth < self.deps.max_depth {
            let mut nested = self.deps.clone();
            nested.depth = self.deps.depth + 1;
            nested.id_prefix = format!("sub{n}:");
            reg.register(Arc::new(DispatchAgentTool::new(nested)));
        }
```

4. Child loop: apply compaction routing after `AgentLoop::new(...)`:

```rust
        let child = match &self.deps.compaction_model {
            Some(m) => child.with_compaction_model(m.clone()),
            None => child,
        };
```

5. Description: append the sentence `" You may dispatch several sub-agents in one message by issuing multiple dispatch_agent calls — they run concurrently."`

Update `exec_deps` (unit tests) and `deps` (integration) with the four new field values.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-core`
Expected: PASS — including the untouched `child_cannot_recurse_into_dispatch_agent` (max_depth 1 default) and all sub-spec #1/#2 pins.

- [ ] **Step 5: Commit**

```bash
git add -A agent/crates/agent-core
git commit -m "feat(core): sub-agent depth budget with visible-id prefix attribution; child compaction routing; fan-out description"
```

---

### Task 5: Assembly + frontends

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs`
- Modify: `agent/crates/agent-cli/src/main.rs` (~line 189 area), `agent/crates/agent-server/src/runtime.rs` (~line 208 area)
- Test: `assemble.rs` `mod tests`

**Interfaces:**
- Consumes: `build_routed_model`, `ModelRef`, cfg fields (Task 1); `with_compaction_model` (Task 2); `DispatchDeps` fields (Task 4).
- Produces: `LoopParts.api_key: Option<String>` + `LoopParts.claude_binary: String`; `#[cfg(test)] BuiltLoop.subagent_model_routed: Option<bool>` and `compaction_model_routed: bool` (`Arc::ptr_eq` pins).

- [ ] **Step 1: Write the failing tests**

In `assemble.rs` `mod tests` (the `parts()` helper gains `api_key: None, claude_binary: "claude".into()`):

```rust
    #[test]
    fn routed_models_default_to_the_primary() {
        let dir = tempfile::tempdir().unwrap();
        let built = assemble_loop(&cfg(), parts(dir.path().to_path_buf(), vec![]));
        assert_eq!(built.subagent_model_routed, Some(false));
        assert!(!built.compaction_model_routed);
    }

    #[test]
    fn routed_models_are_distinct_clients_when_configured() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.subagent_model = Some(crate::ModelRef { model: Some("mini".into()), ..Default::default() });
        c.compaction_model = Some(crate::ModelRef { model: Some("tiny".into()), ..Default::default() });
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        assert_eq!(built.subagent_model_routed, Some(true));
        assert!(built.compaction_model_routed);
    }

    #[test]
    fn depth_zero_is_clamped_to_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.subagent_max_depth = 0;
        // Assembles fine; the clamp is a read-site rule (no panic, tool registered).
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        assert!(built.registered_names.iter().any(|n| n == "dispatch_agent"));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-runtime-config routed_models ; cargo test -p agent-runtime-config depth_zero`
Expected: compile errors — `LoopParts` missing fields / `subagent_model_routed` undefined.

- [ ] **Step 3: Implement**

`LoopParts`:

```rust
    /// Inputs for constructing ROUTED model clients (spec G3). The primary
    /// model stays caller-built; both values are frontend-held today.
    pub api_key: Option<String>,
    pub claude_binary: String,
```

`assemble_loop`, before the dispatch-tool block:

```rust
    let compaction_model = cfg
        .compaction_model
        .as_ref()
        .map(|r| crate::build_routed_model(cfg, r, &parts.claude_binary, parts.api_key.clone()));
    #[cfg(test)]
    let compaction_model_routed = compaction_model.is_some();
```

Parent loop: after the existing retriever match, apply

```rust
    let agent = match &compaction_model {
        Some(m) => agent.with_compaction_model(m.clone()),
        None => agent,
    };
```

Dispatch construction gains/changes (inside the `if let Some(child_base) = child_base` block):

```rust
        let (child_model, child_protocol) = match &cfg.subagent_model {
            Some(r) => (
                crate::build_routed_model(cfg, r, &parts.claude_binary, parts.api_key.clone()),
                pick_protocol(r.protocol.as_deref().unwrap_or(&cfg.protocol)),
            ),
            None => (parts.model.clone(), pick_protocol(&cfg.protocol)),
        };
        #[cfg(test)]
        let subagent_model_routed = Some(!Arc::ptr_eq(&child_model, &parts.model));
        // DispatchDeps: model: child_model, protocol: child_protocol,
        //   compaction_model: compaction_model.clone(),
        //   depth: 1, max_depth: cfg.subagent_max_depth.max(1),
        //   id_prefix: String::new(),
        //   ... (existing fields unchanged)
```

(`#[cfg(test)]` bookkeeping mirrors `dispatch_base_names`: `subagent_model_routed: Option<bool>` is `None` when `subagents: false`; add both fields to `BuiltLoop` with `#[cfg(test)]`.)

Frontends: `agent-cli/src/main.rs` and `agent-server/src/runtime.rs` add to their `LoopParts` construction:

```rust
        api_key: api_key.clone(),
        claude_binary: claude_binary.clone(),
```

(using each frontend's existing local of the same name — CLI: `cli.claude_binary`; server: its `claude_binary` field.)

- [ ] **Step 4: Run to verify pass**

Run: `cargo build --workspace && cargo test -p agent-runtime-config -p agent-cli -p agent-server`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A agent
git commit -m "feat(runtime-config): assemble routed subagent/compaction models; depth clamp; frontends supply routed-construction inputs"
```

---

### Task 6: Workspace sweep + spec cross-check

**Files:** none expected; fix fallout only.

- [ ] **Step 1: Full CI**

Run: `cd /home/kalen/rust-agent-runtime && bash scripts/ci.sh`
Expected: green. Fix fallout minimally; commit as `chore: workspace sweep for sub-agent advanced dispatch`.

- [ ] **Step 2: Spec cross-check**

Re-read `docs/superpowers/specs/2026-07-02-subagent-advanced-dispatch-design.md` — verify G1–G10 + every Testing bullet landed or is explicitly out-of-scope; verify NO changes landed under `web/` or in `agent-server/src/wire.rs` (`git diff --stat main..HEAD` must show neither). Report gaps; don't silently fix design-level ones.

- [ ] **Step 3: Commit (only if fixes were needed)**
