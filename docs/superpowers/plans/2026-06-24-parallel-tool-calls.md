# Parallel Tool-Call Concurrency Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Execute an assistant turn's parallel tool calls concurrently (bounded), while keeping approval prompts serialized and tool-result ordering deterministic, and lock the multi-call behavior with tests.

**Architecture:** Replace the sequential `for call … run_tool(call).await` block in `AgentLoop::run` with three phases: (1) **gate** every call sequentially (resolve tool → intent → policy → interactive approval, one prompt at a time), (2) **execute** the approved calls concurrently via `buffer_unordered(cap)`, (3) **append** one `role:"tool"` message per call in the model's original call order. A new `max_parallel_tools` config bounds concurrency.

**Tech Stack:** Rust, tokio, `futures` (`StreamExt::buffer_unordered`), `async_trait`. Crates: `agent-core` (the loop), `agent-tools`, `agent-policy`, `agent-model`.

## Global Constraints

- Cargo is not on PATH by default: run `source ~/.cargo/env` before any cargo command.
- All work is under `agent/` (the cargo workspace root). Run tests with `cargo test -p agent-core`.
- Lint clean: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` must pass.
- Preserve existing tool-result semantics exactly: a successful tool emits an `AgentEvent::ToolResult` and appends its `output.content`; a denied/failed/unknown tool emits **no** `ToolResult` event but still appends a `role:"tool"` message whose content is `format!("ERROR: {e}")`. The turn never aborts on a single tool failure.
- `LoopConfig` derives `Default`; test/e2e construction sites use `..Default::default()`, but the two production sites (`agent-cli/src/main.rs`, `agent-server/src/runtime.rs`) list **all** fields explicitly — a new field must be added there too.

---

### Task 1: Add a multi-call variant to the test model

**Files:**
- Modify: `agent/crates/agent-core/src/testkit.rs` (enum `Scripted` ~line 12, `stream()` match ~line 40)
- Test: `agent/crates/agent-core/src/loop_.rs` (tests module, after the existing `merge_tool_call_*` tests ~line 312)

**Interfaces:**
- Produces: `Scripted::Calls(Vec<(String, String, String)>)` — one scripted assistant turn emitting N native tool-call deltas `(id, name, json_args)` with ascending `index`, terminated by `Chunk::Done(StopReason::ToolCalls)`. Later tasks use it to drive multi-call turns.

- [ ] **Step 1: Add the `Calls` variant to the `Scripted` enum**

In `testkit.rs`, add to `enum Scripted` (keep the single-call `Call` as sugar):

```rust
    /// One assistant turn emitting several native tool calls: each (id, name, json-args).
    Calls(Vec<(String, String, String)>),
```

- [ ] **Step 2: Handle `Calls` in `ScriptedModel::stream`**

In the `match next { … }` in `testkit.rs`, add an arm (after the `Scripted::Call` arm):

```rust
            Scripted::Calls(calls) => {
                let mut chunks: Vec<Result<Chunk, ModelError>> = Vec::new();
                for (i, (id, name, args)) in calls.into_iter().enumerate() {
                    chunks.push(Ok(Chunk::ToolCallDelta(RawToolCall {
                        index: Some(i), id: Some(id), name: Some(name), args_fragment: args })));
                }
                chunks.push(Ok(Chunk::Done(StopReason::ToolCalls)));
                Ok(stream::iter(chunks).boxed())
            }
```

- [ ] **Step 3: Write a test that the variant yields N native deltas**

In the `loop_.rs` `#[cfg(test)] mod tests`, add (the module already has `use crate::testkit::*;`; add `use agent_model::{Chunk, CompletionRequest, ModelClient};` and `use futures::StreamExt;` to the test module if not present):

```rust
    #[tokio::test]
    async fn scripted_calls_yields_multiple_native_tool_calls() {
        let model = ScriptedModel::new(vec![Scripted::Calls(vec![
            ("c1".into(), "f0".into(), "{}".into()),
            ("c2".into(), "f1".into(), "{}".into())])]);
        let mut stream = model.stream(CompletionRequest::default()).await.unwrap();
        let mut raw = Vec::new();
        while let Some(item) = stream.next().await {
            if let Chunk::ToolCallDelta(rc) = item.unwrap() { raw.push(rc); }
        }
        assert_eq!(raw.len(), 2);
        assert_eq!(raw[0].name.as_deref(), Some("f0"));
        assert_eq!(raw[1].id.as_deref(), Some("c2"));
    }
```

