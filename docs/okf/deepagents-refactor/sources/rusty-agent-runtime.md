---
type: Source
title: rusty-agent — current Rust agent runtime
description: The repo's own agent/ Cargo workspace — the refactor target whose live source is ground truth for all "current state" claims.
resource: https://github.com/kwcantrell/rusty-agent
org: kwcantrell
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# Summary

The refactor target itself: the `agent/` Cargo workspace in this repository
(local path `agent/crates/`). Live source is the authority; the crate map
below is from `agent/AGENTS.md`.

# Key claims

- Twelve crates, one agent core exposed through three surfaces (CLI, Tauri desktop, browser SPA via a Cloudflare Worker)
- `agent-core` — agent loop, context manager, event model
- `agent-model` — model client, native/prompted tool-call protocols, inference types
- `agent-tools` — shared tool vocabulary and the `Tool` trait
- `agent-http` — outbound HTTP fetch tool, gates egress in-tool
- `agent-mcp` — MCP client for external MCP servers
- `agent-memory` — long-term semantic memory (remember/recall/forget over a local vector store)
- `agent-policy` — permission policy engine + approval channel
- `agent-sandbox` — sandboxed tool/command execution
- `agent-skills` — discover, load-on-demand, author, preload markdown skills
- `agent-server` — daemon bridging the local agent to the Cloudflare Worker
- `agent-cli` — terminal front-end binary
- `agent-runtime-config` — shared loop wiring (tool registry, protocol picker, command lists)
