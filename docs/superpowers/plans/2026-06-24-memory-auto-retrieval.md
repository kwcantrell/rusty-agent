# Memory Auto-Retrieval Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Each user turn, automatically retrieve memories relevant to the user's input and inject them into the context window as a distinct recall block, before the model runs.

**Architecture:** A new `Retriever` port trait in `agent-core` (no dependency on `agent-memory`). `agent-memory` provides a `MemoryRetriever` implementing it, reusing the `recall` tool's query machinery via a shared fn. `AgentLoop` gains an optional retriever attached via a builder; at the top of `run()` it retrieves and hands lines to `WindowContext::set_recall`, which `build()` emits as a budgeted system-role block after the system prompt. `agent-runtime-config` builds the retriever sharing the same store/embedder as the memory tools; the CLI wires it in.

**Tech Stack:** Rust, tokio, async-trait, the existing `agent-memory` vector store (cosine over BGE-Small embeddings).

## Global Constraints

- `cargo` is not on PATH in this repo — every shell step must run `source ~/.cargo/env` first.
- Dependency direction: `agent-memory` → `agent-core` is allowed (no cycle; `agent-core` never depends on `agent-memory`). `agent-runtime-config` → `agent-core` is allowed.
- Retrieval MUST NOT break a turn: all embed/store errors are swallowed inside the retriever, logged at `warn`, and yield an empty `Vec<String>`.
- Zero behavior change when no retriever is attached: `set_recall` is a default no-op on the trait; `AgentLoop` defaults `retriever: None`. All existing tests must compile and pass untouched.
- Reuse `MemoryConfig` fields `default_k` (=5) and `relevance_threshold` (=0.3); scope filter is `ScopeFilter::ProjectAndGlobal` (mirrors the `recall` tool).

### Deviation from spec (server path)

The spec says both production callers are covered. In reality `agent-server/src/setup.rs:27` passes an **empty** `memory_tools` vec — the desktop bridge does not wire memory at all today. So live auto-retrieval is wired only in the **CLI** (`agent-cli/src/main.rs`). The `agent-runtime-config` builder (Task 5) is reusable, so the server can adopt it when memory is enabled there. No server-side code is changed in this plan.

---

### Task 1: `Retriever` port trait in `agent-core`

**Files:**
- Create: `agent/crates/agent-core/src/recall.rs`
- Modify: `agent/crates/agent-core/src/lib.rs:1-8`
- Test: `agent/crates/agent-core/src/recall.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces: `pub trait Retriever: Send + Sync { async fn retrieve(&self, query: &str) -> Vec<String>; }` (via `#[async_trait]`), re-exported at crate root as `agent_core::Retriever`.

- [ ] **Step 1: Write the failing test**

Create `agent/crates/agent-core/src/recall.rs`:

```rust
use async_trait::async_trait;

/// Port for pulling relevant long-term memories into context at the start of a turn.
/// Implemented by `agent-memory`'s `MemoryRetriever`; defined here so `agent-core`
/// has no dependency on the memory crate.
///
/// Implementations MUST swallow their own errors and return an empty `Vec` on
/// failure — retrieval must never break a turn.
#[async_trait]
pub trait Retriever: Send + Sync {
    /// Return memory facts relevant to `query`, best-first, one plain string per
    /// memory (no formatting — the context manager owns presentation).
    async fn retrieve(&self, query: &str) -> Vec<String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct Two;
    #[async_trait]
    impl Retriever for Two {
        async fn retrieve(&self, _q: &str) -> Vec<String> {
            vec!["a".into(), "b".into()]
        }
    }

    #[tokio::test]
    async fn retriever_is_object_safe_and_returns_lines() {
        let r: Arc<dyn Retriever> = Arc::new(Two);
        assert_eq!(r.retrieve("q").await, vec!["a".to_string(), "b".to_string()]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `source ~/.cargo/env && cargo test -p agent-core retriever_is_object_safe -v`
Expected: FAIL to compile — `recall` module not declared / `Retriever` unresolved.

- [ ] **Step 3: Declare and export the module**

Modify `agent/crates/agent-core/src/lib.rs` — add the module and re-export. The file becomes:

```rust
//! Agent loop, context manager, and event model.
mod event;
mod context;
mod loop_;
mod recall;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
pub use context::*;
pub use event::*;
pub use loop_::*;
pub use recall::*;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `source ~/.cargo/env && cargo test -p agent-core retriever_is_object_safe -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
source ~/.cargo/env
git add agent/crates/agent-core/src/recall.rs agent/crates/agent-core/src/lib.rs
git commit -m "feat(agent-core): add Retriever port trait for memory auto-retrieval"
```

