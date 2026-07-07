---
type: Source
title: Where to use sub-agents versus agents as tools
description: Google Cloud material on the SDLC of AI agents.
resource: https://cloud.google.com/blog/topics/developers-practitioners/where-to-use-sub-agents-versus-agents-as-tools/
org: Google Cloud
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# Summary

First-party material by Google Cloud (published 2025-11-07). Key claims extracted below; the live document is the authority.

# Key claims

- Control and context handling are the fundamental architectural differentiators between agents-as-tools and sub-agents.
- Agents-as-tools are self-contained expert agents designed for specific, discrete, reusable tasks with encapsulated context.
- Sub-agents handle complex, multi-step, stateful processes requiring shared context and hierarchical delegation.
- Task complexity (low-to-medium vs. high) determines whether to implement as tools or sub-agents.
- Context isolation requirements determine architectural choice: isolated/stateless tasks map to tools; conversational context requirements map to sub-agents.
- Reusability and scope guide implementation patterns: generic, widely applicable capabilities should be built as tools; specialized roles in specific workflows should be sub-agents.
- Google Cloud provides reference implementations and architecture documentation for multi-agent system design via the Agent Development Kit (ADK).
