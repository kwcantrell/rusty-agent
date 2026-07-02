# Turn-Atomic Context Curation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Context curation (window eviction in both builds + the compaction split) treats an assistant `tool_calls` message and its `Role::Tool` results as one atomic unit — a built request can never contain an orphaned tool result (mid-session 400) — and plain eviction becomes visible via a new `ContextEvent::Evicted`.

**Architecture:** Four shared `pub(crate)` helpers land in `agent-core/src/context.rs` (`turn_unit_ranges`, `evict_start`, `snap_split_to_unit_boundary`, `orphaned_tool_positions`). Both `build()` impls delegate their eviction cut to `evict_start` (unit-based, newest-first, keep-≥1-unit) with a `debug_assert!` orphan check. `CuratedContext::maintain` snaps its compaction split to a unit boundary (left only) and, on every exit, emits a change-deduped `Evicted{messages, est_tokens}` through the existing ContextEvent plumbing (render/wire/testkit/trace/web). Spec: `docs/superpowers/specs/2026-07-01-turn-atomic-context-curation-design.md`.

**Tech Stack:** Rust (workspace under `agent/`, edition 2021), React/vitest under `web/`.

## Global Constraints

- Run `cargo` from `agent/` (`source ~/.cargo/env` first if missing); clippy must stay clean under `-D warnings`.
- Conventional commits: `type(scope): summary`.
- Invariant (verbatim from spec): a built request never contains a tool result whose parent `tool_calls` message was dropped.
- Snapping only moves **left** (keeps more), never right.
- Keep-at-least-one floor is preserved, now unit-shaped: the newest unit is always kept even if it alone exceeds budget.
- The eviction-visibility check runs on **every** `maintain` exit (the current early `return` in the compaction arm must be restructured away).
- `WindowContext` emits nothing (no sink; test/e2e-only).
- Final gate: `bash scripts/ci.sh` from repo root stays green. Do not commit unless the task's tests pass.

---

### Task 1: Turn-unit helpers in `context.rs`

**Files:**
- Modify: `agent/crates/agent-core/src/context.rs` (helpers after `message_tokens`, ~line 24; tests in the existing `mod tests`)

**Interfaces:**
- Consumes: existing `message_tokens(&Message) -> usize`; `agent_model::{Message, Role}` (add `Role` to the existing `use agent_model::...` import).
- Produces (Tasks 2–4 rely on these exact signatures, all `pub(crate)` in `agent_core::context`):
  - `fn turn_unit_ranges(history: &[Message]) -> Vec<std::ops::Range<usize>>`
  - `fn evict_start(history: &[Message], budget: usize) -> usize`
  - `fn snap_split_to_unit_boundary(history: &[Message], split: usize) -> usize`
  - `fn orphaned_tool_positions(messages: &[Message]) -> Vec<usize>`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `context.rs`. Helper constructors: `Message::user/assistant/tool` exist; an assistant parent with calls is built like the existing `message_tokens_counts_tool_calls_and_reasoning` test:

