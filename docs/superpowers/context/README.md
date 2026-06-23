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

| # | Subsystem | Primer | Attaches via | Status |
|---|---|---|---|---|
| 1 | HTTP / browser tool | [`http-tool.md`](./http-tool.md) | `Tool` | deferred (smallest; good warm-up) |
| 2 | OS-level sandboxing | [`os-sandboxing.md`](./os-sandboxing.md) | `intent()`/exec boundary | deferred |
| 3 | MCP client support | [`mcp-client.md`](./mcp-client.md) | `Tool`/`ToolRegistry` | deferred |
| 4 | Vector / long-term memory | [`memory-system.md`](./memory-system.md) | `ContextManager` + `Tool` | deferred |
| 5 | Cloudflare control plane | [`cloudflare-control-plane.md`](./cloudflare-control-plane.md) | `EventSink` + WS `ApprovalChannel` | ✅ **built & merged** |
| 6 | React frontend | [`react-frontend.md`](./react-frontend.md) | consumes #5's API (same-origin) | ✅ **built & merged** |

**#5 and #6 are done** — the browser experience works end-to-end (validated live against the real model). What remains are the independent local deepeners **#1–#4**, doable in any order; **#1 (http-tool)** is the smallest warm-up.

### Completed subsystems (read these for current truth, not just the primers)

| # | Spec | Plan | Notes |
|---|---|---|---|
| 5 | [`cloudflare-control-plane-design`](../specs/2026-06-22-cloudflare-control-plane-design.md) + [best-practices revision](../specs/2026-06-22-cloudflare-control-plane-bestpractices-revision-design.md) | [plan](../plans/2026-06-22-cloudflare-control-plane.md) + [revision plan](../plans/2026-06-22-cloudflare-control-plane-bestpractices-revision.md) | New `agent-server` (daemon, **WS client via `tokio-tungstenite` — not Axum**) + `agent-runtime-config` crates; `cloud/` Worker + `AgentSession` Durable Object (**WebSocket Hibernation API**, durable SQLite seq) + D1 + R2, under `wrangler dev`. |
| 6 | [`react-frontend-design`](../specs/2026-06-23-react-frontend-design.md) | [plan](../plans/2026-06-23-react-frontend.md) | New top-level `web/` React+Vite+TS+Tailwind SPA, **served same-origin by the Worker via Workers static assets** (not Cloudflare Pages → no CORS). Pure WS client; daemon/wire-protocol unchanged. |

> The primers for #5 and #6 below are kept as historical context, but they predate the build — where a primer and the shipped design differ (e.g. "Axum"→`tokio-tungstenite` client; "Cloudflare Pages"→Workers static assets), the **spec/plan above is the source of truth**.
