# Design Spec — Remove Cloud Control Plane (Fully-Local Desktop)

**Date:** 2026-06-24
**Status:** Approved (pending implementation plan)

## 1. Summary

Remove the Cloudflare control plane (`cloud/`) and every cloud-only code path so the
project has exactly **one** execution model: the Tauri desktop app with its embedded
local WebSocket bridge. The browser/pairing path — which only ever functioned with the
Worker running — is deleted along with the Rust cloud-client code that dialed the Worker.

This is a deletion/cleanup subsystem, not a feature. No new behavior is added; the
desktop app already works fully locally (`src-tauri/src/bridge.rs` embeds the runtime via
`agent_server::daemon::serve`). The goal is to delete the now-dead second path and fix the
vestigial Vite proxy that produces a startup `ECONNREFUSED 127.0.0.1:8787`.

## 2. Background — the two modes today

| | Cloud / browser mode (REMOVE) | Desktop / Tauri mode (KEEP) |
|---|---|---|
| Backend | `cloud/` Cloudflare Worker (enroll / pair / relay) | `src-tauri/src/bridge.rs` embeds the runtime — no network hop |
| Rust entry | `agent-server` bin `agent-serverd`: `Enroll`/`Run` dial the Worker (`config.rs`, `daemon::run`, `wire::Presence`) | `bridge.rs` calls the **library** (`setup::local_params` + `daemon::serve`) |
| Frontend | `PairingScreen.tsx`, `transport.ts` `needsPairing` branch, `App.tsx` routing, Vite proxy → `:8787` | `transport.ts` `isTauri()` → `invoke("get_local_ws_url")` → `ws://127.0.0.1:<port>/agent` |
| Launcher | `scripts/launch-web-ui.sh` (worker + vite + Worker-dialing daemon) | `npm run desktop:dev` / `desktop:build` |
| Tests | `web/test/{pairing,app,smoke}.test.tsx`, `agent .../tests/daemon_roundtrip.rs` | `agent .../tests/serve_inbound.rs` |

The desktop path never touches the cloud path — confirmed in code. That is why the
desktop app boots and works while the Worker is absent; the only symptom is the Vite
proxy logging one failed connection attempt to `:8787`.

## 3. Scope of changes

### 3.1 Delete cloud outright
- `cloud/` — the entire Worker subsystem (`worker.ts`, `session.ts`, `util.ts`,
  `schema.sql`, `wrangler.jsonc`, `RUNNING.md`, tests, `testpage/`, `node_modules/`,
  `.wrangler/`, `.dev.vars`).
- `scripts/launch-web-ui.sh` — orchestrates the cloud stack. `scripts/` becomes empty →
  remove the directory.
- `agent-server.json` (repo root) — stale cloud enrollment credentials.

### 3.2 Rust — strip the cloud client from `agent-server`
The local library surface (`daemon::serve`, `setup::local_params`, `runtime`, `sink`,
`approval`, `wire` minus `Presence`) is **kept**; the outbound/cloud surface is removed.

- **Delete** `src/config.rs` (`DaemonConfig`, `enroll`, `ws_url`); remove `pub mod config`
  from `lib.rs`.
- **Delete** `src/main.rs` and the `[[bin]] agent-serverd` entry in `Cargo.toml`. Both
  subcommands (`Enroll`, `Run`) are cloud-only. `agent-server` becomes a **library-only**
  crate. (The terminal runner `agent-cli` is a separate crate and is untouched.)
- **`daemon.rs`:** remove `run()`; remove the now-dead `ws_url` and `agent_token` fields
  from `DaemonParams`. Keep `serve()`, `SYSTEM_PROMPT`, and the slimmed `DaemonParams`.
- **`setup.rs`:** drop the two empty-string `ws_url`/`agent_token` initializers.
- **`wire.rs`:** remove the `Presence { online: bool }` variant (cloud presence; not
  emitted by `serve`).
- **Tests:** delete `tests/daemon_roundtrip.rs` (drives `daemon::run`); keep
  `tests/serve_inbound.rs`. In `Cargo.toml`, remove the `[[bin]]` and prune `reqwest` if it
  becomes unused by the crate.

