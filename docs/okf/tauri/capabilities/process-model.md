---
type: Capability
title: Process model
description: Tauri's multi-process split between a privileged Rust Core and OS-provided WebView renderers, and why the architecture is shaped that way.
tags: [core, security, performance]
timestamp: 2026-07-09T00:00:00Z
---
# Process model

Tauri runs as a multi-process application, comparable to modern browsers and Electron: it separates a single privileged **Core process** from one or more **WebView processes** that render the UI [1]. The split exists to buy three properties a single-process design cannot: resilience (a crash in one component does not take down the others), performance (work spreads across multiple cores), and security (each process gets only the privileges it needs) [1].

## The Core process

The Core process is the application's primary entry point and the only part with full operating-system access [1]. Tauri implements it in Rust, leaning on Rust's ownership model for memory safety alongside native performance [1]. Its responsibilities include creating and managing windows, driving system-tray menus and notifications, routing IPC messages between frontend and backend, holding global state (settings, database connections, shared resources), and reaching the filesystem, network, and OS APIs directly [1].

The Core is also the routing layer for all Inter-Process Communication: messages between WebView processes and backend functionality pass through it, and it is where security boundaries are enforced [1]. Because IPC is asynchronous message passing rather than shared memory, the Core is free to reject or discard a request before it executes — a malicious message can be dropped without ever running [2].

## The WebView process

The Core spawns WebView processes that render the interface using OS-provided WebView libraries, executing HTML, CSS, and JavaScript with standard web frameworks [1]. These libraries are dynamically linked rather than bundled, which reduces the shipped executable size [1]. WebView processes run with restricted OS access, enforced by both the operating system and the Tauri framework; they can only reach backend functionality through defined IPC channels [1].

Which WebView library backs a given process is platform-dependent: Microsoft Edge WebView2 on Windows, WKWebView on macOS, and WebKitGTK on Linux [1]. (The per-platform consequences of this choice are treated in [Webview](/capabilities/webview.md).)

## Why least privilege

The multi-process design is a direct implementation of the Principle of Least Privilege: the framework's own framing is that "the less access we give them, the less harm they can do if they get compromised" [1]. Restricting WebView processes to minimal OS access confines a compromise to a single renderer process rather than the whole application, while the Core retains full access for legitimate backend work [1]. This is the runtime half of Tauri's security posture; the declarative half — which commands a given window may even reach — is the capability system, and the controlled bridge between the two trust domains is IPC [3].

# Citations

1. [Process Model | Tauri](/sources/tauri-process-model.md)
2. [Inter-Process Communication | Tauri](/sources/tauri-ipc-overview.md)
3. [Security | Tauri](/sources/tauri-security-overview.md)
