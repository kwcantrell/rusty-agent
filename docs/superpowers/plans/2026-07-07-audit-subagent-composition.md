# Audit Cluster 4 — Sub-agent Composition Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close audit findings 4.1, 4.2, 4.3, 4.4, 2.3≡4.5 — the composition seams between July's individually-reviewed sub-agent clusters.

**Architecture:** Five independent fixes on the dispatch/loop/config seam: thread `tool_description_overrides` into child registries via `DispatchDeps`; add optional `context_limit`/`max_tokens` to `ModelRef` with inherit-on-None plus a `min()` maintenance limit on `LoopConfig`; inject the budget wrap-up prompt into the request only (never durable history); return the child's partial transcript from the timeout/failure dispatch arms; compute `dispatch_agent`'s description from depth at construction.

**Tech Stack:** Rust (agent/ Cargo workspace — crates agent-core, agent-runtime-config), tokio, existing testkit (`ScriptedModel`/`Scripted`, `FullSink`, `CollectingSink`).

**Spec:** `docs/superpowers/specs/2026-07-07-audit-subagent-composition-design.md`

## Global Constraints

- Branch `feature/audit-subagent-composition` in a worktree off local `main` tip (NOT stale `origin/main`).
- Before EVERY commit: `git rev-parse --show-toplevel` must print the WORKTREE path. Never commit in the main checkout; never `git reset` / `git rebase`.
- Two Cargo workspaces in the repo: all commands here run in `agent/` (e.g. `cd <worktree>/agent`). `-p` names below target that workspace.
- Conventional commits: `type(scope): summary`.
- Every fix lands with its regression test (TDD: failing test first).
- Run `cargo fmt --all` before every commit and `cargo fmt --all --check` in verification (fmt failures have reached the gate twice before).
- Old-SPA wire compat: additive/optional serde fields only (Task 5 complies; nothing else touches the wire).
- Line numbers below are from live source at plan time — re-locate by the quoted code if drifted.

---

### Task 1: Depth-aware dispatch_agent description (findings 2.3≡4.5)

**Files:**
- Modify: `agent/crates/agent-core/src/dispatch.rs` (struct `DispatchAgentTool` ~line 231, `new` ~236, `description()` ~246)
- Test: `agent/crates/agent-core/tests/dispatch_tool.rs`

**Interfaces:**
- Consumes: `DispatchDeps.depth` / `DispatchDeps.max_depth` (existing).
- Produces: `DispatchAgentTool { deps, description: String }`; `description()` returns `&self.description`. Task 2 and 3 edit the same file — this task changes only the struct, `new`, and `description`.

- [ ] **Step 1: Write the failing test**

Append to `agent/crates/agent-core/tests/dispatch_tool.rs`:

```rust
/// Findings 2.3/4.5: the "minus dispatch_agent itself" claim is only true at
/// the depth floor; with nesting allowed the description must say the child
/// can dispatch its own sub-agents (it gets a nested dispatch_agent by default).
#[test]
fn description_is_depth_aware() {
    let base = deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    );
    // Depth floor (depth 1, max_depth 1 — the default): child cannot dispatch.
    let floor = DispatchAgentTool::new(base.clone());
    assert!(
        floor.description().contains("minus dispatch_agent itself"),
        "{}",
        floor.description()
    );
    assert!(
        floor.schema().description.contains("minus dispatch_agent itself"),
        "schema must flow through the stored description"
    );
    // Nesting allowed (depth 1 < max_depth 2): the child CAN dispatch.
    let mut d = base;
    d.max_depth = 2;
    let nested = DispatchAgentTool::new(d);
    assert!(
        !nested.description().contains("minus dispatch_agent"),
        "{}",
        nested.description()
    );
    assert!(
        nested.description().contains("dispatch its own sub-agents"),
        "{}",
        nested.description()
    );
}
```

Note: `deps(...)` is the existing helper at dispatch_tool.rs:151; `DispatchDeps` is `Clone`. `Tool` is already imported (needed for `.description()`/`.schema()`).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p agent-core --test dispatch_tool description_is_depth_aware`
Expected: FAIL — the nested assertions fail (`description()` is static, still contains "minus dispatch_agent").

- [ ] **Step 3: Implement the stored depth-computed description**

In `agent/crates/agent-core/src/dispatch.rs`, replace the struct + `new` + `description`:

```rust
pub struct DispatchAgentTool {
    deps: DispatchDeps,
    /// Computed at construction from depth/max_depth: the "minus dispatch_agent"
    /// claim is only true at the depth floor (findings 2.3/4.5).
    description: String,
}

