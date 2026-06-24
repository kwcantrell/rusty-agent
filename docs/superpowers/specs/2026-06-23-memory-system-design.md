# Design Spec — Vector / Long-Term Memory System (tools-first slice)

**Date:** 2026-06-23
**Status:** Approved design — ready for `writing-plans`.
**Subsystem:** #4 (Vector / Long-Term Memory) from the deferred list.
**Primer:** [`../context/memory-system.md`](../context/memory-system.md)
**Attaches via:** the existing async `Tool` / `ToolRegistry` seam + binary wiring (`agent-runtime-config`). **Zero core change this slice.**

---

## 1. Goal & scope

Give the agent **persistent memory across sessions**: the model can deliberately store a fact and later retrieve it by *semantic* similarity (not lexical match) in a brand-new process, so prior knowledge shapes behavior.

This is the **tools-first slice**. We ship explicit `remember` / `recall` / `forget` tools and the full storage + embedding infrastructure behind two trait seams. The *automatic* retrieval-augmented `ContextManager` (silent top-K injection every turn) is **deliberately deferred** to a follow-on, because it forces an async refactor of the synchronous `ContextManager::build` core seam — a change best made on its own, on top of a proven store, rather than bundled with brand-new embedding/storage code.

### In scope
- New `agent-memory` crate.
- `Embedder` trait + in-process ONNX impl (fastembed-rs, `bge-small-en-v1.5`, 384-dim) behind a default-on cargo feature; deterministic stub for tests.
- `MemoryStore` trait + `SqliteStore` (single local DB) + `InMemoryStore` for tests.
- Three tools: `remember`, `recall`, `forget`, with dedup-on-write, per-project + global scoping, relevance thresholding, and result budgeting.
- Wiring into **both** `agent-cli` and `agent-server`, gated by a `--memory` / `memory_enabled` switch.
- Failure isolation (memory is always best-effort, never fatal), resource limits, observability.

### Out of scope / deferred (tracked in follow-ups at cycle end)
- **Automatic `RetrievingContextManager`** (silent injection) + the async-`build()` core refactor it needs.
- **Auto-ingestion** (end-of-session summarization / salient-fact extraction) — pairs with the auto-retrieval slice.
- **Approval-gated writes** + any wire/web/Settings surface for memory.
- **LanceDB / ANN backend** — kept swappable behind `MemoryStore`; not needed at single-user scale.
- **HTTP `/v1/embeddings` Embedder** — kept swappable behind `Embedder`.
- **Re-embedding / migration** when the embedding model changes (old rows go inert with a notice).
- **PII/secret scrubbing**, full CRUD (`update`/`list`), multi-user.

---

## 2. Architecture & crate layout

New `agent-memory` crate, following the established one-crate-per-subsystem pattern (`agent-http`, `agent-mcp`, `agent-skills`, `agent-sandbox`). Everything attaches via the existing async `Tool` seam — **no edits to `agent-core`**.

```
agent-memory/
  embedder.rs   Embedder trait + FastEmbedEmbedder (feature "onnx") + StubEmbedder
  store.rs      MemoryStore trait + SqliteStore + InMemoryStore
  record.rs     MemoryRecord (id, text, scope, tags, vector, created/updated, source)
  scope.rs      MemoryScope::{Project(key), Global}; project-key derivation
  tools.rs      Remember / Recall / Forget tools (impl Tool)
  config.rs     MemoryConfig (paths, model, k, thresholds, caps)
  lib.rs        build store+embedder from config; register tools
```

### 2.1 `Embedder` trait
```rust
#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
    fn dim(&self) -> usize;
}
```
- **`FastEmbedEmbedder`** (shipped, behind cargo feature `onnx`, default-on): in-process ONNX via fastembed-rs. Default model `bge-small-en-v1.5` (384-dim). Resolves a **configurable local model dir** first; downloads from HuggingFace once on first init if absent and online, then caches.
- **`StubEmbedder`** (tests + `onnx`-off builds): deterministic, hash-seeded, L2-normalized vectors so cosine relationships are reproducible with no native runtime.

Feature-gating the heavy native dep keeps `cargo test --workspace` runnable in constrained CI (stub path) while shipping binaries get the real embedder.

