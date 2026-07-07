---
type: Source
title: Effective context engineering for AI agents
description: Anthropic material on the SDLC of AI agents.
resource: https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
org: Anthropic
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# Summary

First-party material by Anthropic (published 2025-09-29). Key claims extracted below; the live document is the authority.

# Key claims

- LLMs degrade in focus and accuracy as context volume increases due to context rot, making context a finite and degrading resource in agent systems.
- Transformer attention mechanisms create quadratic token relationships, stretching capacity thin and reducing precision on longer sequences.
- Position encoding techniques can extend sequence length but trade off token position understanding compared to originally-trained context lengths.
- Optimal system prompts balance between overly prescriptive hardcoded logic and vague guidance, calibrating specificity to agent task complexity.
- System prompts should contain the minimal sufficient information to outline expected behavior, starting minimal and adding only what failure modes reveal.
- Tool sets should be minimally viable and non-overlapping; bloated tools with redundant functionality create ambiguous decisions that degrade agent reliability.
- Agents should maintain lightweight identifiers and dynamically retrieve data at runtime rather than pre-loading all potentially relevant context upfront.
- Progressive disclosure through autonomous exploration—where agents discover relevant context via interaction—reduces upfront context loading and improves efficiency.
- Agents can maintain effectiveness across long-horizon tasks by summarizing conversation history near context limits and reinitializing with compressed summaries.
- Compaction during context compression must preserve architectural decisions and unresolved details while discarding redundancy to prevent critical context loss.
- Persistent external note-taking enables agents to maintain critical context across multiple tool calls that would otherwise exceed context windows.
- Specialized sub-agents handling focused tasks with clean context windows and returning condensed summaries (1,000-2,000 tokens) improve complex reasoning.
- Model capability directly correlates with agent autonomy: smarter models allow agents to navigate nuanced problem spaces with less prescriptive engineering.
- The core engineering principle for all context components is finding the smallest set of high-signal tokens that maximize desired outcome likelihood.
- File systems, naming conventions, and metadata structures serve as important informational signals that guide both human and agent understanding.
