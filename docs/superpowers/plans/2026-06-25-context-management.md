# Advanced Context Management Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a curating context manager that auto-offloads stale tool errors / large tool outputs into a retrievable table, compacts an over-full window into a high-fidelity summary, and re-grounds the model on the original goal — without breaking the existing sliding-window behavior.

**Architecture:** Keep the sync `ContextManager` trait + `WindowContext` as the windowing primitive. Add one defaulted async `maintain` method to the trait, plus a new `CuratedContext` impl that layers an `OffloadStore` (pluggable, in-memory v1), a pure `OffloadPolicy`, and an async `Compactor`. The loop calls `ctx.maintain(...)` once per turn after tool results are appended. The model gets two tools (`context_recall`, `context_compact`) for agency.

**Tech Stack:** Rust, `async_trait`, `tokio`, `futures`, `tokio_util::sync::CancellationToken`, `serde_json`, `thiserror`, `tracing`.

## Global Constraints

- All new code lives in `agent-core` except where a task says otherwise; the two model-facing tools also live in `agent-core` (they depend on `OffloadStore`, which `agent-tools` cannot import without a dependency cycle).
- `agent-core` dependencies already available: `agent-tools`, `agent-policy`, `agent-model`, `async-trait`, `serde_json`, `futures`, `tokio`, `tokio-util`, `thiserror`, `tracing`. Do **not** add new crate dependencies.
- Token estimation reuses the existing `estimate_tokens` / `message_tokens` in `agent-core/src/context.rs` (~4 chars/token). Do not introduce a new tokenizer.
- Curation is **best-effort**: a failure in `maintain` must never break the turn. Windowing in `build()` remains the floor that provably fits `model_limit`.
- Offload **only** rewrites a `tool_result` message's `content`; it never drops a message and never touches an assistant message's `tool_calls`. The `tool_call_id` and `name` are always preserved.
- Error content is detected by the existing convention: tool-error results start with `"ERROR: "` (`loop_.rs:267`).
- TDD: every task writes the failing test first, watches it fail, implements minimally, watches it pass, commits.
- Run all tests with the cargo env sourced: `source ~/.cargo/env` first (cargo is not on PATH by default in this repo).

## Deviations from the spec (deliberate)

- The spec's `OffloadConfig` listed both `stale_after_turns` and `keep_recent`. This plan uses a **single** staleness rule — `keep_recent` (protect the N most-recent tool results; older ones are eligible) — to keep one clear, testable rule. `stale_after_turns` is dropped (YAGNI).
- Context events are surfaced through a single `AgentEvent::Context(ContextEvent)` variant (Task 4) rather than three top-level variants, to minimize the blast radius across exhaustive `AgentEvent` match sites.

## File Structure

- Create `agent-core/src/offload.rs` — `OffloadId`, `OffloadKind`, `OffloadEntry`, `OffloadStore` trait, `InMemoryOffloadStore`.
- Create `agent-core/src/offload_policy.rs` — `OffloadConfig`, `OffloadHit`, `select_offloads`, `placeholder_for`.
- Create `agent-core/src/curated.rs` — `CuratedContext`, `MaintCtx`, `MaintReport`.
- Create `agent-core/src/compactor.rs` — `CompactError`, `run_compaction`, `collect_stream`.
- Create `agent-core/src/context_tools.rs` — `ContextRecallTool`, `ContextCompactTool`, `context_tools()` helper.
- Modify `agent-core/src/context.rs` — make `message_tokens` `pub(crate)`; extract `recall_block`; add `#[async_trait]`, `set_goal`, and `maintain` (defaulted) to the `ContextManager` trait.
- Modify `agent-core/src/event.rs` — add `ContextEvent` enum + `AgentEvent::Context(ContextEvent)`.
- Modify `agent-core/src/lib.rs` — add `mod offload; mod offload_policy; mod curated; mod compactor; mod context_tools;` and re-export.
- Modify `agent-core/src/loop_.rs` — call `ctx.maintain(...)` once per turn.
- Modify `agent-cli/src/render.rs` and `agent-server/src/wire.rs` — handle the new `AgentEvent::Context` arm.
- Modify `agent-runtime-config/src/assemble.rs` — create the shared store + compact flag, register the two tools, expose them on `BuiltLoop`.
- Modify `agent-cli/src/main.rs` and `agent-server/src/session.rs` — construct `CuratedContext` with the shared store + flag.
- Create `.agents/skills/context-management/SKILL.md` — guidance skill.

---

### Task 1: Offload store

**Files:**
- Create: `agent/crates/agent-core/src/offload.rs`
- Modify: `agent/crates/agent-core/src/lib.rs`

**Interfaces:**
- Produces: `pub type OffloadId = u64;`
  `pub enum OffloadKind { Error, Output }` (derives `Debug, Clone, PartialEq, Eq`)
  `pub struct OffloadEntry { pub id: OffloadId, pub tool_call_id: String, pub tool_name: String, pub kind: OffloadKind, pub content: String, pub bytes: usize, pub turn: usize }`
  `pub trait OffloadStore: Send + Sync { fn put(&self, entry: OffloadEntry) -> OffloadId; fn get(&self, id: OffloadId) -> Option<OffloadEntry>; fn len(&self) -> usize; fn is_empty(&self) -> bool { self.len() == 0 } }`
  `pub struct InMemoryOffloadStore` with `pub fn new() -> Self`.
- Note: `put` takes `&self` (interior mutability) so the store can be shared as `Arc<dyn OffloadStore>` between the context and the recall tool. `put` assigns the id and ignores `entry.id` on input.

- [ ] **Step 1: Write the failing test**

Add to `agent/crates/agent-core/src/offload.rs`:

```rust
use std::collections::HashMap;
use std::sync::Mutex;

pub type OffloadId = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OffloadKind {
    Error,
    Output,
}

#[derive(Debug, Clone)]
pub struct OffloadEntry {
    pub id: OffloadId,
    pub tool_call_id: String,
    pub tool_name: String,
    pub kind: OffloadKind,
    pub content: String,
    pub bytes: usize,
    pub turn: usize,
}

/// A retrievable side-table for content lifted out of the live context window.
/// `put` uses interior mutability so the store can be shared (`Arc<dyn OffloadStore>`)
/// between the context manager and the `context_recall` tool.
pub trait OffloadStore: Send + Sync {
    /// Store full content; returns the assigned id. `entry.id` is ignored on input.
    fn put(&self, entry: OffloadEntry) -> OffloadId;
    fn get(&self, id: OffloadId) -> Option<OffloadEntry>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

struct Inner {
    next: OffloadId,
    map: HashMap<OffloadId, OffloadEntry>,
}

/// Process-local, per-session offload table. v1 impl; the `OffloadStore` trait
/// is the seam for a persisted/semantic store later.
pub struct InMemoryOffloadStore {
    inner: Mutex<Inner>,
}

impl InMemoryOffloadStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                next: 1,
                map: HashMap::new(),
            }),
        }
    }
}

impl Default for InMemoryOffloadStore {
    fn default() -> Self {
        Self::new()
    }
}

impl OffloadStore for InMemoryOffloadStore {
    fn put(&self, mut entry: OffloadEntry) -> OffloadId {
        let mut g = self.inner.lock().unwrap();
        let id = g.next;
        g.next += 1;
        entry.id = id;
        g.map.insert(id, entry);
        id
    }

    fn get(&self, id: OffloadId) -> Option<OffloadEntry> {
        self.inner.lock().unwrap().map.get(&id).cloned()
    }

    fn len(&self) -> usize {
        self.inner.lock().unwrap().map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(content: &str) -> OffloadEntry {
        OffloadEntry {
            id: 0,
            tool_call_id: "call-1".into(),
            tool_name: "shell".into(),
            kind: OffloadKind::Error,
            content: content.into(),
            bytes: content.len(),
            turn: 0,
        }
    }

    #[test]
    fn put_assigns_increasing_ids_and_get_round_trips() {
        let store = InMemoryOffloadStore::new();
        let id1 = store.put(entry("first"));
        let id2 = store.put(entry("second"));
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(store.get(id1).unwrap().content, "first");
        assert_eq!(store.get(id2).unwrap().content, "second");
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn get_unknown_id_is_none() {
        let store = InMemoryOffloadStore::new();
        assert!(store.get(42).is_none());
        assert!(store.is_empty());
    }

    #[test]
    fn put_overwrites_input_id_field() {
        let store = InMemoryOffloadStore::new();
        let mut e = entry("x");
        e.id = 999; // should be ignored
        let id = store.put(e);
        assert_eq!(id, 1);
        assert_eq!(store.get(1).unwrap().id, 1);
    }
}
```