impl DispatchAgentTool {
    pub fn new(deps: DispatchDeps) -> Self {
        // Matches the child-registry rule in execute(): a child gets a nested
        // dispatch_agent by default whenever depth < max_depth.
        let caps = if deps.depth < deps.max_depth {
            "(including dispatch_agent while nesting depth allows, so it can \
             dispatch its own sub-agents; the tools allowlist restricts this \
             transitively)"
        } else {
            "(minus dispatch_agent itself)"
        };
        let description = format!(
            "Delegate an independent, multi-step subtask to an isolated sub-agent with \
             its own fresh context window. The sub-agent has the same permissions and \
             tools as you {caps}, works autonomously on the \
             prompt you give it, and its final answer is returned as this tool's \
             result. Make the prompt self-contained: the sub-agent cannot see this \
             conversation. You may dispatch several sub-agents in one message by \
             issuing multiple dispatch_agent calls — they run concurrently."
        );
        Self { deps, description }
    }
}
```

And in the `impl Tool for DispatchAgentTool` block, replace the static `description()` body (the whole quoted string literal) with:

```rust
    fn description(&self) -> &str {
        &self.description
    }
```

`schema()` already uses `self.description().into()` — no change there.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agent-core --test dispatch_tool && cargo test -p agent-core --lib dispatch`
Expected: PASS (all existing dispatch tests too — the floor text is byte-identical for the default config).

- [ ] **Step 5: fmt + commit**

```bash
cargo fmt --all && cargo fmt --all --check
git add -A && git commit -m "fix(dispatch): depth-aware dispatch_agent description (audit 2.3/4.5)"
```

---

### Task 2: Thread description overrides into child registries (finding 4.1)

**Files:**
- Modify: `agent/crates/agent-core/src/dispatch.rs` (`DispatchDeps` ~line 201, `execute` after the context-tools registration ~line 430, in-file test helper `exec_deps` ~line 825)
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (`DispatchDeps` literal ~line 282)
- Modify: `agent/crates/agent-core/tests/dispatch_tool.rs` (`deps()` helper ~line 151)
- Test: in-file `mod tests` of `agent/crates/agent-core/src/dispatch.rs` (needs the `futures` crate for the model-wrapper signature — a regular dep of agent-core, available to in-file tests but NOT to integration tests)

**Interfaces:**
- Consumes: `ToolRegistry::set_description_overrides(HashMap<String, String>)` (agent-tools/src/registry.rs:39 — warns on unknown names, applied in `schemas()`).
- Produces: `DispatchDeps.description_overrides: std::collections::HashMap<String, String>` — Task 3's arms and later tests must include this field in any `DispatchDeps` literal.

- [ ] **Step 1: Write the failing test**

Append inside `mod tests` in `agent/crates/agent-core/src/dispatch.rs` (near the other `exec_deps` tests):

```rust
    /// Finding 4.1: registry-level description overrides must reach the CHILD
    /// registry too (the seam spec's uniformity claim). context_recall is
    /// always-registered for children, so overriding it needs no base tool.
    #[tokio::test]
    async fn description_overrides_reach_child_registry() {
        struct SchemaCapturingModel {
            inner: ScriptedModel,
            seen: std::sync::Mutex<Vec<(String, String)>>,
        }
        #[async_trait::async_trait]
        impl agent_model::ModelClient for SchemaCapturingModel {
            async fn stream(
                &self,
                req: agent_model::CompletionRequest,
            ) -> Result<
                futures::stream::BoxStream<
                    'static,
                    Result<agent_model::Chunk, agent_model::ModelError>,
                >,
                agent_model::ModelError,
            > {
                self.seen.lock().unwrap().extend(
                    req.tools
                        .iter()
                        .map(|t| (t.name.clone(), t.description.clone())),
                );
                self.inner.stream(req).await
            }
        }
        let model = Arc::new(SchemaCapturingModel {
            inner: ScriptedModel::new(vec![Scripted::Text("x".into())]),
            seen: Default::default(),
        });
        let mut d = exec_deps(ScriptedModel::new(vec![]), 5);
        d.model = model.clone();
        d.description_overrides =
            [("context_recall".to_string(), "OVERRIDDEN".to_string())].into();
        // Clone-propagation pin: nested deps are self.deps.clone() in execute().
        assert_eq!(
            d.clone().description_overrides.get("context_recall"),
            Some(&"OVERRIDDEN".to_string())
        );
        let tool = DispatchAgentTool::new(d);
        tool.execute(serde_json::json!({"prompt": "p"}), &exec_ctx())
            .await
            .unwrap();
        let seen = model.seen.lock().unwrap();
        assert!(
            seen.iter()
                .any(|(n, desc)| n == "context_recall" && desc.starts_with("OVERRIDDEN")),
            "child request schemas must carry the override: {seen:?}"
        );
    }
```

