# Audit Drain Cluster 5 — Context, Trace & Config Guards Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Drain audit cluster 5 — cap the pinned goal block (7.1), floor `max_tool_result_bytes` (7.2), record run inputs in the trace (6.1), terminal `trace_disabled` marker (6.2), snapshot ledger segment (7.3), CLI sandbox_mode pin (3.3), allowlist interpreter warn (5.2), /dev-residual corpus rows (5.3).

**Architecture:** Per spec `docs/superpowers/specs/2026-07-07-audit-context-config-guards-design.md`. Six tasks: two context guards in `agent-core`, one new event flowing loop→wire/trace/CLI, one trace-writer marker, one snapshot segment, and a small config/policy trio. No wire frames added (RunStart is None-mapped); no web changes (explorer renders unknown categories generically — verified).

**Tech Stack:** Rust (the `agent/` Cargo workspace ONLY — not src-tauri), serde, tokio tests where present.

## Global Constraints

- Worktree: `.claude/worktrees/audit-context-config-guards`, branch `feature/audit-context-config-guards`. **Immediately after creation, `git reset --hard <local main tip>`** — EnterWorktree branches from stale origin/main (recurring gotcha).
- Before EVERY commit: `git rev-parse --show-toplevel` must print the worktree path. **git reset / rebase are forbidden**; commit on top of HEAD only.
- Every task's verification includes `cargo fmt --all --check` (run from `agent/`) and the named test commands. TDD: write the failing test first, watch it fail, then implement.
- Conventional commits: `type(scope): summary`.
- Line numbers below were live at plan time and drift — re-open the file and match on content before editing.
- Owner-locked values: `GOAL_MAX_TOKENS = 512`; RunStart carries the FULL system prompt text every run (no dedup, no hash).

---

### Task 1: Finding 7.1 — cap the pinned goal block

**Files:**
- Modify: `agent/crates/agent-core/src/curated.rs` (set_goal at ~:206-210; consts near `FOLDED_FACTS_MAX_TOKENS` ~:289; tests at bottom)

**Interfaces:**
- Consumes: `estimate_tokens` / `message_tokens` from `crate::context` (already imported or add to the existing `use` list at curated.rs:5).
- Produces: `const GOAL_MAX_TOKENS: usize = 512` and `const GOAL_TRUNCATION_MARKER: &str` (module-private; Task 5 does not depend on them).

- [ ] **Step 1: Write the failing tests** (in curated.rs `#[cfg(test)] mod`, mirroring the construction used by the existing `set_goal_is_set_once` test at ~:658 — use the same constructor/helper it uses):

```rust
#[test]
fn set_goal_caps_oversized_input_with_marker() {
    let mut c = CuratedContext::new(Message::system("S")); // match set_goal_is_set_once's constructor
    let big = "word ".repeat(3000); // ~3750 est. tokens, well over the 512 cap
    c.set_goal(big);
    let g = c.goal.clone().expect("goal set");
    assert!(g.content.contains("[goal truncated"), "marker missing");
    // cap + marker + "Original goal: " prefix, small slack for overheads
    assert!(
        message_tokens(&g) <= GOAL_MAX_TOKENS + 40,
        "goal block too big: {}",
        message_tokens(&g)
    );
}

#[test]
fn set_goal_under_cap_is_untouched() {
    let mut c = CuratedContext::new(Message::system("S"));
    c.set_goal("ship the feature".into());
    let g = c.goal.clone().unwrap();
    assert_eq!(g.content, "Original goal: ship the feature");
    assert!(!g.content.contains("[goal truncated"));
}

#[test]
fn oversized_first_paste_does_not_wedge_the_window() {
    // The audit-7.1 property: an over-window first paste must not make
    // pinned_tokens() exceed the window forever (budget saturates to 0,
    // second overflow is fatal → permanently wedged session).
    let mut c = CuratedContext::new(Message::system("S"));
    let big = "x".repeat(400_000); // ~100k est. tokens >> the 8192 window below
    c.set_goal(big.clone());
    c.append(Message::user(big));
    let model_limit = 8192;
    assert!(
        c.pinned_tokens() < model_limit / 2,
        "pinned blocks must leave a real history budget, got {}",
        c.pinned_tokens()
    );
    let msgs = c.build(model_limit);
    assert!(!msgs.is_empty(), "build must still produce a request");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd agent && cargo test -p agent-core set_goal_caps -- --nocapture` (and the other two names)
