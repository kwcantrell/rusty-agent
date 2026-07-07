---
type: Source
title: Going to Production with Deep Agents
description: LangChain material on the SDLC of AI agents.
resource: https://docs.langchain.com/oss/python/deepagents/going-to-production
org: LangChain
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# Summary

First-party material by LangChain (published 2025-07-02). Key claims extracted below; the live document is the authority.

# Key claims

- Memory and execution environments must respect user, assistant, and organizational boundaries to maintain data isolation in multi-tenant deployments
- User-scoped memory is the recommended default for most agent deployments
- CompositeBackend pattern enables combining thread-scoped scratch space with cross-thread persistent storage
- Sandboxed execution environments isolate agent workloads from host system resource exhaustion
- Production agent calls require stable thread_id and context parameters for reproducible and testable behavior
- Error classification strategy must differentiate between transient failures requiring retry and recoverable errors to feed back to the model
- LangSmith Deployments auto-provision infrastructure components including threads, runs, checkpointers, and stores
- Async-first design with native async tools and graph factories avoids threading overhead in I/O-bound agent workloads
- Checkpoint persistence enables worker recovery from crash and indefinite human-in-the-loop pauses without state loss
- Multi-tenant agent systems require three distinct authentication and authorization layers
- Shared memory in multi-tenant agent systems is a prompt injection attack vector if users can write to memory other users read
- Sandbox auth proxy intercepts outbound requests to inject credentials securely, keeping API keys out of sandbox code and logs
- Middleware-based data privacy redaction detects and masks emails, credit cards, and custom PII patterns before model inference
- Model and tool call limiting middleware prevents budget exhaustion from agent loops with configurable per-invocation or per-thread limits
- ModelRetryMiddleware handles transient model failures using exponential backoff strategy
- ModelFallbackMiddleware provides graceful model provider failure recovery by switching to alternative models
- LangSmith Cloud automatically sends all deployment traces to a project and monitors for issues with proposed fixes
- Deep agent workflows spanning many subagents require recursion limit configuration (recursionLimit: 10000) to prevent execution cutoff
- langgraph.json configuration file is required specification for both local development and production deployment