---

### Task 2: Budgeted recall block in `WindowContext`

**Files:**
- Modify: `agent/crates/agent-core/src/context.rs:19-23` (trait), `:27-67` (WindowContext)
- Test: `agent/crates/agent-core/src/context.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: `Message`, `estimate_tokens`, `message_tokens` (existing in this file).
- Produces: `ContextManager::set_recall(&mut self, items: Vec<String>)` (default no-op); `WindowContext::with_recall_budget(self, budget: usize) -> Self`; `pub const DEFAULT_RECALL_TOKEN_BUDGET: usize = 512`. `build()` now emits a recall block as the 2nd message when recall lines are present.

- [ ] **Step 1: Write the failing tests**

Add these tests to the `#[cfg(test)] mod tests` block in `agent/crates/agent-core/src/context.rs`:

```rust
    #[test]
    fn set_recall_injects_block_after_system_before_history() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        ctx.append(Message::user("hello"));
        ctx.set_recall(vec!["user likes rust".into(), "project uses tokio".into()]);
        let built = ctx.build(100_000);
        assert!(matches!(built[0].role, Role::System)); // system first
        assert_eq!(built[1].content,
            "Relevant memories from past sessions:\n- user likes rust\n- project uses tokio");
        assert!(matches!(built[1].role, Role::System));  // recall block is system-role
        assert_eq!(built.last().unwrap().content, "hello"); // history after recall
    }

    #[test]
    fn empty_recall_injects_no_block() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        ctx.append(Message::user("hello"));
        ctx.set_recall(vec![]);
        let built = ctx.build(100_000);
        assert_eq!(built.len(), 2); // system + history only
    }

    #[test]
    fn set_recall_replaces_previous_lines() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        ctx.set_recall(vec!["old".into()]);
        ctx.set_recall(vec!["new".into()]);
        let built = ctx.build(100_000);
        assert!(built[1].content.contains("new"));
        assert!(!built[1].content.contains("old"));
    }

    #[test]
    fn recall_block_is_capped_by_budget() {
        // 30 long lines vastly exceed a 64-token budget; the block must stay under it
        // (plus the soft floor of one line) — never inject all 30.
        let mut ctx = WindowContext::new(Message::system("SYS")).with_recall_budget(64);
        let lines: Vec<String> = (0..30)
            .map(|i| format!("memory fact number {i} with a fair amount of padding text"))
            .collect();
        ctx.set_recall(lines);
        let built = ctx.build(100_000);
        let block = &built[1].content;
        // Far fewer than 30 lines survived.
        assert!(block.matches("\n- ").count() < 30);
        assert!(block.starts_with("Relevant memories from past sessions:"));
    }

    #[test]
    fn history_is_evicted_before_recall_and_system() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        for i in 0..50 {
            ctx.append(Message::user(format!("message number {i} with some padding text")));
        }
        ctx.set_recall(vec!["pinned memory".into()]);
        let built = ctx.build(40); // tiny limit forces history eviction
        assert!(matches!(built[0].role, Role::System));
        assert!(built[1].content.contains("pinned memory")); // recall survives
        assert!(built.len() < 51);                            // history evicted
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `source ~/.cargo/env && cargo test -p agent-core set_recall -v`
Expected: FAIL to compile — `set_recall` / `with_recall_budget` not found.

- [ ] **Step 3: Implement the recall block**

In `agent/crates/agent-core/src/context.rs`:

Add the constant just above the trait (after `built_tokens`):

```rust
/// Default cap on the auto-retrieval recall block, in estimated tokens. Keeps
/// recall from crowding out conversation history. Override per-context with
/// `WindowContext::with_recall_budget`.
pub const DEFAULT_RECALL_TOKEN_BUDGET: usize = 512;
```

Add the default no-op method to the trait:

```rust
pub trait ContextManager: Send + Sync {
    fn append(&mut self, msg: Message);
    fn build(&self, model_limit: usize) -> Vec<Message>;
    fn set_system(&mut self, system: Message);
    /// Replace the auto-retrieved recall lines surfaced this turn. Default no-op
    /// so non-memory implementations are unaffected.
    fn set_recall(&mut self, _items: Vec<String>) {}
}
```

Replace the `WindowContext` struct, `new`, and the `ContextManager for WindowContext` impl with:

```rust
/// Sliding-window context: always keeps the system prompt; evicts oldest
/// history turns until the estimate fits `model_limit`. An optional recall block
/// (auto-retrieved memories) sits right after the system prompt, capped at
/// `recall_budget` tokens so it can never starve history.
pub struct WindowContext {
    system: Message,
    history: Vec<Message>,
    recall: Vec<String>,
    recall_budget: usize,
}

