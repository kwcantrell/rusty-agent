# Sub-agent Surfaces & Observability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give sub-agent activity first-class attribution on every surface — `parent_id` on forwarded frames, full child transcript in the session trace, undistorted stats, indented CLI/web rendering — while the deployed old SPA keeps rendering byte-identically.

**Architecture:** `ToolCtx.call_id` (filled by `gate_tool`) becomes the lineage root; `AgentEvent::{ToolStart,ToolResult,ServerUsage}` gain `parent_id: Option<String>` mirrored as `skip_serializing_if` optional fields on the wire (`ServerEvent`) and trace (`TraceEvent`) schemas; `SubagentSink` stamps forwards with the dispatch call's id and tees suppressed child events to a `SubagentTrace` tap implemented over `TraceWriter` (record-level `sub: Option<u64>`); `SessionStats` grows subset counters keyed on `parent_id`; CLI indents attributed rows; the web reducer switches to id-first tool correlation and flat `parentId` nesting.

**Tech Stack:** Rust (serde, tokio), React/TS (vitest) — existing patterns only, no new dependencies.

**Spec:** `docs/superpowers/specs/2026-07-02-subagent-observability-design.md` (decisions E1–E9).

## Global Constraints

- Old-SPA byte-compat is the invariant: every new serialized field is `Option` + `#[serde(skip_serializing_if = "Option::is_none")]` (Rust) / optional (TS); a session with no sub-agents serializes byte-identically to today. No new top-level frame types. No event that v1 suppressed may start reaching a frontend.
- Two Cargo workspaces; all Rust work is in `agent/` — run cargo from `/home/kalen/rust-agent-runtime/agent` (`source ~/.cargo/env` if missing). Web work: `cd web && npm test` / `npm run typecheck`.
- Conventional commits; TDD (failing test → minimal code → green → commit).
- `agent-core` cannot depend on `agent-runtime-config` — the child-trace hook is a trait in agent-core, implemented in runtime-config.

---

### Task 1: `ToolCtx.call_id` — the lineage root

**Files:**
- Modify: `agent/crates/agent-tools/src/types.rs` (~line 131, `ToolCtx`)
- Modify: `agent/crates/agent-core/src/loop_.rs` (~line 684, `gate_tool`)
- Modify (sweep): every `ToolCtx {` literal — 31 sites across 15 files (grep `ToolCtx {`), incl. `agent-core/src/dispatch.rs` (`exec_ctx`), `agent-core/tests/dispatch_tool.rs` (`tool_ctx`), `agent-core/src/context_tools.rs` tests, agent-tools/memory/mcp/http/skills tests
- Test: `agent/crates/agent-core/tests/timeout_override.rs` (extend the probe)

**Interfaces:**
- Produces: `ToolCtx.call_id: String` — `gate_tool` fills it with the model's tool_call id. Task 3 reads `ctx.call_id` in `DispatchAgentTool::execute`. Test helpers set fixed ids: `dispatch_tool.rs::tool_ctx()` MUST use `call_id: "d1".into()` (Task 3's integration tests assert `parent_id == "d1"`).

- [ ] **Step 1: Write the failing test**

In `agent/crates/agent-core/tests/timeout_override.rs`, extend `TimeoutProbe` to also record the call id, and add one test:

```rust
// In TimeoutProbe: add field
//     seen_call_id: Mutex<Option<String>>,
// and in execute(), after recording the timeout:
//     *self.seen_call_id.lock().unwrap() = Some(ctx.call_id.clone());

#[test]
fn tool_ctx_carries_the_model_call_id() {
    let probe = Arc::new(TimeoutProbe {
        name: "probe_c",
        override_secs: None,
        seen: Mutex::new(None),
        seen_call_id: Mutex::new(None),
    });
    run_probe(probe.clone()); // run_probe scripts Scripted::Call("c1", ...)
    assert_eq!(probe.seen_call_id.lock().unwrap().as_deref(), Some("c1"));
}
```

(Update the two existing `TimeoutProbe` constructions with the new field.)

- [ ] **Step 2: Run to verify failure**

Run: `cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-core --test timeout_override`
Expected: compile error — `ToolCtx` has no field `call_id`.

- [ ] **Step 3: Implement**

`agent-tools/src/types.rs`, add to `ToolCtx`:

```rust
    /// The tool_call id this execution serves (`gate_tool` fills it from the
    /// model's call). Lineage root for sub-agent attribution (spec E2).
    pub call_id: String,
```

`agent-core/src/loop_.rs` `gate_tool` (~line 684), in the `ToolCtx` literal:

```rust
            call_id: call.id.clone(),
```

(`call.id` is moved into `ReadyCall` afterwards — clone here.)

