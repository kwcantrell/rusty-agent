# Tauri IPC Transport — Design (2026-06-25)

Supersedes security-audit finding **A2** (cluster **A-a**, unbounded event channel) by
**dissolving** it rather than patching it, and folds in **A1** (concurrent `user_input`
cross-talk) and the deferred **B3 interactive server-cancel**. The `RuntimeState`
three-mutex refactor (remainder of **A-c**) stays deferred to its own spec.

## Why this instead of "bound the channel"

The event channel, `WireEnvelope` protocol, `session_id` correlation, and reconnect
machinery are **cloud-control-plane architecture**: built when an untrusted browser talked
to a remote Worker over a network WebSocket. The app is now a **local Tauri desktop app** —
one trusted webview on loopback. In that model:

- The A2 "slow remote client exhausts memory" threat does not exist. The realistic failure
  is "our own renderer briefly stalls," not an adversary.
- A localhost `TcpListener` + tungstenite + a versioned wire envelope + per-connection
  session ids is overbuilt for in-process IPC between the shell and its own webview.

So rather than add a bounded channel + backpressure policy to a cloud-shaped design, we
replace the transport with native Tauri IPC. The unbounded `mpsc` event channel and its
writer task simply cease to exist, so A2 cannot recur.

## Scope

In scope:

- Replace the loopback WebSocket transport with Tauri IPC: typed `#[command]`s inbound, one
  long-lived `ipc::Channel<ServerEvent>` outbound.
- Move per-connection runtime state to app-lifetime session state.
- A1 fix: a single-active-run guard.
- B3 interactive cancel: a `cancel()` command tripping the existing `CancellationToken`.

Out of scope (deferred, unchanged):

- A-c's `RuntimeState` god-object / three-independent-mutex refactor. Internals are carried
  over verbatim; only its construction site and reply path change.
- Any change to `agent-core` loop logic, the `EventSink`/`ApprovalChannel` traits, the
  model layer, or tool execution.

## Architecture

### State model: per-connection → app-lifetime

Today each WebSocket connection runs a fresh `agent_server::daemon::serve()` that owns the
`RuntimeState`, the `WindowContext`, the `session` id, and the event channel. There is no
connection to anchor lifetime on with IPC, so this state becomes Tauri-managed app state,
constructed once in `bridge::start()` and shared across command invocations.

```rust
struct Session {
    runtime: Arc<RuntimeState>,                       // internals unchanged (A-c deferred)
    ctx: Arc<tokio::sync::Mutex<WindowContext>>,
    event_sink: Mutex<Option<Channel<ServerEvent>>>,  // registered by `subscribe`
    approvals: Arc<ApprovalRegistry>,                 // cN -> oneshot (was WsApprovalChannel.pending)
    active_run: Mutex<Option<RunHandle>>,             // run-guard + cancel token
    workspace: Mutex<PathBuf>,
}

struct RunHandle {
    cancel: tokio_util::sync::CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
```

The `Session` is held in the existing Tauri `AppState` (replacing the `Bridge` struct). It
is created at app setup, not per connection.

### Outbound: one event channel

The agent loop's `EventSink::emit` writes to the registered `ipc::Channel<ServerEvent>`.
`Channel::send` is synchronous and returns `Result`, which fits the infallible
`fn emit(&self, event)` contract: on a closed/absent channel we drop the event, exactly as
today's `let _ = self.tx.send(env)` does on a dead socket. There is no `mpsc`, no writer
task, and no unbounded queue — which is what removes A2 structurally.

`emit` maps `AgentEvent → ServerEvent` (the existing `wire_event_from` mapping, minus the
envelope wrapper). `AgentEvent::Approval` continues to return nothing from the sink; the
approval path emits its own `ServerEvent::ApprovalRequest` (see below), mirroring today.

The sink reads `Session.event_sink` **live on each emit** (it holds an `Arc<Session>` or an
`Arc<Mutex<Option<Channel>>>`, not a snapshot taken at run-start). So a re-`subscribe`
mid-run redirects subsequent events to the new channel, and an absent channel drops. This
keeps emit cheap and makes reconnect/strict-mode-remount transparent to an in-flight run.

### Inbound: typed commands