impl WindowContext {
    pub fn new(system: Message) -> Self {
        Self {
            system,
            history: Vec::new(),
            recall: Vec::new(),
            recall_budget: DEFAULT_RECALL_TOKEN_BUDGET,
        }
    }

    /// Override the recall-block token cap (default `DEFAULT_RECALL_TOKEN_BUDGET`).
    pub fn with_recall_budget(mut self, budget: usize) -> Self {
        self.recall_budget = budget;
        self
    }

    /// Build the recall block message, greedily keeping lines under `recall_budget`.
    /// Always includes at least the first line if any are present (soft cap).
    fn recall_message(&self) -> Option<Message> {
        if self.recall.is_empty() {
            return None;
        }
        const HEADER: &str = "Relevant memories from past sessions:";
        let mut body = String::from(HEADER);
        for line in &self.recall {
            let candidate = format!("{body}\n- {line}");
            if estimate_tokens(&candidate) > self.recall_budget && body != HEADER {
                break;
            }
            body = candidate;
        }
        Some(Message::system(body))
    }
}

impl ContextManager for WindowContext {
    fn append(&mut self, msg: Message) {
        self.history.push(msg);
    }

    fn set_system(&mut self, system: Message) {
        self.system = system;
    }

    fn set_recall(&mut self, items: Vec<String>) {
        self.recall = items;
    }

    fn build(&self, model_limit: usize) -> Vec<Message> {
        let sys_tokens = message_tokens(&self.system);
        let recall_msg = self.recall_message();
        let recall_tokens = recall_msg.as_ref().map(message_tokens).unwrap_or(0);
        let budget = model_limit
            .saturating_sub(sys_tokens)
            .saturating_sub(recall_tokens);
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
        let mut out = Vec::with_capacity(kept_rev.len() + 2);
        out.push(self.system.clone());
        if let Some(m) = recall_msg {
            out.push(m);
        }
        out.extend(kept_rev);
        out
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `source ~/.cargo/env && cargo test -p agent-core context -v`
Expected: PASS — new recall tests plus all existing `WindowContext` tests (`build_keeps_system_and_drops_oldest_when_over_limit`, etc.).

- [ ] **Step 5: Commit**

```bash
source ~/.cargo/env
git add agent/crates/agent-core/src/context.rs
git commit -m "feat(agent-core): budgeted recall block in WindowContext"
```

---

### Task 3: Attach retriever to `AgentLoop` and retrieve per turn

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs:1` (import), `:52-74` (struct + new), `:125-127` (run)
- Test: `agent/crates/agent-core/src/loop_.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `Retriever` (Task 1), `ContextManager::set_recall` (Task 2).
- Produces: `AgentLoop::with_retriever(self, r: Arc<dyn Retriever>) -> Self`. `run()` now calls `retrieve` before appending the user message.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `agent/crates/agent-core/src/loop_.rs`:

```rust
    struct FakeRetriever(Vec<String>);
    #[async_trait::async_trait]
    impl crate::Retriever for FakeRetriever {
        async fn retrieve(&self, _q: &str) -> Vec<String> { self.0.clone() }
    }

    #[tokio::test]
    async fn auto_retrieval_injects_recall_block_into_context() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let model = Arc::new(ScriptedModel::new(vec![Scripted::Text("ok".into())]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 2, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60), ..Default::default() })
            .with_retriever(Arc::new(FakeRetriever(vec!["user prefers rust 2021".into()])));
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "hello".into()).await.unwrap();

        let built = ctx.build(100_000);
        assert!(built.iter().any(|m|
            m.content.contains("Relevant memories from past sessions:")
            && m.content.contains("user prefers rust 2021")));
    }

    #[tokio::test]
    async fn empty_retrieval_injects_no_block_and_turn_completes() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let model = Arc::new(ScriptedModel::new(vec![Scripted::Text("ok".into())]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 2, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60), ..Default::default() })
            .with_retriever(Arc::new(FakeRetriever(vec![])));
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "hello".into()).await.unwrap();

        let built = ctx.build(100_000);
        assert!(!built.iter().any(|m| m.content.contains("Relevant memories")));
        assert!(sink.events.lock().unwrap().last().unwrap() == "done");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `source ~/.cargo/env && cargo test -p agent-core auto_retrieval -v`
Expected: FAIL to compile — `with_retriever` not found.

- [ ] **Step 3: Add the field, builder, and import**

In `agent/crates/agent-core/src/loop_.rs`, change line 1 to also import `Retriever`:

```rust
use crate::{built_tokens, AgentEvent, ContextManager, EventSink, Retriever};
```

Add the field to `AgentLoop` (after `config: LoopConfig,`):

```rust
pub struct AgentLoop {
    model: Arc<dyn ModelClient>,
    protocol: Arc<dyn ToolCallProtocol>,
    tools: Arc<ToolRegistry>,
    policy: Arc<dyn PolicyEngine>,
    approval: Arc<dyn ApprovalChannel>,
    sink: Arc<dyn EventSink>,
    config: LoopConfig,
    retriever: Option<Arc<dyn Retriever>>,
}
```

Update `new` to default the field and add the builder (replace the `Self { ... }` line in `new` and add `with_retriever` right after `new`):

```rust
        Self { model, protocol, tools, policy, approval, sink, config, retriever: None }
    }