Add to `agent/crates/agent-core/src/lib.rs` after `mod context;`:

```rust
mod offload;
pub use offload::*;
```

- [ ] **Step 2: Run test to verify it fails (before lib.rs wiring it won't compile; wire lib.rs first, then run)**

Run: `source ~/.cargo/env && cargo test -p agent-core offload:: 2>&1 | tail -20`
Expected: tests compile and PASS (this task is pure data; the "failing" stage is trivially satisfied). If compilation fails, fix before proceeding.

- [ ] **Step 3: (implementation already in Step 1)**

- [ ] **Step 4: Run the full crate to confirm no regressions**

Run: `source ~/.cargo/env && cargo test -p agent-core 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/offload.rs agent/crates/agent-core/src/lib.rs
git commit -m "feat(context): offload store trait + in-memory impl"
```

---

### Task 2: Offload policy (pure selection)

**Files:**
- Create: `agent/crates/agent-core/src/offload_policy.rs`
- Modify: `agent/crates/agent-core/src/lib.rs`

**Interfaces:**
- Consumes: `OffloadEntry`, `OffloadKind`, `OffloadId` (Task 1); `agent_model::{Message, Role}`.
- Produces:
  `pub struct OffloadConfig { pub error_min_bytes: usize, pub output_min_bytes: usize, pub keep_recent: usize, pub exclude_tools: Vec<String> }` with `Default` (`error_min_bytes: 200`, `output_min_bytes: 1024`, `keep_recent: 3`, `exclude_tools: vec![]`).
  `pub struct OffloadHit { pub history_index: usize, pub entry: OffloadEntry }`
  `pub fn select_offloads(history: &[Message], config: &OffloadConfig) -> Vec<OffloadHit>`
  `pub fn placeholder_for(id: OffloadId, tool_name: &str, kind: &OffloadKind, bytes: usize) -> String`
- Note: `select_offloads` is pure (no I/O). It scans `history` for `Role::Tool` messages, classifies kind by the `"ERROR: "` prefix, qualifies by size, skips the `keep_recent` most-recent tool results, skips `exclude_tools`, and skips messages already offloaded (content starting with `"[tool_result#"`).

- [ ] **Step 1: Write the failing test**

Create `agent/crates/agent-core/src/offload_policy.rs`:

```rust
use crate::offload::{OffloadEntry, OffloadId, OffloadKind};
use agent_model::{Message, Role};

const PLACEHOLDER_PREFIX: &str = "[tool_result#";

#[derive(Debug, Clone)]
pub struct OffloadConfig {
    /// Tool ERROR results at or above this many bytes are eligible.
    pub error_min_bytes: usize,
    /// Successful tool outputs at or above this many bytes are eligible.
    pub output_min_bytes: usize,
    /// The N most-recent tool results are always kept verbatim.
    pub keep_recent: usize,
    /// Tool names never offloaded.
    pub exclude_tools: Vec<String>,
}

impl Default for OffloadConfig {
    fn default() -> Self {
        Self {
            error_min_bytes: 200,
            output_min_bytes: 1024,
            keep_recent: 3,
            exclude_tools: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OffloadHit {
    pub history_index: usize,
    pub entry: OffloadEntry,
}

fn classify(content: &str) -> OffloadKind {
    if content.starts_with("ERROR: ") {
        OffloadKind::Error
    } else {
        OffloadKind::Output
    }
}

fn qualifies(kind: &OffloadKind, bytes: usize, config: &OffloadConfig) -> bool {
    match kind {
        OffloadKind::Error => bytes >= config.error_min_bytes,
        OffloadKind::Output => bytes >= config.output_min_bytes,
    }
}

/// Select tool-result messages eligible to be lifted out of the live window.
/// Pure: no I/O, deterministic. Skips the `keep_recent` most-recent tool results,
/// excluded tools, and already-offloaded placeholders.
pub fn select_offloads(history: &[Message], config: &OffloadConfig) -> Vec<OffloadHit> {
    // Indices of all tool-result messages, oldest-first.
    let tool_indices: Vec<usize> = history
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m.role, Role::Tool))
        .map(|(i, _)| i)
        .collect();
    // Protect the most-recent `keep_recent` tool results.
    let protected_from = tool_indices.len().saturating_sub(config.keep_recent);
    let eligible_indices = &tool_indices[..protected_from.min(tool_indices.len())];

    let mut hits = Vec::new();
    for &i in eligible_indices {
        let m = &history[i];
        if m.content.starts_with(PLACEHOLDER_PREFIX) {
            continue; // already offloaded
        }
        let tool_name = m.name.clone().unwrap_or_default();
        if config.exclude_tools.iter().any(|t| t == &tool_name) {
            continue;
        }
        let bytes = m.content.len();
        let kind = classify(&m.content);
        if !qualifies(&kind, bytes, config) {
            continue;
        }
        hits.push(OffloadHit {
            history_index: i,
            entry: OffloadEntry {
                id: 0,
                tool_call_id: m.tool_call_id.clone().unwrap_or_default(),
                tool_name,
                kind,
                content: m.content.clone(),
                bytes,
                turn: i,
            },
        });
    }
    hits
}

/// The compact stub left in the live window in place of offloaded content.
pub fn placeholder_for(
    id: OffloadId,
    tool_name: &str,
    kind: &OffloadKind,
    bytes: usize,
) -> String {
    let kind_str = match kind {
        OffloadKind::Error => "error",
        OffloadKind::Output => "output",
    };
    format!(
        "[tool_result#{id} offloaded: {bytes}B {kind_str} from \"{tool_name}\" \
         — recall with context_recall({id})]"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_model::Message;

    fn tool_msg(name: &str, content: &str) -> Message {
        Message::tool("call-x", name, content)
    }

    #[test]
    fn large_error_is_selected() {
        let history = vec![tool_msg("shell", &format!("ERROR: {}", "x".repeat(300)))];
        let hits = select_offloads(&history, &OffloadConfig { keep_recent: 0, ..Default::default() });
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry.kind, OffloadKind::Error);
        assert_eq!(hits[0].history_index, 0);
    }

    #[test]
    fn small_error_under_threshold_is_kept() {
        let history = vec![tool_msg("shell", "ERROR: nope")];
        let hits = select_offloads(&history, &OffloadConfig { keep_recent: 0, ..Default::default() });
        assert!(hits.is_empty());
    }

    #[test]
    fn large_success_output_is_selected() {
        let history = vec![tool_msg("read_file", &"y".repeat(2000))];
        let hits = select_offloads(&history, &OffloadConfig { keep_recent: 0, ..Default::default() });
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry.kind, OffloadKind::Output);
    }

    #[test]
    fn keep_recent_protects_newest_tool_results() {
        let history = vec![
            tool_msg("shell", &format!("ERROR: {}", "a".repeat(300))),
            tool_msg("shell", &format!("ERROR: {}", "b".repeat(300))),
            tool_msg("shell", &format!("ERROR: {}", "c".repeat(300))),
        ];
        let hits = select_offloads(&history, &OffloadConfig { keep_recent: 2, ..Default::default() });
        // Only the oldest of three is eligible.
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].history_index, 0);
    }

    #[test]
    fn exclude_tools_is_honored() {
        let history = vec![tool_msg("shell", &format!("ERROR: {}", "a".repeat(300)))];
        let config = OffloadConfig {
            keep_recent: 0,
            exclude_tools: vec!["shell".into()],
            ..Default::default()
        };
        assert!(select_offloads(&history, &config).is_empty());
    }

    #[test]
    fn already_offloaded_placeholder_is_skipped() {
        let history = vec![tool_msg("shell", &placeholder_for(7, "shell", &OffloadKind::Error, 300))];
        let hits = select_offloads(&history, &OffloadConfig { keep_recent: 0, ..Default::default() });
        assert!(hits.is_empty(), "must be idempotent — never re-offload a placeholder");
    }

    #[test]
    fn non_tool_messages_are_ignored() {
        let history = vec![
            Message::user(&"u".repeat(3000)),
            Message::assistant(&"a".repeat(3000), None),
        ];
        let hits = select_offloads(&history, &OffloadConfig { keep_recent: 0, ..Default::default() });
        assert!(hits.is_empty());
    }

    #[test]
    fn placeholder_preserves_id_and_tool() {
        let p = placeholder_for(7, "shell", &OffloadKind::Error, 3100);
        assert!(p.contains("tool_result#7"));
        assert!(p.contains("context_recall(7)"));
        assert!(p.contains("shell"));
        assert!(p.contains("3100B"));
    }
}
```

Add to `agent/crates/agent-core/src/lib.rs` after `mod offload;`:

```rust
mod offload_policy;
pub use offload_policy::*;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `source ~/.cargo/env && cargo test -p agent-core offload_policy:: 2>&1 | tail -25`
Expected: PASS after wiring (pure logic implemented alongside tests). If any assertion fails, fix the logic, not the test.

- [ ] **Step 3: (implementation already in Step 1)**

- [ ] **Step 4: Run crate tests**

Run: `source ~/.cargo/env && cargo test -p agent-core 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/offload_policy.rs agent/crates/agent-core/src/lib.rs
git commit -m "feat(context): pure offload-selection policy + placeholder"
```

---

### Task 3: Trait extension + CuratedContext (build, set_goal, windowing)

**Files:**
- Modify: `agent/crates/agent-core/src/context.rs` (make `message_tokens` `pub(crate)`; extract `recall_block`; add `#[async_trait]`, `set_goal`, `maintain` to the trait; define `MaintCtx`, `MaintReport`)
- Create: `agent/crates/agent-core/src/curated.rs`
- Modify: `agent/crates/agent-core/src/lib.rs`

**Interfaces:**
- Consumes: `OffloadStore` (Task 1), `OffloadConfig` (Task 2), `recall_block`/`message_tokens` (this task), `agent_model::{Message, ModelClient}`, `crate::EventSink`, `tokio_util::sync::CancellationToken`.
- Produces:
  `pub struct MaintReport { pub offloaded: usize, pub offloaded_bytes: usize, pub compacted_turns: usize }` (derives `Debug, Clone, Default, PartialEq, Eq`).
  `pub struct MaintCtx<'a> { pub model_limit: usize, pub model: &'a Arc<dyn ModelClient>, pub sink: &'a Arc<dyn EventSink>, pub cancel: &'a CancellationToken }`
  Trait now has `fn set_goal(&mut self, _goal: String) {}` and `async fn maintain(&mut self, _deps: &MaintCtx<'_>) -> MaintReport { MaintReport::default() }`.
  `pub struct CuratedContext` with `pub fn new(system: Message, store: Arc<dyn OffloadStore>, compact_flag: Arc<std::sync::atomic::AtomicBool>) -> Self`, `pub fn with_recall_budget(self, budget: usize) -> Self`, `pub fn with_offload_config(self, config: OffloadConfig) -> Self`.
- Note: `maintain` is defaulted, so `WindowContext` is untouched and all its tests keep passing. `set_goal` is set-once (preserves the original prompt across multi-turn runs).

- [ ] **Step 1: Modify `context.rs` — visibility, `recall_block`, trait**

In `agent/crates/agent-core/src/context.rs`:

Change the signature of `message_tokens` from private to crate-visible:

```rust
pub(crate) fn message_tokens(m: &Message) -> usize {
```

Add this free function just above the `ContextManager` trait:

```rust
/// Build a capped recall/notes block: greedily keep lines under `budget` tokens,
/// always including at least the first line if any are present (soft cap).
/// Shared by `WindowContext` and `CuratedContext`.
pub(crate) fn recall_block(lines: &[String], budget: usize) -> Option<Message> {
    if lines.is_empty() {
        return None;
    }
    const HEADER: &str = "Relevant memories from past sessions:";
    let mut body = String::from(HEADER);
    for line in lines {
        let candidate = format!("{body}\n- {line}");
        if estimate_tokens(&candidate) > budget && body != HEADER {
            break;
        }
        body = candidate;
    }
    Some(Message::system(body))
}
```

Replace `WindowContext::recall_message` body to delegate (keeps behavior identical):

```rust
    fn recall_message(&self) -> Option<Message> {
        recall_block(&self.recall, self.recall_budget)
    }
```

Add imports at the top of `context.rs`:

```rust
use crate::EventSink;
use agent_model::ModelClient;
use async_trait::async_trait;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
```

Add the two new types just above the trait:

```rust
/// What one `maintain` pass did, for telemetry/tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaintReport {
    pub offloaded: usize,
    pub offloaded_bytes: usize,
    pub compacted_turns: usize,
}

/// Dependencies a context manager needs to run maintenance (compaction needs a
/// model; offload does not). Borrowed for the duration of the call.
pub struct MaintCtx<'a> {
    pub model_limit: usize,
    pub model: &'a Arc<dyn ModelClient>,
    pub sink: &'a Arc<dyn EventSink>,
    pub cancel: &'a CancellationToken,
}
```

Change the trait to add `#[async_trait]` and the two new methods (defaulted):

```rust
#[async_trait]
pub trait ContextManager: Send + Sync {
    fn append(&mut self, msg: Message);
    fn build(&self, model_limit: usize) -> Vec<Message>;
    fn set_system(&mut self, system: Message);
    /// Replace the auto-retrieved recall lines surfaced this turn. Default no-op.
    fn set_recall(&mut self, _items: Vec<String>) {}
    /// Record the original goal for re-grounding. Default no-op; set-once impls.
    fn set_goal(&mut self, _goal: String) {}
    /// Best-effort per-turn curation (offload + compaction). Default no-op so
    /// `WindowContext` and other simple impls are unaffected.
    async fn maintain(&mut self, _deps: &MaintCtx<'_>) -> MaintReport {
        MaintReport::default()
    }
}
```

- [ ] **Step 2: Create `curated.rs` with the failing build/set_goal tests**

Create `agent/crates/agent-core/src/curated.rs`:

```rust
use crate::context::{message_tokens, recall_block, ContextManager, DEFAULT_RECALL_TOKEN_BUDGET};
use crate::offload::OffloadStore;
use crate::offload_policy::OffloadConfig;
use agent_model::Message;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Default fraction of `model_limit` at which `maintain` triggers a compaction pass.
pub const DEFAULT_HIGH_WATER_PCT: f32 = 0.85;

/// A curating context manager. Pins `system → re-grounding → recall → compaction
/// summary → windowed recent history`, offloads stale/large tool results into a
/// side table each turn, and compacts the old span when over the high-water mark.
pub struct CuratedContext {
    system: Message,
    goal: Option<Message>,
    history: Vec<Message>,
    recall: Vec<String>,
    recall_budget: usize,
    pub(crate) compaction_summary: Option<Message>,
    pub(crate) store: Arc<dyn OffloadStore>,
    pub(crate) config: OffloadConfig,
    pub(crate) high_water_pct: f32,
    pub(crate) compact_flag: Arc<AtomicBool>,
}

impl CuratedContext {
    pub fn new(
        system: Message,
        store: Arc<dyn OffloadStore>,
        compact_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            system,
            goal: None,
            history: Vec::new(),
            recall: Vec::new(),
            recall_budget: DEFAULT_RECALL_TOKEN_BUDGET,
            compaction_summary: None,
            store,
            config: OffloadConfig::default(),
            high_water_pct: DEFAULT_HIGH_WATER_PCT,
            compact_flag,
        }
    }

    pub fn with_recall_budget(mut self, budget: usize) -> Self {
        self.recall_budget = budget;
        self
    }

    pub fn with_offload_config(mut self, config: OffloadConfig) -> Self {
        self.config = config;
        self
    }

    /// The pinned blocks, in assembly order, that precede windowed history.
    fn pinned(&self) -> Vec<Message> {
        let mut out = vec![self.system.clone()];
        if let Some(g) = &self.goal {
            out.push(g.clone());
        }
        if let Some(r) = recall_block(&self.recall, self.recall_budget) {
            out.push(r);
        }
        if let Some(c) = &self.compaction_summary {
            out.push(c.clone());
        }
        out
    }

    /// Borrow history (used by the compaction-failure test in Task 5).
    pub(crate) fn history(&self) -> &[Message] {
        &self.history
    }
    pub(crate) fn goal_text(&self) -> Option<&str> {
        self.goal.as_ref().map(|m| m.content.as_str())
    }
}

