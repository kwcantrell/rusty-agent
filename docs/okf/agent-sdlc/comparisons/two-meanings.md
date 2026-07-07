---
type: Comparison
title: SDLC for building agents vs SDLC run by agents
description: Distinguishes the eval-centered lifecycle for building agent products from the spec-first, human-gated workflows in which agents execute the SDLC, and shows where the two converge.
tags: [building-agents, agent-run-sdlc]
timestamp: 2026-07-06T00:00:00Z
---

"Agent SDLC" names two different things. One is a lifecycle for building agents as products; the other is a software lifecycle executed by agents. They impose different disciplines, and conflating them obscures both.

## Sense 1: the lifecycle for building agents

When the agent is the artifact, the artifact is probabilistic: agentic systems are non-deterministic, so standard unit tests are insufficient [7]. The process compensates with measurement. Building an agent is "a continuous cycle, not a one-off task" [7], and evals are useful at every stage of that cycle [6] — see [Evaluation](/phases/evaluation.md). Evals force teams to specify what success means before building toward it: capability-driven development defines planned behaviors first, then iterates until the agent meets them [6]. Testing shifts from output assertions to trajectory evaluation — grading the agent's step-by-step reasoning [7] — and effective harnesses combine deterministic, model-based, and human graders [6]. The payoff is operational: teams with evals get baselines and regression tracking for free and can adopt new models in days rather than weeks [6]. Even so, pre-deployment testing alone cannot capture real-world behavior — idealized capability assessments differ from what happens in practice — so post-deployment monitoring is part of the lifecycle, not an afterthought [9].

## Sense 2: agents running the SDLC

Here the artifact is ordinary software and the agents are the workforce. The field's central object has shifted "from code generation to delegated execution under human supervision" [1], with an explicit A-SDLC framework contrasted against the traditional SDLC [1]. The division of labor is consistent across evidence: people decide what to build, the agent decides how to build it [4]; HITL frameworks deployed in production (JIRA) keep engineers guiding planning and code generation [5], reflecting a documented trust gap — agent PRs across 456,000 real pull requests are faster but accepted less often than human ones [2]. Practice compensates with [spec-first workflows](/practices/spec-first-agent-workflows.md) — plan mode separates exploration from execution so agents don't solve the wrong problem, and completed specs are executed in a fresh session [3] — and [verification-first coding](/practices/verification-first-agent-coding.md): give the agent a pass/fail check and the loop closes on its own [3]. The compiler-team experiment shows the stakes: the task verifier must be nearly perfect or agents solve the wrong problem [8]. [Human gates](/practices/human-in-the-loop-gates.md) sit where judgment is irreplaceable: deploying never-verified software is a real concern even for capable autonomous teams [8], and effective oversight means trustworthy visibility plus simple intervention mechanisms, not prescriptive approval chains [9].

## Convergence

Both senses reduce to the same three moves. First, define success criteria machines can check: unambiguous eval tasks with reference solutions in sense 1 [6]; tests, builds, and specs that produce a pass or fail in sense 2 [3]. Reliability in either case comes from "rigorous engineering structures that anchor stochastic AI within deterministic business processes," not model intelligence alone [10]. Second, treat context as the scarce resource: agent performance degrades as context fills [3], and long-running agent teams compensate with grep-friendly outputs and persistent progress files [8]. Third, place humans where judgment is irreplaceable — calibration review in evals [6], planning and approval authority in execution [4][5].

## Recursion

The two compose: teams use agent-run SDLC to build agent products, and building agents itself increasingly runs through agents — operating software (deploying, configuring, monitoring) already comprises 17% of Claude Code sessions [4], agents in the wild initiate, review, and evolve code across the lifecycle [2], and the eval harnesses of sense 1 are exactly the machine-checkable verifiers sense 2 depends on [6][8].

# Citations

1. [Agentic AI in the Software Development Lifecycle](/sources/arxiv-agentic-sdlc-architecture.md)
2. [The Rise of AI Teammates in SE 3.0](/sources/arxiv-se3-ai-teammates.md)
3. [Best practices for Claude Code](/sources/anthropic-claude-code-best-practices.md)
4. [Agentic coding and persistent returns to expertise](/sources/anthropic-claude-code-expertise.md)
5. [Human-In-the-Loop Software Development Agents](/sources/arxiv-human-in-the-loop-sd-agents.md)
6. [Demystifying evals for AI agents](/sources/anthropic-demystifying-evals.md)
7. [Startup technical guide: AI agents](/sources/google-startup-guide-production-agents.md)
8. [Building a C compiler with a team of parallel Claudes](/sources/anthropic-building-c-compiler.md)
9. [Measuring AI agent autonomy in practice](/sources/anthropic-measuring-agent-autonomy.md)
10. [Agentic Problem Frames](/sources/arxiv-agentic-problem-frames.md)
