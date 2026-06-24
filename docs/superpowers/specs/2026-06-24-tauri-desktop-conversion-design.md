# Design: convert rust-agent-runtime into a Tauri desktop app

**Date:** 2026-06-24
**Status:** Approved (design), pending spec review

## Goal

Ship the existing web UI + Rust agent runtime as a single **Tauri v2 desktop
app** (Linux-primary) that runs **fully locally**: the React frontend and the
agent runtime run together in one process and talk over a localhost WebSocket,
with no Cloudflare Worker in the loop. The LLM is a fixed local
`llama-server` (Docker) on `http://localhost:8080`.

Guided by `.agents/skills/tauri/SKILL.md` and `.agents/skills/llama-server/SKILL.md`.

## Current architecture (for reference)

```
Browser (web/ React+Vite) ──WS──> Cloudflare Worker (:8787) <──WS── agent-serverd ──HTTP──> llama-server (:8080)
```

- `web/` connects via WebSocket through the Worker (dev: Vite proxy). Cloud mode
  requires an enroll/pairing/token handshake (`PairingScreen`).
- The agent runtime lives in the `agent/` Cargo workspace: `agent-core` (loop),
  `agent-server` (`daemon`/`runtime`/`sink`/`approval`/`wire`), plus
  `agent-tools`, `agent-mcp`, `agent-memory`, `agent-sandbox`, `agent-model`,
  `agent-skills`, `agent-policy`, `agent-runtime-config`.
- The frontend↔runtime protocol is `WireEnvelope`/`WireBody` JSON over one WS
  (`web/src/wire.ts` mirrors `agent-server/src/wire.rs`).

## Target architecture (fully local)

```
┌─ Tauri app (one process) ─────────────────────────────┐
│  Webview: web/ React UI  ──ws://127.0.0.1:<port>/agent │
│                                  │  (WireEnvelope)      │
│  Tauri Rust backend ─────────────┘                     │
│    • local WS server (accepts the webview)             │
│    • embeds the agent runtime (agent-* crates)         │
│    • workspace picker + persisted app config           │
└──────────────────────────────────┼────────────────────┘
                                    │ HTTP (OpenAI-compatible)
                                    ▼
                       llama-server :8080 (Docker, fixed)
```

The Worker, enroll/pairing/token flow, and the daemon's outbound dial are **not
used** by the desktop build. That code remains in the repo, unchanged. (Cloud
mode and desktop mode share the same frontend and the same wire protocol.)

The fixed llama-server is launched out-of-band by the user:

```
docker run -d --name llama-agent --gpus all -p 8080:8080 \
  -v /mnt/storage/models:/models:ro ghcr.io/ggml-org/llama.cpp:server-cuda \
  -m /models/qwen3.6-35b-a3b-gguf/Qwen3.6-35B-A3B-UD-IQ4_XS.gguf \
  -a qwen3.6-35b-a3b -ngl 99 -np 1 -c 262144 -fa on \
  --cache-type-k q8_0 --cache-type-v q8_0 --host 0.0.0.0 --port 8080 --jinja
```

The app connects to it; it does not manage the container.

## Repo layout

Conventional Tauri layout; the frontend stays `web/`:

```
rust-agent-runtime/
├─ web/                      # unchanged React+Vite frontend (the Tauri frontend)
├─ agent/                    # Cargo workspace; small refactor (see "Agent refactor")
├─ src-tauri/                # NEW — the Tauri Rust app
│  ├─ Cargo.toml            # standalone crate; path-deps into agent/crates/*
│  ├─ tauri.conf.json
│  ├─ build.rs
│  ├─ capabilities/default.json
│  ├─ icons/
│  └─ src/{main.rs, lib.rs, bridge.rs, workspace.rs}
└─ docs/superpowers/specs/…
```

`src-tauri` is its own crate that path-depends on `agent-server` (and
`agent-runtime-config`). It is **not** a member of the `agent/` workspace, so
`tauri-build` stays isolated from the agent workspace. (Cargo permits depending
by path on a package that is a member of another workspace.)

## Agent refactor (in `agent/`, no behavior change to the cloud path)

1. **Extract a transport-agnostic `serve()` in `agent-server`.**
   `daemon::run()` currently dials the Worker, then runs: build sink
   (`WsEventSink`) → approval (`WsApprovalChannel`) → `RuntimeState`, spawn a
   writer task draining the `mpsc<WireEnvelope>` channel to the socket, and a
   reader loop dispatching `UserInput`/`ApprovalResponse`/`SettingsGet|Update`.
   Everything after the `(write, read)` split is transport-agnostic.
   - New: `pub async fn serve<W, R>(write: W, read: R, params: DaemonParams)`
     where `W: Sink<WsMessage>` and `R: Stream<Item = Result<WsMessage, _>>` —
     contains the sink/approval/runtime/writer/reader body verbatim.
   - `run()` keeps its signature: dial Worker → `serve(write, read, params)`.
   - The cloud path behaves identically; its existing tests stay green.

2. **Factor runtime-params setup out of `main.rs`.** The block that builds
   `RuntimeConfig` (`from_launch` + field overrides), connects MCP
   (`connect_mcp`), and builds memory tools (`build_memory`) moves into a
   reusable helper, e.g. `agent_server::setup::build_params(opts) -> DaemonParams`,
   used by both the daemon binary and the Tauri backend. No duplicated logic.

