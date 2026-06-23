# Design Spec — Cloudflare Control Plane: Best-Practices Revision

**Date:** 2026-06-22
**Status:** Approved design. Next step: `writing-plans` → implementation plan.
**Revises:** the merged `cloud/` control plane (see [`2026-06-22-cloudflare-control-plane-design.md`](./2026-06-22-cloudflare-control-plane-design.md) and its plan).
**Trigger:** the Cloudflare plugin (skills + docs MCP) is now available; the existing `cloud/` code predates that official guidance.

## 1. Purpose & scope

Bring the already-built, already-merged `cloud/` Cloudflare control plane up to current Cloudflare best practices and the latest tooling, **without changing externally observable behavior**. Two halves:

1. **Tooling/config modernization** — latest `wrangler`, latest `@cloudflare/vitest-pool-workers`, `wrangler.jsonc`, real secret handling, observability, a supported `compatibility_date`.
2. **Durable Object hibernation re-architecture** — rewrite `AgentSession` to use the WebSocket **Hibernation API** with hibernation-safe state, the single best-practice gap that changes runtime behavior and cost.

All changes are confined to `cloud/` and validated under `wrangler dev` + Miniflare, exactly as today.

### Invariant (the constraint that bounds this work)

**Behavior-preserving on the wire.** The Rust `agent-server` daemon, the wire protocol (`{ v, session_id, id?, kind, ... }` with event `payload: { type, ... }`), and the throwaway test page semantics are **unchanged**. The daemon must connect to and behave identically against the revised Worker. Zero Rust changes.

### Out of scope (deliberately)

- Any Rust / `agent-server` / wire-protocol change.
- New features: multi-session-per-agent, real OAuth/IdP, RPC-ifying the Worker, R2 artifact uploads. Single-active-session-per-agent still holds.
- Re-opening the persistence design: R2 remains the event log, D1 remains users/agents/sessions (see §4).

## 2. Authoritative references (retrieved from Cloudflare docs, not memory)

- **Hibernation API:** `this.ctx.acceptWebSocket(server, [tags])`; handlers `webSocketMessage(ws, msg)`, `webSocketClose(ws, code, reason, wasClean)`, `webSocketError(ws, err)`; per-connection state via `server.serializeAttachment(obj)` / `ws.deserializeAttachment()`; live-socket enumeration via `this.ctx.getWebSockets(tag?)`. The runtime auto-responds to protocol pings **without waking** from hibernation.
- **Close handshake:** with `web_socket_auto_reply_to_close` (default on `compatibility_date ≥ 2026-04-07`), the runtime completes the close handshake; a manual `ws.close()` in `webSocketClose` is no longer required.
- **DO base class:** `export class X extends DurableObject<Env>` from `cloudflare:workers`, `super(ctx, env)`, use `this.ctx`.
- **SQLite storage:** `this.ctx.storage.sql.exec(sql, ...params)` → cursor with `.toArray()` / `.one()`; available because the class is `new_sqlite_classes` (already configured). GA, 10GB/DO.
- **Latest tooling at design time:** `wrangler 4.103`, `@cloudflare/vitest-pool-workers 0.16.18` (current pins are `^3.90` / `0.5.41`).

## 3. Tooling & config modernization

