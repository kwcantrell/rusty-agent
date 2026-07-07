---
type: Lifecycle Phase
title: Deployment
description: The phase in which validated agents are shipped to production through staged rollouts, traffic-isolated revisions, eval-gated pipelines, and agent-specific runtime infrastructure.
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# Deployment

Moving an agent from prototype to production requires infrastructure and release
patterns designed for systems that maintain state, use tools dynamically, and act
autonomously; traditional deployment patterns need adaptation because agents reason,
act, and adapt rather than behaving like deterministic code [1]. Deployment is not a
one-off task but part of a continuous lifecycle of iteration and management [5].

## Staged rollouts

Most teams deploy in stages: a sandbox for internal testing, a canary for limited
real-world exposure, and production for full rollout, with each stage validating
performance and catching issues before access expands [1]. A practical evaluation
approach pairs this progression with unit tests for individual components and
trajectory analysis for multi-step decision sequences [1] (see
[/phases/testing-and-safety.md](/phases/testing-and-safety.md)). Agents are typically
deployed to managed platforms such as Vertex AI Agent Engine or Cloud Run and operated
under the operational discipline of AgentOps [5].

## Shadow revisions and rainbow deployments

Safe promotion of agent changes relies on shadow revisions: a new version is deployed
alongside the live one but serves zero production traffic while it is evaluated in the
exact production environment — same network, secrets, and latency characteristics —
decoupling deployment (code on the server) from release (users seeing the code) [2].
Shadow revisions also serve as the target for large-scale evaluation, where many
concurrent requests are run against them to capture reasoning traces and execution
histories for root-cause analysis [2]. For the transition itself, rainbow deployments
keep old and new implementations running simultaneously and gradually shift traffic
from one to the other, enabling safe version changes for stateful, long-running agents
[3].

## CI/CD quality gates driven by eval metrics

Production pipelines enforce automated, evaluation-based quality gates that prevent
code from reaching users when metrics fall below thresholds — a "quality firewall" in
which a failing build fails the pipeline, commit, PR, and merge [2]. These gates draw
on versioned evaluation datasets treated like source code, with expected outputs and
reference trajectories [2], and on custom metrics that validate tool selection and
parameter correctness [2]. To keep the suite from drifting, evaluators fetch live tool
schemas from running agents at evaluation time (e.g. via an `/agent-info` endpoint)
rather than hardcoding definitions [2]. Because minor prompt changes can move success
rates sharply, even small test sets surface meaningful behavioral deltas before
release [3].

## Agent-specific infrastructure

Production agents need infrastructure distinct from standard software: session
management to maintain context across interactions, persistent memory systems for
long-term recall, tool integration with appropriate authentication and permissions,
and real-time logging to trace decisions and actions [1]. Memory and execution
environments must respect user, assistant, and organizational boundaries, with
user-scoped memory as the recommended default and composite backends combining
thread-scoped scratch space with cross-thread persistent storage [4]. Multi-tenancy
requires three distinct layers — end-user identity verification, authorization
handlers controlling resource access, and credential injection [4] — and shared memory
is a prompt-injection vector when one user can write to memory another user reads [4].
A sandbox auth proxy can intercept outbound requests and inject authentication headers,
keeping API keys out of sandbox code and logs [4]. Deployment platforms may
auto-provision threads, runs, checkpointers, and stores [4], and checkpoint persistence
lets another worker resume a crashed run from the latest checkpoint without
reprocessing [4]. Fault-tolerant execution that resumes from the point of failure is
essential for stateful agents [3]. Once live, agents move into
[/phases/monitoring-and-operations.md](/phases/monitoring-and-operations.md), where
tracing diagnoses failures and informs the next iteration [3].

# Citations

1. [A dev's guide to production-ready AI agents](/sources/google-devs-guide-production-agents.md)
2. [From 'Vibe Checks' to Continuous Evaluation](/sources/google-continuous-evaluation.md)
3. [How we built our multi-agent research system](/sources/anthropic-multi-agent-research-system.md)
4. [Going to Production with Deep Agents](/sources/langchain-deepagents-production.md)
5. [Startup technical guide: AI agents — production-ready AI](/sources/google-startup-guide-production-agents.md)
