---
type: Practice
title: Running WebDriver tests in CI
description: Configure GitHub Actions to run tauri-driver WebDriver tests across Linux and Windows — installing webkit2gtk-driver and a synced Edge driver, and creating a fake display with xvfb for headless Linux runs.
tags: [testing]
timestamp: 2026-07-09T00:00:00Z
---

# Running WebDriver tests in CI

WebDriver tests with `tauri-driver` run fine in continuous integration, and the
canonical setup is a GitHub Actions workflow reusing the WebdriverIO example from
the testing docs [1]. The practice is a per-platform matrix: each OS needs its own
native WebDriver installed, and Linux additionally needs a virtual display. Run
the matrix with `fail-fast: false` so one platform's failure does not cancel the
others [1].

## Linux: install webkit2gtk-driver and fake a display with xvfb

On Linux the driver is `WebKitWebDriver`, packaged as `webkit2gtk-driver` on
Debian/Ubuntu — install it before running tests [1][2]. CI runners have no
graphical display, so wrap the test command in `xvfb` (X Virtual Framebuffer),
which provides display emulation without a real GUI [1][3]. The tests "are
executed on Linux by creating a fake display" [1]:

```yaml
- name: install dependencies (linux)
  if: runner.os == 'Linux'
  run: sudo apt-get update && sudo apt-get install -y webkit2gtk-driver xvfb
- name: run tests
  run: xvfb-run -a yarn test
```

`xvfb-run -a` picks a free display number automatically. WebKitWebDriver support
is solid on Debian/Ubuntu, Fedora, and Arch; some distros (CentOS Stream,
openSUSE) lack adequate packaging, and Alpine works only as a runtime container,
not for building — pick your runner image accordingly [2].

## Windows: keep the Edge driver in sync

On Windows the driver is Microsoft Edge Driver, and its version must match the
runner's Edge/WebView2 version or the suite hangs on connect [4][2]. Version
mismatch is the most common cause of Windows WebDriver failures, so install the
driver as a CI step rather than assuming a preinstalled one — `msedgedriver-tool`
downloads the matching driver [1][4]. On the service route, `autoDownloadEdgeDriver:
true` handles version management for you [2]. Windows needs no display emulation;
run tests directly without `xvfb` [1].

## Full workflow shape

Beyond the platform-specific driver and display steps, the workflow checks out the
code, configures a cached Rust toolchain, installs Node (the example pins v24) with
cached dependencies, installs `tauri-driver`, and coordinates the Tauri build with
the test run [1]. Cache Rust and Node dependencies to keep runs fast [1]. A
minimal cross-platform skeleton:

```yaml
strategy:
  fail-fast: false
  matrix:
    os: [ubuntu-latest, windows-latest]
runs-on: ${{ matrix.os }}
steps:
  - uses: actions/checkout@v4
  - name: install dependencies (linux)
    if: runner.os == 'Linux'
    run: sudo apt-get install -y webkit2gtk-driver xvfb
  - name: install edge driver (windows)
    if: runner.os == 'Windows'
    run: cargo install --git https://github.com/chippers/msedgedriver-tool
  - uses: dtolnay/rust-toolchain@stable
  - uses: actions/setup-node@v4
    with: { node-version: '24' }
  - run: yarn install
  - run: cargo install tauri-driver --locked
  - name: run tests
    run: ${{ runner.os == 'Linux' && 'xvfb-run -a' || '' }} yarn test
```

The WebDriver example repositories ship their own CI scripts, so those are worth
copying as a starting point; the guide above explains the *why* behind each step
[4]. For the test-authoring side of this — the service versus direct-driver
choice and the macOS gap — see
[/practices/webdriver-e2e-testing.md](/practices/webdriver-e2e-testing.md).

# Citations

1. [Continuous Integration](/sources/tauri-webdriver-ci.md)
2. [WebdriverIO platform support](/sources/wdio-tauri-service.md)
3. [Tests](/sources/tauri-tests-overview.md)
4. [Manual setup](/sources/tauri-webdriver-manual-setup.md)