impl ContextManager for CuratedContext {
    fn append(&mut self, msg: Message) {
        self.history.push(msg);
    }

    fn set_system(&mut self, system: Message) {
        self.system = system;
    }

    fn set_recall(&mut self, items: Vec<String>) {
        self.recall = items;
    }

    fn set_goal(&mut self, goal: String) {
        if self.goal.is_none() {
            self.goal = Some(Message::system(format!("Original goal: {goal}")));
        }
    }

    fn build(&self, model_limit: usize) -> Vec<Message> {
        let pinned = self.pinned();
        let pinned_tokens: usize = pinned.iter().map(message_tokens).sum();
        let budget = model_limit.saturating_sub(pinned_tokens);
        // Walk history newest-first, keep while it fits.
        let mut kept_rev: Vec<Message> = Vec::new();
        let mut used = 0usize;
        for m in self.history.iter().rev() {
            let t = message_tokens(m);
            if used + t > budget && !kept_rev.is_empty() {
                break;
            }
            used += t;
            kept_rev.push(m.clone());
        }
        kept_rev.reverse();
        let mut out = pinned;
        out.extend(kept_rev);
        out
    }
    // `maintain` uses the trait default until Task 4 overrides it.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offload::InMemoryOffloadStore;
    use agent_model::Role;