    /// Attach a memory retriever. When set, each turn auto-retrieves relevant
    /// memories and injects them into the context before the model runs.
    pub fn with_retriever(mut self, retriever: Arc<dyn Retriever>) -> Self {
        self.retriever = Some(retriever);
        self
    }
```

- [ ] **Step 4: Call retrieval at the top of `run()`**

In `run()`, replace line 127 (`ctx.append(Message::user(user_input));`) with retrieval first, then append (borrow before move):

```rust
        if let Some(retriever) = &self.retriever {
            let lines = retriever.retrieve(&user_input).await;
            if !lines.is_empty() {
                ctx.set_recall(lines);
            }
        }
        ctx.append(Message::user(user_input));
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `source ~/.cargo/env && cargo test -p agent-core -v`
Expected: PASS — new `auto_retrieval` / `empty_retrieval` tests and the full existing `agent-core` suite (regression: default `AgentLoop` has `retriever: None`).

- [ ] **Step 6: Commit**

```bash
source ~/.cargo/env
git add agent/crates/agent-core/src/loop_.rs
git commit -m "feat(agent-core): per-turn auto-retrieval via optional Retriever"
```

---

### Task 4: `MemoryRetriever` + shared query fn + config fields in `agent-memory`

**Files:**
- Modify: `agent/crates/agent-memory/Cargo.toml` (add `agent-core` dep)
- Modify: `agent/crates/agent-memory/src/config.rs:4-18` (struct), `:24-44` (Default + test)
- Modify: `agent/crates/agent-memory/src/tools.rs` (extract `query_memories`; reuse in `Recall`)
- Create: `agent/crates/agent-memory/src/retriever.rs`
- Modify: `agent/crates/agent-memory/src/lib.rs` (declare module, export)
- Test: inline in `retriever.rs` and `config.rs`

**Interfaces:**
- Consumes: `Embedder`, `MemoryStore`, `MemoryConfig`, `Scored`, `ScopeFilter`, `agent_core::Retriever`.
- Produces:
  - `pub(crate) async fn query_memories(embedder: &dyn Embedder, store: &dyn MemoryStore, cfg: &MemoryConfig, project_key: &str, query: &str, k: usize) -> Result<Vec<Scored>, ToolError>` in `tools.rs`.
  - `pub struct MemoryRetriever { embedder, store, cfg, project_key }` implementing `agent_core::Retriever`.
  - `MemoryConfig.auto_recall: bool` (default `true`), `MemoryConfig.recall_token_budget: usize` (default `512`).

- [ ] **Step 1: Add config fields (failing test first)**

In `agent/crates/agent-memory/src/config.rs`, add to the `defaults_match_spec_table` test:

```rust
        assert!(c.auto_recall);
        assert_eq!(c.recall_token_budget, 512);
```

- [ ] **Step 2: Run to verify it fails**

Run: `source ~/.cargo/env && cargo test -p agent-memory defaults_match_spec_table -v`
Expected: FAIL to compile — no field `auto_recall`.

- [ ] **Step 3: Implement config fields**

In `agent/crates/agent-memory/src/config.rs`, add to the struct (after `candidate_warn_threshold`):

```rust
    pub auto_recall: bool,
    pub recall_token_budget: usize,
```

And to `Default` (after `candidate_warn_threshold: 50_000,`):

```rust
            auto_recall: true,
            recall_token_budget: 512,
```

- [ ] **Step 4: Run to verify config test passes**

Run: `source ~/.cargo/env && cargo test -p agent-memory defaults_match_spec_table -v`
Expected: PASS

- [ ] **Step 5: Extract the shared `query_memories` fn**

In `agent/crates/agent-memory/src/tools.rs`, add this free fn near the other helpers (after `first_embedding`):

```rust
/// Shared retrieval core for the `recall` tool and auto-retrieval: embed the
/// query, search the project+global scope, drop sub-threshold hits, keep top-k.
pub(crate) async fn query_memories(
    embedder: &dyn Embedder,
    store: &dyn MemoryStore,
    cfg: &MemoryConfig,
    project_key: &str,
    query: &str,
    k: usize,
) -> Result<Vec<crate::record::Scored>, ToolError> {
    let qv = first_embedding(embedder.embed(&[query.to_string()]).await.map_err(embed_failed)?)?;
    let filter = ScopeFilter::ProjectAndGlobal { project_key: project_key.to_string() };
    let mut hits = store.query(&qv, cfg.max_k, &filter).await.map_err(store_failed)?;
    hits.retain(|h| h.score >= cfg.relevance_threshold);
    hits.truncate(k);
    Ok(hits)
}
```

Then rewrite the body of `Recall::execute` to use it (replace the embed/query/retain/truncate block, lines ~170-180, keeping the surrounding arg parsing, logging, and `render_hits`):

```rust
    async fn execute(&self, args: Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let query = args.get("query").and_then(Value::as_str)
            .map(str::trim).filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::InvalidArgs("missing non-empty 'query'".into()))?;
        let k = args.get("k").and_then(Value::as_u64).map(|n| n as usize)
            .unwrap_or(self.cfg.default_k).clamp(1, self.cfg.max_k);
        let hits = query_memories(
            self.embedder.as_ref(), self.store.as_ref(), &self.cfg, &self.project_key, query, k,
        ).await?;
        tracing::info!(target: "memory", returned = hits.len(),
            top = hits.first().map(|h| h.score).unwrap_or(0.0), "recall");
        if hits.is_empty() {
            return Ok(ToolOutput { content: "No relevant memories found.".into(), display: None });
        }
        let body = render_hits(&hits, self.cfg.max_recall_chars);
        Ok(ToolOutput { content: body, display: None })
    }
```

- [ ] **Step 6: Run existing recall tests (refactor must not change behavior)**

Run: `source ~/.cargo/env && cargo test -p agent-memory recall -v`
Expected: PASS — existing `recall_tests` still pass against the extracted fn.

- [ ] **Step 7: Add `agent-core` dependency**

In `agent/crates/agent-memory/Cargo.toml`, under `[dependencies]`, add:

```toml
agent-core = { path = "../agent-core" }
```

- [ ] **Step 8: Write the failing `MemoryRetriever` test**

Create `agent/crates/agent-memory/src/retriever.rs`:

```rust
use crate::config::MemoryConfig;
use crate::embedder::Embedder;
use crate::store::MemoryStore;
use crate::tools::query_memories;
use agent_core::Retriever;
use async_trait::async_trait;
use std::sync::Arc;

/// Auto-retrieval adapter: implements `agent_core::Retriever` by running the same
/// query the `recall` tool runs, returning plain fact strings (no formatting).
pub struct MemoryRetriever {
    pub embedder: Arc<dyn Embedder>,
    pub store: Arc<dyn MemoryStore>,
    pub cfg: Arc<MemoryConfig>,
    pub project_key: String,
}

#[async_trait]
impl Retriever for MemoryRetriever {
    async fn retrieve(&self, query: &str) -> Vec<String> {
        match query_memories(
            self.embedder.as_ref(), self.store.as_ref(), &self.cfg,
            &self.project_key, query, self.cfg.default_k,
        ).await {
            Ok(hits) => hits.into_iter().map(|h| h.record.text).collect(),
            Err(e) => {
                tracing::warn!(target: "memory", "auto-retrieval failed: {e}");
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{MemoryRecord, MemoryScope, now_secs};
    use crate::store::{InMemoryStore, MemoryStore};
    use crate::embedder::{Embedder, StubEmbedder};

    async fn seed(store: &InMemoryStore, embedder: &dyn Embedder, key: &str, text: &str) {
        let v = embedder.embed(&[text.to_string()]).await.unwrap().remove(0);
        store.insert(MemoryRecord {
            id: uuid::Uuid::new_v4().to_string(), text: text.into(),
            scope: MemoryScope::Project(key.into()), tags: vec![], vector: v,
            created_at: now_secs(), updated_at: now_secs(), source: "test".into(),
        }).await.unwrap();
    }

    #[tokio::test]
    async fn retrieve_returns_plain_fact_lines() {
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
        let store = Arc::new(InMemoryStore::new());
        seed(&store, embedder.as_ref(), "K", "user prefers rust").await;
        let r = MemoryRetriever {
            embedder: embedder.clone(), store: store.clone(),
            cfg: Arc::new(MemoryConfig::default()), project_key: "K".into(),
        };
        let lines = r.retrieve("what language?").await;
        assert!(lines.iter().any(|l| l == "user prefers rust"));
        // Plain text only — no score/age/tag formatting.
        assert!(lines.iter().all(|l| !l.starts_with('[')));
    }

    #[tokio::test]
    async fn retrieve_is_empty_when_store_is_empty() {
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
        let store = Arc::new(InMemoryStore::new());
        let r = MemoryRetriever {
            embedder, store, cfg: Arc::new(MemoryConfig::default()), project_key: "K".into(),
        };
        assert!(r.retrieve("anything").await.is_empty());
    }
}
```

> **Note for implementer:** verify the exact `InMemoryStore` insert method name and `MemoryRecord` field set against `agent/crates/agent-memory/src/store.rs` and `record.rs` before running — adjust the `seed` helper if the insert method differs (e.g. `upsert`). The `MemoryRecord` fields are listed in `record.rs` (`id, text, scope, tags, vector, created_at, updated_at, source`).

- [ ] **Step 9: Declare and export the module**

In `agent/crates/agent-memory/src/lib.rs`, add `mod retriever;` with the other `mod` lines and export:

```rust
pub use retriever::MemoryRetriever;
```

- [ ] **Step 10: Run the tests to verify they pass**

Run: `source ~/.cargo/env && cargo test -p agent-memory retriev -v`
Expected: PASS — `retrieve_returns_plain_fact_lines`, `retrieve_is_empty_when_store_is_empty`.

- [ ] **Step 11: Commit**

```bash
source ~/.cargo/env
git add agent/crates/agent-memory/
git commit -m "feat(agent-memory): MemoryRetriever + shared query_memories + recall config"
```

---

### Task 5: Build the retriever in `agent-runtime-config`

**Files:**
- Modify: `agent/crates/agent-runtime-config/Cargo.toml` (add `agent-core` dep)
- Modify: `agent/crates/agent-memory/src/lib.rs` (add `build_tools_and_retriever`)
- Modify: `agent/crates/agent-runtime-config/src/lib.rs:16` (import), `:99-122` (add builder)
- Test: inline in `agent/crates/agent-runtime-config/src/lib.rs`

**Interfaces:**
- Consumes: `agent_memory::build_tools_and_retriever`, `agent_core::Retriever`.
- Produces:
  - `agent_memory::build_tools_and_retriever(cfg, workspace) -> Result<(Vec<Arc<dyn Tool>>, Arc<dyn agent_core::Retriever>), MemoryInitError>`.
  - `agent_runtime_config::MemoryBuild { tools: Vec<Arc<dyn Tool>>, retriever: Option<Arc<dyn agent_core::Retriever>>, recall_token_budget: usize }`.
  - `agent_runtime_config::build_memory_full(enabled, db_path, model_dir, workspace) -> MemoryBuild`.

- [ ] **Step 1: Add `build_tools_and_retriever` to `agent-memory`**

In `agent/crates/agent-memory/src/lib.rs`, add after `build_tools`:

```rust
/// Like `build_tools`, but also returns a `MemoryRetriever` sharing the SAME
/// store + embedder, for auto-retrieval. Errors disable memory (caller falls back).
pub fn build_tools_and_retriever(
    cfg: MemoryConfig,
    workspace: &Path,
) -> Result<(Vec<Arc<dyn agent_tools::Tool>>, Arc<dyn agent_core::Retriever>), MemoryInitError> {
    let store: Arc<dyn MemoryStore> =
        Arc::new(SqliteStore::open(&cfg.db_path).map_err(|e| MemoryInitError::Store(e.to_string()))?);
    #[cfg(feature = "onnx")]
    let embedder: Arc<dyn Embedder> = Arc::new(
        embedder::FastEmbedEmbedder::new(&cfg).map_err(|e| MemoryInitError::Embedder(e.to_string()))?);
    #[cfg(not(feature = "onnx"))]
    let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
    let scope = project_scope(workspace);
    let key = match &scope {
        MemoryScope::Project(k) => k.clone(),
        MemoryScope::Global => String::new(),
    };
    let cfg = Arc::new(cfg);
    let tools = build_tools_with(embedder.clone(), store.clone(), cfg.clone(), scope);
    let retriever: Arc<dyn agent_core::Retriever> = Arc::new(retriever::MemoryRetriever {
        embedder, store, cfg, project_key: key,
    });
    Ok((tools, retriever))
}
```

> Add `agent-core = { path = "../agent-core" }` to `agent/crates/agent-memory/Cargo.toml` if Task 4 Step 7 has not already (it has). `retriever` module must be `mod retriever;` (declared in Task 4); reference it as `retriever::MemoryRetriever`.

- [ ] **Step 2: Write the failing runtime-config test**

In `agent/crates/agent-runtime-config/src/lib.rs` test module, add:

```rust
    #[test]
    fn build_memory_full_disabled_has_no_retriever() {
        let mb = build_memory_full(false, None, None, std::path::Path::new("/tmp/ws"));
        assert!(mb.tools.is_empty());
        assert!(mb.retriever.is_none());
        assert_eq!(mb.recall_token_budget, 512);
    }

    #[test]
    fn build_memory_full_enabled_has_retriever_and_tools() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("memory.db");
        let mb = build_memory_full(true, Some(db), None, tmp.path());
        assert_eq!(mb.tools.len(), 3);
        assert!(mb.retriever.is_some());
    }
```

- [ ] **Step 3: Run to verify it fails**

Run: `source ~/.cargo/env && cargo test -p agent-runtime-config build_memory_full -v`
Expected: FAIL to compile — `build_memory_full` / `MemoryBuild` not found.

- [ ] **Step 4: Implement `MemoryBuild` + `build_memory_full`**

In `agent/crates/agent-runtime-config/Cargo.toml`, add under `[dependencies]`:

```toml
agent-core = { path = "../agent-core" }
```

In `agent/crates/agent-runtime-config/src/lib.rs`, change the memory import (line 16):

```rust
use agent_memory::{build_tools, build_tools_and_retriever, MemoryConfig};
```

Add after `build_memory` (around line 122):

```rust
/// Result of building memory with auto-retrieval: the tools to register, an
/// optional retriever to attach to the loop, and the recall-block token budget.
pub struct MemoryBuild {
    pub tools: Vec<Arc<dyn Tool>>,
    pub retriever: Option<Arc<dyn agent_core::Retriever>>,
    pub recall_token_budget: usize,
}

/// Build memory tools AND an auto-retrieval retriever sharing the same store/embedder.
/// Disabled, `auto_recall = false`, or a build failure all yield `retriever: None`
/// (memory is best-effort — never fatal). `recall_token_budget` always reflects config.
pub fn build_memory_full(
    enabled: bool,
    db_path: Option<PathBuf>,
    model_dir: Option<PathBuf>,
    workspace: &Path,
) -> MemoryBuild {
    let mut cfg = MemoryConfig::default();
    if let Some(p) = db_path {
        cfg.db_path = p;
    }
    cfg.model_cache_dir = model_dir;
    let recall_token_budget = cfg.recall_token_budget;
    let auto_recall = cfg.auto_recall;

    if !enabled {
        return MemoryBuild { tools: Vec::new(), retriever: None, recall_token_budget };
    }
    match build_tools_and_retriever(cfg, workspace) {
        Ok((tools, retriever)) => MemoryBuild {
            tools,
            retriever: if auto_recall { Some(retriever) } else { None },
            recall_token_budget,
        },
        Err(e) => {
            tracing::warn!(target: "memory", "disabled: {e}");
            MemoryBuild { tools: Vec::new(), retriever: None, recall_token_budget }
        }
    }
}
```

> `build_memory` (the original) stays as-is for back-compat; `build_tools` import is retained because the original still uses it.

- [ ] **Step 5: Run to verify it passes**

Run: `source ~/.cargo/env && cargo test -p agent-runtime-config build_memory_full -v`
Expected: PASS (both new tests).

- [ ] **Step 6: Commit**

```bash
source ~/.cargo/env
git add agent/crates/agent-memory/ agent/crates/agent-runtime-config/
git commit -m "feat(runtime-config): build_memory_full with shared retriever"
```

---

### Task 6: Wire auto-retrieval into the CLI

**Files:**
- Modify: `agent/crates/agent-cli/src/main.rs:7` (import), `:187-189` (memory build), `:206` (AgentLoop), `:218` (WindowContext)
- Test: manual smoke (no new unit test — this is wiring; behavior is covered by Tasks 2–5).

**Interfaces:**
- Consumes: `agent_runtime_config::build_memory_full`, `AgentLoop::with_retriever`, `WindowContext::with_recall_budget`.

- [ ] **Step 1: Update the import**

In `agent/crates/agent-cli/src/main.rs` line 7, swap `build_memory` for `build_memory_full`:

```rust
use agent_runtime_config::{backend_name_is_valid, build_memory_full, build_registry, build_model,
```

(Keep the remaining names on that `use` exactly as they are.)

- [ ] **Step 2: Build memory with retriever and register tools**

Replace the memory block (lines ~187-189):

```rust
    // Long-term memory: construct once (loads the embedding model), register the
    // tools, and keep the retriever for auto-retrieval.
    let memory = build_memory_full(cli.memory, cli.memory_db.clone(),
        cli.memory_model_dir.clone(), &workspace);
    for t in memory.tools.iter().cloned() {
        registry.register(t);
    }
```

- [ ] **Step 3: Attach the retriever to the loop**

At the `AgentLoop::new(...)` construction (line ~206), append `.with_retriever` when present. Replace:

```rust
    let agent = AgentLoop::new(model, protocol, tools, policy, Arc::new(TerminalApproval),
```

…through the end of that constructor call so the result is bound, then conditionally attach. Concretely, after the `let agent = AgentLoop::new(...);` statement, add:

```rust
    let agent = match memory.retriever.clone() {
        Some(r) => agent.with_retriever(r),
        None => agent,
    };
```

- [ ] **Step 4: Set the recall budget on the context**

Replace the `WindowContext::new` line (~218):

```rust
    let mut ctx = WindowContext::new(Message::system(system_prompt))
        .with_recall_budget(memory.recall_token_budget);
```

- [ ] **Step 5: Build and run the full workspace test suite**

Run: `source ~/.cargo/env && cargo build -p agent-cli && cargo test --workspace`
Expected: builds clean; all crates' tests PASS.

- [ ] **Step 6: Manual smoke test**

Run (writes then auto-recalls a fact across two turns):

```bash
source ~/.cargo/env
cargo run -p agent-cli -- --memory --memory-db /tmp/auto-recall-smoke.db
```

In the session: turn 1 — tell the agent a memorable fact and have it `remember` it. Restart, turn 2 — ask a related question WITHOUT mentioning the fact. Verify the response reflects the remembered fact (auto-retrieval injected it). Then:

```bash
rm -f /tmp/auto-recall-smoke.db
```

Expected: the second-session answer uses the remembered fact with no explicit `recall` call.

- [ ] **Step 7: Commit**

```bash
source ~/.cargo/env
git add agent/crates/agent-cli/src/main.rs
git commit -m "feat(agent-cli): wire memory auto-retrieval into the loop"
```

---

## Self-Review

**1. Spec coverage:**
- Per-turn retrieval relevant to user input → Task 3 (run()) + Task 4 (MemoryRetriever). ✓
- Reuse retrieval machinery (no drift) → Task 4 `query_memories` shared by `Recall` and `MemoryRetriever`. ✓
- Distinct system-role recall block after system prompt → Task 2. ✓
- Budgeted so it can't starve history → Task 2 (`recall_budget`, eviction order test). ✓
- Scope = project + global, `default_k`, `relevance_threshold` → Task 4 `query_memories`. ✓
- Zero behavior change when memory off → Task 1 (no-op default), Task 3 (`retriever: None`), Task 5 (`auto_recall`/disabled → `None`). ✓
- Retrieval failure never breaks a turn → Task 4 (`retrieve` swallows errors → empty). ✓
- Config `auto_recall` + `recall_token_budget` → Task 4. ✓
- Wiring → Task 5 (runtime-config) + Task 6 (CLI). Server path documented as out-of-scope (empty `memory_tools` today). ✓
- Tests per crate → Tasks 1–5 each include unit tests; Task 6 adds a smoke test. ✓

**2. Placeholder scan:** No TBD/TODO. Every code step shows complete code. One implementer note in Task 4 Step 8 flags verifying the `InMemoryStore` insert method name against source — this is a verification instruction, not a placeholder (the fields are enumerated).

**3. Type consistency:**
- `Retriever::retrieve(&self, query: &str) -> Vec<String>` — identical in Tasks 1, 3, 4. ✓
- `set_recall(&mut self, items: Vec<String>)` — Task 2 trait/impl, called in Task 3. ✓
- `with_recall_budget(self, usize) -> Self` — Task 2, used in Task 6. ✓
- `with_retriever(self, Arc<dyn Retriever>) -> Self` — Task 3, used in Task 6. ✓
- `query_memories(...) -> Result<Vec<Scored>, ToolError>` — Task 4, used by `Recall` and `MemoryRetriever`. ✓
- `MemoryBuild { tools, retriever, recall_token_budget }` — Task 5, consumed in Task 6. ✓
- `build_tools_and_retriever(...) -> Result<(Vec<Arc<dyn Tool>>, Arc<dyn agent_core::Retriever>), MemoryInitError>` — Task 5, consumed by `build_memory_full`. ✓
