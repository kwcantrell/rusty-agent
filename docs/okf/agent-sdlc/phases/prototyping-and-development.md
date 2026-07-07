---
type: Lifecycle Phase
title: Prototyping and development
description: The build phase, where agents grow incrementally from direct API calls into structured systems by developing tools, context, and prompts in tandem under versioning discipline.
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# Prototyping and development

Prototyping and development is the build phase that follows [design and scoping](/phases/design-and-scoping.md): turning a scoped problem into a working agent. The through-line is restraint — add structure only when it earns its place, and treat the agent as something you grow rather than assemble once.

## Start direct, add framework only as needed

The most successful implementations avoid complex frameworks and specialized libraries, building instead with simple, composable patterns [1]. Developers should start by using LLM APIs directly, since many patterns can be implemented in a few lines of code [1]. Frameworks add abstraction layers that obscure the underlying prompts and responses, making the system harder to debug [1]. Complexity is therefore something to add deliberately — only when it demonstrably improves measurable outcomes [1] — and even then after the agent has been optimized with comprehensive evaluation [1]. Frameworks retain their place when teams need high control: a code-first kit lets developers build, manage, evaluate, and deploy agents [5], while no-code visual builders serve application-first work by non-technical members [5]. The choice is about matching structure to need, not defaulting to it.

## Develop tools and context alongside the agent

Tools are typically the load-bearing part of an agent, and their definitions and specifications deserve as much prompt-engineering attention as the overall prompt [1]. In practice, optimizing tools can consume more effort than optimizing the prompt itself [1]. Parameter format matters — some formats are far harder for a model to write correctly than others [1] — and tools should be error-proofed (poka-yoke) so arguments make mistakes harder to make [1]. Because agents pick a tool from its name and description, clear, unique naming avoids looping or incorrect actions [5]. This is [tool design as engineering](/practices/tool-design-as-engineering.md), developed hand-in-hand with the agent.

Context strategy evolves in parallel. System prompts should carry the smallest set of information that fully outlines expected behavior, starting minimal and adding examples based on observed failure modes [3], calibrated between hardcoded logic and vague guidance [3]. Tool sets should be minimally viable and non-overlapping, since redundant functionality creates ambiguous decision points [3]. The governing principle across every context component is to find the smallest set of high-signal tokens that maximize the desired outcome [3] — the core of [context engineering](/practices/context-engineering.md). Few-shot examples and explicit guidance on tool usage help on complex tasks [5].

## Prompt versioning and experimentation discipline

GenOps — MLOps for generative AI — provides the framework for building and maintaining these systems [4]. Prompt engineering to refine outputs is a first-class activity [4], and prompt versioning manages, tracks, and controls changes to prompts over time [4]. Version control extends across models, prompts, and dataset versions together [4], so experiments are reproducible and regressions traceable. Prototypes can be built and compared across enterprise and open-weight models [4], and LLMs themselves can generate improved prompts for a given task [4]. This experimentation discipline connects directly to [evaluation](/phases/evaluation.md): agents can iterate on code solutions using automated test results as feedback [1], and standing up quick tool prototypes for local testing [2] before broader deployment keeps iteration cheap.

## Development is continuous, not one-shot

Building an agent is a continuous cycle, not a one-off task [5]. Small refinements to tool descriptions can yield dramatic improvements, discoverable through evaluation-driven iteration [2], and agents can even analyze results to propose refinements to their own tools [2]. As underlying models become more capable, an agent's autonomy can scale, requiring less prescriptive engineering over time [3]. The development phase does not end at first working output — it loops.

# Citations

1. [Building Effective Agents](/sources/anthropic-building-effective-agents.md)
2. [Writing effective tools for AI agents—using AI agents](/sources/anthropic-writing-tools-for-agents.md)
3. [Effective context engineering for AI agents](/sources/anthropic-context-engineering.md)
4. [Learn how to build and scale Generative AI solutions with GenOps](/sources/google-genops.md)
5. [Startup technical guide: AI agents — production-ready AI](/sources/google-startup-guide-production-agents.md)
