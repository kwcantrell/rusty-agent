# Layer 2 (fallback): synthesizing input + managing windows

Use this **only** when the accessibility tree (Layer 1, `automation-atspi.md`)
can't express what you need: custom-painted surfaces, precise pointer gestures
(drag paths), or real OS-level input in headless CI. Three sanctioned options,
in order of "correctness":

1. **libei/EIS via the RemoteDesktop portal** — in-policy, no root, may prompt.
2. **`ydotool` via kernel uinput** — compositor-agnostic, needs device perms
   (root). Best for CI you control.
3. **A private compositor interface** (e.g. KWin's internal EIS D-Bus method) —
   no prompt, but non-portable and unstable. Use knowingly.

Plus **window management** (activate/move/resize/list) via KWin scripting /
`kdotool`, which is *not* input synthesis and has no Wayland equivalent of the
restriction.

---

## 1. In-policy input: the RemoteDesktop portal

The `org.freedesktop.portal.RemoteDesktop` interface of **xdg-desktop-portal**
is the sanctioned channel. Two ways to use it:

### (a) Portal `Notify*` methods directly (simplest, fully runnable)

The portal exposes `NotifyPointerMotion`, `NotifyPointerButton`,
`NotifyKeyboardKeycode`, etc. You: create a session, select devices, `Start` it
(this is where the **user consent dialog** appears), then call the notify
methods. Runnable Python over D-Bus:

```python
#!/usr/bin/env python3
"""Inject pointer+keyboard via the RemoteDesktop portal (in-policy)."""
from dbus_next.aio import MessageBus            # pip install dbus-next
from dbus_next import Variant
import asyncio

PORTAL = "org.freedesktop.portal.Desktop"
PATH   = "/org/freedesktop/portal/desktop"

async def main():
    bus = await MessageBus().connect()
    intro = await bus.introspect(PORTAL, PATH)
    obj = bus.get_proxy_object(PORTAL, PATH, intro)
    rd = obj.get_interface("org.freedesktop.portal.RemoteDesktop")

    # 1) CreateSession -> returns a request; the real result arrives on a
    #    Response signal. (Production code subscribes to the Request object's
    #    Response signal; condensed here.)
    await rd.call_create_session({
        "session_handle_token": Variant("s", "tok1"),
        "handle_token": Variant("s", "req1"),
    })
    # 2) SelectDevices: 1=keyboard, 2=pointer, 3=touchscreen (bitmask)
    #    -> SelectDevices(session_handle, {"types": Variant('u', 3), ...})
    # 3) Start(session_handle, parent_window, options)
    #    -> THIS triggers the user-consent dialog. After approval:
    # 4) NotifyPointerMotion(session, {}, dx, dy)
    #    NotifyPointerButton(session, {}, BTN_LEFT=0x110, state=1/0)
    #    NotifyKeyboardKeycode(session, {}, evdev_keycode, state=1/0)
    print("see comments: wire CreateSession/SelectDevices/Start responses, "
          "then call Notify* on the returned session handle")

asyncio.run(main())
```

Key facts:
- **`Start` shows a permission dialog** the first time (and the compositor may
  remember the choice). This is by design — see `architecture.md` §security.
- Keycodes are **evdev** codes (e.g. `KEY_A = 30`), not characters or X
  keysyms. Map characters via the XKB layout, or use `NotifyKeyboardKeysym`
  where available.
- Pointer motion is **relative** (`NotifyPointerMotion dx dy`); for absolute
  positioning use `NotifyPointerMotionAbsolute` with a stream node, or combine
  with Layer-1 `Component.get_position()` to compute deltas.

### (b) libei via `ConnectToEIS` (modern, higher-throughput)

Newer portals add **`ConnectToEIS`**, which returns a **file descriptor** from
the compositor. You wrap that fd in a **libei** sender context and emit events
through libei directly — after setup, input flows **straight between your client
and the compositor**, bypassing the portal processes (lower latency, full device
semantics). The **`liboeffis`** helper does the portal negotiation for you.

- C clients: link `libei` + `liboeffis`; call `oeffis_*` to get the fd, then
  `ei_setup_backend_fd()`.
- This is the path heavyweight tools (remote-desktop apps, sophisticated test
  harnesses) use. For most UI tests the portal `Notify*` methods (a) or
  `ydotool` (below) are simpler.

### (c) Private compositor interface (no prompt — use knowingly)

Some compositors expose an internal D-Bus EIS method that skips the portal
dialog — e.g. KDE's **`org.kde.KWin.EIS.RemoteDesktop`**. Tools aimed at fully
automated KDE sessions use it to get *zero authorization prompts*. Trade-off:
it's a **private, non-public API** that can change between releases and isn't
portable to other compositors. Reasonable for a CI image you control; avoid in
anything meant to be portable.

---

## 2. Out-of-policy input: `ydotool` (uinput) — the CI workhorse

`ydotool` creates a **virtual input device via the kernel `uinput`** facility,
*below* the compositor. Because it's kernel-level it's **protocol-agnostic**:
works on Wayland (any compositor), X11, the console, fbdev. That also means it
sits **outside** Wayland's per-app policy and needs **`/dev/uinput` access
(usually root)**.

```bash
sudo apt install ydotool          # ships ydotool + ydotoold
# Since v1.0.0 the daemon is mandatory:
sudo ydotoold &                   # or a systemd service; needs /dev/uinput
export YDOTOOL_SOCKET=/run/user/$(id -u)/.ydotool_socket   # match the daemon

ydotool mousemove --absolute 200 150
ydotool click 0xC0                # left press+release
ydotool type "Ada"
ydotool key 28:1 28:0             # KEY_ENTER down, up (evdev keycodes)
```

To avoid running as root, give a dedicated user access to `/dev/uinput`
(udev rule adding it to a group that owns the node). `ydotool` is the most
reliable choice for **headless CI** because there's no portal dialog and no
compositor cooperation required — see `headless-ci.md`.

> `wtype` is a lighter alternative that types via the `zwp_virtual_keyboard_v1`
> protocol (no root). It works on **wlroots** compositors; **KWin/Mutter support
> for that protocol is inconsistent**, so don't rely on it on KDE/GNOME — use
> AT-SPI text entry (Layer 1) or `ydotool` there.

---

## 3. Window management (not input): KWin scripting & `kdotool`

Activating, moving, resizing, and listing windows is **separate** from input
synthesis and is available on KDE without special privilege, via **KWin
scripting over D-Bus**.

### `kdotool` — an xdotool-shaped wrapper

`kdotool` provides familiar `xdotool`-style window commands on KDE Wayland. Under
the hood it **generates a KWin script on the fly, loads it into KWin via D-Bus,
runs it, then deletes it.** It does **window management only** — it explicitly
does **not** do keyboard/mouse emulation (it points you to `ydotool` / `dotool` /
`wtype` for that).

```bash
# find a window by title and activate it
win=$(kdotool search --name "Hello Wayland")
kdotool windowactivate "$win"
kdotool windowmove     "$win" 100 100
kdotool windowsize     "$win" 800 600
kdotool getactivewindow getwindowname
```

### Raw KWin scripting (no extra tool)

You can do the same by loading a tiny KWin script via D-Bus — useful when
`kdotool` isn't installed:

```bash
cat > /tmp/activate.js <<'JS'
const clients = workspace.windowList ? workspace.windowList() : workspace.clientList();
for (const c of clients) {
    if (c.caption && c.caption.indexOf("Hello Wayland") !== -1) {
        workspace.activeWindow = c;   // Plasma 6 API (was workspace.activeClient)
    }
}
JS
id=$(qdbus org.kde.KWin /Scripting loadScript /tmp/activate.js)
qdbus org.kde.KWin /Scripting/Script$id run
qdbus org.kde.KWin /Scripting/Script$id stop
```

> KWin's scripting API changed names between Plasma 5 and 6 (`clientList()` →
> `windowList()`, `activeClient` → `activeWindow`). Check the running version.

On GNOME/Mutter the analogous mechanism is Mutter/Shell D-Bus + GNOME Shell
JavaScript (`org.gnome.Shell.Eval`, often disabled outside dev mode) — there is
no single cross-compositor window-management tool.

---

## 4. Choosing within Layer 2

| Situation | Use |
|-----------|-----|
| Interactive desktop, occasional input | RemoteDesktop portal `Notify*` (§1a) |
| High-throughput / device-faithful input | libei + `ConnectToEIS` (§1b) |
| **Headless CI**, no user to click a dialog | `ydotool` (§2) or a compositor you own |
| Fully-automated KDE session, no prompts allowed | KWin private EIS iface (§1c), knowingly |
| Just need to focus/move/resize a window | `kdotool` / KWin scripting (§3) |

Always re-check whether Layer 1 (AT-SPI Action / EditableText) can do it first —
it's more robust than any of these.
