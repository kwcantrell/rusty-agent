# Memory Auto-Retrieval — Design Spec

**Date:** 2026-06-24
**Status:** Approved (design), pending spec review
**Scope:** Auto-retrieval only. Auto-ingestion (auto-writing memories from turn output) and HTTP remote embeddings remain deferred and are explicitly out of scope.

## Problem

The `agent-memory` crate can *store* facts: the model calls the `remember` tool to write to long-term memory, and a `recall` tool lets the model explicitly search it. But nothing brings memories back into context automatically. Stored knowledge is only used if the model chooses to call `recall` — so cross-session memory is effectively dormant by default.

**Auto-retrieval** closes this loop: on each user turn, the system automatically searches long-term memory for facts relevant to the user's input and injects the best matches into the context window, before the model runs. This is the payoff the whole crate was built toward.

## Goals

- Each user turn, retrieve memories relevant to the user's input and surface them to the model automatically.
- Reuse the existing retrieval machinery (embed → cosine search → scope filter → threshold) so auto-retrieval and the `recall` tool cannot drift.
- Zero behavior change when memory is disabled. All existing tests compile and pass untouched.
- A memory retrieval failure must never break a turn.

## Non-Goals

- Auto-ingestion / automatic memory writing (separate future spec).
- HTTP remote embeddings (deferred).
- Changing the embedding model, store schema, or scope model.
- Summarizing or re-ranking memories beyond the existing cosine ordering.

## Approach (chosen)

**Loop-owned retrieval (Approach A).** The agent loop owns the per-turn retrieval lifecycle. Both production callers (`agent-server/src/daemon.rs`, `agent-cli/src/main.rs`) are covered automatically with no per-call-site wiring.

An alternative — caller-side retrieval (Approach C) — was considered to avoid colliding with concurrent work on `loop_.rs` (the `worktree-parallel-tool-calls` branch). That branch has since merged to `main`, removing the constraint, so Approach A (cleaner single ownership) is chosen.

## Architecture

```
agent-cli / agent-server                 agent-runtime-config
   loop.run(ctx, input)                     build_loop(): if --memory,
        |                                    attach MemoryRetriever via
        v                                    .with_retriever()
   AgentLoop.run()  --(top of turn)-->  Retriever::retrieve(query)  [agent-core port trait]
        |                                            ^
        |                                            | impl
        v                                    MemoryRetriever          [agent-memory adapter]
   ctx.set_recall(lines)                     embed → store.query → filter → format
        |
        v
   WindowContext.build()  emits a system-role recall block after the
                          system prompt, ahead of history, budgeted.
```

### 1. Port trait (`agent-core`)

A new trait keeps `agent-core` free of any `agent-memory` dependency (correct dependency direction — `agent-memory` already depends on `agent-model`, not the other way around):

```rust
#[async_trait]
pub trait Retriever: Send + Sync {
    /// Return formatted recall lines relevant to `query`, best-first.
    /// Implementations swallow their own errors and return an empty Vec
    /// on failure — retrieval must never break a turn.
    async fn retrieve(&self, query: &str) -> Vec<String>;
}
```

Lives in a new `agent-core/src/recall.rs` (or alongside `ContextManager` in `context.rs`). `async_trait` is already an available pattern in the crate's dependencies (model/protocol traits are async).

### 2. AgentLoop integration (`agent-core/src/loop_.rs`)

- `AgentLoop` gains a field `retriever: Option<Arc<dyn Retriever>>`, defaulting to `None`.
- Attached via a **builder method**, not a new `new()` parameter, so the 7-arg `AgentLoop::new` signature stays stable and ~10 existing test call sites are untouched:

```rust
impl AgentLoop {
    pub fn with_retriever(mut self, r: Arc<dyn Retriever>) -> Self {
        self.retriever = Some(r);
        self
    }
}
```

- In `run()`, immediately after `ctx.append(Message::user(user_input))` (loop_.rs:127):

```rust
if let Some(r) = &self.retriever {
    let lines = r.retrieve(&user_input).await;
    if !lines.is_empty() {
        ctx.set_recall(lines);
    }
}
```

This is the only change to `run()`'s body and sits at the top of the turn, away from the tool-execution / parallel-call section.

### 3. Context injection (`agent-core/src/context.rs`)

- `ContextManager` gains a **default no-op** method so no existing impl is affected:

```rust
fn set_recall(&mut self, _items: Vec<String>) {}
```

