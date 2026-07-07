---
type: Lifecycle Phase
title: Monitoring and operations (AgentOps)
description: The post-deployment phase where production traces, online evaluation, drift detection, and autonomy measurement keep live agents observable and safe, feeding failures back into evaluation and increasingly using agents themselves as operators.
tags: [building-agents, agent-run-sdlc]
timestamp: 2026-07-06T00:00:00Z
---

# Monitoring and operations (AgentOps)

Building an agent is a continuous cycle, not a one-off task, and the operational rigor of running a deployed agent safely in production is what Google calls AgentOps [1]. Because pre-deployment testing alone cannot capture how agents behave in real-world scenarios, and idealized capability assessments do not reflect what happens in practice, post-deployment monitoring is essential for understanding how agents are actually used [2]. This phase follows [/phases/deployment.md](/phases/deployment.md) and closes the loop back into [/practices/eval-driven-development.md](/practices/eval-driven-development.md).

## Production tracing

Production traces are generated automatically on release, enabling post-hoc evaluation of real interactions [4]. Comprehensive production tracing lets teams diagnose why agents failed and fix issues systematically — critical because agents make dynamic decisions and are non-deterministic between runs even with identical prompts [7]. For distributed multi-agent systems, logging is not enough: distributed tracing via OpenTelemetry correlates each agent's reasoning trace with the physical execution flow, revealing whether a wrong answer came from bad orchestrator logic, bad data from a sub-agent, or a network timeout [6]. Shared utility components such as tracing middleware across all services keep observability consistent [6]. Monitoring can operate on decision patterns and interaction structures without inspecting the contents of individual conversations, preserving privacy [7]. A known gap: model providers still lack a reliable way to link independent public-API requests into coherent agent sessions [2].

## Online evaluation and drift detection

Beyond offline evaluation on curated datasets, online evaluation assesses real user interactions in real time to detect issues and measure quality on live traffic [4]. Automated evaluators run on production traces with configurable sampling rates to manage cost, applying real-time quality assessment and anomaly detection [4]. Production monitoring should track operational metrics, quality metrics, and drift detection together [3]. Google SRE augments static SLI/SLO thresholds with AI-based anomaly detection that alerts on deviations from regular behavior rather than predefined thresholds [5].

## Autonomy measurement

Because today's autonomous systems are powerful and not always deterministic, teams instrument them to track autonomous levels and understand how autonomous a system truly is [5]. Real-world data can show agents improving while needing less oversight: Claude Code's success rate on the hardest internal tasks doubled while average human interventions per session fell from 5.4 to 3.3 [2]. Effective oversight requires trustworthy visibility into what agents are doing plus simple intervention mechanisms — not merely approval chains — while overly prescriptive interaction requirements add friction without safety benefit [2].

## Feedback loop into evaluation

Production data should feed back into evaluation assets to create a virtuous cycle of continuous improvement [3]. Failing production cases and targeted failures become custom evaluators and are integrated back into evaluation datasets for future iterations [4]. This is the operational engine of eval-driven development, turning live failures into regression guards.

## Agents as operators (SRE automation)

Agents increasingly run operations themselves. Google SRE built agents that continuously monitor and improve playbooks and generate new ones from incidents [5], triage agents that group, pre-process, and enrich alerts [5], and investigation agents that autonomously form hypotheses and propose or apply mitigations using observability data, system topology, and dependency information [5]. AI Insights extracts knowledge from historical incidents via Gemini embeddings and vector databases to drive better future investigations [5], and agents draft postmortems automatically [5]. This does not remove people; a number of issues are auto-addressed before human review, reducing effort while retaining oversight [5]. Operator agents must favor transparency over black-box automation by explaining their reasoning and rejected options [5], hold strong identities with assigned roles and permissions [5], and be continuously evaluated against a quality framework with auditing support [5]. Such systems require continuous access to production data to decide correctly [5].

# Citations

1. [Startup technical guide: AI agents — production-ready AI](/sources/google-startup-guide-production-agents.md)
2. [Measuring AI agent autonomy in practice](/sources/anthropic-measuring-agent-autonomy.md)
3. [A methodical approach to agent evaluation](/sources/google-methodical-agent-evaluation.md)
4. [LangSmith Evaluation Framework](/sources/langchain-langsmith-evaluation.md)
5. [How Google SRE is using agentic AI to improve operations](/sources/google-sre-agentic-ai.md)
6. [From 'Vibe Checks' to Continuous Evaluation: Engineering Reliable AI Agents](/sources/google-continuous-evaluation.md)
7. [How we built our multi-agent research system](/sources/anthropic-multi-agent-research-system.md)
