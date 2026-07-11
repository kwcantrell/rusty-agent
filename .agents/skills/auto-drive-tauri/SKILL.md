---
name: auto-drive-tauri
description: >-
  Use when automating tests or verifying a change end-to-end for THIS repo
  (rust-agent-runtime) on Kalen's machine — the Tauri desktop app, its localhost
  WebSocket bridge, the React web UI, or the live model loop. Covers how to drive
  the app via WebDriver (DOM-level, primary layer) or the WebSocket bridge, plus
  xdotool as a native-chrome-only fallback. Trigger on "verify it works",
  "automate a test", "drive the app", "run the e2e", "does the change work end
  to end".
---

# Driving rust-agent-runtime to automate tests

## Core principle

**Drive the WebSocket bridge or WebDriver — not pixels.** The desktop app is a
WebKitGTK webview whose only job is to open `ws://127.0.0.1:<port>/agent` and
exchange JSON frames with the embedded Rust runtime (`src-tauri/src/bridge.rs` →
`agent_server::daemon::serve`). Everything the UI can do, a WS client can do —
headlessly, deterministically, with no compositor involved. The existing
`src-tauri/tests/smoke_context_explorer.rs` is the reference bridge test: start
the bridge, connect, send `user_input`, assert on the streamed events.

For UX / rendering assertions that require the real webview, use WebDriver
(§GUI driving — WebDriver) — not pixels. Pixel-driving the window is the
**last resort**, limited to native GTK chrome the WebDriver cannot reach
(§Pixel fallback).

**Do not** use this skill for: generic Tauri v2 development (→ `tauri` skill);
"what's the right Tauri practice" lookups (→ `tauri-okf` skill, verified bundle
at `docs/okf/tauri/`); automating GUI apps outside this repo on Wayland
(→ `wayland` skill); or
pixel-driving when a WS-bridge or WebDriver rung is available — the bridge and
WebDriver are the point.

## Prerequisites (check first)

- **cargo / node on PATH** — normally already true on this machine; only `source ~/.cargo/env` if a bare shell lacks `cargo`.
- **llama-server on :8080** for any live/model run: `curl -s -m 2 localhost:8080/health`
  must return `{"status":"ok"}`. Model is `qwen3.6-35b-a3b`, launched `-np 1`
  (no `n>1`). Drive it manually with `agent/scripts/chat.sh`. Note: `:8080` is where the
  server actually runs — some on-disk/CLI defaults say `:30000`; the desktop bridge
  and `smoke_context_explorer.rs` both override to `:8080`, so trust `:8080`.
- A clean build exists (`agent/`, `src-tauri/`, `web/` all build). First `cargo test`
  after a change recompiles (~1 min cold) before the 120s test window — a slow start
  is the build, not a hang.

## The driving ladder — pick the lowest rung that proves your change

| Rung | What it covers | Command | Needs :8080 |
|------|----------------|---------|-------------|
| L0 unit/integration | Rust logic, web components | `cd agent && cargo test` · `cd web && npm test` · `cd src-tauri && cargo test` | no |
| L1 protocol e2e (**default for "does it work end to end"**) | webview→bridge→runtime→model, real token stream | `cd src-tauri && cargo test --test smoke_context_explorer -- --ignored --nocapture` (fast no-model gate: `--test llama_health`) | **yes** |
| L2 GUI driving (WebDriver) | the actual rendered webview / UX, via the DOM | `cd src-tauri && cargo test --test gui_smoke -- --ignored --test-threads=1` or drive interactively — see §GUI driving (WebDriver) | boot: no · turn: yes |
| L3 pixel fallback | native GTK chrome only (file dialogs, window decorations) | see §Pixel fallback | — |