- [ ] **Step 4: Run the test**

Run: `source ~/.cargo/env && cargo test -p agent-core scripted_calls_yields_multiple_native_tool_calls`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/testkit.rs agent/crates/agent-core/src/loop_.rs
git commit -m "test(agent-core): add Scripted::Calls multi-tool-call test variant"
```

---

### Task 2: Concurrent two-phase tool execution

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` — add config field + const (~line 20-42), replace the tool block in `run` (lines 181-192), split `run_tool` (lines 198-239) into `gate_tool`, add private types.
- Modify: `agent/crates/agent-cli/src/main.rs:207` (LoopConfig literal) — add field.
- Modify: `agent/crates/agent-server/src/runtime.rs:225` (LoopConfig literal) — add field.
- Test: `agent/crates/agent-core/src/loop_.rs` (tests module).

**Interfaces:**
- Consumes: `Scripted::Calls` (Task 1); `ToolRegistry::get -> Option<Arc<dyn Tool>>`; `Tool::{intent, execute}`; `PolicyEngine::check -> Decision::{Allow, Deny(String), Ask}`; `ApprovalChannel::request`.
- Produces: `LoopConfig.max_parallel_tools: usize` (0 ⇒ `DEFAULT_MAX_PARALLEL_TOOLS`); `pub const DEFAULT_MAX_PARALLEL_TOOLS: usize = 8;`. Private to the loop: `enum GateOutcome`, `struct ReadyCall`, `enum Resolved`, `async fn gate_tool(&self, call: ToolCall) -> GateOutcome`.

- [ ] **Step 1: Write the concurrency-proof test (must hang/fail on current sequential code)**

In the `loop_.rs` tests module, add a permissive policy + a barrier tool + the test. (Add imports to the test module: `use agent_policy::PolicyEngine;` is already present; add `use agent_tools::{Tool, ToolIntent, Access, ToolOutput, ToolSchema, ToolCtx, ToolError};` and `use agent_model::Role;`.)

```rust
    struct AllowAll;
    impl PolicyEngine for AllowAll {
        fn check(&self, _i: &ToolIntent) -> Decision { Decision::Allow }
    }

    /// Tool that blocks on a shared 2-party barrier — only completes if a sibling
    /// call runs concurrently. Sequential execution deadlocks it.
    struct BarrierTool { name: String, barrier: Arc<tokio::sync::Barrier> }
    #[async_trait::async_trait]
    impl Tool for BarrierTool {
        fn name(&self) -> &str { &self.name }
        fn description(&self) -> &str { "waits on a shared barrier" }
        fn schema(&self) -> ToolSchema {
            ToolSchema { name: self.name.clone(), description: "barrier".into(),
                parameters: serde_json::json!({"type":"object","properties":{}}) }
        }
        fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent { tool: self.name.clone(), access: Access::Read,
                paths: vec![], command: None, summary: "barrier".into() })
        }
        async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx)
            -> Result<ToolOutput, ToolError> {
            self.barrier.wait().await;
            Ok(ToolOutput { content: format!("{} done", self.name), display: None })
        }
    }

    #[tokio::test]
    async fn parallel_tool_calls_execute_concurrently() {
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let mut r = ToolRegistry::new();
        r.register(Arc::new(BarrierTool { name: "wait_a".into(), barrier: barrier.clone() }));
        r.register(Arc::new(BarrierTool { name: "wait_b".into(), barrier: barrier.clone() }));
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "wait_a".into(), "{}".into()),
                ("c2".into(), "wait_b".into(), "{}".into())]),
            Scripted::Text("both done".into()),
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
        // Sequential execution would block wait_a forever (wait_b never starts) -> timeout.
        let res = tokio::time::timeout(std::time::Duration::from_secs(5),
            agent.run(&mut ctx, "go".into())).await;
        assert!(res.is_ok(), "parallel calls did not run concurrently (barrier deadlock)");
        res.unwrap().unwrap();
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(events.iter().filter(|e| e.starts_with("tool_result:")).count(), 2);
    }
```