    fn ctx() -> CuratedContext {
        CuratedContext::new(
            Message::system("SYS"),
            Arc::new(InMemoryOffloadStore::new()),
            Arc::new(AtomicBool::new(false)),
        )
    }

    #[test]
    fn build_assembly_order_system_goal_recall_then_history() {
        let mut c = ctx();
        c.set_goal("ship the feature".into());
        c.set_recall(vec!["user likes rust".into()]);
        c.append(Message::user("hello"));
        let built = c.build(100_000);
        assert!(matches!(built[0].role, Role::System));
        assert_eq!(built[0].content, "SYS");
        assert_eq!(built[1].content, "Original goal: ship the feature");
        assert!(built[2].content.starts_with("Relevant memories"));
        assert_eq!(built.last().unwrap().content, "hello");
    }

    #[test]
    fn set_goal_is_set_once() {
        let mut c = ctx();
        c.set_goal("first goal".into());
        c.set_goal("second goal".into());
        let built = c.build(100_000);
        assert_eq!(built[1].content, "Original goal: first goal");
    }

    #[test]
    fn goal_block_survives_tiny_limit() {
        let mut c = ctx();
        c.set_goal("the goal".into());
        for i in 0..50 {
            c.append(Message::user(format!("message number {i} with padding text")));
        }
        let built = c.build(40);
        assert!(built.iter().any(|m| m.content == "Original goal: the goal"));
        assert!(built.len() < 51);
    }

    #[test]
    fn build_returns_pinned_plus_history_under_limit() {
        let mut c = ctx();
        c.append(Message::user("hi"));
        let built = c.build(100_000);
        assert_eq!(built.len(), 2); // system + history (no goal/recall set)
    }
}
```

Add to `agent/crates/agent-core/src/lib.rs` after `mod offload_policy;`:

```rust
mod curated;
pub use curated::*;
```

- [ ] **Step 3: Run tests to verify they pass and nothing regressed**

Run: `source ~/.cargo/env && cargo test -p agent-core 2>&1 | tail -25`
Expected: new `curated::tests` PASS; existing `context::tests` (including `WindowContext`) still PASS — proving the defaulted `maintain` kept backward compatibility.

- [ ] **Step 4: Confirm the workspace still builds (trait change is widest-reaching)**

Run: `source ~/.cargo/env && cargo build --workspace 2>&1 | tail -15`
Expected: builds. If a `WindowContext` `impl` now needs `#[async_trait]`, add the attribute to that impl block and rebuild.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/context.rs agent/crates/agent-core/src/curated.rs agent/crates/agent-core/src/lib.rs
git commit -m "feat(context): CuratedContext build + re-grounding; async maintain trait hook"
```

---

### Task 4: Context events + maintain offload pass + loop wiring

**Files:**
- Modify: `agent/crates/agent-core/src/event.rs` (add `ContextEvent` + `AgentEvent::Context`)
- Modify: `agent/crates/agent-core/src/curated.rs` (override `maintain` with the offload pass)
- Modify: `agent/crates/agent-core/src/loop_.rs` (call `ctx.maintain` once per turn)
- Modify: `agent/crates/agent-cli/src/render.rs` and `agent/crates/agent-server/src/wire.rs` (handle the new arm)

**Interfaces:**
- Consumes: `select_offloads`, `placeholder_for` (Task 2); `MaintCtx`, `MaintReport` (Task 3); `OffloadStore::put` (Task 1).
- Produces: `pub enum ContextEvent { Offloaded { id: u64, bytes: usize, tool: String }, Compacted { turns_replaced: usize, tokens_before: usize, tokens_after: usize }, CompactionFailed { reason: String } }`; `AgentEvent::Context(ContextEvent)`; `CuratedContext::maintain` now performs the offload pass and emits `ContextEvent::Offloaded`.

- [ ] **Step 1: Add the event variant**

In `agent/crates/agent-core/src/event.rs`, add above the `AgentEvent` enum:

```rust
/// Telemetry for context-window curation (offload / compaction).
#[derive(Debug, Clone)]
pub enum ContextEvent {
    Offloaded { id: u64, bytes: usize, tool: String },
    Compacted { turns_replaced: usize, tokens_before: usize, tokens_after: usize },
    CompactionFailed { reason: String },
}
```

Add a variant to `AgentEvent`:

```rust
    Context(ContextEvent),
