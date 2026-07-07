---
type: Source
title: How we built our multi-agent research system
description: Anthropic material on the SDLC of AI agents.
resource: https://www.anthropic.com/engineering/multi-agent-research-system
org: Anthropic
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# Summary

First-party material by Anthropic (published 2025-06-13). Key claims extracted below; the live document is the authority.

# Key claims

- Subagents operating in parallel with separate context windows enable token-efficient compression by processing multiple research threads concurrently.
- Multi-agent systems achieve significant performance improvements despite consuming substantially more tokens than single-agent chats.
- Prompt engineering is the primary lever for steering agent behavior and improving multi-agent system performance.
- Tool descriptions critically influence agent navigation; poorly designed tool interfaces send agents down incorrect execution paths.
- Extended thinking capabilities enhance instruction-following, reasoning depth, and overall efficiency in agent systems.
- Parallel tool calling dramatically accelerates complex task execution by reducing sequential overhead.
- Small sample-based evaluation detects significant behavioral changes; minor prompt tweaks produce measurable performance deltas on minimal test sets.
- LLM-as-judge evaluation produces consistent scoring alignment with human judgment for free-form agent outputs.
- Human testing uncovers edge-case failures and emergent behaviors that automated evaluation suites miss.
- Small changes to lead agent prompts produce unpredictable cascading effects on subagent behavior.
- Comprehensive production tracing enables systematic diagnosis of agent failure modes and iterative remediation.
- Agent non-determinism persists across identical prompts and inputs, complicating reproducible debugging.
- High-level observability of agent decision patterns and interaction structures enables privacy-preserving monitoring.
- Stateful agents require fault-tolerant execution systems capable of resuming from failure points.
- Graceful error handling where agents learn of tool failures and adapt their strategy improves robustness.
- Rainbow deployments enable safe version transitions by running old and new agent implementations simultaneously with graduated traffic shifts.
- Memory systems prevent context overflow by spawning fresh subagents with isolated contexts while maintaining continuity through handoff protocols.
- Artifact filesystem systems reduce information loss by providing subagents direct access to persistent state instead of full round-trip communication through the lead agent.
