---
type: Practice
title: Middleware composition over monolithic loops
description: Build the harness as an ordered stack of middleware — each shipping its own tools, state, prompt fragments, and model/tool-call wrappers — composed onto a minimal tool-calling loop by a single factory.
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# Middleware composition over monolithic loops

The load-bearing structural idea in deepagents: the loop itself stays
minimal, and every capability — planning, filesystem, subagents,
summarization, tool-call repair, prompt caching, memory, HITL — is a
middleware unit composed in a deterministic order by one factory
(`create_deep_agent`) [1][2].

## What a middleware unit is

A middleware can do four things [2]:

- **Ship tools** — `write_todos`, the fs toolset, and `task` all enter the
  agent's toolset because their middleware declares them, not because the
  loop knows about them.
- **Extend state** — declare additional state keys (e.g. `todos`) merged
  into agent state.
- **Hook the loop at nodes** — `before_agent` / `before_model` /
  `after_model` / `after_agent`, each returning state updates, with optional
  jumps (end/tools/model).
- **Wrap calls** — `wrap_model_call` / `wrap_tool_call` nest like function
  calls (first middleware outermost), which is where system-prompt edits,
  retries, and caching live.

## Ordering and override semantics

Order is part of the contract: core capability middleware first, user
middleware spliced after it, then per-model extras, with prompt caching in
the tail (before memory and HITL) so cached prefixes match what the model
actually receives, and per-profile tool exclusion appended last [2][1]. A user middleware whose name matches a default **replaces it in
place** — you can retune summarization triggers without rebuilding the
stack [1][2]. Two members are load-bearing scaffolding that profiles cannot
remove: filesystem and subagents [1].

## Per-model profiles

`HarnessProfile` makes the harness model-aware: excluded tools, extra
middleware, and a model-specific prompt suffix deliberately placed last,
"closest to conversation history for maximum model influence" [1]. The
harness-engineering evidence backs this: harnesses are model-specific, and
harness-only changes (model fixed) moved Terminal Bench 2.0 by 13.7
points [3]. The same experiment shipped its wins *as middleware* —
pre-completion checklist, local-context discovery, per-file edit-loop
detection — which is the pattern's payoff: behavioral fixes become pluggable
units rather than loop edits [3].

## Why this matters for the refactor

The current runtime has the *seams* (trait objects for model, protocol,
policy, sandbox, context, events) but not the *stack*: cross-cutting behavior
lives inside `AgentLoop` and `assemble_loop()`, so adding a capability means
editing the loop ([current runtime](/perspectives/current-runtime.md)) [4].
The refactor target is a Rust middleware trait with node hooks + wrap hooks +
tool contribution, and an assembly function that becomes a pure stack
composition.

# Citations

1. [deepagents documentation (docs.langchain.com)](/sources/deepagents-docs.md)
2. [deepagents source (langchain-ai/deepagents)](/sources/deepagents-source.md)
3. [Improving Deep Agents with Harness Engineering](/sources/langchain-harness-engineering-blog.md)
4. [rusty-agent — current Rust agent runtime](/sources/rusty-agent-runtime.md)
