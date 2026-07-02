# Sub-agent Dispatch Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `dispatch_agent` tool that runs a nested `AgentLoop` with a fresh context and a scoped tool registry, inheriting the parent's policy/approval/sandbox, and returns the child's final text as the tool result.

**Architecture:** Sub-agents-as-tools. A new `agent-core/src/dispatch.rs` holds `DispatchAgentTool` (constructed at assemble time with Arc'd dependencies, the `context_tools` injection pattern) and `SubagentSink` (captures the child's text, selectively forwards child `ToolStart`/`ToolResult`/`ServerUsage` to the parent sink on existing wire frames). An additive `Tool::timeout_override()` lets the dispatch call outlive the 120 s tool budget. Three new `RuntimeConfig` knobs gate/bound it.

**Tech Stack:** Rust (tokio, async-trait), existing `agent_core::testkit` (ScriptedModel/PassthroughProtocol/CollectingSink/AlwaysApprove).

**Spec:** `docs/superpowers/specs/2026-07-01-subagent-dispatch-core-design.md` — decisions D1–D13 referenced below.

## Global Constraints

- Two Cargo workspaces; everything here is in `agent/` — run cargo from `/home/kalen/rust-agent-runtime/agent` (`source ~/.cargo/env` if cargo missing).
- Conventional commits: `type(scope): summary`.
- TDD: failing test first, minimal implementation, frequent commits.
- Invariant (spec): the child is never more privileged than the parent — same policy/approval/sandbox **Arc instances**, registry a subset never containing `dispatch_agent`.
- New config fields must be serde-defaulted + partial-merge aware (old on-disk configs keep working).
- Do not change wire.ts / web/ / agent-server wire enums — v1 rides existing frames (spec D9).

---

### Task 1: `Tool::timeout_override` + gate honors it + `LoopConfig: Clone`

**Files:**
- Modify: `agent/crates/agent-tools/src/tool.rs` (add default method)
- Modify: `agent/crates/agent-core/src/loop_.rs` (gate_tool ~line 686; `LoopConfig` struct ~line 47)
- Test: `agent/crates/agent-core/tests/timeout_override.rs` (new)

**Interfaces:**
- Produces: `Tool::timeout_override(&self) -> Option<std::time::Duration>` (default `None`) — per-call `ToolCtx.timeout` becomes `tool.timeout_override().unwrap_or(config.tool_timeout)`.
- Produces: `#[derive(Clone)]` on `agent_core::LoopConfig` (all fields are Arc/PathBuf/scalars).

- [ ] **Step 1: Write the failing integration test**

Create `agent/crates/agent-core/tests/timeout_override.rs`:

```rust
//! Pins spec D7: gate_tool builds ToolCtx with the tool's timeout_override
//! when present, else the loop's tool_timeout.
use agent_core::testkit::{AlwaysApprove, CollectingSink, PassthroughProtocol, Scripted, ScriptedModel};
use agent_core::{AgentLoop, CuratedContext, InMemoryOffloadStore, LoopConfig};
use agent_model::Message;
use agent_policy::RulePolicy;
use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolRegistry, ToolSchema};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Records the ToolCtx.timeout it was executed with.
struct TimeoutProbe {
    name: &'static str,
    override_secs: Option<u64>,
    seen: Mutex<Option<Duration>>,
}
#[async_trait::async_trait]
impl Tool for TimeoutProbe {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "probe"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name.into(),
            description: "probe".into(),
            parameters: serde_json::json!({"type":"object"}),
        }
    }
    fn timeout_override(&self) -> Option<Duration> {
        self.override_secs.map(Duration::from_secs)
    }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: self.name.into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "probe".into(),
        })
    }
    async fn execute(&self, _a: serde_json::Value, ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        *self.seen.lock().unwrap() = Some(ctx.timeout);
        Ok(ToolOutput { content: "ok".into(), display: None })
    }
}

fn run_probe(probe: Arc<TimeoutProbe>) {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let mut reg = ToolRegistry::new();
        reg.register(probe.clone());
        let dir = tempfile::tempdir().unwrap();
        let config = LoopConfig {
            model_limit: 16384,
            max_turns: 3,
            max_retries: 1,
            tool_timeout: Duration::from_secs(5),
            stream_idle_timeout: Duration::from_secs(5),
            workspace: dir.path().to_path_buf(),
            ..LoopConfig::default()
        };
        let agent = AgentLoop::new(
            Arc::new(ScriptedModel::new(vec![
                Scripted::Call("c1".into(), probe.name.into(), "{}".into()),
                Scripted::Text("done".into()),
            ])),
            Arc::new(PassthroughProtocol),
            Arc::new(reg),
            Arc::new(RulePolicy {
                workspace: dir.path().to_path_buf(),
                command_allowlist: vec![],
                command_denylist: vec![],
            }),
            Arc::new(AlwaysApprove),
            Arc::new(CollectingSink::default()),
            config,
        );
        let mut ctx = CuratedContext::new(
            Message::system("s"),
            Arc::new(InMemoryOffloadStore::new()),
            Arc::new(AtomicBool::new(false)),
        );
        agent.run(&mut ctx, "go".into()).await.unwrap();
    });
}

#[test]
fn tool_ctx_uses_timeout_override_when_present() {
    let probe = Arc::new(TimeoutProbe { name: "probe_a", override_secs: Some(555), seen: Mutex::new(None) });
    run_probe(probe.clone());
    assert_eq!(*probe.seen.lock().unwrap(), Some(Duration::from_secs(555)));
}

#[test]
fn tool_ctx_defaults_to_loop_tool_timeout() {
    let probe = Arc::new(TimeoutProbe { name: "probe_b", override_secs: None, seen: Mutex::new(None) });
    run_probe(probe.clone());
    assert_eq!(*probe.seen.lock().unwrap(), Some(Duration::from_secs(5)));
}
```

Note: if `LoopConfig::default()` + struct-update syntax fails to compile because `LoopConfig` isn't `Clone`/has non-Default fields, set every field explicitly the way `agent/crates/agent-core/tests/` siblings do — copy an existing test's `LoopConfig` construction and adjust `tool_timeout`.

- [ ] **Step 2: Run to verify failure**

Run: `cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-core --test timeout_override`
Expected: compile error — `timeout_override` is not a member of trait `Tool`.

- [ ] **Step 3: Implement**

In `agent/crates/agent-tools/src/tool.rs`, after `when_not_to_call`:

```rust
    /// Per-tool execution budget. `None` (default) uses the loop's
    /// `tool_timeout`. Long-lived tools (e.g. sub-agent dispatch) override this;
    /// `execute_isolated`'s 2x backstop scales with it automatically.
    fn timeout_override(&self) -> Option<std::time::Duration> {
        None
    }
```

In `agent/crates/agent-core/src/loop_.rs`:
1. Add `#[derive(Clone)]` directly above `pub struct LoopConfig {` (~line 47).
2. In `gate_tool` (~line 686), change `timeout: self.config.tool_timeout,` to:

