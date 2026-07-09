---
type: Capability
title: System-webview model
description: Tauri renders through the OS-provided webview (WebView2/WKWebView/WebKitGTK) rather than bundling an engine — the size and security upside, and the per-platform-rendering-difference downside.
tags: [core, security, performance]
timestamp: 2026-07-09T00:00:00Z
---
# System-webview model

Tauri's defining rendering decision is that it does **not bundle a browser engine**. Instead it renders the frontend through the WebView library the operating system already provides [1]. The abstraction that selects and drives the platform webview is **WRY**, a cross-platform rendering library that determines which webview implementation is used on each platform; Tauri is explicitly not a lightweight kernel wrapper, VM, or virtualized environment layered over a bundled engine [2].

## Three engines, one per platform

The system webview differs by platform [1][2]:

- **Windows** — Microsoft Edge WebView2 (Chromium-based)
- **macOS** — WKWebView (native Apple WebKit)
- **Linux** — WebKitGTK (GTK-based WebKit)

Because these libraries are dynamically linked rather than embedded, the shipped executable is smaller [1], and Tauri's binaries are small by default [3].

## The upside: size, updates, attack surface

Not bundling an engine has two structural benefits. First, size — the webview is already on the machine, so it is not shipped [1][3]. Second, security maintenance: Tauri's own framing is that "WebView package maintainers are significantly faster to patch and roll out security updates than application developers who bundle WebView directly," which reduces both the attack surface and the maintenance burden on the app developer [4]. The OS keeps the engine patched; the application inherits those fixes without re-releasing.

## The downside: per-platform behavior and no version pinning

The cost is that the app runs on a **different engine per platform**, and on a version it does not control. A Chromium-based WebView2 on Windows and two WebKit variants on macOS and Linux will differ in web-feature support, rendering, and bugs — the frontend must be tested on each target rather than assumed uniform. The engine is also not pinned: the app gets whatever version the OS supplies.

This shows up concretely in resource use. Contrary to Tauri's positioning as more memory-efficient than Electron, a maintainer-tracked issue reports the WebKit-based webview consuming more RAM than Chromium/Electron in real-world use — a gap exceeding 90 MB consistently across macOS, Ubuntu, and Windows — and argues the original benchmarks failed to account for Chromium's shared memory [5]. WebKit-based apps, in other words, can consume *more* memory than Chromium-based ones during typical web-app usage [5]. The official benchmark repository does compare Tauri, Wry, and Electron across execution time, binary size, memory, thread count, and syscalls, but on hello-world, prime-computation, custom-protocol, and file-transfer test apps rather than arbitrary real-world workloads [6].

## Testing implication

Because the engine is the OS's, end-to-end testing drives the real platform webview through WebDriver, and the tooling is itself per-platform — most notably macOS's WKWebView required a purpose-built W3C WebDriver implementation to test Tauri apps at all [7]. (End-to-end testing practice is covered by the practices layer.)

# Citations

1. [Process Model | Tauri](/sources/tauri-process-model.md)
2. [Tauri Architecture | Tauri](/sources/tauri-architecture.md)
3. [App Size Optimization in Tauri](/sources/tauri-app-size.md)
4. [Security | Tauri](/sources/tauri-security-overview.md)
5. [Memory benchmark might be incorrect: Tauri might consume more RAM than Electron](/sources/tauri-issue-memory-benchmarks.md)
6. [tauri-apps/benchmark_results](/sources/tauri-benchmark-results.md)
7. [I Built a WebDriver for WKWebView Tauri Apps on macOS](/sources/wkwebview-driver-macos.md)
