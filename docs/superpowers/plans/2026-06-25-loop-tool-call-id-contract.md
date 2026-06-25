# B1 — Collision-Proof Tool-Call Result Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make duplicate or empty model-supplied tool-call ids unable to crash the agent or produce a malformed transcript, and fix the `message_tokens` undercount.

**Architecture:** Enforce the id invariant (unique, non-empty per turn) at the single point it originates for all protocols — right after `protocol.parse()` in the loop — then make the Phase-3 drain panic-proof as defense-in-depth. Separately, extend the context token estimate to count `reasoning` and `tool_calls`. All changes are in `agent-core` (`loop_.rs`, `context.rs`).

**Tech Stack:** Rust, `tokio`, `futures`, `serde_json`. Test harness: `crate::testkit` (`ScriptedModel`, `Scripted::Calls`, `PassthroughProtocol`, `CollectingSink`, `AlwaysApprove`), `tempfile`.

## Global Constraints

- TDD: write the failing test first, watch it fail, then the minimal fix.
- Run tests from the Rust workspace root: `cd agent` first (the workspace `Cargo.toml` is at `agent/Cargo.toml`, not the repo root). `source ~/.cargo/env` if cargo is not on PATH.
- Single crate: `agent-core`. Do **not** modify `agent-model/src/protocol.rs` — the chokepoint approach is protocol-agnostic by design.
- Preserve existing behavior: every existing test in `loop_.rs` and `context.rs` stays green.
- Determinism: the id normalizer must not use clocks or randomness.
- Out of scope: B2 (OpenAI stream robustness) and B3 (cancellation wiring).

## Reference — confirmed types (do not redefine)

- `agent_tools::ToolCall { id: String, name: String, args: serde_json::Value }`
- `agent_model::Message { role: Role, content: String, tool_calls: Option<Vec<ToolCall>>, tool_call_id: Option<String>, name: Option<String>, reasoning: Option<String> }`; `Role::{System,User,Assistant,Tool}`; `Message::assistant(impl Into<String>, Option<Vec<ToolCall>>)`, `Message::tool(call_id, name, content)`, `.with_reasoning(impl Into<String>)`.
- `context::estimate_tokens(&str) -> usize`; `context::message_tokens(&Message) -> usize` (private; tests live in the same module).
- In `loop_.rs` `run()`: `let parsed = match self.protocol.parse(&assistant) {...};` is at ~line 173-186; the assistant message is built at ~188 from `parsed.tool_calls.clone()`; Phase-3 drain `results.remove(&id).expect(...)` is at ~240-242.

---

### Task 1: Collision-proof the tool-call id contract (`loop_.rs`)

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` — add `normalize_tool_call_ids` (free fn near `merge_tool_call`, ~line 343), call it in `run()` after parse (~186), and replace the Phase-3 drain (~240-242).
- Test: `agent/crates/agent-core/src/loop_.rs` (the existing `#[cfg(test)] mod tests`).

**Interfaces:**
- Produces: `fn normalize_tool_call_ids(calls: &mut [ToolCall])` — rewrites only empty or duplicate ids to deterministic, unique, order-stable values; leaves already-unique ids untouched.
- Consumes (tests): `ScriptedModel`, `Scripted::{Calls,Text}`, `PassthroughProtocol`, `CollectingSink`, `AlwaysApprove`, `registry()`, `policy()`, `AgentLoop::new`, `LoopConfig`, `WindowContext` — all already in scope in the test module.

- [ ] **Step 1: Write the failing unit tests for the normalizer**

Add to `mod tests` in `loop_.rs`. Add `ToolCall` to the test imports — change `use agent_tools::{fs::ReadFile, ToolRegistry};` to `use agent_tools::{fs::ReadFile, ToolCall, ToolRegistry};`.

```rust
fn tc(id: &str) -> ToolCall {
    ToolCall { id: id.into(), name: "read_file".into(), args: serde_json::json!({}) }
}

#[test]
fn normalize_ids_makes_empty_and_duplicate_ids_unique() {
    let mut calls = vec![tc(""), tc(""), tc("x"), tc("x")];
    normalize_tool_call_ids(&mut calls);
    let ids: Vec<&str> = calls.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids.len(), 4);
    assert!(ids.iter().all(|s| !s.is_empty()), "no empty ids: {ids:?}");
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), 4, "all ids distinct: {ids:?}");
    assert_eq!(ids[2], "x", "an already-unique id is left intact when first seen");
}

#[test]
fn normalize_ids_synthetic_avoids_collision_with_model_supplied_literal() {
    // id-less first call AND a model literally sending "call_0" -> still distinct.
    let mut calls = vec![tc(""), tc("call_0")];
    normalize_tool_call_ids(&mut calls);
    assert_ne!(calls[0].id, calls[1].id, "synthetic id must not collide: {:?}",
        calls.iter().map(|c| c.id.clone()).collect::<Vec<_>>());
    assert!(!calls[0].id.is_empty() && !calls[1].id.is_empty());
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd agent && cargo test -p agent-core normalize_ids`
Expected: FAIL to compile with "cannot find function `normalize_tool_call_ids`".

