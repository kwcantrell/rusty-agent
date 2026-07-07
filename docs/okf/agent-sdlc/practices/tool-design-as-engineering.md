---
type: Practice
title: Tool design as an engineering discipline
description: Tool names, descriptions, and argument shapes steer agent behavior as strongly as prompts, and warrant equal engineering rigor—error-proofing, eval-driven iteration, and minimally viable non-overlapping toolsets.
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# Tool design as an engineering discipline

Tools are not a peripheral concern in agent systems; they are a primary behavioral lever. Tool definitions and specifications warrant just as much prompt engineering attention as the overall agent prompt, and in practice some teams spend more time optimizing their tools than the prompt itself [2]. This makes tool design an engineering discipline in its own right, closely tied to [prototyping and development](/phases/prototyping-and-development.md).

## Names and descriptions steer behavior

An agent chooses which tool to invoke based on the tool's name and description, so making them clear and unique is crucial to avoid looping behaviors or incorrect actions [3]. Agent-tool interfaces are as critical as human-computer interfaces: bad tool descriptions can send agents down completely wrong paths [4]. Because the model treats every part of a definition as a prompt, vague descriptions risk "context poisoning" that causes the agent to pursue incorrect goals [3]. The payoff of precision is large and often surprising—small refinements to tool descriptions can yield dramatic improvements [1].

## Poka-yoke: error-proofing the interface

Beyond wording, the shape of a tool's arguments determines how reliably a model can call it. Some parameter formats are much more difficult for an LLM to write correctly than others [2]. The design response is to poka-yoke tools—change the arguments so that it is harder to make mistakes in the first place [2]. This shifts reliability from hope to structure: an interface that cannot easily be misused produces fewer malformed calls than one that merely documents the correct usage.

## Minimally viable, non-overlapping toolsets

More tools do not mean more capability. Bloated toolsets with overlapping functionality create ambiguous decision points; minimal viable toolsets improve reliability and context maintenance [5]. The guiding principle mirrors the broader context-engineering rule of finding the smallest set of high-signal tokens that maximizes the likelihood of the desired outcome [5]. Every redundant tool is another fork the agent can take wrongly, so curating the vocabulary down to distinct, non-overlapping capabilities directly reduces error surface.

## Eval-driven tool iteration

Tool quality is measurable, and measurement should drive iteration—see [eval-driven development](/practices/eval-driven-development.md). Systematically measure tool performance across multiple dimensions: top-level accuracy plus total runtime, number of tool calls, token consumption, and tool errors [1]. Extensive testing with varied inputs reveals the mistakes a model makes, which then guide iterative improvement [2], and code-backed tools can iterate on solutions using automated test results as feedback [2]. Even minimal test sets are informative: a prompt or tool tweak can move success rates from 30% to 80%, and such changes are visible with just a few test cases [4]. Held-out test sets guard against overfitting during these iteration cycles [1].

Notably, agents can participate in improving their own tools. Claude Code can quickly explore a set of tools and create dozens of prompt-and-response pairs to seed evaluation tasks [1]; agents are helpful partners in spotting issues by analyzing evaluation transcripts [1]; and you can even let agents analyze results and improve tools for you [1]. This closes the loop—the same system that consumes the tools generates the evidence for refining them.

## Why it is a discipline

The stakes justify the rigor. Because tool descriptions, argument shapes, and set composition all directly determine navigation and reliability, they deserve deliberate design, systematic measurement, and iterative refinement rather than ad-hoc definition. Treating tool design with the same discipline as prompt engineering [2] is what turns an unreliable agent into a dependable one.

# Citations

1. [Writing Effective Tools for AI Agents](/sources/anthropic-writing-tools-for-agents.md)
2. [Building Effective Agents](/sources/anthropic-building-effective-agents.md)
3. [Startup Technical Guide: AI Agents in Production](/sources/google-startup-guide-production-agents.md)
4. [How We Built Our Multi-Agent Research System](/sources/anthropic-multi-agent-research-system.md)
5. [Effective Context Engineering for AI Agents](/sources/anthropic-context-engineering.md)