```rust
    fn parent(id: &str) -> Message {
        Message::assistant(
            "calling",
            Some(vec![agent_tools::ToolCall {
                id: id.into(),
                name: "shell".into(),
                args: serde_json::json!({}),
            }]),
        )
    }
    fn parent2(id1: &str, id2: &str) -> Message {
        Message::assistant(
            "calling two",
            Some(vec![
                agent_tools::ToolCall { id: id1.into(), name: "shell".into(), args: serde_json::json!({}) },
                agent_tools::ToolCall { id: id2.into(), name: "shell".into(), args: serde_json::json!({}) },
            ]),
        )
    }

    #[test]
    fn turn_units_group_parent_with_consecutive_tool_results() {
        let h = vec![
            Message::user("u0"),                          // unit 0..1
            parent("c1"),                                 // unit 1..3
            Message::tool("c1", "shell", "r1"),
            Message::user("u1"),                          // unit 3..4
        ];
        assert_eq!(turn_unit_ranges(&h), vec![0..1, 1..3, 3..4]);
    }

    #[test]
    fn turn_units_parallel_calls_are_one_unit() {
        let h = vec![
            parent2("c1", "c2"),                          // unit 0..3
            Message::tool("c1", "shell", "r1"),
            Message::tool("c2", "shell", "r2"),
        ];
        assert_eq!(turn_unit_ranges(&h), vec![0..3]);
    }

    #[test]
    fn turn_units_stray_tool_is_a_singleton() {
        // Defensive: a Role::Tool with no preceding parent must not panic or
        // mis-attach; it stays a singleton unit.
        let h = vec![Message::tool("cX", "shell", "stray"), Message::user("u")];
        assert_eq!(turn_unit_ranges(&h), vec![0..1, 1..2]);
        assert_eq!(turn_unit_ranges(&[]), Vec::<std::ops::Range<usize>>::new());
    }

    #[test]
    fn evict_start_drops_whole_units_and_keeps_newest_even_over_budget() {
        let h = vec![
            Message::user("old message with padding padding padding"),
            parent("c1"),
            Message::tool("c1", "shell", &"x".repeat(200)),
            Message::user("newest"),
        ];
        // Budget 0: only the newest unit survives (keep-≥1-unit floor).
        assert_eq!(evict_start(&h, 0), 3);
        // Huge budget: everything kept.
        assert_eq!(evict_start(&h, 1_000_000), 0);
        // Budget that fits "newest" + the tool unit but not the old user msg:
        let tool_unit: usize = h[1..3].iter().map(message_tokens).sum();
        let newest = message_tokens(&h[3]);
        assert_eq!(evict_start(&h, tool_unit + newest), 1);
        // One token short of the tool unit: the cut moves to the unit start,
        // never inside it.
        assert_eq!(evict_start(&h, tool_unit + newest - 1), 3);
    }

    #[test]
    fn snap_split_moves_left_to_a_unit_boundary() {
        let h = vec![
            Message::user("u0"),                          // boundary at 1
            parent("c1"),                                 // unit 1..4
            Message::tool("c1", "shell", "r1"),
            Message::tool("c1", "shell", "r2"),
            Message::user("u1"),                          // boundary at 4, 5
        ];
        assert_eq!(snap_split_to_unit_boundary(&h, 4), 4); // exact boundary unchanged
        assert_eq!(snap_split_to_unit_boundary(&h, 3), 1); // mid-unit snaps left
        assert_eq!(snap_split_to_unit_boundary(&h, 2), 1);
        assert_eq!(snap_split_to_unit_boundary(&h, 0), 0); // snap to zero
        assert_eq!(snap_split_to_unit_boundary(&h, 5), 5);
    }

    #[test]
    fn orphan_checker_flags_tool_without_live_parent() {
        let clean = vec![parent("c1"), Message::tool("c1", "shell", "r")];
        assert!(orphaned_tool_positions(&clean).is_empty());
        // Parent evicted → orphan.
        let torn = vec![Message::tool("c1", "shell", "r"), Message::user("u")];
        assert_eq!(orphaned_tool_positions(&torn), vec![0]);
        // A non-tool interloper breaks adjacency → later result is orphaned.
        let interloped = vec![
            parent("c1"),
            Message::tool("c1", "shell", "r1"),
            Message::user("interloper"),
            Message::tool("c1", "shell", "r2"),
        ];
        assert_eq!(orphaned_tool_positions(&interloped), vec![3]);
        // Wrong id → orphaned.
        let wrong = vec![parent("c1"), Message::tool("c9", "shell", "r")];
        assert_eq!(orphaned_tool_positions(&wrong), vec![1]);
    }
```

- [ ] **Step 2: Run to verify compile failure (RED)**

Run: `cd agent && cargo test -p agent-core turn_unit`
Expected: COMPILE FAILURE — the four helpers don't exist yet.

- [ ] **Step 3: Implement the helpers**

In `context.rs`, extend the import to `use agent_model::{Message, ModelClient, Role};` and add after `built_tokens`:

```rust
/// Chronological turn-unit grouping: a message with non-empty `tool_calls`
/// absorbs the consecutive `Role::Tool` messages that follow it; every other
/// message is a singleton unit. Curation (eviction, compaction splits) must
/// keep or drop a unit whole — a `Role::Tool` result serialized without its
/// parent `tool_calls` message 400s on OpenAI-compatible servers.
pub(crate) fn turn_unit_ranges(history: &[Message]) -> Vec<std::ops::Range<usize>> {
    let mut units = Vec::new();
    let mut i = 0;
    while i < history.len() {
        let start = i;
        let is_parent = history[i]
            .tool_calls
            .as_ref()
            .is_some_and(|c| !c.is_empty());
        i += 1;
        if is_parent {
            while i < history.len() && matches!(history[i].role, Role::Tool) {
                i += 1;
            }
        }
        units.push(start..i);
    }
    units
}

/// Index into `history` where the kept window begins for `budget`: walk turn
/// units newest-first, keep whole units while they fit, always keeping at
/// least the newest unit (even if it alone exceeds budget — the keep-≥1
/// floor, unit-shaped).
pub(crate) fn evict_start(history: &[Message], budget: usize) -> usize {
    let units = turn_unit_ranges(history);
    let mut start = history.len();
    let mut used = 0usize;
    for r in units.iter().rev() {
        let t: usize = history[r.clone()].iter().map(message_tokens).sum();
        if used + t > budget && start < history.len() {
            break;
        }
        used += t;
        start = r.start;
    }
    start
}

/// Largest unit boundary `<= split`. Snapping only moves left (keeps more in
/// the recent window), never right.
pub(crate) fn snap_split_to_unit_boundary(history: &[Message], split: usize) -> usize {
    let mut boundary = 0;
    for r in turn_unit_ranges(history) {
        if r.end <= split {
            boundary = r.end;
        } else {
            break;
        }
    }
    boundary
}

/// Positions of `Role::Tool` messages whose `tool_call_id` is not covered by
/// the nearest preceding assistant `tool_calls` block with only `Role::Tool`
/// messages in between — the exact shape OpenAI-compatible servers reject.
pub(crate) fn orphaned_tool_positions(messages: &[Message]) -> Vec<usize> {
    let mut orphans = Vec::new();
    let mut live_ids: std::collections::HashSet<&str> = Default::default();
    for (i, m) in messages.iter().enumerate() {
        if matches!(m.role, Role::Tool) {
            match m.tool_call_id.as_deref() {
                Some(id) if live_ids.contains(id) => {}
                _ => orphans.push(i),
            }
        } else {
            live_ids.clear();
            if let Some(calls) = &m.tool_calls {
                live_ids.extend(calls.iter().map(|c| c.id.as_str()));
            }
        }
    }
    orphans
}
```

- [ ] **Step 4: Run the tests (GREEN)**

