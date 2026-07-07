---
type: Practice
title: Multi-agent decomposition
description: Splitting an agentic workload across specialized, coordinated agents instead of one monolith, trading extra tokens for higher quality on complex tasks.
tags: [building-agents, agent-run-sdlc]
timestamp: 2026-07-06T00:00:00Z
---

# Multi-agent decomposition

## Specialized agents over monoliths

Monolithic "super" agents crumble under instruction overload, producing inaccurate outputs and brittle systems that resist scaling [1]. The alternative is to give each agent focused, domain-specific instructions and let several specialists work together [1]. A focused task is more likely to succeed, and each agent can carry its own prompt, few-shot examples, and even a separate or fine-tuned model [2]. Production-grade systems favor a distributed, loosely-coupled composition of specialized agents rather than a monolithic prompt trying to do everything — and that same modularity becomes the key to evaluating components in isolation [7]. Decomposition also lets you evaluate and improve each agent individually without breaking the larger application [2]. See [/phases/design-and-scoping.md](/phases/design-and-scoping.md) for scoping these boundaries.

## Coordinator and orchestrator-worker patterns

A coordinator or root agent whose only job is to understand the request and route it to the correct specialist organizes the workflow [1]. Anthropic's long-running harness work found a three-agent architecture — planner, generator, and evaluator — outperformed single-agent systems on complex tasks, producing full-stack applications over multi-hour autonomous sessions [6]. Feature-level decomposition with explicit sprint contracts that define what "done" means before any code is written improves coherence across agents [6].

## Sub-agents versus agents-as-tools

Control and context handling are the fundamental differentiators [4]. An agent-as-a-tool is a self-contained expert packaged for a specific, discrete, reusable task with encapsulated context — like a specialized function call [4]; a root agent can wrap specialists as tools and call them sequentially, e.g. book a flight, then find a hotel [1]. A sub-agent is a delegated team member handling a complex, multi-step, stateful process requiring shared context and hierarchical delegation [4]. The decision criteria: low-to-medium complexity, isolated/stateless, or generic reusable capabilities favor tools; high complexity, conversational-context needs, or specialized roles in specific workflows favor sub-agents [4].

## Parallel execution and shared-state communication

Independent tasks should run concurrently to cut total execution time [1]. Subagents operating in parallel with their own context windows also compress information, processing multiple threads at once [3]; parallel tool calling cut research time by up to 90% for complex queries [3]. Multiple agents can even work on a shared codebase without active human oversight, using lock files to prevent duplicate work [5]. Agents typically communicate through shared state rather than direct messaging [1], passing structured results via output keys [1]. To reduce information loss, artifact/filesystem systems give subagents direct access to persistent state instead of routing everything through the lead agent [3], and standardized protocols (e.g. A2A over HTTP + JSON) solve the N×N integration problem so agents can be swapped without downstream changes [7].

## Reviewer agents as QA

A system needs a feedback loop to review its own work, with dedicated reviewer agents validating outputs against quality standards and emitting pass/fail gates through structured output keys [1]. This matters because agents systematically praise their own outputs even when quality is mediocre, so external evaluation is required [6]. Subjective judgments become gradable when operationalized as explicit criteria and few-shot calibration [6], and runtime interaction testing (e.g. an evaluator screenshotting and studying the running page) catches bugs static review misses [6]. In autonomous teams, specialized agents can own distinct concerns such as correctness, documentation, and performance [5]. Wiring these gates is part of [/practices/harness-engineering.md](/practices/harness-engineering.md).

## Cost versus performance

The tradeoff is explicit: multi-agent systems use about 15× more tokens than chats but deliver a 90.2% performance improvement on research tasks [3]. The token multiplier makes decomposition worthwhile chiefly for high-value, complex work — not for tasks a single focused agent already handles.

# Citations

1. [Build multi-agentic systems using Google ADK](/sources/google-adk-multi-agent-systems.md)
2. [LangGraph: Multi-Agent Workflows](/sources/langchain-multi-agent-workflows.md)
3. [How we built our multi-agent research system](/sources/anthropic-multi-agent-research-system.md)
4. [Where to use sub-agents versus agents as tools](/sources/google-subagents-vs-agents-as-tools.md)
5. [Building a C compiler with a team of parallel Claudes](/sources/anthropic-building-c-compiler.md)
6. [Harness design for long-running application development](/sources/anthropic-harness-design-long-running-apps.md)
7. [From 'Vibe Checks' to Continuous Evaluation](/sources/google-continuous-evaluation.md)
