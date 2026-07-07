---
type: Perspective
title: Anthropic's perspective
description: Anthropic frames the agent SDLC as simplicity-first design plus context engineering, made rigorous by evals and harnesses and validated by large-scale evidence from Claude Code usage.
tags: [building-agents, agent-run-sdlc]
timestamp: 2026-07-06T00:00:00Z
---

# Anthropic's perspective

Anthropic's writing on agents converges on a consistent thesis: start simple, treat context as the scarce resource, make progress measurable with evals, scaffold autonomy with harnesses, and validate all of it against real deployment data.

## Simplicity first

The most successful agent implementations avoid complex frameworks in favor of simple, composable patterns [1]. Developers should begin by using LLM APIs directly, since many patterns take only a few lines of code, whereas frameworks add abstraction layers that obscure the underlying prompts and responses and make debugging harder [1]. Complexity is earned, not assumed: add it only when it demonstrably improves measurable outcomes [1]. Within that minimal footprint, tools carry disproportionate weight—teams often spend more time optimizing tools than the overall prompt [1], and small refinements to tool descriptions can yield dramatic improvements [2]. This is the shared boundary with LangChain's workflow-versus-agent framing; see [/perspectives/langchain.md](/perspectives/langchain.md).

## Context engineering as the core discipline

Context is finite and degrades: LLMs lose focus as context grows ("context rot"), because transformer attention creates n-squared token relationships that stretch capacity thin [3]. The governing rule across every context component is to find the smallest set of high-signal tokens that maximize the desired outcome [3]. Practically, this means minimal-but-sufficient system prompts calibrated to task complexity [3], minimal non-overlapping toolsets [3], just-in-time retrieval via lightweight identifiers rather than upfront pre-loading [3], compaction that preserves architectural decisions while discarding redundancy [3], persistent external notes [3], and specialized sub-agents that return condensed 1,000–2,000-token summaries [3]. Crucially, model capability and autonomy scale together—smarter models need less prescriptive engineering [3].

## Evals demystified

Evals let teams ship confidently instead of catching issues only in production [4], and force teams to specify what success means [4]. A practical start is 20–50 unambiguous tasks drawn from actual failures, with reference solutions and balanced positive/negative cases [4]. The most effective teams combine automated evals for fast iteration, production monitoring for ground truth, and periodic human review for calibration [4], using deterministic, model-based, and human graders together [4]. Teams with evals adopt new models in days rather than weeks [4]. Even tiny sets have signal—a prompt tweak can move success from 30% to 80% visibly across a few cases [6]—while a 100% pass rate signals saturation [4].

## Harness engineering for autonomy

Long-running agents need scaffolding to bridge sessions: two-stage initialization, comprehensive feature-requirement files, structured progress logs, and visual/browser testing that catches end-to-end failures unit tests miss [5]. Every harness component encodes an assumption about what the model cannot do yet, and should be stress-tested as models improve [7]. Because agents praise their own mediocre work, evaluation must be externalized—separate planner/generator/evaluator roles with explicit "done" contracts and few-shot grading criteria [7]. Claude Code operationalizes this with verification loops, plan mode, CLAUDE.md, hooks, subagent verification, and worktrees [11]. The C-compiler team pushes it furthest: parallel Claudes coordinating via lock files and CI, gated by a near-perfect verifier; the author flags autonomous deployment without personal human review as a standing concern [10].

## Large-scale evidence

Anthropic uniquely backs these claims with deployment data. Post-deployment monitoring shows Claude Code's success rate on the hardest internal tasks doubled while human interventions fell from 5.4 to 3.3 per session—behavior invisible to idealized pre-deployment tests [8]. Expertise persists: the more domain expertise a user brings, the more work Claude does per instruction, and expert-led sessions recover from trouble at 15% versus 4% for novices, with each prompt triggering roughly 10 agent actions [9]. This grounds Anthropic's two operative meanings of the SDLC; see [/comparisons/two-meanings.md](/comparisons/two-meanings.md), and contrast with [/perspectives/google.md](/perspectives/google.md).

# Citations

1. [Building Effective Agents](/sources/anthropic-building-effective-agents.md)
2. [Writing Effective Tools for AI Agents](/sources/anthropic-writing-tools-for-agents.md)
3. [Effective Context Engineering for AI Agents](/sources/anthropic-context-engineering.md)
4. [Demystifying Evals for AI Agents](/sources/anthropic-demystifying-evals.md)
5. [Effective Harnesses for Long-Running Agents](/sources/anthropic-effective-harnesses.md)
6. [How We Built Our Multi-Agent Research System](/sources/anthropic-multi-agent-research-system.md)
7. [Harness Design for Long-Running Application Development](/sources/anthropic-harness-design-long-running-apps.md)
8. [Measuring AI Agent Autonomy in Practice](/sources/anthropic-measuring-agent-autonomy.md)
9. [Agentic Coding and Persistent Returns to Expertise](/sources/anthropic-claude-code-expertise.md)
10. [Building a C Compiler with a Team of Parallel Claudes](/sources/anthropic-building-c-compiler.md)
11. [Best Practices for Claude Code](/sources/anthropic-claude-code-best-practices.md)