```

Update the two exhaustive consumers.

`agent/crates/agent-cli/src/render.rs` — add an arm alongside `AgentEvent::Usage { .. } => {}`:

```rust
            AgentEvent::Context(_) => {} // curation telemetry; not shown in the CLI
```

`agent/crates/agent-server/src/wire.rs` — add an arm in `server_event_from` alongside `AgentEvent::Approval(_) => return None`:

```rust
        AgentEvent::Context(_) => return None, // curation telemetry; not forwarded to clients in v1
```

- [ ] **Step 2: Write the failing maintain-offload test**

In `agent/crates/agent-core/src/curated.rs`, add to the `tests` module:

```rust
    use crate::context::MaintCtx;
    use crate::event::EventSink;
    use crate::testkit::{CollectingSink, ScriptedModel};
    use agent_model::ModelClient;
    use tokio_util::sync::CancellationToken;

    fn maint_deps<'a>(
        model: &'a Arc<dyn ModelClient>,
        sink: &'a Arc<dyn EventSink>,
        cancel: &'a CancellationToken,
    ) -> MaintCtx<'a> {
        MaintCtx { model_limit: 100_000, model, sink, cancel }
    }

    #[tokio::test]
    async fn maintain_offloads_stale_large_error_to_store_and_leaves_placeholder() {
        let mut c = ctx().with_offload_config(OffloadConfig {
            keep_recent: 0,
            ..Default::default()
        });
        let big_err = format!("ERROR: {}", "x".repeat(400));
        c.append(Message::tool("call-1", "shell", big_err.clone()));

        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;

        assert_eq!(report.offloaded, 1);
        assert_eq!(c.store.len(), 1);
        // Live message is now a placeholder; full content recoverable from the store.
        let built = c.build(100_000);
        let tool_msg = built.iter().find(|m| matches!(m.role, Role::Tool)).unwrap();
        assert!(tool_msg.content.starts_with("[tool_result#1 offloaded"));
        assert_eq!(c.store.get(1).unwrap().content, big_err);
    }

    #[tokio::test]
    async fn maintain_is_idempotent() {
        let mut c = ctx().with_offload_config(OffloadConfig { keep_recent: 0, ..Default::default() });
        c.append(Message::tool("call-1", "shell", format!("ERROR: {}", "x".repeat(400))));
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        let report2 = c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        assert_eq!(report2.offloaded, 0, "second pass must not re-offload");
        assert_eq!(c.store.len(), 1);
    }
```

- [ ] **Step 3: Implement the offload pass — override `maintain`**

In `agent/crates/agent-core/src/curated.rs`, add imports at the top:

```rust
use crate::context::{MaintCtx, MaintReport};
use crate::event::{AgentEvent, ContextEvent};
use crate::offload_policy::{placeholder_for, select_offloads};
use async_trait::async_trait;
```

Replace the trailing `// maintain uses the trait default...` comment inside `impl ContextManager for CuratedContext` — that is, change the impl block header to `#[async_trait]` and add the method. The impl header becomes:

```rust
#[async_trait]
impl ContextManager for CuratedContext {
```

And add this method at the end of the impl block (after `build`):

```rust
    async fn maintain(&mut self, deps: &MaintCtx<'_>) -> MaintReport {
        let mut report = MaintReport::default();

        // (a) Deterministic offload — sync, cheap, every turn.
        let hits = select_offloads(&self.history, &self.config);
        for hit in hits {
            let idx = hit.history_index;
            let tool = hit.entry.tool_name.clone();
            let kind = hit.entry.kind.clone();
            let bytes = hit.entry.bytes;
            let id = self.store.put(hit.entry);
            self.history[idx].content = placeholder_for(id, &tool, &kind, bytes);
            report.offloaded += 1;
            report.offloaded_bytes += bytes;
            deps.sink
                .emit(AgentEvent::Context(ContextEvent::Offloaded { id, bytes, tool }));
        }

        // (b) Compaction added in Task 5.
        report
    }
```

- [ ] **Step 4: Wire the loop to call `maintain` each turn**

In `agent/crates/agent-core/src/loop_.rs`, inside `run_with_cancel`, two edits.

First, set the goal once, right after `ctx.append(Message::user(user_input));` (line 159). Replace those lines so the user input is cloned for the goal:

```rust
        ctx.set_goal(user_input.clone());
        ctx.append(Message::user(user_input));
```

Second, add the maintain call at the very end of the `for turn` loop body — immediately after the Phase-3 `for id in order { ... }` block closes (after `loop_.rs:289`) and before the loop's closing brace:

```rust
            let deps = crate::MaintCtx {
                model_limit: self.config.model_limit,
                model: &self.model,
                sink: &self.sink,
                cancel: &cancel,
            };
            let report = ctx.maintain(&deps).await;
            if report.offloaded > 0 || report.compacted_turns > 0 {
                tracing::debug!(
                    offloaded = report.offloaded,
                    offloaded_bytes = report.offloaded_bytes,
                    compacted_turns = report.compacted_turns,
                    "context maintained"
                );
            }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `source ~/.cargo/env && cargo test -p agent-core curated:: 2>&1 | tail -25`
Expected: `maintain_offloads_*` and `maintain_is_idempotent` PASS.

- [ ] **Step 6: Build the workspace (event variant + loop touch the most crates)**

Run: `source ~/.cargo/env && cargo build --workspace 2>&1 | tail -15`
Expected: builds clean.

- [ ] **Step 7: Commit**

```bash
git add agent/crates/agent-core/src/event.rs agent/crates/agent-core/src/curated.rs agent/crates/agent-core/src/loop_.rs agent/crates/agent-cli/src/render.rs agent/crates/agent-server/src/wire.rs
git commit -m "feat(context): per-turn offload pass + maintain loop hook + context events"
```

---

### Task 5: Compactor + gated compaction pass

**Files:**
- Create: `agent/crates/agent-core/src/compactor.rs`
- Modify: `agent/crates/agent-core/src/curated.rs` (compaction half of `maintain`)
- Modify: `agent/crates/agent-core/src/lib.rs`

**Interfaces:**
- Consumes: `agent_model::{Message, ModelClient, CompletionRequest, Chunk}`, `MaintCtx`, `message_tokens`.
- Produces:
  `pub enum CompactError { Model(String), Cancelled }` (impls `std::fmt::Display`).
  `pub async fn run_compaction(span: &[Message], goal: Option<&str>, model: &Arc<dyn ModelClient>, cancel: &CancellationToken) -> Result<Message, CompactError>` — returns one `Message::system(summary)`.
- Note: compaction only commits if the summary is non-empty AND strictly smaller (in estimated tokens) than the span it replaces. On any error/cancel, history is left untouched.

- [ ] **Step 1: Create `compactor.rs` with the failing test**

Create `agent/crates/agent-core/src/compactor.rs`:

```rust
use crate::context::message_tokens;
use agent_model::{Chunk, CompletionRequest, Message, ModelClient, Role};
use futures::StreamExt;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub enum CompactError {
    Model(String),
    Cancelled,
}

impl std::fmt::Display for CompactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompactError::Model(m) => write!(f, "compaction model error: {m}"),
            CompactError::Cancelled => write!(f, "compaction cancelled"),
        }
    }
}

const COMPACTION_SYSTEM: &str = "You are a context compaction engine. Compress the \
conversation excerpt below into a dense, high-fidelity summary that preserves: \
decisions made, unresolved problems, key facts, and file/identifier names. Drop \
redundant tool output and chatter. Be terse. Output only the summary.";

fn render_span(span: &[Message], goal: Option<&str>) -> String {
    let mut s = String::new();
    if let Some(g) = goal {
        s.push_str(&format!("Original goal: {g}\n\n"));
    }
    s.push_str("Conversation excerpt to compact:\n");
    for m in span {
        let role = match m.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        s.push_str(&format!("[{role}] {}\n", m.content));
    }
    s
}