Note: the override lands as the BASE description; the registry's `when_not_to_call` fold appends afterwards — hence `starts_with`, not `==`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p agent-core --lib dispatch::tests::description_overrides_reach_child_registry`
Expected: FAIL to COMPILE — `DispatchDeps` has no field `description_overrides`.

- [ ] **Step 3: Add the field and thread it**

(a) `dispatch.rs`, `DispatchDeps` struct — add after `id_prefix`:

```rust
    /// Parent-configured tool-description overrides, applied to the child
    /// registry too so the tool vocabulary stays uniform across depths
    /// (finding 4.1; seam spec 2026-07-02-tool-description-override-seam).
    pub description_overrides: std::collections::HashMap<String, String>,
```

(b) `dispatch.rs`, in `execute()` right after the `for t in crate::context_tools(...) { reg.register(t); }` loop (all registrations done):

```rust
        // Finding 4.1: apply the parent's description overrides to the child
        // registry (registry-level, matching assemble.rs's parent application).
        // Names not in THIS child's registry (e.g. allowlist-filtered tools)
        // just warn — same posture as the parent path.
        reg.set_description_overrides(self.deps.description_overrides.clone());
```

(c) `assemble.rs`, the `agent_core::DispatchDeps { ... }` literal — add:

```rust
                description_overrides: cfg.tool_description_overrides.clone(),
```

(d) Update the two test helpers to keep everything compiling — add to BOTH `DispatchDeps` literals:
- `dispatch.rs` `exec_deps()` (~line 827)
- `tests/dispatch_tool.rs` `deps()` (~line 153)

```rust
        description_overrides: Default::default(),
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agent-core && cargo test -p agent-runtime-config`
Expected: PASS (new test green; every existing dispatch/assemble test still compiles and passes).

- [ ] **Step 5: fmt + commit**

```bash
cargo fmt --all && cargo fmt --all --check
git add -A && git commit -m "fix(dispatch): thread tool description overrides into child registries (audit 4.1)"
```

---

### Task 3: Partial transcript on child timeout/failure (finding 4.4)

**Files:**
- Modify: `agent/crates/agent-core/src/dispatch.rs` (the `match tokio::time::timeout(ctx.timeout, run).await` arms ~line 468)
- Test: `agent/crates/agent-core/tests/dispatch_tool.rs` (new tests + UPDATE existing `wall_clock_timeout_cancels_the_child_and_reports_timeout` at ~line 719)

**Interfaces:**
- Consumes: `SubagentSink::summary() -> CaptureSummary { final_text, tool_calls, turns, stop }` (dispatch.rs:93); `DispatchDeps.description_overrides` from Task 2 (already in the helpers).
- Produces: private `fn failure_output(sink: &SubagentSink, what: String, stop_fallback: &str) -> ToolOutput` in dispatch.rs. Behavior contract: timeout/failure arms return `Ok(ToolOutput)`; parent-cancel still returns `Err`.

- [ ] **Step 1: Write the failing tests**

Append to `agent/crates/agent-core/tests/dispatch_tool.rs`:

```rust
/// Finding 4.4: a child killed by the wall-clock timeout hands its parent the
/// captured partial transcript instead of a bare ToolError::Timeout.
#[tokio::test(start_paused = true)]
async fn timed_out_child_returns_partial_transcript() {
    use agent_model::{Chunk, RawToolCall, StopReason as MStop};
    let sink = Arc::new(FullSink::default());
    let mut d = deps(
        ScriptedModel::new(vec![
            // Turn 1: streams text (captured as a segment) then a tool call, so
            // the run continues into turn 2.
            Scripted::Chunks(vec![
                Chunk::Text("partial progress note".into()),
                Chunk::ToolCallDelta(RawToolCall {
                    index: None,
                    id: Some("c1".into()),
                    name: Some("echo".into()),
                    args_fragment: "{}".into(),
                }),
                Chunk::Done(MStop::ToolCalls),
            ]),
            // Turn 2: hangs; the wall-clock timeout fires (virtual time).
            Scripted::Hang,
        ]),
        sink,
        vec![Arc::new(Echo)],
    );
    d.subagent_timeout = Duration::from_secs(1);
    let tool = DispatchAgentTool::new(d);
    let mut ctx = tool_ctx();
    ctx.timeout = Duration::from_secs(1);
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &ctx)
        .await
        .unwrap();
    assert!(
        out.content
            .starts_with("[sub-agent timed out after 1s — partial transcript follows]"),
        "{}",
        out.content
    );
    assert!(out.content.contains("partial progress note"), "{}", out.content);
    assert!(out.content.contains("stop: timeout"), "{}", out.content);
}

/// Finding 4.4 (empty capture): a child that produced nothing still reports the
/// note + footer, with the no-transcript wording and no misleading "stop: Stop".
#[tokio::test(start_paused = true)]
async fn timed_out_child_with_no_capture_reports_note_and_footer() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![Scripted::Hang]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    let mut ctx = tool_ctx();
    ctx.timeout = Duration::from_secs(1);
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &ctx)
        .await
        .unwrap();
    assert!(
        out.content
            .starts_with("[sub-agent timed out after 1s — no partial transcript captured]"),
        "{}",
        out.content
    );
    assert!(
        out.content.contains("stop: timeout"),
        "no Done was recorded — the footer must not claim a clean Stop: {}",
        out.content
    );
    assert!(!out.content.contains("\n\n"), "no blank-line runs: {}", out.content);
}

/// Finding 4.4: a child whose model fails fatally still hands the parent the
/// captured partial transcript, with the failure note.
#[tokio::test]
async fn failed_child_returns_partial_transcript() {
    use agent_model::{Chunk, ModelError, RawToolCall, StopReason as MStop};
    let sink = Arc::new(FullSink::default());
    let d = deps(
        ScriptedModel::new(vec![
            Scripted::Chunks(vec![
                Chunk::Text("progress so far".into()),
                Chunk::ToolCallDelta(RawToolCall {
                    index: None,
                    id: Some("c1".into()),
                    name: Some("echo".into()),
                    args_fragment: "{}".into(),
                }),
                Chunk::Done(MStop::ToolCalls),
            ]),
            // Status 401 is Fatal on first sight (types.rs class()); a second
            // Fail keeps the test robust if classification ever loosens.
            Scripted::Fail(ModelError::Status {
                code: 401,
                body: "no auth".into(),
                retry_after: None,
            }),
            Scripted::Fail(ModelError::Status {
                code: 401,
                body: "no auth".into(),
                retry_after: None,
            }),
        ]),
        sink,
        vec![Arc::new(Echo)],
    );
    let tool = DispatchAgentTool::new(d);
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .unwrap();
    assert!(
        out.content.starts_with("[sub-agent failed: "),
        "{}",
        out.content
    );
    assert!(
        out.content.contains("— partial transcript follows]"),
        "{}",
        out.content
    );
    assert!(out.content.contains("progress so far"), "{}", out.content);
    assert!(out.content.contains("stop: failed"), "{}", out.content);
}
```

Then UPDATE the existing test at ~line 719 (`wall_clock_timeout_cancels_the_child_and_reports_timeout`) — it currently asserts `Err(ToolError::Timeout)`; the arm now returns Ok. Replace its body's tail:

```rust
    let started = tokio::time::Instant::now();
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &ctx)
        .await
        .unwrap();
    assert!(
        out.content.starts_with("[sub-agent timed out after 1s"),
        "{}",
        out.content
    );
    assert_eq!(started.elapsed(), Duration::from_secs(1)); // virtual time: exactly the budget
```