```rust
            timeout: tool.timeout_override().unwrap_or(self.config.tool_timeout),
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-core --test timeout_override && cargo test -p agent-tools && cargo test -p agent-core`
Expected: PASS (all).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-tools/src/tool.rs agent/crates/agent-core/src/loop_.rs agent/crates/agent-core/tests/timeout_override.rs
git commit -m "feat(tools): additive Tool::timeout_override honored by gate_tool; LoopConfig: Clone"
```

---

### Task 2: `RuntimeConfig` sub-agent knobs

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs`
- Test: same file's `mod tests`

**Interfaces:**
- Produces: `RuntimeConfig.subagents: bool` (default `true`), `RuntimeConfig.subagent_max_turns: usize` (default `10`), `RuntimeConfig.subagent_timeout_secs: u64` (default `600`) — serde-defaulted, partial-merge aware, set in `from_launch`. Task 6 reads all three.

- [ ] **Step 1: Write the failing tests**

In `runtime_config.rs` `mod tests`, mirroring `max_tool_result_bytes_defaults_and_merges` (~line 443) and `max_tool_result_bytes_partial_file_overrides_only_that_field` (~line 468):

```rust
    #[test]
    fn subagent_fields_default_and_survive_old_files() {
        // from_launch defaults.
        let c = base();
        assert!(c.subagents);
        assert_eq!(c.subagent_max_turns, 10);
        assert_eq!(c.subagent_timeout_secs, 600);

        // Old on-disk file without the fields -> defaults.
        let mut v = serde_json::to_value(&c).unwrap();
        let o = v.as_object_mut().unwrap();
        o.remove("subagents");
        o.remove("subagent_max_turns");
        o.remove("subagent_timeout_secs");
        let parsed: RuntimeConfig = serde_json::from_value(v).unwrap();
        assert!(parsed.subagents);
        assert_eq!(parsed.subagent_max_turns, 10);
        assert_eq!(parsed.subagent_timeout_secs, 600);
    }

    #[test]
    fn subagent_fields_partial_file_overrides_only_those_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.json");
        std::fs::write(
            &path,
            r#"{"subagents": false, "subagent_max_turns": 4, "subagent_timeout_secs": 30}"#,
        )
        .unwrap();
        let b = base();
        let loaded = RuntimeConfig::load_over(b.clone(), &path);
        assert!(!loaded.subagents);
        assert_eq!(loaded.subagent_max_turns, 4);
        assert_eq!(loaded.subagent_timeout_secs, 30);
        assert_eq!(loaded.model, b.model); // absent fields fall back to base
    }
```

(Use the existing `base()` helper in that test module; if it doesn't exist under that name, use whatever helper `max_tool_result_bytes_partial_file_overrides_only_that_field` uses.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-runtime-config subagent_fields`
Expected: compile error — no field `subagents`.

- [ ] **Step 3: Implement**

Follow the `max_tool_result_bytes` pattern exactly (struct field ~line 27, default fn ~line 120, `from_launch` ~line 169, `PartialConfig` field ~line 92, `apply` arm ~line 316):

```rust
    // struct RuntimeConfig — next to `memory`:
    #[serde(default = "default_true")]
    pub subagents: bool,
    #[serde(default = "default_subagent_max_turns")]
    pub subagent_max_turns: usize,
    #[serde(default = "default_subagent_timeout_secs")]
    pub subagent_timeout_secs: u64,
```

```rust
fn default_subagent_max_turns() -> usize {
    10
}
fn default_subagent_timeout_secs() -> u64 {
    600
}
```

```rust
    // from_launch (where memory: true is set):
    subagents: true,
    subagent_max_turns: default_subagent_max_turns(),
    subagent_timeout_secs: default_subagent_timeout_secs(),
```

```rust
    // PartialConfig:
    subagents: Option<bool>,
    subagent_max_turns: Option<usize>,
    subagent_timeout_secs: Option<u64>,
```

```rust
    // apply():
    if let Some(v) = p.subagents {
        self.subagents = v;
    }
    if let Some(v) = p.subagent_max_turns {
        self.subagent_max_turns = v;
    }
    if let Some(v) = p.subagent_timeout_secs {
        self.subagent_timeout_secs = v;
    }
```

If other construction sites fail to compile (e.g. a `Default` impl or server settings mapping), set the same defaults there.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-runtime-config`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A agent/crates/agent-runtime-config
git commit -m "feat(config): subagents / subagent_max_turns / subagent_timeout_secs knobs"
```

---

### Task 3: `SubagentSink` — capture + selective forwarding

**Files:**
- Create: `agent/crates/agent-core/src/dispatch.rs`
- Modify: `agent/crates/agent-core/src/lib.rs` (add `pub mod dispatch; pub use dispatch::*;` alongside the existing pub mods)
- Test: unit `mod tests` inside `dispatch.rs`

**Interfaces:**
- Produces (crate-public, used by Task 4 in the same file):

```rust
pub struct SubagentSink { /* parent, n, Mutex<Capture> */ }
impl SubagentSink {
    pub fn new(parent: Arc<dyn EventSink>, n: u64) -> Self;
    pub fn summary(&self) -> CaptureSummary;
}
pub struct CaptureSummary {
    pub final_text: String,   // last token segment; falls back to all text if blank
    pub tool_calls: u64,
    pub turns: usize,
    pub stop: Option<agent_model::StopReason>,
}
pub fn next_dispatch_n() -> u64;  // process-wide AtomicU64, starts at 1
pub const SUBAGENT_PREAMBLE: &str = /* below */;
```

- [ ] **Step 1: Write the failing tests**

Create `agent/crates/agent-core/src/dispatch.rs` with the test module first (implementation stubs come in Step 3; write tests against the interface above):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentEvent, ContextEvent, EventSink};
    use agent_model::StopReason;
    use agent_tools::{ToolOutput, ToolStatus};
    use std::sync::{Arc, Mutex};

    /// Captures full (kind, id, name) triples — testkit's CollectingSink drops ids.
    #[derive(Default)]
    struct FullSink {
        events: Mutex<Vec<(String, String, String)>>,
    }
    impl EventSink for FullSink {
        fn emit(&self, event: AgentEvent) {
            let triple = match event {
                AgentEvent::ToolStart { id, name, .. } => ("tool_start".into(), id, name),
                AgentEvent::ToolResult { id, name, status, .. } => {
                    (format!("tool_result:{}", status.as_str()), id, name)
                }
                AgentEvent::ServerUsage { prompt_tokens, .. } => {
                    ("server_usage".into(), prompt_tokens.to_string(), String::new())
                }
                // Anything else reaching the parent is a forwarding-table bug —
                // record it so the exact-equality assertion below catches the leak.
                _ => ("unexpected".to_string(), String::new(), String::new()),
            };
            self.events.lock().unwrap().push(triple);
        }
    }

    fn tool_result(id: &str, name: &str) -> AgentEvent {
        AgentEvent::ToolResult {
            id: id.into(),
            name: name.into(),
            status: ToolStatus::Ok,
            output: ToolOutput { content: "r".into(), display: None },
            duration_ms: 1,
        }
    }

    #[test]
    fn forwards_tool_events_rewritten_and_suppresses_the_rest() {
        let parent = Arc::new(FullSink::default());
        let sink = SubagentSink::new(parent.clone(), 7);
        sink.emit(AgentEvent::Token("hi".into()));
        sink.emit(AgentEvent::Reasoning("r".into()));
        sink.emit(AgentEvent::Usage { prompt_tokens: 1, context_limit: 10, turn: 1, max_turns: 5 });
        sink.emit(AgentEvent::ToolStart { id: "c1".into(), name: "echo".into(), args: serde_json::json!({}) });
        sink.emit(tool_result("c1", "echo"));
        sink.emit(AgentEvent::ServerUsage {
            prompt_tokens: 42, completion_tokens: 1, reasoning_tokens: None,
            cached_tokens: None, cost_usd: None, turn_duration_ms: 1, turn: 1,
        });
        sink.emit(AgentEvent::Error("boom".into()));
        sink.emit(AgentEvent::Context(ContextEvent::OverflowRecovery));
        sink.emit(AgentEvent::Done(StopReason::Stop));

        let got = parent.events.lock().unwrap().clone();
        // ONLY ToolStart/ToolResult (rewritten) + ServerUsage (verbatim) forwarded.
        assert_eq!(
            got,
            vec![
                ("tool_start".to_string(), "sub7:c1".to_string(), "sub:echo".to_string()),
                ("tool_result:ok".to_string(), "sub7:c1".to_string(), "sub:echo".to_string()),
                ("server_usage".to_string(), "42".to_string(), String::new()),
            ]
        );
    }

    #[test]
    fn summary_final_text_is_tail_after_last_tool_result() {
        let sink = SubagentSink::new(Arc::new(FullSink::default()), 1);
        sink.emit(AgentEvent::Token("thinking...".into()));
        sink.emit(tool_result("c1", "echo"));
        sink.emit(AgentEvent::Token("final ".into()));
        sink.emit(AgentEvent::Token("answer".into()));
        sink.emit(AgentEvent::Done(StopReason::Stop));
        let s = sink.summary();
        assert_eq!(s.final_text, "final answer");
        assert_eq!(s.tool_calls, 0); // no ToolStart was emitted
        assert_eq!(s.stop, Some(StopReason::Stop));
    }

    #[test]
    fn summary_falls_back_to_all_text_when_tail_is_blank() {
        let sink = SubagentSink::new(Arc::new(FullSink::default()), 1);
        sink.emit(AgentEvent::Token("early words".into()));
        sink.emit(tool_result("c1", "echo"));
        // no tokens after the last tool result
        let s = sink.summary();
        assert_eq!(s.final_text, "early words");
    }

    #[test]
    fn summary_counts_tool_calls_and_turns() {
        let sink = SubagentSink::new(Arc::new(FullSink::default()), 1);
        sink.emit(AgentEvent::Usage { prompt_tokens: 1, context_limit: 10, turn: 1, max_turns: 5 });
        sink.emit(AgentEvent::ToolStart { id: "c1".into(), name: "a".into(), args: serde_json::json!({}) });
        sink.emit(AgentEvent::ToolStart { id: "c2".into(), name: "b".into(), args: serde_json::json!({}) });
        sink.emit(AgentEvent::Usage { prompt_tokens: 2, context_limit: 10, turn: 2, max_turns: 5 });
        let s = sink.summary();
        assert_eq!(s.tool_calls, 2);
        assert_eq!(s.turns, 2);
    }

    #[test]
    fn dispatch_ordinals_are_unique() {
        let a = next_dispatch_n();
        let b = next_dispatch_n();
        assert_ne!(a, b);
    }
}
```

If `StopReason` lacks `PartialEq`, compare with `matches!` instead of `assert_eq!`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-core dispatch`
Expected: compile error — `SubagentSink` undefined (add the `pub mod dispatch;` to lib.rs first so the file compiles at all).

