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

**Do not** use this skill for: generic Tauri v2 development (→ `tauri` skill);
automating GUI apps outside this repo on Wayland (→ `wayland` skill); or
pixel-driving when a WS-bridge rung is available — the bridge is the point.

## Prerequisites (check first)

- **cargo / node on PATH** — normally already true on this machine; only `source ~/.cargo/env` if a bare shell lacks `cargo`.
- **llama-server on :8080** for any live/model run: `curl -s -m 2 localhost:8080/health`
  must return `{"status":"ok"}`. Model is `qwen3.6-35b-a3b`, launched `-np 1`
  (no `n>1`). Drive it manually with `agent/scripts/chat.sh`. Note: `:8080` is where the
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
| Input tools installed | `xdotool` CLI + **`libxdo-dev`** (X11 headers, installed 2026-06-24) | enables X11/XWayland input via the CLI **or** a Rust `enigo` helper (links `-lxdo`); still no `ydotool`/`wtype`/`kdotool`/`wmctrl` → native-Wayland input unavailable |
| Tauri Linux system deps | **all satisfied** (incl. `libxdo-dev` as of 2026-06-24) | `tauri build` and input/global-shortcut/tray plugins that link `libxdo` now compile |
| AT-SPI (semantic layer) | bus runs, but **no Python `Atspi` binding**, `toolkit-accessibility=false` | Layer-1 a11y driving is not usable as-is; WebKitGTK web a11y is unreliable anyway |
| Webview | WebKitGTK 2.52.3 | renders as a Wayland client under this session |
| Screenshot tool | **`spectacle`** (no `grim`) | use spectacle for captures |
| XWayland | present (`DISPLAY=:0`) | the one escape hatch for `xdotool` (see below) |

This is exactly why L0/L1 win: the semantic (AT-SPI) and input-injection (Wayland)
layers the `wayland` skill prefers are both un-provisioned here, while the app
hands you a clean protocol socket.

## GUI driving — last resort (only to validate actual rendering/UX)

This path **works on this machine** — verified 2026-06-24 end-to-end: typed a prompt
into the composer, clicked Send, and saw the model's `pong` render in the transcript.
But it only works after clearing **two gates** and using XTEST correctly:

1. **Run the webview on XWayland** so `xdotool` can see it:
   `GDK_BACKEND=x11 npm run desktop:dev`. Without this the window is a native
   Wayland surface and `xdotool` silently no-ops. Then grab the id:
   `WID=$(DISPLAY=:0 xdotool search --name 'rust-agent-runtime' | head -1)`
   (title comes from `tauri.conf.json`; window is 1280×820).

2. **Approve KWin's input-control consent ONCE per session.** The first synthetic
   event raises a KDE *"Remote Control — An application is asking for special
   privileges: Control input devices"* Approve/Deny dialog (KWin gating XTEST
   fake-input). Until a **human** clicks Approve, no injected input reaches any
   app — your keystrokes just raise the prompt. You cannot reliably self-approve
   it (that click is itself gated); ask the user to click Approve. The grant
   persists for the rest of the session.

3. **Inject the way WebKit accepts it** (the load-bearing gotcha):
   - Position the pointer **window-relative** (sidesteps the HiDPI screen-coord mess
     — xdotool's logical space is 3840×2160 but the framebuffer is 6400×2160):
     `xdotool mousemove --window "$WID" <x> <y>`
   - Click/type with **XTEST — NO `--window`**: `xdotool click 1`; to focus an
     input, click it, then `xdotool type --delay 50 "your text"`.
   - **Never** `xdotool type/key --window <id>` into the webview — that uses
     `XSendEvent`, which WebKitGTK **silently ignores** (text never lands). This is
     the #1 reason a "successful" type does nothing.
   - Coord mapping for the 1280×820 window: window-relative ≈
     `(screenshot_x − ~40, screenshot_y − ~95)`; calibrate against a screenshot.

4. **Assert with a screenshot** — `xdotool` can inject input but cannot read the DOM:
   `spectacle -b -n -a -o /tmp/shot.png` (background, no-notify, active-window;
   captures at logical scale ~1410×978, readable). Read the PNG to verify layout /
   that a click did the right thing. To watch the running app's bridge externally
   instead, find its ephemeral port: `ss -tlnp | grep <app-pid>`.

`libxdo-dev` is installed, so you can alternatively drive X11/XWayland input from
Rust via the `enigo` crate (links `-lxdo`) — still X11-only, still behind gate 2.
For pure-Wayland input you'd need `ydotool` (uinput, root) or `kdotool` — neither is
present today.

## Common mistakes

- Trying to `xdotool` the window in the default Wayland session → no-op. Use
  `GDK_BACKEND=x11`, or better, drive the bridge.
- `xdotool type/key --window <id>` into the webview → silently ignored
  (`XSendEvent`). Click to focus, then type with XTEST (no `--window`).
- First synthetic input "does nothing" → it raised KWin's "Control input devices"
  prompt; a human must click Approve once per session before input flows.
- Running L1/e2e without `llama-server` on :8080 → hang/timeout. Check
  `curl localhost:8080/health` first.
- `source ~/.cargo/env` → only needed when `cargo` is missing from PATH (normally it isn't here).
- Hardcoding a bridge port → it's ephemeral; read `bridge.ws_url()`.
- Asserting on screenshots for L0/L1 → assert on `event` frames
  (`done`/`error`/`token`); screenshots are only for L2 GUI rendering checks.
