---
type: Comparison
title: "Mock IPC vs WebDriver: unit-testing the frontend vs end-to-end testing"
description: "What frontend mock-IPC unit tests catch versus what WebDriver E2E tests catch, their cost and platform coverage including the macOS WebDriver gap, and why serious apps need both."
tags: [testing, ipc-architecture]
timestamp: 2026-07-09T00:00:00Z
---

Tauri supports two testing strategies that sit at different levels of the stack: unit and integration tests against a mock runtime, and end-to-end tests driven over the WebDriver protocol [1]. They catch different failures at different costs, and the sources treat them as complementary rather than substitutes [1].

Under the mock runtime, native webview libraries are not executed [1].

## Mock IPC: fast frontend unit tests

When you test the frontend in isolation, there is no real Tauri backend, so the `@tauri-apps/api/mocks` module lets you fake the Tauri environment and intercept IPC calls [2]. The core tool is `mockIPC`, which intercepts every IPC request with a handler you supply — enough to "ensure the correct backend calls are made" and to "simulate different results from backend functions" [2]. Under the mock runtime, native webview libraries are never executed [1].

What this buys you:

- **Assert on backend calls.** Combine `mockIPC` with a spy (`vi.spyOn` on `window.__TAURI_INTERNALS__.invoke`) to check a command was invoked, and with what arguments — for example asserting `invoke("add", { a: 12, b: 15 })` resolves to `27` [2].
- **Simulate results and failures** by returning values or promises from the handler, so the frontend's branches are exercised without a running Rust side [2].
- **Fake windows** with `mockWindows`, where the first label is the "current" window and the rest are additional ones — useful for window-specific code like a splash screen [2].
- **Partial event mocking** since 2.7.0 via the `shouldMockEvents` option, so `listen`/`emit` can be tested without a backend — though `emitTo` and `emit_filter` are not yet supported [2].

These tests run in a Node/jsdom context under a runner like Vitest, need no compiled binary and no driver, and are correspondingly cheap and fast [2]. Their blind spot is exactly what they mock away: because the backend is faked, they cannot catch a mismatch between the frontend's assumptions and the real Rust command, a permission that is not actually granted, or anything that only surfaces in the native webview [1][2].

## WebDriver: end-to-end tests against a real app

WebDriver is a standardized interface for automating web documents, and Tauri supports E2E testing through it [1][3]. Here the real application runs: the test drives the actual webview, exercising the frontend, the real IPC boundary, and the Rust backend together. The recommended path is WebdriverIO with `@wdio/tauri-service`, which exposes the Tauri API through `browser.tauri.execute()`, plus command (IPC) mocking, frontend and backend log capture, and multiremote [3].

What this buys you that mocks cannot:

- **The real IPC round-trip and backend behavior**, not a simulated stand-in [3].
- **Real webview rendering**, which the mock runtime deliberately skips [1].
- **Cross-platform confidence**, since the same suite can run on multiple OSes [3][4].

The cost is higher: you must build the application binary, arrange a WebDriver server, and — on Linux CI without a display — run under a virtual framebuffer such as `xvfb-run` [4]. The most common Windows failure mode is a WebDriver version mismatch with the WebView2 runtime, which auto-download mitigates [4].

## The macOS WebDriver gap

Coverage is not uniform across platforms, and macOS is the sharp edge. Driven directly, `tauri-driver` supports only Windows and Linux, "as macOS provides no desktop WebDriver client" [1] — Apple's WebDriver story is `safaridriver` for automating Safari itself, which does not help when the UI is a WKWebView inside a desktop app [5].

macOS is nonetheless testable, through routes that do not rely on a native OS driver:

- **The embedded WebDriver server.** `@wdio/tauri-service` can run an embedded server *inside* your app (via `tauri-plugin-wdio-webdriver`), needing no external driver on any platform — and this is precisely how macOS is supported [3][4].
- **CrabNebula's cross-platform fork** of `tauri-driver`, which covers all platforms but requires a paid API key for macOS [3][4].
- **A community open-source implementation**, Tauri-WebDriver, a W3C WebDriver for WKWebView Tauri apps built from an in-app plugin plus a `tauri-wd` CLI on port 4444 [5].

So the gap is specifically in the *direct native-driver* route; the embedded-server and commercial routes close it, at the cost of extra in-app plugins or a subscription [1][3][4][5].

## When you need both

The strategies catch disjoint failures, which is why the sources describe both. Mock-IPC unit tests are fast, backend-free, and let you assert on exact IPC calls and frontend logic — but by mocking the backend they cannot see integration failures, real permissions, or webview rendering [1][2]. WebDriver tests exercise the whole stack on real platforms — but cost a binary build, a driver setup, and CI plumbing, and they inherit the macOS driver gap [1][4]. Use mocks as the wide, cheap base for frontend logic and IPC-call assertions, and WebDriver E2E for the narrower, expensive confidence that the assembled app actually works on each target platform.

# Citations

1. [Tests](/sources/tauri-tests-overview.md)
2. [Mock Tauri APIs](/sources/tauri-tests-mocking.md)
3. [WebDriver](/sources/tauri-webdriver-overview.md)
4. [Platform Support | WebdriverIO](/sources/wdio-tauri-service.md)
5. [I Built a WebDriver for WKWebView Tauri Apps on macOS](/sources/wkwebview-driver-macos.md)
