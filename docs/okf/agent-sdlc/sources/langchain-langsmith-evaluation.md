---
type: Source
title: LangSmith Evaluation Framework
description: LangChain material on the SDLC of AI agents.
resource: https://docs.langchain.com/langsmith/evaluation
org: LangChain
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# Summary

First-party material by LangChain (published 2026-07-06). Key claims extracted below; the live document is the authority.

# Key claims

- Offline evaluation on curated datasets during development enables comparison of agent versions and detection of regressions.
- LangSmith supports multiple evaluator types including human review, code-based rules, LLM-as-judge, and pairwise comparison.
- Online evaluation in production monitors real user interactions in real-time to detect issues and measure quality on live traffic.
- Automated evaluation can be applied to production traces with configurable sampling rates to manage costs during monitoring.
- LangSmith enables benchmarking, unit testing, regression testing, and backtesting workflows for agent validation.
- Production traces are automatically generated during application deployment to enable post-hoc evaluation.
- Real-time quality assessment and anomaly detection are applied during production monitoring of agent systems.
- Production failures can be captured and targeted to create custom evaluators for future iterations.
- Offline validation before redeployment prevents regression of agent performance in production.
- A feedback loop integrates failing production cases back into evaluation datasets for iterative improvement.
