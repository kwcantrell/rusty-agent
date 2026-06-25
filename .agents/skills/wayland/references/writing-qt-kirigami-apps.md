# Writing a Qt6 / QML / Kirigami app that is easy to automate

Lead stack for a native Wayland (KDE-aligned) GUI app:

- **Qt6** — the C++/QML framework Plasma itself is built on.
- **QML + Qt Quick Controls 2** — declarative UI language and standard controls.
- **Kirigami** — KDE's convergent component set, built on QML / Qt Quick
  Controls 2. Gives you `ApplicationWindow`, `Page`, `Action`, responsive
  layouts.

> GTK4 + libadwaita is the GNOME-aligned alternative. It also publishes its
> widget tree on AT-SPI2, so the automation half of this skill applies equally —
> the difference is only the code you write here. See §6.

The whole point of this page: **write the app so a test driver can find its
widgets by name.** Everything else is ordinary Qt.

## 1. Dependencies (Debian/Ubuntu/Kubuntu family)

```bash
sudo apt install \
  build-essential cmake \
  qt6-base-dev qt6-declarative-dev \
  qml6-module-qtquick qml6-module-qtquick-controls \
  qml6-module-qtquick-layouts qml6-module-qtquick-window \
  kf6-kirigami2-dev          # package name varies: kirigami2 / libkf6kirigami
# Accessibility bus (needed to TEST the app — see automation-atspi.md):
sudo apt install at-spi2-core
```

On Fedora: `qt6-qtbase-devel qt6-qtdeclarative-devel kf6-kirigami-devel
at-spi2-core`. On Arch: `qt6-base qt6-declarative kirigami at-spi2-core`.

## 2. A complete, runnable app

Three files. It has a text field, a button, and a label — concrete targets for
the automation examples in `automation-atspi.md`.

**`CMakeLists.txt`**

```cmake
cmake_minimum_required(VERSION 3.20)
project(HelloWayland LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_AUTOMOC ON)

find_package(Qt6 REQUIRED COMPONENTS Core Gui Qml Quick)
qt_standard_project_setup(REQUIRES 6.5)

qt_add_executable(hello-wayland main.cpp)
qt_add_qml_module(hello-wayland
    URI HelloWayland
    VERSION 1.0
    QML_FILES Main.qml
)
target_link_libraries(hello-wayland PRIVATE Qt6::Quick)
```

**`main.cpp`**

```cpp
#include <QGuiApplication>
#include <QQmlApplicationEngine>

int main(int argc, char *argv[])
{
    QGuiApplication app(argc, argv);
    app.setApplicationName("HelloWayland");      // shows up as the AT-SPI app name
    QQmlApplicationEngine engine;
    engine.loadFromModule("HelloWayland", "Main");
    if (engine.rootObjects().isEmpty())
        return -1;
    return app.exec();
}
```

**`Main.qml`**

```qml
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ApplicationWindow {
    id: root
    title: "Hello Wayland"
    width: 400; height: 200
    visible: true

    // objectName -> stable AT-SPI "accessible id"; Accessible.name -> the label
    // a driver searches by. Set BOTH on every interactive element.
    pageStack.initialPage: Kirigami.Page {
        ColumnLayout {
            anchors.centerIn: parent
            spacing: Kirigami.Units.largeSpacing

            TextField {
                id: nameField
                objectName: "nameField"
                Accessible.name: "name input"
                Accessible.role: Accessible.EditableText
                placeholderText: "Type your name"
                Layout.preferredWidth: 250
            }

            Button {
                objectName: "greetButton"
                Accessible.name: "Greet"
                text: "Greet"
                Layout.alignment: Qt.AlignHCenter
                onClicked: greeting.text = "Hello, " + nameField.text + "!"
            }

            Label {
                id: greeting
                objectName: "greetingLabel"
                Accessible.name: "greeting"
                text: ""
            }
        }
    }
}
```

**Build & run (inside a Wayland session):**

```bash
cmake -B build -S . && cmake --build build
QT_QPA_PLATFORM=wayland ./build/hello-wayland
```

