---
type: Source
title: Building Effective Agents
description: Anthropic material on the SDLC of AI agents.
resource: https://www.anthropic.com/research/building-effective-agents
org: Anthropic
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# Summary

First-party material by Anthropic (published 2024-12-19). Key claims extracted below; the live document is the authority.

# Key claims

- Simple, composable patterns outperform complex frameworks for agent implementation
- Developers should start by using LLM APIs directly, as many patterns can be implemented with minimal code
- Frameworks introduce abstraction layers that obscure prompts and responses, hindering debugging
- Tool optimization requires more effort than overall prompt engineering for agent performance
- Code solutions can be verified through automated tests, enabling agents to iterate using test feedback
- Organizations must optimize agents with comprehensive evaluation before increasing system complexity
- Complexity should only be added when it demonstrably improves measurable outcomes
- Extensive tool testing with varied inputs reveals model mistakes and guides iterative improvement
- Sandboxed testing environments with appropriate guardrails are essential before autonomous deployment
- Agents should incorporate human checkpoints for feedback and blocker resolution in deployment
- Usage-based pricing models validate agent effectiveness by charging only for successful resolutions
- Tool parameter format significantly impacts LLM ability to write correct tool calls
- Tool definitions and specifications warrant equivalent prompt engineering effort as overall agent prompts
- Tools should be designed with error-proofing (poka-yoke) principles to reduce user mistakes