- [ ] **Step 2: Run it and watch it fail (deadlock → timeout)**

Run: `source ~/.cargo/env && cargo test -p agent-core parallel_tool_calls_execute_concurrently -- --nocapture`
Expected: FAIL on `assert!(res.is_ok(), …)` — the current sequential loop deadlocks on the barrier and the 5s timeout fires.

- [ ] **Step 3: Add the config field and concurrency default const**

In `loop_.rs`, below `DEFAULT_STREAM_IDLE_TIMEOUT` (~line 20):

```rust
/// Default bound on how many of a turn's tool calls execute concurrently.
/// A `LoopConfig.max_parallel_tools` of 0 (the `Default`) resolves to this.
pub const DEFAULT_MAX_PARALLEL_TOOLS: usize = 8;
```

Add the field to `LoopConfig` (after `sandbox`):

```rust
    /// Max tool calls from one assistant turn to execute concurrently.
    /// 0 (the default) means `DEFAULT_MAX_PARALLEL_TOOLS`.
    pub max_parallel_tools: usize,
```

- [ ] **Step 4: Add the private phase types and `gate_tool`, replacing the body of `run_tool`**

In `loop_.rs`, add `use std::collections::HashMap;` and `use agent_tools::Tool;` to the file's imports. Add these private types above `impl AgentLoop` (or just below it):

```rust
/// A call that passed policy/approval and is ready to execute.
struct ReadyCall {
    tool: Arc<dyn Tool>,
    args: serde_json::Value,
    id: String,
    name: String,
    ctx: ToolCtx,
}

/// Outcome of gating a single call before execution.
enum GateOutcome {
    Ready(ReadyCall),
    /// Rejected before execution (unknown tool / intent error / denied). `content`
    /// is the final `ERROR: …` text to append as this call's tool result.
    Rejected { id: String, name: String, content: String },
}

/// Final per-call result feeding the tool-result message.
enum Resolved {
    Ok(agent_tools::ToolOutput),
    /// Terminal `ERROR: …` content (rejected, failed, or timed out).
    Err(String),
}
```

Replace `run_tool` (lines 198-239) with `gate_tool`, which keeps the existing emit/policy/approval logic but returns a `GateOutcome` instead of executing:

```rust
    /// Resolve, policy-check, and (if needed) get approval for one call — but do NOT
    /// execute it. Sequential by design so approval prompts never overlap.
    async fn gate_tool(&self, call: ToolCall) -> GateOutcome {
        self.sink.emit(AgentEvent::ToolStart { name: call.name.clone(), args: call.args.clone() });
        let tool = match self.tools.get(&call.name) {
            Some(t) => t,
            None => return GateOutcome::Rejected { id: call.id, name: call.name.clone(),
                content: format!("ERROR: {}",
                    ToolError::NotFound(format!("unknown tool {}", call.name))) },
        };
        let intent = match tool.intent(&call.args) {
            Ok(i) => i,
            Err(e) => return GateOutcome::Rejected { id: call.id, name: call.name,
                content: format!("ERROR: {e}") },
        };
        let allowed = match self.policy.check(&intent) {
            Decision::Allow => true,
            Decision::Deny(reason) => return GateOutcome::Rejected { id: call.id, name: call.name,
                content: format!("ERROR: {}", ToolError::Denied(reason)) },
            Decision::Ask => {
                let d = self.config.sandbox.as_ref()
                    .map(|s| s.describe())
                    .unwrap_or(agent_tools::SandboxDescriptor {
                        mode: agent_tools::Mode::Off, mechanism: "host", image: None,
                        network: true, degraded: None });
                let posture = if d.degraded.is_some() {
                    format!(" (sandbox: {} unavailable->host, network on)", d.mechanism)
                } else {
                    format!(" (sandbox: {}, network {})",
                        d.mechanism, if d.network { "on" } else { "off" })
                };
                let mut intent = intent;
                if intent.command.is_some() { intent.summary.push_str(&posture); }
                let req = ApprovalRequest { intent, display: None };
                self.sink.emit(AgentEvent::Approval(req.clone()));
                matches!(self.approval.request(req).await,
                    ApprovalResponse::Approve | ApprovalResponse::ApproveAlways)
            }
        };
        if !allowed {
            return GateOutcome::Rejected { id: call.id, name: call.name,
                content: format!("ERROR: {}", ToolError::Denied("user declined".into())) };
        }
        let sandbox = self.config.sandbox.clone()
            .unwrap_or_else(|| std::sync::Arc::new(agent_tools::HostExecutor));
        let ctx = ToolCtx { workspace: self.config.workspace.clone(),
            timeout: self.config.tool_timeout, cancel: CancellationToken::new(), sandbox };
        GateOutcome::Ready(ReadyCall { tool, args: call.args, id: call.id, name: call.name, ctx })
    }
```

