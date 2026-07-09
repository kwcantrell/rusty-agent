---
type: Practice
title: Planning by recitation (the no-op todo tool)
description: "Give the agent a write_todos tool that performs no computation — its entire value is context engineering: rewriting the plan into recent context keeps long-horizon goals in the attention window."
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# Planning by recitation (the no-op todo tool)

Deep agents carry a planning tool that is a deliberate **no-op**: Claude
Code's todo tool "performs no actual computation" and exists purely as "a
context engineering strategy to keep the agent on track" over long tasks [1].
deepagents ships this as `TodoListMiddleware`, always first in the default
stack [2][3].

## Mechanics

- State is a plain list: `{content, status: pending | in_progress |
  completed}`; each `write_todos` call rewrites the whole list into a `todos`
  state key [2].
- The plan state is deliberately **not** merged back from subagents — each
  context plans for itself [2].

## The behavioral contract lives in prose

The tool description and injected prompt carry the discipline [2]:

- Use for complex multi-step tasks (3+ distinct steps), non-trivial planning,
  or user-supplied task lists; NOT for single straightforward tasks or
  conversational turns — "for simple objectives that only require a few
  steps, it is better to just complete the objective directly."
- "Unless all tasks are completed, you should always have at least one task
  in_progress."
- Mark items completed immediately; do not batch completions.

## Why it matters for the refactor

The current runtime has **no plan state at all** — planning is implicit in
prompts and skills, with no audit trail of plan vs outcome
([current runtime](/perspectives/current-runtime.md)) [4]. This is the
cheapest of the deepagents pillars to add: one tool + one state key + prompt
guidance, no loop changes if middleware composition
([middleware composition](/practices/middleware-composition.md)) lands
first — and it also gives frontends a progress surface for free (the todo
list is renderable UI state).

# Citations

1. [Deep Agents (LangChain blog)](/sources/langchain-deep-agents-blog.md)
2. [deepagents source (langchain-ai/deepagents)](/sources/deepagents-source.md)
3. [deepagents documentation (docs.langchain.com)](/sources/deepagents-docs.md)
4. [rusty-agent — current Rust agent runtime](/sources/rusty-agent-runtime.md)