Run: `cd agent && cargo test -p agent-core`
Expected: all PASS (new helper tests + existing suite untouched).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/context.rs
git commit -m "feat(core): turn-unit grouping helpers for atomic context curation"
```

---

### Task 2: Unit-based eviction in `WindowContext::build`

**Files:**
- Modify: `agent/crates/agent-core/src/context.rs:137-163` (`WindowContext::build`); tests in the same file

**Interfaces:**
- Consumes: `evict_start`, `orphaned_tool_positions` from Task 1.
- Produces: no API change — `build(&self, model_limit) -> Vec<Message>` behavior becomes unit-atomic.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn build_never_orphans_tool_results() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        ctx.append(Message::user("old old old message with lots of padding text here"));
        ctx.append(parent("c1"));
        ctx.append(Message::tool("c1", "shell", &"y".repeat(120)));
        ctx.append(Message::user("recent"));
        // Budget forces the cut inside the tool turn under the old walk:
        // recent fits, tool result fits, parent does not.
        let tool_result_t = message_tokens(&Message::tool("c1", "shell", &"y".repeat(120)));
        let recent_t = message_tokens(&Message::user("recent"));
        let sys_t = message_tokens(&Message::system("SYS"));
        let limit = sys_t + recent_t + tool_result_t + 2; // parent excluded
        let built = ctx.build(limit);
        assert!(
            orphaned_tool_positions(&built).is_empty(),
            "eviction must drop the torn tool turn whole, got: {:?}",
            built.iter().map(|m| (&m.role, &m.content)).collect::<Vec<_>>()
        );
        // The torn turn was dropped whole: no c1 result without its parent.
        let has_result = built.iter().any(|m| m.tool_call_id.as_deref() == Some("c1"));
        let has_parent = built.iter().any(|m| m.tool_calls.is_some());
        assert_eq!(has_result, has_parent);
    }

    #[test]
    fn window_build_budget_sweep_never_orphans() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        ctx.append(Message::user("intro message with padding"));
        ctx.append(parent("c1"));
        ctx.append(Message::tool("c1", "shell", &"a".repeat(100)));
        ctx.append(Message::user("middle instruction"));
        ctx.append(parent2("c2", "c3"));
        ctx.append(Message::tool("c2", "shell", &"b".repeat(80)));
        ctx.append(Message::tool("c3", "shell", "tiny"));
        ctx.append(Message::user("latest"));
        let total = built_tokens(&ctx.build(usize::MAX)) + 16;
        for limit in 1..=total {
            let built = ctx.build(limit);
            assert!(
                orphaned_tool_positions(&built).is_empty(),
                "orphan at model_limit={limit}"
            );
        }
    }
```

(`parent`/`parent2` are the Task 1 test helpers in the same `mod tests`.)

- [ ] **Step 2: Run to verify failure (RED)**

Run: `cd agent && cargo test -p agent-core build_never_orphans`
Expected: FAIL — the per-message walk keeps the `c1` result while evicting its parent (the orphan checker returns a non-empty list).

- [ ] **Step 3: Rewrite the eviction walk**

Replace the body of `WindowContext::build` (`context.rs:137-163`):

```rust
    fn build(&self, model_limit: usize) -> Vec<Message> {
        let sys_tokens = message_tokens(&self.system);
        let recall_msg = self.recall_message();
        let recall_tokens = recall_msg.as_ref().map(message_tokens).unwrap_or(0);
        let budget = model_limit
            .saturating_sub(sys_tokens)
            .saturating_sub(recall_tokens);
        // Walk history newest-first in turn units, keep whole units while they
        // fit — never split a tool_calls parent from its Role::Tool results.
        let start = evict_start(&self.history, budget);
        let mut out = Vec::with_capacity(self.history.len() - start + 2);
        out.push(self.system.clone());
        if let Some(m) = recall_msg {
            out.push(m);
        }
        out.extend(self.history[start..].iter().cloned());
        debug_assert!(
            orphaned_tool_positions(&out).is_empty(),
            "WindowContext::build produced an orphaned tool message"
        );
        out
    }
```

- [ ] **Step 4: Run the crate suite (GREEN)**

