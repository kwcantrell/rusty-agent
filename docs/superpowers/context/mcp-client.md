# Context Primer — MCP Client Support

**Status:** Not started. Context primer — run `brainstorming` before implementing.
**Attaches via:** `Tool` trait + `ToolRegistry` (MCP tools appear as ordinary tools).
**Depends on:** agent core only.

## What it is

A Model Context Protocol client so the runtime can consume tools/resources from external MCP servers (filesystem, GitHub, databases, arbitrary APIs) without us hand-writing each integration. Greatly expands capability via the MCP ecosystem.

## Where it fits

The core's `Tool` trait and `ToolRegistry` are the integration point. An MCP server exposes a list of tools with JSON schemas — each maps cleanly onto one dynamic `Tool` whose `execute()` forwards a JSON-RPC `tools/call` to the server and returns the result. The agent loop is unchanged: to it, MCP tools look identical to native ones.

## Key responsibilities

- **`McpClient`:** speak MCP (JSON-RPC 2.0) over the standard transports — stdio (spawn a server process) and Streamable HTTP/SSE.
- **Discovery:** on connect, call `tools/list`; wrap each result as a `McpTool: Tool`. Translate MCP input schemas → our `ToolSchema`.
- **Invocation:** `Tool::execute()` → `tools/call` → normalize the MCP result into `ToolOutput`.
- **`intent()` for MCP tools:** MCP doesn't describe side effects, so default these to a conservative "Ask" intent (or per-server config) — they're third-party code.
- **Config:** an `mcpServers` config (command/args/env for stdio; URL for HTTP), mirroring the familiar MCP config shape.
- Lifecycle: connect, health, reconnect, clean shutdown of spawned processes.

## Proposed approach

- Consider the official Rust SDK (`rmcp`) vs a thin hand-rolled JSON-RPC client; evaluate in brainstorming.
- Register a `McpToolProvider` that, at startup, connects configured servers and bulk-registers their tools into `ToolRegistry`.
- Namespacing: prefix tool names by server (e.g. `github__create_issue`) to avoid collisions.
- Treat MCP servers as **untrusted** by default — strong synergy with the OS-sandboxing primer for HTTP/WASM-isolated tool execution.

## Open questions for brainstorming

- `rmcp` SDK vs minimal custom client?
- Transports for v1 — stdio only, or stdio + HTTP?
- MCP **resources** and **prompts** too, or just **tools** first?
- Default trust/permission posture for third-party servers.
- Do we surface MCP server status through `EventSink` for UI later?

## Definition of done (high level)

The runtime connects to at least one real MCP server (e.g. the filesystem server) over stdio, its tools appear in `ToolRegistry`, the agent invokes them through the normal loop, and results flow back as `ToolOutput`. Tested against a mock MCP server with golden JSON-RPC fixtures.
