---
type: Practice
title: Threshold summarization with cache-aware assembly
description: Summarize at a fraction-of-window trigger with a keep policy, retry immediately on overflow, offload the evicted span to a readable file — and order the prompt so static prefixes stay cache-hit.
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# Threshold summarization with cache-aware assembly

deepagents treats in-window curation as a middleware with explicit, tunable
policy [1][2].

## The policy knobs

- **Trigger**: `("fraction", 0.85)` of the model profile's input limit by
  default (fallback 170k tokens without a profile); triggers compose — a
  dict of conditions ANDs, a list ORs — and can also fire on message
  count [2][1].
- **Keep**: `("fraction", 0.10)` of the window (fallback: last 6 messages) —
  the recent tail survives verbatim [2].
- **Overflow recovery**: on a context-overflow error, summarize immediately
  and retry the request [1].
- **Nothing is destroyed**: the evicted span appends, timestamped, to
  `conversation_history/{thread_id}.md` (inline media extracted to hashed
  files), and the summary message embeds the path so the agent can read the
  canonical record back [2][1].
- Oversized tool *inputs* near the ceiling are also truncated from older
  calls [1].

## Cache-aware ordering

Prompt-caching middleware sits in the stack tail — after tool-call patching,
before memory and HITL — "ensuring cached prefixes match actual model
input"; system-prompt assembly preserves cache-control markers, and the
static prefix (base instructions, memory, skills metadata) is what gets
cached [1][2]. Model-specific prompt suffixes go last, nearest the
conversation [1].

## Why it matters for the refactor

The current runtime's `CuratedContext` already has the same skeleton — 85%
high-water compaction on a routed compaction model, single overflow-retry,
offload of large tool results, plus two things deepagents *lacks*: a pinned
goal/re-grounding block and an append-only folded-facts ledger that survives
compaction ([current runtime](/perspectives/current-runtime.md)) [3]. The
deltas to adopt are: evicted history lands in an agent-readable file rather
than being lost into a regenerated summary; composable trigger/keep policy
as data; and deliberate cache-aware assembly ordering, which today has no
explicit contract in `assemble_loop()` [3][1]. The deltas to *keep* are the
goal block and facts ledger — they are genuine improvements over the
deepagents baseline [3].

# Citations

1. [deepagents documentation (docs.langchain.com)](/sources/deepagents-docs.md)
2. [deepagents source (langchain-ai/deepagents)](/sources/deepagents-source.md)
3. [rusty-agent — current Rust agent runtime](/sources/rusty-agent-runtime.md)
