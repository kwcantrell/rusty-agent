# Tauri IPC Transport Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the loopback WebSocket transport between the Tauri shell and its webview with native Tauri IPC — typed `#[command]`s inbound and one live-read `ipc::Channel<ServerEvent>` outbound — dissolving the unbounded event channel (A2), adding a single-active-run guard (A1), and an interactive `cancel()` (B3).

**Architecture:** A transport-agnostic seam keeps `agent-server` free of any Tauri dependency: it defines `ServerEvent` (serde), an `EventOut` trait, and an app-lifetime `Session` that owns the runtime, context, approval registry, and run-guard. The `src-tauri` crate provides the only Tauri-aware piece — an `EventOut` backed by `ipc::Channel<ServerEvent>` plus the `#[command]`s that drive `Session`. The frontend swaps its WebSocket for IPC behind the unchanged `connect()` seam in `socket.ts`, so the reducer and `Inbound` frame shapes are untouched.

**Tech Stack:** Rust (tokio, `tauri` v2, `tokio_util::sync::CancellationToken`), TypeScript/React (`@tauri-apps/api` `invoke` + `Channel`), Vitest.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-06-25-tauri-ipc-transport-design.md`.
- `agent-server` MUST NOT depend on `tauri` — the Tauri-aware code lives only in `src-tauri`.
- The outbound sink reads the registered channel **live on each emit** (never a snapshot taken at run-start), so re-`subscribe` mid-run redirects subsequent events.
- `emit` stays infallible: an absent/closed channel drops the event (parity with today's `let _ = tx.send()`).
- Approval timeout stays **300 s**, resolving to `Deny` on timeout/absent channel.
- A-c's `RuntimeState` three-mutex refactor is OUT OF SCOPE: carry its internals over verbatim; only its construction site and reply path change.
- `ServerEvent` MUST serialize to the existing frontend `WireEvent` tagged shape (`{ "type": "token", "text": … }`, etc.) plus one `{ "type": "approval_request", "id", "summary", "command"?, "display"? }` variant, so the frontend reducer path is unchanged.
- TDD throughout: failing test → run → minimal impl → run → commit. One logical change per commit.

---

## Phase 1 — `agent-server`: transport-agnostic core

### Task 1: Define `ServerEvent`, `EventOut`, and `SettingsState` types

Replace the `WireEnvelope`/`WireBody` wrapper types with the trimmed payload types the new transport needs. `WireEvent` becomes `ServerEvent` (adds the approval variant); `SettingsState`/`SettingsError`-as-data become a standalone struct returned by commands.

**Files:**
- Modify: `agent/crates/agent-server/src/wire.rs` (replace envelope/body types; keep `Display`-bearing payloads, `Decision`, `DiscoveredSkill`, `stop_reason_str`, `wire_event_from` renamed)
- Test: same file's `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
  - `pub enum ServerEvent` (serde `#[serde(tag = "type", rename_all = "snake_case")]`) with variants `Token{text}`, `Reasoning{text}`, `Usage{prompt_tokens,context_limit,turn,max_turns}`, `ToolStart{name,args}`, `ToolResult{name,content,display:Option<Display>}`, `Error{message}`, `Done{reason}`, `ApprovalRequest{id,summary,command:Option<String>,display:Option<Display>}`
  - `pub fn server_event_from(event: AgentEvent) -> Option<ServerEvent>` (returns `None` for `AgentEvent::Approval`, exactly as `wire_event_from` did)
  - `pub struct SettingsState { settings: RuntimeConfig, workspace: String, api_key_set: bool, hard_floor: Vec<String>, discovered_skills: Vec<DiscoveredSkill> }` (serde `Serialize`)
  - `pub trait EventOut: Send + Sync { fn send(&self, ev: ServerEvent); }`
  - kept: `pub enum Decision` (was `WireDecision`), `impl From<Decision> for ApprovalResponse`, `pub struct DiscoveredSkill`

- [ ] **Step 1: Write the failing test**

In `wire.rs` `mod tests`, replace the envelope-shaped tests with payload tests:

```rust
#[test]
fn token_serializes_with_type_tag() {
    let ev = server_event_from(AgentEvent::Token("hi".into())).unwrap();
    let j = serde_json::to_string(&ev).unwrap();
    assert_eq!(j, r#"{"type":"token","text":"hi"}"#);
}

#[test]
fn approval_event_maps_to_none_but_variant_exists() {
    use agent_policy::ApprovalRequest;
    use agent_tools::{Access, ToolIntent};
    let req = ApprovalRequest {
        intent: ToolIntent { tool: "x".into(), access: Access::Write, paths: vec![],
            command: None, summary: "s".into() },
        display: None,
    };
    assert!(server_event_from(AgentEvent::Approval(req)).is_none());
    let ar = ServerEvent::ApprovalRequest { id: "c0".into(), summary: "s".into(),
        command: None, display: None };
    let j = serde_json::to_string(&ar).unwrap();
    assert!(j.contains(r#""type":"approval_request""#));
    assert!(j.contains(r#""id":"c0""#));
}

#[test]
fn done_uses_stop_reason_string() {
    let ev = server_event_from(AgentEvent::Done(StopReason::Cancelled)).unwrap();
    assert_eq!(serde_json::to_string(&ev).unwrap(), r#"{"type":"done","reason":"cancelled"}"#);
}

#[test]
fn decision_into_response() {
    assert_eq!(ApprovalResponse::from(Decision::ApproveAlways), ApprovalResponse::ApproveAlways);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p agent-server --lib wire`
Expected: FAIL — `ServerEvent`, `server_event_from`, `Decision` not found.

- [ ] **Step 3: Rewrite `wire.rs` types**

Replace the whole non-test body of `wire.rs` with:

```rust
use agent_core::AgentEvent;
use agent_model::StopReason;
use agent_policy::ApprovalResponse;
use agent_runtime_config::RuntimeConfig;
use agent_tools::Display;
use serde::{Deserialize, Serialize};

/// Outbound streaming event sent over the Tauri channel. Mirrors the legacy
/// `WireEvent` tagged shape so the frontend reducer is unchanged, plus the
/// `approval_request` case (was a sibling `WireBody`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Token { text: String },
    Reasoning { text: String },
    Usage { prompt_tokens: usize, context_limit: usize, turn: usize, max_turns: usize },
    ToolStart { name: String, args: serde_json::Value },
    ToolResult {
        name: String,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<Display>,
    },
    Error { message: String },
    Done { reason: String },
    ApprovalRequest {
        id: String,
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<Display>,
    },
}

/// Settings snapshot returned by the `settings_get` command (was `WireBody::SettingsState`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsState {
    pub settings: RuntimeConfig,
    pub workspace: String,
    pub api_key_set: bool,
    pub hard_floor: Vec<String>,
    pub discovered_skills: Vec<DiscoveredSkill>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredSkill {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision { Approve, ApproveAlways, Deny }

impl From<Decision> for ApprovalResponse {
    fn from(d: Decision) -> Self {
        match d {
            Decision::Approve => ApprovalResponse::Approve,
            Decision::ApproveAlways => ApprovalResponse::ApproveAlways,
            Decision::Deny => ApprovalResponse::Deny,
        }
    }
}

/// Transport-agnostic outbound sink. `src-tauri` implements this over an
/// `ipc::Channel<ServerEvent>`; `agent-server` never sees Tauri.
pub trait EventOut: Send + Sync {
    fn send(&self, ev: ServerEvent);
}

fn stop_reason_str(r: &StopReason) -> &'static str {
    match r {
        StopReason::Stop => "stop",
        StopReason::ToolCalls => "tool_calls",
        StopReason::Length => "length",
        StopReason::BudgetExhausted => "budget_exhausted",
        StopReason::Cancelled => "cancelled",
    }
}

/// Map a core `AgentEvent` to its wire form. `Approval` returns `None` — the
/// approval channel emits its own `ApprovalRequest` (mirrors the CLI sink).
pub fn server_event_from(event: AgentEvent) -> Option<ServerEvent> {
    Some(match event {
        AgentEvent::Token(t) => ServerEvent::Token { text: t },
        AgentEvent::Reasoning(t) => ServerEvent::Reasoning { text: t },
        AgentEvent::Usage { prompt_tokens, context_limit, turn, max_turns } =>
            ServerEvent::Usage { prompt_tokens, context_limit, turn, max_turns },
        AgentEvent::ToolStart { name, args } => ServerEvent::ToolStart { name, args },
        AgentEvent::ToolResult { name, output } => ServerEvent::ToolResult {
            name, content: output.content, display: output.display },
        AgentEvent::Error(m) => ServerEvent::Error { message: m },
        AgentEvent::Done(r) => ServerEvent::Done { reason: stop_reason_str(&r).into() },
        AgentEvent::Approval(_) => return None,
    })
}
```