- [ ] **Step 5: Replace the sequential `for` block in `run` with the three phases**

In `run`, replace lines 181-192 (`for call in parsed.tool_calls { … }`) with:

```rust
            // Phase 1 — gate every call sequentially (one approval prompt at a time).
            let mut order: Vec<String> = Vec::with_capacity(parsed.tool_calls.len());
            let mut results: HashMap<String, (String, Resolved)> = HashMap::new();
            let mut ready: Vec<ReadyCall> = Vec::new();
            for call in parsed.tool_calls {
                match self.gate_tool(call).await {
                    GateOutcome::Rejected { id, name, content } => {
                        order.push(id.clone());
                        results.insert(id, (name, Resolved::Err(content)));
                    }
                    GateOutcome::Ready(rc) => {
                        order.push(rc.id.clone());
                        ready.push(rc);
                    }
                }
            }

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

            // Phase 3 — append one tool message per call, in the model's call order.
            for id in order {
                let (name, resolved) = results.remove(&id)
                    .expect("every gated call id has a result");
                let content = match resolved {
                    Resolved::Ok(output) => {
                        self.sink.emit(AgentEvent::ToolResult {
                            name: name.clone(), output: output.clone() });
                        output.content
                    }
                    Resolved::Err(content) => content,
                };
                ctx.append(Message::tool(id, name, content));
            }
```

- [ ] **Step 6: Add the new field to both production `LoopConfig` literals**

In `agent/crates/agent-cli/src/main.rs:207`, inside the `LoopConfig { … }`, add after `sandbox: Some(sandbox.clone()),`:

```rust
            max_parallel_tools: 8,
```

In `agent/crates/agent-server/src/runtime.rs:225`, inside the `LoopConfig { … }`, add after `sandbox: Some(build_sandbox(cfg)),`:

```rust
            max_parallel_tools: 8,
```

- [ ] **Step 7: Write the ordering-under-out-of-order-completion test**

Add a configurable fake tool and a test to the `loop_.rs` tests module:

```rust
    /// Deterministic tool: sleeps `delay_ms`, then returns `body` as its content.
    struct FakeTool { name: String, delay_ms: u64, body: String }
    #[async_trait::async_trait]
    impl Tool for FakeTool {
        fn name(&self) -> &str { &self.name }
        fn description(&self) -> &str { "fake" }
        fn schema(&self) -> ToolSchema {
            ToolSchema { name: self.name.clone(), description: "fake".into(),
                parameters: serde_json::json!({"type":"object","properties":{}}) }
        }
        fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent { tool: self.name.clone(), access: Access::Read,
                paths: vec![], command: None, summary: "fake".into() })
        }
        async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx)
            -> Result<ToolOutput, ToolError> {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
            Ok(ToolOutput { content: self.body.clone(), display: None })
        }
    }

    fn tool_messages(ctx: &WindowContext) -> Vec<(String, String)> {
        ctx.build(usize::MAX).into_iter()
            .filter(|m| m.role == Role::Tool)
            .map(|m| (m.tool_call_id.unwrap_or_default(), m.content))
            .collect()
    }

    #[tokio::test]
    async fn tool_results_keep_model_call_order_despite_completion_order() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(FakeTool { name: "slow".into(), delay_ms: 150, body: "SLOW".into() }));
        r.register(Arc::new(FakeTool { name: "fast".into(), delay_ms: 5, body: "FAST".into() }));
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "slow".into(), "{}".into()),   // finishes LAST
                ("c2".into(), "fast".into(), "{}".into())]), // finishes FIRST
            Scripted::Text("done".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), Arc::new(r), Arc::new(AllowAll),
            Arc::new(AlwaysApprove), sink,
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 1, temperature: 0.0,
                max_tokens: None, workspace: std::env::temp_dir(),
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60), ..Default::default() });
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let msgs = tool_messages(&ctx);
        assert_eq!(msgs, vec![
            ("c1".into(), "SLOW".into()),
            ("c2".into(), "FAST".into())]);
    }
```

