---
type: Source
title: Effective harnesses for long-running agents
description: Anthropic material on the SDLC of AI agents.
resource: https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents
org: Anthropic
tags: [building-agents, agent-run-sdlc]
timestamp: 2026-07-06T00:00:00Z
---

# Summary

First-party material by Anthropic (published 2025-11-26). Key claims extracted below; the live document is the authority.

# Key claims

- Agents require specialized architectural patterns to bridge coding session gaps in long-running tasks
- Two-stage initialization with environment setup followed by incremental coding improves agent task completion
- Comprehensive feature requirement documentation prevents agents from prematurely declaring task completion
- Agents fail to verify end-to-end functionality without explicit visual/browser testing capabilities
- Providing visual testing tools dramatically improves agent bug detection and remediation
- Agent failures in autonomous development cluster around scope management, state documentation, and testing completeness
- Structured progress logs enable session continuity and reduce context window switching overhead
- Version control integration is critical for agent recovery from failures in autonomous development workflows
