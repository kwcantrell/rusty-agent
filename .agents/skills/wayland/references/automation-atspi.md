# Layer 1 (primary): driving apps via the AT-SPI2 accessibility bus

This is the **preferred** way to automate a Wayland GUI app. AT-SPI2 is a
D-Bus-based accessibility bus that every major toolkit (Qt, GTK) feeds. It lets
you find widgets by **name and role**, **invoke** them semantically, and **read
their state** for assertions — all without any global-input privilege and
without caring about window position, theme, or DPI.

There are two client styles. Pick by need:

- **`pyatspi` / `gi.repository.Atspi` (Python)** — direct, dependency-light,
  great for custom harnesses. Start here.
- **`selenium-webdriver-at-spi` (KDE)** — an Appium/Selenium **WebDriver**. Use
  it if you want Selenium/Appium ergonomics, existing WebDriver tooling, or CI
  integration. This is what KDE itself uses to test its apps.

## 0. Prerequisites

```bash
sudo apt install at-spi2-core python3-gi gir1.2-atspi-2.0   # Debian/Ubuntu
```

Accessibility must be **enabled** so toolkits populate the bus:

- Inside a normal KDE/GNOME session it's typically already on.
- Qt activates its bridge when an AT client connects; force with
  `QT_ACCESSIBILITY=1`.
- GTK: `GTK_A11Y` must not be `none`; `at-spi2-core` running.
- In headless CI you launch the a11y bus yourself — see `headless-ci.md`.

Quick sanity check that the bus is alive and your app is on it:

```bash
python3 -c 'import gi; gi.require_version("Atspi","2.0"); \
from gi.repository import Atspi; Atspi.init(); \
d=Atspi.get_desktop(0); \
print([d.get_child_at_index(i).get_name() for i in range(d.get_child_count())])'
```

Your app's name (e.g. `HelloWayland`) should be in the printed list.

## 1. Concepts: the AT-SPI object model

- **Desktop** → **Applications** → **Accessible** node tree (windows, panels,
  buttons, text fields…). Each node has a **name**, a **role**
  (`push button`, `text`, `label`, …), and **states** (`enabled`, `focused`,
  `checked`, …).
- Nodes implement **interfaces** that expose *capabilities*:
  - **Action** — `do_action()` to invoke (click a button, toggle a checkbox)
    **without coordinates**. This is the robust path.
  - **Text** / **EditableText** — read text, `set_text_contents()` to type.
  - **Value** — sliders/spinboxes.
  - **Component** — geometry (`get_position`, `get_extents`) — used only when you
    must fall back to coordinate input (Layer 2).

The golden rule: **invoke through Action / EditableText, not coordinates.** A
click via Action is layout-independent; a click via coordinates is brittle.

## 2. A complete runnable driver (`gi.repository.Atspi`)

Drives the example app from `writing-qt-kirigami-apps.md`: type a name, click
Greet, assert the label updates.

```python
#!/usr/bin/env python3
"""Drive a Wayland GUI app over AT-SPI2. No coordinates, no input injection."""
import sys, time
import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

Atspi.init()

def find_app(name, timeout=10):
    desktop = Atspi.get_desktop(0)
    deadline = time.time() + timeout
    while time.time() < deadline:
        for i in range(desktop.get_child_count()):
            app = desktop.get_child_at_index(i)
            if app and app.get_name() == name:
                return app
        time.sleep(0.2)
    raise RuntimeError(f"app {name!r} not found on the a11y bus")

def find(node, *, name=None, role=None, obj_id=None, timeout=10):
    """Depth-first search for the first matching descendant."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        stack = [node]
        while stack:
            n = stack.pop()
            ok = True
            if name is not None and n.get_name() != name:
                ok = False
            if role is not None and n.get_role_name() != role:
                ok = False
            if obj_id is not None:
                attrs = n.get_attributes() or {}
                # Qt exposes objectName here under "id" (a.k.a. accessible id)
                if attrs.get("id") != obj_id:
                    ok = False
            if ok and node is not n:
                return n
            for i in range(n.get_child_count()):
                c = n.get_child_at_index(i)
                if c is not None:
                    stack.append(c)
        time.sleep(0.2)
    raise RuntimeError(f"no node name={name} role={role} id={obj_id}")

def do_default_action(node):
    """Invoke the widget's primary action (semantic 'click')."""
    action = node.get_action_iface() if hasattr(node, "get_action_iface") else node
    n_actions = action.get_n_actions()
    for i in range(n_actions):
        if action.get_action_name(i) in ("click", "press", "activate", "Press"):
            action.do_action(i); return
    action.do_action(0)   # fall back to the first action

def type_text(node, text):
    et = node.get_editable_text_iface() if hasattr(node, "get_editable_text_iface") else node
    et.set_text_contents(text)

def main():
    app = find_app("HelloWayland")
    field = find(app, role="text")              # the TextField
    button = find(app, name="Greet", role="push button")
    label  = find(app, role="label", name="greeting")

    type_text(field, "Ada")
    do_default_action(button)
    time.sleep(0.3)                              # let the UI update

    # read back for the assertion
    got = label.get_text(0, -1) if hasattr(label, "get_text") else label.get_name()
    assert got == "Hello, Ada!", f"expected greeting, got {got!r}"
    print("PASS:", got)

if __name__ == "__main__":
    sys.exit(main())
```

