# Context Primer — Vector / Long-Term Memory System

**Status:** Not started. Context primer — run `brainstorming` before implementing.
**Attaches via:** `ContextManager` trait (retrieval-augmented context) + `Tool` (explicit memory ops).
**Depends on:** agent core only.

## What it is

Persistent memory beyond the current conversation: user preferences, project knowledge, and lessons from previous tasks, retrieved by semantic similarity to augment context. The core ships short-term memory only (token-windowed conversation); this adds the long-term tier.

## Where it fits

Two complementary attach points, both already designed for in the core (§10):

1. **As an enhanced `ContextManager`** — a `RetrievingContextManager` wraps the default one: on `build()`, embed the recent turn, query the vector store, and inject the top-K relevant memories alongside the sliding window.
2. **As tools** — `remember(text, tags)` / `recall(query)` so the model can deliberately store and fetch knowledge.

Short-term storage (current conversation/session metadata) lands in **SQLite**; long-term semantic memory lands in a **vector DB**.

## Key responsibilities

- **Embeddings:** turn text into vectors. Likely via the OpenAI-compatible embeddings endpoint (SGLang/vLLM can serve embedding models) — reuse the `agent-model` client pattern.
- **Vector store abstraction:** a `MemoryStore` trait (`upsert`, `query(vector, k, filter)`, `delete`) with pluggable backends.
- **Ingestion:** what gets stored, when (end of task? explicit `remember`?), with metadata/tags/timestamps and dedup.
- **Retrieval:** top-K with relevance threshold; budget how much retrieved memory enters context.
- **SQLite** for session/short-term + memory metadata mirror.

## Proposed approach (backends to weigh in brainstorming)

- **LanceDB** — embedded, no server, file-based. Best fit for a local-first single-user tool; recommend as default.
- **Qdrant** — standalone server, rich filtering; better if it ever goes multi-user.
- **pgvector** — if a Postgres dependency is otherwise wanted.

Define `MemoryStore` so the backend is swappable; ship LanceDB first.

## Open questions for brainstorming

- What's worth remembering, and what triggers a write (auto vs explicit)?
- Embedding model + dimensionality; local-served vs cloud.
- Retrieval budget: how many tokens of memory may enter context, and how does it interact with the sliding window?
- Memory hygiene: expiry, dedup, contradiction/staleness handling.
- Per-project scoping vs global memory.

## Definition of done (high level)

The agent can persist a memory and later retrieve it semantically to influence behavior in a new session, via a `MemoryStore`-backed `RetrievingContextManager` and/or `remember`/`recall` tools. Tested with an in-memory `MemoryStore` and deterministic stub embeddings.
