---
type: Practice
title: Memory and state management
description: How agents persist, scope, categorize, and prune memory and state so runs are resumable, personalized, and isolated across tenants.
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

Agents differ from deterministic code because they maintain state, use tools dynamically, and operate autonomously, so their memory and state layer needs infrastructure designed for agent-specific needs rather than adapted standard patterns [3]. A production architecture separates short-term memory (session state) for immediate recall from long-term memory (a memory service) for retaining past interactions [3].

## Checkpointing for resumability and HITL pauses

State is persisted to a database using a checkpointer, which makes agent execution resumable across restarts and failures [1]. When a worker crashes mid-run, another worker picks the run up from the latest checkpoint and resumes without reprocessing [2]. The same checkpoint mechanism supports indefinite human-in-the-loop pauses without state loss [2]. Beyond a single run, fault-tolerant execution systems that resume from the point where an error occurred are a requirement for stateful agents [4]. Every production call is parameterized by a stable `thread_id` for conversation continuity and a context object carrying per-run data [2], and managed deployments auto-provision the threads, runs, checkpointers, and stores this requires [2].

## Memory scoping

Scoping strategy shapes architecture. Short-term memories are thread-scoped, while long-term memories exist across conversational threads using custom namespaces [1]. A composite backend can combine thread-scoped scratch space with cross-thread persistent storage, enabling both session-specific work and durable long-term memory [2]. User-scoped memory is the recommended default for most deployments [2].

## Semantic, episodic, and procedural categories

Three human-inspired memory categories map to agent development: semantic (facts), episodic (experiences), and procedural (rules) [1]. Agents ground responses in semantic memories, which tends to produce more personalized and relevant interactions [1]. Two storage shapes trade off against each other: profile-based semantic memory consolidates facts in a compact structure but becomes error-prone as the profile grows, while collection-based storage extends document sets continuously—reducing information loss but raising complexity for deletion and update operations [1]. Collection approaches also suffer model-side data-quality issues where the LLM over-inserts or over-updates items [1]. Update timing has its own tradeoff: hot-path updates make new memories immediately available at the cost of latency, whereas background tasks eliminate latency in the primary path [1]. Strict decoding and schema validation help keep memory schemas valid across update cycles [1].

## Pruning against context overflow

A full history may not fit inside an LLM's context window, producing an irrecoverable error, so active pruning is required [1]. This is closely tied to [context engineering](/practices/context-engineering.md): most LLMs still perform poorly over long contexts, which constrains memory-heavy workloads [1]. Multi-agent designs address overflow structurally — subagents run in parallel with their own context windows to compress work [4], and an agent can spawn fresh subagents with clean contexts while preserving continuity through careful handoffs [4]. Artifact filesystems reduce information loss by giving subagents direct access to persistent state instead of routing everything back through a lead agent [4].

## Multi-tenant isolation as a security boundary

Memory and execution environments must respect user, assistant, and organizational boundaries to maintain data isolation in multi-tenant deployments [2]. This is a security boundary, not merely an organizational one: shared memory is a vector for prompt injection, because if one user can write to memory that another user's conversation reads, a malicious user could inject instructions [2]. Enforcing isolation requires three distinct layers — end-user identity verification, authorization handlers controlling resource access, and credential injection [2] — concerns that carry into [deployment](/phases/deployment.md). Sandboxed execution environments further isolate agent workloads inside an isolated container providing a filesystem and an execute tool [2].

# Citations

1. [Memory Management in AI Agent Development Lifecycle](/sources/langchain-memory-concepts.md)
2. [Going to Production with Deep Agents](/sources/langchain-deepagents-production.md)
3. [A dev's guide to production-ready AI agents](/sources/google-devs-guide-production-agents.md)
4. [How we built our multi-agent research system](/sources/anthropic-multi-agent-research-system.md)
