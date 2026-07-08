---
type: Practice
title: Sub-agent context quarantine
description: Delegate noisy multi-step work to ephemeral subagents with isolated context and single-message handoff — and make subagents a named registry with per-agent prompt, tools, model, and permissions.
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# Sub-agent context quarantine

Sub-agents give deep agents "context management and prompt shortcuts" — each
child goes deep on one domain instead of one monolithic context attempting
everything [1]. The stated rationale is quarantine: large tool outputs fill
the parent window quickly, so subagents run autonomously with fresh context
and return only the final result, "keeping the main agent's context
clean" [2].

## deepagents mechanics

- One `task` tool with `description` (all necessary context + expected
  output format — no other context is inherited) and `subagent_type` [3][2].
- **Named registry, not just one child shape**: a `SubAgent` spec carries
  required `name` / `description` / `system_prompt` (never inherited from
  the parent prompt) and optional `tools`, `model`, `middleware` (merged into
  the default child stack), `skills`, `permissions` (replace, not merge), and
  `response_format`; `CompiledSubAgent` wraps any graph with a `messages`
  key [2].
- A `general-purpose` subagent inheriting parent model/tools/skills is always
  registered [2][3].
- Handoff is a single message: the child's last non-empty AI message (or its
  JSON structured response); child state merges back **except**
  `messages`, `todos`, and `structured_response` — transcripts and plans stay
  quarantined [3].
- Sync children block the parent; an async middleware variant runs children
  in parallel [3].

## Why it matters for the refactor

This is the pillar where the current runtime is *closest*: `DispatchAgentTool`
already spawns an isolated child loop with fresh context, snapshot tools,
depth/turn/timeout limits, and a routed child model
([current runtime](/perspectives/current-runtime.md)) [4]. The gaps are
shape, not existence:

- one general dispatch tool (per-call `prompt`/`tools`/`role` args — the
  `role` preamble is appended to the child's system prompt) vs a **persistent
  named subagent registry** with per-subagent prompt/tools/model/permissions
  specs [2][4];
- no structured-response option for the handoff [3][4];
- event forwarding exists (`parent_id`-tagged tool events) but there is no
  product-level typed subagent stream like `stream.subagents` [4][2].

# Citations

1. [Deep Agents (LangChain blog)](/sources/langchain-deep-agents-blog.md)
2. [deepagents documentation (docs.langchain.com)](/sources/deepagents-docs.md)
3. [deepagents source (langchain-ai/deepagents)](/sources/deepagents-source.md)
4. [rusty-agent — current Rust agent runtime](/sources/rusty-agent-runtime.md)