- `WindowContext` stores `recall: Vec<String>` and:
  - `set_recall` **replaces** the stored lines (newest query wins; each turn overwrites).
  - `build()` emits the recall lines as a **single system-role `Message`** placed **immediately after the system prompt and before history**. `WindowContext` owns the presentation — it wraps the lines with the header and bullet prefixes (the adapter returns plain fact text, one string per memory):
    ```
    Relevant memories from past sessions:
    - <fact 1>
    - <fact 2>
    ```
  - **Eviction order in `build()` (explicit):** (1) the system prompt is always kept; (2) the recall block is kept next, but capped at `recall_token_budget` — if the formatted block exceeds the cap, drop trailing lines until it fits; (3) the remaining budget (`model_limit − system − recall`) is filled with conversation history, evicting oldest-first exactly as today. The recall cap guarantees recall can never consume more than `recall_token_budget`, so history always retains at least `model_limit − system − recall_token_budget`.

### 4. Memory adapter (`agent-memory`)

A new `MemoryRetriever` implementing `agent_core::Retriever`:

```rust
pub struct MemoryRetriever {
    embedder: Arc<dyn Embedder>,
    store: Arc<dyn MemoryStore>,
    project_key: String,
    cfg: MemoryConfig,
}
```

`retrieve(query)` mirrors the `recall` tool exactly:
1. `embedder.embed(&[query])` → query vector (on error: `warn!`, return `vec![]`).
2. `store.query(qv, cfg.max_k, &ScopeFilter::ProjectAndGlobal { project_key })` (on error: `warn!`, return `vec![]`).
3. `retain(|h| h.score >= cfg.relevance_threshold)`.
4. Truncate to `cfg.default_k`.
5. Return each hit's `record.text` as a plain string (one per memory), best-first. Header and bullet formatting are `WindowContext`'s responsibility (§3), not the adapter's.

The query core (steps 1–4) is **extracted into a shared private fn** reused by both the `recall` tool and `MemoryRetriever`, so the two cannot diverge. The `recall` tool keeps returning a tool-result string; `MemoryRetriever` returns the formatted lines.

### 5. Wiring (`agent-runtime-config`)

Where memory is constructed today (the `--memory` path that builds the store/embedder/tools), additionally construct a `MemoryRetriever` from the same store/embedder/project_key/config and attach it: `let agent = AgentLoop::new(...).with_retriever(Arc::new(retriever));`. When `--memory` is off, no retriever is attached and behavior is unchanged.

## Configuration

Reuse `MemoryConfig` fields already used by `recall`:
- `default_k` — max memories injected per turn.
- `relevance_threshold` — minimum cosine score to inject.

Add:
- `auto_recall: bool` (default `true`) — gate auto-retrieval independently while keeping the memory tools available. When `false`, no retriever is attached even if `--memory` is on.
- `recall_token_budget: usize` (default: a small fraction of a typical window, e.g. **512 tokens**) — hard cap on the injected recall block, estimated with the existing `estimate_tokens`.

## Error Handling

- All retrieval errors (embed, store) are caught inside `MemoryRetriever::retrieve`, logged at `warn`, and yield an empty vec. The loop never sees an error from retrieval.
- Empty result (no hits, all below threshold, or error) → `set_recall` is not called → no recall block, identical to the no-memory path.

## Testing

**agent-core (`context.rs`):**
- `set_recall` + `build` places the recall block immediately after the system prompt and before history.
- The recall block is capped at `recall_token_budget`; excess lines are dropped.
- `set_recall` replaces prior lines (turn N+1 overwrites turn N).
- Conversation history is evicted before the recall block when over `model_limit`.

**agent-core (`loop_.rs`):**
- With a fake `Retriever` returning lines, the built context contains the recall block.
- A `Retriever` returning `vec![]` injects no block and the turn completes normally.
- A `Retriever` that simulates failure (returns `vec![]` after logging) still completes the turn.
- Default `AgentLoop` (no retriever) behaves exactly as before — regression guard.

**agent-memory (`MemoryRetriever`):**
- Returns scope-filtered, threshold-filtered, top-`default_k` lines ordered best-first.
- Returns empty when nothing matches or scores are below threshold.
- Shared query fn means the `recall` tool's existing tests also cover the query core.

## Files Touched

- `agent/crates/agent-core/src/recall.rs` — **new**: `Retriever` trait.
- `agent/crates/agent-core/src/context.rs` — `ContextManager::set_recall` default no-op; `WindowContext` recall storage + budgeted block in `build()`.
- `agent/crates/agent-core/src/loop_.rs` — `retriever` field, `with_retriever()` builder, retrieval call at top of `run()`.
- `agent/crates/agent-core/src/lib.rs` — export `Retriever`.
- `agent/crates/agent-memory/src/` — `MemoryRetriever` adapter; extract shared query fn from the `recall` tool.
- `agent/crates/agent-memory/src/config.rs` — `auto_recall`, `recall_token_budget`.
- `agent/crates/agent-runtime-config/` — construct + attach `MemoryRetriever` on the `--memory` path.

## Rollout

No migration. Off when `--memory` is off or `auto_recall = false`. When on, the only observable change is a recall block appearing in context when relevant memories exist.
