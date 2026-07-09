---
type: Source
title: "Process Model | Tauri"
description: "Multi-process architecture separating Core and WebView processes for resilience, performance, and security through privilege isolation."
resource: https://v2.tauri.app/concept/process-model/
tags: [core]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---
# Summary

Tauri implements a multi-process architecture comparable to Electron and modern browsers, separating concerns into distinct processes (Core and WebView) for improved stability, security, and performance. This design addresses legacy single-process limitations and enables better utilization of modern computing resources.

## Motivation for Multi-Process Architecture

**Legacy Single-Process Problems**: Expensive computations froze entire interfaces, and component failures crashed entire applications.

**Modern Multi-Process Benefits**:
- **Resilience**: Crashes in one component don't affect others
- **Performance**: Better utilization of multi-core processors
- **Security**: Implementation of the Principle of Least Privilege by restricting permissions per process

As stated in the documentation: "The less access we give them, the less harm they can do if they get compromised."

## The Core Process

The application's primary entry point with full OS access. Responsibilities include:

- Creating and managing application windows
- Managing system-tray menus and notifications
- Routing Inter-Process Communication (IPC) messages between frontend and backend
- Managing global state (settings, database connections, shared resources)
- Direct access to file system, network, and OS APIs

**Implementation**: Tauri uses Rust for the Core process, leveraging Rust's ownership system for memory safety alongside performance optimization.

## The WebView Process

The Core spawns WebView processes that render the user interface using OS-provided WebView libraries. Key characteristics:

- Executes HTML, CSS, and JavaScript
- Leverages standard web development practices and frameworks
- WebView libraries are dynamically linked, reducing executable size
- WebView processes have restricted OS access (enforced by OS and Tauri framework)

**Platform-Specific WebView Libraries**:
- **Windows**: Microsoft Edge WebView2
- **macOS**: WKWebView
- **Linux**: webkitgtk

## Communication Architecture

The Core routes Inter-Process Communication (IPC) messages between WebView processes and backend functionality. This routing layer enforces security boundaries and manages message passing semantics.

## Security Model

The multi-process design implements the Principle of Least Privilege: WebView processes have minimal OS access, containing potential compromises to a single process rather than the entire application. The Core process maintains full access for legitimate backend operations while WebView processes can only interact with Core through defined IPC channels.
