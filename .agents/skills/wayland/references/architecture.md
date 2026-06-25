# Wayland architecture & the automation security model

Read this once before doing non-trivial automation. It explains *why* the rest
of the skill is shaped the way it is.

## 1. The compositor is the display server

X11 split responsibilities across three processes: the **X server** (owns the
screen and input), a **window manager**, and a **compositor**. Any X client
could talk to the X server and, via `XTEST` / `XSendEvent` / `XQueryTree`,
inject input into and inspect *any* window. That is exactly what made `xdotool`
and old GUI-test tools possible — and exactly what made global keyloggers
trivial.

Wayland collapses all three roles into one process: **the compositor.** On KDE
that is **KWin**; on GNOME, **Mutter**; elsewhere, a **wlroots**-based
compositor (Sway, etc.). The compositor:

- owns the display hardware directly via **KMS** (Kernel Mode Setting),
- reads input devices directly via **evdev** / **libinput**,
- maintains a **scene graph** of client surfaces, and uses it to decide which
  surface an input event belongs to and to convert global screen coordinates
  into surface-local coordinates.

There is no separate "server" you can connect to and ask about other windows.
**A client sees only its own surfaces and the input the compositor routes to it.**

## 2. The protocol: requests and events

Wayland is an **asynchronous, object-oriented** protocol over a UNIX socket
(`$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY`, e.g. `wayland-0`):

- **Requests** — client → server. Each is a method invocation on a protocol
  **object**, identified by an integer **object ID**.
- **Events** — server → client. State changes, input, configuration.

You start from the singleton `wl_display`, get the `wl_registry`, and **bind**
the globals the compositor advertises:

- `wl_compositor` → creates `wl_surface`s (your drawable regions).
- `xdg_wm_base` → `xdg_surface` → `xdg_toplevel` (the **xdg-shell** protocol:
  application windows, titles, min/max, close).
- `wl_seat` → `wl_pointer` / `wl_keyboard` / `wl_touch` (input *you receive*).

You almost never write this by hand — a toolkit (Qt, GTK) does it for you. But
the model matters: there is **no protocol request** that means "give me another
app's window" or "type into the focused window globally." Those don't exist by
design.

## 3. The security model (the crux for automation)

Because the compositor owns everything and clients are isolated, these X11
capabilities are **deliberately absent** from core Wayland:

| X11 capability                  | Wayland status | Consequence for automation |
|---------------------------------|----------------|----------------------------|
| `XTEST` — inject input anywhere | removed        | No global "type/click" primitive |
| `XSendEvent` — fake events      | removed        | Can't forge events to other clients |
| `XQueryTree` — enumerate windows| removed        | No global window list from a client |
| Read other windows' pixels      | removed        | Screen capture is gated behind portals |

This is why `xdotool`, `xdpyinfo`, and similar tools **do not work** on native
Wayland clients (under XWayland they only see XWayland apps). It is also why
there is no single replacement — instead there are **scoped, permissioned
channels** for the things that used to be free-for-all.

### How sanctioned input synthesis works: libei/EIS

The modern, in-policy way to emulate input is the **EI (Emulated Input)** stack:

- **libei** — the *client* library: the thing that wants to send fake input.
- **libeis** — the *server* side: normally **the compositor itself**.
- **liboeffis** — a small helper that negotiates an EI connection through the
  **xdg-desktop-portal `RemoteDesktop`** interface.

The crucial design property: **emulated events remain distinguishable inside the
compositor.** The compositor knows an event was injected (and by whom), so it can
apply fine-grained policy — which device types, which client, when, and whether
to ask the user first. Contrast X11, where injected and real input were
indistinguishable. Peter Hutterer frames the three pillars as **separation**
(real vs emulated), **distinction** (the compositor can tell them apart), and
**control** (per-event policy). Ordinary clients still cannot tell the events
were emulated — only the compositor can.

Practically, an automation client either:

1. goes through the **RemoteDesktop portal** (in-policy; may show a permission
   dialog the first time), or
2. drops **below** the compositor via the kernel's **uinput** device
   (`ydotool`) — protocol-agnostic but outside Wayland's policy and needing
   device permissions (usually root), or
3. uses a **private compositor interface** (e.g. a KWin-internal D-Bus method) —
   convenient for kiosk/CI you control, but non-portable and unstable.

### How sanctioned screen capture works: portals

Reading pixels (screenshots, screen recording) goes through
**xdg-desktop-portal** `Screenshot` / `ScreenCast` (PipeWire) interfaces, again
with user consent. There is no "grab the whole screen from any client" call.

## 4. What this means for your test strategy

- **Don't fight the model with input injection.** Prefer the **accessibility
  tree (AT-SPI2)**: it's a *sanctioned, semantic* introspection channel that
  every major toolkit feeds, and it lets you find and invoke widgets without any
  global-input privilege. That's why this skill makes it Layer 1.
- **Use input synthesis as a targeted fallback**, through the portal when you can
  and uinput when you must.
- **For CI, run your own compositor instance** so *you* are the policy authority
  and there's no user to click a permission dialog. See
  `references/headless-ci.md`.

## 5. Glossary

- **KMS** — Kernel Mode Setting; how the compositor drives displays.
- **evdev / libinput** — kernel input events / the library that interprets them.
- **xdg-shell** — the protocol giving surfaces "window" semantics.
- **AT-SPI2** — Assistive Technology Service Provider Interface; the D-Bus
  accessibility bus. The semantic introspection channel for automation.
- **EI / libei / libeis / liboeffis** — the emulated-input stack and its portal
  helper.
- **xdg-desktop-portal** — the D-Bus service brokering permissioned access
  (RemoteDesktop input, ScreenCast capture, file chooser, …) for sandboxed and
  ordinary apps alike.
- **uinput** — the Linux kernel facility for creating virtual input devices;
  what `ydotool` uses to bypass Wayland entirely.
- **XWayland** — the X server that runs legacy X11 clients inside a Wayland
  session; X11 tools only see *these* clients, not native Wayland ones.
