# Runtime Knobs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Promote `max_parallel_tools` into `RuntimeConfig` (item 8) and add a graceful, tools-disabled wrap-up completion when `max_turns` exhausts (item 7).

**Architecture:** Task 1 is pure config plumbing following the existing `subagent_max_turns` serde-default/partial-merge pattern in `agent-runtime-config`. Task 2 replaces the hard `Done(BudgetExhausted)` fall-through in `agent-core/src/loop_.rs` with one best-effort completion driven through the existing `one_completion` (single attempt, no retry/overflow machinery).

**Tech Stack:** Rust (Cargo workspace under `agent/`), tokio, serde.

**Spec:** `docs/superpowers/specs/2026-07-02-runtime-knobs-design.md`

## Global Constraints

- Two Cargo workspaces: all `-p` targets here are in `agent/` (`cd agent` first; `source ~/.cargo/env` if cargo is missing).
- Old-SPA wire compat: additive frames/fields only. Neither task adds any wire event.
- Conventional commits: `type(scope): summary`.
- Line numbers cited are from 2026-07-02 main; re-locate by anchor text if drifted.

---

### Task 1: `max_parallel_tools` → RuntimeConfig

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs` (struct ~line 66-68, PartialRuntimeConfig ~line 148, default fns ~line 182, base constructor ~line 240, merge ~line 393, validate ~line 283, tests ~line 538)
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs:117` (+ test at ~504)

**Interfaces:**
- Consumes: `agent_core::DEFAULT_MAX_PARALLEL_TOOLS` (pub const = 8; already re-exported — the crate already uses `agent_core::DEFAULT_MAX_TOOL_RESULT_BYTES` the same way at line 183).
- Produces: `RuntimeConfig.max_parallel_tools: usize` (serde-defaulted), passed into `LoopConfig.max_parallel_tools` by `loop_config_from`.

- [ ] **Step 1: Write the failing tests**

In `runtime_config.rs` tests, next to `max_tool_result_bytes_defaults_and_merges` (~line 538), mirroring its style exactly:

```rust
#[test]
fn max_parallel_tools_defaults_and_merges() {
    // A JSON blob missing the field parses to the default (old files).
    let mut v = serde_json::to_value(RuntimeConfig::default_for_tests()).unwrap();
    v.as_object_mut().unwrap().remove("max_parallel_tools");
    let parsed: RuntimeConfig = serde_json::from_value(v).unwrap();
    assert_eq!(parsed.max_parallel_tools, 8, "serde default is DEFAULT_MAX_PARALLEL_TOOLS");

    // An explicit value round-trips.
    let mut c = RuntimeConfig::default_for_tests();
    c.max_parallel_tools = 3;
    let round: RuntimeConfig =
        serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
    assert_eq!(round.max_parallel_tools, 3);
}

#[test]
fn max_parallel_tools_partial_file_overrides_only_that_field() {
    let base = RuntimeConfig::default_for_tests();
    let merged = base.merge(
        serde_json::from_str::<PartialRuntimeConfig>(r#"{"max_parallel_tools": 2}"#).unwrap(),
    );
    assert_eq!(merged.max_parallel_tools, 2);
}

#[test]
fn validate_rejects_zero_max_parallel_tools() {
    let mut c = RuntimeConfig::default_for_tests();
    c.max_parallel_tools = 0;
    assert!(c.validate().unwrap_err().contains("max_parallel_tools"));
    c.max_parallel_tools = 1;
    assert!(c.validate().is_ok());
}
```

NOTE: the existing tests use some local helper to build a valid base config (see how `max_tool_result_bytes_defaults_and_merges` at ~538 constructs its config — reuse that exact helper/pattern instead of `default_for_tests` if it's named differently; also note `PartialRuntimeConfig` is private to the module, matching the existing partial-merge test's access).

- [ ] **Step 2: Run to verify failure**

Run: `cd agent && cargo test -p agent-runtime-config max_parallel_tools`
Expected: compile error (no such field) — that counts as the failing state.

- [ ] **Step 3: Implement**

In `runtime_config.rs`:

```rust
// struct RuntimeConfig, next to max_tool_result_bytes (~line 68):
    /// Max tool calls executed concurrently within one turn.
    #[serde(default = "default_max_parallel_tools")]
    pub max_parallel_tools: usize,

// struct PartialRuntimeConfig (~line 148):
    max_parallel_tools: Option<usize>,

// default fns (~line 185):
fn default_max_parallel_tools() -> usize {
    agent_core::DEFAULT_MAX_PARALLEL_TOOLS
}

// the flag-derived base constructor that lists every field (~line 240):
    max_parallel_tools: default_max_parallel_tools(),

// merge() (~line 393), next to the max_tool_result_bytes arm:
    if let Some(v) = p.max_parallel_tools {
        self.max_parallel_tools = v;
    }

// validate() (~line 313), next to the max_turns check:
    if self.max_parallel_tools == 0 {
        return Err("max_parallel_tools must be >= 1".into());
    }
```

In `assemble.rs:117`, replace the literal:

```rust
        max_parallel_tools: cfg.max_parallel_tools,
```

