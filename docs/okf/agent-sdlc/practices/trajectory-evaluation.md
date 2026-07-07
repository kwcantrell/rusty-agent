---
type: Practice
title: Trajectory evaluation
description: Evaluating an agent's full decision path—tool selection, reasoning, and recovery—against reference trajectories rather than only its final output.
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# Trajectory evaluation

Trajectory evaluation assesses the full sequence of decisions and actions an agent
takes to reach a result, not just its final answer [3]. For systems that make a
sequence of decisions, metrics focused only on the final output are no longer
enough [2]; it is not enough to check outputs alone—one must understand the "why"
behind an agent's actions, its reasoning and the path it takes to a solution [1].
Agent evaluation therefore divides into two categories: assessment of final
responses and assessment of action trajectories [1]. Because agentic systems are
non-deterministic, standard unit tests are insufficient [4], and evaluating the
agent's step-by-step reasoning becomes a primary testing methodology for quality
and reliability [4].

## What the decision path exposes

Good trajectory evaluation examines tool selection, reasoning quality, error
recovery, and whether the agent asked clarifying questions when it should have [3].
It validates both tool selection and parameter correctness against reference
trajectories [6], which matters because agents choose which tool to use from its
name and description, and unclear choices lead to looping or incorrect actions [4].
These are foundational LLM capabilities—planning and tool use—that must be
systematically evaluated as autonomous systems plan, reason, and use tools while
interacting with dynamic environments [5]. Standard text-generation metrics such
as coherence are insufficient here, since they focus on text structure rather than
effectiveness within an environment [1].

## Matching and scoring over actions

Trajectory metrics apply at varying strictness levels [1]. Exact match requires a
trajectory that perfectly mirrors the ideal solution [1]. In-order match requires
all necessary actions in the correct order but tolerates extra, unnecessary
steps [1]. Any-order match cares only that all necessary actions are present,
regardless of order [1]. Precision measures the accuracy of the agent's actions,
while recall measures its ability to capture all essential actions [1]. Single-tool
use checks for the presence of a specific action, useful for assessing whether an
agent has learned to use a particular tool or capability [1]. Order-matching
constraints alongside precision and recall can be enforced through custom metrics
when tool usage is a mandatory part of an agent's flow [6].

## Reference trajectories in datasets

These metrics depend on evaluation datasets that pair each case with a ground-truth
path. A dataset should contain the user prompt, a reference trajectory, the
generated trajectory, and the response [1]. Evaluation quality depends on properly
structured, versioned datasets stored like source code, with expected outputs and
reference execution trajectories [6]—garbage in, garbage out, since evaluation is
only as good as its dataset [6]. To keep the suite from drifting, live tool schemas
can be fetched from running agents at evaluation time rather than hardcoded, so
reference definitions track the actual code [6].

## Where it fits

Trajectory evaluation is one component of a broader
[/phases/evaluation.md](/phases/evaluation.md) approach that also includes unit
tests for individual components and staged rollouts from sandbox to canary to
production [3]. It supplies the action-level signal that
[/practices/eval-driven-development.md](/practices/eval-driven-development.md)
turns into automated quality gates: an evaluation suite acts as a gate that runs
automatically with every proposed change [2], and custom metrics can assess
whether an agent's response logically follows from its tool choices [1].

# Citations

1. [Evaluate your AI agents with Vertex Gen AI evaluation service](/sources/google-vertex-agent-evaluation.md)
2. [A methodical approach to agent evaluation](/sources/google-methodical-agent-evaluation.md)
3. [A dev's guide to production-ready AI agents](/sources/google-devs-guide-production-agents.md)
4. [Startup technical guide: AI agents — production-ready AI](/sources/google-startup-guide-production-agents.md)
5. [Survey on Evaluation of LLM-based Agents](/sources/arxiv-agent-evaluation-survey.md)
6. [From 'Vibe Checks' to Continuous Evaluation: Engineering Reliable AI Agents](/sources/google-continuous-evaluation.md)
