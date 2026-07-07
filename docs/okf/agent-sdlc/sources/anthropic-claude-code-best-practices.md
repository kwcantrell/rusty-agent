---
type: Source
title: Best practices for Claude Code
description: Anthropic material on the SDLC of AI agents.
resource: https://code.claude.com/docs/en/best-practices
org: Anthropic
tags: [agent-run-sdlc]
timestamp: 2026-07-06T00:00:00Z
---

# Summary

First-party material by Anthropic (published 2026-07-06). Key claims extracted below; the live document is the authority.

# Key claims

- Agent performance degrades as context window fills; context management is the fundamental constraint for autonomous agent development
- Autonomous agents require explicit verification criteria (tests, builds, screenshots) to close the feedback loop without human intervention
- Separating exploration and planning from implementation prevents agents from solving the wrong problem
- Persistent configuration files (CLAUDE.md) provide stable context for agent behavior across sessions
- Automatic permission classification enables unattended agent execution while blocking risky operations
- Deterministic stop hooks enforce verification gates for autonomous runs, with escalation timeout after 8 consecutive blocks
- Independent subagents in fresh context verify implementations without bias from the implementing agent
- Parallel agent sessions scale development through isolated contexts and git worktrees
- Non-interactive mode integrates agents into CI pipelines and automation frameworks
- Agent teams coordinate multiple sessions with shared tasks and messaging for scaled autonomous work
- Tool access control via allowlists scopes agent permissions for batch and unattended operations
- Specification-driven workflows separate spec authoring in a fresh session from implementation, reducing context pollution
- Multi-agent test-first patterns have one agent write tests and another write implementation code to pass them
- Hooks enforce deterministic actions at workflow checkpoints independent of agent reasoning
- MCP servers connect agents to external systems (issue trackers, databases, monitoring, design tools) for end-to-end automation
- Checkpointing enables rewind and recovery of agent sessions and code state across multiple interactions
- Streaming JSON output enables real-time processing of agent results in data pipelines
- Tight feedback loops with immediate correction outperform long sessions; rewind and restart with better prompts after two failed corrections
