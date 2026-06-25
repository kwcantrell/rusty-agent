---
name: auto-drive-tauri
description: >-
  Use when automating tests or verifying a change end-to-end for THIS repo
  (rust-agent-runtime) on Kalen's machine — the Tauri desktop app, its localhost
  WebSocket bridge, the React web UI, or the live model loop. Covers how to drive
  the app without a GUI, the bridge wire protocol, prerequisites (llama-server on
  :8080, cargo on PATH), and — as a last resort — driving the actual window on
  this KDE/Wayland desktop. Trigger on "verify it works", "automate a test",
  "drive the app", "run the e2e", "does the change work end to end".
---

# Driving rust-agent-runtime to automate tests

## Core principle

**Drive the WebSocket bridge, not the GUI.** The desktop app is a WebKitGTK
webview whose only job is to open `ws://127.0.0.1:<port>/agent` and exchange JSON
frames with the embedded Rust runtime (`src-tauri/src/bridge.rs` →
`agent_server::daemon::serve`). Everything the UI can do, a WS client can do —
headlessly, deterministically, with no compositor involved. The existing
`src-tauri/tests/e2e_live.rs` is the reference: start the bridge, connect, send
`user_input`, assert on the streamed events.

Pixel-driving the window is the **last resort** here (see §This machine for why).

## Prerequisites (check first)

- **cargo / node on PATH** — they are; do NOT `source ~/.cargo/env`.
- **llama-server on :8080** for any live/model run: `curl -s -m 2 localhost:8080/health`
  must return `{"status":"ok"}`. Model is `qwen3.6-35b-a3b`, launched `-np 1`
  (no `n>1`). Drive it manually with `scripts/chat.sh`. Note: `:8080` is where the
  server actually runs — some on-disk/CLI defaults say `:30000`; the desktop bridge
  and `e2e_live.rs` both override to `:8080`, so trust `:8080`.
- A clean build exists (`agent/`, `src-tauri/`, `web/` all build). First `cargo test`
  after a change recompiles (~1 min cold) before the 120s test window — a slow start
  is the build, not a hang.

## The driving ladder — pick the lowest rung that proves your change

| Rung | What it covers | Command | Needs :8080 |
|------|----------------|---------|-------------|
| L0 unit/integration | Rust logic, web components | `cd agent && cargo test` · `cd web && npm test` · `cd src-tauri && cargo test` | no |
| L1 protocol e2e (**default for "does it work end to end"**) | webview→bridge→runtime→model, real token stream | `cd src-tauri && cargo test --test e2e_live -- --ignored --nocapture` | **yes** |
| L2 GUI driving | the actual rendered webview / UX | see §GUI driving — last resort | yes |

`src-tauri/tests/bridge.rs` (`bridge_serves_local_runtime`) is an L0/L1 hybrid: it
exercises the full bridge→serve wiring with a **closed** model port, so it proves
the plumbing without needing :8080. Copy its pattern for new protocol tests.

## Bridge wire protocol (what a WS client sends/receives)

Connect to `bridge.ws_url()` = `ws://127.0.0.1:<port>/agent`. Text frames, JSON:

```jsonc
// send: run a turn (exactly what the React app sends on send())
{ "v": 1, "session_id": "e2e", "kind": "user_input", "text": "Reply with: pong" }
// send: read current settings
{ "v": 1, "session_id": "s1", "kind": "settings_get" }

// receive: event stream
{ "kind": "event", "payload": { "type": "token",      "text": "po" } }
{ "kind": "event", "payload": { "type": "tool_start", "name": "..." } }
{ "kind": "event", "payload": { "type": "error",      "message": "..." } }   // panic/fail the test
{ "kind": "event", "payload": { "type": "done",       "reason": "..." } }    // turn complete
{ "kind": "settings_state", /* ... */ }
```

**Assert on the event stream, never on pixels.** A turn is complete when you see
`type:"done"`; treat `type:"error"` as failure; collect `type:"token"` text.

To write a new headless e2e, start the bridge in-process exactly like
`e2e_live.rs:15`. The signature is `bridge::start(workspace, config_path,
base_url, model)` — e.g. `rust_agent_runtime_desktop_lib::bridge::start(
ws_dir.path().to_path_buf(), cfg, "http://localhost:8080".into(),
"qwen3.6-35b-a3b".into())` where `cfg` is a separate runtime-config path inside
`ws_dir`. Then connect with
`tokio_tungstenite::connect_async(bridge.ws_url())`. The bridge binds an
**ephemeral** port (`127.0.0.1:0`); always read it from `bridge.ws_url()`, never
hardcode.

## This machine (verified facts that dictate the approach)

| Fact | Value | Consequence |
|------|-------|-------------|
| Session | **Wayland**, KDE Plasma / **KWin** (`kwin_wayland`) | no X11 global input; XTEST-based tools don't see Wayland surfaces |
| Input tools installed | **only `xdotool`** (X11) | no `ydotool`/`wtype`/`kdotool`/`wmctrl` → native Wayland input is unavailable |
| AT-SPI (semantic layer) | bus runs, but **no Python `Atspi` binding**, `toolkit-accessibility=false` | Layer-1 a11y driving is not usable as-is; WebKitGTK web a11y is unreliable anyway |
| Webview | WebKitGTK 2.52.3 | renders as a Wayland client under this session |
| Screenshot tool | **`spectacle`** (no `grim`) | use spectacle for captures |
| XWayland | present (`DISPLAY=:0`) | the one escape hatch for `xdotool` (see below) |

This is exactly why L0/L1 win: the semantic (AT-SPI) and input-injection (Wayland)
layers the `wayland` skill prefers are both un-provisioned here, while the app
hands you a clean protocol socket.

## GUI driving — last resort (only to validate actual rendering/UX)

If you genuinely must drive the rendered window:

1. Force the webview onto **XWayland** so the only installed input tool works:
   `GDK_BACKEND=x11 npm run desktop:dev`.
2. Find the window and drive it with `xdotool` (confirm the actual title first —
   `xdotool search --name .` or check `tauri.conf.json`'s window `title`):
   `xdotool search --name 'rust-agent-runtime' windowactivate --sync key --clearmodifiers Return`
   (type with `xdotool type`, click with `xdotool mousemove`/`click`).
3. Capture state with `spectacle -b -n -o /tmp/shot.png` (background, no-notify).
4. **Still assert via the WS bridge or app logs, not the screenshot** — `xdotool`
   can inject input but cannot read the web DOM. To watch the running app's bridge
   externally, find its ephemeral port: `ss -tlnp | grep <app-pid>`.

Without `GDK_BACKEND=x11`, `xdotool` will silently fail to find/drive the window
(it's a native Wayland surface). To stay pure-Wayland instead, you must first
install `ydotool` (uinput, needs root) or `kdotool` — neither is present today.

## Common mistakes

- Trying to `xdotool` the window in the default Wayland session → no-op. Use
  `GDK_BACKEND=x11`, or better, drive the bridge.
- Running L1/e2e without `llama-server` on :8080 → hang/timeout. Check
  `curl localhost:8080/health` first.
- `source ~/.cargo/env` → unnecessary; cargo is on PATH.
- Hardcoding a bridge port → it's ephemeral; read `bridge.ws_url()`.
- Asserting on screenshots → flaky and can't see the DOM; assert on `event`
  frames (`done`/`error`/`token`).