- [ ] **Step 8: Run the full agent-core suite**

Run: `source ~/.cargo/env && cargo test -p agent-core`
Expected: PASS — including `parallel_tool_calls_execute_concurrently`, `tool_results_keep_model_call_order_despite_completion_order`, and all pre-existing loop tests (`runs_tool_then_finishes`, `denied_tool_feeds_error_back_and_continues`, budget/retry/timeout).

- [ ] **Step 9: Lint**

Run: `source ~/.cargo/env && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: clean (no warnings; if `fmt --check` fails, run `cargo fmt` and re-stage).

- [ ] **Step 10: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs agent/crates/agent-cli/src/main.rs agent/crates/agent-server/src/runtime.rs
git commit -m "feat(agent-core): execute parallel tool calls concurrently

Two-phase: sequential gate/approval -> bounded buffer_unordered execute ->
ordered append. Adds max_parallel_tools (default 8). Preserves tool-result
semantics and approval serialization."
```

---

### Task 3: Lock multi-call correctness (regression tests)

**Files:**
- Test: `agent/crates/agent-core/src/loop_.rs` (tests module — reuses `FakeTool`, `AllowAll`, `tool_messages` from Task 2).

**Interfaces:**
- Consumes: `Scripted::Calls`, `FakeTool`, `AllowAll`, `tool_messages` (Tasks 1-2); a new `AskAll` policy + a counting approval channel defined here.

These characterize behavior that Task 2 must preserve; they should pass immediately against the new code.

- [ ] **Step 1: Multi-call happy path — N results, id-matched, in order**

```rust
    #[tokio::test]
    async fn multiple_tool_calls_produce_matched_results_in_order() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(FakeTool { name: "ta".into(), delay_ms: 0, body: "AAA".into() }));
        r.register(Arc::new(FakeTool { name: "tb".into(), delay_ms: 0, body: "BBB".into() }));
        r.register(Arc::new(FakeTool { name: "tc".into(), delay_ms: 0, body: "CCC".into() }));
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "ta".into(), "{}".into()),
                ("c2".into(), "tb".into(), "{}".into()),
                ("c3".into(), "tc".into(), "{}".into())]),
            Scripted::Text("done".into()),
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
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert_eq!(tool_messages(&ctx), vec![
            ("c1".into(), "AAA".into()),
            ("c2".into(), "BBB".into()),
            ("c3".into(), "CCC".into())]);
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(events.iter().filter(|e| e.starts_with("tool_result:")).count(), 3);
    }
```

- [ ] **Step 2: Per-call error isolation — middle call unknown, siblings unaffected**

```rust
    #[tokio::test]
    async fn one_failing_call_does_not_abort_the_others() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(FakeTool { name: "ta".into(), delay_ms: 0, body: "AAA".into() }));
        r.register(Arc::new(FakeTool { name: "tc".into(), delay_ms: 0, body: "CCC".into() }));
        // "tb" is intentionally NOT registered -> unknown-tool rejection for c2.
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "ta".into(), "{}".into()),
                ("c2".into(), "tb".into(), "{}".into()),
                ("c3".into(), "tc".into(), "{}".into())]),
            Scripted::Text("done".into()),
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
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let msgs = tool_messages(&ctx);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0], ("c1".into(), "AAA".into()));
        assert_eq!(msgs[1].0, "c2");
        assert!(msgs[1].1.starts_with("ERROR:"), "got {:?}", msgs[1].1);
        assert_eq!(msgs[2], ("c3".into(), "CCC".into()));
        // Only the two successes emit ToolResult; the loop still completes.
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(events.iter().filter(|e| e.starts_with("tool_result:")).count(), 2);
        assert_eq!(events.last().unwrap(), "done");
    }
```