There is no offline bridge-wiring hybrid test anymore (`src-tauri/tests/bridge.rs`
was deleted in `474b7af`). For new protocol tests copy the pattern of
`src-tauri/tests/smoke_context_explorer.rs` (L1: drives the in-process bridge +
Session exactly like the desktop app's Tauri commands and asserts on the event
stream; needs :8080), using `src-tauri/tests/llama_health.rs` as the fast
no-model gate (wiremock-backed, no server required).

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
`smoke_context_explorer.rs`. The signature is `bridge::start(workspace, config_path,
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
| XWayland | present (`DISPLAY=:0`) | the one escape hatch for `xdotool` (see §Pixel fallback) |

This is why the driving ladder exists: L1/L2 win for most cases; the semantic
(AT-SPI) and input-injection (Wayland) layers the `wayland` skill prefers are
both un-provisioned here, while the app hands you a clean protocol socket and a
full WebDriver interface.

## GUI driving — WebDriver (primary)

Verified 2026-07-06: `tauri-driver` + WebKitGTK WebDriver drives the real app's
DOM — CSS selectors, real key events, `execute_script`, focus-independent
screenshots. No coordinates, no KWin consent dialog, no xclip, no focus click.

**Tier-2/Tier-3 lifecycle pointer (2026-07-10):** Tier-2 GUI lifecycle tests
(parked-run banner + DOM approve, deny-with-feedback, real cross-surface
GUI→CLI switch) live at `src-tauri/tests/gui_lifecycle.rs` — run with
`cd src-tauri && cargo test --test gui_lifecycle -- --ignored --test-threads=1`
(needs Vite on :5173, `tauri-driver`, and `llama-server` on :8080; relocates
app state via `HOME`/`XDG_*` env on the driver spawn so it never touches the
real `~/.rusty-agent`). The Tier-1 deterministic lifecycle/stress matrix
(no GUI, no model) lives in `agent/crates/agent-e2e` and joins `ci.sh`; it
drives both the `agent_server::Session` API in-process and the real `agent`
CLI binary against a scripted model stub.

**Route choice (deliberate):** Tauri's first-party recommendation for e2e is
WebdriverIO + `@wdio/tauri-service`. This repo takes the documented
*direct-driver* route instead — a custom Rust harness (thirtyfour) plus
Selenium one-offs, Linux-only, no Node test runner — exactly the case Tauri
supports it for. Don't "upgrade" it to the service; reconsider if e2e ever
needs capabilities the direct route lacks (current delta:
`docs/okf/tauri/practices/webdriver-e2e-testing.md`).

**Bring-up (order matters):**
1. Vite on :5173 (`curl -s -o /dev/null -w '%{http_code}' localhost:5173` → 200,
   else `setsid npm --prefix web run dev` and wait).
2. Debug binary built (`cd src-tauri && cargo build`).
3. `setsid ~/.cargo/bin/tauri-driver --port 4444 --native-port 4445 >/tmp/td.log 2>&1 &`
   then `curl -s 127.0.0.1:4444/status` → `"ready":true`. Do NOT launch the app
   yourself — the WebDriver session launches (and owns) it. (If the binary is
   missing: `cargo install tauri-driver --locked`; it proxies to
   `/usr/bin/WebKitWebDriver` — on this machine from the `webkitgtk-webdriver`
   package. `webkit2gtk-driver` is the older Debian/Ubuntu name upstream docs
   use; it has no install candidate on this release.)

**Drive with `/usr/bin/python3` (the PATH python is a uv shim without selenium):**

```python
/usr/bin/python3 - <<'EOF'
from selenium.webdriver import Remote
from selenium.webdriver.common.options import ArgOptions
from selenium.webdriver.common.by import By
from selenium.webdriver.support.ui import WebDriverWait
from selenium.webdriver.support import expected_conditions as EC
o = ArgOptions()
o.set_capability("tauri:options", {"application": "/home/kalen/rust-agent-runtime/src-tauri/target/debug/rust-agent-runtime-desktop"})
o.set_capability("browserName", "wry")
d = Remote("http://127.0.0.1:4444", options=o)
# React tree mounts ~3-5s after session create; wait before interacting.
WebDriverWait(d, 15).until(EC.presence_of_element_located((By.CSS_SELECTOR, "button[role='tab']")))
d.find_element(By.CSS_SELECTOR, "textarea[aria-label='prompt']").send_keys("Reply with: pong\ue007")  # \ue007 = W3C Enter, submits
# Poll d.execute_script("return document.body.innerText") for the reply.
# Use a unique marker per run (e.g. SQRL<epoch-millis>) so stale transcript
# state from a previous run cannot fake a reply.
d.save_screenshot("/tmp/shot.png")   # webview-exact, focus-independent
d.quit()                             # ALWAYS quit — see session-lifetime note
EOF
```

**Useful selectors:** composer `textarea[aria-label='prompt']` (Enter submits —
there is no send button); tabs `button[role='tab']` — count VARIES: 5 fixed
nav tabs (Workspace / Context / Design / Architecture / Config) plus per-design
sub-tabs (one per design when >1 exist) and Workspace artifact tabs (also
`role='tab'`), so the live total changes with app state; always select by
text, never by count or index
(e.g. `By.XPATH, "//button[@role='tab' and normalize-space()='Architecture']"`);
Architecture and Config tabs are present only under real Tauri IPC — asserting
their presence confirms you drove the real app, not a plain browser; Context
breakdown total `//div[contains(., ' / ') and contains(., ' tokens')]`, legend chips
`//button[contains(., 'system')]`.