The daemon read loop is replaced by typed Tauri commands:

```rust
subscribe(channel: Channel<ServerEvent>)                       // register the outbound sink
send_input(text: String) -> Result<(), Busy>                   // run-guard
approve(id: String, decision: Decision)                        // resolve a pending approval
cancel()                                                       // trip the active run's token
settings_get() -> SettingsState                                // request/response
settings_update(settings: RuntimeConfig) -> Result<(), String> // request/response
```

`settings_get`/`settings_update` become true request/response: the `SettingsState` and
`SettingsError` frames disappear from the event stream entirely. The remaining inbound frame
kinds (`user_input`, `approval_response`) map one-to-one to `send_input`/`approve`.

### Outbound payload: `ServerEvent`

`ServerEvent` is today's `WireEvent` minus the `WireEnvelope` wrapper, plus the
approval-request case (which the old design carried as a sibling `WireBody`):

```
ServerEvent =
  | Token { text }
  | Reasoning { text }
  | Usage { prompt_tokens, context_limit, turn, max_turns }
  | ToolStart { name, args }
  | ToolResult { name, content, display? }
  | Error { message }
  | Done { reason }
  | ApprovalRequest { id, summary, command?, display? }
```

Deleted: `WireEnvelope`, `session_id`, `PROTOCOL_VERSION`, `WireBody`. Kept: the approval
correlation id (`cN`), now a field on `ApprovalRequest`/`approve`; `Decision`
(`approve` | `approve_always` | `deny`); the `DiscoveredSkill` / `SettingsState` shapes,
now returned from `settings_get` instead of wrapped in a frame.

## Lifecycle & control flow

**Subscribe handshake.** On mount, `App.tsx` (via `socket.ts`) creates one `Channel` and
calls `invoke("subscribe", { channel })`; the command stores it in `Session.event_sink`.
A re-subscribe (React strict-mode double-mount, reload) replaces it — last writer wins. If
no channel is registered when the loop emits, `emit` drops silently.

**Run-guard (A1).** `send_input` takes the `active_run` lock:

- `Some` → return `Err(Busy)`. The frontend disables the composer while a run is in flight
  (the reducer already tracks run state via `Done`/status).
- `None` → build the agent loop with a sink bound to the registered channel and an approval
  channel bound to `Session.approvals`, spawn the run, store a
  `RunHandle { cancel, task }`. The task clears `active_run` on completion.

This structurally eliminates the "second concurrent input misattributes frames" cross-talk:
there is never a second detached run sharing the stream.

**Cancel (B3).** `cancel()` trips `active_run`'s `CancellationToken` (the plumbing landed in
B3 / `run_with_cancel`). The loop emits `Done(Cancelled)`; the task clears `active_run`.
No-op when idle.

**Approval correlation.** Unchanged in spirit. The loop's `ApprovalChannel::request`
allocates `cN`, inserts a oneshot into `Session.approvals`, emits
`ServerEvent::ApprovalRequest { id, … }`, and awaits with the 300 s timeout → `Deny` on
timeout or absent channel. `approve(id, decision)` resolves the matching oneshot. The
`ApprovalRegistry` is the old `WsApprovalChannel.pending` map lifted into shared state.

**Workspace switch.** `pick_workspace` no longer aborts a socket. It cancels any active run,
resets `Session.ctx` to a fresh `WindowContext`, rebinds the workspace + reloads config on
`RuntimeState`, and returns. The webview stays connected and simply sees a fresh stream.

## File-by-file changes

Backend (`agent/crates/agent-server`, `src-tauri`):

- `bridge.rs` — delete `TcpListener`/`accept_async`/`ws_url`/reconnect loop. Becomes
  `Session` construction + workspace rebind. The `Bridge` struct is replaced by `Session`.
- `daemon.rs` — delete `serve()`; its read-loop dispatch becomes the command bodies, its
  writer task is removed.
- `sink.rs` — `WsEventSink` internals: `mpsc::UnboundedSender<WireEnvelope>` →
  `Channel<ServerEvent>`. `EventSink` trait impl otherwise unchanged.
- `approval.rs` — `WsApprovalChannel` keeps the oneshot/timeout logic and emits over the
  channel; `pending` moves into the shared `ApprovalRegistry`.