Expected: FAIL — `GOAL_MAX_TOKENS` not found (compile error) — that counts as RED for the two new consts; after adding consts only, the cap test fails on the missing marker.

- [ ] **Step 3: Implement.** Next to `FOLDED_FACTS_MAX_TOKENS` (~:289) add:

```rust
/// Token cap for the pinned goal block (audit 7.1). Same scale as
/// `DEFAULT_RECALL_TOKEN_BUDGET` and `FOLDED_FACTS_MAX_TOKENS` — every pinned
/// block is budgeted. The full input stays in history; only the pin truncates.
const GOAL_MAX_TOKENS: usize = 512;
const GOAL_TRUNCATION_MARKER: &str =
    "… [goal truncated; the full input remains in the conversation history]";
```

Replace `set_goal` (keep the set-once guard):

```rust
fn set_goal(&mut self, goal: String) {
    if self.goal.is_none() {
        // Cap the pin at GOAL_MAX_TOKENS estimated tokens (char-prefix via the
        // chars/4 estimator; char-boundary safe). The marker's own ~15 tokens
        // ride on top of the cap — bounded, and the cap is order-of-magnitude.
        let goal = if estimate_tokens(&goal) > GOAL_MAX_TOKENS {
            let kept: String = goal.chars().take(GOAL_MAX_TOKENS * 4).collect();
            format!("{kept}{GOAL_TRUNCATION_MARKER}")
        } else {
            goal
        };
        self.goal = Some(Message::system(format!("Original goal: {goal}")));
    }
}
```

If `estimate_tokens` is not already in curated.rs's `use crate::context::{...}` list, add it.

- [ ] **Step 4: Run to verify green**

Run: `cargo test -p agent-core` — all three new tests pass, `set_goal_is_set_once` and the full crate stay green.

- [ ] **Step 5: fmt + commit**

```bash
cargo fmt --all --check
git add crates/agent-core/src/curated.rs
git commit -m "fix(context): cap the pinned goal block at 512 tokens (audit 7.1)"
```

---

