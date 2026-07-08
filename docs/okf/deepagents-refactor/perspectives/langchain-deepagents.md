---
type: Perspective
title: LangChain's deep-agents architecture
description: The target shape — a batteries-included harness where an opinionated middleware stack composes planning, filesystem, subagents, summarization, caching, memory, and HITL onto a minimal tool-calling loop.
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# LangChain's deep-agents architecture

## The thesis

"Shallow" agents run tool calls in a loop but lack the ability to plan over
longer horizons; "deep" agents add four integrated components — a detailed
system prompt, a planning tool, sub-agents, and file system access — with
Claude Code, Deep Research, and Manus as the exemplars [1]. The `deepagents`
package explicitly "draws inspiration from Claude Code, attempting to
identify what makes it general-purpose, and push that further" [3].

The companion evidence for why this layer matters: LangChain raised Terminal
Bench 2.0 from 52.8% to 66.5% by changing only the harness — system prompt,
tools, middleware — with the model held fixed [2].

## The architecture in one sentence

`create_deep_agent` is a single factory that composes an **ordered middleware
stack** onto LangChain's minimal tool-calling loop (itself on the LangGraph
runtime); every capability — todos, filesystem, subagents, summarization,
tool-call repair, prompt caching, memory, HITL — is a middleware that can
ship tools, extend state, and wrap model/tool calls [3][4].

Key structural properties:

- **Middleware hook model** — node-style hooks (`before_agent`,
  `before_model`, `after_model`, `after_agent`) merge returned dicts into
  graph state; wrap-style hooks (`wrap_model_call`, `wrap_tool_call`) nest
  like function calls; a middleware can register its own tools — this is how
  `write_todos`, the fs tools, and `task` enter the toolset [3].
- **Deterministic stack order** with user middleware spliced between the core
  and the tail (caching in the tail before memory and HITL, so cached
  prefixes match actual model input; profile tool-exclusion appended last);
  same-name user middleware replaces a default in place [3][4].
- **Per-model profiles** — `HarnessProfile` tunes excluded tools, extra
  middleware, and a model-specific prompt suffix placed last, "closest to
  conversation history for maximum model influence"; harnesses are
  model-specific (Claude vs Codex need different tailoring) [4][2].
- **The filesystem is the substrate** — evicted conversation history,
  oversized tool results, skills, and memory files all live on one pluggable
  `BackendProtocol` (state / disk / store / sandbox / composite-with-prefix-
  routing), making the backend the central portability seam [4].
- **Structured errors, not exceptions** — backends return results with an
  `error` field rather than raising (a convention of the built-in
  backends) [4].

## The four pillars, concretely

1. **Planning** — `write_todos` is a deliberate no-op: no computation, pure
   context engineering to keep the agent on track; prompt guidance gates it
   to 3+-step tasks and demands exactly one `in_progress` item [1][3].
2. **Filesystem** — ten built-in tools (`ls`, `read_file`, `write_file`,
   `edit_file`, `delete`, `glob`, `grep`, `execute`, `task`, `write_todos`)
   over the backend protocol; results over 20k tokens auto-evict to
   `large_tool_results/` with a pointer + preview [4][3].
3. **Sub-agents** — the `task` tool spawns ephemeral children with isolated
   context and single-message handoff; named `SubAgent` specs carry their own
   prompt/tools/model/permissions; a `general-purpose` subagent is always
   available [4][3].
4. **Steering** — declarative filesystem permissions (allow/deny/interrupt,
   first-match-wins) plus `interrupt_on` per-tool HITL with
   approve/edit/reject/respond decisions over durable LangGraph interrupts [4].

## Production posture

Durable checkpointing enables mid-run recovery, indefinite HITL pauses, and
audit trails; guardrail middleware (model-call limits, tool-call limits,
retry, fallback, PII) hardens the loop; typed stream projections separate the
product-level `subagents` view from internal graph structure [4].

# Citations

1. [Deep Agents (LangChain blog)](/sources/langchain-deep-agents-blog.md)
2. [Improving Deep Agents with Harness Engineering](/sources/langchain-harness-engineering-blog.md)
3. [deepagents source (langchain-ai/deepagents)](/sources/deepagents-source.md)
4. [deepagents documentation (docs.langchain.com)](/sources/deepagents-docs.md)