`QT_QPA_PLATFORM=wayland` forces the native Wayland backend (Qt may otherwise
pick XWayland). To confirm you're native, not XWayland: the window will *not*
appear in `xwininfo`/`xlsclients`.

### Even faster: pure-QML prototype

For a throwaway target you can skip C++/CMake entirely:

```bash
# Save the Main.qml body (without the module import line tweaks) and run:
QT_QPA_PLATFORM=wayland qml6 Main.qml
```

## 3. The testability rules (do not skip)

A driver finds widgets through **AT-SPI2**, which mirrors Qt's accessibility
tree. You control what shows up there:

1. **Set `Accessible.name` on every interactive element.** This is the human-
   readable name a driver searches by (`"Greet"`, `"name input"`). Controls with
   visible `text` get a name for free, but set it explicitly for icon-only
   buttons, fields, and custom items — never rely on coordinates.
2. **Set `objectName`** too. `selenium-webdriver-at-spi` exposes it as the
   element's **accessible id** — the most stable selector, independent of
   translation. (Display text changes per language; `objectName` doesn't.)
3. **Set `Accessible.role`** when it isn't obvious (e.g. a custom `Item` acting
   as a button → `Accessible.role: Accessible.Button`). The role lets a driver
   filter "find me the *button* named X".
4. **Expose state** through standard properties: `Accessible.checked`,
   `enabled`, `Accessible.description`. Drivers assert on these.
5. **Prefer real controls** (`Button`, `CheckBox`, `TextField`) over custom
   `MouseArea`-driven `Item`s. Real controls publish an **Action** interface, so
   a driver can *invoke* them semantically (no coordinate click needed). Custom
   items expose nothing unless you add `Accessible` attached properties.

### Verify your widgets are visible to automation

With the app running, dump the accessibility tree (install `at-spi2-core`):

```bash
# 'accerciser' is the GUI inspector; for a quick CLI check use Python:
python3 - <<'PY'
import gi; gi.require_version("Atspi", "2.0")
from gi.repository import Atspi
Atspi.init()
d = Atspi.get_desktop(0)
for i in range(d.get_child_count()):
    app = d.get_child_at_index(i)
    if app and app.get_name() == "HelloWayland":
        def walk(n, depth=0):
            print("  "*depth, repr(n.get_name()), n.get_role_name())
            for j in range(n.get_child_count()):
                walk(n.get_child_at_index(j), depth+1)
        walk(app)
PY
```

If your button shows as `'Greet' push button` — it's drivable. If it shows as
`'' filler` or is missing, fix the rules above. See `troubleshooting.md` if the
whole app is absent from the tree.

## 4. Activate accessibility for the app

Qt's AT-SPI bridge activates **on demand** when an assistive client connects, so
usually nothing is required. If your widgets don't appear, force it:

```bash
QT_ACCESSIBILITY=1 QT_QPA_PLATFORM=wayland ./build/hello-wayland
```

and ensure the a11y bus is running (it is, inside a normal desktop session;
under `dbus-run-session` in CI you must launch it — see `headless-ci.md`).

## 5. Window identity for window-management tools

Set these so tools like `kdotool` / KWin scripting can target the window:

- `QGuiApplication::setDesktopFileName("org.example.HelloWayland")` and ship a
  matching `.desktop` file — this drives the `app_id` (Wayland's window class).
- A clear window `title`.

`app_id` is the Wayland equivalent of the X11 `WM_CLASS`; KWin scripts match on
it.

## 6. GTK4 note (the alternative)

If you write GTK4 instead, the automation half is unchanged — set accessible
names so AT-SPI sees them:

```c
GtkWidget *button = gtk_button_new_with_label("Greet");
gtk_accessible_update_property(GTK_ACCESSIBLE(button),
    GTK_ACCESSIBLE_PROPERTY_LABEL, "Greet", -1);
```

GTK4 has accessibility built in (no separate `at-spi2-atk` bridge as in GTK3).
Everything in `automation-atspi.md` works against GTK4 apps too.
