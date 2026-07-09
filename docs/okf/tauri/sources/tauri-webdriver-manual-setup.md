---
type: Source
title: "Manual setup"
description: "Manual tauri-driver WebDriver setup for Windows and Linux"
resource: https://v2.tauri.app/develop/tests/webdriver/manual-setup/
tags: [testing]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---
# Summary

Manual setup
This page covers driving
tauri-driver
directly, without the
@wdio/tauri-service
. Reach for it if you are not
using Node.js, prefer
Selenium
, or are integrating WebDriver into a custom test harness. For most projects the service
is the easier path — it automates everything below and additionally supports macOS. See the
WebDriver overview
to get
started with it.
When driving
tauri-driver
directly, only Windows and Linux are supported on desktop, as macOS has no WKWebView driver
tool available. iOS and Android work through Appium 2, but the process is not currently streamlined.
System Dependencies
Section titled “System Dependencies”
Install the latest
tauri-driver
or update an existing installation by running:
Terminal window
cargo
install
tauri-driver
--locked
Because we currently utilize the platform’s native
WebDriver
server, there are some requirements for running
tauri-driver
on supported platforms.
Linux
Section titled “Linux”
We use
WebKitWebDriver
on Linux platforms. Check if this binary exists already by running the
which WebKitWebDriver
command as
some distributions bundle it with the regular WebKit package. Other platforms may have a separate package for them, such
as
webkit2gtk-driver
on Debian-based distributions.
Windows
Section titled “Windows”
Make sure to grab the version of
Microsoft Edge Driver
that matches your Windows Edge version that the application is
being built and tested on. This should almost always be the latest stable version on up-to-date Windows installs. If the
two versions do not match, you may experience your WebDriver testing suite hanging while trying to connect.
You can use the
msedgedriver-tool
to download the appropriate Microsoft Edge Driver:
Terminal window
cargo install
--
git https:
//
github.com
/
chippers
/
msedgedriver
-
tool
&
"
$HOME
/.cargo/bin/msedgedriver-tool.exe
"
The download contains a binary called
msedgedriver.exe
.
tauri-driver
looks for that binary in the
$PATH
so make
sure it’s either available on the path or use the
--native-driver
option on
tauri-driver
. You may want to download this automatically as part of the CI setup process to ensure the Edge, and Edge Driver versions
stay in sync on Windows CI machines. A guide on how to do this may be added at a later date.
Example Applications
Section titled “Example Applications”
Below are step-by-step guides to show how to create a minimal example application that
is tested with WebDriver.
If you prefer to see the result of the guide and look over a finished minimal codebase that utilizes it, you
can look at
https://github.com/tauri-apps/webdriver-example
.
Selenium
WebdriverIO
Continuous Integration (CI)
Section titled “Continuous Integration (CI)”
The above examples also comes with a CI script to test with GitHub Actions, but you may still be interested in the below WebDriver CI guide as it explains the concept a bit more.
Continuous Integration (CI)
Edit page
Last updated:
Jun 29, 2026
Previous
Overview
Next
Continuous Integration
Support on Open Collective
Sponsor on GitHub
© 2026 Tauri Contributors. CC-BY / MIT
