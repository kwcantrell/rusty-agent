---
type: Source
title: Harness design for long-running application development
description: Anthropic material on the SDLC of AI agents.
resource: https://www.anthropic.com/engineering/harness-design-long-running-apps
org: Anthropic
tags: [agent-run-sdlc]
timestamp: 2026-07-06T00:00:00Z
---

# Summary

First-party material by Anthropic (published 2026-03-24). Key claims extracted below; the live document is the authority.

# Key claims

- Context resets provide cleaner state transitions than in-place compaction for multi-session agent work
- Agents exhibit systematic bias when evaluating their own outputs, requiring external evaluation frameworks
- Multi-agent specialization with distinct planner, generator, and evaluator roles outperforms single-agent systems on complex tasks
- Feature-level decomposition with explicit sprint contracts defining completion criteria improves multi-agent coherence
- Harness scaffolding should be re-evaluated when model capabilities improve to avoid unnecessary complexity
- Subjective quality judgments can be operationalized through explicit grading criteria and few-shot calibration examples
- Runtime interaction testing with browser automation catches implementation bugs that static code review misses
