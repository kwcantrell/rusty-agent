# Design Spec — Cloudflare Control Plane

**Date:** 2026-06-22
**Status:** Approved design. Next step: `writing-plans` → implementation plan.
**Subsystem:** #5 in the deferred-subsystem build order (see [`../context/README.md`](../context/README.md)).
**Primer:** [`../context/cloudflare-control-plane.md`](../context/cloudflare-control-plane.md)
**Attaches to the core via:** `EventSink` (stream out) + a WebSocket-backed `ApprovalChannel` (gate in), plus a new `agent-server` crate. The agent core is **unmodified**.

## 1. Purpose & scope

The cloud control plane that lets a user reach their *local* agent from a browser. Cloudflare is **frontend + control plane only** — it never executes commands, runs the ReAct loop, or runs inference. It authenticates the user, tracks daemon presence, relays messages between browser and the local daemon, and records the event stream.

This spec covers subsystem #5 only. The polished **React frontend is subsystem #6** and is deliberately out of scope here; this slice is verified with `wscat` and a throwaway single-file HTML page.

### In scope (the verifiable end-state)

Full daemon→Worker outbound WebSocket; one Durable Object per agent relaying both directions and fanning out to attached browsers; presence; D1 (users/agents/sessions metadata) and R2 (append-only event log) persistence; approval round-trip with timeout→Deny; everything running under `wrangler dev`.

### Out of scope (deliberately deferred)

