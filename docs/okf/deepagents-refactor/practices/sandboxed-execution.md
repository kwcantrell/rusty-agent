---
type: Practice
title: Sandboxed execution behind the filesystem seam
description: Sandboxes implement one method — execute() — and inherit all file operations derived from it; interpreters complement them with in-process capability-scoped code execution and programmatic tool calling.
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# Sandboxed execution behind the filesystem seam

deepagents folds code execution into the same backend abstraction as file
access: a sandbox backend is "the only method a provider must implement is
`execute()`" — the base class derives every filesystem operation by running
scripts inside the sandbox, and the `execute` tool only appears in the
toolset when the backend supports it [1]. Providers (LangSmith, Daytona,
E2B, Modal, Runloop, Vercel, AgentCore) are thin `execute()` adapters;
files move in and out via out-of-band upload/download using provider-native
APIs [1].

Stated security posture worth adopting verbatim: **never put secrets inside
a sandbox** — a context-injected agent can read and exfiltrate them;
sandboxes protect the host, not against context injection; secrets belong in
host-side tools or credential-injecting proxies [1].

## Interpreters as the lightweight tier

`CodeInterpreterMiddleware` (a satellite package, `langchain_quickjs`) adds
an `eval` tool running JavaScript in in-process QuickJS — "capability-scoped,
not memory-isolated," with no fs/network/shell access by default
(64 MB / 5 s defaults) [1]. Its
distinctive feature is **Programmatic Tool Calling**: allowlisted tools are
exposed as async functions inside the interpreter, so the model can write a
loop that calls a tool 50 times without 50 model turns [1]. The docs'
decision rule: interpreter for in-memory transforms and orchestrated tool
calls; sandbox for shells, packages, and OS access [1].

## Why it matters for the refactor

The current runtime has a solid Docker sandbox — resource limits,
network-off default, and a notable *refusal-on-degraded* posture (exec tools
refuse rather than silently running unconfined when Docker is unavailable)
that deepagents has no equivalent of and that should survive the refactor
([current runtime](/perspectives/current-runtime.md)) [2]. But the sandbox
is a command-execution strategy only: file tools always hit the host
workspace, so agent file access cannot be redirected into the sandbox the
way a sandbox *backend* redirects everything at once [2][1]. There is no
interpreter tier and no programmatic tool calling [2]. Unifying sandbox
execution under the backend seam
([filesystem as context substrate](/practices/filesystem-as-context-substrate.md))
is the structural move; PTC is an optional later win.

# Citations

1. [deepagents documentation (docs.langchain.com)](/sources/deepagents-docs.md)
2. [rusty-agent — current Rust agent runtime](/sources/rusty-agent-runtime.md)
