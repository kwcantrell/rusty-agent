# MCP Client Support — Design Spec

**Date:** 2026-06-23
**Status:** Approved design — ready for `writing-plans`.
**Subsystem:** #3 of the deferred local-deepeners (see `docs/superpowers/context/README.md`).
**Attaches via:** `Tool` trait + `ToolRegistry`. **Depends on:** agent core (`agent-tools`) only.
**Primer:** `docs/superpowers/context/mcp-client.md`.

## 1. Summary

Add a Model Context Protocol (MCP) client so the runtime can consume **tools** from
external MCP servers (filesystem, git, github, databases, arbitrary APIs) without
hand-writing each integration. An MCP server exposes a list of tools with JSON schemas;
each maps cleanly onto one dynamic `Tool` whose `execute()` forwards a JSON-RPC
`tools/call` to the server and returns the result. To the agent loop, MCP tools are
indistinguishable from native ones.

This subsystem holds the project's showcase bar: it attaches entirely through the existing
`Tool`/`ToolRegistry` seam and changes **zero lines** of the four core crates
(`agent-core`, `agent-model`, `agent-tools`, `agent-policy`), exactly as #5 and #6 did.

## 2. Scope

### In scope (v1)

- A hand-rolled JSON-RPC 2.0 MCP client over the **stdio** transport (spawn a local
  server process; newline-delimited JSON over its stdin/stdout).
- The `initialize` → `notifications/initialized` → `tools/list` handshake on connect.
- Wrapping each discovered tool as a dynamic `Tool` registered into `ToolRegistry`,
  namespaced `server__tool`.
- `tools/call` invocation with result normalization into `ToolOutput` (and `ToolError`
  on `isError`).
- Conservative-by-default trust: every MCP tool requires approval (`Ask`) unless its
  server is explicitly marked trusted (`allow`), implemented with **no policy change**.
- Graceful degradation: a server that fails to connect is skipped; the agent starts with
  whatever connected.
- A `mcp.json` config file in the familiar `{ "mcpServers": { ... } }` shape, opt-in via
  an `--mcp-config <path>` flag on both binaries.
- Per-server status surfaced via `tracing` logs + a one-line CLI startup summary.

### Non-goals (named deferred follow-ups)

- **Streamable HTTP / remote MCP servers** (and their auth/OAuth, SSE parsing, reconnect).
  The `McpTransport` trait is designed so this is additive and never touches `McpClient`.
- **MCP resources** (`resources/list`, `resources/read`) and **prompts** (`prompts/list`,
  `prompts/get`) — the core has no seam for these; surfacing them means inventing a new
  concept with no v1 consumer.
- **Browser-side MCP management / Settings-wire integration** — `mcp.json` is deliberately
  kept off the wire-mirrored `RuntimeConfig` (letting a browser edit which local processes
  get spawned is a security smell).
- **`EventSink`/UI server-status events** — would require adding an `AgentEvent` variant,
  a core change with no UI consumer yet. Deferred until there is one.

## 3. Architecture

### 3.1 New crate: `agent-mcp`

A new workspace crate, isolated and independently testable (mirroring how `agent-server`
and `agent-runtime-config` were added). It depends only on `agent-tools` (for the `Tool`
trait + `ToolSchema`/`ToolOutput`/`ToolIntent`/`ToolError`/`ToolCtx` types) plus
`tokio`, `serde`/`serde_json`, `async-trait`, `thiserror`, `tracing`. It does **not**
depend on `agent-core`, `agent-model`, or `agent-policy`.

Modules:

| Module | Responsibility |
|---|---|
| `config` | Parse `mcp.json` → `McpServersConfig` (`name → { command, args, env, trust }`). |
| `transport` | `McpTransport` trait + `StdioTransport`. The single seam a future HTTP transport slots into. |
| `client` | `McpClient`: owns the child process + background reader task; correlates JSON-RPC requests/responses by `id`; drives the handshake and `tools/list`/`tools/call`. |
| `tool` | `McpTool: Tool` — the dynamic wrapper registered into `ToolRegistry`. |
| `manager` | `McpManager`: connect all servers concurrently (each under a timeout), own all `McpClient`s for lifetime/shutdown, return the `McpTool`s + a status summary. |

