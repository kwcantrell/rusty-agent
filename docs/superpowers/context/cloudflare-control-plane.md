# Context Primer — Cloudflare Control Plane

**Status:** Not started. Context primer — run `brainstorming` before implementing.
**Attaches via:** the local Rust agent's `EventSink` (stream out) + a WebSocket `ApprovalChannel` (gate in). Requires a new **API-server crate** in the agent workspace.
**Depends on:** agent core + an `agent-server` crate (Axum HTTP/WebSocket) that drives `AgentLoop`.

## What it is

The cloud control plane that lets a user reach their *local* agent from a browser anywhere. Cloudflare is frontend + control plane only — it **never** executes commands, touches the user's machine, runs the loop, or runs inference. It authenticates users, tracks which local agents are online, and relays messages between browser and the local daemon.

## Where it fits

```
Browser ── Cloudflare Pages ── Worker ──(WebSocket)── Local Rust daemon ── tools/model
                                  │
                          Durable Object (per agent/session)
                          D1 (users/agents/sessions)   R2 (logs/artifacts)
```

The local daemon needs a new **`agent-server`** crate (Axum) exposing an outbound WebSocket to the Worker. It implements `EventSink` by serializing `AgentEvent`s onto that socket, and a WebSocket-backed `ApprovalChannel` that round-trips approval prompts to the browser. **The agent core is unchanged** — this is purely additive at the two seams.

## Key responsibilities

- **Worker (API gateway):** auth, session management, routing a user to *their* agent, WebSocket relay, presence.
- **Durable Objects:** one per agent (and/or session) — hold the live WebSocket connection state, track active sessions, coordinate browser↔daemon message fan-out. DOs are the right primitive because connection state is inherently stateful and single-owner.
- **D1:** users, agents (registration + last-seen), sessions, metadata.
- **R2:** logs, artifacts, uploaded files.
- **Presence:** daemon registers + heartbeats; Worker/DO marks online/offline; browser sees agent availability.
- **Local daemon ↔ cloud:** the daemon dials *out* to the Worker (no inbound ports/NAT issues), authenticates as a registered agent, and maintains the WebSocket.

## Proposed approach

- Daemon → Worker: outbound persistent WebSocket (daemon is the client; avoids exposing the user's machine).
- One Durable Object per agent connection as the rendezvous point; browser WS and daemon WS both attach to it; it relays + buffers.
- Auth: pick a model (e.g. OAuth/email for users; per-agent tokens for daemons) during brainstorming.
- Wire protocol: a versioned JSON message envelope shared by daemon and Worker (mirror `AgentEvent` + control messages).

## Open questions for brainstorming

- Auth provider/model for users; pairing/enrollment flow for a new local agent.
- Message protocol + envelope versioning between daemon and Worker.
- Reconnect/resume semantics (does a session survive a daemon restart?).
- What actually persists in D1/R2 vs stays only in the live DO.
- Multi-tenant isolation (even if single-user first).

## Definition of done (high level)

A browser, authenticated through the Worker, exchanges messages with a specific local Rust daemon over WebSockets via a Durable Object; agent events stream to the browser and approval prompts round-trip back; presence reflects daemon online/offline; sessions/metadata persist in D1 and logs/artifacts in R2.
