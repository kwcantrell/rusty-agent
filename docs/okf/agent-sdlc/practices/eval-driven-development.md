---
type: Practice
title: Eval-driven development
description: Building agents by defining success criteria as evaluations first, then gating every change against a versioned dataset of real and adversarial cases.
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# Eval-driven development

Eval-driven development treats evaluations as the primary artifact of agent
building rather than an afterthought. Writing evals is useful at any stage of
the agent lifecycle [1], but investing early forces product teams to specify
what success means before the agent can fulfill it [1]. An effective evaluation
strategy is built on a foundation of clear, unambiguous success criteria [3],
defined at the outset of the evaluation strategy so they ground the whole approach [3]. In the
capability-first form of the practice, teams build evals to define planned
behaviors before agents can meet them, then iterate until performance thresholds
are reached [1]. See [/phases/evaluation.md](/phases/evaluation.md) for where
this sits in the lifecycle.

## Seeding the dataset

A practical starting point is 20-50 simple tasks drawn from actual agent
failures [1]. Tasks must be unambiguous and carry reference solutions so
automated grading is reliable [1], and the set should balance positive and
negative test cases to prevent overfitting [1]. The most robust suites blend
synthetic and production data to generate diverse, realistic cases at scale
[3]. AI agents themselves can accelerate seeding: given tool specifications,
Claude Code can quickly create dozens of prompt-and-response pairs [4]. Held-out
test sets guard against overfitting during iterative improvement [4].

## Evals as regression gates in CI

Once evals exist, teams get baselines and regression tests for free—latency,
token usage, cost per task, and error rates become trackable [1]. The
evaluation suite should act as a quality gate that runs automatically with
every proposed change [3]. In a CI/CD pipeline this becomes a quality firewall
so that code failing metric thresholds cannot physically reach production
users: the build fails the pipeline, the pipeline fails the commit, and the
merge is blocked [2]. Offline evaluation on curated datasets during development
lets teams compare versions, benchmark performance, and catch regressions [5],
with offline validation before redeployment preventing performance regressions
in production [5]. Grading is typically multi-method, combining deterministic
checks, model-based assessment, and human judgment [1]; beyond output
correctness, trajectory-level checks assess the sequence of agent decisions
(see [/practices/trajectory-evaluation.md](/practices/trajectory-evaluation.md)).

## Production failures loop back

Without systematic pre-launch evaluation, teams get stuck in reactive
loops—catching issues only in production—and fixing one failure creates
others [1]. The remedy is a feedback loop: production data is fed back into
evaluation assets to create a virtuous cycle of continuous improvement [3].
Failing production cases are integrated back into evaluation datasets for
iterative improvement [5], and captured production failures can be targeted to
create custom evaluators for future iterations [5]. The most effective teams
combine automated evals for fast iteration, production monitoring for ground
truth, and periodic human review for calibration [1]; reading transcripts
regularly validates and improves the grading logic [1]. Watch for eval
saturation—a 100% pass rate means the set has stopped providing signal [1].

## Versioned like source code

Evaluation quality depends on properly structured datasets that carry expected
outputs and reference trajectories, stored and versioned like source code [2].
Treating the dataset as a first-class, versioned artifact is what lets evals
serve as durable regression defense: when more powerful models arrive, teams
with evals can determine a model's strengths, tune prompts, and upgrade in days
rather than facing weeks of manual testing [1].

# Citations

1. [Demystifying evals for AI agents](/sources/anthropic-demystifying-evals.md)
2. [From 'Vibe Checks' to Continuous Evaluation](/sources/google-continuous-evaluation.md)
3. [A methodical approach to agent evaluation](/sources/google-methodical-agent-evaluation.md)
4. [Writing effective tools for AI agents](/sources/anthropic-writing-tools-for-agents.md)
5. [LangSmith Evaluation Framework](/sources/langchain-langsmith-evaluation.md)
