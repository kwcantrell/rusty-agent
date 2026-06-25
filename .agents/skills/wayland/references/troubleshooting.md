# Troubleshooting Wayland app automation

Symptom → cause → fix. Most failures are one of: the a11y bus isn't running, the
app didn't expose its widgets, input perms, or an XWayland mix-up.

## The app isn't in the AT-SPI tree at all

`Atspi.get_desktop(0)` doesn't list your app.

- **The accessibility bus isn't running.** In a normal desktop session it is; in
  CI / `dbus-run-session` you must launch it:
  `/usr/libexec/at-spi-bus-launcher --launch-immediately &`
  (path varies: `/usr/lib/at-spi2-core/at-spi-bus-launcher`).
- **Qt's bridge never activated.** It activates on demand when an AT client
  connects, but force it: `QT_ACCESSIBILITY=1`. Ensure `at-spi2-core` is
  installed (provides the bus and the Qt/GTK bridges' counterpart).
- **GTK app:** make sure `GTK_A11Y` is not set to `none`.
- **You connected before the app registered.** Poll: loop over
  `desktop.get_child_count()` with a timeout instead of checking once (the helper
  in `automation-atspi.md` does this).

## The app is there, but widgets are nameless / missing

Buttons show as `'' filler` / `'' push button`, or the field you want isn't
found.

- **You didn't set accessible names.** In QML set `Accessible.name` and
  `objectName` on every interactive element; in GTK set the accessible label.
  See `writing-qt-kirigami-apps.md` §3.
- **Custom `Item`/`MouseArea` instead of a real control.** Custom items expose no
  Action interface and often no node. Use real `Button`/`TextField`/`CheckBox`,
  or add `Accessible` attached properties (`Accessible.role`, `.name`, and an
  `onPressAction`).
- **Searching by translated text.** Display text changes per locale. Search by
  **accessible id** (`objectName`, exposed in `get_attributes()["id"]` /
  Selenium "accessibility id") instead.

## `do_action` / click does nothing

- The node has **no Action interface** (it's a label/container, not a control).
  Find the actual interactive descendant.
- Wrong action index/name. Enumerate: `for i in range(n.get_n_actions()):
  print(n.get_action_name(i))` and call the right one.
- The widget is **disabled** — check the `enabled` state before acting.

## `ydotool`: "failed to open /dev/uinput" / nothing happens

- **`ydotoold` not running**, or your client can't reach its socket. Start the
  daemon and point the client at it:
  `export YDOTOOL_SOCKET=/run/user/$(id -u)/.ydotool_socket`.
- **No permission on `/dev/uinput`.** Run the daemon as root, or add a udev rule
  granting a group access to the node and join it. In containers, expose the
  device (`--device /dev/uinput`).
- Right tool, wrong layer: if the target is a normal widget, prefer AT-SPI
  (Layer 1) — it needs none of this.

## `wtype` does nothing on KDE/GNOME

- `wtype` uses `zwp_virtual_keyboard_v1`, a **wlroots** protocol. KWin/Mutter
  support is inconsistent. Use AT-SPI `EditableText.set_text_contents()` or
  `ydotool` on KDE/GNOME instead.

## The RemoteDesktop portal blocks on a dialog (CI hangs)

- `Start` requests **user consent** by design. In CI there's no user.
  - Use **AT-SPI** (no input privilege) or **`ydotool`** (no portal) instead, or
  - run a **compositor you control** whose policy auto-approves, or
  - use the compositor's private no-prompt interface knowingly
    (`automation-input-injection.md` §1c).

## `kdotool` / KWin script: "no such method" or window not found

- **Plasma 5 vs 6 API drift:** `clientList()`→`windowList()`,
  `activeClient`→`activeWindow`. Match the running KWin version.
- **Matching the wrong identifier:** match on `app_id` (Wayland window class) or
  `caption` (title). Set `app_id` via `setDesktopFileName` +
  a `.desktop` file (see `writing-qt-kirigami-apps.md` §5).
- KWin scripting is **KDE-only** — there's no portable cross-compositor
  equivalent.

## `xdotool` / `xwininfo` / `xdpyinfo` don't see my app

- They're **X11** tools. A native Wayland client is invisible to them; they only
  see **XWayland** clients. This is expected — use AT-SPI / `kdotool`, not X11
  tools.
- If you *want* the app under XWayland for a legacy tool, force it:
  `QT_QPA_PLATFORM=xcb ./app` (then it shows in `xwininfo`) — but you lose native
  Wayland behavior; do this only to bridge old tooling.

## Qt picks XWayland when I wanted native Wayland (or vice-versa)

- Force the backend: `QT_QPA_PLATFORM=wayland` (native) or `=xcb` (XWayland).
- Confirm which you got: native Wayland windows do **not** appear in
  `xlsclients` / `xwininfo`.

## CI is flaky / races

- Almost always a **missing wait**. Don't `sleep` and hope:
  - wait for the compositor socket file (`$XDG_RUNTIME_DIR/wayland-*`) to exist,
  - then poll the a11y bus until the app's node appears,
  - then poll for the specific widget before acting.
- Set `XDG_RUNTIME_DIR` to a private `0700` dir owned by the run user; a missing
  or world-readable runtime dir makes both Wayland and the bus refuse to start.

## Screenshots are black / empty

- Headless compositors may not composite an unmapped/occluded surface. Ensure
  the app window is actually mapped (visible) on the virtual output.
- Prefer **toolkit-side capture** (`QQuickWindow::grabWindow()`) for
  deterministic pixels; it doesn't depend on the compositor's screencopy support.
- On KWin use `spectacle -b -n -o out.png` or the `Screenshot` portal; `grim`
  needs `wlr-screencopy`, which KWin does not implement.
