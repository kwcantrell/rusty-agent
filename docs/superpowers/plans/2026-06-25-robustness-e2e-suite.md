# Robustness E2E Suite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add four deterministic, CI-runnable e2e tests that validate this session's merged fixes (C/B1/B2/B3) through the real `assemble_loop`, driven by a scripted model (and wiremock for B2).

**Architecture:** One new file `agent/crates/agent-runtime-config/tests/e2e_robustness.rs`. Each test builds the real loop via `assemble_loop` + `LoopParts { model: Arc<dyn ModelClient>, … }`, using `agent_core::testkit::ScriptedModel` (or the real `OpenAiCompatClient` against wiremock for B2). A shared `Capture` `EventSink` records structural events; assertions run against it and the `WindowContext` transcript. Not `#[ignore]`'d.

**Note on TDD:** the fixes under test are already merged, so each test **passes on the first run** (green). The spec records the revert-failure rationale per test — we do not revert merged code to force red.

**Tech Stack:** Rust, `tokio`, `agent_core::testkit`, `wiremock`, `tokio-util`.

## Global Constraints

- Run tests from the workspace root: `cd agent` first. `source ~/.cargo/env` if needed.
- Deterministic only — no live model server, no network beyond loopback wiremock.
- `cfg.memory = false`, `cfg.sandbox_mode = "off"`, backend `"openai"`, protocol `"native"` for every test.
- `loop_config_from` hardcodes `max_retries = 3` (not from cfg) — T4 accepts 4 model attempts (~600 ms of backoff sleeps); do not try to set retries via cfg.
- `BuiltLoop.registered_names` is `#[cfg(test)]` on the lib → **not** visible to this external test crate; do not reference it.

## Reference — confirmed APIs

- `agent_runtime_config::{assemble_loop, LoopParts, RuntimeConfig, BuiltLoop}`.
  `RuntimeConfig::from_launch(backend: String, base_url: String, model: String, protocol: String, context_limit: usize) -> Self` (memory defaults true → set false).
- `LoopParts { model: Arc<dyn ModelClient>, sink: Arc<dyn EventSink>, approval: Arc<dyn ApprovalChannel>, workspace: PathBuf, mcp_tools: Vec<Arc<dyn Tool>>, memory_tools: Vec<…>, memory_retriever: Option<…>, stream_idle_timeout: Duration, base_system_prompt: String }`.
- `BuiltLoop { loop_: Arc<AgentLoop>, system_prompt: String, … }`; `AgentLoop::run(&mut ctx, String)` and `run_with_cancel(&mut ctx, String, CancellationToken)`.
- `agent_core::{AgentEvent, EventSink, WindowContext, AgentError}`; `AgentEvent::{Token(String), ToolStart{name,..}, ToolResult{name,..}, Approval(ApprovalRequest), Error(String), Done(StopReason)}`.
- `agent_core::testkit::{ScriptedModel, Scripted, AlwaysApprove}`; `Scripted::{Call(id,name,args), Calls(Vec<(id,name,args)>), Text(String)}`.
- `agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse}`; `ApprovalRequest { intent: ToolIntent, .. }`; `ToolIntent { tool: String, paths: Vec<PathBuf>, .. }`.
- `agent_model::{Message, ModelClient, OpenAiCompatClient, Role, StopReason}`; `OpenAiCompatClient::new(base_url, model, api_key: Option<String>)`.

---

### Task 1: Scaffold the suite + dev-deps + T1 (B1 duplicate ids)

**Files:**
- Modify: `agent/crates/agent-runtime-config/Cargo.toml` (`[dev-dependencies]`)
- Create: `agent/crates/agent-runtime-config/tests/e2e_robustness.rs`

- [ ] **Step 1: Add dev-dependencies**

In `agent/crates/agent-runtime-config/Cargo.toml`, under `[dev-dependencies]` (currently `tempfile`, `tokio`, `async-trait`), add:

```toml
wiremock.workspace = true
tokio-util.workspace = true
```

- [ ] **Step 2: Create the file with the shared harness + T1**

Create `agent/crates/agent-runtime-config/tests/e2e_robustness.rs`:

```rust
//! Deterministic, CI-runnable e2e for the merged robustness/security fixes,
//! driven through the real `assemble_loop` with a scripted model (and wiremock
//! for the stream-layer fix). Complements — does not replace — the per-crate
//! unit tests. See docs/superpowers/specs/2026-06-25-robustness-e2e-suite-design.md.
//!
//! Validated here: B1 (tool-call id collision), B3 (cancellation), C (read-path
//! approval gate), B2 (truncated stream surfaced). NOT here (unit-tested, with
//! reasons): C redirect (strict SSRF blocks a local wiremock target), B2
//! skip-malformed / in-band-error (pure client-layer SSE parsing).

use agent_core::testkit::{AlwaysApprove, Scripted, ScriptedModel};
use agent_core::{AgentError, AgentEvent, EventSink, WindowContext};
use agent_model::{Message, ModelClient, OpenAiCompatClient, Role};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use agent_runtime_config::{assemble_loop, LoopParts, RuntimeConfig};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Records structural events (Token text is accumulated separately so `events`
/// stays a clean list of structural markers for exact-match assertions).
#[derive(Default)]
struct Capture {
    events: Mutex<Vec<String>>,
    text: Mutex<String>,
}
impl EventSink for Capture {
    fn emit(&self, e: AgentEvent) {
        match e {
            AgentEvent::Token(t) => self.text.lock().unwrap().push_str(&t),
            AgentEvent::ToolStart { name, .. } =>
                self.events.lock().unwrap().push(format!("tool_start:{name}")),
            AgentEvent::ToolResult { name, .. } =>
                self.events.lock().unwrap().push(format!("tool_result:{name}")),
            AgentEvent::Approval(req) =>
                self.events.lock().unwrap().push(format!("approval:{}:{:?}", req.intent.tool, req.intent.paths)),
            AgentEvent::Error(m) =>
                self.events.lock().unwrap().push(format!("error:{m}")),
            AgentEvent::Done(r) =>
                self.events.lock().unwrap().push(format!("done:{r:?}")),
            _ => {}
        }
    }
}

/// Build the real loop with `memory=false`, `sandbox=off`, native/openai. The
/// model's URL is irrelevant for a ScriptedModel; the OpenAiCompatClient carries
/// its own URL. cfg.base_url is unused by the loop (the model is injected).
fn assemble_test(
    workspace: PathBuf,
    model: Arc<dyn ModelClient>,
    approval: Arc<dyn ApprovalChannel>,
) -> (agent_runtime_config::BuiltLoop, Arc<Capture>) {
    let mut cfg = RuntimeConfig::from_launch(
        "openai".into(), "http://unused".into(), "test-model".into(), "native".into(), 262_144);
    cfg.memory = false;
    cfg.sandbox_mode = "off".into();
    let sink = Arc::new(Capture::default());
    let built = assemble_loop(&cfg, LoopParts {
        model,
        sink: sink.clone(),
        approval,
        workspace,
        mcp_tools: vec![],
        memory_tools: vec![],
        memory_retriever: None,
        stream_idle_timeout: Duration::from_secs(10),
        base_system_prompt: "You are a test agent.".into(),
    });
    (built, sink)
}

// T1 — B1: two tool calls sharing an id don't crash, and produce two distinct
// tool messages in the transcript. (Would panic on pre-B1 code.)
#[tokio::test]
async fn b1_duplicate_tool_call_ids_through_assembled_loop() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().to_path_buf();
    std::fs::write(ws.join("a.txt"), "BODY").unwrap();
    let model = Arc::new(ScriptedModel::new(vec![
        Scripted::Calls(vec![
            ("c1".into(), "read_file".into(), r#"{"path":"a.txt"}"#.into()),
            ("c1".into(), "read_file".into(), r#"{"path":"a.txt"}"#.into()),
        ]),
        Scripted::Text("done".into()),
    ]));
    let (built, sink) = assemble_test(ws, model, Arc::new(AlwaysApprove));
    let mut ctx = WindowContext::new(Message::system(built.system_prompt.clone()));

    built.loop_.run(&mut ctx, "read twice".into()).await.unwrap(); // must not panic

    let events = sink.events.lock().unwrap().clone();
    assert_eq!(events.iter().filter(|e| *e == "tool_result:read_file").count(), 2,
        "both reads should produce a result; events: {events:?}");
    let transcript = ctx.build(262_144);
    let tool_ids: Vec<String> = transcript.iter()
        .filter(|m| matches!(m.role, Role::Tool))
        .map(|m| m.tool_call_id.clone().unwrap_or_default())
        .collect();
    assert_eq!(tool_ids.len(), 2, "two tool messages expected: {tool_ids:?}");
    assert_ne!(tool_ids[0], tool_ids[1], "duplicate ids must normalize to distinct");
}
```

