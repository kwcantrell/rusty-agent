---
type: Comparison
title: Refactor priorities and sequencing
description: What the gap analysis implies, ranked — middleware trait first, then virtual filesystem, then the cheap wins (todos, named subagents, caching), with an explicit do-not-regress list.
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# Refactor priorities and sequencing

Derived from the [capability gap analysis](/comparisons/capability-gap-analysis.md).
The evidence frame: harness-only changes moved Terminal Bench 2.0 by 13.7
points with the model fixed — the harness layers below are where performance
lives [2]. Sequencing follows dependency order, not ambition.

## Phase 1 — the middleware seam (enables everything else)

Introduce a Rust middleware trait (node hooks + wrap hooks + tool
contribution + state extension) and rewrite `assemble_loop()` as stack
composition, porting existing loop-resident behavior (recall injection,
compaction maintenance, stuck detection) into middleware without behavior
change [1][3]. This is the highest-risk, highest-leverage step: most
"absent" capabilities become one middleware each afterwards
([middleware composition](/practices/middleware-composition.md)) [1].
Existing trait seams (`ModelClient`, `ToolCallProtocol`, `PolicyEngine`,
`EventSink`, `SandboxStrategy`) survive unchanged beneath it [3].

## Phase 2 — the backend seam (virtual filesystem)

A `Backend` trait (ls/read/write/edit/glob/grep/delete, structured errors
instead of panics — hardening deepagents' error-return convention into a
contract, per [filesystem as context
substrate](/practices/filesystem-as-context-substrate.md)) with host-disk,
in-memory, and composite prefix-routing implementations; migrate file tools onto it; fold `OffloadStore` into
`large_tool_results/` files and evicted compaction spans into
`conversation_history/` files, retiring the bespoke `context_recall` tool
([filesystem as context substrate](/practices/filesystem-as-context-substrate.md)) [1][3].
Skills and memory files ride the same seam later; the Docker sandbox becomes
an execute-capable backend rather than a command-only strategy
([sandboxed execution](/practices/sandboxed-execution.md)) [1][3].

## Phase 3 — cheap high-value middleware

Each is small once Phases 1–2 exist [1][3]:

- `write_todos` planning middleware with the deepagents prompt discipline
  ([planning by recitation](/practices/planning-by-recitation.md)) — also a
  free progress surface for the three frontends.
- Named sub-agent registry over the existing dispatch machinery
  ([sub-agent context quarantine](/practices/subagent-context-quarantine.md)).
- Cache-aware prompt assembly ordering
  ([summarization and caching](/practices/summarization-and-caching.md)).
- Tool-call repair middleware (generalizing the loop's built-in one-shot
  re-ask retry into a pluggable unit); model-call/tool-call limit guardrails
  (generalizing stuck detection).
- Product-level typed subagent stream over the existing `EventSink`
  (deepagents' `stream.subagents` view vs today's `parent_id`-tagged raw
  events) — frontend-facing, no loop changes [1][3].

## Phase 4 — judgment calls, not blind ports

- **Memory**: adopt file-based memory (AGENTS.md + edit_file + trust
  framing) on the backend; decide whether the vector store remains as a
  retrieval layer or retires
  ([memory as editable files](/practices/memory-as-editable-files.md)) [1][3].
- **Declarative permissions + richer HITL** (edit/respond decisions): worth
  it, but durable *resumable* interrupts imply checkpointing — a large
  LangGraph-shaped dependency the Rust runtime lacks; scope deliberately [1][3].
- **Interpreter/PTC**: optional; only after evals justify it [1].

## Do not regress

The gap analysis marks these current-design wins as keep-invariants: goal
block + folded-facts ledger, `ToolIntent` policy richness,
refusal-on-degraded sandbox posture, first-class MCP, calibrated token
estimation [3]. Any phase that touches their subsystem must preserve the
behavior, and the repo's SDLC applies — this bundle is research input; each
phase still gets a spec, adversarial spec review, and plan before code [3].

# Citations

1. [deepagents documentation (docs.langchain.com)](/sources/deepagents-docs.md)
2. [Improving Deep Agents with Harness Engineering](/sources/langchain-harness-engineering-blog.md)
3. [rusty-agent — current Rust agent runtime](/sources/rusty-agent-runtime.md)
