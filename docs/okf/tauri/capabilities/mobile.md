---
type: Capability
title: Mobile support
description: Survey of Tauri v2's Android/iOS reach — the shared architecture, the native plugin split, the project layout, and where the desktop/mobile capability line falls.
tags: [mobile, core]
timestamp: 2026-07-09T00:00:00Z
---
# Mobile support

Tauri v2 targets Android and iOS in addition to the three desktop platforms — a major expansion over v1. The reach is structural rather than bolted on: the window layer, TAO, supports Windows, macOS, Linux, iOS, and Android from the same abstraction [1], and the capability system's `platforms` field enumerates `android` and `ios` alongside `linux`, `windows`, and `macos`, so a single access-control model spans all five [2]. This survey sketches what mobile gives you and where it diverges from desktop; it is not a setup walkthrough.

## What the same core buys you

The Core/WebView process model and the IPC surface carry over — mobile still renders a web frontend in a system webview driven by a Rust core. What changes is the native edge. Mobile support is realized largely through the plugin system: a plugin's scaffold already includes `android/` (Kotlin) and `ios/` (Swift) native projects next to its Rust `src/` [3], and the official plugin catalog carries a compatibility matrix precisely because not every plugin supports Android and iOS [4].

## The native plugin split

Where a plugin needs native mobile behavior, the default template splits the Rust implementation into `desktop.rs` and `mobile.rs` modules, with the mobile side sending a message to native code to execute a function and return a result [5]. The two native sides:

- **Android** — plugins are Kotlin (or Java) classes extending `app.tauri.plugin.Plugin` with the `@TauriPlugin` annotation; methods marked `@Command` become callable from Rust or JavaScript, and cross-language calls use JNI [5].
- **iOS** — plugins are Swift classes extending Tauri's `Plugin`; functions with the `@objc` attribute and an `Invoke` parameter become commands, dependencies flow through Swift Package Manager, and cross-language calls use C FFI [5].

Both platforms support lifecycle hooks (`load`, `onNewIntent`), type-safe argument parsing, permission checking/requesting, event emission to JavaScript, and calling into shared Rust for performance-critical work [5]. One platform caveat worth flagging at survey depth: Android requires careful handling of long-running operations to avoid ANR (Application Not Responding) errors [5].

## Project layout and the desktop/mobile line

Mobile is opt-in at setup: Android Studio and iOS tooling are listed as an optional prerequisite category needed only when targeting mobile, separate from the base Rust/Node dependencies [6]. Signing diverges too — Android distribution through the Play Store requires signing App Bundles/APKs with a `keytool`-generated keystore wired into Gradle, a flow distinct from desktop code signing [7], and the distribution tooling routes Android and iOS builds to Google Play and the App Store respectively [8].

The desktop/mobile capability split therefore falls along three seams: **platform-gated capabilities** (the `platforms` field lets a permission apply on desktop but not mobile, or vice versa) [2]; **plugin availability** (a plugin may simply not implement Android or iOS, per the compatibility matrix) [4]; and **native implementation** (behavior that must differ lives in `mobile.rs` plus Kotlin/Swift rather than shared Rust) [5]. Setup and per-platform build minutiae are out of scope here.

# Citations

1. [Tauri Architecture | Tauri](/sources/tauri-architecture.md)
2. [Capabilities for Different Windows and Platforms](/sources/tauri-learn-capabilities-multiwindow.md)
3. [Plugin Development | Tauri](/sources/tauri-plugins-develop.md)
4. [Features & Recipes | Tauri](/sources/tauri-plugin-catalog.md)
5. [Mobile Plugin Development | Tauri](/sources/tauri-mobile-plugin-dev.md)
6. [Prerequisites | Tauri](/sources/tauri-prerequisites.md)
7. [Android Code Signing | Tauri](/sources/tauri-sign-android.md)
8. [Distribute | Tauri](/sources/tauri-distribute-overview.md)
