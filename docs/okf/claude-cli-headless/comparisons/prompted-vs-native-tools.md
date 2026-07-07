---
type: Comparison
tags: [claude-cli]
---

# Prompted vs Native Tools

The Claude CLI's headless surface exposes two possible tool-use approaches:

**Native tool_use (MCP or CLI-native tools):** the CLI owns the inner loop —
it selects and executes tools, then returns the final result. This requires
either MCP bridging (the repo exposes its tools as an MCP server that the CLI
connects to) or full delegation (the CLI drives the agent loop directly). Both
paths have the same problem: the CLI holds the inner loop, so `agent-core`'s
context manager idles and accumulates no turn-by-turn visibility; the prompt
budget, policy engine, and sandbox approvals in `agent-policy` and
`agent-sandbox` are bypassed; and the backend becomes non-interchangeable (a
local-model backend cannot replicate the same tool surface, breaking backend
parity). The design alternatives and their rejection rationale are documented in
docs/superpowers/specs/2026-07-07-claude-cli-optimization-design.md.

**Prompted protocol (this repo's choice):** the CLI receives a full message
transcript on stdin and responds with a single assistant turn. Tool selection,
execution, policy approval, and context management all happen in `agent-core`'s
loop, outside the CLI. The CLI is a model inference subprocess only; the loop
in `agent-core` is authoritative. This preserves backend parity (the same loop
code drives local models and the Claude CLI), sandbox and policy enforcement,
and the context manager's turn-by-turn control [1].

# Citations

1. [headless-print-mode](/sources/headless-print-mode.md)