Keep the existing assertion at assemble.rs:504 (`assert_eq!(lc.max_parallel_tools, 8)` — still true via the default) and extend that test (or add a sibling) to pin passthrough of a non-default value:

```rust
    let mut cfg2 = cfg.clone();
    cfg2.max_parallel_tools = 2;
    assert_eq!(loop_config_from(&cfg2, /* same args as the existing call */).max_parallel_tools, 2);
```

- [ ] **Step 4: Run tests**

Run: `cd agent && cargo test -p agent-runtime-config`
Expected: PASS (all — including untouched merge/serde suites).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/runtime_config.rs agent/crates/agent-runtime-config/src/assemble.rs
git commit -m "feat(config): promote max_parallel_tools into RuntimeConfig"
```

---

### Task 2: Graceful max_turns landing

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` — the fall-through at ~879-881 (`self.sink.emit(AgentEvent::Done(StopReason::BudgetExhausted)); Ok(())` after the `for turn` loop), a new const near `STUCK_NUDGE_AFTER` (~line 30), tests in the same file's `#[cfg(test)]` module
- Check (likely adjust): `agent/crates/agent-core/tests/dispatch_tool.rs:319` (child BudgetExhausted footer test — the child model script now gets one extra wrap-up pull)

**Interfaces:**
- Consumes: `one_completion(req, cancel, &mut emitted) -> Result<AssistantTurn, ModelError>` (private helper, ~line 247); `completion_request(messages, preserve_thinking)` (~line 388); `Message::assistant(text, None)` (as used at line 569); `effective_model_limit()`.
- Produces: no new public API, no new events. Behavior: one tools-disabled completion before `Done(BudgetExhausted)`.

- [ ] **Step 1: Write the failing tests**

In the loop_.rs test module, using the established harness (`ScriptedModel`, `CollectingSink`, `DetailSink`, `PassthroughProtocol`, `WindowContext`, `AlwaysApprove`, `policy(ws)`, `registry()` — see `server_usage_event_carries_token_totals` at ~1305 for the construction shape). A small wrapper model records each request's tool-schema count:

```rust
    struct ToolCountingModel {
        inner: ScriptedModel,
        tool_counts: std::sync::Mutex<Vec<usize>>,
    }
    #[async_trait::async_trait]
    impl agent_model::ModelClient for ToolCountingModel {
        async fn stream(
            &self,
            req: agent_model::CompletionRequest,
        ) -> Result<
            futures::stream::BoxStream<'static, Result<agent_model::Chunk, agent_model::ModelError>>,
            agent_model::ModelError,
        > {
            self.tool_counts.lock().unwrap().push(req.tools.len());
            self.inner.stream(req).await
        }
    }
```

Four tests (all `max_turns: 1` so one tool-calling turn exhausts the budget; `read_file` on a real temp file is the established scripted call — see the stuck tests for args shape):

```rust
    /// Budget exhaustion triggers ONE tools-disabled wrap-up completion; its text
    /// streams and lands as a text-only assistant append; run still ends
    /// Done(BudgetExhausted). (spec: runtime-knobs Part 2)
    #[tokio::test]
    async fn budget_exhaustion_runs_tools_disabled_wrap_up() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        std::fs::write(ws.join("f.txt"), "x").unwrap();
        let model = Arc::new(ToolCountingModel {
            inner: ScriptedModel::new(vec![
                Scripted::Call("c1".into(), "read_file".into(),
                    format!(r#"{{"path":"{}"}}"#, ws.join("f.txt").display())),
                Scripted::Text("wrap-up summary".into()),
            ]),
            tool_counts: Default::default(),
        });
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(model.clone(), Arc::new(PassthroughProtocol), registry(),
            policy(ws.clone()), Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 1, max_retries: 2, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60), ..Default::default() });
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        let counts = model.tool_counts.lock().unwrap().clone();
        assert_eq!(counts.len(), 2, "turn + wrap-up = exactly two model calls");
        assert!(counts[0] > 0, "the real turn carries tool schemas");
        assert_eq!(counts[1], 0, "the wrap-up is tools-disabled");
        let events = sink.events.lock().unwrap().clone();
        assert!(events.iter().any(|e| e == "token:wrap-up summary"),
            "wrap-up streamed: {events:?}");
    }

    /// A wrap-up failure is swallowed: no append, still Done(BudgetExhausted).
    #[tokio::test]
    async fn budget_wrap_up_failure_is_best_effort() { /* same harness;
        script: [Call(read_file), Fail(ModelError::Http("boom".into()))];
        DetailSink; assert done == Some(StopReason::BudgetExhausted) and
        run() returned Ok(()) */ }

    /// Cancel during the wrap-up ends Done(Cancelled), matching loop-entry behavior.
    #[tokio::test]
    async fn budget_wrap_up_cancel_yields_cancelled() { /* script:
        [Call(read_file), Hang]; DetailSink; spawn a task that cancels the token
        after tokio::time::sleep(50ms) (real time — Hang pends forever, cancel
        races it); run_with_cancel; assert done == Some(StopReason::Cancelled) */ }

    /// Stray tool calls in the wrap-up reply are discarded — text-only append,
    /// no dangling ids, still Done(BudgetExhausted).
    #[tokio::test]
    async fn budget_wrap_up_discards_stray_tool_calls() { /* script:
        [Call(read_file), Call(read_file)]; DetailSink; assert
        done == Some(StopReason::BudgetExhausted) and tool_starts.len() == 1
        (only the real turn's call executed — the wrap-up call was NOT run) */ }
```