### 2.2 `MemoryStore` trait
```rust
#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn upsert(&self, rec: MemoryRecord) -> Result<(), StoreError>;
    async fn query(&self, vector: &[f32], k: usize, filter: &ScopeFilter)
        -> Result<Vec<Scored<MemoryRecord>>, StoreError>;
    async fn get(&self, id: &str) -> Result<Option<MemoryRecord>, StoreError>;
    async fn delete(&self, id: &str) -> Result<bool, StoreError>;
    async fn count(&self, filter: &ScopeFilter) -> Result<usize, StoreError>;
}
```
- **`SqliteStore`** (shipped): `rusqlite`, **one DB file** at `~/.agent/memory.db` (0600), WAL + busy-timeout. Schema: `memories(id TEXT PK, scope_kind TEXT, scope_key TEXT, text TEXT, tags TEXT, vector BLOB, dim INT, created_at INT, updated_at INT, source TEXT)`. `query` does a **SQL pre-filter** (scope, optional tags) then **exact cosine in-process** over the candidate set — no ANN index. Vectors stored as little-endian f32 BLOBs.
- **`InMemoryStore`** (tests): `HashMap` + brute-force; same trait battery as `SqliteStore` (parity tests).

Exact search is correct and microsecond-fast at single-user scale (thousands–tens of thousands of rows). A candidate-set-size warning fires past a threshold — the "time to consider ANN" signal.

### 2.3 Tools
`remember`, `recall`, `forget` implement the existing `Tool` trait, registered via `agent-runtime-config` like the skills/MCP/http tools. Disabled (`--memory` off) → not registered, no DB created.

---

## 3. Data flow

### 3.1 Project-key derivation (shared)
Resolved **once** at tool construction from the agent's workspace root:
1. Canonicalize the workspace path.
2. `git rev-parse --show-toplevel` if inside a repo (stable across subdirs); else the canonical workspace root.
3. Store the key as a **SHA-256 hex of that path** so raw filesystem paths aren't scattered through the DB.

`MemoryScope::Project(key)` vs `MemoryScope::Global`.

### 3.2 `remember(text, tags?, scope?)` — write
1. Validate: non-empty `text`, `len ≤ max_text_len` (8 KiB) → `ToolError::InvalidArgs` on violation; clamp `tags` count/length.
2. Resolve scope: explicit arg, else default `Project`.
3. `embed([text])` → vector. Embedder error → `ToolError::Failed`; **loop continues** (write fails, never fatal).
4. **Dedup-on-write:** `query` the *same scope* for the nearest existing memory; if cosine ≥ `dedup_threshold` (0.95), **supersede** it (update text/tags, bump `updated_at`, keep id). Else insert a new record with a fresh UUID.
5. **Cap:** if scope at `max_memories_per_scope` (10 000), evict least-recently-updated (logged) before insert.
6. Return id + new-or-superseded.

### 3.3 `recall(query, k?, scope?)` — read
1. `embed([query])` → query vector.
2. SQL pre-filter: rows where `scope ∈ {current Project, Global}`, optional tag filter.
3. Exact cosine over candidates; top-`k` (default 5, hard cap `max_k` = 20) **above `relevance_threshold`** (0.3).
4. Render each hit compactly (`[score] text (tags, age)`); **budget the result** to `max_recall_chars` (4 KiB) with a truncation marker.
5. Empty → clear "no relevant memories" string, not an error.

### 3.4 `forget(id? | query?, scope?)` — removal
- By `id`: direct `delete`.
- By `query`: embed + single best match above `forget_threshold` (0.85); delete only that one, report it. Ambiguous/none → report, delete **nothing** (never mass-delete on a fuzzy match).

### 3.5 Persistence
Single global DB at `~/.agent/memory.db`; `scope_kind`/`scope_key` columns separate Project (hash-keyed) from Global (shared) rows. One file, one backup, one durability story; isolation is SQL-enforced.

---

## 4. Failure modes (memory is always best-effort — never fatal)

- **Model missing/offline first run** — resolve configurable local model dir first; if absent and download fails (offline), surface a clear one-time error and **disable memory tools for the session** (loop runs without them). Online first-run downloads once, then cached.
- **Embedder runtime error mid-session** — `remember`/`recall` return `ToolError::Failed` with a readable message; the loop feeds it back and continues.
- **SQLite errors** (locked / I/O / corruption) — busy-timeout on open; on a corrupt-DB open error, log loudly and disable memory for the session rather than aborting the daemon. Writes are transactional (no half-written records).
- **Dimension mismatch** (model changed) — rows whose stored vector length ≠ current `embedder.dim()` are **skipped in cosine with a warning** + a one-time "embedding model changed; old memories inert — re-embed or clear" notice. Never a panic. (Re-embed/migrate deferred.)
- **Concurrency** — the daemon's per-turn task is the only writer; SQLite WAL + busy-timeout covers the not-simultaneous CLI/daemon case.

