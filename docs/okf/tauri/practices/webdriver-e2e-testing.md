---
type: Practice
title: WebDriver end-to-end testing
description: Drive an assembled Tauri app through the W3C WebDriver protocol — choosing between the @wdio/tauri-service and driving tauri-driver directly, and working around the macOS WKWebView driver gap.
tags: [testing]
timestamp: 2026-07-09T00:00:00Z
---

# WebDriver end-to-end testing

WebDriver is the standardized, automated-testing interface for driving web
documents, and Tauri supports it for end-to-end testing of the assembled desktop
app [1][2]. Because tests interact through the standard W3C protocol, they require
*no modification to your Tauri application* — you drive the shipped UI as a user
would [3][4]. The decision that shapes everything else is which of two routes you
take: the WebdriverIO service, or driving `tauri-driver` directly.

## Default to the @wdio/tauri-service

The recommended way to use WebDriver with Tauri is WebdriverIO plus the
`@wdio/tauri-service`, which works on Windows, Linux, *and* macOS [2]. Maintained
under the WebdriverIO project, it provides Tauri API access via
`browser.tauri.execute()`, command (IPC) mocking, frontend/backend log capture,
and multiremote [2]. Scaffold with `npm create wdio@latest ./`, pick *Desktop
Testing*, and choose *Tauri*; a minimal config points at your release binary and
selects a driver provider [2]:

```ts
export const config: WebdriverIO.Config = {
  services: [['tauri', {
    appBinaryPath: './src-tauri/target/release/my-tauri-app',
    driverProvider: 'embedded',
  }]],
};
```

The service supports three driver providers [2][5]. The `embedded` provider
(default) runs a WebDriver server *inside* your app via
`tauri-plugin-wdio-webdriver`, needs no external driver on any platform, and is
how macOS is supported [2][5]. The `external` provider drives the platform's
native WebDriver through `tauri-driver` on Windows and Linux [2]. The `crabnebula`
provider uses CrabNebula's cross-platform fork — a commercial option requiring a
paid API key (which is what unlocks macOS on that route) [2][5]. Add
`tauri-plugin-wdio` for backend access — `browser.tauri.execute()`, IPC mocking,
and log capture [2]. For fast renderer-only tests there is also a browser mode
that runs the frontend in plain Chrome against a Vite dev server, intercepting
`invoke()` so you can mock commands with the same WDIO API — no binary, driver, or
plugin required [2].

## Drive tauri-driver directly only when you must

Reach past the service and drive `tauri-driver` directly if you are not using
Node.js, prefer Selenium, or are integrating WebDriver into a custom harness [3].
Accept the cost of doing so up front: driven directly, **only Windows and Linux
are supported on desktop, because macOS has no WKWebView driver tool** — use the
service's embedded server for macOS [2][3]. Install with `cargo install
tauri-driver --locked` [3]. On Linux the driver uses `WebKitWebDriver` (packaged
as `webkit2gtk-driver` on Debian-based distros — check with `which
WebKitWebDriver`) [3]. On Windows it uses Microsoft Edge Driver, and the Edge and
Edge Driver versions **must match** the Windows Edge version, or the suite hangs
while trying to connect — `msedgedriver-tool` downloads the matching driver, and
`tauri-driver` finds `msedgedriver.exe` on `$PATH` or via `--native-driver` [3].

## Selenium versus WebdriverIO on the manual route

On the direct-driver route, both frameworks talk to `tauri-driver` on
`127.0.0.1:4444` and need no app changes; pick by ecosystem fit. The Selenium
example uses Mocha and Chai, builds the app and starts `tauri-driver` in a
`before` hook, creates a session against the built binary, and tears the driver
down in `after` [4] — "we just enabled e2e testing without modifying our Tauri
application at all" [4]. The WebdriverIO example builds in debug in `onPrepare`,
spawns the driver in `beforeSession`, tears it down in `afterSession`, and points
`tauri:options.application` at `target/debug/tauri-app` [7]. Both docs still
recommend the service over hand-rolling this lifecycle for most projects [3][7].

## The macOS gap and the DIY escape hatch

The macOS gap is structural: Apple's WebDriver story is `safaridriver` for Safari
itself, which does not help when the UI is a WKWebView inside a desktop app [6].
The supported answers are the service's embedded server (free, cross-platform) or
CrabNebula's paid fork [2][5]. A community project, Tauri-WebDriver (`tauri-wd`),
independently implements the W3C protocol for macOS WKWebView with the same
in-app-plugin-plus-CLI shape as the service, and even ships MCP integration so AI
agents can drive the app [6] — useful context, but the first-party guidance for
macOS remains the embedded provider [2].

For running any of these under CI, see
[/practices/webdriver-ci.md](/practices/webdriver-ci.md).

# Citations

1. [Tests](/sources/tauri-tests-overview.md)
2. [WebDriver overview](/sources/tauri-webdriver-overview.md)
3. [Manual setup](/sources/tauri-webdriver-manual-setup.md)
4. [Selenium example](/sources/tauri-webdriver-selenium.md)
5. [WebdriverIO platform support](/sources/wdio-tauri-service.md)
6. [I Built a WebDriver for WKWebView Tauri Apps on macOS](/sources/wkwebview-driver-macos.md)
7. [WebdriverIO example](/sources/tauri-webdriver-webdriverio.md)