Sweep: `grep -rn "ToolCtx {" agent/crates --include=*.rs` and add `call_id` to every literal. Tests use a descriptive fixed id (`"test".into()`), EXCEPT `agent-core/tests/dispatch_tool.rs::tool_ctx()` and `agent-core/src/dispatch.rs::exec_ctx()` which use `"d1".into()` (Task 3 depends on it).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-core --test timeout_override && cargo build --workspace && cargo test -p agent-tools -p agent-core`
Expected: PASS, whole workspace compiles.

- [ ] **Step 5: Commit**

```bash
git add -A agent
git commit -m "feat(tools): ToolCtx.call_id — gate_tool records the model call id per execution"
```

---

### Task 2: `parent_id` through event, wire, and trace schemas

**Files:**
- Modify: `agent/crates/agent-core/src/event.rs` (ToolStart ~79, ToolResult ~85, ServerUsage ~70)
- Modify (sweep): `agent/crates/agent-core/src/loop_.rs` (emissions at ~380, ~557, ~571, ~611 → `parent_id: None`), `agent-core/src/dispatch.rs` (SubagentSink forwards + tests → `parent_id: None` placeholder; Task 3 replaces), any other construction site the compiler finds
- Modify: `agent/crates/agent-server/src/wire.rs` (ServerEvent ToolStart ~43, ToolResult ~48, ServerUsage ~30 + `server_event_from` ~163)
- Modify: `agent/crates/agent-runtime-config/src/trace.rs` (TraceEvent mirrors ~176-183 + `trace_event`)
- Test: `wire.rs` and `trace.rs` `mod tests`

**Interfaces:**
- Consumes: nothing new.
- Produces: `AgentEvent::ToolStart{ .., parent_id: Option<String> }`, same on `ToolResult` and `ServerUsage`; `ServerEvent`/`TraceEvent` counterparts with `#[serde(skip_serializing_if = "Option::is_none")] parent_id`. Tasks 3–7 rely on these exact field names.

- [ ] **Step 1: Write the failing tests**