### Task 2: Finding 7.2 — `max_tool_result_bytes` floor + cap-vs-window warn

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs` (`validate()` ~:355-392, tests below)
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (helper near `prompt_over_budget` ~:93, warn site ~:202-212, tests near `prompt_over_budget_trips_above_a_quarter_of_the_window` ~:508)

**Interfaces:**
- Produces: validate() error string `"max_tool_result_bytes must be >= 1024"`; `pub(crate) fn result_cap_over_budget(cap_bytes: usize, limit: usize) -> bool` in assemble.rs.

- [ ] **Step 1: Write the failing tests.** In runtime_config.rs tests (next to `validate_rejects_zero_max_parallel_tools` ~:788):

```rust
#[test]
fn validate_floors_max_tool_result_bytes() {
    let mut c = RuntimeConfig::from_launch(
        "openai".into(), "http://x".into(), "m".into(), "native".into(), 8192,
    );
    c.max_tool_result_bytes = 0;
    assert!(c.validate().unwrap_err().contains("max_tool_result_bytes"));
    c.max_tool_result_bytes = 1023;
    assert!(c.validate().is_err(), "1023 is below the floor");
    c.max_tool_result_bytes = 1024;
    assert!(c.validate().is_ok(), "1024 is the floor");
}
```

(Mirror the `from_launch` argument shape used by the neighboring validate tests — copy it from `validate_rejects_zero_max_parallel_tools`.)

In assemble.rs tests (next to the quarter-window test ~:508):

```rust
#[test]
fn result_cap_over_budget_trips_above_a_quarter_of_the_window() {
    // est tokens = bytes/4; quarter of window = limit/4. Integer division:
    // the first value that trips is one whole token past, not one byte.
    assert!(!result_cap_over_budget(8192, 8192)); // exactly at: not over
    assert!(!result_cap_over_budget(8195, 8192)); // same token bucket (8195/4 == 2048)
    assert!(result_cap_over_budget(8196, 8192)); // one token past trips (2049 > 2048)
    assert!(!result_cap_over_budget(16 * 1024, 262_144)); // default cap vs big window: quiet
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-runtime-config validate_floors_max_tool_result_bytes result_cap_over_budget`
Expected: FAIL (validate passes a 0 cap today; `result_cap_over_budget` undefined).

- [ ] **Step 3: Implement.** In `validate()` directly after the `max_parallel_tools` guard (~:387-389):

```rust
if self.max_tool_result_bytes < 1024 {
    return Err("max_tool_result_bytes must be >= 1024".into());
}
```

In assemble.rs, next to `prompt_over_budget`:

```rust
/// True when the tool-result ingestion cap's estimated tokens (bytes/4) exceed
/// a quarter of the context window — a cap that big re-opens the
/// single-oversized-result overflow path the ingestion cap exists to close.
/// Advisory only (the caller warns); pure so it can be unit-tested log-free.
pub(crate) fn result_cap_over_budget(cap_bytes: usize, limit: usize) -> bool {
    cap_bytes / 4 > limit / 4
}
```

At the warn site, directly after the composed-system-prompt quarter warn (~:203-212):

```rust
// Advisory (audit 7.2): a tool-result cap comparable to the window re-opens
// the single-oversized-result overflow path. No behavior change.
if result_cap_over_budget(cfg.max_tool_result_bytes, cfg.context_limit) {
    tracing::warn!(
        max_tool_result_bytes = cfg.max_tool_result_bytes,
        context_limit = cfg.context_limit,
        "max_tool_result_bytes estimated tokens exceed a quarter of the context window"
    );
}
```

- [ ] **Step 4: Run to verify green**

Run: `cargo test -p agent-runtime-config`
Expected: PASS, including all pre-existing validate tests (default cap is 16 KiB ≥ 1024 — nothing else trips).

- [ ] **Step 5: fmt + commit**

```bash
cargo fmt --all --check
git add crates/agent-runtime-config/src/runtime_config.rs crates/agent-runtime-config/src/assemble.rs
git commit -m "fix(config): floor max_tool_result_bytes at 1024 + cap-vs-window warn (audit 7.2)"
```

---

### Task 3: Finding 6.1 — run inputs in the trace (`AgentEvent::RunStart`)

**Files:**
- Modify: `agent/crates/agent-core/src/event.rs` (AgentEvent enum ~:59-119)
- Modify: `agent/crates/agent-core/src/context.rs` (ContextManager trait ~:217-235; WindowContext impl ~:271+)
- Modify: `agent/crates/agent-core/src/curated.rs` (ContextManager impl ~:193+)
- Modify: `agent/crates/agent-core/src/loop_.rs` (run_with_cancel entry ~:448-475)
- Modify: `agent/crates/agent-server/src/wire.rs` (`server_event_from` ~:268+, next to the `Approval(_) => return None` arm ~:312)
- Modify: `agent/crates/agent-cli/src/render.rs` (exhaustive event match ~:121-192)
- Modify: `agent/crates/agent-runtime-config/src/trace.rs` (TraceEvent enum ~:180-242, `trace_event` fn ~:244+, tests)

**Interfaces:**
- Produces: `AgentEvent::RunStart { input: String, system: Option<String> }`; `ContextManager::system(&self) -> Option<&Message>` (default `None`); trace record `{"type":"run_start","input":…,"system":…}`.
- NOT touched: `agent-core/src/dispatch.rs` — SubagentSink's `other =>` arm already forwards unknown events to the child trace tap and its inner capture match ends in `_ => {}`; `stats.rs` fold ends in `_ => {}`. Verify both by reading, don't edit.

- [ ] **Step 1: Write the failing tests.**

In trace.rs tests (mirror the existing record/read-back test pattern — tests there create a TraceWriter in a tempdir, record events, and re-read the file):

```rust
#[test]
fn run_start_record_carries_input_and_system() {
    let dir = tempfile::tempdir().unwrap();
    let t = TraceWriter::create(dir.path(), 64).unwrap();
    t.record(&AgentEvent::RunStart {
        input: "fix the bug".into(),
        system: Some("SYSTEM PROMPT".into()),
    });
    let content = std::fs::read_to_string(
        dir.path().join(format!("{}.jsonl", t.session_id())),
    )
    .unwrap();
    let line = content.lines().nth(1).expect("record after header");
    let v: serde_json::Value = serde_json::from_str(line).unwrap();
    assert_eq!(v["event"]["type"], "run_start");
    assert_eq!(v["event"]["input"], "fix the bug");
    assert_eq!(v["event"]["system"], "SYSTEM PROMPT");
}

#[test]
fn child_run_start_joins_via_parent_id() {
    let dir = tempfile::tempdir().unwrap();
    let t = TraceWriter::create(dir.path(), 64).unwrap();
    t.record_child(2, "call-7", &AgentEvent::RunStart {
        input: "child task".into(),
        system: None,
    });
    let content = std::fs::read_to_string(
        dir.path().join(format!("{}.jsonl", t.session_id())),
    )
    .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(content.lines().nth(1).unwrap()).unwrap();
    assert_eq!(v["event"]["type"], "run_start");
    assert_eq!(v["sub"], 2);
    assert_eq!(v["parent_id"], "call-7");
    assert!(v["event"].get("system").is_none(), "None system is omitted");
}
```

In wire.rs tests (or the existing test module for `server_event_from`):

```rust
#[test]
fn run_start_is_not_a_wire_frame() {
    // Old-SPA compat: RunStart must never reach the browser (audit 6.1).
    assert!(server_event_from(AgentEvent::RunStart {
        input: "x".into(),
        system: None,
    })
    .is_none());
}
```

In context.rs tests: `WindowContext::new(Message::system("S")).system()` returns `Some` with content `"S"`; in curated.rs tests the same for CuratedContext. In loop_.rs tests (use the existing Vec-capturing test sink + stub ContextManager pattern the loop tests already use — e.g. the sink used by the SandboxDegraded/Done assertions):

```rust
#[test]
fn run_emits_run_start_with_the_exact_input() {
    // ... construct the loop exactly as the neighboring loop tests do ...
    // run with input "hello world"
    // assert the captured events contain RunStart { input, system } where
    // input == "hello world" and it appears BEFORE any Token/ToolStart event.
}
```

(The stub ContextManagers default `system()` to None — assert `system.is_none()` there; the CuratedContext getter is covered by its own unit test above.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-core -p agent-runtime-config -p agent-server run_start`
Expected: compile FAIL — `RunStart` variant does not exist. That is RED for all of these.

- [ ] **Step 3: Implement, in this order (compiler-guided):**

1. event.rs — add to `AgentEvent` (after `Approval(ApprovalRequest)` or at the end, order is not semantic):

```rust
/// Emitted once at run start with the run's inputs, so a failed top-level
/// turn is replayable from the trace alone and traces can be harvested into
/// eval datasets (audit 6.1). `system` is the composed system prompt as the
/// context manager holds it at run start (None for managers without one).
/// Wire: never forwarded to frontends (server_event_from maps it to None).
RunStart {
    input: String,
    system: Option<String>,
},
```

2. context.rs — trait method with default (after `set_goal`):

```rust
/// The current system message, when the implementation holds one. Read by
/// the loop's run-start trace record; default None so simple/test impls
/// are unaffected.
fn system(&self) -> Option<&Message> {
    None
}
```

WindowContext impl: `fn system(&self) -> Option<&Message> { Some(&self.system) }` — same one-liner in CuratedContext's `impl ContextManager` block.

3. loop_.rs — in `run_with_cancel`, directly after the SandboxDegraded emit block (~:465) and before the retriever block:

```rust
// Record the run's inputs for trace replay / eval harvest (audit 6.1).
// Full user input + composed system prompt, every run (owner call — no
// dedup). Never a wire frame; ObservedSink writes it to the trace.
self.sink.emit(AgentEvent::RunStart {
    input: user_input.clone(),
    system: ctx.system().map(|m| m.content.clone()),
});
```

4. wire.rs — next to the Approval arm: `AgentEvent::RunStart { .. } => return None,`

5. render.rs — new arm in the CLI match: `AgentEvent::RunStart { .. } => {} // trace-only record; nothing to render`

6. trace.rs — TraceEvent variant:

```rust
RunStart {
    input: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
},
```

and in `trace_event`: `AgentEvent::RunStart { input, system } => TraceEvent::RunStart { input, system: system.as_deref() },`

7. Read (do not edit) dispatch.rs's SubagentSink `other =>` arm and stats.rs's `_ => {}` to confirm RunStart flows to the child trace tap and folds as a no-op. If either match is exhaustive without a catch-all, STOP and report — the plan's premise is wrong.

- [ ] **Step 4: Run to verify green**

Run: `cargo test --workspace` (from `agent/`)
Expected: PASS. The compiler will have forced every exhaustive AgentEvent match to declare an arm — that is the point.

- [ ] **Step 5: fmt + commit**

```bash
cargo fmt --all --check
git add crates/agent-core/src/event.rs crates/agent-core/src/context.rs crates/agent-core/src/curated.rs crates/agent-core/src/loop_.rs crates/agent-server/src/wire.rs crates/agent-cli/src/render.rs crates/agent-runtime-config/src/trace.rs
git commit -m "feat(trace): RunStart record with run input + composed system prompt (audit 6.1)"
```

---

### Task 4: Finding 6.2 — terminal `trace_disabled` marker

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/trace.rs` (`write_record` ~:88-127, const near the top, tests)

**Interfaces:**
- Produces: raw JSONL record `{"seq":N,"ts_ms":…,"event":{"type":"trace_disabled","reason":"cap"|"io_error"}}`; `const TRACE_DISABLED_HEADROOM: u64 = 256`.

- [ ] **Step 1: Write the failing test:**

```rust
#[test]
fn cap_breach_writes_terminal_trace_disabled_marker() {
    let dir = tempfile::tempdir().unwrap();
    let t = TraceWriter::create(dir.path(), 1).unwrap(); // 1 MB cap
    let big = "x".repeat(64 * 1024);
    for _ in 0..20 {
        // 20 × 64KB ≈ 1.25MB — guaranteed breach
        t.record(&AgentEvent::Token(big.clone()));
    }
    let content = std::fs::read_to_string(
        dir.path().join(format!("{}.jsonl", t.session_id())),
    )
    .unwrap();
    let last = content.lines().last().unwrap();
    let v: serde_json::Value = serde_json::from_str(last).expect("marker is valid JSON");
    assert_eq!(v["event"]["type"], "trace_disabled");
    assert_eq!(v["event"]["reason"], "cap");
    assert!(
        (last.len() as u64) < TRACE_DISABLED_HEADROOM,
        "marker must fit the reserved headroom, got {}",
        last.len()
    );
    // Disabled means disabled: a later record must not resurrect the file.
    let len_before = content.len();
    t.record(&AgentEvent::Token("after".into()));
    let after = std::fs::read_to_string(
        dir.path().join(format!("{}.jsonl", t.session_id())),
    )
    .unwrap();
    assert_eq!(after.len(), len_before, "no writes after disable");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-runtime-config cap_breach_writes_terminal`
Expected: FAIL — last line is a plain Token record (or `TRACE_DISABLED_HEADROOM` undefined).

- [ ] **Step 3: Implement.** Const near `RETAIN_FILES`/`TRACE_SCHEMA` at the top of trace.rs:

```rust
/// Reserved so the terminal `trace_disabled` marker always fits under the cap.
const TRACE_DISABLED_HEADROOM: u64 = 256;
```

Marker helper (free fn or `impl TraceWriter`, next to `write_record`):

```rust
/// Best-effort terminal marker so a capped/failed trace is distinguishable
/// from a mid-turn crash (audit 6.2). Cap-path headroom is pre-reserved; on
/// the io_error path the write may itself fail — accepted (best-effort by
/// nature). Must not recurse into the cap check.
fn write_disabled_marker(inner: &mut Inner, reason: &str) {
    // Read seq BEFORE borrowing w mutably (same borrow-order note as record()).
    let seq = inner.seq;
    let Some(w) = inner.w.as_mut() else { return };
    let rec = serde_json::json!({
        "seq": seq,
        "ts_ms": epoch_ms(),
        "event": { "type": "trace_disabled", "reason": reason },
    });
    let _ = writeln!(w, "{rec}");
    let _ = w.flush();
}
```

In `write_record`, change the cap check to reserve headroom and write the marker on both disable paths:

```rust
if inner.written + line.len() as u64 + 1 > self.max_bytes.saturating_sub(TRACE_DISABLED_HEADROOM) {
    tracing::warn!(target: "trace", cap_mb = self.max_bytes / (1024 * 1024),
        "trace size cap reached; tracing disabled for this session");
    write_disabled_marker(&mut inner, "cap");
    inner.w = None;
    return;
}
```

(the old explicit flush is subsumed by the marker's flush) — and on the write-failure path:

```rust
if failed {
    tracing::warn!(target: "trace", "trace write failed; tracing disabled for this session");
    write_disabled_marker(&mut inner, "io_error");
    inner.w = None;
    return;
}
```

- [ ] **Step 4: Run to verify green**

Run: `cargo test -p agent-runtime-config`
Expected: PASS, including all pre-existing trace tests (header/0600/retention/record_child).

- [ ] **Step 5: fmt + commit**

```bash
cargo fmt --all --check
git add crates/agent-runtime-config/src/trace.rs
git commit -m "fix(trace): terminal trace_disabled marker before the writer drops (audit 6.2)"
```

---

### Task 5: Finding 7.3 — snapshot gains the ledger segment

**Files:**
- Modify: `agent/crates/agent-core/src/snapshot.rs` (`build_snapshot` ~:32-101, tests)
- Modify: `agent/crates/agent-core/src/curated.rs` (`snapshot()` ~:178-189)

**Interfaces:**
- Consumes: `CuratedContext::folded_block()` and `folded_facts: Vec<String>` (existing, curated.rs ~:146).
- Produces: `build_snapshot` gains two params after `goal`: `ledger: Option<&Message>, ledger_items: &[String]`; new segment `category: "ledger"`. All existing `build_snapshot` test call sites gain `None, &[]`.
- Web: NO change — ContextExplorer.tsx renders unknown categories with a fallback color (`COLORS[s.category] ?? "var(--text-muted)"`, verified at plan time).

- [ ] **Step 1: Write the failing tests.** In snapshot.rs tests:

```rust
#[test]
fn ledger_block_becomes_ledger_segment() {
    let ledger = Message::system("Ledger of earlier user instructions…\n1. port = 8080\n2. name = zephyr");
    let facts = vec!["port = 8080".to_string(), "name = zephyr".to_string()];
    let snap = build_snapshot(
        1,
        1000,
        &Message::system("S"),
        Some(&Message::system("Original goal: g")),
        Some(&ledger),
        &facts,
        &[],
        DEFAULT_RECALL_TOKEN_BUDGET,
        None,
        &[],
    );
    let cats: Vec<&str> = snap.segments.iter().map(|s| s.category.as_str()).collect();
    assert_eq!(cats, vec!["system", "goal", "ledger", "messages"]);
    let ledger_seg = snap.segments.iter().find(|s| s.category == "ledger").unwrap();
    assert_eq!(ledger_seg.est_tokens, message_tokens(&ledger));
    assert_eq!(ledger_seg.count, 2);
    assert!(ledger_seg.items[0].contains("port = 8080"));
    assert_eq!(
        snap.est_total,
        snap.segments.iter().map(|s| s.est_tokens).sum::<usize>()
    );
}

#[test]
fn no_ledger_segment_without_folded_facts() {
    let snap = build_snapshot(
        1, 1000, &Message::system("S"), None, None, &[], &[],
        DEFAULT_RECALL_TOKEN_BUDGET, None, &[],
    );
    assert!(snap.segments.iter().all(|s| s.category != "ledger"));
}
```

In curated.rs tests — an integration pin that the snapshot matches the budget math:

```rust
#[test]
fn snapshot_est_total_includes_the_pinned_ledger() {
    // Build a CuratedContext whose folded_facts is non-empty the same way the
    // existing fold tests do (reuse their setup — e.g. the test around
    // curated.rs:974 that asserts folded_block() <= FOLDED_FACTS_MAX_TOKENS),
    // then assert:
    //   snapshot(model_limit, 1).est_total
    //     == c.pinned_tokens() + history message_tokens sum
    // and that a "ledger" segment exists with count == folded_facts.len().
}
```

(Write the body concretely by copying the fold-test setup found in curated.rs — the reviewer checks the assertion, not the setup style.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-core ledger`
Expected: compile FAIL — `build_snapshot` has no ledger params.

- [ ] **Step 3: Implement.** snapshot.rs — extend the signature (keep the `#[allow(clippy::too_many_arguments)]`):

```rust
pub(crate) fn build_snapshot(
    turn: usize,
    model_limit: usize,
    system: &Message,
    goal: Option<&Message>,
    ledger: Option<&Message>,
    ledger_items: &[String],
    recall: &[String],
    recall_budget: usize,
    compaction_summary: Option<&Message>,
    history: &[Message],
) -> ContextSnapshot {
```

After the goal segment push:

```rust
// The folded-facts ledger is pinned (it rides inside the goal block in
// pinned()) and charged in pinned_tokens() as its OWN message — a separate
// segment here keeps est_total equal to the budget math (audit 7.3).
if let Some(l) = ledger {
    segments.push(ContextSegment {
        category: "ledger".into(),
        est_tokens: message_tokens(l),
        items: ledger_items.iter().map(|f| preview(f, 100)).collect(),
        count: ledger_items.len(),
    });
}
```

curated.rs `snapshot()`:

```rust
pub fn snapshot(&self, model_limit: usize, turn: usize) -> crate::ContextSnapshot {
    let ledger = (!self.folded_facts.is_empty()).then(|| self.folded_block());
    crate::snapshot::build_snapshot(
        turn,
        model_limit,
        &self.system,
        self.goal.as_ref(),
        ledger.as_ref(),
        &self.folded_facts,
        &self.recall,
        self.recall_budget,
        self.compaction_summary.as_ref(),
        &self.history,
    )
}
```

Fix every existing `build_snapshot(` test call site by inserting `None, &[]` after the goal argument.

- [ ] **Step 4: Run to verify green**

Run: `cargo test -p agent-core`
Expected: PASS including all pre-existing snapshot tests.

- [ ] **Step 5: fmt + commit**

```bash
cargo fmt --all --check
git add crates/agent-core/src/snapshot.rs crates/agent-core/src/curated.rs
git commit -m "fix(context): snapshot reports the pinned folded-facts ledger (audit 7.3)"
```

---

### Task 6: Findings 3.3 (test pin) + 5.2 (interpreter warn) + 5.3 (corpus rows)

**Files:**
- Modify: `agent/crates/agent-cli/src/main.rs` (tests ~:390-410; warnings print after the validate gate ~:224-227)
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs` (new `warnings()` near `validate()`, tests)
- Modify: `agent/crates/agent-server/src/runtime.rs` (`apply()` after `cfg.validate()?` ~:144)
- Modify: `agent/crates/agent-runtime-config/tests/policy_corpus.tsv` (/dev class block, after the `ask … /dev/shm/f` row ~:52)

**Interfaces:**
- Produces: `pub fn warnings(&self) -> Vec<String>` on RuntimeConfig. Nothing downstream consumes it beyond the two surfacing sites.

- [ ] **Step 1: Write the failing tests.**

agent-cli main.rs (next to `cli_bad_claude_effort_fails_validate` ~:402):

```rust
#[test]
fn cli_bad_sandbox_mode_fails_validate() {
    // Audit 3.3 pin: the gate itself shipped in claude-cli-followups
    // (rt.validate() + exit 2); this pins that a typo'd mode trips it.
    let cli = Cli::parse_from(["agent-cli", "--sandbox-mode", "enfore"]);
    let rc = runtime_config_from_cli(&cli, "prompted");
    let err = rc.validate().unwrap_err();
    assert!(err.contains("sandbox_mode"), "got: {err}");
}
```

runtime_config.rs tests:

```rust
#[test]
fn warnings_flag_interpreters_in_command_allowlist() {
    let mut c = RuntimeConfig::from_launch(
        "openai".into(), "http://x".into(), "m".into(), "native".into(), 8192,
    );
    for entry in ["bash", "/bin/bash", "xargs -0"] {
        c.command_allowlist = vec![entry.to_string()];
        let w = c.warnings();
        assert_eq!(w.len(), 1, "'{entry}' must warn");
        assert!(w[0].contains(entry));
    }
    for entry in ["git status", "cargo build", "ls"] {
        c.command_allowlist = vec![entry.to_string()];
        assert!(c.warnings().is_empty(), "'{entry}' must not warn");
    }
}

#[test]
fn default_config_is_warning_free() {
    // Guard against warn fatigue: a plain run must print nothing.
    let mut c = RuntimeConfig::from_launch(
        "openai".into(), "http://x".into(), "m".into(), "native".into(), 8192,
    );
    c.command_allowlist = crate::default_allowlist();
    assert!(c.warnings().is_empty());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-cli cli_bad_sandbox_mode; cargo test -p agent-runtime-config warnings_flag`
Expected: 3.3 pin PASSES immediately (the gate already exists — that is fine and expected; note it in the report); the warnings tests FAIL to compile (`warnings()` undefined).

- [ ] **Step 3: Implement 5.2.** In runtime_config.rs, next to `validate()`:

```rust
/// Leading tokens that hollow the hard floor if allowlisted: a quoted wrapper
/// (`bash -c "sudo …"`) is a single token the command scanner cannot see
/// (agent-policy command.rs KNOWN LIMITATION).
const INTERPRETER_TOKENS: [&str; 8] = ["bash", "sh", "zsh", "dash", "ksh", "eval", "xargs", "env"];

/// Advisory config lints (audit 5.2). Non-fatal by owner decision —
/// `validate()` stays reject-only; frontends surface these (CLI stderr,
/// server tracing::warn).
pub fn warnings(&self) -> Vec<String> {
    let mut out = Vec::new();
    for entry in &self.command_allowlist {
        let Some(first) = entry.split_whitespace().next() else {
            continue;
        };
        let base = first.rsplit('/').next().unwrap_or(first);
        if INTERPRETER_TOKENS.contains(&base) {
            out.push(format!(
                "command_allowlist entry '{entry}' auto-allows a shell interpreter / \
                 exec vehicle: wrappers like `{base} -c \"…\"` bypass the command \
                 scanner (command.rs KNOWN LIMITATION) — remove it unless deliberate"
            ));
        }
    }
    out
}
```

(Place `INTERPRETER_TOKENS` as a module-level `const` outside the impl if that matches file style.)

Surfacing — agent-cli main.rs, directly after the validate gate:

```rust
for w in rt.warnings() {
    eprintln!("warning: {w}");
}
```

agent-server runtime.rs `apply()`, directly after `cfg.validate()?;`:

```rust
for w in cfg.warnings() {
    tracing::warn!(target: "config", "{w}");
}
```

- [ ] **Step 4: Implement 5.3.** Append to the /dev class block in policy_corpus.tsv (after the `ask	echo x > /dev/shm/f` row), TAB-separated exactly like neighbors:

```
ask	tee /dev/sda	# non-redirect write vehicle: documented dev-redirect residual — reaches Ask, not Deny
ask	cp /tmp/x /dev/sda	# non-redirect write vehicle (`cp` un-allowlisted → Ask)
ask	echo x > $DEV/sda	# variable-expansion target: `$` is SHELL_SIGNIFICANT; deny handler sees only unexpanded text → Ask
ask	dd of=$DEV	# variable-expansion target in dd `of=` → Ask
ask	cd /dev && echo x > sda	# cwd-relative redirect: target never literally names /dev → Ask
```

Run the corpus FIRST and treat any failure as a live posture drift to report, NOT an expectation to edit:

Run: `cargo test -p agent-runtime-config --test policy_corpus`
Expected: PASS (all five rows Ask through the production engine wiring).

- [ ] **Step 5: Run everything, fmt + commit**

Run: `cargo test -p agent-cli -p agent-runtime-config -p agent-server && cargo fmt --all --check`

```bash
git add crates/agent-cli/src/main.rs crates/agent-runtime-config/src/runtime_config.rs crates/agent-server/src/runtime.rs crates/agent-runtime-config/tests/policy_corpus.tsv
git commit -m "feat(policy): allowlist interpreter warnings + /dev residual corpus rows + sandbox_mode CLI pin (audit 5.2/5.3/3.3)"
```

---

### Final phase (controller, not a subagent task)

- [ ] Full gate: `bash scripts/ci.sh` from the repo root of the WORKTREE — green.
- [ ] Whole-branch review (fable) over `main..HEAD`.
- [ ] Fix waves as needed, re-review.
- [ ] Merge `--no-ff` to main, post-merge ci.sh on main, remove worktree, delete branch.
- [ ] **Post-merge champion guard sweep (spec §1, mandatory):** re-run the context-evolve paired guard sweep with `champion_k10.json` — ceilings are (config, rate) pairs. Expected no-op; the sweep is the evidence.
- [ ] Re-stamp `.agents/skills/harness-engineering/audit.md` for findings 7.1/7.2/6.1/6.2/7.3/3.3/5.2/5.3 (3.3: closed independently by claude-cli-followups, cluster adds the pin).
- [ ] Ledger section in `.superpowers/sdd/progress.md` + memory updates (`harness-sdlc-audit-2026-07.md`, MEMORY.md).