---

## 5. Threat surface

- **Prompt-injection persistence (headline risk):** fetched web content / tool output could induce the model to `remember` adversarial text that later resurfaces via `recall` to steer a future session. Mitigations: memory text is **stored and returned as inert data, never executed**; `recall` results are framed as untrusted "retrieved memories," not system instructions; `forget` provides a correction path; per-project scoping limits cross-repo blast radius. Documented as an **accepted residual risk** (a fully-trusted memory store is out of scope for a single-user local tool).
- **Path/scope isolation:** project key is a hash of the canonicalized git-toplevel; `recall` reaches only current-project + global rows (SQL-enforced), never a sibling project's.
- **PII/secrets:** `remember` persists whatever it's given; the user owns the single local DB file (0600, under `$HOME`). No scan/scrub this slice (documented).
- **No new network surface:** embedding is in-process; the only network is the one-time model download.

---

## 6. Resource limits

| Limit | Default | Purpose |
|---|---|---|
| `max_text_len` | 8 KiB | reject oversized memory writes |
| `max_tags` / tag length | small caps | bound metadata |
| `max_memories_per_scope` | 10 000 | LRU-evict; bounded DB growth |
| `max_k` | 20 | cap recall fan-out |
| `recall` default `k` | 5 | sane default |
| `relevance_threshold` | 0.3 | suppress junk hits |
| `dedup_threshold` | 0.95 | supersede near-duplicates on write |
| `forget_threshold` | 0.85 | refuse ambiguous fuzzy deletes |
| `max_recall_chars` | 4 KiB | budget the tool result (context safety) |
| SQLite busy-timeout | set | tolerate transient locks |
| candidate-set warn threshold | configurable | "time to consider ANN" signal |

All defaults live in `MemoryConfig`, overridable via flags/config.

---

## 7. Observability

Structured `tracing` on every op (counts/latencies only — **never memory text at info level**; text at `trace` only):
- `memory.remember` — new vs superseded, scope, latency, evicted?
- `memory.recall` — query scope, candidates scanned, hits returned, top score, latency.
- `memory.forget` — matched / deleted.
- embedder init — model, dim, cache hit/miss.
- disable-reason warnings (model offline, DB corrupt, dimension mismatch, candidate-set over threshold).

---

## 8. Testing strategy

Hermetic + deterministic (DoD: in-memory store + stub embeddings).

- **`StubEmbedder`** for all logic tests; real `FastEmbedEmbedder` only behind an `#[ignore]`-gated live test (os-sandboxing / http-tool precedent).
- **Store parity:** the same trait battery over `SqliteStore` (`tempfile` DB) and `InMemoryStore`; a reopen-the-file test proving **cross-session persistence** (write, drop, reopen, recall).
- **Tool tests:** dedup supersede-above / insert-below threshold; cap LRU eviction; `recall` honors relevance threshold, `k` cap, scope filter (project rows invisible from a different project key; global visible from both), `max_recall_chars` truncation; `forget` by-id and by-query (refuses ambiguous/below-threshold).
- **Failure paths:** embedder error → `ToolError::Failed` + loop continues; dimension-mismatch row skipped-with-warning not panic; oversized text rejected; corrupt/locked DB disables memory without aborting.
- **Wiring:** `--memory` off → no tools / no DB; on → exactly three tools; project-key derivation (git-toplevel vs canonical root; stable across subdirs).
- **Gates:** `cargo test --workspace` + `cargo clippy --all-targets -- -D warnings` clean. **Live `#[ignore]` DoD:** real fastembed model — persist a memory, recall it semantically from a fresh process via a *paraphrased* (non-lexical) query.

---

## 9. Definition of done

The agent can `remember` a fact in one session and, in a **fresh process**, `recall` it via a semantically-similar (not lexically-matching) query, with per-project + global scoping, dedup-on-write, and `forget`-based correction — all backed by a `MemoryStore`-trait `SqliteStore` and an in-process `Embedder`, wired into both `agent-cli` and `agent-server` behind a `--memory` switch. Memory failures never abort the agent loop. Hermetic suite + clippy green; live DoD validated against the real embedding model.
