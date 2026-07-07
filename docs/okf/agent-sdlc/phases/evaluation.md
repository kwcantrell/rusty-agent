---
type: Lifecycle Phase
title: Evaluation
description: The systematic measurement discipline at the center of the agent SDLC, turning subjective judgement into repeatable signal that gates every change and unlocks fast model upgrades.
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# Evaluation

Evaluation is the discipline that lets teams ship agents with confidence instead of catching issues only in production [1]. Relying on "vibe checks" — manually chatting with an agent to see if it feels right — is subjective, unscalable, and prone to confirmation bias [2]. Systematic evaluation replaces this with data-driven signal, which is why it sits at the center of the [agent SDLC](/practices/eval-driven-development.md) rather than at its end.

## Start early, from real failures

Writing evals is useful at any stage of the agent lifecycle [1]. Early investment forces teams to specify what success means before the agent can fulfill it, then iterate until performance thresholds are met [1]. A practical starting point is 20–50 simple tasks drawn from actual agent failures, written unambiguously with reference solutions and balancing positive and negative cases to avoid overfitting [1]. Without this, fixing one production failure tends to create others [1]. Clear, testable success criteria ground the entire strategy [3].

## Discovery vs defense

Evaluation runs in two distinct operational modes. Discovery mode favors rapid iteration over few samples — a prompt tweak can move success from 30% to 80%, and such shifts are visible with only a handful of test cases [9]. Defense mode is large-scale regression testing that protects a shipped quality bar [2]; its suite acts as a quality gate that runs automatically on every proposed change, failing the pipeline when metrics fall below thresholds [3][2].

## Offline vs online

Offline evaluation on curated, versioned datasets during development compares agent versions, benchmarks performance, and catches regressions before redeployment [5]. Online evaluation scores real user interactions on live traffic in real time, with configurable sampling rates to manage cost [5]. The two close a loop: production failures are captured and fed back into evaluation datasets, and production data flows back into eval assets as a virtuous cycle [5][3]. The most effective teams combine automated evals for fast iteration, production monitoring for ground truth, and periodic human review for calibration [1] — human testing surfaces edge cases and emergent behaviors that automated suites miss [9]. Because agents make a sequence of decisions, evaluation must assess the full [trajectory](/practices/trajectory-evaluation.md), not just final output [4]. Free-form outputs are best scored with a [calibrated LLM-as-judge](/practices/llm-as-judge-with-calibration.md), which aligns well with human judgement when kept simple [9].

## Benchmark limits

Benchmarks must be frequently refreshed to stay relevant as the field moves toward more demanding, realistic evaluations [6]. Single-trial scores overstate reliability: agent behavior is non-deterministic even with identical prompts [9], so multi-trial metrics like pass^k are needed — state-of-the-art function-calling agents succeed on under 50% of τ-bench tasks and are highly inconsistent (pass^8 under 25% in retail) [8]. Existing benchmarks also under-test user interaction and domain-rule following, both vital for deployment [8], alongside cost, safety, and robustness [6]. Multi-environment benchmarks are most useful when they support root-cause analysis — AgentBench traces most failures to poor long-term reasoning, decision-making, and instruction following [7].

## Saturation and fast upgrades

An eval set at 100% pass rate has saturated and stops providing signal for iteration [1]. Kept sharp, evals yield baselines and regression tests for latency, token usage, cost, and error rates for free [1], and they enable fast model upgrades: teams with evals can determine a new model's strengths, tune prompts, and upgrade in days while teams without them face weeks of manual testing [1]. Evaluation thus feeds directly into [testing and safety](/phases/testing-and-safety.md).

# Citations

1. [Demystifying evals for AI agents](/sources/anthropic-demystifying-evals.md)
2. [From 'Vibe Checks' to Continuous Evaluation](/sources/google-continuous-evaluation.md)
3. [A methodical approach to agent evaluation](/sources/google-methodical-agent-evaluation.md)
4. [Evaluate your AI agents with Vertex Gen AI evaluation service](/sources/google-vertex-agent-evaluation.md)
5. [LangSmith Evaluation Framework](/sources/langchain-langsmith-evaluation.md)
6. [Survey on Evaluation of LLM-based Agents](/sources/arxiv-agent-evaluation-survey.md)
7. [AgentBench: Evaluating LLMs as Agents](/sources/arxiv-agentbench.md)
8. [τ-bench: A Benchmark for Tool-Agent-User Interaction](/sources/arxiv-tau-bench.md)
9. [How we built our multi-agent research system](/sources/anthropic-multi-agent-research-system.md)
