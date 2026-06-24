# agent-memory

Cross-session long-term memory for the Rust agent runtime. Provides three tools
(`remember`, `recall`, `forget`) backed by a local SQLite vector store, so the
agent can retain and retrieve facts across independent process invocations.

## Purpose

The agent's in-context window is ephemeral. `agent-memory` adds a persistent
semantic store: the agent calls `remember` to write a fact, `recall` to retrieve
the most relevant facts for a query, and `forget` to remove stale entries. All
operations are scoped to the current project (identified by a SHA-256 of the
git-toplevel path) or optionally to a `global` tier visible across all projects.
Memory ops declare a `Read`-access, no-path, no-command intent so `RulePolicy`
auto-allows them without user approval prompts.

## Extension seams

| Seam | Type | Role |
|---|---|---|
| `Embedder` | `async_trait` | Converts text → `Vec<f32>`. Swap for any encoder. |
| `MemoryStore` | `async_trait` | Persists and cosine-queries `MemoryRecord`s. Swap for ANN backends. |

Both are `Arc<dyn …>` — wire them independently and pass to `build_tools_with`.

## The three tools

| Tool | Args | Behaviour |
|---|---|---|
| `remember` | `text` (required), `tags` (optional), `scope` (`project`\|`global`, default `project`) | Embeds and stores. Deduplicates near-identical memories (cosine ≥ 0.95 → supersede). Evicts the oldest entry when the per-scope cap (10 000) is reached. |
| `recall` | `query` (required), `k` (optional, default 5) | Embeds query, cosine-queries project + global tiers, filters below relevance threshold (0.30), returns top-k as a scored list (capped at 4 KiB). |
| `forget` | `id` OR `query` | By `id`: exact delete. By `query`: deletes the single best match only if cosine ≥ 0.85; never mass-deletes. |

## CLI flags

Both `agent-cli` and `agent-server` expose:

| Flag | Default | Description |
|---|---|---|
| `--memory` | off | Enable the three memory tools. |
| `--memory-db <PATH>` | `~/.agent/memory.db` | Override the SQLite DB path. |
| `--memory-model-dir <PATH>` | fastembed default cache | Override the ONNX model cache directory. |

Memory is opt-in: omitting `--memory` registers no tools and touches no files.

## Feature flags

| Feature | Default | Description |
|---|---|---|
| `onnx` | **enabled** | Links fastembed (ONNX Runtime) and loads **BGE-Small-EN-v1.5** (384-dim) on first use. The model is downloaded once to the cache dir; subsequent runs are offline. |

Without `onnx` (e.g. in test builds), a `StubEmbedder` is used instead — it
produces deterministic hash-based vectors that are not semantically meaningful.
All unit tests use `StubEmbedder` and run without network access; paraphrase
recall is only validated by the live `#[ignore]` integration test.

## Default DB location

`~/.agent/memory.db` — created automatically on first write with mode `0600`.
The file is a standard SQLite3 database containing a single `memories` table.

## Resource limits (defaults)

| Limit | Default |
|---|---|
| Max text length | 8 KiB |
| Max memories per scope | 10 000 |
| Max recall result chars | 4 KiB |
| Max tags per memory | 16 |
| Max tag length | 64 chars |
| Recall candidates before warning | 50 000 |

## Deferred items

The following were intentionally left out of this slice (see the spec for rationale):

- **Auto-retrieval `ContextManager`**: a `RetrievingContextManager` that
  automatically prepends relevant memories to every turn, and the associated
  async refactor of `ContextManager::build`. The current design is tools-first —
  the agent calls `recall` explicitly.
- **Auto-ingestion**: automatic extraction and storage of facts from agent
  turn output.
- **`RuntimeConfig` / web Settings persistence**: `--memory` flags are CLI-only;
  they are not yet wired into `RuntimeConfig` or the browser Settings panel.
- **LanceDB / ANN backends**: the `MemoryStore` trait is designed to support
  approximate-nearest-neighbour stores, but only `SqliteStore` (brute-force
  cosine) and `InMemoryStore` (tests) are shipped.
- **HTTP embeddings**: calling a remote embedding API instead of the local ONNX
  model.
