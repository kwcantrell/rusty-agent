# Context Primer — React Frontend

**Status:** Not started. Context primer — run `brainstorming` before implementing.
**Attaches via:** consumes the Cloudflare control plane's WebSocket/API — never touches the local machine directly.
**Depends on:** the Cloudflare control plane primer (must exist first).

## What it is

The browser UI: a Claude Code–style chat experience hosted on Cloudflare Pages. It renders the agent's streamed work and lets the user steer it. It is a **pure client of the Worker** — it does **not** access the filesystem, execute commands, or run models. All of that happens on the local Rust daemon, reached only via the Worker.

## Where it fits

```
React (Cloudflare Pages) ──(WebSocket via Worker)── Durable Object ── Local daemon
```

The frontend subscribes to the same `AgentEvent` stream the core emits (relayed by the Worker) and renders each event type. It sends user messages and approval responses back through the socket. Because the daemon already produces structured events and `ToolOutput.display` payloads (diffs, terminal blocks), the UI gets rich visualization without the core doing anything frontend-specific.

## Key responsibilities (maps to `AgentEvent` types)

- **Chat interface** — conversation history, user input.
- **Streaming responses** — render `Token` events incrementally.
- **Tool execution visualization** — `ToolStart`/`ToolResult`; show tool name, args, status.
- **File diffs** — render `ToolOutput.display::Diff` for write/edit tools.
- **Terminal output** — render `Display::Terminal` for `execute_command`.
- **Permission prompts** — render `ApprovalRequest`; send back Approve / ApproveAlways / Deny over the `ApprovalChannel` round-trip.
- **Settings** — model/endpoint, policy/allowlist config, MCP servers, etc.
- **Errors / done states** — `Error`, `Done(StopReason)`.

## Proposed approach

- React + Vite + TypeScript; deploy to Cloudflare Pages.
- A typed WebSocket client matching the shared message envelope from the control-plane spec (generate/share types from the Rust event schema where practical).
- Component-per-event-type rendering; a diff viewer and an xterm-style terminal pane.
- Keep state minimal and stream-driven; the daemon is the source of truth.

## Open questions for brainstorming

- Component library / styling approach and overall layout.
- How much session history is replayed on reconnect (ties to control-plane resume semantics).
- Optimistic UI vs strictly server-driven rendering.
- Settings: which live in the browser vs pushed to the daemon.
- Auth/login UX (depends on the control-plane auth choice).

## Definition of done (high level)

A user opens the Pages app, authenticates, and drives a task on their local agent: sees streamed tokens, watches tool calls with diffs and terminal output, responds to permission prompts, and adjusts settings — all over the Worker, with the browser never touching local resources directly.
