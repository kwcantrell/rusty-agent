---
type: Practice
title: Memory as agent-editable files
description: Long-term memory is a set of files (AGENTS.md-style) loaded into the system prompt at startup, self-edited by the agent with its ordinary edit_file tool, and scoped by routing /memories/ to a durable store backend.
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# Memory as agent-editable files

deepagents models persistent memory as **files, not a database API**:
`memory=["/memories/AGENTS.md", ...]` names files that `MemoryMiddleware`
loads at startup and injects into the system prompt via a template slot [1][2].

## Mechanics

- Loading happens in `before_agent` via the backend's file download; HTML
  comments are stripped; an optional cache breakpoint sits after the memory
  block so the static prefix stays cacheable [2][1].
- **Self-editing uses the ordinary `edit_file` tool** — no bespoke
  remember/forget API; the prompt tells the agent to update memory
  "usually in the same turn" it learns something [1].
- **Trust framing is explicit**: memory content "may be outdated, incorrect,
  or written by someone other than the current user," and must not override
  the user's explicit request [2][1].
- **Scoping is backend routing**: `/memories/` routes to a `StoreBackend`
  inside a composite, with namespace factories for user- / assistant- /
  org-scoped memory; production docs recommend user-scoping and warn that
  shared writable memory is a prompt-injection surface [1].

## Tension with the current design

The current runtime took the other fork: semantic vector memory —
remember/recall/forget tools over SQLite with ONNX embeddings and in-process
cosine similarity, plus automatic similarity-based recall injected under a
token budget at run start
([current runtime](/perspectives/current-runtime.md)) [3]. These are
not equivalent: file memory is transparent, human-auditable, cacheable, and
cheap to load wholesale; vector memory retrieves by semantic similarity
without the agent knowing what exists. A refactor can host file-based memory
on the new backend (routing `/memories/` durably) while keeping the vector
store as a *retrieval* layer — the deepagents docs themselves note background
consolidation as an alternative to same-turn editing [1] — but the
agent-facing contract (files + edit_file, with explicit trust framing) is
the pattern to adopt.

# Citations

1. [deepagents documentation (docs.langchain.com)](/sources/deepagents-docs.md)
2. [deepagents source (langchain-ai/deepagents)](/sources/deepagents-source.md)
3. [rusty-agent — current Rust agent runtime](/sources/rusty-agent-runtime.md)
