---
type: Practice
title: LLM-as-judge with human calibration
description: Grading agent outputs with a rubric-driven model judge that is calibrated to and periodically validated against human judgment, combined with deterministic checks and human review.
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# LLM-as-judge with human calibration

Much of an agent's output is free-form and subjective, so it cannot be graded by exact-match assertions alone. An LLM-as-judge evaluator scores such outputs, and a single LLM call with a single prompt that outputs a score from 0.0 to 1.0 has been found to be the most consistent approach and the most aligned with human judgments [1]. LangSmith and similar frameworks expose LLM-as-judge as one evaluator type alongside human review, code-based rules, and pairwise comparison [2]. This practice sits inside the broader [evaluation phase](/phases/evaluation.md) and complements [trajectory evaluation](/practices/trajectory-evaluation.md), which grades tool selection and execution paths rather than output quality.

## Rubric-based grading, not vibe checks

Relying on manually chatting with an agent to see whether it "feels right" is subjective, unscalable, and susceptible to confirmation bias [3]. The systematic alternative is rubric-based LLM-as-judge scoring against explicit criteria [3]. Subjective judgments become tractable when reframed as concrete rubrics: "Is this design beautiful?" is hard to answer consistently, but "does this follow our principles for good design?" gives the model something concrete to grade against [4]. Eval tasks themselves should be written unambiguously, with reference solutions, so that automated grading is reliable [5].

## Few-shot calibration to human judgment

Subjective quality judgments can be operationalized through explicit grading criteria paired with few-shot calibration examples that anchor the judge to the intended standard [4]. Calibration matters because models are biased evaluators of their own work: an agent asked to assess its own output tends to respond by confidently praising it even when the quality is obviously mediocre to a human observer, which is why an external evaluation framework is needed [6].

## Periodic transcript review to validate the grader

A judge can drift or grade incorrectly, so its logic must itself be checked. Reading transcripts regularly is necessary to validate and improve automated grading [7]. The most effective teams combine automated evals for fast iteration, production monitoring for ground truth, and periodic human review for calibration [8]. Human testing also surfaces edge cases that evals miss, such as hallucinated answers on unusual queries and system failures [9].

## Combine with deterministic checks and human review

LLM-as-judge is one grader among several, not the whole harness. Effective eval harnesses use multi-method grading that combines deterministic checks, model-based assessment, and human judgment [10]. For interactive outputs, the evaluator can navigate the page itself, screenshotting and studying the implementation before producing its assessment, catching runtime bugs that static review misses [11]. Layering deterministic assertions, a calibrated model judge, and human review keeps grading both scalable and trustworthy.

# Citations

1. [How we built our multi-agent research system](/sources/anthropic-multi-agent-research-system.md)
2. [LangSmith Evaluation Framework](/sources/langchain-langsmith-evaluation.md)
3. [From 'Vibe Checks' to Continuous Evaluation](/sources/google-continuous-evaluation.md)
4. [Harness design for long-running application development](/sources/anthropic-harness-design-long-running-apps.md)
5. [Demystifying evals for AI agents](/sources/anthropic-demystifying-evals.md)
6. [Harness design for long-running application development](/sources/anthropic-harness-design-long-running-apps.md)
7. [Demystifying evals for AI agents](/sources/anthropic-demystifying-evals.md)
8. [Demystifying evals for AI agents](/sources/anthropic-demystifying-evals.md)
9. [How we built our multi-agent research system](/sources/anthropic-multi-agent-research-system.md)
10. [Demystifying evals for AI agents](/sources/anthropic-demystifying-evals.md)
11. [Harness design for long-running application development](/sources/anthropic-harness-design-long-running-apps.md)