Run it while the app is up:

```bash
python3 driver.py
```

Notes:
- The exact accessor names (`get_action_iface`, `get_editable_text_iface`) vary
  slightly across `Atspi` GI versions; if one is missing, the interface methods
  are also exposed directly on the node (`node.do_action(i)`,
  `node.set_text_contents(...)`). The helper above tries both.
- `get_attributes()["id"]` is how Qt surfaces `objectName` — the most stable,
  translation-proof selector. Prefer it over visible-text name when you can.

## 3. The WebDriver path: `selenium-webdriver-at-spi`

KDE's `selenium-webdriver-at-spi` is a **WebDriver** server speaking the Appium /
Selenium protocol but acting on the **AT-SPI2** tree instead of a browser DOM.
Use it for Selenium-style tests, language-agnostic clients, and CI. It supports
Qt5/Qt6 and GTK3/GTK4 apps, and can run the app inside a localhost webserver for
non-GUI/CI environments as well as locally in KWin.

### Run the server

```bash
# Build/install from KDE's repo (https://invent.kde.org / github.com/KDE/...).
# It ships an `inputsynth` helper and starts a WebDriver endpoint, by default:
#   http://127.0.0.1:4723
selenium-webdriver-at-spi-run   # name varies by package; see its README
```

### A Python Selenium client

```python
from appium import webdriver                       # pip install Appium-Python-Client
from appium.options.common.base import AppiumOptions
from selenium.webdriver.common.by import By

opts = AppiumOptions()
# launch the app under test by its executable or .desktop app id:
opts.set_capability("app", "/path/to/build/hello-wayland")
opts.set_capability("platformName", "Linux")
opts.set_capability("automationName", "AT-SPI")

driver = webdriver.Remote("http://127.0.0.1:4723", options=opts)
try:
    # Most stable: locate by accessible id (Qt objectName)
    driver.find_element(By.NAME, "name input").send_keys("Ada")
    driver.find_element(By.NAME, "Greet").click()
    label = driver.find_element(By.NAME, "greeting")
    assert label.text == "Hello, Ada!", label.text
    print("PASS")
finally:
    driver.quit()
```

Selector strategies map onto AT-SPI:

| Selenium `By`        | AT-SPI meaning |
|----------------------|----------------|
| `By.NAME`            | accessible **name** (`Accessible.name`) |
| `"accessibility id"` | accessible **id** (Qt `objectName`) — most stable |
| `By.XPATH`           | path over the accessible-role tree |
| `By.CLASS_NAME`      | accessible **role** name |

Prefer accessibility id, then name. Avoid XPath-by-position — it's the brittle
equivalent of coordinates.

## 4. What AT-SPI can and can't do

**Can:** find by name/role/id, click/toggle/activate via Action, read & set text,
read values and states (enabled/checked/focused), read geometry, wait for nodes
to appear/disappear.

**Can't (drop to Layer 2 — `automation-input-injection.md`):**
- Drive **custom-painted** surfaces with no accessible nodes (canvases, games,
  some OpenGL/QtQuick `Canvas` content). *Fix the app first if you own it* —
  add `Accessible` attached properties.
- Precise **pointer gestures** (drag-and-drop paths, hover-to-reveal, kinetic
  scroll) that aren't a single semantic action.
- Real **OS-level keyboard focus** edge cases (global shortcuts, grabs).

## 5. dogtail — know the trap

`dogtail` is a popular Python AT-SPI2 automation framework, and dogtail 1.0+ is
"Wayland-enabled." **But its Wayland input path uses gnome-ponytail-daemon and
the Mutter ScreenCast/RemoteDesktop API — it is GNOME/Mutter-only and does NOT
work on KDE/KWin.** On KWin its *introspection* (reading the tree) works, but its
*input* will fail. **On KDE, use `selenium-webdriver-at-spi` or the `Atspi`
Action-based approach above instead.** Don't reach for dogtail expecting full
Wayland support on Plasma.