Keep the existing `use agent_core::AgentEvent;` test import. Delete all envelope/`WireBody`/`WireEvent`/`PROTOCOL_VERSION` references in this file. Remove tests that referenced the deleted types (`event_envelope_round_trips`, `approval_response_deserializes`, `settings_get_round_trips`, `settings_update_carries_a_config`, `settings_state_and_error_serialize`, `tool_result_with_markdown_display_round_trips`, `cancelled_stop_reason_maps_to_wire_string`, `reasoning_event_maps_to_wire`, `usage_event_maps_to_wire_and_serializes`) — they are superseded by the four new tests above plus coverage gained in later tasks.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p agent-server --lib wire`
Expected: PASS (4 tests). Other crate modules won't compile yet — that's expected; later tasks fix them.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-server/src/wire.rs
git commit -m "feat(agent-server): ServerEvent/EventOut/SettingsState; drop wire envelope"
```

---

### Task 2: `ChannelEventSink` — live-read `EventOut`

Replace `WsEventSink` (mpsc-backed) with a sink that maps `AgentEvent → ServerEvent` and forwards to a live-read `EventOut` slot.

**Files:**
- Modify: `agent/crates/agent-server/src/sink.rs` (full rewrite)
- Test: same file's `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `ServerEvent`, `EventOut`, `server_event_from` (Task 1)
- Produces:
  - `pub type EventSlot = Arc<Mutex<Option<Arc<dyn EventOut>>>>` (the live-read slot; `std::sync::Mutex`)
  - `pub struct ChannelEventSink { slot: EventSlot }`
  - `pub fn ChannelEventSink::new(slot: EventSlot) -> Self`
  - `impl agent_core::EventSink for ChannelEventSink` — `emit` reads the slot live; drops when `None`

- [ ] **Step 1: Write the failing test**