- **`wrangler ^3.90` → `^4`.** Config-compatible; ships a current workerd.
- **`@cloudflare/vitest-pool-workers 0.5.41` → `^0.16`** (and `vitest` to the major its peer range requires). The 0.16 line post-dates the `.sqlite-wal` isolated-storage bug, so the expectation is to **delete the `patch-package` machinery entirely** — the committed `cloud/patches/` file, the `postinstall` script, and the `patch-package` / `postinstall-postinstall` devDependencies. This is **verified, not assumed**, by a clean `rm -rf node_modules && npm install && npm test`. The patch is retained **only if** 0.16 still exhibits the bug.
- **`wrangler.toml` → `wrangler.jsonc`** (Cloudflare's recommended format). Same bindings: DO `AGENT` (`class_name: "AgentSession"`), D1 `DB`, R2 `LOGS`; `migrations` with `new_sqlite_classes: ["AgentSession"]` (already correct — keep the same tag).
- **`compatibility_date` → a supported date `≥ 2026-04-07`** (today is 2026-06-22), enabling `web_socket_auto_reply_to_close`. Also resolves the prior "date exceeds installed workerd" warning under wrangler 4.
- **Secret hygiene:** `BOOTSTRAP_SECRET` moves out of plaintext `[vars]` into a real secret — a gitignored `.dev.vars` for local dev plus documented `wrangler secret put` for deploy. The Worker reads `env.BOOTSTRAP_SECRET` identically; the only code-adjacent change is removing the `[vars]` entry. `.dev.vars` is added to `cloud/.gitignore`.
- **Observability:** add `"observability": { "enabled": true }` and replace the DO's ad-hoc `console.*` calls with consistent structured logging at lifecycle points (attach, close, presence change, persist failure).

## 4. `AgentSession` Durable Object hibernation rewrite (Approach 1)

`export class AgentSession extends DurableObject<Env>` (from `cloudflare:workers`), `super(ctx, env)`. The constructor ensures the SQLite `seq` table exists (`CREATE TABLE IF NOT EXISTS`).

### Accepting connections (`fetch`, still routed by the Worker with `X-Role` / `X-Session-Id` / `X-Agent-Id` headers)

- `this.ctx.acceptWebSocket(server, [role])` — tag `"agent"` or `"browser"`.
- `server.serializeAttachment({ role, sessionId, agentId })` — survives hibernation; replaces in-memory role/session/agent tracking.
- On **agent** attach: mark D1 `online`, broadcast `presence { online: true }`.
- On **browser** attach: replay the session's event log **from R2** (the existing paginated `replayFromR2`), then send a `presence` frame reflecting whether an agent socket currently exists.

### Message handling (hibernation handlers replace the `addEventListener` closures)

- `webSocketMessage(ws, msg)`: read `ws.deserializeAttachment()` for the role.
  - `role === "agent"` → fan out to `this.ctx.getWebSockets("browser")`; if the frame is an `event`, write it to R2 keyed by the **durable seq** (and increment the SQLite seq row).
  - `role === "browser"` → forward to `this.ctx.getWebSockets("agent")[0]` (the daemon), if present.
- `webSocketClose(ws, code, reason, wasClean)`: if the closing socket is the agent (its attachment role is `agent` and no other `agent` socket remains) → mark D1 `offline`, broadcast `presence { online: false }`. Browsers need no bookkeeping (`getWebSockets` simply stops returning them). No manual `ws.close()` (compat-date auto-reply).
- `webSocketError(ws, err)`: structured-log it.

### State mapping (in-memory → hibernation-safe)

| Old (in-memory) | New |
|---|---|
| `this.daemon: WebSocket \| null` | `getWebSockets("agent")[0]` |
| `this.browsers: Set<WebSocket>` | `getWebSockets("browser")` |
| `this.agentId`, role, sessionId | per-socket `serializeAttachment` / `deserializeAttachment` |
| `this.seq` | one-row DO **SQLite** table (durable across hibernation) |
| `this.buffer: Map` | **dropped** — browser-reconnect replay reads from **R2** |

### Why minimal SQLite

The only state that *must* be durable across hibernation/eviction is the monotonic event `seq` — otherwise a woken DO would reset to 0 and overwrite R2 keys. That single concrete need justifies a one-row SQLite table. The richer "mirror every event into a local SQLite replay buffer" option (Approach 2) was rejected as over-engineering for a single-user portfolio slice; dropping R2 entirely (Approach 3) was rejected for reopening the settled persistence design and leaving the DoD's R2 requirement unmet.

### Heartbeat compatibility

The daemon's periodic WS ping keeps the connection alive; the runtime auto-responds to pings **without waking** the DO, so hibernation is fully compatible with the unchanged daemon. `user_input` and approval round-trips are identical on the wire — only the DO's relay mechanism underneath changes.

## 5. Testing

- **Regression-first:** all existing Miniflare tests (util, enroll/pair, wrong-secret, relay, R2-replay) stay green after the toolchain bump. Major-bump API drift is fixed mechanically; assertions don't change. The clean `rm -rf node_modules && npm install && npm test` is the gate that also confirms whether `patch-package` can be deleted.
- **Hibernation handler coverage:** relay/presence tests now exercise the `webSocketMessage` / `webSocketClose` path (sockets accepted via `acceptWebSocket`). Use `cloudflare:test`'s `runInDurableObject` to assert DO-internal state — that the **seq** persists in SQLite and R2 keys stay monotonic across multiple events.
- **Reconstruct-from-attachment path:** a test that connects agent + browser, sends an event, and asserts fan-out works purely through `getWebSockets` / `deserializeAttachment` (no in-memory Set) — proving the hibernation-safe derivation.
- **Presence via close handler:** agent socket close → D1 `offline` + `presence { online: false }` broadcast.
- **Behavior-preserving cross-check (manual, deferred to the human):** the Rust `daemon_roundtrip` integration test is untouched (it uses a fake Worker), so the real proof that the daemon still works against the *revised* Worker is the manual E2E in `RUNNING.md`, updated for the wrangler-4 / `.dev.vars` flow.

## 6. Cutover & risk

- **No new DO migration:** the class keeps its name and `new_sqlite_classes` tag; only the implementation changes. The SQLite `seq` table is created lazily in the constructor. R2/D1 schemas untouched → the original spec's Definition of Done still holds.
- **Primary risk:** the `vitest-pool-workers` 0.5→0.16 major jump (test-API drift). Mitigation: bump and fix mechanically; if 0.16 is hard-incompatible, fall back to the newest version that runs our tests, retaining `patch-package` only if the WAL bug persists. Expect iteration here.
- **Secondary:** `wrangler.jsonc` conversion and the `compatibility_date` bump — both low-risk and mechanically verifiable under `wrangler dev`.

## 7. Definition of done

1. `wrangler ^4`, `@cloudflare/vitest-pool-workers ^0.16`, `wrangler.jsonc`, `compatibility_date ≥ 2026-04-07`, `BOOTSTRAP_SECRET` via `.dev.vars`/secret (gitignored), `observability` enabled.
2. `AgentSession extends DurableObject<Env>` using the Hibernation API (`acceptWebSocket` + `webSocketMessage`/`Close`/`Error` + `serializeAttachment`/`getWebSockets`), durable `seq` in DO SQLite, R2 still the event log, D1 presence updated via the close handler.
3. All Miniflare tests green from a **clean** `npm install`; `patch-package` machinery removed unless still required.
4. Rust daemon + test page unchanged; manual E2E verified; `RUNNING.md` updated for the wrangler-4 / `.dev.vars` flow.
5. Behavior on the wire is identical to the pre-revision control plane.