- [ ] **Step 3: Run T1**

Run: `cd agent && cargo test -p agent-runtime-config --test e2e_robustness b1_duplicate`
Expected: PASS (1 test). Validates B1 through the assembled loop + real `read_file`.

- [ ] **Step 4: Commit**

```bash
cd agent && git add crates/agent-runtime-config/Cargo.toml crates/agent-runtime-config/tests/e2e_robustness.rs
git commit -m "test(e2e): assembled-loop B1 duplicate-tool-call-id validation"
```

---

### Task 2: T2 (B3 cancellation through the assembled loop)

**Files:**
- Modify: `agent/crates/agent-runtime-config/tests/e2e_robustness.rs`

- [ ] **Step 1: Add T2**

Append to `e2e_robustness.rs`:

```rust
// T2 — B3: a pre-cancelled token stops the assembled loop cleanly, before the
// model is ever consulted. (run_with_cancel is the B3 entry point.)
#[tokio::test]
async fn b3_precancelled_token_stops_assembled_loop() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().to_path_buf();
    let model = Arc::new(ScriptedModel::new(vec![Scripted::Text("should not run".into())]));
    let (built, sink) = assemble_test(ws, model, Arc::new(AlwaysApprove));
    let mut ctx = WindowContext::new(Message::system(built.system_prompt.clone()));

    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel(); // cancelled before the run starts

    built.loop_.run_with_cancel(&mut ctx, "go".into(), cancel).await.unwrap();

    // Only the terminal Done(Cancelled); no Usage/Token (model never consulted).
    let events = sink.events.lock().unwrap().clone();
    assert_eq!(events, vec!["done:Cancelled".to_string()], "events: {events:?}");
    assert!(sink.text.lock().unwrap().is_empty(), "no model text should stream");
}
```

- [ ] **Step 2: Run T2**

Run: `cd agent && cargo test -p agent-runtime-config --test e2e_robustness b3_precancelled`
Expected: PASS. `Done(StopReason::Cancelled)` formats as `"done:Cancelled"`.

- [ ] **Step 3: Commit**

```bash
cd agent && git add crates/agent-runtime-config/tests/e2e_robustness.rs
git commit -m "test(e2e): assembled-loop B3 cancellation validation"
```

---

### Task 3: T3 (C read-path approval gate through the assembled loop)

**Files:**
- Modify: `agent/crates/agent-runtime-config/tests/e2e_robustness.rs`

- [ ] **Step 1: Add a denying approval channel + T3**

Append to `e2e_robustness.rs`:

```rust
struct DenyAll;
#[async_trait::async_trait]
impl ApprovalChannel for DenyAll {
    async fn request(&self, _r: ApprovalRequest) -> ApprovalResponse { ApprovalResponse::Deny }
}

// T3 — C: a `..`-escaping read now routes through approval (the normalized engine
// returns Ask instead of silently Allow). The sink witnesses the Approval event;
// the channel denies, so the read is rejected and fed back as an error result.
// (On pre-C code the gate returned Allow → no approval event.)
#[tokio::test]
async fn c_escaping_read_requests_approval_through_assembled_loop() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().to_path_buf();
    let model = Arc::new(ScriptedModel::new(vec![
        Scripted::Call("c1".into(), "read_file".into(), r#"{"path":"../../escape.txt"}"#.into()),
        Scripted::Text("ok".into()),
    ]));
    let (built, sink) = assemble_test(ws, model, Arc::new(DenyAll));
    let mut ctx = WindowContext::new(Message::system(built.system_prompt.clone()));

    built.loop_.run(&mut ctx, "read escaping".into()).await.unwrap();

    let events = sink.events.lock().unwrap().clone();
    assert!(events.iter().any(|e| e.starts_with("approval:read_file")),
        "escaping read must request approval (normalized gate); events: {events:?}");
}
```