- [ ] **Step 3: Implement**

Top of `dispatch.rs`:

```rust
//! Sub-agent dispatch: sub-agents-as-tools (spec 2026-07-01-subagent-dispatch-core).
use crate::{AgentEvent, EventSink};
use agent_model::StopReason;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Appended to the parent's composed system prompt for every child.
pub const SUBAGENT_PREAMBLE: &str = "You are a sub-agent dispatched by a parent \
agent to complete one self-contained task. Work autonomously: no one can answer \
questions. Your final message is returned verbatim to the parent as the task \
result, so end with a complete, standalone answer.";

static DISPATCH_ORDINAL: AtomicU64 = AtomicU64::new(1);

/// Process-wide dispatch ordinal: keeps forwarded child event ids unique across
/// parallel siblings and across the parent's own tool-call ids (spec D9).
pub fn next_dispatch_n() -> u64 {
    DISPATCH_ORDINAL.fetch_add(1, Ordering::Relaxed)
}

#[derive(Default)]
struct Capture {
    /// Token text split into segments at ToolResult boundaries; the last
    /// segment is the child's final-turn text (spec D10).
    segments: Vec<String>,
    tool_calls: u64,
    turns: usize,
    stop: Option<StopReason>,
}

pub struct CaptureSummary {
    pub final_text: String,
    pub tool_calls: u64,
    pub turns: usize,
    pub stop: Option<StopReason>,
}

/// The child loop's sink: captures the transcript for the tool result and
/// forwards ONLY ToolStart/ToolResult (ids `sub{n}:{id}`, names `sub:{name}`)
/// plus ServerUsage (real cost) to the parent sink — all existing wire frames,
/// so no wire/web/CLI changes (spec D9). Child Token/Done/Error/Context stay
/// private: the parent's streamed transcript must not be corrupted.
pub struct SubagentSink {
    parent: Arc<dyn EventSink>,
    n: u64,
    cap: Mutex<Capture>,
}

impl SubagentSink {
    pub fn new(parent: Arc<dyn EventSink>, n: u64) -> Self {
        Self {
            parent,
            n,
            cap: Mutex::new(Capture { segments: vec![String::new()], ..Capture::default() }),
        }
    }

    pub fn summary(&self) -> CaptureSummary {
        let cap = self.cap.lock().unwrap();
        let tail = cap.segments.last().cloned().unwrap_or_default();
        let final_text = if tail.trim().is_empty() {
            cap.segments.concat().trim().to_string()
        } else {
            tail.trim().to_string()
        };
        CaptureSummary {
            final_text,
            tool_calls: cap.tool_calls,
            turns: cap.turns,
            stop: cap.stop.clone(),
        }
    }
}

impl EventSink for SubagentSink {
    fn emit(&self, event: AgentEvent) {
        let mut cap = self.cap.lock().unwrap();
        match event {
            AgentEvent::Token(t) => {
                cap.segments.last_mut().expect("segments never empty").push_str(&t);
            }
            AgentEvent::ToolStart { id, name, args } => {
                cap.tool_calls += 1;
                drop(cap);
                self.parent.emit(AgentEvent::ToolStart {
                    id: format!("sub{}:{}", self.n, id),
                    name: format!("sub:{name}"),
                    args,
                });
            }
            AgentEvent::ToolResult { id, name, status, output, duration_ms } => {
                cap.segments.push(String::new());
                drop(cap);
                self.parent.emit(AgentEvent::ToolResult {
                    id: format!("sub{}:{}", self.n, id),
                    name: format!("sub:{name}"),
                    status,
                    output,
                    duration_ms,
                });
            }
            e @ AgentEvent::ServerUsage { .. } => {
                drop(cap);
                self.parent.emit(e);
            }
            AgentEvent::Usage { turn, .. } => {
                cap.turns = cap.turns.max(turn);
            }
            AgentEvent::Done(reason) => {
                cap.stop = Some(reason);
            }
            // Suppressed: Reasoning, Approval, Error, Context, SandboxDegraded
            // (spec D9 — child terminal/context events are the tool result's
            // business, not the parent transcript's).
            _ => {}
        }
    }
}
```