/// Drive one non-streaming-from-the-caller's-view completion to a single string.
async fn collect_stream(
    model: &Arc<dyn ModelClient>,
    req: CompletionRequest,
    cancel: &CancellationToken,
) -> Result<String, CompactError> {
    let mut stream = tokio::select! {
        _ = cancel.cancelled() => return Err(CompactError::Cancelled),
        opened = model.stream(req) => opened.map_err(|e| CompactError::Model(e.to_string()))?,
    };
    let mut text = String::new();
    loop {
        let step = tokio::select! {
            _ = cancel.cancelled() => return Err(CompactError::Cancelled),
            s = stream.next() => s,
        };
        match step {
            None => break,
            Some(item) => match item.map_err(|e| CompactError::Model(e.to_string()))? {
                Chunk::Text(t) => text.push_str(&t),
                Chunk::Done(_) => break,
                _ => {}
            },
        }
    }
    Ok(text)
}

/// Summarize `span` into a single high-fidelity system message. Read-only: the
/// caller decides whether to commit the result.
pub async fn run_compaction(
    span: &[Message],
    goal: Option<&str>,
    model: &Arc<dyn ModelClient>,
    cancel: &CancellationToken,
) -> Result<Message, CompactError> {
    let req = CompletionRequest {
        messages: vec![
            Message::system(COMPACTION_SYSTEM),
            Message::user(render_span(span, goal)),
        ],
        temperature: 0.0,
        ..Default::default()
    };
    let summary = collect_stream(model, req, cancel).await?;
    let body = format!("Summary of earlier conversation:\n{}", summary.trim());
    Ok(Message::system(body))
}

/// True when `summary` is a net token win over `span` (and non-empty).
pub(crate) fn compaction_is_worthwhile(summary: &Message, span: &[Message]) -> bool {
    let summary_body = summary
        .content
        .strip_prefix("Summary of earlier conversation:\n")
        .unwrap_or(&summary.content);
    if summary_body.trim().is_empty() {
        return false;
    }
    let span_tokens: usize = span.iter().map(message_tokens).sum();
    message_tokens(summary) < span_tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{Scripted, ScriptedModel};

    #[tokio::test]
    async fn run_compaction_returns_summary_message() {
        let span = vec![Message::user("a".repeat(50)), Message::assistant("b".repeat(50), None)];
        let model: Arc<dyn ModelClient> =
            Arc::new(ScriptedModel::new(vec![Scripted::Text("decided X; bug Y open".into())]));
        let cancel = CancellationToken::new();
        let msg = run_compaction(&span, Some("do the thing"), &model, &cancel).await.unwrap();
        assert!(matches!(msg.role, Role::System));
        assert!(msg.content.contains("decided X; bug Y open"));
    }

    #[tokio::test]
    async fn worthwhile_rejects_empty_or_larger_summary() {
        let span = vec![Message::user("tiny")];
        let empty = Message::system("Summary of earlier conversation:\n   ");
        assert!(!compaction_is_worthwhile(&empty, &span));
        let huge = Message::system(format!("Summary of earlier conversation:\n{}", "x".repeat(9999)));
        assert!(!compaction_is_worthwhile(&huge, &span));
    }
}
```

Add to `agent/crates/agent-core/src/lib.rs` after `mod curated;`:

```rust
mod compactor;
pub use compactor::*;
```

- [ ] **Step 2: Run compactor unit tests to verify pass**

Run: `source ~/.cargo/env && cargo test -p agent-core compactor:: 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 3: Write the failing compaction-in-maintain e2e test**

In `agent/crates/agent-core/src/curated.rs` `tests` module, add:

```rust
    #[tokio::test]
    async fn maintain_compacts_old_span_when_over_high_water() {
        let mut c = ctx();
        c.high_water_pct = 0.0; // force compaction regardless of size
        c.config.keep_recent = 1;
        for i in 0..6 {
            c.append(Message::user(format!("turn {i} with a fair bit of padding text here")));
        }
        let model: Arc<dyn ModelClient> =
            Arc::new(ScriptedModel::new(vec![Scripted::Text("compact summary".into())]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;

        assert!(report.compacted_turns > 0);
        let built = c.build(100_000);
        // A compaction summary block is present and the most-recent turn survives verbatim.
        assert!(built.iter().any(|m| m.content.contains("compact summary")));
        assert!(built.iter().any(|m| m.content.contains("turn 5")));
    }

    #[tokio::test]
    async fn maintain_leaves_history_intact_when_compaction_fails() {
        let mut c = ctx();
        c.high_water_pct = 0.0;
        c.config.keep_recent = 1;
        for i in 0..6 {
            c.append(Message::user(format!("turn {i} with padding text")));
        }
        let before = c.history().len();
        // Empty script => stream yields nothing => empty summary => not worthwhile => discarded.
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![Scripted::Text(String::new())]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        assert_eq!(report.compacted_turns, 0);
        assert_eq!(c.history().len(), before, "history must be untouched on failed/empty compaction");
    }
```

- [ ] **Step 4: Implement the compaction half of `maintain`**

In `agent/crates/agent-core/src/curated.rs`, add imports:

```rust
use crate::compactor::{compaction_is_worthwhile, run_compaction};
use std::sync::atomic::Ordering;
```

Replace the `// (b) Compaction added in Task 5.` comment in `maintain` with:

```rust
        // (b) Compaction — async, gated by the high-water mark or an explicit request.
        let requested = self.compact_flag.swap(false, Ordering::SeqCst);
        let over_high_water = {
            let built = self.build(deps.model_limit);
            let used: usize = built.iter().map(message_tokens).sum();
            (used as f32) > (deps.model_limit as f32 * self.high_water_pct)
        };
        if (requested || over_high_water) && self.history.len() > self.config.keep_recent + 1 {
            let split = self.history.len() - self.config.keep_recent;
            // Carry prior summary forward so its information isn't lost on re-compaction.
            let mut span: Vec<Message> = Vec::new();
            if let Some(prev) = &self.compaction_summary {
                span.push(prev.clone());
            }
            span.extend_from_slice(&self.history[..split]);
            let tokens_before: usize = span.iter().map(message_tokens).sum();
            match run_compaction(&span, self.goal_text(), deps.model, deps.cancel).await {
                Ok(summary) if compaction_is_worthwhile(&summary, &span) => {
                    let tokens_after = message_tokens(&summary);
                    let recent = self.history.split_off(split);
                    self.history = recent;
                    self.compaction_summary = Some(summary);
                    report.compacted_turns = split;
                    deps.sink.emit(AgentEvent::Context(ContextEvent::Compacted {
                        turns_replaced: split,
                        tokens_before,
                        tokens_after,
                    }));
                }
                Ok(_) => {
                    tracing::debug!("compaction not worthwhile; discarded");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "compaction failed; leaving history intact");
                    deps.sink.emit(AgentEvent::Context(ContextEvent::CompactionFailed {
                        reason: e.to_string(),
                    }));
                }
            }
        }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `source ~/.cargo/env && cargo test -p agent-core curated:: 2>&1 | tail -25`
Expected: `maintain_compacts_old_span_when_over_high_water` and `maintain_leaves_history_intact_when_compaction_fails` PASS, plus all earlier `curated` tests.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-core/src/compactor.rs agent/crates/agent-core/src/curated.rs agent/crates/agent-core/src/lib.rs
git commit -m "feat(context): gated compaction pass with worthwhile-guard + failure safety"
```

---

### Task 6: context_recall and context_compact tools

**Files:**
- Create: `agent/crates/agent-core/src/context_tools.rs`
- Modify: `agent/crates/agent-core/src/lib.rs`