- [ ] **Step 2: Run T3**

Run: `cd agent && cargo test -p agent-runtime-config --test e2e_robustness c_escaping_read`
Expected: PASS. The real `RulePolicy` (workspace = the tempdir) returns `Ask` for the escaping path, so the loop emits `AgentEvent::Approval` before the `DenyAll` channel rejects it.

- [ ] **Step 3: Commit**

```bash
cd agent && git add crates/agent-runtime-config/tests/e2e_robustness.rs
git commit -m "test(e2e): assembled-loop C read-path approval-gate validation"
```

---

### Task 4: T4 (B2 truncated stream surfaced through the assembled loop)

**Files:**
- Modify: `agent/crates/agent-runtime-config/tests/e2e_robustness.rs`

- [ ] **Step 1: Add T4 (real client + wiremock)**

Append to `e2e_robustness.rs`. Add the wiremock imports at the top of the file (next to the existing `use` block):

```rust
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
```

Then the test:

```rust
// T4 — B2: a truncated model stream (no finish_reason / [DONE]) is detected,
// retried, and finally surfaced as an Error through the assembled loop — not
// silently accepted as a clean completion. (On pre-B2 code: clean Ok, no Error.)
#[tokio::test]
async fn b2_truncated_stream_surfaces_error_through_assembled_loop() {
    let server = MockServer::start().await;
    // A content delta, then the body just ends: no finish_reason, no [DONE].
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(body))
        .mount(&server).await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().to_path_buf();
    let model = Arc::new(OpenAiCompatClient::new(server.uri(), "test-model".into(), None));
    let (built, sink) = assemble_test(ws, model, Arc::new(AlwaysApprove));
    let mut ctx = WindowContext::new(Message::system(built.system_prompt.clone()));

    // max_retries is 3 (hardcoded in loop_config_from): 4 attempts, all truncated,
    // then the loop gives up and propagates the model error.
    let err = built.loop_.run(&mut ctx, "go".into()).await.unwrap_err();
    assert!(matches!(err, AgentError::Model(_)), "expected Model error, got {err:?}");
    let events = sink.events.lock().unwrap().clone();
    assert!(events.iter().any(|e| e.starts_with("error:")),
        "the truncation must surface as an Error event; events: {events:?}");
}
```

- [ ] **Step 2: Run T4**

Run: `cd agent && cargo test -p agent-runtime-config --test e2e_robustness b2_truncated`
Expected: PASS (takes ~1 s for the retry backoff). On pre-B2 code this would return `Ok` with no `error:` event.

- [ ] **Step 3: Commit**

```bash
cd agent && git add crates/agent-runtime-config/tests/e2e_robustness.rs
git commit -m "test(e2e): assembled-loop B2 truncated-stream validation"
```

---

### Task 5: Final verification

**Files:** none (verification only)

- [ ] **Step 1: Run the whole new suite**

Run: `cd agent && cargo test -p agent-runtime-config --test e2e_robustness`
Expected: 4 passed, 0 failed, 0 ignored — confirming the suite is **not** `#[ignore]`'d and runs in normal CI.

- [ ] **Step 2: Full crate test (no regressions, existing ignored e2e still skipped)**

Run: `cd agent && cargo test -p agent-runtime-config 2>&1 | grep "test result"`
Expected: the unit tests pass, `e2e_robustness` 4 pass, and the live `e2e_auto_retrieval` tests remain `ignored`.

- [ ] **Step 3: Confirm spec coverage**

Cross-check against `docs/superpowers/specs/2026-06-25-robustness-e2e-suite-design.md`: T1–T4 present and passing, and the suite header documents the unit-only exclusions (redirect, B2 skip/in-band). If anything is missing, add it.
```
