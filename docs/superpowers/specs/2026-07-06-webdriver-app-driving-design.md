# WebDriver-based desktop app driving

**Date:** 2026-07-06
**Status:** Approved (Option A of three considered)

## Problem

End-to-end verification of the Tauri desktop app currently relies on pixel-level
X automation (xdotool clicks at window-relative coordinates, xclip
primary-selection paste, xwd/spectacle + ffmpeg screenshots). It works, but every
session re-fights the same failure modes: KWin focus-stealing prevention eating
synthetic clicks, absolute-vs-window-relative coordinate misses on the
multi-monitor layout, xclip losing the selection when the sandboxed shell exits,
stub windows matching the app's name, and `pkill` self-matches. Assertions are
screenshot-based, so every UI layout change silently re-breaks the recipe.

The frontend cannot be driven in a plain browser instead: the desktop UI is
Tauri-IPC-bound (`subscribe`, `send_input`, `approve`, settings, dev-server
commands all go through `invoke`), so the real app must be the thing driven.

## Decision

Adopt Tauri 2's official WebDriver path — `tauri-driver` +
WebKitGTK's WebDriver — as the primary driving layer, for both interactive
verification sessions and a checked-in scripted e2e suite. Keep the existing
xdotool recipe only as a documented fallback for native GTK chrome that
WebDriver cannot reach.

Alternatives considered and rejected:

- **Dev-only automation backdoor in the app** (debug-only Tauri command that
  evals JS in the webview): no new system deps, but bespoke machinery to
  maintain, test code shipping in debug binaries, and a strictly worse
  reimplementation of WebDriver.
- **Hardening the xdotool stack** (scripts codifying the footgun fixes,
  ydotool): least new surface but keeps blind pixel driving and
  screenshot-based assertions.

## 1. System setup (one-time, this machine)

- `sudo apt install webkitgtk-webdriver` — provides `WebKitWebDriver` matched
  to the installed `libwebkit2gtk-4.1` (2.52.x); both build from the same
  source package, so apt keeps them in lockstep.
- `cargo install tauri-driver` — the shim that speaks standard WebDriver on
  one port, proxies to WebKitWebDriver, and launches the app binary named in
  the `tauri:options.application` capability.
- `sudo apt install python3-selenium` — client for interactive driving.

No app code changes. The driver launches the existing
`src-tauri/target/debug/rust-agent-runtime-desktop` binary in automation mode.

## 2. Scripted e2e suite

New integration test target in the `src-tauri` workspace, following the
existing live-gated pattern of `tests/smoke_context_explorer.rs`.

**Files**

- `src-tauri/tests/gui_smoke.rs` — the tests.
- `src-tauri/tests/e2e_harness/mod.rs` — shared harness module owning
  process lifecycle.
- Dev-dependencies: `thirtyfour` (Rust WebDriver client); `tokio` and
  `tempfile` are already present.

**Harness responsibilities**

1. Preflight Vite: if `http://localhost:5173` does not answer, spawn
   `npm --prefix web run dev` and wait for readiness (the debug binary loads
   its frontend from the dev URL, not bundled assets). Only kill Vite on
   teardown if the harness started it.
2. Spawn `tauri-driver` on a free port with `--native-driver` pointing at the
   apt-installed `WebKitWebDriver` if it is not on PATH.
3. Connect a `thirtyfour` client with the `tauri:options.application`
   capability set to the debug binary path; wait for the session.
4. Teardown via a Drop guard that kills tauri-driver (which owns the app
   process) and any harness-spawned Vite — kill by held child PID, never by
   pattern match.

**Initial tests**

- `boot_smoke` (`#[ignore = "live: needs display + vite"]`): app renders;
  composer textarea and main tabs are present in the DOM. No model required.
- `turn_smoke` (`#[ignore = "live: needs display + vite + llama :8080"]`):
  additionally gated on `curl localhost:8080/health`; types a prompt via
  `send_keys`, clicks send, waits for the reply bubble, asserts its text and
  that the Context tab shows populated segments — all DOM reads, no pixels.

Both are `#[ignore]`d because they need a display and local services; they run
on demand with `cargo test --test gui_smoke -- --ignored` from `src-tauri/`.
Not added to `scripts/ci.sh` (src-tauri is not part of that gate today).

## 3. Interactive driving (verification sessions)

Rewrite `.agents/skills/auto-drive-tauri/SKILL.md` so WebDriver is the primary
layer:

- Bring-up: Vite on :5173, then `tauri-driver`, then a selenium session from
  short `python3 -c` snippets (the apt python has selenium; note the PATH
  python is a uv shim — use `/usr/bin/python3`).
- Verbs: find by CSS selector, `.click()`, `.send_keys()`,
  `execute_script(...)` for state reads, protocol screenshots
  (focus-independent, webview-exact).
- The current xdotool/xclip/spectacle recipe moves to an explicit **fallback
  section**, reserved for native GTK chrome — today that is the
  `pick_workspace` file dialog — and whole-window screenshots that must include
  native decorations.

## 4. Error handling and known risks

- **Version skew** between `WebKitWebDriver` and `libwebkit2gtk`: same apt
  source package, but the skill documents a one-line preflight check
  (compare `dpkg -l` versions) before driving.
- **No attach semantics**: WebDriver cannot connect to an already-running,
  hand-launched app instance; the driver must own the launch. For inspecting a
  live instance, use the xdotool fallback.
- **Port conflicts**: harness picks a free port for tauri-driver rather than
  assuming 4444.
- **Native dialogs**: out of WebDriver's reach by design; covered by the
  fallback layer.
- **Screenshot scope**: protocol screenshots capture webview content only,
  which is what DOM-level assertions need; native-chrome shots use the
  fallback recipe.

## Success criteria

1. `cargo test --test gui_smoke -- --ignored` passes on this machine with
   llama :8080 up, with no xdotool/xclip involvement and no manual focus click.
2. An interactive session can type into the composer, send a turn, and read
   the reply text from the DOM using selenium one-liners.
3. The updated skill lets a fresh session reach "driving the app" without
   rediscovering any of the recorded footguns.