Add to `agent/crates/agent-core/src/lib.rs` (with the other pub mods): `pub mod dispatch;` and `pub use dispatch::*;`.

If `StopReason` isn't `Clone`, it derives `Clone` already in agent-model (check `agent-model/src/types.rs:89`); `.clone()` on `Option<StopReason>` needs it — if missing, add `Clone` to its derive list.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-core dispatch`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/dispatch.rs agent/crates/agent-core/src/lib.rs
git commit -m "feat(core): SubagentSink — child transcript capture + selective forwarding on existing frames"
```

---

### Task 4: `DispatchAgentTool` — core tool

**Files:**
- Modify: `agent/crates/agent-core/src/dispatch.rs`
- Test: `agent/crates/agent-core/tests/dispatch_tool.rs` (new)

**Interfaces:**
- Consumes: `SubagentSink`, `next_dispatch_n`, `SUBAGENT_PREAMBLE` (Task 3); `Tool::timeout_override`, `LoopConfig: Clone` (Task 1).
- Produces:

```rust
pub struct DispatchDeps {
    pub model: Arc<dyn agent_model::ModelClient>,
    pub protocol: Arc<dyn agent_model::ToolCallProtocol>,
    pub policy: Arc<dyn agent_policy::PolicyEngine>,
    pub approval: Arc<dyn agent_policy::ApprovalChannel>,
    pub sink: Arc<dyn EventSink>,
    pub base_tools: Vec<Arc<dyn agent_tools::Tool>>,
    pub child_system_prompt: String,
    pub loop_config: LoopConfig,      // max_turns already = subagent_max_turns
    pub max_result_bytes: usize,
    pub subagent_timeout: std::time::Duration,
}
pub struct DispatchAgentTool { /* deps */ }
impl DispatchAgentTool { pub fn new(deps: DispatchDeps) -> Self; }
// implements Tool: name "dispatch_agent", Access::Read intent,
// timeout_override -> Some(deps.subagent_timeout)
```

- [ ] **Step 1: Write the failing tests**

Create `agent/crates/agent-core/tests/dispatch_tool.rs`:

```rust
//! Integration tests for DispatchAgentTool (spec D1-D13 core behaviors).
use agent_core::testkit::{AlwaysApprove, PassthroughProtocol, Scripted, ScriptedModel};
use agent_core::{AgentEvent, DispatchAgentTool, DispatchDeps, EventSink, LoopConfig, SUBAGENT_PREAMBLE};
use agent_policy::{Decision, PolicyEngine, RulePolicy};
use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Full-fidelity parent sink (testkit CollectingSink drops ids).
#[derive(Default)]
struct FullSink {
    events: Mutex<Vec<(String, String, String)>>, // (kind, id, name)
}
impl EventSink for FullSink {
    fn emit(&self, event: AgentEvent) {
        let t = match event {
            AgentEvent::ToolStart { id, name, .. } => ("tool_start".to_string(), id, name),
            AgentEvent::ToolResult { id, name, status, .. } => {
                (format!("tool_result:{}", status.as_str()), id, name)
            }
            AgentEvent::Token(t) => ("token".to_string(), String::new(), t),
            AgentEvent::Done(_) => ("done".to_string(), String::new(), String::new()),
            _ => return,
        };
        self.events.lock().unwrap().push(t);
    }
}

/// A trivial child-visible tool.
struct Echo;
#[async_trait::async_trait]
impl Tool for Echo {
    fn name(&self) -> &str { "echo" }
    fn description(&self) -> &str { "echo" }
    fn schema(&self) -> ToolSchema {
        ToolSchema { name: "echo".into(), description: "echo".into(), parameters: serde_json::json!({"type":"object"}) }
    }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent { tool: "echo".into(), access: Access::Read, paths: vec![], command: None, summary: "echo".into() })
    }
    async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput { content: "echoed".into(), display: None })
    }
}

fn workspace() -> PathBuf {
    std::env::temp_dir()
}

fn child_config(ws: PathBuf) -> LoopConfig {
    LoopConfig {
        model_limit: 16384,
        max_turns: 5,
        max_retries: 1,
        tool_timeout: Duration::from_secs(5),
        stream_idle_timeout: Duration::from_secs(3600),
        workspace: ws,
        ..LoopConfig::default()
    }
}

fn deps(model: ScriptedModel, sink: Arc<dyn EventSink>, base: Vec<Arc<dyn Tool>>) -> DispatchDeps {
    let ws = workspace();
    DispatchDeps {
        model: Arc::new(model),
        protocol: Arc::new(PassthroughProtocol),
        policy: Arc::new(RulePolicy { workspace: ws.clone(), command_allowlist: vec![], command_denylist: vec![] }),
        approval: Arc::new(AlwaysApprove),
        sink,
        base_tools: base,
        child_system_prompt: format!("SYS\n\n{SUBAGENT_PREAMBLE}"),
        loop_config: child_config(ws),
        max_result_bytes: 16 * 1024,
        subagent_timeout: Duration::from_secs(600),
    }
}

fn tool_ctx() -> ToolCtx {
    ToolCtx {
        workspace: workspace(),
        timeout: Duration::from_secs(600),
        cancel: CancellationToken::new(),
        sandbox: Arc::new(agent_tools::HostExecutor),
    }
}

#[tokio::test]
async fn returns_child_final_text_with_footer() {
    let sink = Arc::new(FullSink::default());
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![Scripted::Text("hello from child".into())]),
        sink,
        vec![],
    ));
    let out = tool
        .execute(serde_json::json!({"prompt": "do the thing"}), &tool_ctx())
        .await
        .unwrap();
    assert!(out.content.starts_with("hello from child"), "{}", out.content);
    assert!(out.content.contains("[sub-agent: "), "{}", out.content);
    assert!(out.content.contains("stop: Stop"), "{}", out.content);
}

#[tokio::test]
async fn child_tool_calls_are_forwarded_rewritten_and_tokens_suppressed() {
    let sink = Arc::new(FullSink::default());
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "echo".into(), "{}".into()),
            Scripted::Text("final".into()),
        ]),
        sink.clone(),
        vec![Arc::new(Echo)],
    ));
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .unwrap();
    assert!(out.content.starts_with("final"));
    let events = sink.events.lock().unwrap().clone();
    // Child echo call forwarded with rewritten id/name; NO child token/done leaked.
    assert!(events.iter().any(|(k, i, n)| k == "tool_start" && i.contains(":c1") && i.starts_with("sub") && n == "sub:echo"), "{events:?}");
    assert!(events.iter().any(|(k, _, n)| k == "tool_result:ok" && n == "sub:echo"), "{events:?}");
    assert!(!events.iter().any(|(k, _, _)| k == "token" || k == "done"), "{events:?}");
}

