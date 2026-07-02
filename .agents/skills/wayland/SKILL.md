---
name: wayland
description: >-
  Use when writing a native Wayland GUI application AND/OR building a client that
  drives a GUI app programmatically to automate UI testing on a Wayland desktop
  (KDE Plasma / KWin, GNOME / Mutter, or wlroots). Covers the Wayland
  client/compositor architecture, writing apps in Qt6 / QML / Kirigami (GTK4
  noted), and the two-layer automation model: semantic driving via the AT-SPI2
  accessibility bus (selenium-webdriver-at-spi / pyatspi — the primary path),
  in-policy input synthesis via libei/EIS through the xdg-desktop-portal
  RemoteDesktop interface, the ydotool/uinput fallback, kdotool / KWin scripting
  for window management, and headless/CI execution under a nested or virtual
  compositor. Trigger on mentions of Wayland, KWin, AT-SPI / accessibility-based
  UI testing, libei/libeis, xdg-desktop-portal RemoteDesktop, ydotool, wtype,
  kdotool, Appium on Linux desktop, Kirigami/QML apps, "automate a Linux GUI app
  on Wayland", or "why doesn't xdotool work on Wayland".
---

# Writing and automating Wayland GUI applications

This skill covers two linked jobs on a modern Wayland desktop:

1. **Writing** a native GUI app (lead stack: **Qt6 / QML / Kirigami**).
2. **Driving that app programmatically** to automate UI tests.

The two are linked: how you write the app determines how easily it can be
driven. An app that labels its widgets for the accessibility bus is trivial to
test; one that doesn't forces brittle pixel-coordinate scripts.

**Do not** use this skill for THIS repo's own desktop app — load
`auto-drive-tauri` instead and drive its WebSocket bridge, not the GUI. Also
not for X11-only automation (there `xdotool` still works; this skill exists
for Wayland's constraints).

This file is the hub. Depth lives in `references/` — pull in only the file the
current task needs (decision table in §5).

## 1. The one thing to understand first

Under Wayland **the compositor *is* the display server** (KWin on KDE, Mutter on
GNOME, or a wlroots compositor). It owns the screen and all input devices. A
client can only see and touch **its own** surfaces. Wayland deliberately dropped
the global X11 APIs that `xdotool` used — `XTEST` (inject input anywhere),
`XSendEvent`, and `XQueryTree` (enumerate all windows). **There is no drop-in
replacement.** This single fact explains why Wayland UI automation is a layered,
"fragmented" ecosystem instead of one magic tool. Read `references/architecture.md`
before doing anything non-trivial.

## 2. Automate in two layers (this is the key mental model)

```
                ┌─────────────────────────────────────────┐
   LAYER 1      │  FIND & ASSERT  — semantic, robust       │
  (primary)     │  AT-SPI2 accessibility tree              │   ← start here
                │  selenium-webdriver-at-spi / pyatspi     │
                │  "click the button named 'Save'"         │
                └─────────────────────────────────────────┘
                                  │  falls through to ↓ only when needed
                ┌─────────────────────────────────────────┐
   LAYER 2      │  SYNTHESIZE INPUT — coordinates/keys     │
  (fallback)    │  in-policy: libei/EIS via RemoteDesktop  │
                │  portal;  fallback: ydotool (uinput)     │
                │  "move pointer to (x,y), press Enter"    │
                └─────────────────────────────────────────┘
   window mgmt: kdotool / KWin scripting (activate/move/resize — NOT input)
```

**Always prefer Layer 1.** AT-SPI2 finds elements by *name and role*, so tests
survive theme changes, window moves, and DPI scaling. Drop to Layer 2 only for
things the accessibility tree can't express (custom-drawn canvases, drag paths,
games) or when you need real OS-level input in headless CI.

## 3. Writing the app (make it drivable)

Use **Qt6 / QML / Kirigami** for a KDE-native app (Qt is what Plasma itself is
built in); **GTK4 + libadwaita** is the GNOME-aligned alternative. Both publish
their widget tree on **AT-SPI2** automatically, so both are drivable by Layer 1.

The single most important testability rule: **give every interactive element a
stable accessible name.** In QML set `objectName` and `Accessible.name`; in GTK
set the accessible label / `gtk_accessible_*`. Without this, your driver can
only find elements by fragile screen coordinates. See
`references/writing-qt-kirigami-apps.md` for a complete runnable app.

## 4. The minimal end-to-end loop

1. Write the app and **name its widgets** (`references/writing-qt-kirigami-apps.md`).
2. Bring up an accessibility-capable session — locally or, for CI, a nested /
   virtual compositor under `dbus-run-session` (`references/headless-ci.md`).
3. Connect a Layer-1 driver, find elements by name, invoke actions, read text
   back for assertions (`references/automation-atspi.md`).
4. Only if a step can't be expressed semantically, synthesize input via Layer 2
   (`references/automation-input-injection.md`).

## 5. Reference decision table

| You need to…                                              | Read |
|-----------------------------------------------------------|------|
| Understand the compositor model / why xdotool is gone     | `references/architecture.md` |
| Write a Qt6/QML/Kirigami app that's easy to test          | `references/writing-qt-kirigami-apps.md` |
| Drive the app by accessibility (find, click, assert)      | `references/automation-atspi.md` |
| Inject raw pointer/keyboard input (in-policy or fallback) | `references/automation-input-injection.md` |
| Manage windows (activate/move/resize/list)                | `references/automation-input-injection.md` (KWin scripting / kdotool) |
| Run + drive + screenshot with no monitor (CI)             | `references/headless-ci.md` |
| Fix "app not in a11y tree" / uinput perms / portal prompts| `references/troubleshooting.md` |

## 6. Quick toolkit reference

| Tool / library            | Layer | Role | Needs root? |
|---------------------------|-------|------|-------------|
| AT-SPI2 (`Atspi`/`pyatspi`) | 1 | Find elements, invoke actions, read state | no |
| `selenium-webdriver-at-spi` | 1 | Appium/Selenium WebDriver over AT-SPI2 | no |
| libei / libeis / liboeffis  | 2 | In-policy input via RemoteDesktop portal | no |
| `ydotool` (+`ydotoold`)     | 2 | Input via kernel uinput (compositor-agnostic) | yes (uinput) |
| `wtype`                     | 2 | Type text via `zwp_virtual_keyboard` (wlroots; KWin support varies) | no |
| `kdotool`                   | — | Window mgmt via KWin scripting (no input) | no |
| KWin scripting (D-Bus)      | — | Window mgmt / introspection on KDE | no |

Anything that "just injects input anywhere" without a portal prompt or root is
either a private compositor interface or a misunderstanding — see
`references/architecture.md` §security.
