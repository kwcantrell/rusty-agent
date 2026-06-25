# B3 — Live Cancellation Wiring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a hung tool or runaway turn abortable by threading a caller-owned `CancellationToken` through the loop, and wire CLI Ctrl-C as the source.

**Architecture:** A new `AgentLoop::run_with_cancel(ctx, input, cancel)` carries the real token; the existing `run(ctx, input)` becomes a one-line wrapper delegating with a fresh `CancellationToken::new()`. The token is checked at the turn boundary, inside the model-stream `select!`, and (already) by tools via `ctx.cancel`. On cancel the loop emits `Done(StopReason::Cancelled)` and returns `Ok(())`.

**Spec refinement (intentional):** the spec said "`run` takes the token; update every call site." This plan instead adds a backward-compatible `run_with_cancel` + keeps `run` as a wrapper — same behavior, but the server and all ~24 existing test call sites stay unchanged; only the CLI and the new tests use the token. This is the idiomatic `_with_` split and avoids mechanical churn.

**Tech Stack:** Rust, `tokio` (`signal` via `features=["full"]`), `tokio-util` (`CancellationToken`), `async_trait`. Tests use the `agent-core` `testkit` harness + `tokio::sync::Notify`.

## Global Constraints

- TDD: failing test first, watch it fail, then the minimal change.
- Run tests from the workspace root: `cd agent` first. `source ~/.cargo/env` if needed.
- Two new types, both additive: `StopReason::Cancelled` (`agent-model`), `AgentError::Cancelled` (`agent-core`). No existing variant changes.
- `run(ctx, input)` keeps working unchanged for the server and all existing tests (it delegates to `run_with_cancel` with an un-cancelled token → identical behavior).
- A cancelled run returns `Ok(())` and emits `AgentEvent::Done(StopReason::Cancelled)`.
- Out of scope: interactive server cancel (wire `Cancel` message / per-session token) — Cluster A.

## Reference — confirmed shapes

- `loop_.rs`: `pub enum AgentError { #[error("model error after retries: {0}")] Model(String) }` (~14-17). `run(&self, ctx: &mut dyn ContextManager, user_input: String) -> Result<(), AgentError>` (~133); turn loop `for turn in 0..self.config.max_turns {` (~150) with `AgentEvent::Usage{..}` emitted first (~152); `let assistant = self.completion_with_retry(&base).await?;` (~171); `self.gate_tool(call).await` (~212); `one_completion(&self, req) -> Result<AssistantTurn, ModelError>` (~85) with two `tokio::time::timeout(idle, …)` points (~87, ~96); `completion_with_retry(&self, base) -> Result<AssistantTurn, AgentError>` (~112); `gate_tool` builds `ToolCtx { …, cancel: CancellationToken::new(), … }` (~307-309). `use tokio_util::sync::CancellationToken;` is already imported at the top of `loop_.rs`.
- `agent-model/src/types.rs`: `pub enum StopReason { #[default] Stop, ToolCalls, Length, BudgetExhausted }`.
- `agent-server/src/wire.rs`: private `fn stop_reason_str(r: &StopReason) -> &'static str` (89-95, exhaustive); `#[cfg(test)] mod tests` at 120.
- `agent-cli/src/main.rs:240`: `if let Err(e) = agent.run(&mut ctx, input.to_string()).await { … }` inside the REPL loop; `main` is `#[tokio::main]`.

---

### Task 1: Add `StopReason::Cancelled` and its wire mapping

**Files:**
- Modify: `agent/crates/agent-model/src/types.rs` (the `StopReason` enum)
- Modify: `agent/crates/agent-server/src/wire.rs:89-95` (`stop_reason_str`)
- Test: `agent/crates/agent-server/src/wire.rs` (`mod tests`)

**Interfaces:**
- Produces: `StopReason::Cancelled`; `stop_reason_str(&StopReason::Cancelled) == "cancelled"`.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `wire.rs`:

```rust
#[test]
fn cancelled_stop_reason_maps_to_wire_string() {
    assert_eq!(super::stop_reason_str(&StopReason::Cancelled), "cancelled");
}
```

(If `StopReason` is not already in scope in the test module, add `use agent_model::StopReason;` to the test.)

- [ ] **Step 2: Run it to verify it fails**

Run: `cd agent && cargo test -p agent-server cancelled_stop_reason_maps_to_wire_string`
Expected: FAIL to compile — `no variant named Cancelled found for enum StopReason`.

- [ ] **Step 3: Add the enum variant**

In `agent-model/src/types.rs`, add `Cancelled` to `StopReason`:

```rust
pub enum StopReason { #[default] Stop, ToolCalls, Length, BudgetExhausted, Cancelled }
```

- [ ] **Step 4: Add the wire arm**

In `agent-server/src/wire.rs`, add to `stop_reason_str`:

```rust
        StopReason::Cancelled => "cancelled",
```

- [ ] **Step 5: Run the test + both crates**

Run: `cd agent && cargo test -p agent-model -p agent-server`
Expected: PASS — new test passes; the `openai.rs`/`claude_cli.rs` `StopReason` mappings use a wildcard so they still compile; all existing tests green.

- [ ] **Step 6: Commit**

```bash
cd agent && git add crates/agent-model/src/types.rs crates/agent-server/src/wire.rs
git commit -m "feat(model): add StopReason::Cancelled + wire mapping"
```

---

### Task 2: Cancellable loop in agent-core

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` — `AgentError`, `run`/`run_with_cancel`, `one_completion`, `completion_with_retry`, `gate_tool`.
- Test: `agent/crates/agent-core/src/loop_.rs` (`mod tests`).

**Interfaces:**
- Produces: `AgentError::Cancelled`; `pub async fn run_with_cancel(&self, ctx: &mut dyn ContextManager, user_input: String, cancel: CancellationToken) -> Result<(), AgentError>`; `run(...)` delegates to it with a fresh token.
- Consumes (tests): `ScriptedModel`, `Scripted::{Call,Text}`, `PassthroughProtocol`, `CollectingSink`, `AlwaysApprove`, `registry()`/`policy()`, `AgentLoop::new`, `LoopConfig`, `WindowContext`, `tokio::sync::Notify`, `CancellationToken`.

- [ ] **Step 1: Write the failing boundary test**

Add to `mod tests` in `loop_.rs`:

```rust
#[tokio::test]
async fn precancelled_token_stops_before_calling_model() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();
    let model = Arc::new(ScriptedModel::new(vec![Scripted::Text("should never run".into())]));
    let sink = Arc::new(CollectingSink::default());
    let agent = AgentLoop::new(
        model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
        Arc::new(AlwaysApprove), sink.clone(),
        LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 2, temperature: 0.0,
            max_tokens: None, workspace: ws, tool_timeout: std::time::Duration::from_secs(5),
            stream_idle_timeout: std::time::Duration::from_secs(60), ..Default::default() });
    let mut ctx = WindowContext::new(Message::system("sys"));
    let cancel = CancellationToken::new();
    cancel.cancel(); // already cancelled before the run starts

    agent.run_with_cancel(&mut ctx, "go".into(), cancel).await.unwrap();

    // Stopped at the turn boundary: only the terminal Done(Cancelled) event, no
    // Usage / Token events (the model was never consulted).
    let events = sink.events.lock().unwrap().clone();
    assert_eq!(events, vec!["done".to_string()], "events were: {events:?}");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd agent && cargo test -p agent-core --lib precancelled_token_stops_before_calling_model`
Expected: FAIL to compile — `no method named run_with_cancel`.

- [ ] **Step 3: Add `AgentError::Cancelled`**

In `loop_.rs`:

```rust
pub enum AgentError {
    #[error("model error after retries: {0}")]
    Model(String),
    #[error("cancelled")]
    Cancelled,
}
```

- [ ] **Step 4: Rename `run` → `run_with_cancel` (add the token) and add the `run` wrapper**

Change the `run` signature to `run_with_cancel` with the token:

```rust
pub async fn run_with_cancel(&self, ctx: &mut dyn ContextManager, user_input: String,
                             cancel: CancellationToken) -> Result<(), AgentError> {
```

Add the boundary check as the **first statement inside** `for turn in 0..self.config.max_turns {` (before the `AgentEvent::Usage` emit):

```rust
        for turn in 0..self.config.max_turns {
            if cancel.is_cancelled() {
                self.sink.emit(AgentEvent::Done(StopReason::Cancelled));
                return Ok(());
            }
```

Replace the model call (`let assistant = self.completion_with_retry(&base).await?;`) with:

```rust
            let assistant = match self.completion_with_retry(&base, &cancel).await {
                Ok(t) => t,
                Err(AgentError::Cancelled) => {
                    self.sink.emit(AgentEvent::Done(StopReason::Cancelled));
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
```

Replace `self.gate_tool(call).await` with `self.gate_tool(call, &cancel).await`.

Add the thin wrapper immediately after `run_with_cancel` (preserves the existing `run(ctx, input)` API for the server and all existing tests):

```rust
    /// Convenience entry point with no external cancel source (server + tests).
    /// Live cancellation goes through [`Self::run_with_cancel`].
    pub async fn run(&self, ctx: &mut dyn ContextManager, user_input: String)
        -> Result<(), AgentError> {
        self.run_with_cancel(ctx, user_input, CancellationToken::new()).await
    }
```

- [ ] **Step 5: Thread the token through `completion_with_retry` and `one_completion`**

`completion_with_retry` — add the param and the no-retry-on-cancel guard:

```rust
    async fn completion_with_retry(&self, base: &CompletionRequest, cancel: &CancellationToken)
        -> Result<AssistantTurn, AgentError> {
        let mut attempt = 0;
        loop {
            let mut req = base.clone();
            self.protocol.prepare(&mut req);
            match self.one_completion(req, cancel).await {
                Ok(turn) => return Ok(turn),
                Err(e) => {
                    if cancel.is_cancelled() { return Err(AgentError::Cancelled); }
                    attempt += 1;
                    if attempt > self.config.max_retries {
                        self.sink.emit(AgentEvent::Error(e.to_string()));
                        return Err(AgentError::Model(e.to_string()));
                    }
                    tracing::warn!(attempt, error = %e, "model error, retrying");
                    tokio::time::sleep(Duration::from_millis(100 * attempt as u64)).await;
                }
            }
        }
    }
```

`one_completion` — add the param and a `cancel.cancelled()` arm to both `select!`s (currently plain `timeout` awaits). Replace the stream-open:

```rust
    async fn one_completion(&self, req: CompletionRequest, cancel: &CancellationToken)
        -> Result<AssistantTurn, ModelError> {
        let idle = self.config.stream_idle_timeout;
        let mut stream = tokio::select! {
            _ = cancel.cancelled() => return Err(ModelError::Stream("cancelled".into())),
            opened = tokio::time::timeout(idle, self.model.stream(req)) => match opened {
                Err(_) => return Err(ModelError::Timeout(idle)),
                Ok(opened) => opened?,
            },
        };
```

and the per-chunk loop body’s `match tokio::time::timeout(idle, stream.next()).await { … }` becomes:

```rust
        loop {
            let step = tokio::select! {
                _ = cancel.cancelled() => return Err(ModelError::Stream("cancelled".into())),
                s = tokio::time::timeout(idle, stream.next()) => s,
            };
            match step {
                Err(_) => return Err(ModelError::Timeout(idle)),
                Ok(None) => break,
                Ok(Some(item)) => match item? {
                    Chunk::Text(t) => { self.sink.emit(AgentEvent::Token(t.clone())); text.push_str(&t); }
                    Chunk::Reasoning(r) => { self.sink.emit(AgentEvent::Reasoning(r.clone())); reasoning.push_str(&r); }
                    Chunk::ToolCallDelta(rc) => merge_tool_call(&mut raw_tool_calls, rc),
                    Chunk::Done(r) => stop = r,
                },
            }
        }
```

(The on-cancel `ModelError` value is irrelevant — `completion_with_retry` checks the token and returns `AgentError::Cancelled` before any retry.)

- [ ] **Step 6: Wire the token into `gate_tool`**

Change the signature and the `ToolCtx`:

```rust
    async fn gate_tool(&self, call: ToolCall, cancel: &CancellationToken) -> GateOutcome {
```

and replace `cancel: CancellationToken::new(),` in the `ToolCtx { … }` with:

```rust
            cancel: cancel.clone(),
```

(Delete or update the stale `// NOTE: this token is currently inert …` comment at ~302-304 — it is no longer inert.)

- [ ] **Step 7: Run the boundary test (should pass now)**

Run: `cd agent && cargo test -p agent-core --lib precancelled_token_stops_before_calling_model`
Expected: PASS.

- [ ] **Step 8: Write the failing hung-tool test**

Add to `mod tests` in `loop_.rs`. Add imports at the top of the test module:
`use agent_tools::{Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema, Access};`
and `use serde_json::Value;` (if not already in scope).

```rust
struct HangsUntilCancel { started: Arc<tokio::sync::Notify> }

#[async_trait::async_trait]
impl Tool for HangsUntilCancel {
    fn name(&self) -> &str { "hang" }
    fn description(&self) -> &str { "hangs until cancelled" }
    fn schema(&self) -> ToolSchema {
        ToolSchema { name: "hang".into(), description: "".into(),
            parameters: serde_json::json!({"type":"object"}) }
    }
    fn intent(&self, _args: &Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent { tool: "hang".into(), access: Access::Read, paths: vec![],
            command: None, summary: "hang".into() })
    }
    async fn execute(&self, _args: Value, ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        self.started.notify_one();
        ctx.cancel.cancelled().await; // blocks until the loop's token is cancelled
        Err(ToolError::Timeout)
    }
}

#[tokio::test]
async fn cancel_aborts_a_hung_tool() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();
    let started = Arc::new(tokio::sync::Notify::new());
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(HangsUntilCancel { started: started.clone() }));
    let registry = Arc::new(reg);
    let model = Arc::new(ScriptedModel::new(vec![
        Scripted::Call("c1".into(), "hang".into(), "{}".into()),
        Scripted::Text("after".into()),
    ]));
    let sink = Arc::new(CollectingSink::default());
    let agent = AgentLoop::new(
        model, Arc::new(PassthroughProtocol), registry, policy(ws.clone()),
        Arc::new(AlwaysApprove), sink.clone(),
        LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 2, temperature: 0.0,
            max_tokens: None, workspace: ws, tool_timeout: std::time::Duration::from_secs(5),
            stream_idle_timeout: std::time::Duration::from_secs(60), ..Default::default() });
    let mut ctx = WindowContext::new(Message::system("sys"));

    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    // Cancel as soon as the tool reports it has started and is blocking.
    let waiter = tokio::spawn(async move { started.notified().await; c2.cancel(); });

    // Without cancellation this never returns (the tool blocks forever); returning
    // at all proves the hang was aborted.
    agent.run_with_cancel(&mut ctx, "go".into(), cancel).await.unwrap();
    waiter.await.unwrap();

    assert_eq!(sink.events.lock().unwrap().last().unwrap(), "done");
}
```

- [ ] **Step 9: Run the hung-tool test**

Run: `cd agent && cargo test -p agent-core --lib cancel_aborts_a_hung_tool`
Expected: PASS (and it returns promptly; if cancellation were broken it would hang until the test harness times out).

- [ ] **Step 10: Run the full crate suite (regression)**

Run: `cd agent && cargo test -p agent-core --lib`
Expected: PASS — all existing tests (which call the unchanged `run(ctx, input)` wrapper) stay green; no warnings about unused variables.

- [ ] **Step 11: Commit**

```bash
cd agent && git add crates/agent-core/src/loop_.rs
git commit -m "feat(loop): live cancellation via run_with_cancel

Thread a caller-owned CancellationToken through the turn loop, the model-stream
select, and every ToolCtx; emit Done(Cancelled) and return Ok on cancel. run()
stays as a no-cancel-source wrapper so the server and existing tests are unchanged."
```

---

### Task 3: CLI Ctrl-C source

**Files:**
- Modify: `agent/crates/agent-cli/Cargo.toml` (add `tokio-util`)
- Modify: `agent/crates/agent-cli/src/main.rs:240-242` (the REPL `run` call)

**Interfaces:**
- Consumes: `AgentLoop::run_with_cancel`, `tokio_util::sync::CancellationToken`, `tokio::signal::ctrl_c`.

- [ ] **Step 1: Add the `tokio-util` dependency**

In `agent/crates/agent-cli/Cargo.toml`, under `[dependencies]`, add:

```toml
tokio-util.workspace = true
```

- [ ] **Step 2: Replace the REPL run call with a Ctrl-C-cancellable run**

In `agent-cli/src/main.rs`, replace:

```rust
        if let Err(e) = agent.run(&mut ctx, input.to_string()).await {
            eprintln!("\x1b[31mfatal: {e}\x1b[0m");
        }
```

with:

```rust
        let cancel = tokio_util::sync::CancellationToken::new();
        let run = agent.run_with_cancel(&mut ctx, input.to_string(), cancel.clone());
        tokio::pin!(run);
        let result = loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => { cancel.cancel(); eprintln!("\n^C cancelling…"); }
                r = &mut run => break r,
            }
        };
        if let Err(e) = result {
            eprintln!("\x1b[31mfatal: {e}\x1b[0m");
        }
```

- [ ] **Step 3: Build and run the existing CLI tests**

Run: `cd agent && cargo build -p agent-cli && cargo test -p agent-cli`
Expected: PASS — compiles with the new dep and `run_with_cancel`; existing CLI tests stay green. (The signal path is not unit-tested; it is verified by compilation + the agent-core cancellation tests in Task 2.)

- [ ] **Step 4: Commit**

```bash
cd agent && git add crates/agent-cli/Cargo.toml crates/agent-cli/src/main.rs
git commit -m "feat(cli): cancel the running turn on Ctrl-C

Wrap run_with_cancel in a select against tokio::signal::ctrl_c; the first Ctrl-C
cancels the turn and we keep awaiting run so in-flight tools tear down cleanly."
```

---

### Task 4: Final verification

**Files:** none (verification only)

- [ ] **Step 1: Workspace build**

Run: `cd agent && cargo build`
Expected: PASS — all crates compile, including the unchanged server call site (`daemon.rs:111` still calls `run`).

- [ ] **Step 2: Full test sweep**

Run: `cd agent && cargo test -p agent-model -p agent-core -p agent-server -p agent-cli -p agent-runtime-config 2>&1 | grep "test result"`
Expected: all `ok`, no failures. (e2e tests requiring a live server remain `ignored`.)

- [ ] **Step 3: Confirm the spec's testing checklist is satisfied**

Cross-check against `docs/superpowers/specs/2026-06-25-live-cancellation-design.md` → "Testing": boundary check, tool-hang abort, un-cancelled-token regression, and the `StopReason::Cancelled` wire mapping. All present and passing. If any is missing, add it before finishing.