- `runtime.rs` — `send()` frame-replies become command return values. `RuntimeState`
  internals untouched (A-c deferred).
- `wire.rs` — delete `WireEnvelope`/`session_id`/`PROTOCOL_VERSION`/`WireBody`; keep/rename
  the payload enums as `ServerEvent` plus the settings/decision/discovered-skill types.
- `src-tauri/src/lib.rs` — register the new commands in `generate_handler!`; drop
  `get_local_ws_url`; `AppState` holds the `Session`.
- `setup.rs` — `local_params` carried over; wired into `Session` construction instead of
  `serve()`.

Frontend (`web/`):

- `socket.ts` — `connect()` keeps its signature `(handlers, opts) → { send, close }`; body
  swapped to `subscribe` a `Channel` (callback maps `ServerEvent → onFrame`) and `send(o)`
  switches on `o.kind` to the matching `invoke()`. `settings_get` is awaited and dispatched
  as an `onFrame` so the reducer path is unchanged.
- `transport.ts` — drop `wsUrl`; keep the local history id only.
- `App.tsx` — drop `localUrl` state + gating; `send`/`decide`/`openSettings`/`saveSettings`
  route through the same `sock` seam. `sessionId` survives only as a client-side
  localStorage history key — it never crosses to the backend.
- `wire.ts` — `Outbound` becomes the command-arg shapes; `Inbound`/`parseInbound` adapt to
  `ServerEvent`. `Inbound` variants the reducer consumes are otherwise stable.

## Error handling & edge cases

- **Channel absent on emit** → drop (parity with send-to-dead-socket today).
- **Second `send_input` during a run** → `Err(Busy)`; composer stays disabled client-side.
- **`approve` for an unknown/expired id** → no-op (the oneshot is already gone); the run's
  300 s timeout still resolves to `Deny`.
- **`cancel` while idle** → no-op.
- **Re-`subscribe`** → replaces the stored channel; the prior channel is dropped.
- **Workspace switch mid-run** → run is cancelled first, then context reset.

## Testing strategy

Rust:

- Unit-test command fns directly against a `Session` with a mock `Channel` sink, asserting
  the emitted `ServerEvent`s.
- Run-guard: a second `send_input` during an active run returns `Busy`.
- Cancel: `cancel()` yields a `Done(Cancelled)` and clears `active_run`.
- Approval round-trip: a run emits `ApprovalRequest`; `approve(id, …)` resolves the oneshot;
  the timeout path resolves to `Deny`.
- Replace the `serve_inbound.rs` / `tests/bridge.rs` / `e2e_live.rs` WebSocket integration
  tests with `tauri::test::MockRuntime` command/event tests.

TypeScript:

- Rewrite `socket.test.ts` / `transport.test.ts` to mock `@tauri-apps/api` `invoke` and
  `Channel` (replacing the `__WS__` injection seam).
- `App.tauri.test.tsx` adapts to the mocked IPC.
- Reducer/state tests (`state.test.ts`, `wire-display.test.ts`, `reasoning.test.ts`) are
  unaffected.

## Risks & mitigations

- **High-frequency token streaming over IPC.** Tauri `ipc::Channel` is the recommended path
  for streamed data (lower overhead than global `emit`/`listen`), so it is the chosen
  outbound mechanism. If profiling later shows overhead, batching tokens at the sink is a
  localized follow-up — not a transport change.
- **`tauri::test::MockRuntime` coverage gaps** for channel delivery. Where the mock runtime
  cannot assert channel payloads directly, test the sink against a trait-level mock channel
  and keep the command/event integration test focused on dispatch + state transitions.
- **Frontend regression surface.** Mitigated by preserving the `connect()` seam and the
  reducer/`Inbound` shapes, so churn concentrates in `socket.ts`/`transport.ts`.

## Backlog updates on completion

- A2 (A-a): resolved by dissolution — note the redesign, not a bounded channel.
- A1: resolved by the run-guard.
- B3 interactive server-cancel: resolved by `cancel()`.
- A-c: reduced to the `RuntimeState` three-mutex refactor only; `session_id` identity is
  gone as a side effect of this work.
