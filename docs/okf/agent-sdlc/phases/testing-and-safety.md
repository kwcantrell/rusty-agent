---
type: Lifecycle Phase
title: Testing and safety
description: The pre-deployment assurance phase that layers sandboxed execution, deterministic and model-based guardrails, permissioning, policy-adherence testing, and multi-tenant isolation to make agent behavior safe before autonomous release.
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# Testing and safety

Testing and safety is the assurance work that sits between [evaluation](/phases/evaluation.md) and [deployment](/phases/deployment.md): it establishes the operational controls that keep an agent within bounds once it acts autonomously. Extensive testing in sandboxed environments, paired with appropriate guardrails, is essential before autonomous deployment [2]. Sandboxes provide both a filesystem and an execute tool for shell commands inside an isolated container, so agent workloads cannot exhaust or reach host resources [3].

## Layered guardrails

Guardrails validate and filter content at key execution points to build safe, compliant applications [1]. Middleware can intercept execution at strategic boundaries — before the agent starts and after it completes — to place safety controls precisely [1]. Two guardrail styles compose: deterministic rule-based logic (regex, keyword matching) is fast, predictable, and cost-effective [1], while model-based guardrails use LLMs or classifiers to catch subtle issues that rules miss [1]. Stacking multiple guardrails in the middleware array builds layered, defense-in-depth protection [1]. The most consequential layer is human: human-in-the-loop middleware is warranted for financial transactions, transfers, and deleting or modifying production data [1], and runtime approval gates can pause before every sensitive or destructive tool call [4]. Agents should pause for human feedback at checkpoints or when they hit blockers [2] — see [human-in-the-loop gates](/practices/human-in-the-loop-gates.md).

## Permissioning and allowlists

Access control should be declarative. Permission models evaluate rules top to bottom, and the first matching rule wins; if no rule matches, the operation is allowed [4]. Resource-limiting middleware such as model- and tool-call limits, configured per invocation or per conversation, prevents budget exhaustion from runaway agent loops [3].

## Policy adherence testing

Because agentic systems are non-deterministic, standard unit tests are insufficient [6]; trajectory evaluation of the agent's step-by-step reasoning becomes the primary method for checking quality and reliability [6]. Existing benchmarks do not test agents on interaction with human users or on following domain-specific rules, both vital for real-world deployment [5]. Reliability cannot be judged from a single trial: a pass^k metric measures consistency across multiple trials [5], and state-of-the-art function-calling agents remain inconsistent (pass^8 below 25% in retail) [5]. Comparing final database state against an annotated goal state gives a faithful check of task completion [5], and the field still needs methods that make agents act consistently and follow rules reliably [5].

## Multi-tenant isolation and prompt-injection surfaces

Memory and execution environments must respect user, assistant, and organizational boundaries [3], with user-scoped memory as the recommended default [3]. Multi-tenancy requires three distinct layers: end-user identity verification, authorization handlers controlling resource access, and credential injection [3]. Shared memory is a prompt-injection vector — if one user can write to memory another user's conversation reads, a malicious user could inject instructions [3]. Credentials should stay out of reach: a sandbox auth proxy intercepts outbound requests and injects authentication headers, keeping API keys out of sandbox code and logs [3]. PII middleware detects and redacts, masks, or hashes emails, credit cards, and custom patterns before content reaches models or logs [3]. Context poisoning is a related surface: models treat every part of an agent's definition as a prompt, so vague descriptions can steer the agent toward incorrect goals [6]. Verifiable code paths help close the loop — code solutions are checkable through automated tests, letting agents iterate on test feedback [2].

# Citations

1. [Guardrails for Agent Governance](/sources/langchain-guardrails.md)
2. [Building Effective Agents](/sources/anthropic-building-effective-agents.md)
3. [Going to Production with Deep Agents](/sources/langchain-deepagents-production.md)
4. [Deep Agents Overview](/sources/langchain-deepagents-overview.md)
5. [τ-bench: A Benchmark for Tool-Agent-User Interaction in Real-World Domains](/sources/arxiv-tau-bench.md)
6. [Startup technical guide: AI agents — production-ready AI](/sources/google-startup-guide-production-agents.md)
