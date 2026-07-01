# Harden Parallel Tool Dispatch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Isolate a panicking or hanging tool in the concurrent Phase-2 dispatch so one bad tool can neither crash the agent loop nor wedge the whole turn.

**Architecture:** Add a sink-free `execute_isolated` helper that wraps each tool execution in `tokio::time::timeout(ctx.timeout, AssertUnwindSafe(fut).catch_unwind())`, returning an `Executed` tag (`Ok`/`ToolErr`/`Panicked`/`TimedOut`). Phase 2 calls it and, for the two failure tags, emits a loud `AgentEvent::Error` + a tracing log while still appending an error tool-result for that `tool_call_id`. Phase 3's unreachable `None` arm is upgraded to an explicit error message for transcript validity. All changes are in one file: `agent-core/src/loop_.rs`.

**Tech Stack:** Rust (Cargo workspace `agent/`), tokio, futures.

## Global Constraints

- All changes are in `agent/crates/agent-core/src/loop_.rs`. No other crate changes. `AgentEvent::Error` and `ToolError` already exist.
- The dispatch timeout reuses `ctx.timeout` (= `config.tool_timeout`). No new config knob.
- Surfacing (decided): **panic** → error tool-result + `AgentEvent::Error` + `tracing::error!`; **timeout** → error tool-result + `AgentEvent::Error` + `tracing::warn!`.
- Do NOT change the concurrency cap, `order`-based Phase-3 ordering, gating/approval flow, or `normalize_tool_call_ids`.
- Test output must be pristine: the by-design tool panic uses the sentinel string `"SENTINEL_TEST_PANIC"` and a `Once`-installed hook that swallows only that sentinel (real panics still print).
- Run cargo from `agent/` (`source ~/.cargo/env` first if `cargo` is not on PATH).
- Conventional commit. Branch is already `fix/parallel-tool-dispatch-hardening`.

---

### Task 1: Panic + timeout isolation in Phase-2 dispatch

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` — add `Executed` enum + `execute_isolated` fn (after the `Resolved` enum, ~line 428); rewire Phase 2 (~lines 293-310); upgrade Phase 3 `None` arm (~line 316-319); add unit + loop tests in the `#[cfg(test)] mod tests` block.

**Interfaces:**
- Produces (module-private): `enum Executed { Ok(agent_tools::ToolOutput), ToolErr(String), Panicked(String), TimedOut(String) }`
- Produces (module-private): `async fn execute_isolated(tool: Arc<dyn Tool>, args: serde_json::Value, name: &str, ctx: &ToolCtx) -> Executed`
- Consumes existing: `Resolved`, `ReadyCall`, `DEFAULT_MAX_PARALLEL_TOOLS`, `AgentEvent::Error`, and test scaffolding already in the module (`ScriptedModel`, `Scripted::Calls`, `CollectingSink`, `PassthroughProtocol`, `AllowAll`, `AlwaysApprove`, `WindowContext`, `FakeTool`, `ToolSchema`, `ToolIntent`, `Access`, `ToolOutput`, `Role`, `tool_messages`).

- [ ] **Step 1: Write the failing unit tests + the sentinel panic-hook helper**

In the `#[cfg(test)] mod tests { ... }` block of `agent/crates/agent-core/src/loop_.rs`, add:

```rust
    /// Install (once) a panic hook that swallows ONLY the sentinel panic our
    /// PanicTool raises, so the expected caught-panic line does not pollute test
    /// output. Any unexpected panic still prints via the default hook. Race-free
    /// (Once), no restore needed.
    fn silence_sentinel_panics() {
        use std::sync::Once;
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            let default = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                let is_sentinel = info.payload().downcast_ref::<&str>()
                    .map(|s| *s == "SENTINEL_TEST_PANIC").unwrap_or(false);
                if !is_sentinel { default(info); }
            }));
        });
    }

    /// A tool that panics inside `execute` (with the sentinel payload).
    struct PanicTool { name: String }
    #[async_trait::async_trait]
    impl Tool for PanicTool {
        fn name(&self) -> &str { &self.name }
        fn description(&self) -> &str { "panics" }
        fn schema(&self) -> ToolSchema {
            ToolSchema { name: self.name.clone(), description: "panics".into(),
                parameters: serde_json::json!({"type":"object","properties":{}}) }
        }
        fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent { tool: self.name.clone(), access: Access::Read,
                paths: vec![], command: None, summary: "panics".into() })
        }
        async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx)
            -> Result<ToolOutput, ToolError> {
            panic!("SENTINEL_TEST_PANIC");
        }
    }

    fn test_ctx(timeout: Duration) -> ToolCtx {
        ToolCtx { workspace: std::env::temp_dir(), timeout,
            cancel: CancellationToken::new(),
            sandbox: Arc::new(agent_tools::HostExecutor) }
    }

    #[tokio::test]
    async fn execute_isolated_catches_panic() {
        silence_sentinel_panics();
        let tool: Arc<dyn Tool> = Arc::new(PanicTool { name: "boom".into() });
        let ex = execute_isolated(tool, serde_json::json!({}), "boom",
            &test_ctx(Duration::from_secs(5))).await;
        assert!(matches!(ex, Executed::Panicked(ref s) if s.contains("boom") && s.contains("panicked")),
            "panic must be caught as Executed::Panicked");
    }

    #[tokio::test(start_paused = true)]
    async fn execute_isolated_trips_timeout() {
        // Huge tool sleep vs a 100ms budget: under paused time the timeout timer
        // fires first, so this is deterministic with no real wall-clock wait.
        let tool: Arc<dyn Tool> = Arc::new(FakeTool {
            name: "slow".into(), delay_ms: 3_600_000, body: "never".into() });
        let ex = execute_isolated(tool, serde_json::json!({}), "slow",
            &test_ctx(Duration::from_millis(100))).await;
        assert!(matches!(ex, Executed::TimedOut(ref s) if s.contains("slow") && s.contains("timed out")),
            "a tool exceeding ctx.timeout must yield Executed::TimedOut");
    }

    #[tokio::test]
    async fn execute_isolated_passes_through_ok_and_err() {
        let ok_tool: Arc<dyn Tool> = Arc::new(FakeTool {
            name: "ok".into(), delay_ms: 0, body: "hi".into() });
        let ex = execute_isolated(ok_tool, serde_json::json!({}), "ok",
            &test_ctx(Duration::from_secs(5))).await;
        assert!(matches!(ex, Executed::Ok(ref o) if o.content == "hi"));

        let err_tool: Arc<dyn Tool> = Arc::new(ErrTool { name: "err".into() });
        let ex = execute_isolated(err_tool, serde_json::json!({}), "err",
            &test_ctx(Duration::from_secs(5))).await;
        assert!(matches!(ex, Executed::ToolErr(ref s) if s.starts_with("ERROR: ")));
    }

    /// A tool that returns Err (not a panic) from `execute`.
    struct ErrTool { name: String }
    #[async_trait::async_trait]
    impl Tool for ErrTool {
        fn name(&self) -> &str { &self.name }
        fn description(&self) -> &str { "errs" }
        fn schema(&self) -> ToolSchema {
            ToolSchema { name: self.name.clone(), description: "errs".into(),
                parameters: serde_json::json!({"type":"object","properties":{}}) }
        }
        fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent { tool: self.name.clone(), access: Access::Read,
                paths: vec![], command: None, summary: "errs".into() })
        }
        async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx)
            -> Result<ToolOutput, ToolError> {
            Err(ToolError::Failed { message: "nope".into(), stderr: None })
        }
    }
```

- [ ] **Step 2: Run the unit tests to verify they fail to compile**

Run: `cargo test -p agent-core execute_isolated 2>&1 | tail -20`
Expected: compile error — `execute_isolated` and `Executed` are not defined. (Compile failure is the red state.)

- [ ] **Step 3: Add the `Executed` enum and `execute_isolated` helper**

In `agent/crates/agent-core/src/loop_.rs`, immediately after the `Resolved` enum (which ends at `enum Resolved { ... }`, ~line 428), add:

```rust
/// Outcome of an isolated tool execution: the terminal result plus a tag the
/// caller uses to decide how loudly to surface it.
enum Executed {
    Ok(agent_tools::ToolOutput),
    /// Tool returned `Err` — a normal outcome, surfaced only to the model.
    ToolErr(String),
    /// Tool panicked — caught; surfaced loudly.
    Panicked(String),
    /// Dispatch timeout tripped — surfaced loudly.
    TimedOut(String),
}

/// Run one tool with panic + timeout isolation. Sink-free and free of `'static`
/// bounds so it is unit-testable without driving the loop; the caller owns event
/// emission. `catch_unwind` keeps a panicking tool from unwinding the loop's task;
/// `timeout` bounds a tool that ignores `ctx.timeout` so one hang can't wedge the
/// whole `buffer_unordered` batch.
async fn execute_isolated(tool: Arc<dyn Tool>, args: serde_json::Value, name: &str,
    ctx: &ToolCtx) -> Executed {
    use futures::FutureExt;
    let fut = std::panic::AssertUnwindSafe(tool.execute(args, ctx)).catch_unwind();
    match tokio::time::timeout(ctx.timeout, fut).await {
        Ok(Ok(Ok(output))) => Executed::Ok(output),
        Ok(Ok(Err(e)))     => Executed::ToolErr(format!("ERROR: {e}")),
        Ok(Err(_panic))    => Executed::Panicked(
            format!("ERROR: tool '{name}' panicked during execution")),
        Err(_elapsed)      => Executed::TimedOut(
            format!("ERROR: tool '{name}' timed out after {:?}", ctx.timeout)),
    }
}
```

- [ ] **Step 4: Run the unit tests to verify they pass**

Run: `cargo test -p agent-core execute_isolated 2>&1 | tail -20`
Expected: PASS — `execute_isolated_catches_panic`, `execute_isolated_trips_timeout`, `execute_isolated_passes_through_ok_and_err`. Output pristine (no stray "thread panicked" line — the sentinel hook swallowed it).

- [ ] **Step 5: Write the failing loop-level isolation tests**

In the same test module, add:

```rust
    #[tokio::test]
    async fn panicking_tool_is_isolated_from_the_batch() {
        silence_sentinel_panics();
        let mut r = ToolRegistry::new();
        r.register(Arc::new(PanicTool { name: "boom".into() }));
        r.register(Arc::new(FakeTool { name: "ok".into(), delay_ms: 0, body: "OK".into() }));
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "boom".into(), "{}".into()),
                ("c2".into(), "ok".into(), "{}".into())]),
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), Arc::new(r), Arc::new(AllowAll),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 1, temperature: 0.0,
                max_tokens: None, workspace: std::env::temp_dir(),
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60), ..Default::default() });
        let mut ctx = WindowContext::new(Message::system("sys"));

        // The panic must NOT abort the run.
        agent.run(&mut ctx, "go".into()).await.expect("panic must be isolated, run completes");

        let msgs = tool_messages(&ctx);
        let boom = msgs.iter().find(|(id, _)| id == "c1").expect("c1 tool message present");
        assert!(boom.1.contains("panicked"), "panicker yields an error tool-result: {boom:?}");
        let ok = msgs.iter().find(|(id, _)| id == "c2").expect("c2 tool message present");
        assert_eq!(ok.1, "OK", "the sibling tool still ran");

        let events = sink.events.lock().unwrap().clone();
        assert!(events.iter().any(|e| e.starts_with("error:") && e.contains("panicked")),
            "a panic emits a loud AgentEvent::Error: {events:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn hanging_tool_trips_dispatch_timeout() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(FakeTool { name: "hang".into(), delay_ms: 3_600_000, body: "never".into() }));
        r.register(Arc::new(FakeTool { name: "ok".into(), delay_ms: 0, body: "OK".into() }));
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "hang".into(), "{}".into()),
                ("c2".into(), "ok".into(), "{}".into())]),
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), Arc::new(r), Arc::new(AllowAll),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 1, temperature: 0.0,
                max_tokens: None, workspace: std::env::temp_dir(),
                tool_timeout: std::time::Duration::from_millis(100),
                stream_idle_timeout: std::time::Duration::from_secs(60), ..Default::default() });
        let mut ctx = WindowContext::new(Message::system("sys"));

        // Under paused time, the 100ms dispatch timeout fires before the 3600s sleep,
        // so the turn completes instead of hanging.
        agent.run(&mut ctx, "go".into()).await.expect("hang must be bounded, run completes");

        let msgs = tool_messages(&ctx);
        let hang = msgs.iter().find(|(id, _)| id == "c1").expect("c1 tool message present");
        assert!(hang.1.contains("timed out"), "hanger yields a timeout tool-result: {hang:?}");
        let ok = msgs.iter().find(|(id, _)| id == "c2").expect("c2 tool message present");
        assert_eq!(ok.1, "OK", "the sibling tool still ran");

        let events = sink.events.lock().unwrap().clone();
        assert!(events.iter().any(|e| e.starts_with("error:") && e.contains("timed out")),
            "a timeout emits a loud AgentEvent::Error: {events:?}");
    }
```

- [ ] **Step 6: Run the loop tests to verify they fail**

Run: `cargo test -p agent-core "tool_is_isolated" "hanging_tool" 2>&1 | tail -25`
Expected: FAIL. `panicking_tool_is_isolated_from_the_batch` fails because the panic currently unwinds the loop (the test panics / errors, not a clean isolated result). `hanging_tool_trips_dispatch_timeout` fails because no dispatch timeout exists yet — no `timed out` tool message / error event is produced.

- [ ] **Step 7: Rewire Phase 2 and upgrade the Phase 3 `None` arm**

In `agent/crates/agent-core/src/loop_.rs`, replace the Phase 2 block. Find:

```rust
            // Phase 2 — execute approved calls concurrently, bounded.
            let cap = if self.config.max_parallel_tools == 0 {
                DEFAULT_MAX_PARALLEL_TOOLS } else { self.config.max_parallel_tools };
            let executed: Vec<(String, String, Result<agent_tools::ToolOutput, ToolError>)> =
                futures::stream::iter(ready.into_iter().map(|rc| {
                    let ReadyCall { tool, args, id, name, ctx } = rc;
                    async move { (id, name, tool.execute(args, &ctx).await) }
                }))
                .buffer_unordered(cap)
                .collect()
                .await;
            for (id, name, out) in executed {
                let resolved = match out {
                    Ok(o) => Resolved::Ok(o),
                    Err(e) => Resolved::Err(format!("ERROR: {e}")),
                };
                results.insert(id, (name, resolved));
            }
