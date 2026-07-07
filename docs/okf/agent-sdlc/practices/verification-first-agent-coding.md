---
type: Practice
title: Verification-first agent coding
description: Structuring agent development around machine-checkable pass/fail signals so an autonomous loop can grade its own work and converge on correct behavior.
tags: [agent-run-sdlc]
timestamp: 2026-07-06T00:00:00Z
---

# Verification-first agent coding

Verification-first agent coding treats machine-checkable feedback as the primary completion criterion for autonomous work. Give an agent something that produces a pass or fail, and the loop closes on its own: the agent does the work, runs the check, reads the result, and iterates until the check passes [2]. Without such explicit verification criteria, agents solve the wrong problem — letting an agent jump straight to coding produces code that satisfies a misread of intent [2], which is why exploration and planning are separated from execution (see [/practices/spec-first-agent-workflows.md](/practices/spec-first-agent-workflows.md)). Success is measured by observable artifacts rather than the agent's own say-so: git commits, passing test suites, and other verifiable signals of success [6], where a verified success requires both a success judgment and a hard verifiable signal [6].

## Harness quality determines effectiveness

The value of the loop is bounded by the quality of the checker. Test-harness quality is a critical determinant of agent effectiveness: the task verifier must be nearly perfect, otherwise the agent will solve the wrong problem [1]. Test output must also be agent-consumable — logged to files and grep-friendly so results fit the context window [1]. Because agents systematically praise their own outputs even when quality is mediocre [5], self-grading is unreliable and external evaluation frameworks are needed [5]. Building these checkers is itself a discipline (see [/practices/harness-engineering.md](/practices/harness-engineering.md)).

## Verification must be end-to-end

Agents make code changes and even run unit tests or curl against a dev server, yet fail to recognize that a feature does not work end-to-end [4]. Providing visual and browser-testing tools dramatically improves bug detection and fixing [4]; a runtime evaluator that navigates and screenshots the page catches implementation bugs that static code review misses [5]. Subjective criteria are operationalized by reframing "is this beautiful?" into "does this follow our design principles?", giving the agent something concrete to grade against, calibrated with few-shot examples [5].

## Independent verifiers and comparative validation

To remove the implementer's bias, a verification subagent runs in fresh context so the agent doing the work is not the one grading it [2]. A multi-role architecture of planner, generator, and evaluator outperforms single-agent systems, with sprint contracts defining what "done" means before code is written [5]. In one build a separate agent wrote tests while another wrote code to pass them [2]. Where a reference exists, comparative validation grounds correctness — GCC served as a known-good oracle to compare a new C compiler against [1] — and probabilistic sampling (a 1% or 10% random subset) verifies behavior across large suites without exhausting context [1].

## Act-Verify-Refine and regression prevention

Formally, this is a closed-loop control system: the Act-Verify-Refine loop transforms execution results into verified knowledge assets [3] and drives behavior toward asymptotic convergence with mission requirements [3]. Frameless development without such loops invites open-loop failures [3]. Tight feedback loops with quick correction outperform long sessions [2]. Finally, verification must persist: continuous integration prevents regressions so new commits cannot break existing code [1], since new features and bugfixes frequently break existing functionality [1]. Even with strong verification, a human touchpoint remains warranted — deploying software no one has personally verified is a real concern [1].

# Citations

1. [Building a C compiler with a team of parallel Claudes](/sources/anthropic-building-c-compiler.md)
2. [Best practices for Claude Code](/sources/anthropic-claude-code-best-practices.md)
3. [Agentic Problem Frames: A Systematic Approach to Engineering Reliable Domain Agents](/sources/arxiv-agentic-problem-frames.md)
4. [Effective harnesses for long-running agents](/sources/anthropic-effective-harnesses.md)
5. [Harness design for long-running application development](/sources/anthropic-harness-design-long-running-apps.md)
6. [Agentic coding and persistent returns to expertise](/sources/anthropic-claude-code-expertise.md)
