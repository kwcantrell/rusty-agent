---
type: Source
title: Deep Agents (LangChain blog)
description: LangChain's four-pillars thesis distinguishing deep agents from shallow tool-calling loops.
resource: https://www.langchain.com/blog/deep-agents
org: LangChain
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# Summary

First-party LangChain blog post introducing the "deep agents" concept and the
`deepagents` package. Key claims extracted below; the live document is the
authority.

# Key claims

- "Shallow" agents run tool calls in a loop without strategic planning over longer horizons; deep agents overcome this with four integrated components
- Pillar 1 — detailed system prompt: the best coding and deep-research agents (Claude Code cited as the exemplar) carry complex system prompts with extensive instructions and few-shot examples
- Pillar 2 — planning tool: Claude Code's todo-list tool is a deliberate no-op — it performs no computation and exists purely as a context-engineering strategy to keep the agent on track over long tasks
- Pillar 3 — sub-agents: spawning specialized sub-agents per task provides context management and prompt shortcuts — each sub-agent goes deep on one domain instead of one monolithic context attempting everything
- Pillar 4 — file system access: Claude Code and Manus use persistent file storage for note-taking and as a shared workspace, addressing accumulated-context management on long-running operations
- The open-source `deepagents` package ships built-in implementations of all four pillars, including a virtual file system integrated with LangGraph agent state
