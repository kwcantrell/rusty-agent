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
| 1 | HTTP / browser tool | _(spec/plan below)_ | `Tool` | ✅ **built & merged** |
| 2 | OS-level sandboxing | [`os-sandboxing.md`](./os-sandboxing.md) | `intent()`/exec boundary | deferred (top hardening priority) |
| 3 | MCP client support | _(spec/plan below)_ | `Tool`/`ToolRegistry` | ✅ **built & merged** |
| 4 | Vector / long-term memory | [`memory-system.md`](./memory-system.md) | `ContextManager` + `Tool` | deferred |
| 5 | Cloudflare control plane | _(spec/plan below)_ | `EventSink` + WS `ApprovalChannel` | ✅ **built & merged** |
| 6 | React frontend | _(spec/plan below)_ | consumes #5's API (same-origin) | ✅ **built & merged** |

**#1, #3, #5, and #6 are done** — the browser experience works end-to-end (validated live against the real model), and the http-tool (#1) + MCP client (#3) deepeners have shipped. What remains from the deferred list are **#2 (OS-level sandboxing — the top production-hardening priority)** and **#4 (vector / long-term memory)**. Several enhancements beyond the original list have also shipped (Settings, sampling/thinking controls, the skills subsystem + runtime-config persistence, the stream-timeout hardening, the Claude CLI backend — see the tables below).

### Completed subsystems (read these for current truth, not just the primers)

| # | Spec | Plan | Notes |
|---|---|---|---|
| 1 | [`http-tool-design`](../specs/2026-06-23-http-tool-design.md) | [plan](../plans/2026-06-23-http-tool.md) | New `agent-http` crate: read-only `fetch_url` web-fetch tool (attaches via `Tool`). Non-overridable SSRF floor + DNS-rebinding pin; GET-only. |
| 3 | [`mcp-client-support-design`](../specs/2026-06-23-mcp-client-support-design.md) | [plan](../plans/2026-06-23-mcp-client-support.md) | New `agent-mcp` crate: connect to MCP servers (stdio transport), surface their tools via `ToolRegistry`. Streamable-HTTP transport + resources/prompts deferred. |
| 5 | [`cloudflare-control-plane-design`](../specs/2026-06-22-cloudflare-control-plane-design.md) + [best-practices revision](../specs/2026-06-22-cloudflare-control-plane-bestpractices-revision-design.md) | [plan](../plans/2026-06-22-cloudflare-control-plane.md) + [revision plan](../plans/2026-06-22-cloudflare-control-plane-bestpractices-revision.md) | New `agent-server` (daemon, **WS client via `tokio-tungstenite` — not Axum**) + `agent-runtime-config` crates; `cloud/` Worker + `AgentSession` Durable Object (**WebSocket Hibernation API**, durable SQLite seq) + D1 + R2, under `wrangler dev`. |
| 6 | [`react-frontend-design`](../specs/2026-06-23-react-frontend-design.md) | [plan](../plans/2026-06-23-react-frontend.md) | New top-level `web/` React+Vite+TS+Tailwind SPA, **served same-origin by the Worker via Workers static assets** (not Cloudflare Pages → no CORS). Pure WS client; daemon/wire-protocol unchanged. |

> The context primers for completed subsystems have been removed now that each is captured in an approved spec + plan — **the spec/plan above is the source of truth** (e.g. where they differ: "Axum"→`tokio-tungstenite` client; "Cloudflare Pages"→Workers static assets). Only the primers for not-yet-built subsystems (#2 os-sandboxing, #4 memory-system) remain.

### Other shipped enhancements (not from the deferred list)

| Enhancement | Spec | Plan | Notes |
|---|---|---|---|
| Claude CLI inference backend | [`claude-cli-inference-backend-design`](../specs/2026-06-23-claude-cli-inference-backend-design.md) | [plan](../plans/2026-06-23-claude-cli-inference-backend.md) | New `ClaudeCliClient` in `agent-model` (attaches via the **`ModelClient`** seam — core untouched) drives an authenticated Claude Code CLI as a pure text generator (tools disabled; **prompted** protocol). Selectable via `--backend claude-cli` on `agent-cli` + `agent-server`; default `openai` unchanged. Spike findings + claude-cli follow-ups now live in [`follow-ups.md`](./follow-ups.md) → "claude-cli backend (standing)". |
| Agent-loop stream timeout | [`agent-loop-stream-timeout-design`](../specs/2026-06-23-agent-loop-stream-timeout-design.md) | [plan](../plans/2026-06-23-agent-loop-stream-timeout.md) | Per-turn idle (inter-chunk) stream timeout in `one_completion` → retryable `ModelError::Timeout`. Config `LoopConfig.stream_idle_timeout` (default 120s) / `--stream-timeout-secs`. Covers SGLang + claude-cli. |
| Settings capability | [`settings-capability-design`](../specs/2026-06-23-settings-capability-design.md) | [plan](../plans/2026-06-23-settings-capability.md) | Browser-driven live daemon reconfiguration — edit model/endpoint/policy from the SPA via a new inbound daemon-config channel (the "Settings" capability deferred out of #6). |
| Sampling & thinking settings | [`sampling-thinking-settings-design`](../specs/2026-06-23-sampling-thinking-settings-design.md) | [plan](../plans/2026-06-23-sampling-thinking-settings.md) | Seven inference controls end-to-end (5 optional sampling params + `enable_thinking` + `preserve_thinking`) + a distinct reasoning channel. Additive core touch (`agent-model`/`agent-core`), accepted per spec. |
| Skills subsystem | [`skills-subsystem-design`](../specs/2026-06-23-skills-subsystem-design.md) | [plan](../plans/2026-06-23-skills-subsystem.md) | New `agent-skills` crate: Claude-Code-style skill packages (discover, load-on-demand, author, presets), attaching only via the `Tool` seam + binary wiring. |
| Skills runtime-config persistence | [`skills-runtime-config-persistence-design`](../specs/2026-06-23-skills-runtime-config-persistence-design.md) | [plan](../plans/2026-06-23-skills-runtime-config-persistence.md) | Persist `skills_dirs`/`active_skills` into `RuntimeConfig` + full browser Settings round-trip (disk+wire, live-apply mid-session, discovered-skills picker). |
