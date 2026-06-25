# Design: generic `wayland` skill (write + automate GUI apps)

**Date:** 2026-06-24

## Goal

A single, portable skill — `.agents/skills/wayland/` — that any agent can use
to (1) write a native Wayland GUI application and (2) build a client that drives
that application programmatically to automate UI testing. The skill must be
**generic**: no references to any particular host repository, project, or
private tooling. The reference environment is a modern Wayland desktop
(e.g. Kubuntu 26.04 LTS / KDE Plasma 6.6 / KWin), but guidance generalizes.

## Decisions (from brainstorming)

- **Structure:** one combined skill (not split), hub `SKILL.md` + `references/`.
- **App toolkit:** Qt6 / QML / Kirigami is the lead stack (KDE-native, best
  AT-SPI exposure). GTK4 noted as the alternative.
- **Automation primary path:** AT-SPI2 accessibility-tree driving (semantic
  find + invoke). libei/EIS via the RemoteDesktop portal is the in-policy input
  layer; ydotool is the headless/CI fallback; kdotool / KWin scripting handle
  window management.
- **Depth:** runnable code examples embedded in `references/`.

## Why this shape

Wayland deliberately removed the global X11 input/inspection APIs
(XTEST / XSendEvent / XQueryTree) that xdotool relied on. There is no
drop-in replacement, so robust automation is **two layers**:

1. **Find & assert** semantically through the accessibility bus (AT-SPI2).
   This is layout-resilient and is how `selenium-webdriver-at-spi` works.
2. **Synthesize input** only when needed, through an in-policy channel
   (libei/EIS via the RemoteDesktop portal) or a kernel-level bypass
   (ydotool/uinput) for headless CI.

The app-writing half exists mainly to make the app *instrumentable*: an app
that sets `objectName` / `Accessible.name` is trivial to drive; one that
doesn't forces brittle coordinate-based tests.

## File layout

```
.agents/skills/wayland/
├── SKILL.md                              # hub: architecture + decision table
└── references/
    ├── architecture.md                   # compositor model, security, why X11 tools fail
    ├── writing-qt-kirigami-apps.md       # runnable Qt6/QML/Kirigami app + testability rules
    ├── automation-atspi.md               # PRIMARY: AT-SPI2 driving (pyatspi + selenium-webdriver-at-spi)
    ├── automation-input-injection.md     # libei/EIS portal, ydotool, kdotool/KWin scripting
    ├── headless-ci.md                    # nested/virtual compositor, dbus-run-session, screenshots, CI
    └── troubleshooting.md                # a11y tree gaps, uinput perms, portal prompts, XWayland
```

## Non-goals

- Not a general Linux desktop or X11 guide.
- Not training/packaging/distribution.
- Not specific to any one compositor's private D-Bus API (those are noted as
  optional accelerators, not the recommended path).