### 3.2 Transport

```rust
#[async_trait]
trait McpTransport: Send {
    async fn send(&mut self, msg: serde_json::Value) -> Result<(), McpError>;
    async fn recv(&mut self) -> Result<serde_json::Value, McpError>; // one JSON-RPC message
}
```

`StdioTransport` spawns the configured `command`/`args`/`env`, writes newline-delimited
JSON to the child's stdin, and reads newline-delimited JSON from its stdout (stderr is
drained to `tracing` for diagnostics). This is the only transport in v1; the trait exists
because Streamable HTTP is a concrete near-term follow-up, not speculative.

### 3.3 Client (actor pattern)

`McpClient` owns the child and a background task that reads messages off the transport and
dispatches each response to the waiter registered for its JSON-RPC `id` (a map of
`id → oneshot::Sender`). A `request(method, params)` helper allocates an id, registers the
oneshot, sends over an `mpsc` to the writer half, and awaits the response. Notifications
(no `id`, e.g. `notifications/initialized`) are fire-and-forget.

Connect sequence:
1. `initialize` (send client `protocolVersion`, `clientInfo`, `capabilities`); receive the
   server's capabilities.
2. `notifications/initialized`.
3. `tools/list` → the raw tool descriptors.

### 3.4 The `McpTool` wrapper (the seam)

For each discovered tool, an `McpTool` holds a handle to its `McpClient`, the server name,
the original tool name, the precomputed namespaced name, the translated `ToolSchema`, and
the server's trust level. Mapping onto the `Tool` trait:

- **`name()`** → `server__tool`. Namespaced with a `__` separator and sanitized to the
  model-tool-name charset (`[a-zA-Z0-9_-]`) to avoid collisions across servers and with
  native tools.
- **`description()`** → the MCP tool's `description`.
- **`schema()`** → the MCP `inputSchema` *is* JSON Schema, so it drops directly into
  `ToolSchema.parameters` (with `name`/`description` set to the namespaced values).
- **`intent()`** → **trust lives here, with zero policy change** (see §4).
- **`execute()`** → forwards `tools/call` (with the namespaced name stripped back to the
  server-local name) via the client, honoring `ToolCtx.timeout` (wrap the request await in
  `tokio::time::timeout`) and `ToolCtx.cancel`. Normalizes the MCP result:
  - join `content[]` text parts into `ToolOutput.content`; set `display: Some(Display::Text(..))`.
  - `isError: true` → `ToolError::Failed { message, .. }`.
  - non-text content parts (images, etc.) are summarized as a placeholder line in v1.

### 3.5 Manager & lifecycle

`McpManager::connect(config, connect_timeout)`:
- connects all configured servers **concurrently**, each under `connect_timeout`;
- a server that fails to spawn or complete the handshake is **skipped** with a `tracing`
  warning — it never blocks the agent;
- returns `(Vec<Arc<dyn Tool>>, Vec<ServerStatus>)` where `ServerStatus` records
  name / connected? / tool count / error.

The `McpManager` owns every `McpClient` (and thus every child process) for the agent's
lifetime. Dropping it (or calling `shutdown()`) terminates the spawned processes cleanly.
The binaries must hold the `McpManager` alive for the whole session.

## 4. Trust & policy integration (zero core change)

The existing `RulePolicy::check` (in the untouched `agent-policy`) decides from a
`ToolIntent`'s `command`, `access`, and `paths` only. We encode per-server trust onto the
axis the policy already understands:

- **`trust: "ask"` (default)** → `intent()` returns `access: Write, command: None, paths: []`.
  `RulePolicy` falls through to `Access::Write => Decision::Ask`. Approval required per call.
- **`trust: "allow"`** → `intent()` returns `access: Read, command: None, paths: []`.
  `RulePolicy`'s read branch checks `paths.iter().all(inside_workspace)`, which is
  **vacuously true for an empty path list**, yielding `Decision::Allow`. Auto-allowed.