#[tokio::test]
async fn budget_exhausted_child_reports_it() {
    let sink = Arc::new(FullSink::default());
    let mut d = deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "echo".into(), "{}".into()),
            Scripted::Call("c2".into(), "echo".into(), "{}".into()),
        ]),
        sink,
        vec![Arc::new(Echo)],
    );
    d.loop_config.max_turns = 1;
    let tool = DispatchAgentTool::new(d);
    let out = tool.execute(serde_json::json!({"prompt": "p"}), &tool_ctx()).await.unwrap();
    assert!(out.content.contains("turn budget"), "{}", out.content);
    assert!(out.content.contains("stop: BudgetExhausted"), "{}", out.content);
}

#[tokio::test]
async fn tools_allowlist_filters_and_rejects_unknown_names() {
    let sink = Arc::new(FullSink::default());
    // Unknown name -> InvalidArgs listing available.
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![Scripted::Text("x".into())]),
        sink.clone(),
        vec![Arc::new(Echo)],
    ));
    let err = tool
        .execute(serde_json::json!({"prompt": "p", "tools": ["nope"]}), &tool_ctx())
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(ref m) if m.contains("nope") && m.contains("echo")), "{err:?}");

    // Filtered-out tool is unknown to the child (gate rejects it as Denied).
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "echo".into(), "{}".into()),
            Scripted::Text("done".into()),
        ]),
        sink.clone(),
        vec![Arc::new(Echo)],
    ));
    let out = tool
        .execute(serde_json::json!({"prompt": "p", "tools": []}), &tool_ctx())
        .await
        .unwrap();
    assert!(out.content.starts_with("done"));
    let events = sink.events.lock().unwrap().clone();
    assert!(events.iter().any(|(k, _, n)| k == "tool_result:denied" && n == "sub:echo"), "{events:?}");
}

#[tokio::test]
async fn missing_prompt_is_invalid_args() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    let err = tool.execute(serde_json::json!({}), &tool_ctx()).await.unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(_)));
}

#[test]
fn intent_is_readonly_and_auto_allowed() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    let intent = tool.intent(&serde_json::json!({"prompt": "summarize the repo"})).unwrap();
    assert!(matches!(intent.access, Access::Read));
    assert!(intent.paths.is_empty());
    assert!(intent.command.is_none());
    assert!(intent.summary.contains("summarize"));
    let policy = RulePolicy { workspace: workspace(), command_allowlist: vec![], command_denylist: vec![] };
    assert!(matches!(policy.check(&intent), Decision::Allow));
}

#[test]
fn timeout_override_is_the_configured_subagent_timeout() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    assert_eq!(tool.timeout_override(), Some(Duration::from_secs(600)));
}

#[test]
fn schema_describes_required_prompt() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    let s = tool.schema();
    assert_eq!(s.name, "dispatch_agent");
    assert!(agent_tools::required_params_missing_description(&s).is_empty());
    assert!(tool.when_not_to_call().is_some());
}
```

Adjust imports if `HostExecutor` lives elsewhere (`agent_tools::HostExecutor` per `agent-tools/src/sandbox.rs:124`; re-export path may be `agent_tools::sandbox::HostExecutor` — check `agent-tools/src/lib.rs`).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-core --test dispatch_tool`
Expected: compile error — `DispatchAgentTool` undefined.

- [ ] **Step 3: Implement in `dispatch.rs`**

```rust
use crate::{AgentLoop, CuratedContext, InMemoryOffloadStore, LoopConfig, OffloadConfig};
use agent_model::{Message, ModelClient, ToolCallProtocol};
use agent_policy::{ApprovalChannel, PolicyEngine};
use agent_tools::{
    Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolRegistry, ToolSchema,
};
use std::sync::atomic::AtomicBool;

pub struct DispatchDeps {
    pub model: Arc<dyn ModelClient>,
    pub protocol: Arc<dyn ToolCallProtocol>,
    pub policy: Arc<dyn PolicyEngine>,
    pub approval: Arc<dyn ApprovalChannel>,
    /// The parent's (Observed) sink — forwarded child events reach stats/trace/UI.
    pub sink: Arc<dyn EventSink>,
    /// Snapshot of the parent's tools taken BEFORE dispatch_agent and the
    /// parent's context tools were registered (spec D4: structural depth-1).
    pub base_tools: Vec<Arc<dyn Tool>>,
    pub child_system_prompt: String,
    /// Parent LoopConfig clone with max_turns = subagent_max_turns (shares the
    /// parent's sandbox Arc — spec Invariant).
    pub loop_config: LoopConfig,
    pub max_result_bytes: usize,
    pub subagent_timeout: std::time::Duration,
}

pub struct DispatchAgentTool {
    deps: DispatchDeps,
}

impl DispatchAgentTool {
    pub fn new(deps: DispatchDeps) -> Self {
        Self { deps }
    }
}

#[async_trait::async_trait]
impl Tool for DispatchAgentTool {
    fn name(&self) -> &str {
        "dispatch_agent"
    }
    fn description(&self) -> &str {
        "Delegate an independent, multi-step subtask to an isolated sub-agent with \
         its own fresh context window. The sub-agent has the same permissions and \
         tools as you (minus dispatch_agent itself), works autonomously on the \
         prompt you give it, and its final answer is returned as this tool's \
         result. Make the prompt self-contained: the sub-agent cannot see this \
         conversation."
    }
    fn when_not_to_call(&self) -> Option<&str> {
        Some(
            "Do NOT use for a single operation another tool does directly — call \
             that tool. Do not use when the answer depends on this conversation's \
             context (the sub-agent cannot see it), and do not expect it to ask \
             you questions — it runs unattended.",
        )
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "dispatch_agent".into(),
            description: self.description().into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "The complete, self-contained task for the sub-agent: goal, relevant paths/facts, and what to return."
                    },
                    "tools": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Optional allowlist restricting which tools the sub-agent may use (default: all). For focus, not safety — permissions are inherited either way."
                    }
                },
                "required": ["prompt"]
            }),
        }
    }
    fn timeout_override(&self) -> Option<std::time::Duration> {
        Some(self.deps.subagent_timeout)
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
        let head: String = prompt.chars().take(80).collect();
        // Read: spawning computation is not an effect — every effectful child
        // action is gated by the same policy + approval as the parent (spec D3).
        Ok(ToolIntent {
            tool: "dispatch_agent".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: format!("dispatch sub-agent: {head}"),
        })
    }

    async fn execute(&self, args: serde_json::Value, ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .filter(|p| !p.trim().is_empty())
            .ok_or_else(|| ToolError::InvalidArgs("prompt (non-empty string) is required".into()))?
            .to_string();
        let allow: Option<Vec<String>> = match args.get("tools") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::Array(a)) => Some(
                a.iter()
                    .map(|v| {
                        v.as_str().map(str::to_string).ok_or_else(|| {
                            ToolError::InvalidArgs("tools must be an array of strings".into())
                        })
                    })
                    .collect::<Result<_, _>>()?,
            ),
            Some(_) => return Err(ToolError::InvalidArgs("tools must be an array of strings".into())),
        };
        if let Some(names) = &allow {
            let available: Vec<&str> = self.deps.base_tools.iter().map(|t| t.name()).collect();
            for n in names {
                if !available.contains(&n.as_str()) {
                    return Err(ToolError::InvalidArgs(format!(
                        "unknown tool '{n}'; available: {}",
                        available.join(", ")
                    )));
                }
            }
        }

        // Child registry: (filtered) base snapshot + child-bound context tools.
        // dispatch_agent is structurally absent (spec D4: no recursion).
        let mut reg = ToolRegistry::new();
        for t in &self.deps.base_tools {
            if allow.as_ref().is_none_or(|names| names.iter().any(|n| n == t.name())) {
                reg.register(t.clone());
            }
        }
        let store: Arc<dyn crate::OffloadStore> = Arc::new(InMemoryOffloadStore::new());
        let flag = Arc::new(AtomicBool::new(false));
        for t in crate::context_tools(store.clone(), flag.clone(), self.deps.max_result_bytes) {
            reg.register(t);
        }
        let mut child_ctx = CuratedContext::new(
            Message::system(self.deps.child_system_prompt.clone()),
            store,
            flag,
        )
        .with_offload_config(OffloadConfig {
            max_result_bytes: self.deps.max_result_bytes,
            ..OffloadConfig::default()
        });

        let sink = Arc::new(SubagentSink::new(self.deps.sink.clone(), next_dispatch_n()));
        let child = AgentLoop::new(
            self.deps.model.clone(),
            self.deps.protocol.clone(),
            Arc::new(reg),
            self.deps.policy.clone(),
            self.deps.approval.clone(),
            sink.clone(),
            self.deps.loop_config.clone(),
        );

        // Parent cancel propagates down; child self-cancel never travels up (D8).
        let child_cancel = ctx.cancel.child_token();
        let run = child.run_with_cancel(&mut child_ctx, prompt, child_cancel.clone());
        match tokio::time::timeout(ctx.timeout, run).await {
            Err(_elapsed) => {
                child_cancel.cancel();
                return Err(ToolError::Timeout);
            }
            Ok(Err(e)) => {
                return Err(ToolError::Failed {
                    message: format!("sub-agent failed: {e}"),
                    stderr: None,
                })
            }
            Ok(Ok(())) => {}
        }
        if ctx.cancel.is_cancelled() {
            return Err(ToolError::Failed { message: "sub-agent cancelled".into(), stderr: None });
        }

        let s = sink.summary();
        let mut content = s.final_text;
        if matches!(s.stop, Some(StopReason::BudgetExhausted)) {
            content = format!("[sub-agent hit its turn budget before finishing]\n{content}");
        }
        let stop = s.stop.unwrap_or(StopReason::Stop);
        content.push_str(&format!(
            "\n\n[sub-agent: {} turns, {} tool calls, stop: {stop:?}]",
            s.turns, s.tool_calls
        ));
        Ok(ToolOutput { content, display: None })
    }
}
```