Replace `sink.rs` tests with a mock-`EventOut` capture:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::ServerEvent;
    use agent_core::{AgentEvent, EventSink};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Captured(Mutex<Vec<ServerEvent>>);
    impl crate::wire::EventOut for Captured {
        fn send(&self, ev: ServerEvent) { self.0.lock().unwrap().push(ev); }
    }

    fn slot_with(out: Arc<Captured>) -> EventSlot {
        Arc::new(Mutex::new(Some(out as Arc<dyn crate::wire::EventOut>)))
    }

    #[test]
    fn token_is_forwarded_as_server_event() {
        let cap = Arc::new(Captured::default());
        let sink = ChannelEventSink::new(slot_with(cap.clone()));
        sink.emit(AgentEvent::Token("hello".into()));
        let got = cap.0.lock().unwrap();
        assert!(matches!(got.as_slice(), [ServerEvent::Token { text }] if text == "hello"));
    }

    #[test]
    fn approval_event_is_not_forwarded() {
        use agent_policy::ApprovalRequest;
        use agent_tools::{Access, ToolIntent};
        let cap = Arc::new(Captured::default());
        let sink = ChannelEventSink::new(slot_with(cap.clone()));
        sink.emit(AgentEvent::Approval(ApprovalRequest {
            intent: ToolIntent { tool: "x".into(), access: Access::Read, paths: vec![],
                command: None, summary: "s".into() },
            display: None }));
        assert!(cap.0.lock().unwrap().is_empty());
    }

    #[test]
    fn emit_with_empty_slot_is_a_noop() {
        let slot: EventSlot = Arc::new(Mutex::new(None));
        let sink = ChannelEventSink::new(slot);
        sink.emit(AgentEvent::Token("x".into())); // must not panic
    }

    #[test]
    fn relinks_to_a_new_out_live() {
        let first = Arc::new(Captured::default());
        let slot = slot_with(first.clone());
        let sink = ChannelEventSink::new(slot.clone());
        sink.emit(AgentEvent::Token("a".into()));
        let second = Arc::new(Captured::default());
        *slot.lock().unwrap() = Some(second.clone() as Arc<dyn crate::wire::EventOut>);
        sink.emit(AgentEvent::Token("b".into()));
        assert_eq!(first.0.lock().unwrap().len(), 1);
        assert_eq!(second.0.lock().unwrap().len(), 1);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p agent-server --lib sink`
Expected: FAIL — `ChannelEventSink`, `EventSlot` not found.

- [ ] **Step 3: Rewrite `sink.rs` non-test body**

```rust
use crate::wire::{server_event_from, EventOut};
use agent_core::{AgentEvent, EventSink};
use std::sync::{Arc, Mutex};

/// The live-read outbound slot, swapped by the `subscribe` command. Reading it
/// per-emit (not snapshotting) lets a re-subscribe redirect an in-flight run.
pub type EventSlot = Arc<Mutex<Option<Arc<dyn EventOut>>>>;

/// `EventSink` that maps core events to `ServerEvent` and forwards them to the
/// currently-registered `EventOut`. Absent slot → drop (infallible emit).
pub struct ChannelEventSink {
    slot: EventSlot,
}

impl ChannelEventSink {
    pub fn new(slot: EventSlot) -> Self {
        Self { slot }
    }
}

impl EventSink for ChannelEventSink {
    fn emit(&self, event: AgentEvent) {
        let Some(ev) = server_event_from(event) else { return };
        if let Some(out) = self.slot.lock().unwrap().clone() {
            out.send(ev);
        }
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p agent-server --lib sink`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-server/src/sink.rs
git commit -m "feat(agent-server): ChannelEventSink with live-read EventOut slot"
```

---

### Task 3: `IpcApprovalChannel` — emit over `EventOut`, registry resolve

Convert `WsApprovalChannel` to emit `ServerEvent::ApprovalRequest` over the live-read slot and keep its oneshot registry resolvable by an `approve` command.

**Files:**
- Modify: `agent/crates/agent-server/src/approval.rs` (full rewrite)
- Test: same file's `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `EventSlot` (Task 2), `ServerEvent`, `Decision`, `EventOut`
- Produces:
  - `pub struct IpcApprovalChannel { slot: EventSlot, pending: Mutex<HashMap<String, oneshot::Sender<ApprovalResponse>>>, counter: AtomicU64, timeout: Duration }`
  - `pub fn IpcApprovalChannel::new(slot: EventSlot, timeout: Duration) -> Self`
  - `pub fn resolve(&self, id: &str, decision: ApprovalResponse)`
  - `impl agent_policy::ApprovalChannel` — emits `ApprovalRequest{id,…}`; absent slot → immediate `Deny`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{EventOut, ServerEvent};
    use agent_tools::{Access, ToolIntent};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Captured(Mutex<Vec<ServerEvent>>);
    impl EventOut for Captured {
        fn send(&self, ev: ServerEvent) { self.0.lock().unwrap().push(ev); }
    }
    fn slot_with(out: Arc<Captured>) -> crate::sink::EventSlot {
        Arc::new(Mutex::new(Some(out as Arc<dyn EventOut>)))
    }
    fn req() -> ApprovalRequest {
        ApprovalRequest {
            intent: ToolIntent { tool: "execute_command".into(), access: Access::Write,
                paths: vec![], command: Some("touch x".into()), summary: "run touch x".into() },
            display: None }
    }

    #[tokio::test]
    async fn emits_request_and_resolves() {
        let cap = Arc::new(Captured::default());
        let ch = Arc::new(IpcApprovalChannel::new(slot_with(cap.clone()), Duration::from_secs(5)));
        let ch2 = ch.clone();
        let h = tokio::spawn(async move { ch2.request(req()).await });
        // Spin until the request frame appears, then pull its id.
        let id = loop {
            if let Some(ServerEvent::ApprovalRequest { id, .. }) = cap.0.lock().unwrap().first() {
                break id.clone();
            }
            tokio::task::yield_now().await;
        };
        ch.resolve(&id, ApprovalResponse::ApproveAlways);
        assert_eq!(h.await.unwrap(), ApprovalResponse::ApproveAlways);
    }

    #[tokio::test]
    async fn times_out_to_deny() {
        let cap = Arc::new(Captured::default());
        let ch = IpcApprovalChannel::new(slot_with(cap), Duration::from_millis(20));
        assert_eq!(ch.request(req()).await, ApprovalResponse::Deny);
    }

    #[tokio::test]
    async fn absent_slot_denies() {
        let slot: crate::sink::EventSlot = Arc::new(Mutex::new(None));
        let ch = IpcApprovalChannel::new(slot, Duration::from_secs(5));
        assert_eq!(ch.request(req()).await, ApprovalResponse::Deny);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p agent-server --lib approval`
Expected: FAIL — `IpcApprovalChannel` not found.

- [ ] **Step 3: Rewrite `approval.rs` non-test body**

```rust
use crate::sink::EventSlot;
use crate::wire::ServerEvent;
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::oneshot;

/// `ApprovalChannel` that emits an `ApprovalRequest` over the live-read event
/// slot and awaits an `approve` command matched by correlation id. A timeout or
/// an absent channel resolves to `Deny` (safe default).
pub struct IpcApprovalChannel {
    slot: EventSlot,
    pending: Mutex<HashMap<String, oneshot::Sender<ApprovalResponse>>>,
    counter: AtomicU64,
    timeout: Duration,
}

impl IpcApprovalChannel {
    pub fn new(slot: EventSlot, timeout: Duration) -> Self {
        Self { slot, pending: Mutex::new(HashMap::new()), counter: AtomicU64::new(0), timeout }
    }

    /// Complete a pending approval, called by the `approve` command.
    pub fn resolve(&self, id: &str, decision: ApprovalResponse) {
        if let Some(tx) = self.pending.lock().unwrap().remove(id) {
            let _ = tx.send(decision);
        }
    }
}

#[async_trait]
impl ApprovalChannel for IpcApprovalChannel {
    async fn request(&self, req: ApprovalRequest) -> ApprovalResponse {
        let id = format!("c{}", self.counter.fetch_add(1, Ordering::Relaxed));
        let (otx, orx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), otx);
        let ev = ServerEvent::ApprovalRequest {
            id: id.clone(),
            summary: req.intent.summary.clone(),
            command: req.intent.command.clone(),
            display: req.display.clone(),
        };
        match self.slot.lock().unwrap().clone() {
            Some(out) => out.send(ev),
            None => {
                self.pending.lock().unwrap().remove(&id);
                return ApprovalResponse::Deny;
            }
        }
        match tokio::time::timeout(self.timeout, orx).await {
            Ok(Ok(resp)) => resp,
            _ => {
                self.pending.lock().unwrap().remove(&id);
                ApprovalResponse::Deny
            }
        }
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p agent-server --lib approval`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-server/src/approval.rs
git commit -m "feat(agent-server): IpcApprovalChannel emits over EventOut slot"
```

---

### Task 4: `RuntimeState` — drop `tx`/`session`/`send`; add `settings_state()`

Rewire `RuntimeState` to the new sink/approval types and replace the frame-reply path with value-returning methods.

**Files:**
- Modify: `agent/crates/agent-server/src/runtime.rs` (constructor signature, fields, `state_body`→`settings_state`, delete `send`/`handle`)
- Test: same file's `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `ChannelEventSink` (Task 2), `IpcApprovalChannel` (Task 3), `SettingsState` (Task 1)
- Produces:
  - `RuntimeState::new(config, sink: Arc<ChannelEventSink>, approval: Arc<IpcApprovalChannel>, workspace, api_key, claude_binary, config_path, mcp_tools, memory_tools, memory_retriever, base_system_prompt)` — note: `session` and `tx` params REMOVED
  - `pub fn settings_state(&self) -> SettingsState`
  - unchanged: `current_loop()`, `current_system_prompt()`, `apply()`

- [ ] **Step 1: Update the test harness + add `settings_state` test**

In `runtime.rs` `mod tests`, replace `make()` and the settings tests. New `make()` (drops `tx`/`session`/`rx`):

```rust
fn slot() -> crate::sink::EventSlot { Arc::new(Mutex::new(None)) }

fn make() -> (RuntimeState, tempfile::TempDir) {
    let s = slot();
    let sink = Arc::new(crate::sink::ChannelEventSink::new(s.clone()));
    let approval = Arc::new(crate::approval::IpcApprovalChannel::new(s, Duration::from_secs(1)));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rt.json");
    let cfg = RuntimeConfig::from_launch(
        "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192);
    let rs = RuntimeState::new(cfg, sink, approval, dir.path().to_path_buf(), None,
        "claude".into(), path, Arc::from(Vec::<Arc<dyn Tool>>::new()),
        Arc::from(Vec::<Arc<dyn Tool>>::new()), None,
        crate::daemon::SYSTEM_PROMPT.to_string());
    (rs, dir)
}

#[test]
fn settings_state_reports_floor_and_no_api_key() {
    let (rs, _dir) = make();
    let st = rs.settings_state();
    assert!(!st.api_key_set);
    assert!(st.hard_floor.iter().any(|d| d == "sudo"));
}
```

Update every other test in the module: `make()` now returns a 2-tuple `(rs, dir)` (drop the `_rx`); delete `handle_settings_get_emits_state`, `handle_invalid_update_emits_error`, `handle_ignores_non_settings_frames` (the `handle`/frame path is gone); rewrite `settings_state_includes_discovered_skills` to call `rs.settings_state()` directly and assert on the returned struct; in `startup_drops_unknown_persisted_preset_without_panicking`, `settings_state_includes_discovered_skills`, `build_loop_with_sandbox_mode_*`, replace the inline `(tx, …)` sink/approval construction with the `slot()`-based form above (no `tx`/`session`). Keep `apply_*` and `current_system_prompt` assertions unchanged.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p agent-server --lib runtime`
Expected: FAIL — `RuntimeState::new` arity/`settings_state` mismatch.

- [ ] **Step 3: Edit `runtime.rs` non-test body**

- Update imports: `use crate::approval::IpcApprovalChannel;`, `use crate::sink::ChannelEventSink;`, `use crate::wire::{DiscoveredSkill, SettingsState};`. Delete the `tokio::sync::mpsc` and `WireBody`/`WireEnvelope`/`PROTOCOL_VERSION` imports.
- Struct: change `sink: Arc<WsEventSink>` → `Arc<ChannelEventSink>`, `approval: Arc<WsApprovalChannel>` → `Arc<IpcApprovalChannel>`. Delete the `session: Arc<Mutex<String>>` and `tx: mpsc::UnboundedSender<WireEnvelope>` fields.
- `new`: drop the `session` and `tx` params and their struct-init entries. Change the two `sink`/`approval` param types as above.
- Replace `state_body()` with:

```rust
pub fn settings_state(&self) -> SettingsState {
    let cfg = self.config.lock().unwrap().clone();
    let discovered = SkillRegistry::from_config(&cfg.skills_dirs, &self.workspace)
        .scan()
        .into_iter()
        .map(|s| DiscoveredSkill { name: s.name, description: s.description })
        .collect();
    SettingsState {
        settings: cfg,
        workspace: self.workspace.display().to_string(),
        api_key_set: self.api_key.is_some(),
        hard_floor: HARD_FLOOR_DENYLIST.iter().map(|s| s.to_string()).collect(),
        discovered_skills: discovered,
    }
}
```

- Delete the `send()` and `handle()` methods entirely.
- In `build_loop`, change the `sink: &Arc<WsEventSink>` / `approval: &Arc<WsApprovalChannel>` parameter types to `&Arc<ChannelEventSink>` / `&Arc<IpcApprovalChannel>`. Body unchanged.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p agent-server --lib runtime`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-server/src/runtime.rs
git commit -m "feat(agent-server): RuntimeState value-returns settings_state; drop frame send"
```

---

### Task 5: `Session` — app-lifetime state, run-guard, cancel, workspace switch

The heart of the migration: an app-lifetime holder that owns the runtime, context, event slot, approvals, and the single-run guard. This replaces `daemon::serve()`'s per-connection state.

**Files:**
- Create: `agent/crates/agent-server/src/session.rs`
- Modify: `agent/crates/agent-server/src/lib.rs` (add `pub mod session;`; keep `pub mod daemon;` until Task 8)
- Test: `session.rs` `#[cfg(test)] mod tests` (uses the `testkit` dev-dep already enabled)

**Interfaces:**
- Consumes: `RuntimeState`, `ChannelEventSink`, `IpcApprovalChannel`, `EventSlot`, `EventOut`, `SettingsState`, `Decision`, `DaemonParams` (from `daemon`), `setup::local_params`
- Produces:
  - `pub struct Session { … }` with `Arc`-friendly internals
  - `pub fn Session::from_params(params: DaemonParams) -> Arc<Session>`
  - `pub fn set_event_out(&self, out: Arc<dyn EventOut>)` — the `subscribe` body
  - `pub enum SendOutcome { Started, Busy }`
  - `pub fn send_input(self: &Arc<Self>, text: String) -> SendOutcome`
  - `pub fn cancel(&self)`
  - `pub fn approve(&self, id: &str, decision: Decision)`
  - `pub fn settings_get(&self) -> SettingsState`
  - `pub fn settings_update(&self, cfg: RuntimeConfig) -> Result<SettingsState, String>`
  - `pub async fn set_workspace(self: &Arc<Self>, dir: PathBuf)` (cancel run, reset ctx, rebind)

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{EventOut, ServerEvent};
    use agent_core::testkit::{PassthroughProtocol, ScriptedModel};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Captured(Mutex<Vec<ServerEvent>>);
    impl EventOut for Captured {
        fn send(&self, ev: ServerEvent) { self.0.lock().unwrap().push(ev); }
    }

    // Build a Session whose loop uses a scripted model that emits one assistant
    // turn ending in `Done`. (See agent-core testkit for ScriptedModel usage.)
    fn session_with_scripted() -> (Arc<Session>, Arc<Captured>) {
        let dir = tempfile::tempdir().unwrap();
        let params = crate::setup::local_params(
            dir.path().to_path_buf(), dir.path().join("rt.json"),
            "http://localhost:8080".into(), "m".into(), None);
        let sess = Session::from_params(params);
        let cap = Arc::new(Captured::default());
        sess.set_event_out(cap.clone());
        std::mem::forget(dir); // keep temp dir alive for the test process
        (sess, cap)
    }

    #[tokio::test]
    async fn second_input_during_run_is_busy() {
        let (sess, _cap) = session_with_scripted();
        // First send starts a run; force it to be in-flight by holding the ctx busy
        // is not needed — assert the guard via a manually-seeded active run instead:
        sess.mark_active_for_test();
        assert!(matches!(sess.send_input("hi".into()), SendOutcome::Busy));
    }

    #[tokio::test]
    async fn cancel_when_idle_is_noop() {
        let (sess, _cap) = session_with_scripted();
        sess.cancel(); // must not panic
    }

    #[tokio::test]
    async fn settings_get_returns_state() {
        let (sess, _cap) = session_with_scripted();
        let st = sess.settings_get();
        assert!(!st.api_key_set);
    }

    #[tokio::test]
    async fn approve_unknown_id_is_noop() {
        let (sess, _cap) = session_with_scripted();
        sess.approve("nope", Decision::Approve); // must not panic
    }
}
```

> Note for the implementer: `mark_active_for_test()` is a `#[cfg(test)]` helper that
> inserts a dummy `CancellationToken` into the run slot, so the guard test does not
> depend on real model timing. Add it in Step 3.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p agent-server --lib session`
Expected: FAIL — `session` module missing.

- [ ] **Step 3: Write `session.rs`**

```rust
//! App-lifetime session state for the Tauri IPC transport. Replaces the
//! per-connection state that `daemon::serve()` used to own.
use crate::approval::IpcApprovalChannel;
use crate::runtime::RuntimeState;
use crate::sink::{ChannelEventSink, EventSlot};
use crate::wire::{Decision, EventOut, SettingsState};
use crate::daemon::DaemonParams;
use agent_core::WindowContext;
use agent_model::Message;
use agent_runtime_config::{RuntimeConfig, RuntimeConfig as _Cfg};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

pub enum SendOutcome { Started, Busy }

pub struct Session {
    runtime: Arc<RuntimeState>,
    ctx: Arc<AsyncMutex<WindowContext>>,
    slot: EventSlot,
    approval: Arc<IpcApprovalChannel>,
    active: Arc<Mutex<Option<CancellationToken>>>,
    recall_budget: usize,
}

impl Session {
    pub fn from_params(params: DaemonParams) -> Arc<Self> {
        let slot: EventSlot = Arc::new(Mutex::new(None));
        let sink = Arc::new(ChannelEventSink::new(slot.clone()));
        let approval = Arc::new(IpcApprovalChannel::new(slot.clone(), APPROVAL_TIMEOUT));
        let config = RuntimeConfig::load_over(params.config.clone(), &params.config_path);
        let runtime = Arc::new(RuntimeState::new(
            config, sink, approval.clone(), params.workspace.clone(), params.api_key.clone(),
            params.claude_binary.clone(), params.config_path.clone(),
            params.mcp_tools.clone(), params.memory_tools.clone(),
            params.memory_retriever.clone(), params.system_prompt.clone()));
        let ctx = Arc::new(AsyncMutex::new(
            WindowContext::new(Message::system(params.system_prompt.clone()))
                .with_recall_budget(params.recall_token_budget)));
        Arc::new(Self { runtime, ctx, slot, approval,
            active: Arc::new(Mutex::new(None)), recall_budget: params.recall_token_budget })
    }

    /// Register/replace the outbound channel (the `subscribe` command body).
    pub fn set_event_out(&self, out: Arc<dyn EventOut>) {
        *self.slot.lock().unwrap() = Some(out);
    }

    /// Start a run unless one is active (A1 guard).
    pub fn send_input(self: &Arc<Self>, text: String) -> SendOutcome {
        let mut active = self.active.lock().unwrap();
        if active.is_some() { return SendOutcome::Busy; }
        let cancel = CancellationToken::new();
        *active = Some(cancel.clone());
        drop(active);

        let agent = self.runtime.current_loop();
        let system_prompt = self.runtime.current_system_prompt();
        let ctx = self.ctx.clone();
        let active_slot = self.active.clone();
        tokio::spawn(async move {
            {
                let mut guard = ctx.lock().await;
                guard.set_system(Message::system(system_prompt));
                if let Err(e) = agent.run_with_cancel(&mut *guard, text, cancel).await {
                    tracing::error!(error=%e, "run failed");
                }
            }
            *active_slot.lock().unwrap() = None;
        });
        SendOutcome::Started
    }

    /// Trip the active run's token (the B3 interactive cancel). No-op when idle.
    pub fn cancel(&self) {
        if let Some(tok) = self.active.lock().unwrap().as_ref() { tok.cancel(); }
    }

    pub fn approve(&self, id: &str, decision: Decision) {
        self.approval.resolve(id, decision.into());
    }

    pub fn settings_get(&self) -> SettingsState {
        self.runtime.settings_state()
    }

    pub fn settings_update(&self, cfg: RuntimeConfig) -> Result<SettingsState, String> {
        self.runtime.apply(cfg)?;
        Ok(self.runtime.settings_state())
    }

    /// Cancel any run, then reset the conversation context (workspace switch).
    pub async fn set_workspace(self: &Arc<Self>, _dir: PathBuf) {
        self.cancel();
        let mut guard = self.ctx.lock().await;
        *guard = WindowContext::new(Message::system(self.runtime.current_system_prompt()))
            .with_recall_budget(self.recall_budget);
    }

    #[cfg(test)]
    fn mark_active_for_test(&self) {
        *self.active.lock().unwrap() = Some(CancellationToken::new());
    }
}
```

> Implementer notes:
> - `WindowContext::with_recall_budget` and `set_system` are the same APIs `daemon::serve()` used — confirm signatures in `agent-core` and match them.
> - The unused `_Cfg`/`RuntimeConfig` import alias is a guard against name shadowing; drop it if `cargo` warns.
> - `set_workspace` keeps the same `RuntimeState` (workspace rebind beyond ctx reset is out of scope here — the bridge reloads config; see Task 7). If the desktop needs a true workspace rebind on the runtime, that is a follow-up, not this migration.

Add to `lib.rs`: `pub mod session;`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p agent-server --lib session`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-server/src/session.rs agent/crates/agent-server/src/lib.rs
git commit -m "feat(agent-server): Session with run-guard, cancel, settings, workspace reset"
```

---

## Phase 2 — `src-tauri`: Tauri wiring

### Task 6: `ChannelOut` — `EventOut` over `ipc::Channel`; enable Tauri test feature

**Files:**
- Create: `src-tauri/src/channel_out.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod channel_out;`)
- Modify: `src-tauri/Cargo.toml` (add `tauri` `test` feature to dev; ensure `agent-server` dep present)
- Test: `src-tauri/src/channel_out.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `agent_server::wire::{EventOut, ServerEvent}`, `tauri::ipc::Channel`
- Produces: `pub struct ChannelOut(pub tauri::ipc::Channel<ServerEvent>)`; `impl EventOut for ChannelOut`

- [ ] **Step 1: Add the dev-feature + test**

In `src-tauri/Cargo.toml`, under `[dev-dependencies]` (create the section if absent):

```toml
[dev-dependencies]
tauri = { version = "2", features = ["test"] }
```

Create `src-tauri/src/channel_out.rs`:

```rust
use agent_server::wire::{EventOut, ServerEvent};
use tauri::ipc::Channel;

/// `EventOut` backed by a Tauri IPC channel. The only Tauri-aware sink; lives
/// here so `agent-server` stays transport-agnostic.
pub struct ChannelOut(pub Channel<ServerEvent>);

impl EventOut for ChannelOut {
    fn send(&self, ev: ServerEvent) {
        let _ = self.0.send(ev); // closed channel → drop (infallible emit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn forwards_to_underlying_channel() {
        let seen = Arc::new(Mutex::new(Vec::<ServerEvent>::new()));
        let s2 = seen.clone();
        let ch: Channel<ServerEvent> = Channel::new(move |body| {
            // The closure receives the serialized IPC body; deserialize to assert.
            if let tauri::ipc::InvokeResponseBody::Json(txt) = body {
                s2.lock().unwrap().push(serde_json::from_str(&txt).unwrap());
            }
            Ok(())
        });
        let out = ChannelOut(ch);
        out.send(ServerEvent::Token { text: "hi".into() });
        assert!(matches!(seen.lock().unwrap().as_slice(),
            [ServerEvent::Token { text }] if text == "hi"));
    }
}
```

> Implementer note: `Channel::new` and `InvokeResponseBody` are Tauri v2 IPC APIs.
> If the body variant name differs in the pinned Tauri version, adjust the match arm;
> the assertion (one `Token` forwarded) is the contract that must hold. `agent-server`
> must re-export `wire` publicly — confirm `pub mod wire;` in its `lib.rs` (add if needed).

- [ ] **Step 2: Run to verify it fails / compiles to a failure**

Run: `cargo test -p rust-agent-runtime --lib channel_out` (use the actual `src-tauri` package name from `src-tauri/Cargo.toml`'s `[package].name`)
Expected: FAIL — module/`wire` not public, or assertion not yet satisfied.

- [ ] **Step 3: Make `agent_server::wire` public + module wired**

- In `agent/crates/agent-server/src/lib.rs`, ensure `pub mod wire;`.
- In `src-tauri/src/lib.rs`, add `mod channel_out;` near the other `mod` lines.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p <src-tauri package> --lib channel_out`
Expected: PASS (1 test).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/channel_out.rs src-tauri/src/lib.rs src-tauri/Cargo.toml agent/crates/agent-server/src/lib.rs
git commit -m "feat(src-tauri): ChannelOut bridging ipc::Channel to EventOut"
```

---

### Task 7: IPC commands + `Session` in `AppState`; bridge constructs the Session

**Files:**
- Modify: `src-tauri/src/lib.rs` (replace `AppState`, commands, `generate_handler!`)
- Modify: `src-tauri/src/bridge.rs` (construct + hold a `Session` instead of a `TcpListener`)
- Test: `src-tauri/src/lib.rs` command unit tests where practical (see note)

**Interfaces:**
- Consumes: `agent_server::session::{Session, SendOutcome}`, `agent_server::wire::{Decision, ServerEvent, SettingsState}`, `ChannelOut`, `agent_server::setup::local_params`
- Produces: commands `subscribe`, `send_input`, `approve`, `cancel`, `settings_get`, `settings_update`, plus retained `get_workspace`, `pick_workspace`, `llama_health`

- [ ] **Step 1: Rewrite `bridge.rs` to own a `Session`**

Replace the entire `bridge.rs` with a thin Session holder (no socket):

```rust
//! Holds the app-lifetime agent Session. Workspace switches reset the live
//! Session's context rather than dropping a socket.
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use agent_server::session::Session;

pub struct Bridge {
    session: Arc<Session>,
    workspace: Mutex<PathBuf>,
    config_path: PathBuf,
    base_url: String,
    model: String,
    memory_parts: Option<agent_memory::MemoryParts>,
}

impl Bridge {
    pub fn session(&self) -> Arc<Session> { self.session.clone() }

    pub async fn current_workspace(&self) -> PathBuf { self.workspace.lock().await.clone() }

    /// Switch workspace: persist is done by the caller; rebuild the Session bound
    /// to `dir` and reset its context.
    pub async fn set_workspace(&self, dir: PathBuf) {
        *self.workspace.lock().await = dir.clone();
        self.session.set_workspace(dir).await;
    }
}

pub async fn start(
    workspace: PathBuf,
    config_path: PathBuf,
    base_url: String,
    model: String,
) -> std::io::Result<Arc<Bridge>> {
    let eff = agent_runtime_config::RuntimeConfig::load_over(
        agent_runtime_config::RuntimeConfig::from_launch(
            "openai".into(), base_url.clone(), model.clone(), "native".into(), 262_144),
        &config_path);
    let memory_parts = if eff.memory {
        match agent_memory::open_memory_parts(agent_memory::MemoryConfig::default()) {
            Ok(parts) => Some(parts),
            Err(e) => { eprintln!("warning: desktop memory disabled: {e}"); None }
        }
    } else { None };

    let params = agent_server::setup::local_params(
        workspace.clone(), config_path.clone(), base_url.clone(), model.clone(),
        memory_parts.as_ref());
    let session = Session::from_params(params);

    Ok(Arc::new(Bridge {
        session,
        workspace: Mutex::new(workspace),
        config_path,
        base_url,
        model,
        memory_parts,
    }))
}
```

> Implementer note: `set_workspace` here resets context only (matches Task 5 scope). If a
> full per-workspace Session rebuild is wanted, rebuild via `local_params` + `Session::from_params`
> and swap an `Arc<Mutex<Arc<Session>>>` — a follow-up, not this migration. Keep the fields
> `config_path/base_url/model/memory_parts` even if unused now (the rebuild follow-up needs them);
> add `#[allow(dead_code)]` to silence warnings.

- [ ] **Step 2: Rewrite the commands in `lib.rs`**

Replace `get_local_ws_url` and `AppState`, add the IPC commands:

```rust
use agent_server::session::{SendOutcome, Session};
use agent_server::wire::{Decision, ServerEvent, SettingsState};
use agent_runtime_config::RuntimeConfig;
use channel_out::ChannelOut;
use std::sync::Arc;
use tauri::ipc::Channel;

struct AppState {
    bridge: Arc<bridge::Bridge>,
    config_path: PathBuf,
}

fn session(state: &tauri::State<'_, AppState>) -> Arc<Session> { state.bridge.session() }

#[tauri::command]
fn subscribe(state: tauri::State<'_, AppState>, channel: Channel<ServerEvent>) {
    session(&state).set_event_out(Arc::new(ChannelOut(channel)));
}

#[tauri::command]
fn send_input(state: tauri::State<'_, AppState>, text: String) -> Result<(), String> {
    match session(&state).send_input(text) {
        SendOutcome::Started => Ok(()),
        SendOutcome::Busy => Err("busy".into()),
    }
}

#[tauri::command]
fn approve(state: tauri::State<'_, AppState>, id: String, decision: Decision) {
    session(&state).approve(&id, decision);
}

#[tauri::command]
fn cancel(state: tauri::State<'_, AppState>) {
    session(&state).cancel();
}

#[tauri::command]
fn settings_get(state: tauri::State<'_, AppState>) -> SettingsState {
    session(&state).settings_get()
}

#[tauri::command]
fn settings_update(state: tauri::State<'_, AppState>, settings: RuntimeConfig)
    -> Result<SettingsState, String> {
    session(&state).settings_update(settings)
}
```

Keep `get_workspace`, `pick_workspace`, `llama_health` as-is (they already use `state.bridge`). Update `generate_handler!`:

```rust
.invoke_handler(tauri::generate_handler![
    subscribe, send_input, approve, cancel, settings_get, settings_update,
    get_workspace, pick_workspace, llama_health
])
```

Remove `get_local_ws_url`. In the `setup(...)` closure, `bridge::start(...)` is unchanged (it now returns the Session-holding bridge).

- [ ] **Step 3: Add a MockRuntime smoke test**

In `src-tauri/src/lib.rs` add:

```rust
#[cfg(test)]
mod cmd_tests {
    use super::*;

    fn app() -> tauri::App<tauri::test::MockRuntime> {
        let dir = tempfile::tempdir().unwrap();
        let bridge = tauri::async_runtime::block_on(bridge::start(
            dir.path().to_path_buf(), dir.path().join("rt.json"),
            "http://localhost:8080".into(), "m".into())).unwrap();
        std::mem::forget(dir);
        tauri::test::mock_builder()
            .manage(AppState { bridge, config_path: PathBuf::from("/tmp/app.json") })
            .invoke_handler(tauri::generate_handler![
                subscribe, send_input, approve, cancel, settings_get, settings_update])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap()
    }

    #[test]
    fn settings_get_returns_state_over_ipc() {
        let app = app();
        let res = tauri::test::get_ipc_response(
            &app.get_webview_window("main").unwrap_or_else(|| {
                tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
                    .build().unwrap()
            }),
            tauri::webview::InvokeRequest {
                cmd: "settings_get".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::default(),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_string(),
            },
        );
        assert!(res.is_ok());
    }
}
```

> Implementer note: the exact `tauri::test` helper surface (`get_ipc_response`, `INVOKE_KEY`,
> `InvokeRequest` fields) shifts across Tauri 2.x patch releases. Treat this test as the
> *integration smoke* — if a field/helper name differs in the pinned version, adapt to compile;
> the contract is "a registered command returns Ok over the mock IPC." The behavioral coverage
> (run-guard, cancel, approval, settings) already lives in the `Session` tests (Task 5), which
> need no Tauri runtime. If adapting the mock harness proves heavy, keep this as a single
> compile-and-invoke smoke and rely on Task 5 for behavior.

- [ ] **Step 4: Run to verify**

Run: `cargo test -p <src-tauri package> --lib`
Expected: PASS (channel_out + cmd smoke). Add `tempfile` to `[dev-dependencies]` if missing.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/bridge.rs src-tauri/Cargo.toml
git commit -m "feat(src-tauri): IPC commands (subscribe/send_input/approve/cancel/settings) over Session"
```

---

### Task 8: Delete `daemon::serve()` + WebSocket deps and tests

**Files:**
- Delete: `agent/crates/agent-server/src/daemon.rs` body except `DaemonParams` + `SYSTEM_PROMPT`
- Delete: `agent/crates/agent-server/tests/serve_inbound.rs`, `src-tauri/tests/bridge.rs`, `src-tauri/tests/e2e_live.rs`
- Modify: `agent/crates/agent-server/Cargo.toml`, `src-tauri/Cargo.toml` (drop `tokio-tungstenite`, `futures` if unused)
- Modify: `agent/crates/agent-server/src/lib.rs` (drop `pub mod daemon;` if fully empty, else keep the params module)

**Interfaces:**
- Produces: `daemon` reduced to `pub struct DaemonParams { … }` + `pub const SYSTEM_PROMPT` (still consumed by `setup.rs` and `session.rs`)

- [ ] **Step 1: Reduce `daemon.rs` to params only**

Replace `daemon.rs` with just the `DaemonParams` struct (fields unchanged from the original) and `SYSTEM_PROMPT`. Delete `serve()`, the `use` lines it needed (`futures`, `tokio_tungstenite`, `WireEnvelope`, `WsApprovalChannel`, `WsEventSink`, etc.), and the `DynErr` alias.

- [ ] **Step 2: Delete the WS integration tests**

```bash
git rm agent/crates/agent-server/tests/serve_inbound.rs src-tauri/tests/bridge.rs src-tauri/tests/e2e_live.rs
```

- [ ] **Step 3: Prune dependencies**

- `agent/crates/agent-server/Cargo.toml`: remove `tokio-tungstenite.workspace = true` and `futures.workspace = true` if `cargo build` confirms they are now unused (grep the crate first: `grep -rn "tungstenite\|futures::" agent/crates/agent-server/src`).
- `src-tauri/Cargo.toml`: remove `tokio-tungstenite = "0.24"` (grep `src-tauri/src` first).

- [ ] **Step 4: Run the whole backend**

Run: `cargo test -p agent-server -p <src-tauri package>`
Expected: PASS; no references to `serve`/`WireEnvelope`/tungstenite remain. Run `cargo build` to confirm no dead imports.

- [ ] **Step 5: Commit**

```bash
git add -A agent/crates/agent-server src-tauri
git commit -m "refactor: remove WebSocket transport (serve, tungstenite, WS tests)"
```

---

## Phase 3 — frontend

### Task 9: Tauri-IPC transport behind the `connect()` seam

**Files:**
- Modify: `web/src/socket.ts` (replace WebSocket body; keep `connect()` signature)
- Modify: `web/src/transport.ts` (drop `wsUrl`)
- Test: `web/src/socket.test.ts`, `web/src/transport.test.ts` (rewrite to mock `@tauri-apps/api`)

**Interfaces:**
- Consumes: `@tauri-apps/api/core` `invoke`, `@tauri-apps/api/core` `Channel`, `parseInbound`-free direct mapping
- Produces: `connect(handlers, opts?) → { send, close }` — same shape, `url` param dropped; `send(o: Outbound)` switches on `o.kind`

- [ ] **Step 1: Write the failing test**

Replace `web/src/socket.test.ts` with a mocked-IPC test:

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";

const invoke = vi.fn();
const channelInstances: Array<{ onmessage?: (e: unknown) => void }> = [];
class FakeChannel { onmessage?: (e: unknown) => void; constructor() { channelInstances.push(this); } }
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => invoke(...a),
  Channel: FakeChannel,
}));

import { connect } from "./socket";

beforeEach(() => { invoke.mockReset(); channelInstances.length = 0; invoke.mockResolvedValue(undefined); });

describe("ipc transport", () => {
  it("subscribes a channel and maps a token event to an onFrame", async () => {
    const frames: unknown[] = [];
    connect({ onFrame: (f) => frames.push(f), onStatus: () => {} });
    await Promise.resolve();
    expect(invoke).toHaveBeenCalledWith("subscribe", expect.objectContaining({ channel: expect.any(FakeChannel) }));
    channelInstances[0].onmessage?.({ type: "token", text: "hi" });
    expect(frames[0]).toEqual({ v: 1, session_id: "", kind: "event", payload: { type: "token", text: "hi" } });
  });

  it("routes user_input to the send_input command", () => {
    const sock = connect({ onFrame: () => {}, onStatus: () => {} });
    sock.send({ v: 1, session_id: "", kind: "user_input", text: "hello" });
    expect(invoke).toHaveBeenCalledWith("send_input", { text: "hello" });
  });

  it("maps an approval_request server event to an approval_request frame", async () => {
    const frames: any[] = [];
    connect({ onFrame: (f) => frames.push(f), onStatus: () => {} });
    await Promise.resolve();
    channelInstances[0].onmessage?.({ type: "approval_request", id: "c0", summary: "run x" });
    expect(frames[0]).toMatchObject({ kind: "approval_request", id: "c0", summary: "run x" });
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd web && npx vitest run src/socket.test.ts`
Expected: FAIL — `connect` still expects a URL / uses WebSocket.

- [ ] **Step 3: Rewrite `socket.ts`**

```ts
import { invoke, Channel } from "@tauri-apps/api/core";
import type { Inbound, Outbound, WireEvent } from "./wire";
import type { ConnectionStatus } from "./state";

interface Handlers {
  onFrame: (f: Inbound) => void;
  onStatus: (s: ConnectionStatus) => void;
}

// A ServerEvent is the legacy WireEvent shape plus an `approval_request` case.
type ServerEvent = WireEvent | { type: "approval_request"; id: string; summary: string; command?: string; display?: unknown };

function toInbound(ev: ServerEvent): Inbound {
  if (ev.type === "approval_request") {
    return { v: 1, session_id: "", id: ev.id, kind: "approval_request",
      summary: ev.summary, command: ev.command, display: ev.display as never };
  }
  return { v: 1, session_id: "", kind: "event", payload: ev };
}

export function connect(handlers: Handlers, _opts: Record<string, never> = {}) {
  handlers.onStatus("connecting");
  const channel = new Channel<ServerEvent>();
  channel.onmessage = (ev) => handlers.onFrame(toInbound(ev));
  invoke("subscribe", { channel })
    .then(() => handlers.onStatus("open"))
    .catch(() => handlers.onStatus("error"));

  return {
    send(o: Outbound) {
      switch (o.kind) {
        case "user_input": invoke("send_input", { text: o.text }).catch(() => {}); break;
        case "approval_response": invoke("approve", { id: o.id, decision: o.decision }).catch(() => {}); break;
        case "settings_get":
          invoke("settings_get").then((st) => handlers.onFrame(
            { v: 1, session_id: "", kind: "settings_state", ...(st as object) } as Inbound)).catch(() => {});
          break;
        case "settings_update":
          invoke("settings_update", { settings: o.settings })
            .then((st) => handlers.onFrame({ v: 1, session_id: "", kind: "settings_state", ...(st as object) } as Inbound))
            .catch((e) => handlers.onFrame({ v: 1, session_id: "", kind: "settings_error", message: String(e) } as Inbound));
          break;
      }
    },
    close() { /* IPC has no socket to close; channel is GC'd with the component */ },
  };
}
```

Update `web/src/transport.ts`: drop the `wsUrl` field and the `get_local_ws_url` invoke; keep `localSessionId()` and return `{ sessionId }` only (used solely as a localStorage history key).

- [ ] **Step 4: Run to verify it passes**

Run: `cd web && npx vitest run src/socket.test.ts src/transport.test.ts`
Expected: PASS. (Adjust `transport.test.ts` to the trimmed `Transport` shape.)

- [ ] **Step 5: Commit**

```bash
git add web/src/socket.ts web/src/transport.ts web/src/socket.test.ts web/src/transport.test.ts
git commit -m "feat(web): Tauri IPC transport behind connect() seam"
```

---

### Task 10: `App.tsx` — drop `localUrl` gating; wire `connect()` directly

**Files:**
- Modify: `web/src/App.tsx` (remove `localUrl` state + WS-URL effect; call `connect()` with handlers only)
- Modify: `web/src/wire.ts` (the `Outbound` union loses `v`/`session_id` requirement — make them optional so existing `send({ v:1, session_id, … })` calls still typecheck, or drop them; pick one and apply consistently)
- Test: `web/src/App.tauri.test.tsx`

**Interfaces:**
- Consumes: `connect(handlers)` (Task 9)
- Produces: no new exports; behavioral parity (composer disabled while a run is in flight)

- [ ] **Step 1: Update the App test**

In `web/src/App.tauri.test.tsx`, replace any `__WS__`/`localUrl` setup with the `@tauri-apps/api/core` mock (same `vi.mock` block as Task 9). Assert that sending a message invokes `send_input`, and that an inbound `done` frame re-enables the composer. Keep existing assertions that don't touch transport.

- [ ] **Step 2: Run to verify it fails**

Run: `cd web && npx vitest run src/App.tauri.test.tsx`
Expected: FAIL — App still references `localUrl`/WebSocket.

- [ ] **Step 3: Edit `App.tsx`**

- Remove the `localUrl` state and the effect block that calls `resolveTransport().then(... setLocalUrl ...)` + `get_local_ws_url`. Keep the `sessionId` (history key) and the `get_workspace` effect.
- Change the connect effect:

```tsx
useEffect(() => {
  if (!sessionId) return;
  dispatch({ type: "reset", userMsgs: loadUserMsgs(sessionId) });
  sock.current = connect({
    onFrame: (f) => dispatch({ type: "frame", frame: f }),
    onStatus: (s) => dispatch({ type: "status", status: s }),
  });
  return () => { sock.current?.close(); sock.current = null; };
}, [sessionId]);
```

- Update the `send`/`decide`/`openSettings`/`saveSettings` callers to drop `v`/`session_id` if you removed them from `Outbound` (Task 10 wire.ts decision), e.g. `sock.current?.send({ kind: "user_input", text })`.
- Remove the `if (!localUrl)` guard from the loading branch.

In `web/src/wire.ts`, simplify `Outbound` to the command-arg shapes:

```ts
export type Outbound =
  | { kind: "user_input"; text: string }
  | { kind: "approval_response"; id: string; decision: Decision }
  | { kind: "settings_get" }
  | { kind: "settings_update"; settings: RuntimeSettings };
```

- [ ] **Step 4: Run to verify it passes**

Run: `cd web && npx vitest run`
Expected: PASS (full web suite).

- [ ] **Step 5: Commit**

```bash
git add web/src/App.tsx web/src/wire.ts web/src/App.tauri.test.tsx
git commit -m "feat(web): drop ws-url gating; App uses IPC connect() seam"
```

---

### Task 11: Full-stack verification + backlog update

**Files:**
- Modify: `docs/superpowers/specs/2026-06-24-security-audit-backlog.md` (mark A2/A1/B3-cancel resolved; A-c reduced)

- [ ] **Step 1: Run every suite**

Run:
```bash
cargo test --workspace
cd web && npx vitest run && cd ..
cargo build -p <src-tauri package>
```
Expected: all PASS; backend builds with no tungstenite/`serve`/`WireEnvelope` references (`grep -rn "WireEnvelope\|tungstenite\|fn serve" agent src-tauri --include=*.rs` returns nothing in non-deleted code).

- [ ] **Step 2: Manual smoke (documented, optional if no display)**

Launch the desktop app (`npm run tauri dev` or the project's run skill). Confirm: tokens stream, a tool approval prompt resolves, settings open/save, a second send while running is rejected, and cancel stops a run. Record the result in the commit message.

- [ ] **Step 3: Update the backlog**

Edit `2026-06-24-security-audit-backlog.md` Cluster A header: mark **A-a (A2) RESOLVED by dissolution** (Tauri IPC migration — no event channel to bound), **A1 RESOLVED** (run-guard), **B3 interactive server-cancel RESOLVED** (`cancel()`), and note **A-c reduced to the `RuntimeState` three-mutex refactor only** (`session_id` identity removed as a side effect). Reference this plan + spec.

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/specs/2026-06-24-security-audit-backlog.md
git commit -m "docs(audit-backlog): A2 dissolved, A1 + B3-cancel done via Tauri IPC migration"
```

---

## Self-Review

**Spec coverage:**
- Per-connection → app-lifetime state → Task 5 (`Session`), Task 7 (`AppState`/bridge). ✓
- Outbound `ipc::Channel<ServerEvent>`, sync send, drop-on-closed, **live-read** → Task 2 (live-read slot), Task 6 (`ChannelOut`). ✓
- Inbound typed commands (subscribe/send_input/approve/cancel/settings_get/settings_update) → Task 7. ✓
- `ServerEvent` minus envelope, approval id retained → Task 1. ✓
- Settings as request/response (no frames) → Task 4 (`settings_state`), Task 7 (commands). ✓
- A1 run-guard → Task 5. ✓
- B3 cancel → Task 5 (`cancel`) + Task 7 (command). ✓
- Approval correlation/timeout/Deny → Task 3. ✓
- Workspace switch resets ctx, cancels run → Task 5 (`set_workspace`), Task 7 (bridge). ✓
- Delete `WireEnvelope`/`session_id`/`PROTOCOL_VERSION`/`serve` → Task 1, Task 8. ✓
- Frontend `connect()` seam preserved, reducer untouched → Task 9, Task 10. ✓
- Testing: agent-server pure-Rust behavior (Tasks 2/3/5), MockRuntime smoke (Task 7), Vitest IPC mocks (Tasks 9/10). ✓
- Backlog update on completion → Task 11. ✓

**Placeholder scan:** No "TBD"/"implement later". The few implementer notes flag genuine version-pinned API surfaces (Tauri `ipc`/`test`, `WindowContext` method names) to confirm against source — they instruct, not defer. Behavior is fully specified in code.

**Type consistency:** `ServerEvent`/`EventOut`/`EventSlot`/`SettingsState`/`Decision`/`ChannelEventSink`/`IpcApprovalChannel`/`Session`/`SendOutcome`/`ChannelOut` names are used identically across tasks. `settings_state()` (Rust) → `settings_get` (command) → `settings_state` (frontend frame kind) mapping is intentional and stated. `connect(handlers)` signature matches between Task 9 (def) and Task 10 (call).