- [ ] **Step 3: Implement the normalizer**

Add this free function in `loop_.rs` (next to `merge_tool_call`, outside the `tests` module):

```rust
/// Guarantee every tool call in a turn has a unique, non-empty id. Model-supplied
/// ids are passed through verbatim by the protocols, so a model can send duplicate
/// or empty ids; the per-call result contract (one `order` entry + one `results`
/// slot per call) requires uniqueness. Rewrites only offending ids, order-stable
/// and deterministically (no clock/random), and bumps the synthetic id if it would
/// collide with a literal the model also supplied.
fn normalize_tool_call_ids(calls: &mut [ToolCall]) {
    let mut seen = std::collections::HashSet::new();
    for (i, c) in calls.iter_mut().enumerate() {
        if c.id.is_empty() || !seen.insert(c.id.clone()) {
            let mut candidate = format!("call_{i}");
            let mut n = 1;
            while !seen.insert(candidate.clone()) {
                candidate = format!("call_{i}_{n}");
                n += 1;
            }
            c.id = candidate;
        }
    }
}
```

- [ ] **Step 4: Run the unit tests to verify they pass**

Run: `cd agent && cargo test -p agent-core normalize_ids`
Expected: PASS (both tests).

- [ ] **Step 5: Write the failing loop regression test (the crash proof)**

Add to `mod tests` in `loop_.rs`. This drives the chokepoint wiring + drain hardening.

```rust
#[tokio::test]
async fn duplicate_tool_call_ids_do_not_panic_and_yield_distinct_tool_ids() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "BODY").unwrap();
    let ws = dir.path().to_path_buf();
    // Two calls share id "c1" — collides under the order/results contract and
    // panics the Phase-3 drain on current code.
    let model = Arc::new(ScriptedModel::new(vec![
        Scripted::Calls(vec![
            ("c1".into(), "read_file".into(), r#"{"path":"a.txt"}"#.into()),
            ("c1".into(), "read_file".into(), r#"{"path":"a.txt"}"#.into()),
        ]),
        Scripted::Text("done".into()),
    ]));
    let sink = Arc::new(CollectingSink::default());
    let agent = AgentLoop::new(
        model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
        Arc::new(AlwaysApprove), sink.clone(),
        LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 2, temperature: 0.0,
            max_tokens: None, workspace: ws, tool_timeout: std::time::Duration::from_secs(5),
            stream_idle_timeout: std::time::Duration::from_secs(60), ..Default::default() });
    let mut ctx = WindowContext::new(Message::system("sys"));

    // Must NOT panic.
    agent.run(&mut ctx, "read twice".into()).await.unwrap();

    // Both calls produced a result — the second was not dropped by a collision.
    let events = sink.events.lock().unwrap().clone();
    assert_eq!(events.iter().filter(|e| *e == "tool_result:read_file").count(), 2);

    // The transcript carries two DISTINCT tool ids.
    let built = ctx.build(100_000);
    let tool_ids: Vec<String> = built.iter()
        .filter(|m| matches!(m.role, agent_model::Role::Tool))
        .map(|m| m.tool_call_id.clone().unwrap_or_default())
        .collect();
    assert_eq!(tool_ids.len(), 2, "two tool messages expected: {tool_ids:?}");
    assert_ne!(tool_ids[0], tool_ids[1], "duplicate ids must normalize to distinct");
}
```

- [ ] **Step 6: Run it to verify it fails (panics) on current wiring**