## Tauri backend (`src-tauri/src/`)

- **`bridge.rs`** — on startup bind `TcpListener` on `127.0.0.1:0` (OS-assigned
  free port); `accept_async` each connection and hand its `(write, read)` to
  `agent_server::serve(...)`. One webview connection at a time. Holds the chosen
  port and a handle so the serve task can be restarted on workspace change.
- **`workspace.rs`** — persisted app config in the Tauri app-config dir
  (`{ workspace: Option<PathBuf> }`). On workspace change, restart the bridge's
  serve task bound to the new directory.
- **Commands (registered in `invoke_handler!`):**
  - `get_local_ws_url() -> String` → `ws://127.0.0.1:<port>/agent`
  - `get_workspace() -> Option<String>`
  - `pick_workspace() -> Option<String>` (opens native dialog via
    `tauri-plugin-dialog`, persists, restarts the bridge)
  - `llama_health() -> { ok: bool, model: Option<String> }` (Rust-side GET of
    `:8080/health` + `/props`; the webview never contacts `:8080` directly)
- **`lib.rs` / `main.rs`** — `tauri::Builder`, register `tauri-plugin-dialog`,
  manage state (port, workspace, bridge handle), `invoke_handler![…]`, run.

**Runtime defaults for the fixed llama-server** (seed `RuntimeConfig`; still
editable live via the existing Settings UI):
`backend=openai`, `base_url=http://localhost:8080`, `model=qwen3.6-35b-a3b`,
native protocol, `preserve_thinking=true` (Qwen3.6 reasoning round-trip).

## Frontend changes (`web/`, additive; `socket.ts` & wire protocol untouched)

- **New `web/src/transport.ts`** — resolve the connection by environment:
  - **Tauri mode** (`window.__TAURI__` present): `wsUrl = await
    invoke('get_local_ws_url')`; `sessionId = persisted crypto.randomUUID()`;
    no token; **skip `PairingScreen`**.
  - **Cloud/browser mode**: existing behavior — `PairingScreen` → token →
    `${location.origin→ws}/browser?token=…`.
- **`App.tsx`** — branch on mode. In Tauri mode, bypass the pairing gate (the
  `if (!token || !sessionId)` block) and feed the injected URL into the existing
  `connect(...)`. Everything downstream (reducer, settings, approvals,
  artifacts) is identical.
- **`TopBar`** — in Tauri mode, show the current workspace and a **"Change…"**
  button calling `pick_workspace`. Hidden in cloud mode.

**Workspace-change reconnect flow:** pick folder → backend restarts the `serve`
task on the new workspace → drops the active WS → `socket.ts` auto-reconnects →
new session runs against the new dir. The `settings_state.workspace` field keeps
the UI label correct.

## Security / capabilities

`src-tauri/capabilities/default.json`:
- Permissions: `core:default`, `dialog:default`.
- CSP (`app.security.csp`): allow the webview to open the local bridge socket —
  `connect-src 'self' ws://127.0.0.1:* ws://localhost:*`; `default-src 'self'`;
  plus what Vite/HMR needs in dev. `:8080` is **not** in the webview CSP.
- App `identifier`: `dev.rust-agent-runtime.desktop` (placeholder; change before
  any real distribution).

## Build / bundling (desktop-focused, Linux primary)

- `tauri.conf.json`: `build.devUrl=http://localhost:5173`,
  `frontendDist=../web/dist`, `beforeDevCommand="npm --prefix web run dev"`,
  `beforeBuildCommand="npm --prefix web run build"`.
- Dev: `tauri dev`. Release: `tauri build` → `.deb` + `.AppImage`.
- macOS/Windows are buildable later but out of scope now.
- Prereqs: Rust toolchain + `libwebkit2gtk-4.1-dev` and friends (present in this
  env). Docker must be running for both llama-server and the agent's sandbox
  tool.

## Testing

- **`agent-server`** — new test driving `serve()` over an in-memory duplex /
  loopback with the testkit `ScriptedModel`: `user_input` → token/`done` events
  observed on the channel. Existing cloud-path tests remain unchanged and green.
- **`web`** — unit test for `transport.ts` mode selection (mock
  `window.__TAURI__` present/absent).
- **`src-tauri`** — unit test for workspace-config persistence (load/save round
  trip); light smoke test that the bridge accepts a localhost connection.

## Non-goals / YAGNI

- No mobile (Android/iOS).
- No Docker/llama-server lifecycle management (assumed running).
- No removal of the cloud Worker / pairing path — it stays for cloud mode.
- No code signing / updater for this iteration (local Linux build only).
- No multi-window or multi-session; one webview, one active session (matches the
  current daemon MVP).

## Verification / acceptance

- `tauri dev` launches a window showing the existing UI with no pairing screen;
  a folder picker sets the workspace.
- Sending a message reaches the local runtime and streams tokens/tool events
  back; approvals prompt and resolve; the Settings panel loads and edits live.
- Changing the workspace reconnects and subsequent file/tool ops act on the new
  directory.
- `llama_health` reflects the `:8080` server (model `qwen3.6-35b-a3b`).
- `cargo test` in `agent/` is green (cloud path unchanged + new `serve` test);
  `npm --prefix web test` green; `tauri build` produces a `.deb`/`.AppImage`.