(Keep the test name and its `start_paused`/HangOpen setup — it still pins that the wall clock, not the idle timeout, fires.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p agent-core --test dispatch_tool -- timed_out failed_child wall_clock`
Expected: the three new tests FAIL (`unwrap()` on `Err(Timeout)`/`Err(Failed)`); the updated existing test FAILS the same way.

- [ ] **Step 3: Implement the failure-output arms**

In `dispatch.rs`, add a free function near `SubagentSink` (module level, above `DispatchDeps`):

```rust
/// Build the tool result for a child that died (wall-clock timeout or fatal
/// model error) from whatever the sink captured — partial results reach the
/// coordinator instead of being discarded (finding 4.4; mirrors the
/// budget-exhaustion posture). `stop_fallback` keeps the footer honest when
/// the child never emitted Done: "timeout" / "failed", never a clean Stop.
fn failure_output(sink: &SubagentSink, what: String, stop_fallback: &str) -> ToolOutput {
    let s = sink.summary();
    let stop_str = match s.stop {
        Some(r) => format!("{r:?}"),
        None => stop_fallback.to_string(),
    };
    let footer = format!(
        "[sub-agent: {} turns, {} tool calls, stop: {stop_str}]",
        s.turns, s.tool_calls
    );
    let content = if s.final_text.is_empty() {
        format!("[{what} — no partial transcript captured]\n{footer}")
    } else {
        format!("[{what} — partial transcript follows]\n{}\n\n{footer}", s.final_text)
    };
    ToolOutput {
        content,
        display: None,
    }
}
```

Replace the two arms in `execute()`:

```rust
        match tokio::time::timeout(ctx.timeout, run).await {
            Err(_elapsed) => {
                child_cancel.cancel();
                return Ok(failure_output(
                    &sink,
                    format!("sub-agent timed out after {}s", ctx.timeout.as_secs()),
                    "timeout",
                ));
            }
            Ok(Err(e)) => {
                return Ok(failure_output(
                    &sink,
                    format!("sub-agent failed: {e}"),
                    "failed",
                ));
            }
            Ok(Ok(())) => {}
        }
```

The parent-cancelled check below the match stays exactly as is (still `Err`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agent-core`
Expected: PASS — including `parent_cancel_mid_run_resolves_to_cancelled_promptly` and `pre_cancelled_parent_token_cancels_the_child` (unchanged `Err` posture).

- [ ] **Step 5: fmt + commit**

```bash
cargo fmt --all && cargo fmt --all --check
git add -A && git commit -m "fix(dispatch): return partial transcript on child timeout/failure (audit 4.4)"
```

---

### Task 4: Budget wrap-up prompt out of durable history (finding 4.3)

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` (budget arm ~line 1014-1016; in-file `mod tests`: extend `budget_wrap_up_failure_is_best_effort` ~line 1704, add two tests)

**Interfaces:**
- Consumes: `ContextManager::build(model_limit) -> Vec<Message>` (trait is in scope in loop_.rs); `BUDGET_WRAP_UP_PROMPT` const (loop_.rs:38).
- Produces: no API change — behavior only (request-local injection).

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `loop_.rs`, next to the existing budget wrap-up tests (~line 1700). The two-run test needs `crate::ContextManager` for direct `ctx.build` calls — add the import inside the test fn if not already in scope.

```rust
    /// Finding 4.3: the wrap-up instruction ("tools are disabled...") must never
    /// enter durable history — it would survive into later runs of the same
    /// session as a stale, false capability statement models imitate.
    #[tokio::test]
    async fn budget_wrap_up_prompt_stays_out_of_durable_history() {
        use crate::ContextManager;
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        std::fs::write(ws.join("f.txt"), "x").unwrap();
        let model = Arc::new(ScriptedModel::new(vec![
            // Run 1, turn 1 (budget = 1): a tool call.
            Scripted::Call(
                "c1".into(),
                "read_file".into(),
                format!(r#"{{"path":"{}"}}"#, ws.join("f.txt").display()),
            ),
            // Run 1: the tools-disabled wrap-up completion.
            Scripted::Text("wrap-up summary".into()),
            // Run 2: a plain reply.
            Scripted::Text("second run reply".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink,
            LoopConfig {
                model_limit: 100_000,
                max_turns: 1,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        // Durable history after run 1: summary yes, instruction no.
        let msgs = ctx.build(100_000);
        assert!(
            msgs.iter().any(|m| m.content.contains("wrap-up summary")),
            "the assistant summary IS durable"
        );
        assert!(
            !msgs
                .iter()
                .any(|m| m.content.contains("turn limit for this run")),
            "the wrap-up instruction must not be durable: {msgs:?}"
        );
        // Run 2 (same session context): still no stale instruction in the build.
        agent.run(&mut ctx, "next".into()).await.unwrap();
        let msgs = ctx.build(100_000);
        assert!(
            !msgs
                .iter()
                .any(|m| m.content.contains("turn limit for this run")),
            "no tools-disabled instruction may reach a later run: {msgs:?}"
        );
    }

    /// Finding 4.3 (other half): the wrap-up REQUEST must still end with the
    /// instruction — injection is request-local, not dropped.
    #[tokio::test]
    async fn budget_wrap_up_request_still_carries_the_prompt() {
        struct LastRequestModel {
            inner: ScriptedModel,
            last: std::sync::Mutex<Vec<Message>>,
        }
        #[async_trait::async_trait]
        impl agent_model::ModelClient for LastRequestModel {
            async fn stream(
                &self,
                req: agent_model::CompletionRequest,
            ) -> Result<
                futures::stream::BoxStream<
                    'static,
                    Result<agent_model::Chunk, agent_model::ModelError>,
                >,
                agent_model::ModelError,
            > {
                *self.last.lock().unwrap() = req.messages.clone();
                self.inner.stream(req).await
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        std::fs::write(ws.join("f.txt"), "x").unwrap();
        let model = Arc::new(LastRequestModel {
            inner: ScriptedModel::new(vec![
                Scripted::Call(
                    "c1".into(),
                    "read_file".into(),
                    format!(r#"{{"path":"{}"}}"#, ws.join("f.txt").display()),
                ),
                Scripted::Text("wrap-up summary".into()),
            ]),
            last: std::sync::Mutex::new(Vec::new()),
        });
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model.clone(),
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink,
            LoopConfig {
                model_limit: 100_000,
                max_turns: 1,
                max_retries: 2,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let last = model.last.lock().unwrap();
        let tail = last.last().expect("wrap-up request has messages");
        assert!(
            tail.content.contains("turn limit for this run"),
            "the wrap-up request must end with the instruction: {tail:?}"
        );
    }
```

Also EXTEND `budget_wrap_up_failure_is_best_effort` (~line 1704) — after its existing `assert_eq!` on Done, add:

```rust
        use crate::ContextManager;
        let msgs = ctx.build(100_000);
        assert!(
            !msgs
                .iter()
                .any(|m| m.content.contains("turn limit for this run")),
            "a failed wrap-up must leave no stale instruction in history: {msgs:?}"
        );
```

Note: if `Message` doesn't derive `Debug` for the `{msgs:?}`/`{tail:?}` formats, print `m.content` instead — adapt at implementation time, don't add derives for a test.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p agent-core --lib budget_wrap_up`
Expected: `budget_wrap_up_prompt_stays_out_of_durable_history` FAILS ("must not be durable") and the extended failure test FAILS; `budget_wrap_up_request_still_carries_the_prompt` PASSES already (append feeds build today) — it is the regression guard for the fix.

- [ ] **Step 3: Make the injection request-local**

In `loop_.rs` at the budget arm (~line 1014), replace:

```rust
        if !cancel.is_cancelled() {
            ctx.append(Message::user(BUDGET_WRAP_UP_PROMPT));
            let messages = ctx.build(self.effective_model_limit());
```

with:

```rust
        if !cancel.is_cancelled() {
            // Finding 4.3: inject the wrap-up instruction into THIS request only.
            // Durable history never sees it — appended, it would survive into
            // later runs of the session as a stale "tools are disabled" claim
            // (models measurably imitate such visible history patterns).
            let mut messages = ctx.build(self.effective_model_limit());
            messages.push(Message::user(BUDGET_WRAP_UP_PROMPT));
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agent-core`
Expected: PASS — including the untouched `budget_exhaustion_runs_tools_disabled_wrap_up` (tool counts/token stream unchanged) and `budget_exhausted_child_wrap_up_summary_reaches_parent` (dispatch integration).

- [ ] **Step 5: fmt + commit**

```bash
cargo fmt --all && cargo fmt --all --check
git add -A && git commit -m "fix(loop): keep budget wrap-up prompt out of durable history (audit 4.3)"
```

---

### Task 5: ModelRef context_limit/max_tokens inheritance (finding 4.2)

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs` (`ModelRef` ~line 20, `validate()` ~line 382, tests)
- Modify: `agent/crates/agent-core/src/loop_.rs` (`LoopConfig` ~line 91 + `Default` ~line 122, new `maint_model_limit()` next to `effective_model_limit()` ~line 221, the three `MaintCtx` sites ~lines 526, 714, 994, in-file test)
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (`loop_config_from` ~line 100, child_config block ~line 279, `BuiltLoop` ~line 47, tests)

**Interfaces:**
- Consumes: existing `ModelRef` serde pattern (`#[serde(default)]` Option fields); `LoopConfig` clone-for-child in assemble.rs.
- Produces: `ModelRef.context_limit: Option<usize>`, `ModelRef.max_tokens: Option<u32>`; `LoopConfig.compaction_model_limit: Option<usize>`; `AgentLoop::maint_model_limit(&self) -> usize`; `BuiltLoop.child_loop_knobs: Option<(usize, Option<u32>)>` (#[cfg(test)]).

- [ ] **Step 1: Write the failing config-surface tests**

In `runtime_config.rs` `mod tests`, add:

```rust
    #[test]
    fn modelref_window_fields_default_none_and_parse() {
        // Old JSON (no new fields) parses with None — back-compat.
        let r: ModelRef = serde_json::from_str(r#"{"model": "mini"}"#).unwrap();
        assert_eq!(r.context_limit, None);
        assert_eq!(r.max_tokens, None);
        let r: ModelRef =
            serde_json::from_str(r#"{"model": "mini", "context_limit": 8192, "max_tokens": 512}"#)
                .unwrap();
        assert_eq!(r.context_limit, Some(8192));
        assert_eq!(r.max_tokens, Some(512));
    }

    #[test]
    fn validate_floors_routed_model_context_limits() {
        let mut c = valid_config();
        c.subagent_model = Some(ModelRef {
            context_limit: Some(1023),
            ..Default::default()
        });
        assert!(c.validate().is_err(), "subagent_model floor");
        c.subagent_model = Some(ModelRef {
            context_limit: Some(1024),
            ..Default::default()
        });
        assert!(c.validate().is_ok(), "1024 is the floor, inclusive");
        c.compaction_model = Some(ModelRef {
            context_limit: Some(512),
            ..Default::default()
        });
        assert!(c.validate().is_err(), "compaction_model floor");
        c.compaction_model = Some(ModelRef::default());
        assert!(c.validate().is_ok(), "None inherits — always valid");
    }
```

Note: use the module's existing valid-config test helper — if it isn't literally `valid_config()`, mirror whatever the neighboring `validate` tests construct (read them first).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-runtime-config --lib modelref_window validate_floors`
Expected: FAIL to COMPILE — no such fields.

- [ ] **Step 3: Add the ModelRef fields + validate floor**

In `ModelRef` (runtime_config.rs:20), after `protocol`:

```rust
    /// Context window of the routed model; None inherits the primary's
    /// `context_limit`. Applied to child loops' `model_limit` (subagent_model)
    /// and, via min(), to the maintenance target (compaction_model).
    #[serde(default)]
    pub context_limit: Option<usize>,
    /// Per-completion output cap for the routed model; None inherits.
    #[serde(default)]
    pub max_tokens: Option<u32>,
```

In `validate()`, after the existing `context_limit < 1024` check:

```rust
        for (name, r) in [
            ("subagent_model", &self.subagent_model),
            ("compaction_model", &self.compaction_model),
        ] {
            if let Some(cl) = r.as_ref().and_then(|r| r.context_limit) {
                if cl < 1024 {
                    return Err(format!("{name}.context_limit must be >= 1024").into());
                }
            }
        }
```

(Match the function's actual error type — the existing checks show the pattern; if they are `return Err("...".into())` on `&str`, use `format!(...)` accordingly.)

Run: `cargo test -p agent-runtime-config --lib modelref_window validate_floors` → PASS.

- [ ] **Step 4: Write the failing loop-side test**

In `loop_.rs` `mod tests`:

```rust
    /// Finding 4.2: the maintenance target is the min of the loop window and a
    /// routed compaction model's declared window — a span the compactor can't
    /// read can't be evicted. None = unchanged.
    #[test]
    fn maint_model_limit_is_min_of_loop_and_compaction_windows() {
        let mk = |compaction_model_limit| {
            AgentLoop::new(
                Arc::new(ScriptedModel::new(vec![])),
                Arc::new(PassthroughProtocol),
                registry(),
                policy(std::env::temp_dir()),
                Arc::new(AlwaysApprove),
                Arc::new(CollectingSink::default()),
                LoopConfig {
                    model_limit: 10_000,
                    compaction_model_limit,
                    ..Default::default()
                },
            )
        };
        assert_eq!(mk(None).maint_model_limit(), 10_000);
        assert_eq!(mk(Some(4_000)).maint_model_limit(), 4_000);
        assert_eq!(
            mk(Some(20_000)).maint_model_limit(),
            10_000,
            "a larger compaction window never widens the target"
        );
    }
```

Run: `cargo test -p agent-core --lib maint_model_limit` → FAIL to COMPILE (no field/method).

- [ ] **Step 5: Add LoopConfig field + helper + rewire the three MaintCtx sites**

(a) `LoopConfig` — after `post_tool_validators`:

```rust
    /// Declared context window of a routed compaction model; None = same as
    /// `model_limit`. Maintenance targets min(model window, this) — a span the
    /// compactor cannot read cannot be evicted (finding 4.2).
    pub compaction_model_limit: Option<usize>,
```

and in `impl Default for LoopConfig`: `compaction_model_limit: None,`.

(b) Next to `effective_model_limit()`:

```rust
    /// The window maintenance targets: the calibrated loop window, further
    /// capped by a routed compaction model's declared window (finding 4.2).
    fn maint_model_limit(&self) -> usize {
        match self.config.compaction_model_limit {
            Some(cl) => self.effective_model_limit().min(cl),
            None => self.effective_model_limit(),
        }
    }
```

(c) At exactly the three `MaintCtx { model_limit: self.effective_model_limit(), ... }` construction sites (~lines 527, 714, 994 — grep `crate::MaintCtx` / `MaintCtx {`), change `model_limit: self.effective_model_limit(),` to `model_limit: self.maint_model_limit(),`. Do NOT touch any `ctx.build(self.effective_model_limit())` or `context_limit:`/Usage sites — build/request sizing keeps the loop window.

Run: `cargo test -p agent-core --lib maint_model_limit` → PASS.

- [ ] **Step 6: Write the failing assemble tests**

In `assemble.rs` `mod tests`:

```rust
    #[test]
    fn loop_config_maps_compaction_model_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        assert_eq!(
            loop_config_from(&c, dir.path().to_path_buf(), Duration::from_secs(77))
                .compaction_model_limit,
            None
        );
        c.compaction_model = Some(crate::ModelRef {
            context_limit: Some(4096),
            ..Default::default()
        });
        assert_eq!(
            loop_config_from(&c, dir.path().to_path_buf(), Duration::from_secs(77))
                .compaction_model_limit,
            Some(4096)
        );
    }

    #[test]
    fn routed_subagent_window_reaches_child_config() {
        let dir = tempfile::tempdir().unwrap();
        // Unset: child inherits the primary knobs.
        let mut c = cfg();
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        let (ml, mt) = built.child_loop_knobs.expect("subagents on by default");
        assert_eq!(ml, c.context_limit);
        assert_eq!(mt, Some(c.max_tokens));
        // Set: the ModelRef limits override the child clone.
        c.subagent_model = Some(crate::ModelRef {
            context_limit: Some(2048),
            max_tokens: Some(256),
            ..Default::default()
        });
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        assert_eq!(built.child_loop_knobs, Some((2048, Some(256))));
    }
```

Run: `cargo test -p agent-runtime-config --lib loop_config_maps_compaction routed_subagent_window` → FAIL to COMPILE.

- [ ] **Step 7: Wire assemble.rs**

(a) `loop_config_from` — add to the `LoopConfig` literal:

```rust
        compaction_model_limit: cfg
            .compaction_model
            .as_ref()
            .and_then(|m| m.context_limit),
```

(b) Child block (~line 279) — replace:

```rust
        let mut child_config = loop_config.clone();
        child_config.max_turns = cfg.subagent_max_turns;
```

with:

```rust
        let mut child_config = loop_config.clone();
        child_config.max_turns = cfg.subagent_max_turns;
        // Finding 4.2: a routed subagent model's declared limits travel with it;
        // None inherits the primary knobs already in the clone.
        if let Some(r) = &cfg.subagent_model {
            if let Some(cl) = r.context_limit {
                child_config.model_limit = cl;
            }
            if let Some(mt) = r.max_tokens {
                child_config.max_tokens = Some(mt);
            }
        }
        #[cfg(test)]
        {
            child_loop_knobs = Some((child_config.model_limit, child_config.max_tokens));
        }
```

and declare, next to the existing `#[cfg(test)] let mut subagent_model_routed ...`:

```rust
    #[cfg(test)]
    let mut child_loop_knobs: Option<(usize, Option<u32>)> = None;
```

(c) `BuiltLoop` — after `compaction_model_routed`:

```rust
    /// (child model_limit, child max_tokens) captured at DispatchDeps build;
    /// None when subagents are disabled. Pins ModelRef limit inheritance.
    #[cfg(test)]
    pub child_loop_knobs: Option<(usize, Option<u32>)>,
```

and add `child_loop_knobs,` (under `#[cfg(test)]`) to the `BuiltLoop` literal at the end of `assemble_loop`, matching how the neighboring test-only fields are populated.

- [ ] **Step 8: Run the full affected suites**

Run: `cargo test -p agent-runtime-config && cargo test -p agent-core`
Expected: PASS (including the untouched `loop_config_maps_runtime_config`, `routed_models_*` tests).

- [ ] **Step 9: fmt + commit**

```bash
cargo fmt --all && cargo fmt --all --check
git add -A && git commit -m "feat(config): ModelRef context_limit/max_tokens inheritance for routed models (audit 4.2)"
```

---

### Task 6: Full gate

**Files:** none (verification only).

- [ ] **Step 1: Full workspace test + lints**

Run from `agent/`: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all green.

- [ ] **Step 2: Repo CI gate**

Run from the worktree root: `bash scripts/ci.sh` (source `~/.cargo/env` first if cargo is missing).
Expected: exit 0 (okf check, skills lint, fmt, clippy, cargo test, conditional src-tauri, web typecheck/vitest).

- [ ] **Step 3: Confirm no stray commits on main**

```bash
git rev-parse --show-toplevel   # must print the WORKTREE path
git log --oneline main..HEAD    # exactly the five task commits (+ any fix waves)
```

---

## Out of scope (controller-level, after the branch merges)

Whole-branch review, `--no-ff` merge, ledger section in `.superpowers/sdd/progress.md`, dated re-stamps in `.agents/skills/harness-engineering/audit.md`, and memory updates are run by the controller per the triage spec — not plan tasks.
