---
type: Source
title: "Tauri Architecture | Tauri"
description: "Polyglot composable toolkit combining Rust with HTML rendered in Webview through message passing between frontend and backend."
resource: https://v2.tauri.app/concept/architecture/
tags: [core]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---
# Summary

Tauri is a "polyglot and generic toolkit that is very composable" for building desktop applications by combining Rust tools with HTML rendered in a Webview. The framework operates through message passing between frontend and backend components and is distinctly not a lightweight kernel wrapper, VM, or virtualized environment. Instead, it directly leverages WRY and TAO for system-level operations.

## Core Crates

**tauri**: The central crate integrating runtimes, macros, utilities, and APIs. Acts as the primary integration point for all framework functionality.

**tauri-runtime**: Glue layer between Tauri and webview libraries, abstracting platform differences.

**tauri-macros**: Generates context, handler, and command macros used in application code.

**tauri-utils**: Shared utilities including configuration parsing and Content Security Policy (CSP) management.

**tauri-build**: Applies macros during compilation time.

**tauri-codegen**: Embeds, hashes, and compresses assets at compile time for distribution.

**tauri-runtime-wry**: Enables direct system interactions for WRY operations.

## Upstream Dependencies

**TAO**: A cross-platform application window creation library forked from winit. Supports Windows, macOS, Linux, iOS, and Android platforms.

**WRY**: A cross-platform WebView rendering library that determines which webview implementation is used across platforms. Acts as the abstraction layer for platform-specific rendering.

## Platform-Specific WebView Implementations

- **Windows**: Microsoft Edge WebView2
- **macOS**: WKWebView (native Apple WebKit)
- **Linux**: webkitgtk (GTK-based WebKit)

## Tooling Ecosystem

The framework provides:
- JavaScript/TypeScript APIs for frontend-backend communication
- Platform-aware bundler for asset management
- CLI tools in both Rust and JavaScript (via napi-rs)
- `create-tauri-app` scaffold for rapid project initialization with various frontend frameworks (React, Vue, Svelte, Angular, etc.)

## Plugins

Plugins enable Rust functionality, provide integration glue between core and specific features, and expose JavaScript APIs for accessing backend operations. They serve as the primary extension mechanism.

## Licensing

Tauri uses dual licensing: MIT or Apache-2.0. Developers retain responsibility for ensuring compliance with upstream dependency licenses.
