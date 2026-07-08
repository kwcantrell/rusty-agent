---
type: Source
title: Improving Deep Agents with Harness Engineering
description: LangChain evidence that harness changes alone (model fixed) moved Terminal Bench 2.0 from 52.8% to 66.5%.
resource: https://www.langchain.com/blog/improving-deep-agents-with-harness-engineering
org: LangChain
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# Summary

First-party LangChain blog post on tuning the deep-agents harness for coding
tasks. Key claims extracted below; the live document is the authority.

# Key claims

- "The goal of a harness is to mold the inherently spiky intelligence of a model for tasks we care about" — harness engineering covers system prompts, tool choices, middleware, and execution flow
- Modifying only the harness (system prompt, tools, middleware) with the model held fixed (gpt-5.2-codex) raised Terminal Bench 2.0 from 52.8% to 66.5% — a 13.7-point gain attributable purely to harness design
- Build-verify loop: structured guidance for planning, building, testing, fixing; self-verification through testing is treated as the agent's primary error-detection channel
- PreCompletionChecklistMiddleware intercepts the agent before exit and enforces a verification pass against the task specification
- LocalContextMiddleware maps working directories and discovers available tooling at startup, reducing context-discovery errors during the run
- LoopDetectionMiddleware tracks per-file edit counts and prompts the agent to reconsider its approach after N edits, breaking "doom loops"
- Reasoning-budget strategy: an "xhigh-high-xhigh reasoning sandwich" — high compute on planning and verification, medium during implementation
- Harnesses are model-specific: Claude and Codex need different harness tailoring
- Stated principles: context engineering on the agent's behalf reduces error surface; tracing is the feedback signal for harness debugging
