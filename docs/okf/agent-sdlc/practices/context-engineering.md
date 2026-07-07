---
type: Practice
title: Context engineering
description: Treat the context window as a finite, degrading resource and curate the smallest set of high-signal tokens an agent needs at each step.
tags: [building-agents, agent-run-sdlc]
timestamp: 2026-07-06T00:00:00Z
---

# Context engineering

Context engineering is the practice of feeding an agent the right information at the right time — spanning prompt design, retrieval, tool selection, and conversation history — because everything in the window shapes how the agent understands and responds to each request [4]. Its governing principle is to find the smallest set of high-signal tokens that maximize the likelihood of the desired outcome across every context decision [1].

## Context is finite and degrading

LLMs lose focus and accuracy as context volume grows — a "context rot" that makes the window a finite, degrading resource rather than a free scratchpad [1]. The transformer's attention mechanism creates n² pairwise token relationships, so precision stretches thin over longer sequences [1], and position-encoding techniques that extend length trade off token-position understanding relative to originally-trained lengths [1]. Most models still perform poorly over long contexts, a constraint that bears directly on memory-heavy workloads [3]. Left unmanaged, a full history may simply not fit, producing an unrecoverable overflow error [3]. See [/practices/memory-and-state.md](/practices/memory-and-state.md) for durable-recall strategies.

## Minimal prompts and tools, grown by failure modes

Optimal system prompts avoid both extremes — overly prescriptive hardcoded logic and vague high-level guidance — and calibrate specificity to task complexity [1]. They should carry the smallest set of information that fully outlines expected behavior, starting minimal and adding examples only as observed failure modes reveal the need [1]. The same discipline applies to tools: bloated, overlapping tool sets create ambiguous decision points, while minimal, non-overlapping sets improve reliability and keep the context clean [1]. Few-shot input-output examples let you program behavior through the prompt without retraining [3], and reflection or meta-prompting lets an agent refine its own instructions from recent interactions [3].

## Just-in-time retrieval and progressive disclosure

Rather than pre-loading everything that might be relevant, agents should hold lightweight identifiers and load data dynamically at runtime through tools [1]. Progressive disclosure lets an agent incrementally discover context through exploration, using file metadata, naming conventions, and timestamps as behavioral signals without full loading [1][1]. In large codebases, embedding search alone becomes unreliable; hybrid retrieval combining grep and file search, knowledge-graph retrieval, and re-ranking is necessary for dependable code context [2]. Heavy artifacts such as images or audio should be held as state variables so they do not pollute the LLM window [2].

## Compaction, note-taking, and sub-agent isolation for long horizons

Long-horizon tasks demand active management. As the window fills, summarizing conversation history and reinitializing from a compressed summary keeps the agent effective over extended time [1]; compaction must preserve architectural decisions and unresolved details while discarding redundant output, balancing recall against precision to avoid losing critical context [1]. Persistent external note-taking lets an agent carry critical state across tool calls that would otherwise exceed the window [1]. Specialized sub-agents, each working a focused task in a clean context and returning condensed 1,000-2,000-token summaries, isolate exploration cost from the main thread and improve complex reasoning [1]. Actively pruning context is required to avoid unrecoverable token overflow [3]. Because context-engineering changes can help or hurt, they must be validated by systematic testing and token-usage tracking before shipping [2]. These runtime mechanics are the province of the surrounding [/practices/harness-engineering.md](/practices/harness-engineering.md).

As underlying models grow more capable, agents can navigate nuanced problem spaces with less prescriptive engineering, so the amount of hand-built context scaffolding can scale down over time [1].

# Citations

1. [Effective context engineering for AI agents](/sources/anthropic-context-engineering.md)
2. [Context Engineering for Agents](/sources/langchain-context-engineering.md)
3. [Memory Management in AI Agent Development Lifecycle](/sources/langchain-memory-concepts.md)
4. [A dev's guide to production-ready AI agents](/sources/google-devs-guide-production-agents.md)