### 3.3 Web — remove the pairing/browser path
- **Delete** `src/components/PairingScreen.tsx`.
- **`transport.ts`:** drop `needsPairing` and the non-Tauri branch. `resolveTransport`
  resolves via `invoke("get_local_ws_url")` when Tauri is present; when Tauri is **absent**
  it returns an empty `wsUrl` (see §4).
- **`App.tsx`:** remove the pairing route. When `wsUrl` is empty (no Tauri), render the
  "Run desktop app" notice (§4); otherwise render the chat view.
- **`vite.config.ts`:** delete the entire `server.proxy` block (`/enroll`, `/pair`,
  `/agent`, `/browser` → `:8787`). **This removes the `ECONNREFUSED` startup noise.** Keep
  the `react()` and `tailwindcss()` plugins.
- **Tests:** delete `test/pairing.test.tsx`; update `test/app.test.tsx` and
  `test/smoke.test.tsx` to assert the non-Tauri notice (or the chat view via a mocked
  transport) instead of the pairing screen.

### 3.4 Docs & system deps
- Leave the dated `docs/superpowers/specs|plans/*` cloud documents as historical record.
  Add one line to `docs/superpowers/context/README.md` marking the Cloudflare control-plane
  subsystem as **removed** (superseded by this spec).
- **`libxdo-dev`** is the only missing Tauri Linux dependency and is **not required** for
  the current build (input-simulation / global-shortcut plugins are not used). Optional:
  `sudo apt-get install -y libxdo-dev`. Out of the critical path; not a blocker.

## 4. Non-Tauri behavior (decided)

The web bundle can load without Tauri (plain browser, jsdom tests). With pairing removed
there is no cloud fallback, so non-Tauri is an **unsupported** runtime that must fail
honestly rather than hang.

- `resolveTransport()` returns `{ wsUrl: "" }` when `isTauri()` is false.
- `App` treats an empty `wsUrl` as "not running under the desktop shell" and renders a
  small notice: *"This is the desktop app — launch it from the rust-agent-runtime window."*
- Tests assert this notice for the no-Tauri case, or inject a mock transport to exercise
  the chat view. No network attempt is made.

## 5. Data flow after the change (unchanged for desktop)

```
Tauri webview (React)
  └─ transport.ts: invoke("get_local_ws_url")  →  ws://127.0.0.1:<port>/agent
        └─ src-tauri/src/bridge.rs  (TcpListener on 127.0.0.1:0)
              └─ agent_server::setup::local_params(workspace, cfg, base_url, model)
                    └─ agent_server::daemon::serve(ws, params)   [the ReAct runtime]
                          └─ llama-server :8080  (model inference)
```

No Worker, no enrollment, no pairing, no outbound WebSocket.

## 6. Verification (definition of done)

1. `cd agent && cargo build` — clean; `agent-server` builds as a library (no `agent-serverd`).
2. `cd agent && cargo test` — passes; `serve_inbound` green; no reference to `daemon::run`,
   `config::enroll`, or `wire::Presence` remains (`grep` clean).
3. `cd src-tauri && cargo build` — clean (bridge still compiles against the slimmed
   `DaemonParams`).
4. `cd web && npm run typecheck && npm test` — green; no `PairingScreen`, no `needsPairing`.
5. `npm run desktop:dev` — app boots, connects to the local bridge, **no `ECONNREFUSED`**
   line in the logs.
6. Repo grep for `8787`, `wrangler`, `enroll`, `pairing`, `control plane`, `cloud/` outside
   the historical `docs/` returns nothing.

## 7. Risks & non-goals

- **Risk:** removing `daemon::run`/`DaemonParams` fields could touch shared code. Mitigated
  by keeping `serve` and slimming `DaemonParams` behind a compiler check (build is the gate).
- **Risk:** `reqwest` removal from `agent-server` — verify no remaining use before pruning;
  leaving it is harmless if uncertain.
- **Non-goal:** rewriting history in old specs/plans; only a pointer note is added.
- **Non-goal:** mobile, packaging/signing, or any change to `agent-cli`, model, sandbox,
  memory, MCP, or skills crates.
