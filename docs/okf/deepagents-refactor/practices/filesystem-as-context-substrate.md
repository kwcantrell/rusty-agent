---
type: Practice
title: Filesystem as the context substrate
description: One pluggable virtual filesystem behind all file tools, with everything — evicted history, oversized tool results, skills, memory — living on it as agent-readable files routed across backends by path prefix.
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# Filesystem as the context substrate

Deep agents "run for long periods of time and accumulate a lot of context
that they need to manage"; persistent file storage is how Claude Code and
Manus handle it [1]. deepagents generalizes this into the architecture's
central seam: a `BackendProtocol` that every file tool speaks, with the
striking property that *all* context-management artifacts become files on
it [2][3].

## The protocol

Backends implement `ls / read / write / edit / glob / grep` (+ optional
`delete`), returning structured results with an `error` field rather than
raising — a convention of the built-in backends worth making a hard contract
in a Rust port [2]. The agent-facing toolset is ten tools (`ls`, `read_file` with
pagination and multimodal support, `write_file`, `edit_file`, `delete`,
`glob`, `grep`, `execute` when the backend is a sandbox, plus `task` and
`write_todos`) [2].

## The backend family

- `StateBackend` (default) — files in graph state, thread-scoped scratchpad [2]
- `FilesystemBackend` — real disk with `virtual_mode` blocking path traversal [2]
- `StoreBackend` — cross-thread store with namespace factories for
  per-user/per-assistant scoping (the long-term-memory substrate) [2]
- Sandbox backends — all fs ops derived from a single `execute()` [2]
- `CompositeBackend` — **routes by longest path prefix** and aggregates
  `ls`/`glob`/`grep` across backends, so `/memories/` can be durable store
  while `/workspace/` is disk and scratch stays in state [2]

## Everything is a file

- Summarized-away conversation history appends to
  `conversation_history/{thread_id}.md`; the summary message embeds the path
  so the agent can `read_file` the canonical record [3][2].
- Tool results over 20k tokens are evicted to
  `large_tool_results/{tool_call_id}` with a pointer + preview, and the
  prompt teaches chunked `read_file` / scoped `grep` recovery [2][3].
- Skills and memory are directories/files on the same backend
  ([progressive skill disclosure](/practices/progressive-skill-disclosure.md),
  [memory as editable files](/practices/memory-as-editable-files.md)) [2].

The payoff: one uniform recall mechanism (file tools the model already
knows), one permission surface, one portability seam.

## Why it matters for the refactor

The current runtime's file tools operate directly on the real workspace, and
offloaded tool results live in a separate id-keyed `OffloadStore` side table
with a bespoke `context_recall` tool; skills and memory are further separate
subsystems ([current runtime](/perspectives/current-runtime.md)) [4]. A
backend trait + composite routing would unify offload, skills storage, and
memory files under the existing file tools — retiring the bespoke recall
tool and giving sandboxed and remote execution a natural home behind the
same interface.

# Citations

1. [Deep Agents (LangChain blog)](/sources/langchain-deep-agents-blog.md)
2. [deepagents documentation (docs.langchain.com)](/sources/deepagents-docs.md)
3. [deepagents source (langchain-ai/deepagents)](/sources/deepagents-source.md)
4. [rusty-agent — current Rust agent runtime](/sources/rusty-agent-runtime.md)
