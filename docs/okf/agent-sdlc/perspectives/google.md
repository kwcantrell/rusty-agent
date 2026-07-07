---
type: Perspective
title: Google's perspective
description: Google frames the agent SDLC as a platform-and-process discipline, wiring ADK multi-agent construction through Vertex trajectory evaluation, CI/CD quality gates, staged rollouts, and SRE agents that operate production.
tags: [building-agents, agent-run-sdlc]
timestamp: 2026-07-06T00:00:00Z
---

# Google's perspective

Google approaches the agent SDLC as a platform-and-process problem: a managed toolchain (ADK, Vertex AI, Cloud Run, Agent Engine) wrapped in operational disciplines it names GenOps and AgentOps. Building an agent is treated as a continuous lifecycle, not a one-off task [3].

## Platform and architecture

Google favors distributed, loosely-coupled multi-agent systems over monolithic "super" agents, which crumble under instruction overload and produce brittle, inaccurate outputs [2]. Specialized agents carry focused, domain-specific instructions; a coordinator or root agent routes requests to the right specialist [2]. The open-source Agent Development Kit (ADK) supplies code-first primitives — `SequentialAgent`, `ParallelAgent`, `AgentTool` — for composing and orchestrating these systems, with independent tasks run concurrently [2]. The architectural choice between packaging an agent *as a tool* (self-contained, stateless, reusable) versus a *sub-agent* (stateful, multi-step, hierarchical delegation) turns on control and context handling [9] — a distinction that parallels [LangChain's](/perspectives/langchain.md) subagent patterns and the broader [two meanings of "agent SDLC"](/comparisons/two-meanings.md). Modularity is also the key to Google's evaluation strategy, letting individual components be scored in isolation [5], with standardized inter-agent protocols (A2A) solving the N×N integration problem [5].

## Evaluation as the center of gravity

Because agentic systems are non-deterministic, standard unit tests are insufficient [3]; success depends on the full trajectory of decisions, not just final output [1][4]. Vertex AI's Gen AI evaluation service splits metrics into final-response and trajectory evaluation, offering six trajectory metrics — exact match, in-order, any-order, precision, recall, and single-tool use — at varying strictness [7]. Robust evaluation spans three pillars: agent success, process trajectory, and trust/safety [1], blending synthetic and production data with human evaluation to establish ground truth [1]. Google draws a sharp line from subjective "vibe checks" to systematic continuous evaluation, running discovery mode (rapid iteration, few samples) and defense mode (large-scale regression) as distinct disciplines [5]. Evaluation datasets are versioned like source code [5], and evaluators fetch live tool schemas at runtime to prevent test-suite drift [5]. This eval-first stance echoes [Anthropic's](/perspectives/anthropic.md) emphasis on measurement.

## GenOps: the operational backbone

GenOps (MLOps for Gen AI) frames the supporting machinery: prompt engineering and versioning, model evaluation with explainable metrics, side-by-side comparison, fine-tuning and RLHF, safety filters, and version control across models, prompts, and datasets [6].

## Continuous delivery and staged rollout

Evaluation suites integrate into CI/CD as automated quality gates that run on every proposed change, forming a "quality firewall" so failing metrics block the merge [1][5]. Safe releases use shadow revisions — a new version deployed alongside the live one, serving zero traffic while evaluated in the exact production environment, decoupling deployment from release [5]. Most teams deploy in stages: sandbox for internal testing, canary for limited exposure, then full production, validating at each step [4]. Production monitoring tracks operational metrics, quality metrics, and drift, feeding data back into evaluation assets as a virtuous cycle [1][4]. Distributed tracing (OpenTelemetry) correlates reasoning traces with physical execution across agents [5].

## SRE agents operating production

Google closes the loop with agentic AI running operations itself. SRE agents detect and auto-address reliability issues before human review [8], use dynamic anomaly detection instead of static SLO thresholds [8], triage and enrich alerts, investigate incidents, and autonomously propose or apply mitigations from observability, topology, and dependency data [8]. Historical incidents are mined via Gemini embeddings and vector databases to drive better investigations [8]. These agents carry strong identity with roles and permissions like human operators, must explain considered-and-rejected options over black-box automation, and are instrumented to track their autonomous levels [8].

# Citations

1. [A methodical approach to agent evaluation](/sources/google-methodical-agent-evaluation.md)
2. [Build multi-agentic systems using Google ADK](/sources/google-adk-multi-agent-systems.md)
3. [Startup technical guide: AI agents — production-ready AI](/sources/google-startup-guide-production-agents.md)
4. [A dev's guide to production-ready AI agents](/sources/google-devs-guide-production-agents.md)
5. [From 'Vibe Checks' to Continuous Evaluation: Engineering Reliable AI Agents](/sources/google-continuous-evaluation.md)
6. [Learn how to build and scale Generative AI solutions with GenOps](/sources/google-genops.md)
7. [Evaluate your AI agents with Vertex Gen AI evaluation service](/sources/google-vertex-agent-evaluation.md)
8. [How Google SRE is using agentic AI to improve operations](/sources/google-sre-agentic-ai.md)
9. [Where to use sub-agents versus agents as tools](/sources/google-subagents-vs-agents-as-tools.md)
