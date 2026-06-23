# Design Spec — React Frontend (Subsystem #6)

**Date:** 2026-06-23
**Status:** Approved design. Next step: `writing-plans` → implementation plan.
**Primer:** [`../context/react-frontend.md`](../context/react-frontend.md)
**Consumes:** the merged Cloudflare control plane (`cloud/`) over its existing WebSocket/HTTP contract. **Depends on** that subsystem being live.

## 1. Purpose & scope

A Claude Code–style browser chat UI that renders the local agent's streamed work and lets the user steer it. It is a **pure client of the Worker** — it never accesses the filesystem, runs commands, or runs models; all of that stays on the local Rust daemon, reached only via the Worker.

### Scope (decision A — "core chat client")

Pairing login → streaming chat → tool-call visualization (diff + terminal) → approval prompts (Approve/ApproveAlways/Deny) → presence + connection status + reconnect-with-replay → error/done states.

### Invariant (bounds the whole subsystem)

**Behavior-preserving on the wire.** The Rust `agent-server` daemon and the wire protocol (`{ v, session_id, id?, kind, ... }`, event `payload: { type, ... }`) are **unchanged**. The frontend speaks the existing contract as-is. The only `cloud/` change is Worker configuration (§4).

### Out of scope (deferred, each its own future cycle)

- **Settings** (model/endpoint, policy/allowlist, MCP) — editing pushes config to the daemon, which needs new inbound wire messages + a daemon capability. That is a daemon subsystem, not a frontend one.
- Rich markdown rendering, diff syntax highlighting, ANSI color in terminal output.
- Cross-device conversation history (would require the DO to persist `user_input`), multi-agent management UI.
- Real OAuth (auth stays pairing-based); production deploy specifics (custom domain, etc.).

## 2. Architecture

```
web/ (React SPA) ──served same-origin by──▶ cloud/ Worker ──relay──▶ Durable Object ──WS──▶ local daemon
        │ pairing: POST /pair ; stream: WS /browser     (same origin → no CORS)
```

The UI subscribes to the same `AgentEvent` stream the core emits (relayed by the Worker) and renders each event type; it sends `user_input` and `approval_response` back over the socket. The daemon already produces structured events and `ToolOutput.display` payloads (diffs, terminal blocks), so the UI gets rich visualization with no core changes.

### Hosting / origin model (decision A — Workers static assets, same-origin)