- Real React UI (subsystem #6).
- Real external OAuth/IdP (the auth boundary is built so this is a localized future swap — see §6).
- Loop resumption across a daemon restart (a daemon restart starts a new session; see §8).
- Queryable per-turn transcript schema in D1 (the R2 event log covers history/replay).
- Multi-tenant/multi-agent concurrency beyond a single active session per agent.

## 2. Constraints carried from the project

- Learning/portfolio, local-first. Optimize for a clean, well-architected vertical slice; the trait-decoupled architecture is the showcase.
- Keep the core untouched; attach only via the existing seams (`EventSink`, `ApprovalChannel`, `Tool`/`ToolRegistry`, `ContextManager`).
- Runs fully offline under `wrangler dev` (Miniflare-emulated DO/D1/R2). No external dependencies on the live path.
- The command allow/deny policy is a guardrail, not a sandbox; the workspace path guard is lexical. This subsystem changes none of that — it only adds a remote UI over the same approval boundary.

## 3. Topology

```
 throwaway test.html / wscat            your machine
        │  ws (browser frame)               │
        ▼                                    │
  Worker (TS)  ──routes──►  AgentSession DO ─┼─ ws (outbound) ── agent-server (Rust daemon)
        │                    (1 per agent)   │                        │ drives
   D1 (users/agents/sessions)  R2 (event log)│                        ▼
                                              │                   AgentLoop + tools + model
```

The **daemon dials out** to the Worker — it is a WebSocket *client*, never a server.

### Divergence from the primer (recorded deliberately)

The primer describes the new crate as "Axum HTTP/WebSocket." Because the daemon only ever dials *out* (the entire reason — no inbound ports, no NAT traversal), it needs **no inbound HTTP server**. `agent-server` is therefore built on `tokio-tungstenite` (WS client) + `reqwest` (one-shot enrollment POST) + `serde_json`, **not** Axum. This is a conscious simplification that reinforces the "no inbound ports" design goal.

## 4. Repo layout (two new homes; core untouched)

- `agent/crates/agent-server/` — new Rust crate, binary daemon. Reuses `agent-cli`'s loop-construction wiring (model client, `ToolCallProtocol`, `ToolRegistry`, `RulePolicy`, `LoopConfig`) verbatim, swapping only the two seams (`TerminalSink → WsEventSink`, `TerminalApproval → WsApprovalChannel`) and replacing the REPL with WS-driven input. Adds an `enroll` subcommand.
- `cloud/` — new top-level directory:
  - `wrangler.toml`
  - `src/worker.ts` — the Worker (routes + auth + routing to the DO)
  - the `AgentSession` Durable Object class
  - `schema.sql` — D1 migration
  - `testpage/index.html` — the throwaway verification client (stays plain/ugly; not a real UI)

## 5. The Rust daemon (`agent-server`)

The daemon constructs one `AgentLoop` (config, model client, tools, policy) exactly as `agent-cli` does today, then runs an outbound WebSocket client loop against the Worker. Three pieces carry the design:

### 5.1 `WsEventSink` — sync emit over an async socket

`EventSink::emit(&self, AgentEvent)` is **synchronous and fire-and-forget**. The implementation maps `AgentEvent → WireEvent` and pushes a `WireFrame` onto an `mpsc::UnboundedSender<WireFrame>`. A dedicated writer task drains that channel to the outbound socket. (Sync producer, async transport, decoupled by the channel.)

### 5.2 `WsApprovalChannel` — async round-trip with correlation + timeout

`ApprovalChannel::request(&self, req) -> ApprovalResponse` is **async request/response**. The implementation:

1. Generates a correlation id and inserts a `oneshot::Sender<ApprovalResponse>` into `pending: Mutex<HashMap<CorrelationId, oneshot::Sender<…>>>`.
2. Sends an `approval_request` frame (carrying the correlation id as the envelope `id`).
3. `await`s the `oneshot` with a **timeout**. On timeout (e.g. disconnect mid-approval) it resolves to **`Deny`** (safe default), recorded as an event.

The inbound read loop, on receiving an `approval_response`, looks up the correlation id and completes the matching `oneshot`.

### 5.3 Session manager

Maps `session_id → WindowContext`, serialized behind a per-session lock so concurrent `user_input` frames cannot race the `&mut dyn ContextManager` that `AgentLoop::run(ctx, user_input)` requires. **One active session per agent** (MVP); multiple browser tabs may attach to the same session and all receive the event fan-out.

## 6. Auth & enrollment (token + pairing, no external IdP)

Two distinct trust relationships, both validated against D1, neither requiring an external IdP — works fully offline under `wrangler dev`.

- **Daemon side (per-agent token):** `POST /enroll`, gated by a **bootstrap secret** (a `wrangler.toml` var in local dev), creates an `agents` row and mints a per-agent token. The daemon stores it in its config and presents it as `Bearer <agent-token>` on the `GET /agent` WS upgrade. The Worker validates against D1 and routes the socket to the agent's DO.
- **User/browser side (pairing → session token):** on startup the daemon prints a **pairing code** (random, stored in D1 against the agent). The browser `POST /pair`s the code, receives a session token bound to that `agent_id`, and presents it on the `GET /browser` WS upgrade. The Worker validates and routes to the same DO.

Because the user side terminates in a session token validated by the Worker, swapping in real OAuth later is a **localized change** (replace the issuer, not the relay).

## 7. The Cloudflare edge

### 7.1 Worker routes (TypeScript)

| Route | Purpose |
|---|---|
| `POST /enroll` | Daemon registration, gated by bootstrap secret. Creates `agents` row, mints + returns per-agent token. |
| `GET /agent` (WS upgrade) | Daemon connects with `Bearer <agent-token>`; Worker validates vs D1, routes socket to that agent's DO. |
| `POST /pair` | Browser exchanges pairing code for a session token bound to the `agent_id`. |
| `GET /browser` (WS upgrade) | Browser connects with session token; Worker validates, routes to the agent's DO. |

The Worker treats `event` payloads as **opaque** — it reads only routing fields (`kind`, `session_id`) plus auth/presence frames. No Rust type knowledge leaks into the edge.

### 7.2 `AgentSession` Durable Object — one per `agent_id`

Holds the live, **ephemeral** state: the daemon WS, the attached browser WS(s), the approval correlation map, the current-session pointer. Responsibilities:

- **Relay** frames in both directions.
- **Fan out** events to all attached browsers.
- **Append** each `event` frame to R2 (see §7.4).
- **Presence:** online while the daemon WS is attached, offline on its disconnect; broadcast a `presence` frame on change.
- Keep a small in-storage **ring buffer** of recent events for instant browser-reconnect replay without round-tripping R2.

The DO id is derived from `agent_id`, making it the natural single-owner rendezvous. DO state is rebuildable and may be lost on eviction (it is not the system of record).

### 7.3 D1 schema (`schema.sql`)

- `users` — minimal (single user for now): `id`, `created_at`.
- `agents` — `id, name, token_hash, user_id, pairing_code, last_seen, online, created_at`.
- `sessions` — `id, agent_id, token_hash, status, created_at`.

Tokens are stored hashed.

### 7.4 R2 event log (no append API)

R2 objects are immutable — there is **no append API**. The DO therefore writes **one object per event** at:

```
sessions/<session_id>/<seq:08>.json
```

with a monotonic `seq`. **Replay** = list the `sessions/<session_id>/` prefix, sort by `seq`. The DO's in-storage ring buffer (§7.2) serves fast reconnect replay; R2 is the durable history. Batching multiple events into fewer objects is a documented **future optimization**, not part of the MVP.

## 8. Reconnect / resume semantics

The ReAct loop and conversation context live **in the daemon** (in-memory `WindowContext`); the cloud only relays + records. Therefore:

- **Browser drops:** the daemon keeps running. On reconnect to the same session, the DO replays buffered/R2 events, then resumes live.
- **Daemon drops:** the DO flips presence offline and broadcasts it; the in-flight turn is lost. On daemon reconnect a **new** session starts; the old transcript remains read-only in R2.
- **Approval interrupted by a disconnect:** the daemon's pending `oneshot` times out → **`Deny`**, recorded as an event.

"Daemon restart starts a new session" is the correct consequence of the cloud not running the loop: the live state lived in the daemon and is genuinely gone. Full resume across a daemon restart (serializable/rehydratable context + resumable loop) is a separate future subsystem.

## 9. Wire protocol (mirror DTOs + versioned envelope, owned by `agent-server`)

None of the core types (`AgentEvent`, `ToolOutput`, `ToolIntent`, `Display`, `StopReason`, `ApprovalRequest`/`ApprovalResponse`) derive serde, and they stay that way. `agent-server` owns serde-derived wire DTOs and `From`/`Into` conversions to/from the core types. This is the anti-corruption layer at the network seam: the browser-facing contract is defined deliberately and versioned independently of how tools represent themselves internally.

Envelope:

```json
{ "v": 1, "session_id": "...", "id": "<correlation>", "kind": "...", "payload": { } }
```

| Direction | `kind` | `payload` |
|---|---|---|
| daemon → browser | `event` | `WireEvent` (mirror of `AgentEvent`: `token` / `tool_start` / `tool_result` / `error` / `done`) |
| daemon → browser | `approval_request` | mirrored request; `id` is the correlation key |
| daemon → browser | `presence` | `{ "online": bool }` |
| browser → daemon | `user_input` | `{ "text": "..." }` |
| browser → daemon | `approval_response` | `{ "decision": "approve" \| "approve_always" \| "deny" }`; `id` echoes the request |

Auth tokens travel on the WS upgrade (header/query), never in-band. `v` versions the envelope.

## 10. Error handling

- **Daemon ↔ Worker WS lost:** daemon reconnects with backoff; presence offline during the gap.
- **Malformed / unknown frame:** log and drop; the DO sends an `error` frame to the browser where relevant.
- **Invalid/expired token on upgrade:** Worker rejects (401), no DO involvement.
- **Model/tool errors:** already surfaced by the core as `AgentEvent::Error` → relayed as an `event`. No new error path is added to the core.

## 11. Testing strategy

### 11.1 Rust (hermetic — no wrangler needed)

- **Unit:** `AgentEvent → WireEvent` mapping; envelope serde round-trip; `WsApprovalChannel` correlation + timeout→Deny.
- **Integration:** spin a **fake WS server** (tokio-tungstenite) standing in for the Worker; drive a scripted `AgentLoop` reusing `agent-core`'s existing `testkit` (`ScriptedModel`/`CollectingSink` patterns); assert the frames on the wire plus a full approval round-trip.

### 11.2 Worker (Miniflare)

A handful of `@cloudflare/vitest-pool-workers` tests: relay both directions, auth rejection, presence flip, R2 append + prefix-replay.

### 11.3 End-to-end manual (`wrangler dev`)

`wrangler dev` + daemon (`enroll`, connect) + the throwaway HTML page (or `wscat`):

1. Send a prompt; watch tokens stream into the browser.
2. Trigger an `execute_command` tool → approval prompt appears in the browser → approve → command runs **in the daemon** → result streams back.
3. Kill the browser, reopen → confirm replay.
4. Inspect D1 rows and R2 objects.

## 12. Definition of done

A browser, authenticated through the Worker (pairing → session token), exchanges messages with a specific local Rust daemon over WebSockets via the `AgentSession` Durable Object — all under `wrangler dev`. Agent events stream to the browser; approval prompts round-trip back (timeout→Deny on disconnect); presence reflects daemon online/offline; session metadata persists in D1; the event log persists in R2 with prefix-based replay. The agent core is unmodified — everything attaches at `EventSink`, `ApprovalChannel`, and the new `agent-server` crate. Verified by the hermetic Rust tests (§11.1), the Miniflare Worker tests (§11.2), and the manual end-to-end script (§11.3).
