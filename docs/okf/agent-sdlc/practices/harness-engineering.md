---
type: Practice
title: Harness engineering for long-running agents
description: Building the scaffolding — initialization stages, progress artifacts, context resets, checkpoints, version control, and locks — that carries autonomous agents across context-window boundaries.
tags: [agent-run-sdlc]
timestamp: 2026-07-06T00:00:00Z
---

# Harness engineering for long-running agents

Long-running agent tasks outlast a single context window, so agents need a way to bridge the gap between coding sessions [1]. Harness engineering is the discipline of building that scaffolding. Every component in a harness encodes an assumption about what the model cannot do on its own, which makes each piece worth stress testing and re-evaluating as model capabilities improve [2]. This work is distinct from the checkers of [/practices/verification-first-agent-coding.md](/practices/verification-first-agent-coding.md) and complements the discipline of [/practices/context-engineering.md](/practices/context-engineering.md); note that "harness" itself carries [/comparisons/two-meanings.md](/comparisons/two-meanings.md).

## Two-stage initialization

Splitting startup from steady-state work addresses environment-setup complexity, one of the primary failure patterns [1]. The pattern uses an initializer agent that sets up the environment on the first run, and a coding agent tasked with making incremental progress on later runs [1]. The initializer also writes a comprehensive file of feature requirements, which prevents the agent from prematurely declaring the task complete [1] — a defense against the "declarative victory" and "premature feature completion" failure modes [1].

## Progress logs and status files

Because agents suffer from undocumented progress across session boundaries [1], the harness externalizes state into durable artifacts. A `claude-progress.txt` file keeps a log of what agents have done [1], and long-running projects maintain extensive READMEs and progress files that are updated frequently [4]. These artifacts enable session continuity and reduce context-switching overhead [1]. Status output must also be agent-consumable: log all important information to a file so the agent can find it when needed [4], with grep-friendly formatting that fits agent context windows [4].

## Context resets over in-place compaction

When a context window fills, the harness can compact in place or reset to a clean slate. A reset provides a clean slate, at the cost of the handoff artifact having enough state for the next agent to pick up the work cleanly [2]. This trades reliance on lossy compaction for an explicit, curated handoff — the same motivation behind starting a fresh session for implementation once a spec is complete, giving clean context focused entirely on the work [3].

## Checkpoints, version control, and recovery

Recovery is the harness's failure path. Without it, after a break the agent would have to guess at what had happened and spend substantial time trying to get the basic app working again — a problem that persists even with compaction [1], which is why version control is the recovery substrate. Checkpointing supplements this: files are snapshotted before each change so a checkpoint can restore them, letting a run restore conversation only, code only, or both [3]. Continuous integration guards the substrate so new commits cannot break existing code [4], countering the tendency for new features and bugfixes to break existing functionality [4].

## Lock files for parallel agents

When multiple agent instances work in parallel on a shared codebase without active human intervention [4] in a loop that runs forever [4], the harness must prevent collisions. An agent takes a lock on a task by writing a text file to `current_tasks/` [4], which prevents duplicate work when several agents target the same task simultaneously [4]. At the session level, git worktrees give parallel sessions isolated checkouts so edits do not collide [3]. Together these mechanisms let a team of agents coordinate through the filesystem and version control rather than shared memory.

# Citations

1. [Effective harnesses for long-running agents](/sources/anthropic-effective-harnesses.md)
2. [Harness design for long-running application development](/sources/anthropic-harness-design-long-running-apps.md)
3. [Best practices for Claude Code](/sources/anthropic-claude-code-best-practices.md)
4. [Building a C compiler with a team of parallel Claudes](/sources/anthropic-building-c-compiler.md)