The SPA is served by the **existing Worker** via Workers static assets (Cloudflare's current recommendation, superseding Pages). The browser loads the app from the Worker's own origin, so `fetch('/pair')` and the `/browser` WebSocket are **same-origin — zero CORS**, no auth-endpoint CORS headers. One `wrangler dev` serves API + UI; `wrangler deploy` ships both. This is a conscious divergence from the primer's "deploy to Pages" wording, justified by the platform having moved to Workers static assets and by deliberately resolving the CORS question (same-origin → the problem does not exist).

## 3. Repo structure (new top-level `web/`, mirroring `agent/` + `cloud/`)

```
web/
  package.json          react, react-dom, vite, @vitejs/plugin-react, tailwindcss v4 (+ @tailwindcss/vite),
                        typescript, vitest, @testing-library/react, jsdom, diff
  vite.config.ts        react + tailwind plugins; build.outDir = "dist";
                        server.proxy { "/enroll","/pair","/agent","/browser" → http://127.0.0.1:8787, ws: true }
  index.html, tsconfig.json
  src/
    main.tsx, App.tsx
    wire.ts             TS mirror of the envelope + frame/payload types + decision enum
    socket.ts           typed WS client: connect, reconnect-with-backoff, status, onFrame
    state.ts            reducer + ConversationState types
    storage.ts          localStorage helpers: session token; sent user messages per session
    components/         PairingScreen, ChatView, StatusBar, MessageList, AssistantMessage,
                        ToolCall, DiffView, TerminalBlock, ApprovalPrompt, Composer
  test/                 reducer, wire, socket, and component tests
```

### Build tooling (decision A — separate `web/` package)

React + Vite + TypeScript as a standalone package; Vitest + React Testing Library (jsdom) for tests. The `cloud/` Worker's code and its `vitest-pool-workers` harness are **untouched** (only the config change in §4). Dev: `vite dev` with `server.proxy` (incl. `ws: true`) forwarding the four API/WS routes to `wrangler dev`, giving HMR while the browser sees one origin. Production-like verification: `vite build && wrangler dev` (everything same-origin on :8787). The Cloudflare Vite plugin (single-process integration) was rejected as churn on the working control plane for marginal gain.

## 4. The `cloud/` Worker change — configuration only

Add to `cloud/wrangler.jsonc` (no `worker.ts` / `session.ts` change):

```jsonc
"assets": {
  "directory": "../web/dist",
  "not_found_handling": "single-page-application",
  "run_worker_first": ["/enroll", "/pair", "/agent", "/browser"]
}
```

`run_worker_first` as an **array** of route patterns (requires Wrangler ≥ 4.20; the project is on 4.103) routes exactly the four API/WS paths to the Worker; every other path serves the built SPA with SPA fallback. The Worker now only ever sees its API routes — its existing 404-for-unknown branch stops being reached by browser navigation, no code change needed.

**Integration risk to manage:** `vitest-pool-workers` loads `wrangler.jsonc`, so an `assets.directory` pointing at a missing `../web/dist` could break `cd cloud && npm test`. The plan keeps `web/dist` present — a committed placeholder `web/dist/index.html` that the real `vite build` overwrites — so the existing cloud tests stay green (preserving the "cloud harness untouched" property).

## 5. Wire types (`wire.ts`) — hand-written TS mirror

Small and stable; no codegen. `Display` arrives as externally-tagged serde from Rust, so the TS union matches that exactly.

```ts
export const PROTOCOL_VERSION = 1;

export type Display =
  | { Text: string }
  | { Diff: { path: string; before: string; after: string } }
  | { Terminal: { command: string; stdout: string; stderr: string; exit_code: number } };

export type WireEvent =
  | { type: "token"; text: string }
  | { type: "tool_start"; name: string; args: unknown }
  | { type: "tool_result"; name: string; content: string; display?: Display }
  | { type: "error"; message: string }
  | { type: "done"; reason: string };

export type Inbound =
  | { v: number; session_id: string; kind: "event"; payload: WireEvent }
  | { v: number; session_id: string; id: string; kind: "approval_request"; summary: string; command?: string; display?: Display }
  | { v: number; session_id: string; kind: "presence"; online: boolean };

export type Decision = "approve" | "approve_always" | "deny";

export type Outbound =
  | { v: number; session_id: string; kind: "user_input"; text: string }
  | { v: number; session_id: string; id: string; kind: "approval_response"; decision: Decision };
```

## 6. Socket client (`socket.ts`)

A thin typed wrapper: `connect(wsUrl, { onFrame, onStatus }) → { send(out: Outbound), close() }`. It tracks status (`connecting | open | closed | error`), parses inbound JSON to `Inbound`, and **reconnects with backoff** on unexpected close (not on a deliberate sign-out / `close()`). On (re)open the DO automatically sends `presence` + replays the R2 event log; the client takes no special action — it just receives frames.

## 7. State model (`state.ts`)

`ConversationState` is a pure function of the frame sequence since connect:

```
ConversationState = { items: Item[]; pendingApproval: ApprovalReq | null; online: boolean; status: ConnectionStatus }
Item =
  | { kind: "user"; text }
  | { kind: "assistant"; text; done?: reason }        // streaming
  | { kind: "tool"; name; args; status: "running"|"done"; content?; display? }
  | { kind: "error"; message }
```

Reducer rules:
- `event.token` → append to the open assistant item (or start one); `tool_start` → push a `running` tool item; `tool_result` → resolve the **most-recent-running tool of that `name`** (single-active-tool, matching the daemon's sequential loop) and attach `content`/`display`; `error` → error item; `done` → close the turn (record `reason`).
- `approval_request` → set `pendingApproval`; sending an `approval_response` clears it optimistically.
- `presence` → set `online`; status transitions come from the socket.

### Reset-and-replay

Every (re)connect dispatches `reset` (empty state), then frames — replayed events first, then live — rebuild state deterministically.

### Optimistic, locally-persisted user messages (interleaved by turn)

The R2 log persists only AgentEvents, not `user_input`. Since each `user_input` produces exactly one assistant turn ending in `done`, the **K-th user message belongs to the K-th turn** (the run ending in the K-th `done`). `storage.ts` persists, per session in `localStorage`, the ordered list of sent user texts. On send, the UI renders the user item immediately (optimistic) and appends to that list. On replay rebuild, the reducer tracks a turn counter and emits stored user message K at the start of turn K — reconstructing **both sides** of the conversation, frontend-only, with no daemon/cloud change. A user message whose turn never reached `done` (sent just before a disconnect) replays as an in-progress turn.

## 8. Components & UI

A dark, Claude Code–style layout. The app shell routes on auth: **`PairingScreen`** when there is no stored session token, **`ChatView`** once paired + connected. Each component renders purely from `ConversationState` props, so it is unit-testable in isolation.

- **`PairingScreen`** — one field for the pairing code → `POST /pair` → store `session_token` in `localStorage` → connect. On load, a stored token auto-connects straight to `ChatView`.
- **`ChatView`** regions:
  - **`StatusBar`** (top) — presence dot (daemon online/offline from `presence`), connection status (`connecting/open/reconnecting`), sign-out (clears the token).
  - **`MessageList`** (scrollable) — renders `items` in order, component-per-type:
    - **user** — distinct style.
    - **`AssistantMessage`** — streamed text rendered incrementally (plain text for v1).
    - **`ToolCall`** — header (`⚙ name` + compact args summary + running-spinner/✓/✗), body switching on `Display`:
      - **`DiffView`** for `Display::Diff` — unified diff from `{before, after}` via the `diff` lib, +/- coloring, `path` header.
      - **`TerminalBlock`** for `Display::Terminal` — monospace `<pre>`: `$ command`, stdout/stderr, `exit_code` badge. No xterm.js (static captured output).
      - plain `content` otherwise.
    - **error** — error styling for `event.error`.
  - **`ApprovalPrompt`** (inline banner when `pendingApproval`) — summary + command + **Approve / Approve Always / Deny** → sends `approval_response` echoing the request `id`, optimistically clears.
  - **`Composer`** (bottom) — textarea + send → `user_send` (optimistic + localStorage) and sends the `user_input` frame; disabled while disconnected.

### Styling (decision A)

Tailwind CSS v4 + a few hand-built components (no component library). Dark, terminal-ish aesthetic.

## 9. Testing

All in the `web/` package (Vitest + jsdom, separate from cloud's `vitest-pool-workers`):

- **Reducer** — token accumulation; `tool_start`→`tool_result` correlation by name; `done` closing a turn; approval set/clear; presence; **reset-and-replay reconstruction**; **user-message turn-interleaving** (K-th user message ↔ K-th `done`).
- **Wire parse** — parse captured-from-real sample frames, including externally-tagged `Display::Diff`/`Display::Terminal`, to lock the TS↔Rust contract.
- **Components (RTL)** — `DiffView` colors +/- lines; `TerminalBlock` shows command/stdout/exit badge; `ToolCall` switches on `Display`; `ApprovalPrompt` buttons emit the right decision; `Composer` disabled when disconnected; `PairingScreen` posts + stores the token (mocked `fetch`).
- **Socket client** — status transitions + reconnect/backoff with a mock `WebSocket`; a deliberate sign-out does not reconnect.
- **Manual E2E (chrome-devtools, deferred to the human)** — the real UI against the live stack: pair → stream → approval round-trip → reconnect-replay → presence, driven same-origin (no CORS), reusing the documented bring-up flow.

## 10. Definition of done

A user opens the app (served same-origin by the Worker under `wrangler dev` / deploy), pairs with the daemon's code, and drives a task on their local agent: streamed tokens, tool calls with diff + terminal rendering, Approve/ApproveAlways/Deny prompts, presence + reconnect-with-replay (both sides of the conversation), and error/done states — all over the Worker, the browser never touching local resources. `web/` unit + component tests green; `cloud/` tests still green (assets placeholder keeps them passing); manual chrome E2E verified. **Zero daemon/wire-protocol changes; the `cloud/` change is configuration-only.**