```

Replace with:

```rust
            // Phase 2 — execute approved calls concurrently, bounded. Each call is
            // panic- and timeout-isolated (execute_isolated) so one bad tool can
            // neither crash the loop nor wedge the batch.
            let cap = if self.config.max_parallel_tools == 0 {
                DEFAULT_MAX_PARALLEL_TOOLS } else { self.config.max_parallel_tools };
            let executed: Vec<(String, String, Executed)> =
                futures::stream::iter(ready.into_iter().map(|rc| {
                    let ReadyCall { tool, args, id, name, ctx } = rc;
                    async move {
                        let ex = execute_isolated(tool, args, &name, &ctx).await;
                        (id, name, ex)
                    }
                }))
                .buffer_unordered(cap)
                .collect()
                .await;
            for (id, name, ex) in executed {
                let resolved = match ex {
                    Executed::Ok(o) => Resolved::Ok(o),
                    Executed::ToolErr(s) => Resolved::Err(s),
                    Executed::Panicked(s) => {
                        tracing::error!(target: "loop", tool = %name,
                            "tool panicked during parallel dispatch");
                        self.sink.emit(AgentEvent::Error(s.clone()));
                        Resolved::Err(s)
                    }
                    Executed::TimedOut(s) => {
                        tracing::warn!(target: "loop", tool = %name,
                            timeout = ?self.config.tool_timeout,
                            "tool timed out during parallel dispatch");
                        self.sink.emit(AgentEvent::Error(s.clone()));
                        Resolved::Err(s)
                    }
                };
                results.insert(id, (name, resolved));
            }
```

Then, in Phase 3, find:

```rust
                let (name, resolved) = match results.remove(&id) {
                    Some(v) => v,
                    None => continue,
                };
```

Replace with:

```rust
                let (name, resolved) = match results.remove(&id) {
                    Some(v) => v,
                    // Unreachable while normalize_tool_call_ids holds. If a future
                    // change ever breaks the one-slot-per-id invariant, emit an error
                    // rather than silently drop the result and desync the transcript
                    // (an assistant tool_call with no matching tool message).
                    None => (String::new(), Resolved::Err(
                        format!("ERROR: internal: no result for tool_call_id {id}"))),
                };
```

- [ ] **Step 8: Run the full agent-core suite and a non-test build**

Run: `cargo test -p agent-core 2>&1 | tail -20`
Expected: all pass, including the four isolation tests. Output pristine (no stray panic line).

Run: `cargo build -p agent-core 2>&1 | tail -5`
Expected: `Finished` — no warnings (no `dead_code`: `execute_isolated`/`Executed` are used by Phase 2).

- [ ] **Step 9: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs
git commit -m "fix(core): isolate panicking/hanging tools in parallel dispatch

Wrap each concurrent tool execution in catch_unwind + tokio::time::timeout
(ctx.timeout) via execute_isolated, so one tool that panics or hangs can
neither abort the AgentLoop nor wedge the buffer_unordered batch. Both are
surfaced loudly (AgentEvent::Error + tracing) and yield an error tool-result
for their tool_call_id. Phase-3 None arm upgraded to an explicit error message
for transcript validity. Audit Finding 1."
```

---

## Final verification

- [ ] From `agent/`: `cargo test -p agent-core 2>&1 | tail -20` — all green, output pristine.
- [ ] From `agent/`: `cargo build 2>&1 | tail -5` — whole workspace compiles, no new warnings.
- [ ] Confirm no behavior change to the happy path: `parallel_tool_calls_execute_concurrently` and `tool_results_keep_model_call_order_despite_completion_order` still pass unchanged.

## Notes for the implementer

- `HostExecutor` is `agent_tools::HostExecutor` (a `SandboxStrategy`), used only to build a `ToolCtx` in unit tests.
- `catch_unwind` relies on the default `panic = "unwind"` strategy; the workspace uses it. If a profile ever sets `panic = "abort"`, isolation degrades to process-abort — out of scope here.
- Do not restore the panic hook after `silence_sentinel_panics()` — it delegates all non-sentinel panics to the default hook, so leaving it installed is correct and race-free (`Once`).
- The `ctx` variable inside the Phase-2 closure is the per-call `ToolCtx` moved out of `ReadyCall` (shadowing is intentional and pre-existing); `&ctx` and `&name` borrows live only across the `.await` inside the closure.