(`is_none_or` needs Rust ≥1.82; if the toolchain rejects it use `allow.as_ref().map_or(true, |names| …)`.)

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-core --test dispatch_tool && cargo test -p agent-core`
Expected: PASS (8 tests + no regressions).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/dispatch.rs agent/crates/agent-core/tests/dispatch_tool.rs
git commit -m "feat(core): DispatchAgentTool — nested AgentLoop as a tool with inherited policy/approval"
```

---

### Task 5: Guard tests — cancellation, timeout, recursion, policy inheritance, parallelism

**Files:**
- Modify: `agent/crates/agent-core/tests/dispatch_tool.rs` (append tests; fix `dispatch.rs` only if a test exposes a bug)

**Interfaces:**
- Consumes: everything from Task 4 (same helpers: `deps`, `tool_ctx`, `FullSink`, `Echo`, `child_config`).

- [ ] **Step 1: Append the failing/verifying tests**

```rust
/// Records approval requests; replies with a fixed response.
struct RecordingApproval {
    seen: Mutex<Vec<String>>,
    reply: agent_policy::ApprovalResponse,
}
#[async_trait::async_trait]
impl agent_policy::ApprovalChannel for RecordingApproval {
    async fn request(&self, req: agent_policy::ApprovalRequest) -> agent_policy::ApprovalResponse {
        self.seen.lock().unwrap().push(req.intent.summary.clone());
        self.reply.clone()
    }
}

/// A Write-access tool: policy says Ask, so the shared approval channel decides.
struct Writey;
#[async_trait::async_trait]
impl Tool for Writey {
    fn name(&self) -> &str { "writey" }
    fn description(&self) -> &str { "writes" }
    fn schema(&self) -> ToolSchema {
        ToolSchema { name: "writey".into(), description: "writes".into(), parameters: serde_json::json!({"type":"object"}) }
    }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent { tool: "writey".into(), access: Access::Write, paths: vec![], command: None, summary: "write something".into() })
    }
    async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput { content: "wrote".into(), display: None })
    }
}

#[tokio::test]
async fn child_ask_routes_to_the_shared_approval_channel_and_deny_sticks() {
    let sink = Arc::new(FullSink::default());
    let approval = Arc::new(RecordingApproval { seen: Mutex::new(vec![]), reply: agent_policy::ApprovalResponse::Deny });
    let mut d = deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "writey".into(), "{}".into()),
            Scripted::Text("done".into()),
        ]),
        sink.clone(),
        vec![Arc::new(Writey)],
    );
    d.approval = approval.clone();
    let tool = DispatchAgentTool::new(d);
    let out = tool.execute(serde_json::json!({"prompt": "p"}), &tool_ctx()).await.unwrap();
    // The Ask reached the PARENT's channel (spec Invariant / D2)...
    assert_eq!(approval.seen.lock().unwrap().as_slice(), &["write something".to_string()]);
    // ...and the denial reached the child (forwarded as a denied tool_result).
    let events = sink.events.lock().unwrap().clone();
    assert!(events.iter().any(|(k, _, n)| k == "tool_result:denied" && n == "sub:writey"), "{events:?}");
    assert!(out.content.starts_with("done"));
}

#[tokio::test]
async fn child_ask_approve_executes() {
    let sink = Arc::new(FullSink::default());
    let approval = Arc::new(RecordingApproval { seen: Mutex::new(vec![]), reply: agent_policy::ApprovalResponse::Approve });
    let mut d = deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "writey".into(), "{}".into()),
            Scripted::Text("done".into()),
        ]),
        sink.clone(),
        vec![Arc::new(Writey)],
    );
    d.approval = approval;
    let tool = DispatchAgentTool::new(d);
    tool.execute(serde_json::json!({"prompt": "p"}), &tool_ctx()).await.unwrap();
    let events = sink.events.lock().unwrap().clone();
    assert!(events.iter().any(|(k, _, n)| k == "tool_result:ok" && n == "sub:writey"), "{events:?}");
}

#[tokio::test]
async fn child_cannot_recurse_into_dispatch_agent() {
    let sink = Arc::new(FullSink::default());
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "dispatch_agent".into(), r#"{"prompt":"nested"}"#.into()),
            Scripted::Text("done".into()),
        ]),
        sink.clone(),
        vec![Arc::new(Echo)],
    ));
    let out = tool.execute(serde_json::json!({"prompt": "p"}), &tool_ctx()).await.unwrap();
    assert!(out.content.starts_with("done"));
    // The child's gate rejected the unknown tool (Denied, "not found").
    let events = sink.events.lock().unwrap().clone();
    assert!(events.iter().any(|(k, _, n)| k == "tool_result:denied" && n == "sub:dispatch_agent"), "{events:?}");
}

#[tokio::test]
async fn pre_cancelled_parent_token_cancels_the_child() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![Scripted::Text("never returned".into())]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    let ctx = tool_ctx();
    ctx.cancel.cancel();
    let err = tool.execute(serde_json::json!({"prompt": "p"}), &ctx).await.unwrap_err();
    assert!(matches!(err, ToolError::Failed { ref message, .. } if message.contains("cancelled")), "{err:?}");
}

#[tokio::test(start_paused = true)]
async fn wall_clock_timeout_cancels_the_child_and_reports_timeout() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![Scripted::HangOpen]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    let mut ctx = tool_ctx();
    ctx.timeout = Duration::from_secs(1);
    let started = tokio::time::Instant::now();
    let err = tool.execute(serde_json::json!({"prompt": "p"}), &ctx).await.unwrap_err();
    assert!(matches!(err, ToolError::Timeout), "{err:?}");
    assert_eq!(started.elapsed(), Duration::from_secs(1)); // virtual time: exactly the budget
}

#[tokio::test]
async fn parallel_dispatches_get_distinct_ordinals_and_both_complete() {
    let sink = Arc::new(FullSink::default());
    let mk = |sink: Arc<FullSink>| {
        DispatchAgentTool::new(deps(
            ScriptedModel::new(vec![
                Scripted::Call("c1".into(), "echo".into(), "{}".into()),
                Scripted::Text("done".into()),
            ]),
            sink,
            vec![Arc::new(Echo)],
        ))
    };
    let (a, b) = (mk(sink.clone()), mk(sink.clone()));
    let (ra, rb) = tokio::join!(
        a.execute(serde_json::json!({"prompt": "a"}), &tool_ctx()),
        b.execute(serde_json::json!({"prompt": "b"}), &tool_ctx()),
    );
    ra.unwrap();
    rb.unwrap();
    // Two children each made one echo call; the two forwarded start ids carry
    // distinct sub{n} prefixes even though both children used child-id c1.
    let ids: Vec<String> = sink.events.lock().unwrap().iter()
        .filter(|(k, _, _)| k == "tool_start")
        .map(|(_, id, _)| id.clone())
        .collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_ne!(ids[0], ids[1], "{ids:?}");
}
```