Run: `cd agent && cargo test -p agent-core duplicate_tool_call_ids -- --nocapture`
Expected: FAIL — panic `every gated call id has a result` from the Phase-3 drain (the normalizer exists but isn't wired into `run()` yet).

- [ ] **Step 7: Wire the normalizer into `run()` and harden the drain**

In `loop_.rs` `run()`, change the parse binding to `mut` and normalize immediately after. Find:

```rust
            let parsed = match self.protocol.parse(&assistant) {
```

Change to `let mut parsed = match self.protocol.parse(&assistant) {`, and immediately after the closing `};` of that match (before `let mut msg = Message::assistant(...)` at ~188) insert:

```rust
            // Enforce the per-call id invariant for EVERY protocol before the ids
            // feed the assistant message and the Phase-3 tool-result drain.
            normalize_tool_call_ids(&mut parsed.tool_calls);
```

Then replace the Phase-3 drain (~240-242):

```rust
            for id in order {
                let (name, resolved) = results.remove(&id)
                    .expect("every gated call id has a result");
```

with the panic-proof form:

```rust
            for id in order {
                // Normalization guarantees a slot per id; if that invariant is ever
                // violated, drop the result rather than crash on untrusted input.
                let (name, resolved) = match results.remove(&id) {
                    Some(v) => v,
                    None => continue,
                };
```

(The body below — `let content = match resolved {...}; ctx.append(Message::tool(id, name, content));` — is unchanged.)

- [ ] **Step 8: Run the regression test + full crate suite**

Run: `cd agent && cargo test -p agent-core`
Expected: PASS — `duplicate_tool_call_ids_...` passes, both `normalize_ids_...` pass, and every existing `loop_.rs` test (`runs_tool_then_finishes`, `denied_tool_feeds_error_back_and_continues`, `scripted_calls_yields_multiple_native_tool_calls`, the `merge_tool_call_*` tests, etc.) stays green.

- [ ] **Step 9: Commit**

```bash
cd agent && git add crates/agent-core/src/loop_.rs
git commit -m "fix(loop): collision-proof tool-call id contract

Normalize duplicate/empty model-supplied tool-call ids at the single
post-parse chokepoint (protocol-agnostic) so the order/results contract
can't panic results.remove().expect() or append duplicate tool_call_ids.
Harden the Phase-3 drain to skip rather than panic as defense-in-depth."
```

---

### Task 2: Count tool_calls and reasoning in `message_tokens` (`context.rs`)

**Files:**
- Modify: `agent/crates/agent-core/src/context.rs:9-11` (`message_tokens`)
- Test: `agent/crates/agent-core/src/context.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `Message::assistant`, `.with_reasoning`, `agent_tools::ToolCall`, `estimate_tokens` — all in scope (`mod tests` already has `use super::*;` and `use agent_model::{Message, Role};`).
- Produces: no signature change; `message_tokens` now also counts `reasoning` and `tool_calls`.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `context.rs`:

```rust
#[test]
fn message_tokens_counts_tool_calls_and_reasoning() {
    let plain = Message::assistant("hi", None);
    let heavy = Message::assistant(
        "hi",
        Some(vec![agent_tools::ToolCall {
            id: "c1".into(),
            name: "read_file".into(),
            args: serde_json::json!({"path": "some/long/path/to/a/file/name.txt"}),
        }]),
    )
    .with_reasoning("a fairly long chain of reasoning that should add tokens");
    assert!(
        message_tokens(&heavy) > message_tokens(&plain),
        "tool_calls + reasoning must increase the estimate"
    );
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd agent && cargo test -p agent-core message_tokens_counts`
Expected: FAIL — current `message_tokens` ignores both fields, so `heavy == plain` (both `estimate_tokens("hi") + 4`) and the `>` assertion fails.

- [ ] **Step 3: Implement the fix**

Replace `message_tokens` in `context.rs`:

```rust
fn message_tokens(m: &Message) -> usize {
    let mut t = estimate_tokens(&m.content) + 4; // per-message overhead
    if let Some(r) = &m.reasoning {
        t += estimate_tokens(r);
    }
    if let Some(calls) = &m.tool_calls {
        for c in calls {
            t += estimate_tokens(&c.name) + estimate_tokens(&c.args.to_string());
        }
    }
    t
}
```

- [ ] **Step 4: Run the test + full crate suite**

Run: `cd agent && cargo test -p agent-core`
Expected: PASS — the new test passes and all existing `context.rs` tests stay green (`built_tokens_sums_per_message_estimate`, `build_keeps_system_and_drops_oldest_when_over_limit`, the recall/eviction tests — none of those messages carry `reasoning`/`tool_calls`, so their estimates are unchanged).

- [ ] **Step 5: Commit**

```bash
cd agent && git add crates/agent-core/src/context.rs
git commit -m "fix(context): count tool_calls and reasoning in message_tokens

The window-eviction estimate ignored Message.reasoning and Message.tool_calls,
undercounting assistant turns with large tool-call args or preserved reasoning."
```

---

### Task 3: Final verification

**Files:** none (verification only)

- [ ] **Step 1: Build and test the crate**

Run: `cd agent && cargo test -p agent-core`
Expected: PASS, no warnings about unused imports/functions.

- [ ] **Step 2: Workspace build + downstream sweep**

Run: `cd agent && cargo build && cargo test -p agent-cli -p agent-server -p agent-runtime-config`
Expected: PASS — the only behavior changes are id normalization (internal to the loop) and a higher token estimate; no public signature changed.

- [ ] **Step 3: Confirm the spec's testing checklist is satisfied**

Cross-check against `docs/superpowers/specs/2026-06-25-loop-tool-call-id-contract-design.md` → "Testing": normalizer unit tests (uniqueness + synthetic-collision edge), the loop crash-proof regression (no panic + distinct ids), and the `message_tokens` test. All present and passing. If any is missing, add it before finishing.