- [ ] **Step 3: Approval serialization — never more than one prompt in flight**

```rust
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct AskAll;
    impl PolicyEngine for AskAll {
        fn check(&self, _i: &ToolIntent) -> Decision { Decision::Ask }
    }

    /// Approval channel that records the peak number of concurrent in-flight requests.
    struct CountingApproval { inflight: AtomicUsize, peak: AtomicUsize }
    #[async_trait::async_trait]
    impl ApprovalChannel for CountingApproval {
        async fn request(&self, _req: ApprovalRequest) -> ApprovalResponse {
            let n = self.inflight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(n, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await; // widen any overlap
            self.inflight.fetch_sub(1, Ordering::SeqCst);
            ApprovalResponse::Approve
        }
    }

    #[tokio::test]
    async fn approvals_are_serialized_across_parallel_calls() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(FakeTool { name: "ta".into(), delay_ms: 0, body: "AAA".into() }));
        r.register(Arc::new(FakeTool { name: "tb".into(), delay_ms: 0, body: "BBB".into() }));
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Calls(vec![
                ("c1".into(), "ta".into(), "{}".into()),
                ("c2".into(), "tb".into(), "{}".into())]),
            Scripted::Text("done".into()),
        ]));
        let approval = Arc::new(CountingApproval {
            inflight: AtomicUsize::new(0), peak: AtomicUsize::new(0) });
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), Arc::new(r), Arc::new(AskAll),
            approval.clone(), Arc::new(CollectingSink::default()),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 1, temperature: 0.0,
                max_tokens: None, workspace: std::env::temp_dir(),
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60), ..Default::default() });
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert_eq!(approval.peak.load(Ordering::SeqCst), 1,
            "approval prompts must never overlap");
    }
```

- [ ] **Step 4: Run the new tests**

Run: `source ~/.cargo/env && cargo test -p agent-core multiple_tool_calls_produce_matched_results_in_order one_failing_call_does_not_abort_the_others approvals_are_serialized_across_parallel_calls`
Expected: PASS (all three).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs
git commit -m "test(agent-core): lock multi-call correctness (order, error isolation, approval serialization)"
```

---

### Task 4: Opt-in live e2e for parallel tool calls

**Files:**
- Create: `agent/crates/agent-core/tests/e2e_parallel_tools.rs` (mirrors `tests/e2e_sglang.rs`).

**Interfaces:**
- Consumes: `AgentLoop`, `OpenAiCompatClient`, `NativeProtocol`, `RulePolicy`, `ReadFile` (production types). Env-gated via `AGENT_E2E_URL` / `AGENT_E2E_MODEL` (+ optional `AGENT_API_KEY`).

- [ ] **Step 1: Write the gated e2e test**

Create `agent/crates/agent-core/tests/e2e_parallel_tools.rs`:

```rust
//! Live, opt-in e2e: does the real server emit *parallel* tool calls in one turn,
//! and does the loop produce one correctly-id-matched result per call?
//! Run with: AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
//!           cargo test -p agent-core --test e2e_parallel_tools -- --ignored --nocapture

use agent_core::{AgentLoop, AgentEvent, EventSink, LoopConfig, WindowContext};
use agent_model::{Message, NativeProtocol, OpenAiCompatClient};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse, RulePolicy};
use agent_tools::{fs::ReadFile, ToolRegistry};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Records (tool_call name) per ToolResult so we can count matched results.
struct Capture(Mutex<Vec<String>>);
impl EventSink for Capture {
    fn emit(&self, e: AgentEvent) {
        if let AgentEvent::ToolResult { name, .. } = e {
            self.0.lock().unwrap().push(name);
        }
    }
}

struct AutoApprove;
#[async_trait::async_trait]
impl ApprovalChannel for AutoApprove {
    async fn request(&self, _r: ApprovalRequest) -> ApprovalResponse { ApprovalResponse::Approve }
}