Run: `cd agent && cargo test -p agent-core`
Expected: all PASS — including the pre-existing eviction tests (`build_keeps_system_and_drops_oldest_when_over_limit` etc.; their histories are singleton units, so behavior is unchanged).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/context.rs
git commit -m "fix(core): WindowContext eviction is turn-atomic — never orphans tool results"
```

---

### Task 3: Unit-based eviction + split snapping in `CuratedContext`

**Files:**
- Modify: `agent/crates/agent-core/src/curated.rs:127-146` (`build`) and `:177` (compaction split); tests in the same file

**Interfaces:**
- Consumes: `evict_start`, `snap_split_to_unit_boundary`, `orphaned_tool_positions` from Task 1 (extend the existing `use crate::context::{...}` import).
- Produces: no API change; Task 4 builds on the snapped-split `maintain`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `curated.rs` (reuse the file's `ctx()`, `maint_deps` helpers; add local `parent()` mirroring Task 1's):

```rust
    fn parent(id: &str) -> Message {
        Message::assistant(
            "calling",
            Some(vec![agent_tools::ToolCall {
                id: id.into(),
                name: "shell".into(),
                args: serde_json::json!({}),
            }]),
        )
    }

    #[test]
    fn curated_build_never_orphans_tool_results() {
        let mut c = ctx();
        c.append(Message::user("old old old message with lots of padding text"));
        c.append(parent("c1"));
        c.append(Message::tool("c1", "shell", &"y".repeat(120)));
        c.append(Message::user("recent"));
        use crate::context::{built_tokens, message_tokens, orphaned_tool_positions};
        let tool_result_t = message_tokens(&Message::tool("c1", "shell", &"y".repeat(120)));
        let recent_t = message_tokens(&Message::user("recent"));
        let sys_t = message_tokens(&Message::system("SYS"));
        let limit = sys_t + recent_t + tool_result_t + 2;
        let built = c.build(limit);
        assert!(orphaned_tool_positions(&built).is_empty());
        let _ = built_tokens(&built); // silence unused-import if optimized differently
    }

    #[tokio::test]
    async fn compaction_split_snaps_to_turn_boundary() {
        use crate::context::orphaned_tool_positions;
        let mut c = ctx();
        c.high_water_pct = 0.0; // force compaction
        // keep_recent = 2 lands the naive split between parent and result.
        c.config.keep_recent = 2;
        c.append(Message::assistant("chatter zero with padding".into(), None));
        c.append(Message::assistant("chatter one with padding".into(), None));
        c.append(parent("c1"));
        c.append(Message::tool("c1", "shell", "result one")); // naive split cuts HERE
        c.append(Message::user("newest instruction"));
        let model: Arc<dyn ModelClient> =
            Arc::new(ScriptedModel::new(vec![Scripted::Text("summary".into())]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        // History after compaction has no orphaned tool results...
        assert!(
            orphaned_tool_positions(c.history()).is_empty(),
            "snapped split must keep parent+result together: {:?}",
            c.history().iter().map(|m| (&m.role, &m.content)).collect::<Vec<_>>()
        );
        // ...and the torn turn stayed whole in the recent window.
        let has_result = c.history().iter().any(|m| m.tool_call_id.as_deref() == Some("c1"));
        let has_parent = c.history().iter().any(|m| m.tool_calls.is_some());
        assert!(has_result && has_parent, "torn turn must land wholly in recent");
    }

    #[test]
    fn curated_build_budget_sweep_never_orphans() {
        use crate::context::{built_tokens, orphaned_tool_positions};
        let mut c = ctx();
        c.set_goal("sweep goal".into());
        c.append(Message::user("intro message with padding"));
        c.append(parent("c1"));
        c.append(Message::tool("c1", "shell", &"a".repeat(100)));
        c.append(Message::user("middle instruction"));
        c.append(parent("c2"));
        c.append(Message::tool("c2", "shell", &"b".repeat(80)));
        c.append(Message::user("latest"));
        let total = built_tokens(&c.build(usize::MAX)) + 16;
        for limit in 1..=total {
            let built = c.build(limit);
            assert!(
                orphaned_tool_positions(&built).is_empty(),
                "orphan at model_limit={limit}"
            );
        }
    }
```

Note: `history()` is `#[cfg(test)] pub(crate)` already. If `Message::assistant` takes `&str` (it does — `Message::assistant("hi", None)` in existing tests), drop the `.into()` calls accordingly — match the file's existing call style exactly.

- [ ] **Step 2: Run to verify failure (RED)**

Run: `cd agent && cargo test -p agent-core -- curated_build_never_orphans compaction_split_snaps`
Expected: both FAIL (per-message walk orphans the result; naive split strands the result in the recent window while its parent is summarized away).

- [ ] **Step 3: Implement**

Extend the curated.rs import:

```rust
use crate::context::{
    evict_start, message_tokens, orphaned_tool_positions, recall_block,
    snap_split_to_unit_boundary, ContextManager, MaintCtx, MaintReport,
    DEFAULT_RECALL_TOKEN_BUDGET,
};
```

Replace the `build` body (`curated.rs:127-146`):

```rust
    fn build(&self, model_limit: usize) -> Vec<Message> {
        let pinned = self.pinned();
        let pinned_tokens: usize = pinned.iter().map(message_tokens).sum();
        let budget = model_limit.saturating_sub(pinned_tokens);
        // Walk history newest-first in turn units, keep whole units while they
        // fit — never split a tool_calls parent from its Role::Tool results.
        let start = evict_start(&self.history, budget);
        let mut out = pinned;
        out.extend(self.history[start..].iter().cloned());
        debug_assert!(
            orphaned_tool_positions(&out).is_empty(),
            "CuratedContext::build produced an orphaned tool message"
        );
        out
    }
```

In `maintain`, replace `let split = self.history.len() - self.config.keep_recent;` with:

```rust
            // Snap left to a unit boundary so the cut never separates a
            // tool_calls parent from its results; the torn turn lands wholly
            // in the recent window (keep_recent temporarily keeps a bit more).
            let split = snap_split_to_unit_boundary(
                &self.history,
                self.history.len() - self.config.keep_recent,
            );
```

(`split == 0` makes `history[..split]` empty → `to_summarize` empty → the existing early return exits without a model call; Task 4 restructures that return.)

- [ ] **Step 4: Run the crate suite (GREEN)**

Run: `cd agent && cargo test -p agent-core`
Expected: all PASS, including the existing compaction tests (their histories are assistant/user singletons — snapping is a no-op for them).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/curated.rs
git commit -m "fix(core): CuratedContext eviction + compaction split are turn-atomic"
```

---

### Task 4: `ContextEvent::Evicted` end-to-end

**Files:**
- Modify: `agent/crates/agent-core/src/event.rs:7-21` (new variant)
- Modify: `agent/crates/agent-core/src/curated.rs` (restructure compaction early-return; `last_evicted` field; `emit_eviction`)
- Modify: `agent/crates/agent-core/src/testkit.rs:183-188` (label arm)
- Modify: `agent/crates/agent-cli/src/render.rs:117-127` (notice arm)
- Modify: `agent/crates/agent-server/src/wire.rs:197-221` (mapping arm; test beside the existing context-event tests ~line 363)
- Modify: `agent/crates/agent-runtime-config/src/trace.rs:258-276` (trace arm)
- Modify: `web/src/state.ts:54-62` (`describeContext` case)
- Test: `web/src/state.stats.test.ts` (has a context-frame test pattern at line ~21)

**Interfaces:**
- Consumes: Task 1's `evict_start`; Task 3's snapped `maintain`.
- Produces: `ContextEvent::Evicted { messages: usize, est_tokens: usize }` (exact field names — wire kind string is `"evicted"`, detail keys `"messages"`/`"est_tokens"`).

- [ ] **Step 1: Write the failing Rust tests**

In `curated.rs` `mod tests`:

```rust
    #[tokio::test]
    async fn maintain_emits_evicted_once_per_change() {
        let mut c = ctx();
        c.high_water_pct = 2.0; // compaction off; isolate eviction
        for i in 0..30 {
            c.append(Message::user(format!("filler message number {i} with padding text")));
        }
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let mut deps = maint_deps(&model, &sink, &cancel);
        deps.model_limit = 100; // tiny window → eviction certain
        c.maintain(&deps).await;
        let events = sink.events.lock().unwrap().clone();
        let evicted: Vec<_> = events.iter().filter(|e| e.starts_with("evicted:")).collect();
        assert_eq!(evicted.len(), 1, "one Evicted on first saturated pass: {events:?}");

        // Same state → same count → no duplicate event.
        c.maintain(&deps).await;
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(
            events.iter().filter(|e| e.starts_with("evicted:")).count(),
            1,
            "unchanged eviction count must not re-emit"
        );

        // More history → count changes → re-emit.
        c.append(Message::user("one more message with plenty of padding here"));
        c.maintain(&deps).await;
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(events.iter().filter(|e| e.starts_with("evicted:")).count(), 2);
    }

    #[tokio::test]
    async fn maintain_emits_nothing_under_budget() {
        let mut c = ctx();
        c.append(Message::user("hello"));
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        let events = sink.events.lock().unwrap().clone();
        assert!(
            !events.iter().any(|e| e.starts_with("evicted:")),
            "no Evicted under budget: {events:?}"
        );
    }
```

Check `CollectingSink` in `testkit.rs`: if its events field is not `events: Mutex<Vec<String>>` accessed as above, match the accessor style used by the existing curated.rs tests. `maint_deps` returns a struct with public fields — `deps.model_limit = 100` works because `MaintCtx.model_limit` is `pub`.

In `wire.rs` tests (beside the existing context mapping test ~line 363):

```rust
    #[test]
    fn evicted_context_event_maps_to_wire() {
        let ev = agent_core::AgentEvent::Context(agent_core::ContextEvent::Evicted {
            messages: 7,
            est_tokens: 1234,
        });
        let se = server_event_from(ev).expect("mapped");
        let js = serde_json::to_value(&se).unwrap();
        assert_eq!(js["kind"], "evicted");
        assert_eq!(js["detail"]["messages"], 7);
        assert_eq!(js["detail"]["est_tokens"], 1234);
    }
```

(Match the surrounding tests' exact call pattern for `server_event_from` — copy their setup.)

- [ ] **Step 2: Run to verify failure (RED)**

Run: `cd agent && cargo test -p agent-core maintain_emits`
Expected: COMPILE FAILURE (`ContextEvent::Evicted` doesn't exist).

- [ ] **Step 3: Implement the variant + emit**

`event.rs` — extend `ContextEvent` (doc comment on the enum mentions offload/compaction; update it to "offload / compaction / eviction"):

```rust
    /// Plain window eviction omitted history messages from the built request
    /// (distinct from offload/compaction, which transform rather than drop).
    /// `est_tokens` uses the same estimate the window evicts against.
    Evicted { messages: usize, est_tokens: usize },
```

`curated.rs` — add field to the struct + init in `new()`:

```rust
    /// Message count omitted by eviction at the last maintain pass; dedups
    /// repeated identical Evicted events while the window stays saturated.
    last_evicted: usize,
```

Restructure `maintain` so no path skips the eviction check. Extract the compaction arm into a private method — move the ENTIRE current body of the `if (requested || over_high_water) && ...` block (from `let split = ...` through the `match run_compaction` block) into:

```rust
    /// Compact the old span into the pinned summary. Extracted from `maintain`
    /// so its early exits cannot skip the eviction check that follows.
    async fn compact_old_span(&mut self, deps: &MaintCtx<'_>, report: &mut MaintReport) {
        // ... moved body; `return report;` becomes `return;`
        // and `report.compacted_turns = ...` stays, now via the &mut param.
    }
```

`maintain`'s tail becomes:

```rust
        if (requested || over_high_water) && self.history.len() > self.config.keep_recent + 1 {
            self.compact_old_span(deps, &mut report).await;
        }

        // (c) Eviction visibility — runs on EVERY maintain exit.
        self.emit_eviction(deps);
        report
    }

    /// Emit `ContextEvent::Evicted` when the built window omits history
    /// messages and the count changed since the last pass.
    fn emit_eviction(&mut self, deps: &MaintCtx<'_>) {
        let pinned_tokens: usize = self.pinned().iter().map(message_tokens).sum();
        let budget = deps.model_limit.saturating_sub(pinned_tokens);
        let start = evict_start(&self.history, budget);
        if start > 0 && start != self.last_evicted {
            let est_tokens: usize = self.history[..start].iter().map(message_tokens).sum();
            deps.sink.emit(AgentEvent::Context(ContextEvent::Evicted {
                messages: start,
                est_tokens,
            }));
        }
        self.last_evicted = start;
    }
```

`testkit.rs` — new arm beside the other context labels:

```rust
            AgentEvent::Context(ContextEvent::Evicted { messages, .. }) => {
                format!("evicted:{messages}")
            }
```

`render.rs` — new `CE::Evicted` arm in the context match:

```rust
                    CE::Evicted { messages, est_tokens } =>
                        format!("⟲ evicted {messages} messages (~{est_tokens} tokens)"),
```

`wire.rs` — new arm in the `(kind, detail)` match:

```rust
                CE::Evicted { messages, est_tokens } => (
                    "evicted",
                    serde_json::json!({"messages": messages, "est_tokens": est_tokens}),
                ),
```

`trace.rs` — new arm in the `AgentEvent::Context(c) => match c` block (lines 258-276), using this content with the exact field names/types the neighboring `Offloaded`/`Compacted` arms use for `TraceEvent::Context` (they carry a kind string + a JSON detail; mirror their construction precisely):

```rust
            ContextEvent::Evicted { messages, est_tokens } => TraceEvent::Context {
                kind: "evicted".into(),
                detail: serde_json::json!({"messages": messages, "est_tokens": est_tokens}),
            },
```

If `TraceEvent::Context`'s fields differ from `kind`/`detail` (read the neighboring arms first), keep the `"evicted"` kind string and the `messages`/`est_tokens` keys and adapt only the surrounding field names.

- [ ] **Step 4: Run the Rust suites (GREEN)**

Run: `cd agent && cargo test -p agent-core -p agent-cli -p agent-server -p agent-runtime-config && cargo clippy --workspace -- -D warnings`
Expected: all PASS, clippy clean.

- [ ] **Step 5: Web — failing test, then implement**

Add to `web/src/state.stats.test.ts` (mirror the existing context-frame test at ~line 21 for setup — same `frame(...)` helper):

```ts
it("renders an evicted context marker", () => {
  const s = reduce(initialState([]), {
    type: "frame",
    frame: frame({ type: "context", kind: "evicted",
      detail: { messages: 7, est_tokens: 1234 } }),
  });
  const item = s.items.find((i) => i.kind === "context");
  expect(item?.text).toBe("evicted 7 messages (~1234 tokens)");
});
```

(Adapt the action/reducer invocation to exactly match the file's existing context test — the shape above follows its pattern; copy its imports/helpers.)

Run: `cd web && npx vitest run src/state.stats.test.ts` → FAIL (`describeContext` default case returns `"evicted"`).

Then add the case in `web/src/state.ts` `describeContext`:

```ts
    case "evicted": return `evicted ${detail.messages} messages (~${detail.est_tokens} tokens)`;
```

Run: `cd web && npm test` → all PASS.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-core/src/event.rs agent/crates/agent-core/src/curated.rs agent/crates/agent-core/src/testkit.rs agent/crates/agent-cli/src/render.rs agent/crates/agent-server/src/wire.rs agent/crates/agent-runtime-config/src/trace.rs web/src/state.ts web/src/state.stats.test.ts
git commit -m "feat(core): ContextEvent::Evicted — plain eviction is visible on CLI, wire, trace, and web"
```

---

### Task 5: Full gate + spec status

**Files:**
- Modify: `docs/superpowers/specs/2026-07-01-turn-atomic-context-curation-design.md` (status line)

- [ ] **Step 1: Run the whole CI gate**

Run from repo root: `bash scripts/ci.sh`
Expected: fmt clean, clippy clean (`-D warnings`), all agent tests pass (including `e2e_context_management` / `stress_context_management`), web typecheck + vitest pass. If fmt flags drift, `cd agent && cargo fmt --all` and fold into the commit. Report any real test failure verbatim — do not paper over it.

- [ ] **Step 2: Mark the spec implemented**

Change the spec's `**Status:**` line to: `Implemented (this plan: docs/superpowers/plans/2026-07-01-turn-atomic-context-curation.md)`.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-07-01-turn-atomic-context-curation-design.md
git commit -m "docs(spec): mark turn-atomic context curation spec implemented"
```