Write the two `/* ... */` sketches as real code following the first test's harness verbatim.

- [ ] **Step 2: Run to verify failure**

Run: `cd agent && cargo test -p agent-core budget_`
Expected: FAIL — today `counts.len() == 1` (no wrap-up call happens) and the cancel test sees `Done(BudgetExhausted)`.

- [ ] **Step 3: Implement**

Const near the stuck consts (~line 30):

```rust
/// Appended when `max_turns` exhausts with the model still issuing tool calls;
/// the run then gets ONE tools-disabled wrap-up completion (best-effort) so it
/// ends on a summary instead of mid-thought (spec: runtime-knobs Part 2).
const BUDGET_WRAP_UP_PROMPT: &str = "The turn limit for this run has been reached and \
tools are now disabled. Reply with a brief summary of what you accomplished, what \
remains to be done, and any state or next steps the user needs.";
```

Replace the fall-through at ~879-881:

```rust
        // Budget exhausted with the model still tool-hungry (text-only replies
        // exit earlier with Done(Stop)). One best-effort, tools-disabled wrap-up
        // completion; it must never fail the run. Single attempt by design: no
        // retry, no overflow recovery, no StreamRetry accounting (spec Part 2).
        if !cancel.is_cancelled() {
            ctx.append(Message::user(BUDGET_WRAP_UP_PROMPT));
            let messages = ctx.build(self.effective_model_limit());
            let mut req = self.completion_request(messages, preserve_thinking);
            req.tools = Vec::new();
            let started = std::time::Instant::now();
            let mut emitted = (0usize, 0usize);
            match self.one_completion(req, &cancel, &mut emitted).await {
                Ok(wrap) => {
                    if wrap.prompt_tokens > 0 || wrap.completion_tokens > 0 {
                        self.sink.emit(AgentEvent::ServerUsage {
                            prompt_tokens: wrap.prompt_tokens,
                            completion_tokens: wrap.completion_tokens,
                            reasoning_tokens: wrap.reasoning_tokens,
                            cached_tokens: wrap.cached_tokens,
                            cost_usd: wrap.cost_usd,
                            turn_duration_ms: started.elapsed().as_millis() as u64,
                            turn: self.config.max_turns,
                            parent_id: None,
                        });
                    }
                    // Text-only append: stray tool calls are discarded so no
                    // dangling tool_call ids enter persistent history. An empty
                    // reply appends nothing.
                    if !wrap.text.is_empty() {
                        ctx.append(Message::assistant(wrap.text, None));
                    }
                }
                Err(ModelError::Cancelled) => {
                    self.sink.emit(AgentEvent::Done(StopReason::Cancelled));
                    return Ok(());
                }
                Err(e) => {
                    tracing::debug!(error = %e, "budget wrap-up completion skipped");
                }
            }
        }
        self.sink
            .emit(AgentEvent::Done(StopReason::BudgetExhausted));
        Ok(())
```

Notes for the implementer:
- `ModelError` is already imported in loop_.rs (used by `one_completion`).
- No estimate `Usage` event for the wrap-up (turn indices must stay <= max_turns); `ServerUsage` with `turn: self.config.max_turns` is gated on nonzero usage so backends that report nothing emit nothing.
- Do NOT touch the stuck-abort path or any other `Done(...)` site.

- [ ] **Step 4: Run the crate suite and repair fallout**

Run: `cd agent && cargo test -p agent-core`
Expected: the four new tests PASS. Known fallout candidates (fix, don't skip):
- `tests/dispatch_tool.rs:319` (`stop: BudgetExhausted` footer): the CHILD loop now pulls one extra scripted turn for its wrap-up. `ScriptedModel` yields `Text("")` when exhausted (testkit.rs:73), which streams an empty token and appends nothing — if the capture's final-text tail changes, add an explicit `Scripted::Text("...")` wrap-up turn to that test's script and assert the richer content.
- Any test scripting exact model-call counts to reach `BudgetExhausted`: account for the extra wrap-up pull the same way.

- [ ] **Step 5: Run the dependent-crate smoke**

Run: `cd agent && cargo test -p agent-runtime-config && cargo test -p agent-server`
Expected: PASS (no API changes, but both consume the loop).

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs agent/crates/agent-core/tests/dispatch_tool.rs
git commit -m "feat(core): graceful tools-disabled wrap-up completion on max_turns exhaustion"
```

---

### Task 3: CI gate

- [ ] **Step 1: Run the full gate**

Run: `bash scripts/ci.sh` (from repo root; stdin-closed no longer required — hermetic since 39932bf)
Expected: fmt + clippy + both workspaces' tests + web typecheck/vitest all green.

- [ ] **Step 2: Fix anything red, amend or follow-up commit as appropriate**