**Session lifetime = app lifetime.** WebKitWebDriver allows ONE session; a
python process that exits without `quit()` wedges the driver. To reset: kill
tauri-driver. Note that `setsid ... &` followed by `TD_PID=$!` captures the
**setsid wrapper's PID**, not tauri-driver's own PID. Kill reliably by port
(primary method): `ss -tlnp | grep 4444` to find the PID, then `kill <pid>`.
If you prefer killing by PID variables, capture the child first before killing
the wrapper: `TD_CHILD=$(pgrep -P $TD_PID) && kill $TD_CHILD && kill $TD_PID`
— killing the wrapper first orphans the child. You cannot attach to
an already-running app — the WebDriver session must launch it. So: plan the
whole drive sequence, run it as ONE script, always `quit()`.

**Scripted equivalent:** `src-tauri/tests/gui_smoke.rs` (`boot_smoke`,
`turn_smoke`) with the reusable harness in `src-tauri/tests/e2e_harness/mod.rs`
— copy its lifecycle pattern for new GUI tests. The harness uses thirtyfour 0.37
(`Capabilities::set(key, value)` rather than `insert`) and kills by process
group, never by pattern-match, to avoid the pkill self-match footgun.

**Preflight if the driver misbehaves:** `dpkg -l libwebkit2gtk-4.1-0
webkitgtk-webdriver | grep ^ii` — the two versions must match.

**If gui_smoke ever moves into CI:** recipe in
`docs/okf/tauri/practices/webdriver-ci.md` (xvfb fake display + the WebKit
driver package — package name varies by distro release, see preflight above).

## Pixel fallback (native GTK chrome only)

WebDriver cannot reach native chrome: the `pick_workspace` GTK file dialog,
window decorations, or an already-running instance you can't relaunch. Only
then, fall back to the xdotool recipe below (verified 2026-06-24):

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
  (`done`/`error`/`token`); screenshots (via WebDriver or spectacle) are only
  for L2/L3 rendering checks — prefer DOM assertions at L2.
- Launching the app yourself and then trying to WebDriver it → no attach
  semantics; the session must launch the app. Kill your instance first.
- Selenium script died without `quit()` → next session hangs at create. Kill
  tauri-driver by PID (or port lookup: `ss -tlnp | grep 4444`) and restart it.
- Using PATH `python3` for selenium → uv shim, no apt packages. Use `/usr/bin/python3`.
- Driving pixels for something the DOM can answer → use WebDriver first; pixels
  are only for native chrome (§Pixel fallback).