Note on `pre_cancelled_parent_token_cancels_the_child`: `tool_ctx()` returns an owned `ToolCtx`; `ctx.cancel.cancel()` works on the owned token before passing `&ctx`.

- [ ] **Step 2: Run**

Run: `cargo test -p agent-core --test dispatch_tool`
Expected: all pass if Task 4's implementation is correct. Any failure here is a real bug in `dispatch.rs` — fix `dispatch.rs` (not the test) until green. Likely trouble spots: the timeout arm must cancel the child token *before* returning; `RecordingApproval` needs `ApprovalResponse: Clone` (it is — check `agent-policy/src/engine.rs:23`; if not, match-and-reconstruct).

- [ ] **Step 3: Commit**

```bash
git add agent/crates/agent-core/tests/dispatch_tool.rs agent/crates/agent-core/src/dispatch.rs
git commit -m "test(core): dispatch guards — cancel propagation, paused-clock timeout, no-recursion, approval inheritance, parallel ordinals"
```

---

### Task 6: Assembly — snapshot, construction, registration, config gate

**Files:**
- Modify: `agent/crates/agent-tools/src/registry.rs` (add `all()`)
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs`
- Test: both files' `mod tests`

**Interfaces:**
- Consumes: `DispatchAgentTool::new(DispatchDeps)`, `SUBAGENT_PREAMBLE` (Tasks 3–4); config knobs (Task 2); `LoopConfig: Clone` (Task 1).
- Produces: `ToolRegistry::all(&self) -> Vec<Arc<dyn Tool>>`; `BuiltLoop.dispatch_base_names: Option<Vec<String>>` (`#[cfg(test)]`, for the snapshot pin); `dispatch_agent` registered iff `cfg.subagents`.

- [ ] **Step 1: Write the failing tests**

In `agent/crates/agent-tools/src/registry.rs` `mod tests` (create if absent):

```rust
    fn fake(name: &'static str) -> Arc<dyn Tool> {
        use crate::{Access, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
        struct F(&'static str);
        #[async_trait::async_trait]
        impl Tool for F {
            fn name(&self) -> &str { self.0 }
            fn description(&self) -> &str { "fake" }
            fn schema(&self) -> ToolSchema {
                ToolSchema { name: self.0.into(), description: "fake".into(), parameters: serde_json::json!({"type":"object"}) }
            }
            fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
                Ok(ToolIntent { tool: self.0.into(), access: Access::Read, paths: vec![], command: None, summary: "x".into() })
            }
            async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx) -> Result<ToolOutput, ToolError> {
                Ok(ToolOutput { content: "ok".into(), display: None })
            }
        }
        Arc::new(F(name))
    }

    #[test]
    fn all_returns_every_registered_tool() {
        let mut r = ToolRegistry::new();
        assert!(r.all().is_empty());
        r.register(fake("a"));
        r.register(fake("b"));
        let mut names: Vec<String> = r.all().iter().map(|t| t.name().to_string()).collect();
        names.sort();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }
```

In `agent/crates/agent-runtime-config/src/assemble.rs` `mod tests`:

```rust
    #[test]
    fn registers_dispatch_agent_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let built = assemble_loop(&cfg(), parts(dir.path().to_path_buf(), vec![]));
        assert!(built.registered_names.iter().any(|n| n == "dispatch_agent"));
    }

    #[test]
    fn omits_dispatch_agent_when_subagents_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.subagents = false;
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        assert!(!built.registered_names.iter().any(|n| n == "dispatch_agent"));
        assert!(built.dispatch_base_names.is_none());
    }

    #[test]
    fn child_base_snapshot_excludes_context_tools_and_dispatch_itself() {
        let dir = tempfile::tempdir().unwrap();
        let built = assemble_loop(&cfg(), parts(dir.path().to_path_buf(), vec![]));
        let base = built.dispatch_base_names.expect("subagents on by default");
        assert!(!base.iter().any(|n| n == "dispatch_agent"), "{base:?}");
        assert!(!base.iter().any(|n| n == "context_recall" || n == "context_compact"), "{base:?}");
        // Sanity: real tools are in the snapshot.
        assert!(base.iter().any(|n| n == "read_file"), "{base:?}");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-tools all_returns && cargo test -p agent-runtime-config dispatch`
Expected: compile errors (`all` and `dispatch_base_names` undefined; `subagents` field exists from Task 2).

- [ ] **Step 3: Implement**

`agent/crates/agent-tools/src/registry.rs`:

```rust
    /// Every registered tool (arbitrary order). Cheap: Arc clones.
    pub fn all(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.values().cloned().collect()
    }
```

`assemble.rs` — reorder and extend `assemble_loop` (current structure at lines 85–175):

1. After skill-tool registration (~line 98) and **before** `agent_core::context_tools` registration, add:

