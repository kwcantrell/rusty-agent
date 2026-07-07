---
type: Lifecycle Phase
title: Design and scoping
description: The phase where you decide what to build before building it — choosing between workflows and agents, fixing precise agent identity, and selecting an architecture while bounding scope against open-loop failure.
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# Design and scoping

Design and scoping is the phase where you decide *what* to build before writing the loop. Agents behave unlike deterministic code — they reason, act, and adapt, so the patterns that served traditional software do not fully translate, and scoping errors compound downstream [7]. Structured scoping matters directly to reliability: frameless development of LLM agents introduces critical risks including scope creep and open-loop failures [6].

## Simple-first bias

The most successful implementations avoid complex frameworks and specialized libraries, building instead with simple, composable patterns [1]. Many patterns can be implemented in a few lines of code by using LLM APIs directly [1], whereas frameworks add abstraction layers that obscure the underlying prompts and responses and make debugging harder [3]. The governing rule is to add complexity only when it demonstrably improves measurable outcomes [1], after optimizing with comprehensive evaluation [1]. This bias is the core of the [start-simple practice](/practices/start-simple.md) and shapes how much you carry into [prototyping and development](/phases/prototyping-and-development.md).

## Workflow or agent

Not every task needs an autonomous agent. LangGraph frames a spectrum of workflow and agent patterns, each suited to different scenarios [5]: predefined-path workflows, orchestrator-worker workflows for when subtasks cannot be predefined [5], and evaluator-optimizer loops where feedback drives iterative refinement [5]. Prefer the most constrained pattern that solves the problem; reserve open-ended agency for tasks that genuinely require it.

## Precise identity and instructions

Precision in agent definition is critical: the model treats every part of the definition as a prompt, and vague descriptions can cause "context poisoning" that leads the agent to pursue incorrect goals [4]. Because agents select tools by name and description, unclear or non-unique descriptions produce looping behavior or incorrect actions [4], so tool definitions deserve as much prompt-engineering attention as the overall prompt [1]. Complex tasks benefit from few-shot examples and explicit guidance on tool use [4]. Formally, the Agentic Job Description defines an agent's jurisdictional boundaries, operational contexts, and epistemic evaluation criteria [6] — a scoping artifact that names what the agent may and may not do.

## Architecture: monolith, sub-agents, or agents-as-tools

Monolithic "super" agents crumble under instruction overload, producing inaccurate, brittle outputs that cannot scale [3]; splitting responsibilities across specialized agents with focused, domain-specific instructions improves quality and scalability [3]. When decomposing (see [multi-agent decomposition](/practices/multi-agent-decomposition.md)), the choice between agents-as-tools and sub-agents turns on control and context [2]. An agent-as-a-tool is a self-contained expert packaged for a specific, discrete, reusable task with encapsulated context [2] — appropriate for low-to-medium complexity, isolated or stateless work, and generic capabilities [2]. A sub-agent is a delegated team member handling a complex, multi-step, stateful process [2] — appropriate for high complexity, tasks needing conversational context, and specialized roles in specific workflows [2]. A coordinator or root agent understands the request and routes it to the correct specialist [3], and specialized agents can be wrapped as tools for sequential orchestration [3].

## Guarding against open-loop failure

Reliability comes not from model intelligence alone but from the structured interaction between agent and environment [6] and the engineering structures that anchor stochastic AI within deterministic business processes [6]. The Act-Verify-Refine loop closes the control loop, transforming execution results into verified knowledge assets [6] and driving behavior toward asymptotic convergence with mission requirements [6]. Scoping each agent with such a closed loop, rather than leaving it open-ended, is what makes domain agents verifiable and dependable [6].

# Citations

1. [Building Effective Agents](/sources/anthropic-building-effective-agents.md)
2. [Where to use sub-agents versus agents as tools](/sources/google-subagents-vs-agents-as-tools.md)
3. [Build multi-agentic systems using Google ADK](/sources/google-adk-multi-agent-systems.md)
4. [Startup technical guide: AI agents — production-ready AI](/sources/google-startup-guide-production-agents.md)
5. [Workflows and Agents — LangGraph Documentation](/sources/langchain-workflows-vs-agents.md)
6. [Agentic Problem Frames](/sources/arxiv-agentic-problem-frames.md)
7. [A dev's guide to production-ready AI agents](/sources/google-devs-guide-production-agents.md)