**Interfaces:**
- Consumes: `OffloadStore` (Task 1), `agent_tools::{Tool, ToolCtx, ToolOutput, ToolError, ToolSchema, ToolIntent, Access}`.
- Produces:
  `pub struct ContextRecallTool` with `pub fn new(store: Arc<dyn OffloadStore>) -> Self`.
  `pub struct ContextCompactTool` with `pub fn new(flag: Arc<AtomicBool>) -> Self`.
  `pub fn context_tools(store: Arc<dyn OffloadStore>, flag: Arc<AtomicBool>) -> Vec<Arc<dyn Tool>>` — the pair ready to register.
- Note: `context_recall` returns `ToolError::NotFound` for an unknown id (the loop renders it as `ERROR: not found: …`, fed back to the model). `context_compact` sets the shared `AtomicBool` that `CuratedContext::maintain` reads.

- [ ] **Step 1: Create `context_tools.rs` with failing tests**

Create `agent/crates/agent-core/src/context_tools.rs`:

```rust
use crate::offload::OffloadStore;
use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Rehydrate an offloaded entry by id, returning its full content to the model.
pub struct ContextRecallTool {
    store: Arc<dyn OffloadStore>,
}

impl ContextRecallTool {
    pub fn new(store: Arc<dyn OffloadStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for ContextRecallTool {
    fn name(&self) -> &str {
        "context_recall"
    }
    fn description(&self) -> &str {
        "Recall the full content of a previously offloaded tool result by its id \
         (the number in a [tool_result#N offloaded ...] placeholder)."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "context_recall".into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": { "id": { "type": "integer", "description": "offload id" } },
                "required": ["id"]
            }),
        }
    }
    fn intent(&self, _args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "context_recall".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "recall offloaded content".into(),
        })
    }
    async fn execute(&self, args: serde_json::Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let id = args
            .get("id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ToolError::InvalidArgs("missing integer 'id'".into()))?;
        match self.store.get(id) {
            Some(entry) => Ok(ToolOutput { content: entry.content, display: None }),
            None => Err(ToolError::NotFound(format!(
                "no offloaded entry #{id} (may have been cleared)"
            ))),
        }
    }
}

/// Request a compaction pass on the next maintenance cycle.
pub struct ContextCompactTool {
    flag: Arc<AtomicBool>,
}

impl ContextCompactTool {
    pub fn new(flag: Arc<AtomicBool>) -> Self {
        Self { flag }
    }
}

#[async_trait]
impl Tool for ContextCompactTool {
    fn name(&self) -> &str {
        "context_compact"
    }
    fn description(&self) -> &str {
        "Request compaction of older conversation history into a summary on the \
         next turn. Use when the context is full of resolved sub-tasks."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "context_compact".into(),
            description: self.description().into(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }
    fn intent(&self, _args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "context_compact".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "request context compaction".into(),
        })
    }
    async fn execute(&self, _args: serde_json::Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        self.flag.store(true, Ordering::SeqCst);
        Ok(ToolOutput {
            content: "Compaction requested; it will run on the next turn.".into(),
            display: None,
        })
    }
}

/// The context-management tool pair, sharing handles with a `CuratedContext`.
pub fn context_tools(store: Arc<dyn OffloadStore>, flag: Arc<AtomicBool>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ContextRecallTool::new(store)),
        Arc::new(ContextCompactTool::new(flag)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offload::{InMemoryOffloadStore, OffloadEntry, OffloadKind};
    use agent_tools::SandboxStrategy;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn tool_ctx() -> ToolCtx {
        ToolCtx {
            workspace: std::env::temp_dir(),
            timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(),
            sandbox: <Arc<dyn SandboxStrategy>>::from(agent_tools::no_sandbox()),
        }
    }

    #[tokio::test]
    async fn recall_returns_full_content() {
        let store = Arc::new(InMemoryOffloadStore::new());
        let id = store.put(OffloadEntry {
            id: 0,
            tool_call_id: "c1".into(),
            tool_name: "shell".into(),
            kind: OffloadKind::Error,
            content: "the full stack trace".into(),
            bytes: 20,
            turn: 0,
        });
        let tool = ContextRecallTool::new(store);
        let out = tool.execute(json!({ "id": id }), &tool_ctx()).await.unwrap();
        assert_eq!(out.content, "the full stack trace");
    }

    #[tokio::test]
    async fn recall_unknown_id_is_not_found() {
        let tool = ContextRecallTool::new(Arc::new(InMemoryOffloadStore::new()));
        let err = tool.execute(json!({ "id": 999 }), &tool_ctx()).await.unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
    }

    #[tokio::test]
    async fn compact_sets_the_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        let tool = ContextCompactTool::new(flag.clone());
        tool.execute(json!({}), &tool_ctx()).await.unwrap();
        assert!(flag.load(Ordering::SeqCst));
    }
}
```

Add to `agent/crates/agent-core/src/lib.rs` after `mod compactor;`:

```rust
mod context_tools;
pub use context_tools::*;
```

- [ ] **Step 2: Verify the test sandbox helper name**

Run: `source ~/.cargo/env && grep -rn "no_sandbox\|NoSandbox\|pub fn.*[Ss]andbox" agent/crates/agent-tools/src/sandbox.rs | head`
Expected: shows the real constructor for a no-op sandbox. If the helper is named differently (e.g. `NoSandbox::new()`), update the `sandbox:` line in `tool_ctx()` accordingly before running tests.

- [ ] **Step 3: Run tests to verify they pass**

Run: `source ~/.cargo/env && cargo test -p agent-core context_tools:: 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add agent/crates/agent-core/src/context_tools.rs agent/crates/agent-core/src/lib.rs
git commit -m "feat(context): context_recall + context_compact model tools"
```

---

### Task 7: End-to-end wiring (assemble + frontends)

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (create shared store + flag, register context tools, expose on `BuiltLoop`)
- Modify: `agent/crates/agent-cli/src/main.rs` (construct `CuratedContext`)
- Modify: `agent/crates/agent-server/src/session.rs` (construct `CuratedContext`)

**Interfaces:**
- Consumes: `context_tools` (Task 6), `InMemoryOffloadStore` (Task 1), `CuratedContext` (Task 3).
- Produces: `BuiltLoop` gains `pub offload_store: Arc<dyn agent_core::OffloadStore>` and `pub compact_flag: Arc<std::sync::atomic::AtomicBool>`; frontends build `CuratedContext::new(system, store, flag)`.
- Note: this is the only cross-crate task. The loop already accepts `&mut dyn ContextManager`, so swapping the concrete type at the frontends is mechanical.

- [ ] **Step 1: Register context tools in `assemble_loop` and expose handles**

In `agent/crates/agent-runtime-config/src/assemble.rs`:

Add to the `BuiltLoop` struct (near `pub loop_: Arc<AgentLoop>`):

```rust
    pub offload_store: Arc<dyn agent_core::OffloadStore>,
    pub compact_flag: Arc<std::sync::atomic::AtomicBool>,
```

In `assemble_loop`, after the registry is created and before `registered_names` is computed, register the context tools:

```rust
    let offload_store: Arc<dyn agent_core::OffloadStore> =
        Arc::new(agent_core::InMemoryOffloadStore::new());
    let compact_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    for t in agent_core::context_tools(offload_store.clone(), compact_flag.clone()) {
        registry.register(t);
    }
```

Add the two fields to the `BuiltLoop { ... }` constructor at the end of the function:

```rust
        offload_store,
        compact_flag,
```

- [ ] **Step 2: Add an assemble test that the context tools register**

In the `tests` module of `assemble.rs`, add:

```rust
    #[test]
    fn registers_context_management_tools() {
        let c = base_config(); // use whatever the existing tests call to get a RuntimeConfig
        let dir = tempfile::tempdir().unwrap();
        let built = assemble_loop(&c, parts(dir.path().to_path_buf(), vec![]));
        assert!(built.registered_names.iter().any(|n| n == "context_recall"));
        assert!(built.registered_names.iter().any(|n| n == "context_compact"));
    }
```

(Match the existing tests' config/`parts` helpers — adapt `base_config()` to the real helper name used at `assemble.rs` test module, visible via `grep -n "fn parts\|RuntimeConfig" assemble.rs`.)

- [ ] **Step 3: Run the assemble test**

Run: `source ~/.cargo/env && cargo test -p agent-runtime-config registers_context_management_tools 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 4: Switch the CLI frontend to `CuratedContext`**

In `agent/crates/agent-cli/src/main.rs`, change the import:

```rust
use agent_core::CuratedContext;
```

Replace the `WindowContext` construction (lines ~227-228) with — using the handles from the built loop (the variable holding the `BuiltLoop`; named `built` in this file per the existing `built.system_prompt`):

```rust
    let mut ctx = CuratedContext::new(
        Message::system(built.system_prompt),
        built.offload_store.clone(),
        built.compact_flag.clone(),
    )
    .with_recall_budget(memory.recall_token_budget);
```

(If the `BuiltLoop` is bound to a different variable name here, use that name. Confirm with `grep -n "assemble_loop\|\.system_prompt" agent/crates/agent-cli/src/main.rs`.)

- [ ] **Step 5: Switch the server session to `CuratedContext`**

In `agent/crates/agent-server/src/session.rs`:

Change the import:

```rust
use agent_core::{ContextManager, CuratedContext};
```

Change the field type:

```rust
    ctx: Arc<AsyncMutex<CuratedContext>>,
```

The struct must now also hold the shared handles so it can rebuild on system-prompt change. Add fields:

```rust
    offload_store: Arc<dyn agent_core::OffloadStore>,
    compact_flag: Arc<std::sync::atomic::AtomicBool>,
```

At construction (line ~42), populate them from the built loop and build the context:

```rust
            ctx: Arc::new(AsyncMutex::new(
                CuratedContext::new(
                    Message::system(params.system_prompt.clone()),
                    params.offload_store.clone(),
                    params.compact_flag.clone(),
                )
                .with_recall_budget(params.recall_token_budget),
            )),
            offload_store: params.offload_store.clone(),
            compact_flag: params.compact_flag.clone(),
```

(`params` must carry `offload_store` and `compact_flag`. Thread them from wherever `assemble_loop`'s `BuiltLoop` is constructed in the server bring-up — add the two fields to the session-params struct and pass `built.offload_store` / `built.compact_flag`. Find the params struct with `grep -n "struct.*Params\|recall_token_budget" agent/crates/agent-server/src/*.rs`.)

At the system-prompt-change rebuild (line ~100):

```rust
        *guard = CuratedContext::new(
            Message::system(self.runtime.current_system_prompt()),
            self.offload_store.clone(),
            self.compact_flag.clone(),
        )
        .with_recall_budget(self.recall_budget);
```

- [ ] **Step 6: Build the workspace and run the full test suite**

Run: `source ~/.cargo/env && cargo build --workspace 2>&1 | tail -20`
Expected: builds. Fix any frontend type mismatches surfaced (the compiler points at the exact line).

Run: `source ~/.cargo/env && cargo test --workspace 2>&1 | tail -30`
Expected: all tests PASS, including existing server/CLI tests.

- [ ] **Step 7: Commit**

```bash
git add agent/crates/agent-runtime-config/src/assemble.rs agent/crates/agent-cli/src/main.rs agent/crates/agent-server/src/session.rs
git commit -m "feat(context): wire CuratedContext + context tools through assemble and frontends"
```

---

### Task 8: Guidance skill `.agents/skills/context-management/SKILL.md`

**Files:**
- Create: `.agents/skills/context-management/SKILL.md`

**Interfaces:**
- Consumes: nothing (documentation). Describes the behavior built in Tasks 1-7.

- [ ] **Step 1: Write the skill**

Create `.agents/skills/context-management/SKILL.md`:

```markdown
---
name: context-management
description: >-
  Use when an agent's context window is filling up, when you see a
  `[tool_result#N offloaded ...]` placeholder, or when recent actions feel like
  they've drifted from the original goal. Explains how to wield this runtime's
  curating context manager (offload table, compaction, re-grounding) well —
  what to trust, when to recall, and when to compact.
---

# Using the context manager well

The runtime curates its own context window every turn: it offloads stale tool
errors and large tool outputs into a retrievable table, compacts old history into
a summary when the window fills, and pins a re-grounding block restating the
original goal. This skill is judgment: how to work *with* that machinery.

## The one big idea

**The live window is a working set, not a transcript.** The offload table is the
durable record. Pull detail back only when you actually need it.

## Non-negotiables

- **A `[tool_result#N offloaded ...]` stub is a pointer, not a loss.** Call
  `context_recall(N)` to rehydrate the full content. Never re-run a tool to
  regenerate output you already produced and offloaded.
- **Compaction is lossy on specifics.** The "Summary of earlier conversation"
  block keeps decisions and open threads, but drops verbatim detail. Before you
  rely on an exact value (a path, a number, an error message) for an edit,
  `context_recall` the raw entry by its id.
- **The re-grounding block is the original goal.** If your last few actions don't
  serve "Original goal: ...", you've drifted — stop and re-read it before acting.
- **Don't hoard.** Recall the one entry you need, not everything. Mass-recall
  refills the window with the bloat the offload pass just cleared.

## When to let it work vs. intervene

- **Let it work:** routine offload and windowing are automatic and safe — you do
  not need to manage them. Most turns need nothing from you.
- **Request `context_compact()`** only at a genuine high-water point or right
  before starting a long new sub-task, when the window is full of *resolved*
  sub-tasks whose detail you won't need verbatim.
- **Keep writes single-threaded.** If you delegate exploration to a subagent, let
  it explore read-only and hand back a short summary; don't fan out parallel
  writers (they disperse decisions and lose shared context).

## Reading the signals

- `Offloaded { id, bytes, tool }` — content moved to the table; recall by `id`.
- `Compacted { turns_replaced, tokens_before, tokens_after }` — old span summarized.
- `CompactionFailed { reason }` — nothing changed; the window still holds raw history.

## Red flags

| Thought | Reality |
|---------|---------|
| "I'll just re-run the tool to see that output again" | It's offloaded — `context_recall(N)` is cheaper and exact. |
| "The summary says the path was X, so I'll edit X" | Compaction is lossy — recall the raw entry by id and confirm. |
| "Context is full, I should give up" | Call `context_compact()` and continue. |
| "Let me recall everything to be safe" | That refills the window — recall only what this step needs. |
```

- [ ] **Step 2: Verify the skill file is well-formed**

Run: `source ~/.cargo/env; head -12 .agents/skills/context-management/SKILL.md`
Expected: shows the frontmatter block with `name:` and `description:`.

- [ ] **Step 3: Commit**

```bash
git add .agents/skills/context-management/SKILL.md
git commit -m "docs(skill): context-management guidance skill"
```

---

## Final verification

- [ ] **Run the whole workspace test suite:**

Run: `source ~/.cargo/env && cargo test --workspace 2>&1 | tail -30`
Expected: all PASS, including the pre-existing `WindowContext` tests (backward-compat proof).

- [ ] **Confirm no clippy regressions on the touched crates:**

Run: `source ~/.cargo/env && cargo clippy -p agent-core -p agent-runtime-config 2>&1 | tail -20`
Expected: no new warnings introduced by these files.
```