#[tokio::test]
#[ignore = "requires AGENT_E2E_URL / AGENT_E2E_MODEL and a live server"]
async fn parallel_reads_against_real_server() {
    let url = std::env::var("AGENT_E2E_URL").expect("set AGENT_E2E_URL");
    let model_name = std::env::var("AGENT_E2E_MODEL").expect("set AGENT_E2E_MODEL");

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alpha.txt"), "ALPHA_BODY").unwrap();
    std::fs::write(dir.path().join("beta.txt"), "BETA_BODY").unwrap();
    let ws = dir.path().to_path_buf();

    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(ReadFile));
    let sink = Arc::new(Capture(Mutex::new(vec![])));
    let agent = AgentLoop::new(
        Arc::new(OpenAiCompatClient::new(url, model_name, std::env::var("AGENT_API_KEY").ok())),
        Arc::new(NativeProtocol), Arc::new(reg),
        Arc::new(RulePolicy { workspace: ws.clone(), command_allowlist: vec![],
            command_denylist: vec![] }),
        Arc::new(AutoApprove), sink.clone(),
        // temperature 0.0 to make parallel emission as deterministic as the model allows.
        LoopConfig { model_limit: 8192, max_turns: 4, max_retries: 2, temperature: 0.0,
            max_tokens: Some(512), workspace: ws, tool_timeout: Duration::from_secs(60),
            stream_idle_timeout: Duration::from_secs(120), ..Default::default() });

    let mut ctx = WindowContext::new(Message::system(
        "You are a coding agent. When asked about multiple files, call read_file \
         once per file IN THE SAME turn (parallel tool calls)."));
    agent.run(&mut ctx,
        "Read BOTH alpha.txt and beta.txt and report each file's contents. \
         Call read_file for each file in the same turn.".into()).await.unwrap();

    let reads = sink.0.lock().unwrap().clone();
    // Distinguish a loop bug from model behavior: <2 calls means the model did not
    // emit parallel calls this run — inconclusive, not a loop failure.
    assert!(reads.len() >= 2,
        "INCONCLUSIVE: model did not emit parallel tool calls (got {} read_file result(s)); \
         re-run or adjust the prompt. This is model behavior, not a loop bug.", reads.len());
    assert!(reads.iter().all(|n| n == "read_file"),
        "every result should be a read_file; got {reads:?}");
}
```

- [ ] **Step 2: Verify it compiles and is skipped by default**

Run: `source ~/.cargo/env && cargo test -p agent-core --test e2e_parallel_tools`
Expected: compiles; reports the test as `ignored` (0 run).

- [ ] **Step 3: (Optional, manual) Run against the live server**

Run: `source ~/.cargo/env && AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b cargo test -p agent-core --test e2e_parallel_tools -- --ignored --nocapture`
Expected: PASS (≥2 `read_file` results), or a clear `INCONCLUSIVE: …` panic if the model serialized this run. Requires the `llama-agent` server reachable on `localhost:8080` (host-network boundary — outside the sandbox).

- [ ] **Step 4: Commit**

```bash
git add agent/crates/agent-core/tests/e2e_parallel_tools.rs
git commit -m "test(agent-core): opt-in live e2e for parallel tool calls"
```

---

## Verification (whole feature)

- `source ~/.cargo/env && cargo test -p agent-core` — all unit/loop tests green (new + existing).
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` — clean.
- `cargo build -p agent-cli -p agent-server` — production `LoopConfig` sites compile with the new field.
- Optional manual e2e (Task 4 Step 3) against `llama-agent`.

## Self-review notes (coverage check)

- Spec decision 1 (both scope) → Tasks 2 (concurrency) + 3 (correctness tests).
- Decision 2-3 (two-phase, approval serialized) → Task 2 `gate_tool` sequential loop; Task 3 Step 3 proves it.
- Decision 4 (model call order) → Task 2 Phase 3; Task 2 Step 7 proves it.
- Decision 5 (`max_parallel_tools` default 8) → Task 2 Steps 3/6 + `DEFAULT_MAX_PARALLEL_TOOLS`.
- Decision 6 (mutation = model's responsibility) → no code; honored by concurrent execution.
- Spec tests 1-5 → Task 2 (concurrency, order) + Task 3 (happy path, error isolation, approval). Spec test 6 → Task 4.
- Type consistency: `GateOutcome` / `ReadyCall` / `Resolved` / `gate_tool` defined in Task 2 and used only there; `Scripted::Calls`, `FakeTool`, `AllowAll`, `tool_messages` defined once and reused by id across Tasks 2-3.