Neither path sets `command`, so an MCP tool can never hit the denylist/allowlist branch or
`Decision::Deny`. This Read/Write-axis encoding is a deliberate, documented mapping — the
only zero-core-change lever, since `intent()` is the sole policy input we control.

The `summary` field carries a human-readable description of the MCP call for the approval
UI. MCP `readOnlyHint` (and other annotations) are **advisory display only** — shown in the
approval summary, never used to auto-allow (annotations are server-self-reported and
optional; trusting them would weaken the guardrail).

## 5. Configuration

`mcp.json` mirrors the familiar MCP config shape so existing blocks copy-paste:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/workspace"],
      "env": { "FOO": "bar" },
      "trust": "ask"
    }
  }
}
```

- `trust` is optional and defaults to `"ask"`.
- The file is opt-in via `--mcp-config <path>`. If the flag is absent, MCP is simply not
  enabled (no servers, no behavior change). A malformed file warns and disables MCP rather
  than aborting the agent.
- It is deliberately **separate** from `RuntimeConfig` and not mirrored on the Settings
  wire (see §2 non-goals).

## 6. Wiring (the only changes to existing code)

- **`agent-runtime-config`** gains an **async** companion to `build_registry()`, e.g.
  `async fn connect_mcp(registry: &mut ToolRegistry, mcp_config_path: &Path) -> McpManager`,
  which connects servers and bulk-registers their `McpTool`s. `build_registry()` stays sync
  and unchanged; native tools register exactly as before.
- **`agent-cli`** and **`agent-server`** add an `--mcp-config <path>` flag, call
  `connect_mcp` after building the native registry, hold the returned `McpManager` for the
  process lifetime, and (CLI) print the one-line status summary.

No changes to `agent-core`, `agent-model`, `agent-tools`, or `agent-policy`.

## 7. Failure, lifecycle & observability

- **Connect failures** degrade gracefully (§3.5): skip + warn, start with what connected.
- **Status summary** (CLI): one line, e.g.
  `mcp: filesystem ✓ (3 tools), github ✗ (timeout)`.
- **Per-call** behavior reuses `ToolCtx.timeout` and `ToolCtx.cancel`; a server that dies
  mid-session surfaces as a `ToolError::Failed` from `execute()`.
- **Shutdown** terminates all spawned children via the `McpManager`.

## 8. Testing

- **Unit / golden (hermetic):** an in-memory `McpTransport` that replays canned JSON-RPC
  fixtures. Cover: the `initialize`/`initialized`/`tools/list` handshake; schema
  translation; name namespacing + charset sanitization; `intent()` mapping (`ask` → `Ask`,
  `allow` → `Allow`) verified against the real `RulePolicy`; `tools/call` success and
  `isError`; result normalization (text join, non-text placeholder); malformed-response
  handling; request/response id correlation.
- **Live (Definition of Done, `#[ignore]`-gated):** connect to the real official
  **filesystem** MCP server over stdio (`npx -y @modelcontextprotocol/server-filesystem`),
  confirm its tools register into `ToolRegistry`, and invoke one through the normal agent
  loop end-to-end.
- **Bar:** `cargo test --workspace` green; `cargo clippy --all-targets -- -D warnings`
  clean. New crate added to the workspace members.

## 9. Definition of done

The runtime connects to at least one real MCP server (the filesystem server) over stdio,
its tools appear in `ToolRegistry` namespaced `server__tool`, the agent invokes them
through the normal ReAct loop (gated by the policy/approval seam per the trust setting),
and results flow back as `ToolOutput`. The four core crates are unchanged. Hermetic
golden-fixture tests cover the protocol surface; an `#[ignore]`-gated live test covers the
real server.

## 10. Open follow-ups recorded for later

Tracked in `docs/superpowers/context/follow-ups.md` at branch finish:

- Streamable HTTP / remote MCP transport (+ auth).
- MCP resources and prompts.
- `EventSink`/UI MCP server-status events (deliberate future core touch).
- Browser-side MCP server management via a new inbound config channel (pairs with the
  deferred Settings capability).
- OS-sandboxed execution of third-party MCP server processes (strong synergy with the
  os-sandboxing primer — MCP servers are untrusted code).
