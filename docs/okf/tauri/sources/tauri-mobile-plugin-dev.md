---
type: Source
title: "Mobile Plugin Development | Tauri"
description: "Guide for developing native mobile plugins for Tauri applications using Kotlin/Java for Android and Swift for iOS."
resource: https://v2.tauri.app/develop/plugins/develop-mobile/
tags: [mobile]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Comprehensive guide to developing native mobile plugins for Tauri. Default template separates implementation into `desktop.rs` and `mobile.rs` modules, where "the mobile implementation sends a message to the native mobile code to execute a function and get a result back."

**Android Development:**
- Plugins are Kotlin classes extending `app.tauri.plugin.Plugin` with `@TauriPlugin` annotation
- Methods marked with `@Command` become callable from Rust or JavaScript
- Supports both Kotlin and Java implementations

**iOS Development:**
- Plugins extend `Plugin` class from Tauri package as Swift classes
- Functions with `@objc` attribute and `Invoke` parameters become callable commands
- Leverages Swift Package Manager for dependency management

**Core Capabilities:**
- Lifecycle event hooks (`load`, `onNewIntent`)
- Command argument parsing with type safety
- Permission checking and requesting
- Inter-language communication (JNI for Android, FFI for iOS)
- Event emission to JavaScript
- Configuration management

**Platform Considerations:** Android requires careful handling of long-running operations to prevent ANR errors; iOS uses standard C FFI. Both platforms support calling shared Rust code for performance-critical operations.