In `agent-server/src/wire.rs` `mod tests` (match the file's existing test style for constructing events):

```rust
    #[test]
    fn parent_id_absent_from_json_when_none_and_present_when_some() {
        let mk = |parent_id: Option<String>| agent_core::AgentEvent::ToolStart {
            id: "c1".into(),
            name: "echo".into(),
            args: serde_json::json!({}),
            parent_id,
        };
        let none = serde_json::to_string(&server_event_from(mk(None)).unwrap()).unwrap();
        assert!(!none.contains("parent_id"), "old-SPA byte-compat broken: {none}");
        let some = serde_json::to_string(&server_event_from(mk(Some("d1".into()))).unwrap()).unwrap();
        assert!(some.contains(r#""parent_id":"d1""#), "{some}");
    }
```

In `agent-runtime-config/src/trace.rs` `mod tests` (the file already has record-then-read-line tests — follow them):

```rust
    #[test]
    fn trace_parent_id_skipped_when_none_present_when_some() {
        let dir = tempfile::tempdir().unwrap();
        let w = TraceWriter::create(dir.path(), 1024 * 1024).unwrap();
        w.record(&agent_core::AgentEvent::ToolStart {
            id: "a".into(), name: "t".into(), args: serde_json::json!({}), parent_id: None,
        });
        w.record(&agent_core::AgentEvent::ToolStart {
            id: "b".into(), name: "t".into(), args: serde_json::json!({}), parent_id: Some("d1".into()),
        });
        let content = std::fs::read_to_string(/* the writer's file path — reuse the
            existing tests' helper for locating the session file */).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert!(!lines[1].contains("parent_id"), "{}", lines[1]); // [0] is the header
        assert!(lines[2].contains(r#""parent_id":"d1""#), "{}", lines[2]);
    }
```

(Adapt `TraceWriter::create`'s real signature and the file-locating helper from the existing tests in that module — do not invent new plumbing.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-server parent_id ; cargo test -p agent-runtime-config trace_parent_id`
Expected: compile error — no field `parent_id`.

- [ ] **Step 3: Implement**

`event.rs` — add to each of the three variants (doc comment on the first):

```rust
        /// Set when this event belongs to a sub-agent: the dispatching
        /// `dispatch_agent` call's id (spec 2026-07-02 E1/E2).
        parent_id: Option<String>,
```

Compiler-led sweep: every construction gets `parent_id: None` for now (loop_.rs emissions are genuinely `None`; dispatch.rs forwards get a placeholder `None` that Task 3 replaces). Match sites: patterns with `..` are untouched; full destructures gain the binding.

`wire.rs` — the three `ServerEvent` variants gain:

```rust
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_id: Option<String>,
```

and `server_event_from` maps `parent_id` straight through on all three arms.

`trace.rs` — `TraceEvent::{ToolStart,ToolResult,ServerUsage}` gain:

```rust
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_id: Option<&'a str>,
```

and `trace_event` maps `parent_id: parent_id.as_deref()` on the three arms.

- [ ] **Step 4: Run to verify pass**

Run: `cargo build --workspace && cargo test -p agent-core -p agent-server -p agent-runtime-config`
Expected: PASS everywhere (existing wire/trace golden tests must still pass — the None case serializes identically).

- [ ] **Step 5: Commit**

```bash
git add -A agent
git commit -m "feat(core): parent_id attribution on ToolStart/ToolResult/ServerUsage across event, wire, and trace schemas"
```

---

### Task 3: Dispatch stamps lineage + child-trace tap + allowlist fold

**Files:**
- Modify: `agent/crates/agent-core/src/dispatch.rs`
- Test: unit `mod tests` in `dispatch.rs` + `agent/crates/agent-core/tests/dispatch_tool.rs`

**Interfaces:**
- Consumes: `ToolCtx.call_id` (Task 1, `"d1"` in the test helpers); `parent_id` fields (Task 2).
- Produces:

```rust
pub trait SubagentTrace: Send + Sync {
    /// Record one non-forwarded child event, attributed to dispatch ordinal `n`.
    fn record(&self, n: u64, event: &AgentEvent);
}
// DispatchDeps gains: pub child_trace: Option<Arc<dyn SubagentTrace>>,
// SubagentSink::new(parent: Arc<dyn EventSink>, n: u64,
//                   parent_call_id: String,
//                   child_trace: Option<Arc<dyn SubagentTrace>>) -> Self
```

Task 4 implements `SubagentTrace` in runtime-config and wires `child_trace` in assemble.

- [ ] **Step 1: Write the failing tests**

Unit tests in `dispatch.rs` (adapt the existing `forwards_tool_events_rewritten_and_suppresses_the_rest` — `FullSink` triples become quads with the parent field):

```rust
    // FullSink now records (kind, id, name, parent) — extend its match arms:
    //   ToolStart { id, name, parent_id, .. } => ("tool_start".into(), id, name, parent_id.unwrap_or_default()),
    //   ToolResult { id, name, status, parent_id, .. } => (format!("tool_result:{}", status.as_str()), id, name, parent_id.unwrap_or_default()),
    //   ServerUsage { prompt_tokens, parent_id, .. } => ("server_usage".into(), prompt_tokens.to_string(), String::new(), parent_id.unwrap_or_default()),

    /// Records (ordinal, kind-name) for every tapped event.
    #[derive(Default)]
    struct TapSpy {
        seen: Mutex<Vec<(u64, &'static str)>>,
    }
    impl SubagentTrace for TapSpy {
        fn record(&self, n: u64, event: &AgentEvent) {
            let kind = match event {
                AgentEvent::Token(_) => "token",
                AgentEvent::Reasoning(_) => "reasoning",
                AgentEvent::Usage { .. } => "usage",
                AgentEvent::Done(_) => "done",
                AgentEvent::Error(_) => "error",
                AgentEvent::Context(_) => "context",
                AgentEvent::Approval(_) => "approval",
                AgentEvent::SandboxDegraded { .. } => "sandbox_degraded",
                AgentEvent::ToolStart { .. } | AgentEvent::ToolResult { .. }
                | AgentEvent::ServerUsage { .. } => "FORWARDED-KIND-MUST-NOT-BE-TAPPED",
            };
            self.seen.lock().unwrap().push((n, kind));
        }
    }

    #[test]
    fn forwards_carry_parent_id_and_tap_gets_exactly_the_suppressed_kinds() {
        let parent = Arc::new(FullSink::default());
        let tap = Arc::new(TapSpy::default());
        let sink = SubagentSink::new(parent.clone(), 7, "d1".into(), Some(tap.clone()));
        sink.emit(AgentEvent::Token("hi".into()));
        sink.emit(AgentEvent::ToolStart { id: "c1".into(), name: "echo".into(), args: serde_json::json!({}), parent_id: None });
        sink.emit(tool_result("c1", "echo"));
        sink.emit(AgentEvent::ServerUsage { prompt_tokens: 42, completion_tokens: 1, reasoning_tokens: None, cached_tokens: None, cost_usd: None, turn_duration_ms: 1, turn: 1, parent_id: None });
        sink.emit(AgentEvent::Error("boom".into()));
        sink.emit(AgentEvent::Done(StopReason::Stop));

        // Forwards stamped with the dispatch call id (even though the child emitted None):
        let got = parent.events.lock().unwrap().clone();
        assert_eq!(got[0], ("tool_start".to_string(), "sub7:c1".to_string(), "sub:echo".to_string(), "d1".to_string()));
        assert_eq!(got[1].3, "d1");
        assert_eq!(got[2].3, "d1"); // server_usage
        // Tap saw exactly the non-forwarded kinds, attributed to ordinal 7:
        assert_eq!(
            tap.seen.lock().unwrap().clone(),
            vec![(7, "token"), (7, "error"), (7, "done")]
        );
    }

    #[test]
    fn no_tap_means_no_panic_and_capture_still_works() {
        let sink = SubagentSink::new(Arc::new(FullSink::default()), 1, "d1".into(), None);
        sink.emit(AgentEvent::Token("t".into()));
        sink.emit(AgentEvent::Done(StopReason::Stop));
        assert_eq!(sink.summary().final_text, "t");
    }
```

Integration additions in `tests/dispatch_tool.rs` (extend its `FullSink` the same quad way; `tool_ctx()` has `call_id: "d1"` from Task 1):

```rust
#[tokio::test]
async fn forwarded_child_events_carry_the_dispatch_call_id() {
    let sink = Arc::new(FullSink::default());
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "echo".into(), "{}".into()),
            Scripted::Text("done".into()),
        ]),
        sink.clone(),
        vec![Arc::new(Echo)],
    ));
    tool.execute(serde_json::json!({"prompt": "p"}), &tool_ctx()).await.unwrap();
    let events = sink.events.lock().unwrap().clone();
    assert!(events.iter().filter(|e| e.0.starts_with("tool_")).all(|e| e.3 == "d1"), "{events:?}");
}

#[tokio::test]
async fn allowlist_accepts_always_available_context_tools() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![Scripted::Text("x".into())]),
        Arc::new(FullSink::default()),
        vec![Arc::new(Echo)],
    ));
    // context_recall is not in base_tools but IS always registered for the child.
    let out = tool
        .execute(serde_json::json!({"prompt": "p", "tools": ["context_recall"]}), &tool_ctx())
        .await;
    assert!(out.is_ok(), "{out:?}");
    // Genuinely unknown names still error, and the message names the implicit tools.
    let err = tool
        .execute(serde_json::json!({"prompt": "p", "tools": ["nope"]}), &tool_ctx())
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(ref m)
        if m.contains("nope") && m.contains("context_recall")), "{err:?}");
}
```

(`deps()` in the integration file gains `child_trace: None` when the struct grows the field.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-core dispatch ; cargo test -p agent-core --test dispatch_tool`
Expected: compile errors — `SubagentTrace` undefined, `SubagentSink::new` arity.

- [ ] **Step 3: Implement in `dispatch.rs`**

```rust
/// Sink-shaped hook for tracing the child's non-forwarded transcript
/// (implemented over TraceWriter in agent-runtime-config — dep direction).
pub trait SubagentTrace: Send + Sync {
    fn record(&self, n: u64, event: &AgentEvent);
}
```

`SubagentSink` gains `parent_call_id: String` and `child_trace: Option<Arc<dyn SubagentTrace>>`; `new` takes both. `emit` becomes:

```rust
    fn emit(&self, event: AgentEvent) {
        match event {
            AgentEvent::ToolStart { id, name, args, .. } => {
                self.cap.lock().unwrap().tool_calls += 1;
                self.parent.emit(AgentEvent::ToolStart {
                    id: format!("sub{}:{}", self.n, id),
                    name: format!("sub:{name}"),
                    args,
                    parent_id: Some(self.parent_call_id.clone()),
                });
            }
            AgentEvent::ToolResult { id, name, status, output, duration_ms, .. } => {
                self.cap.lock().unwrap().segments.push(String::new());
                self.parent.emit(AgentEvent::ToolResult {
                    id: format!("sub{}:{}", self.n, id),
                    name: format!("sub:{name}"),
                    status,
                    output,
                    duration_ms,
                    parent_id: Some(self.parent_call_id.clone()),
                });
            }
            AgentEvent::ServerUsage {
                prompt_tokens, completion_tokens, reasoning_tokens, cached_tokens,
                cost_usd, turn_duration_ms, turn, ..
            } => {
                self.parent.emit(AgentEvent::ServerUsage {
                    prompt_tokens, completion_tokens, reasoning_tokens, cached_tokens,
                    cost_usd, turn_duration_ms, turn,
                    parent_id: Some(self.parent_call_id.clone()),
                });
            }
            // Everything else stays off the frontends (spec D9/E9) but goes to
            // the child-trace tap so a failed child turn is replayable (E4).
            other => {
                if let Some(t) = &self.child_trace {
                    t.record(self.n, &other);
                }
                let mut cap = self.cap.lock().unwrap();
                match other {
                    AgentEvent::Token(t) => {
                        cap.segments.last_mut().expect("segments never empty").push_str(&t);
                    }
                    AgentEvent::Usage { turn, .. } => cap.turns = cap.turns.max(turn),
                    AgentEvent::Done(reason) => cap.stop = Some(reason),
                    _ => {}
                }
            }
        }
    }
```

(Note the lock-scope discipline the file already follows: never hold `cap` across `parent.emit`.)

`DispatchDeps` gains:

```rust
    /// Trace tap for the child's non-forwarded events; None = tracing off.
    pub child_trace: Option<Arc<dyn SubagentTrace>>,
```

`execute()`:

```rust
        let sink = Arc::new(SubagentSink::new(
            self.deps.sink.clone(),
            next_dispatch_n(),
            ctx.call_id.clone(),
            self.deps.child_trace.clone(),
        ));
```

Allowlist fold (E8) — the validation block becomes:

```rust
        const IMPLICIT_CHILD_TOOLS: [&str; 2] = ["context_recall", "context_compact"];
        if let Some(names) = &allow {
            let available: Vec<&str> = self.deps.base_tools.iter().map(|t| t.name()).collect();
            for n in names {
                if !available.contains(&n.as_str()) && !IMPLICIT_CHILD_TOOLS.contains(&n.as_str()) {
                    return Err(ToolError::InvalidArgs(format!(
                        "unknown tool '{n}'; available: {}, plus always-available: {}",
                        available.join(", "),
                        IMPLICIT_CHILD_TOOLS.join(", ")
                    )));
                }
            }
        }
```

and the schema `tools` description gains the sentence: `"The child's context tools (context_recall, context_compact) are always available."`

Update `exec_deps` (unit tests) and `deps` (integration) with `child_trace: None`; the existing quad-extended assertions from Step 1 drive the rest.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-core`
Expected: PASS (all dispatch unit + integration tests, incl. the pre-existing 17).

- [ ] **Step 5: Commit**

```bash
git add -A agent/crates/agent-core
git commit -m "feat(core): SubagentSink stamps parent_id lineage, tees suppressed child events to a SubagentTrace tap; allowlist accepts implicit context tools"
```

---

### Task 4: Trace `record_child` + `ChildTraceTap` + assemble wiring

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/trace.rs`
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (DispatchDeps construction, ~line 170)
- Test: `trace.rs` and `assemble.rs` `mod tests`

**Interfaces:**
- Consumes: `agent_core::SubagentTrace` (Task 3), `TraceEvent.parent_id` (Task 2).
- Produces: `TraceWriter::record_child(&self, n: u64, event: &AgentEvent)`; `pub struct ChildTraceTap(pub Arc<TraceWriter>)` implementing `SubagentTrace`; `assemble_loop` passes `child_trace` into `DispatchDeps`.

- [ ] **Step 1: Write the failing tests**

`trace.rs` `mod tests` (reuse the module's existing create/read-lines helpers):

```rust
    #[test]
    fn record_child_lines_carry_sub_ordinal_and_normal_lines_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let w = TraceWriter::create(dir.path(), 1024 * 1024).unwrap();
        w.record(&agent_core::AgentEvent::Token("parent".into()));
        w.record_child(3, &agent_core::AgentEvent::Token("child".into()));
        let content = /* read the session file as in existing tests */;
        let lines: Vec<&str> = content.lines().collect();
        assert!(!lines[1].contains(r#""sub""#), "{}", lines[1]);
        assert!(lines[2].contains(r#""sub":3"#), "{}", lines[2]);
        // seq stays monotonic across both write paths:
        assert!(lines[2].contains(r#""seq":1"#), "{}", lines[2]);
    }
```

`assemble.rs` `mod tests`:

```rust
    #[test]
    fn assemble_wires_child_trace_only_when_tracing_is_on() {
        // No trace → assembles fine (child_trace None path).
        let dir = tempfile::tempdir().unwrap();
        let _ = assemble_loop(&cfg(), parts(dir.path().to_path_buf(), vec![]));
        // With a trace writer → also assembles fine (tap constructed).
        let mut p = parts(dir.path().to_path_buf(), vec![]);
        let tdir = tempfile::tempdir().unwrap();
        p.trace = Some(Arc::new(crate::trace::TraceWriter::create(tdir.path(), 1024 * 1024).unwrap()));
        let _ = assemble_loop(&cfg(), p);
    }
```

(Compile-level pin: the real behavioral pin is the `ChildTraceTap` unit test above; assemble-side, `DispatchDeps.child_trace` is a required field so the compiler enforces the wiring exists.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-runtime-config record_child`
Expected: compile error — no method `record_child`.

- [ ] **Step 3: Implement**

`trace.rs`:
1. `TraceRecord` gains `#[serde(skip_serializing_if = "Option::is_none")] sub: Option<u64>,`.
2. Refactor the body of `record` into a private `fn write_record(&self, sub: Option<u64>, event: &AgentEvent)`; `record` calls `write_record(None, event)`; add:

```rust
    /// Record a sub-agent child event, attributed to dispatch ordinal `n`
    /// (spec 2026-07-02 E4). Same file, same seq counter, same size cap.
    pub fn record_child(&self, n: u64, event: &AgentEvent) {
        self.write_record(Some(n), event);
    }
```

3. Adapter:

```rust
/// `SubagentTrace` over the session TraceWriter: child transcript lines land in
/// the same JSONL with a `sub` ordinal (spec E4).
pub struct ChildTraceTap(pub Arc<TraceWriter>);
impl agent_core::SubagentTrace for ChildTraceTap {
    fn record(&self, n: u64, event: &agent_core::AgentEvent) {
        self.0.record_child(n, event);
    }
}
```

`assemble.rs`, in the `DispatchDeps` construction:

```rust
                child_trace: parts.trace.clone().map(|t| {
                    Arc::new(crate::trace::ChildTraceTap(t)) as Arc<dyn agent_core::SubagentTrace>
                }),
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-runtime-config`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A agent/crates/agent-runtime-config
git commit -m "feat(runtime-config): child transcript in the session trace — record_child sub ordinals + ChildTraceTap wired at assemble"
```

---

### Task 5: Stats attribution (Rust)

**Files:**
- Modify: `agent/crates/agent-core/src/stats.rs`
- Modify: `agent/crates/agent-cli/src/render.rs` (`format_stats_line`)
- Test: both files' `mod tests`

**Interfaces:**
- Consumes: `parent_id` fields (Task 2).
- Produces: `SessionStats.subagent_tool_calls: u64`, `SessionStats.subagent_turns: u64` (serde `#[serde(default)]` so old traces/clients deserialize). Task 7's TS type mirrors them as optional.

- [ ] **Step 1: Write the failing tests**

`stats.rs` `mod tests`:

```rust
    #[test]
    fn subagent_events_fold_into_subset_counters_and_do_not_bump_turns() {
        let mut s = SessionStats::default();
        s.fold(&AgentEvent::ServerUsage {
            prompt_tokens: 10, completion_tokens: 5, reasoning_tokens: None,
            cached_tokens: None, cost_usd: Some(0.01), turn_duration_ms: 100,
            turn: 2, parent_id: None,
        });
        assert_eq!(s.turns, 2);
        // Child turn 7 must NOT pollute parent turns, but its cost/tokens count.
        s.fold(&AgentEvent::ServerUsage {
            prompt_tokens: 20, completion_tokens: 5, reasoning_tokens: None,
            cached_tokens: None, cost_usd: Some(0.02), turn_duration_ms: 100,
            turn: 7, parent_id: Some("d1".into()),
        });
        assert_eq!(s.turns, 2, "child turn index leaked into turns");
        assert_eq!(s.subagent_turns, 1);
        assert_eq!(s.prompt_tokens, 30);
        assert!((s.cost_usd - 0.03).abs() < 1e-9);

        s.fold(&AgentEvent::ToolStart { id: "c".into(), name: "t".into(), args: serde_json::json!({}), parent_id: None });
        s.fold(&AgentEvent::ToolStart { id: "sub1:c".into(), name: "sub:t".into(), args: serde_json::json!({}), parent_id: Some("d1".into()) });
        assert_eq!(s.tool_calls, 2, "totals stay totals");
        assert_eq!(s.subagent_tool_calls, 1, "subset counter");
    }
```

`agent-cli` (`render.rs` tests, matching the existing `format_stats_line` test style):

```rust
    #[test]
    fn stats_line_mentions_subagents_only_when_present() {
        let mut s = agent_core::SessionStats::default();
        assert!(!format_stats_line(&s).contains("sub-agent"));
        s.subagent_tool_calls = 3;
        s.subagent_turns = 2;
        let line = format_stats_line(&s);
        assert!(line.contains("sub-agent: 3 calls/2 turns"), "{line}");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-core stats ; cargo test -p agent-cli stats_line`
Expected: compile error — no field `subagent_tool_calls`.

- [ ] **Step 3: Implement**

`stats.rs` — two fields (with `#[serde(default)]`, doc comments noting subset semantics: `subagent_tool_calls ⊆ tool_calls`); fold arms:

```rust
            AgentEvent::ToolStart { parent_id, .. } => {
                self.tool_calls += 1;
                if parent_id.is_some() {
                    self.subagent_tool_calls += 1;
                }
            }
            AgentEvent::ServerUsage { /* existing bindings */, parent_id, turn, .. } => {
                // token/cost/time sums unchanged (billed truth includes children)…
                if parent_id.is_some() {
                    self.subagent_turns += 1; // one flagged ServerUsage per child turn
                } else {
                    self.turns = self.turns.max(*turn);
                }
            }
```

`render.rs::format_stats_line` — append, using the line's existing separator style:

```rust
    if s.subagent_tool_calls > 0 || s.subagent_turns > 0 {
        line.push_str(&format!(" · sub-agent: {} calls/{} turns", s.subagent_tool_calls, s.subagent_turns));
    }
```

(Adapt to how the function actually builds its string — read it first.)

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-core -p agent-cli`
Expected: PASS (existing stats tests unchanged — None-attribution folding is byte-identical).

- [ ] **Step 5: Commit**

```bash
git add -A agent/crates/agent-core agent/crates/agent-cli
git commit -m "feat(core): sub-agent stats attribution — subset counters, child turns no longer distort turns"
```

---

### Task 6: CLI indented child rows

**Files:**
- Modify: `agent/crates/agent-cli/src/render.rs` (ToolStart ~75-77, ToolResult ~78-114 arms)
- Test: `render.rs` `mod tests`

**Interfaces:**
- Consumes: `parent_id` (Task 2). Self-contained otherwise.

- [ ] **Step 1: Write the failing test**

`TerminalSink` writes to stdout, so pin via a pure formatting helper (precedent: `format_stats_line`). Extract the ToolStart line construction into `fn format_tool_start(name: &str, args: &serde_json::Value, parent_id: Option<&str>) -> String` and test:

```rust
    #[test]
    fn child_tool_rows_are_indented() {
        let args = serde_json::json!({});
        let top = format_tool_start("read_file", &args, None);
        let child = format_tool_start("sub:read_file", &args, Some("d1"));
        assert!(!top.contains('↳'));
        assert!(child.starts_with("  ↳"), "{child:?}");
        assert!(child.contains("sub:read_file"));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-cli child_tool_rows`
Expected: compile error — `format_tool_start` undefined.

- [ ] **Step 3: Implement**

```rust
/// The ToolStart display line; child (attributed) calls get a two-space
/// `↳` indent so nested activity reads as nested (spec E7).
fn format_tool_start(name: &str, args: &serde_json::Value, parent_id: Option<&str>) -> String {
    let indent = if parent_id.is_some() { "  ↳ " } else { "" };
    format!("\n{indent}\x1b[36m⚙ {name}\x1b[0m {args}")
}
```

The `emit` ToolStart arm calls it (binding `parent_id` from the event, `.as_deref()`). Apply the same `indent` prefix to the ToolResult arm's first output line when its `parent_id.is_some()` (keep the existing status coloring untouched — only prepend the indent; if the arm's structure makes a helper cleaner, mirror `format_tool_start`).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A agent/crates/agent-cli
git commit -m "feat(cli): indent attributed sub-agent tool rows"
```

---

### Task 7: Web — id-correlation, nesting, usage guard, stats row

**Files:**
- Modify: `web/src/wire.ts` (WireEvent tool_start/tool_result/server_usage; `SessionStats`)
- Modify: `web/src/state.ts` (Item tool variant ~line 10; reducer tool_start ~140, tool_result ~141-152, server_usage ~117)
- Modify: `web/src/components/ToolCall.tsx` (+ check `App.tsx`/`AnimatedToolCall.tsx` — apply the nesting display in whichever component App actually renders; if `AnimatedToolCall` delegates to `ToolCall`, edit `ToolCall` only)
- Modify: `web/src/components/StatsPanel.tsx`
- Test: `web/src/state.dispatch.test.ts` (new) + extend an existing StatsPanel test

**Interfaces:**
- Consumes: wire fields from Task 2 (`parent_id?: string`) and stats fields from Task 5 (`subagent_tool_calls?: number; subagent_turns?: number` — optional: old servers omit them).
- Produces: Item tool variant gains `id?: string; parentId?: string`.

- [ ] **Step 1: Write the failing tests**

`web/src/state.dispatch.test.ts` (mirror the frame-building helpers of the existing state tests, e.g. `state.stats.test.ts`):

```ts
import { describe, expect, it } from "vitest";
import { initialState, reduce } from "./state"; // match existing test imports

const frame = (payload: unknown) =>
  ({ v: 1, session_id: "s", kind: "event", payload }) as never;
const red = (s: ReturnType<typeof initialState>, p: unknown) =>
  reduce(s, { type: "frame", frame: frame(p) });

describe("sub-agent attribution", () => {
  it("correlates tool_result by id when two same-named tools run", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "a", name: "read_file", args: {} });
    s = red(s, { type: "tool_start", id: "b", name: "read_file", args: {} });
    s = red(s, { type: "tool_result", id: "a", name: "read_file", status: "ok", duration_ms: 1, content: "first" });
    const tools = s.items.filter((i) => i.kind === "tool");
    expect(tools[0]).toMatchObject({ id: "a", status: "done", content: "first" });
    expect(tools[1]).toMatchObject({ id: "b", status: "running" });
  });

  it("falls back to name-correlation for items without ids (old persisted state)", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", name: "legacy", args: {} }); // no id
    s = red(s, { type: "tool_result", id: "x", name: "legacy", status: "ok", duration_ms: 1, content: "c" });
    expect(s.items.find((i) => i.kind === "tool")).toMatchObject({ status: "done" });
  });

  it("stores parentId on attributed child rows", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "d1", name: "dispatch_agent", args: {} });
    s = red(s, { type: "tool_start", id: "sub1:c1", name: "sub:read_file", args: {}, parent_id: "d1" });
    const child = s.items.filter((i) => i.kind === "tool")[1];
    expect(child).toMatchObject({ parentId: "d1", name: "sub:read_file" });
  });

  it("child server_usage does not touch the turn readout", () => {
    let s = initialState([]);
    s = red(s, { type: "server_usage", prompt_tokens: 10, completion_tokens: 1, turn: 2 });
    s = red(s, { type: "server_usage", prompt_tokens: 99, completion_tokens: 1, turn: 7, parent_id: "d1" });
    expect(s.serverUsage).toMatchObject({ promptTokens: 10, turn: 2 });
  });
});
```

StatsPanel: extend the existing StatsPanel test file with a render assert that stats `{ ...zeroStats, subagent_tool_calls: 3, subagent_turns: 2 }` shows a `Sub-agent` row (`3 calls / 2 turns`) and that the zero case shows no such row (mirror how the cost row is asserted).

- [ ] **Step 2: Run to verify failure**

Run: `cd /home/kalen/rust-agent-runtime/web && npx vitest run src/state.dispatch.test.ts`
Expected: FAIL (tool_result matched by name resolves the wrong item; `parentId` undefined).

- [ ] **Step 3: Implement**

`wire.ts`:

```ts
  | { type: "server_usage"; prompt_tokens: number; completion_tokens: number; turn: number;
      reasoning_tokens?: number; cached_tokens?: number; cost_usd?: number; turn_duration_ms?: number;
      parent_id?: string }
  | { type: "tool_start"; id: string; name: string; args: unknown; parent_id?: string }
  | { type: "tool_result"; id: string; name: string; status: string; duration_ms: number;
      content: string; display?: Display; parent_id?: string }
```

`SessionStats` gains `subagent_tool_calls?: number; subagent_turns?: number;`.

`state.ts` — Item tool variant:

```ts
  | { kind: "tool"; name: string; args: unknown; status: "running" | "done";
      id?: string; parentId?: string;
      content?: string; display?: Display; resultStatus?: string; durationMs?: number }
```

Reducer:

```ts
    case "server_usage":
      // A sub-agent's usage frame must not flicker the parent turn readout;
      // its tokens still land in session_stats (spec E5/E6c).
      if (p.parent_id) return s;
      return { ...s, serverUsage: { promptTokens: p.prompt_tokens, turn: p.turn } };
```

```ts
    case "tool_start":
      return { ...s, items: [...s.items, { kind: "tool", id: p.id, parentId: p.parent_id,
        name: p.name, args: p.args, status: "running" }] };
    case "tool_result": {
      const items = [...s.items];
      for (let i = items.length - 1; i >= 0; i--) {
        const it = items[i];
        // id-first correlation (parallel same-named child tools); name-fallback
        // only for pre-id items restored from old persisted state.
        if (it.kind === "tool" && it.status === "running" &&
            (it.id !== undefined ? it.id === p.id : it.name === p.name)) {
          items[i] = { ...it, status: "done", content: p.content, display: p.display,
            resultStatus: p.status, durationMs: p.duration_ms };
          break;
        }
      }
      return { ...s, items };
    }
```

`ToolCall.tsx` (verify via `App.tsx` which component renders tool items; if `AnimatedToolCall` wraps this one, this is the right place):

```tsx
  const nested = !!item.parentId;
  const displayName = nested && item.name.startsWith("sub:") ? item.name.slice(4) : item.name;
  // outer div: style={{ ..., marginLeft: nested ? "1.25rem" : undefined }}
  // before the name badge: {nested && <span style={{ color: "var(--text-muted)" }}>↳</span>}
  // name badge renders {displayName}
```

`StatsPanel.tsx`, with the other conditional rows:

```tsx
  if ((stats.subagent_tool_calls ?? 0) > 0 || (stats.subagent_turns ?? 0) > 0)
    rows.push(["Sub-agent", `${stats.subagent_tool_calls ?? 0} calls / ${stats.subagent_turns ?? 0} turns`]);
```

- [ ] **Step 4: Run to verify pass**

Run: `cd web && npm run typecheck && npm test`
Expected: PASS (all 168 existing + new; the forward-compat unknown-event test must stay green).

- [ ] **Step 5: Commit**

```bash
git add web
git commit -m "feat(web): sub-agent nesting — id-first tool correlation, parentId indent, usage-flicker guard, Sub-agent stats row"
```

---

### Task 8: Workspace sweep + spec cross-check

**Files:** none expected; fix fallout only.

- [ ] **Step 1: Full CI**

Run: `cd /home/kalen/rust-agent-runtime && bash scripts/ci.sh`
Expected: green (fmt + clippy + all Rust suites + web typecheck/vitest). Fix fallout minimally; commit as `chore: workspace sweep for sub-agent observability`.

- [ ] **Step 2: Spec cross-check**

Re-read `docs/superpowers/specs/2026-07-02-subagent-observability-design.md` — verify E1–E9 and every Testing bullet landed or is explicitly out-of-scope. Report gaps; do not silently fix design-level ones.

- [ ] **Step 3: Commit (only if fixes were needed)**