```rust
    // Snapshot for sub-agent children BEFORE context tools (child gets its own,
    // bound to a per-dispatch store/flag) and before dispatch itself (spec D4:
    // structural no-recursion). The POSITION of this line is the invariant.
    let child_base = cfg.subagents.then(|| registry.all());
```

2. Move the `policy` construction (~lines 138–142) and the `ObservedSink` construction (~lines 146–150) to just after the `system_prompt` composition — both must exist before the dispatch tool is built. Bind the loop config once:

```rust
    let loop_config = loop_config_from(cfg, parts.workspace.clone(), parts.stream_idle_timeout);
```

3. Register the dispatch tool (still before the `#[cfg(test)]` schema snapshot — move that snapshot down so it sees `dispatch_agent`):

```rust
    #[cfg(test)]
    let dispatch_base_names: Option<Vec<String>> = child_base
        .as_ref()
        .map(|b| b.iter().map(|t| t.name().to_string()).collect());
    if let Some(child_base) = child_base {
        let mut child_config = loop_config.clone();
        child_config.max_turns = cfg.subagent_max_turns;
        registry.register(Arc::new(agent_core::DispatchAgentTool::new(
            agent_core::DispatchDeps {
                model: parts.model.clone(),
                protocol: pick_protocol(&cfg.protocol),
                policy: policy.clone(),
                approval: parts.approval.clone(),
                sink: sink.clone(),
                base_tools: child_base,
                child_system_prompt: format!("{system_prompt}\n\n{}", agent_core::SUBAGENT_PREAMBLE),
                loop_config: child_config,
                max_result_bytes: cfg.max_tool_result_bytes,
                subagent_timeout: Duration::from_secs(cfg.subagent_timeout_secs),
            },
        )));
    }
```

(`policy` is `Arc<RulePolicy>`; `policy.clone()` coerces to `Arc<dyn PolicyEngine>` at the field. If inference balks, bind explicitly first: `let p: Arc<dyn agent_policy::PolicyEngine> = policy.clone();`. In non-test builds `dispatch_base_names` doesn't exist — the `#[cfg(test)]` binding and the `BuiltLoop` field are both test-gated, mirroring `registered_names`.)

4. `AgentLoop::new(...)` now takes the prebuilt `loop_config` variable instead of calling `loop_config_from` inline; `BuiltLoop` gains:

```rust
    /// Child-base snapshot names when subagents are enabled — pins the "snapshot
    /// excludes context tools + dispatch itself" invariant.
    #[cfg(test)]
    pub dispatch_base_names: Option<Vec<String>>,
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-tools && cargo test -p agent-runtime-config`
Expected: PASS — including the pre-existing contract tests (`every_required_param_is_described_in_the_assembled_registry` now covers `dispatch_agent`'s schema; `confusable_tools_carry_disambiguation` unchanged — `dispatch_agent` is not in `CONFUSABLE_TOOLS`).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-tools/src/registry.rs agent/crates/agent-runtime-config/src/assemble.rs
git commit -m "feat(runtime-config): assemble dispatch_agent — pre-context child-base snapshot, gated by cfg.subagents"
```

---

### Task 7: `TerminalApproval` prompt serialization

**Files:**
- Modify: `agent/crates/agent-cli/src/approval.rs`
- Test: same file's `mod tests`

**Interfaces:**
- Self-contained. Existing public API (`TerminalApproval::new(timeout)`, `Default`) unchanged.

- [ ] **Step 1: Write the failing test**

Append to `approval.rs` `mod tests`:

```rust
    #[tokio::test]
    async fn concurrent_requests_serialize() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let live = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let (l, m) = (live.clone(), max.clone());
        let ch = Arc::new(TerminalApproval::with_prompt(
            Duration::from_secs(5),
            Arc::new(move |_summary: String| {
                let now = l.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(20));
                l.fetch_sub(1, Ordering::SeqCst);
                ApprovalResponse::Approve
            }),
        ));
        let (a, b) = tokio::join!(ch.request(req()), ch.request(req()));
        assert!(matches!(a, ApprovalResponse::Approve));
        assert!(matches!(b, ApprovalResponse::Approve));
        // Two children prompting at once must never overlap on stdin (spec D12).
        assert_eq!(max.load(Ordering::SeqCst), 1);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-cli concurrent_requests`
Expected: compile error — `with_prompt` undefined.

- [ ] **Step 3: Implement**

Refactor `TerminalApproval` so the blocking prompt is an injectable closure and requests hold an async mutex:

```rust
type BlockingPrompt = std::sync::Arc<dyn Fn(String) -> ApprovalResponse + Send + Sync>;

pub struct TerminalApproval {
    timeout: Duration,
    /// Serializes concurrent requesters (parallel sub-agents both hitting Ask)
    /// so prompts never interleave on stdin.
    gate: tokio::sync::Mutex<()>,
    prompt: BlockingPrompt,
}

fn stdin_prompt(summary: String) -> ApprovalResponse {
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
}

impl TerminalApproval {
    #[allow(dead_code)]
    pub fn new(timeout: Duration) -> Self {
        Self { timeout, gate: tokio::sync::Mutex::new(()), prompt: std::sync::Arc::new(stdin_prompt) }
    }
    #[cfg(test)]
    fn with_prompt(timeout: Duration, prompt: BlockingPrompt) -> Self {
        Self { timeout, gate: tokio::sync::Mutex::new(()), prompt }
    }
}
```

`Default` mirrors `new(DEFAULT_TERMINAL_APPROVAL_TIMEOUT)`. In `request`, take the gate for the whole prompt (keep the existing orphaned-thread NOTE comment), and call the closure from `spawn_blocking`:

```rust
    async fn request(&self, req: ApprovalRequest) -> ApprovalResponse {
        let _serialized = self.gate.lock().await;
        let summary = req.intent.summary.clone();
        let prompt = self.prompt.clone();
        let handle = tokio::task::spawn_blocking(move || prompt(summary));
        match tokio::time::timeout(self.timeout, handle).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(_join_err)) => ApprovalResponse::Deny,
            Err(_elapsed) => {
                eprintln!("\nApproval timed out; denying.");
                ApprovalResponse::Deny
            }
        }
    }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-cli`
Expected: PASS (both approval tests, incl. the pre-existing timeout test).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-cli/src/approval.rs
git commit -m "fix(cli): serialize concurrent TerminalApproval prompts (parallel sub-agents)"
```

---

### Task 8: Workspace sweep + CI gate

**Files:**
- None expected; fix fallout only.

- [ ] **Step 1: Full CI**

Run: `cd /home/kalen/rust-agent-runtime && bash scripts/ci.sh`
Expected: fmt + clippy + full `agent/` test suite + web typecheck/vitest all green. Fix any fallout minimally (e.g. clippy nits in new code, other crates constructing `RuntimeConfig` literally that now miss fields — add the defaults).

- [ ] **Step 2: Spec cross-check**

Re-read `docs/superpowers/specs/2026-07-01-subagent-dispatch-core-design.md` sections "Decisions" (D1–D13) and "Testing"; confirm each has landed or is explicitly listed under "Out of scope". Report any gap instead of silently skipping.

- [ ] **Step 3: Commit (only if fixes were needed)**

```bash
git add -A && git commit -m "chore: workspace sweep for sub-agent dispatch"
```
