---
type: Practice
title: Start simple, add complexity only when measured
description: Begin with direct API calls and simple composable patterns, adding orchestration complexity only when evaluation demonstrably shows it improves outcomes.
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# Start simple, add complexity only when measured

The most successful agent implementations do not rely on complex frameworks or specialized libraries; they are built from simple, composable patterns [1]. Developers should start by using LLM APIs directly, because many patterns can be implemented in only a few lines of code [1]. This is a default, not a limitation: complexity should be added only when it demonstrably improves outcomes [1], and organizations should first optimize their agents with comprehensive evaluation before increasing system complexity [1]. This posture connects [design-and-scoping](/phases/design-and-scoping.md) to [eval-driven-development](/practices/eval-driven-development.md) — you scope minimally, then let measurement justify each added layer.

## Why simple wins first

Frameworks create extra layers of abstraction that can obscure the underlying prompts and responses, making agents harder to debug [1]. The same minimalism applies inside the prompt itself: optimal system prompts avoid both overly complex hardcoded logic and vague high-level guidance, and should contain the smallest set of information that fully outlines expected behavior — starting minimal and adding examples based on failure modes [4]. Tool sets follow the identical rule: bloated tool sets with overlapping functionality create ambiguous decision points, while minimal viable, non-overlapping tool sets improve reliability [4]. The unifying engineering principle across every context component is to find the smallest set of high-signal tokens that maximize the likelihood of the desired outcome [4].

## Add complexity only when measured

More elaborate patterns exist and are legitimate when the task demands them. Orchestrator-worker workflows provide more flexibility and are used when subtasks cannot be predefined [2], and evaluator-optimizer workflows support iterative refinement where feedback drives response recreation [2]. On complex, multi-hour tasks, a three-agent architecture of planner, generator, and evaluator produced richer full-stack applications than single-agent systems [3]. But such structure should be adopted because evidence supports it, not by default: trace and compare workflow patterns to evaluate which actually helps [2], and pair generation with an external evaluator, since agents exhibit systematic bias when judging their own work — confidently praising output that a human sees as mediocre [3]. Code solutions are verifiable through automated tests, letting agents iterate on results as feedback [1], and runtime interaction testing catches implementation bugs that static review misses [3].

## Re-evaluate scaffolding as models improve

Complexity added today is not permanent. Every component in a harness encodes an assumption about what the model cannot do on its own, and each is worth stress testing as capabilities change [3]. Model capability correlates directly with agent autonomy: smarter models let agents navigate nuanced problem spaces with less prescriptive engineering [4]. As underlying models improve, scaffolding built to compensate for past weaknesses can become unnecessary complexity that obscures rather than helps. The discipline is therefore continuous: start from direct API use and minimal context, justify every added pattern with comprehensive evaluation [1], and periodically remove scaffolding whose original assumption no longer holds.

# Citations

1. [Building Effective Agents](/sources/anthropic-building-effective-agents.md)
2. [Workflows and Agents - LangGraph Documentation](/sources/langchain-workflows-vs-agents.md)
3. [Harness design for long-running application development](/sources/anthropic-harness-design-long-running-apps.md)
4. [Effective context engineering for AI agents](/sources/anthropic-context-engineering.md)
