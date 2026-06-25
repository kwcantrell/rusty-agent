# Running and driving Wayland apps headless (CI)

Goal: launch a GUI app, drive it, and capture screenshots with **no physical
monitor and no logged-in desktop** — e.g. in a CI container. The strategy:
**run your own compositor instance**, so *you* are the policy authority (no user
to click permission dialogs) and the app has a real Wayland display to attach to.

## 1. The three pieces you must provide

A normal desktop session gives these for free; headless, you assemble them:

1. **A runtime dir + session D-Bus** — `XDG_RUNTIME_DIR` and a private session
   bus. `dbus-run-session` creates the bus; it does **not** create a compositor.
2. **A compositor with a virtual/headless output** — so surfaces have somewhere
   to render without hardware. Options below.
3. **The accessibility bus** (only if using Layer-1 AT-SPI) — `at-spi2-core`'s
   bus must be running on that session bus.

## 2. Pick a compositor backend

| Compositor | Headless invocation | Notes |
|------------|--------------------|-------|
| **KWin** (KDE-native) | `kwin_wayland --virtual --width 1920 --height 1080` | Virtual backend, no output hardware. Closest to a real Plasma/KWin target — use this when testing KWin-specific behavior. Needs the `kwin-wayland-backend-virtual` package on some distros. |
| **Weston** (reference) | `weston --backend=headless --width=1920 --height=1080` | Lightweight, predictable; has a built-in screenshooter. Good generic target. |
| **Cage** (kiosk, wlroots) | `WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 cage -- ./app` | Single-app fullscreen; wlroots headless backend. Good for "run exactly one app". |
| **wlroots helper** | `wlheadless-run -- ./app` | Convenience wrapper that spins up a headless wlroots compositor around a command. |

Use **KWin `--virtual`** when your production target is KDE/KWin (matches real
behavior, including KWin scripting). Use **Weston headless** or **cage** for a
toolkit-only test that doesn't care about the compositor.

## 3. Full runnable pattern (KWin + AT-SPI driver)

```bash
#!/usr/bin/env bash
set -euo pipefail

# 1) runtime dir
export XDG_RUNTIME_DIR="$(mktemp -d)"; chmod 700 "$XDG_RUNTIME_DIR"

# 2) everything below runs under a private session bus
dbus-run-session -- bash -euo pipefail <<'SESSION'
  # 2a) start the accessibility bus (needed for Layer-1 AT-SPI driving)
  /usr/libexec/at-spi-bus-launcher --launch-immediately &
  export QT_ACCESSIBILITY=1

  # 2b) start a nested/virtual compositor; capture the display it creates
  kwin_wayland --virtual --width 1280 --height 800 --xwayland &
  KWIN_PID=$!
  # KWin announces its socket as wayland-0/1/... in XDG_RUNTIME_DIR; wait for it:
  for i in $(seq 1 50); do
    sock=$(ls "$XDG_RUNTIME_DIR"/wayland-* 2>/dev/null | grep -v '\.lock' | head -1) && break
    sleep 0.2
  done
  export WAYLAND_DISPLAY="$(basename "$sock")"

  # 3) launch the app under test ON that compositor
  QT_QPA_PLATFORM=wayland ./build/hello-wayland &
  APP_PID=$!
  sleep 1   # or poll the a11y bus until the app appears (see automation-atspi.md)

  # 4) drive it (Layer 1)
  python3 driver.py   # the AT-SPI driver from automation-atspi.md

  # 5) screenshot for artifacts (see §4)
  grim screenshot.png 2>/dev/null || spectacle -b -n -o screenshot.png || true

  kill "$APP_PID" "$KWIN_PID" 2>/dev/null || true
SESSION
```

For **Weston** instead, swap step 2b:

```bash
weston --backend=headless --socket=wayland-ci --width=1280 --height=800 &
export WAYLAND_DISPLAY=wayland-ci
```

## 4. Screenshots headless

- **wlroots compositors** (cage/sway): `grim out.png`.
- **Weston**: built-in screenshooter (key binding) or `grim` if `wlr-screencopy`
  is available.
- **KDE/KWin**: `spectacle -b -n -o out.png` (background, no-notify), or the
  **`Screenshot` / `ScreenCast` portal** for a sanctioned capture.
- **Toolkit-side**: have the app render to an offscreen surface and dump a
  framebuffer (`QQuickWindow::grabWindow()` in Qt) — fully compositor-independent
  and the most deterministic for pixel assertions.

## 5. The WebDriver path in CI

`selenium-webdriver-at-spi` is built for this: it can run the app inside a
**localhost webserver for non-GUI/CI environments** as well as locally in KWin,
and it bundles the AT-SPI plumbing. In CI you still need the session bus + a11y
bus (steps 2/2a); the WebDriver server then launches and drives the app. This is
how KDE runs its own app tests in CI — prefer it if you want a managed harness
rather than hand-rolling the bring-up above.

## 6. Input synthesis in CI

- **Layer 1 (AT-SPI)** needs no special input device — preferred in CI.
- If you must inject raw input (Layer 2), **`ydotool` is the CI-friendliest**:
  no portal dialog, works against any compositor. The catch is `/dev/uinput`
  access — the CI container must expose the device and run `ydotoold`
  (`--device /dev/uinput` on the container, plus a udev/group grant). See
  `automation-input-injection.md` §2.
- The **RemoteDesktop portal** path is awkward in CI because `Start` wants user
  consent. Either avoid it (use AT-SPI / ydotool) or use a compositor whose
  policy auto-approves a session you own.

## 7. Container checklist

- Packages: the compositor (`kwin-wayland` / `weston` / `cage`), `dbus`,
  `at-spi2-core`, your toolkit runtime (`qt6-*` + `kf6-kirigami2`), the driver
  deps (`python3-gi gir1.2-atspi-2.0`), and capture (`grim` or `spectacle`).
- Env: set `XDG_RUNTIME_DIR` (700, owned by the run user), `WAYLAND_DISPLAY`
  (after the compositor is up), `QT_QPA_PLATFORM=wayland`, `QT_ACCESSIBILITY=1`.
- For `ydotool`: mount/allow `/dev/uinput` and start `ydotoold`.
- Don't forget to **wait** for each stage (socket exists → app on a11y bus)
  rather than `sleep`-guessing; flaky CI is almost always a missing wait. See
  `troubleshooting.md`.
