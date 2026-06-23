# Deferred Subsystems — Context Primers

These documents are **context primers, not approved designs**. Each describes a subsystem that was intentionally deferred out of the first slice (the Rust agent core — see
[`../specs/2026-06-22-rust-agent-core-design.md`](../specs/2026-06-22-rust-agent-core-design.md)).

**Purpose:** give an agent starting a fresh session enough context to understand *what* the subsystem is, *where it bolts onto the core*, and *what questions remain* — so it can run its own `brainstorming` → spec → `writing-plans` → implementation cycle.

> **Before implementing any of these, run the `brainstorming` skill** to turn the primer into a full, approved design spec in `../specs/`. Do not skip straight to code — these primers deliberately leave design decisions open.

## The core's two extension seams

Everything here attaches to the agent core through interfaces that already exist by design (see core spec §4–5, §10):

- **`EventSink`** — the loop emits structured `AgentEvent`s (tokens, tool start/result, approval requests, errors, done). Anything that wants to *observe or stream* the agent taps this.
- **`ApprovalChannel`** — async approval requests. Anything that wants to *gate tool execution through a different UI* implements this.
- **`Tool` + `ToolRegistry`** — new capabilities register here.
- **`ContextManager`** — alternate context strategies (summarization, retrieval) implement this trait.
- **`intent()` → `PolicyEngine`/execution boundary** — where enforcement/sandboxing plugs in.

## Subsystems & recommended build order

After the agent core works end-to-end from the CLI, a sensible sequence (each is independent unless noted):

| # | Subsystem | Primer | Attaches via | Depends on |
|---|---|---|---|---|
| 1 | HTTP / browser tool | [`http-tool.md`](./http-tool.md) | `Tool` | core only (smallest; good warm-up) |
| 2 | OS-level sandboxing | [`os-sandboxing.md`](./os-sandboxing.md) | `intent()`/exec boundary | core only |
| 3 | MCP client support | [`mcp-client.md`](./mcp-client.md) | `Tool`/`ToolRegistry` | core only |
| 4 | Vector / long-term memory | [`memory-system.md`](./memory-system.md) | `ContextManager` + `Tool` | core only |
| 5 | Cloudflare control plane | [`cloudflare-control-plane.md`](./cloudflare-control-plane.md) | `EventSink` + WS `ApprovalChannel` | core + an API-server crate |
| 6 | React frontend | [`react-frontend.md`](./react-frontend.md) | (consumes #5's API) | Cloudflare control plane |

#5 and #6 are the path to the browser experience; #1–#4 deepen the local agent and can be done in any order.
