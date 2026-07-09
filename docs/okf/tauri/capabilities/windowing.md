---
type: Capability
title: Windowing
description: Tauri's window creation and customization surface — decorations, custom titlebars, drag regions, constraints, and the config/JS/Rust triad — plus its per-window security tie-in.
tags: [core, security]
timestamp: 2026-07-09T00:00:00Z
---
# Windowing

Windows in Tauri are created and configured through three interchangeable routes: the `tauri.conf.json` configuration file, the JavaScript API, and the Rust `Window` implementation [1]. Windows can be declared statically in configuration or built programmatically at runtime — the config file takes a `windows` array of labelled entries, while Rust uses `WebviewWindowBuilder::new(app, label, url)` with a fluent builder [2].

The underlying cross-platform window creation is provided by **TAO**, a library forked from `winit` that supports Windows, macOS, Linux, iOS, and Android [3]. Each window hosts a WebView process, so a Tauri "window" couples an OS window with a webview.

## Decorations and custom titlebars

Native window decorations can be disabled by setting `decorations` to `false`, after which the application builds its own titlebar in HTML, CSS, and JavaScript — including the minimize, maximize, and close controls [1]. This is the standard path to a fully custom chrome.

## Drag behavior

A frameless window needs an explicit drag region so the user can still move it. The declarative approach is the `data-tauri-drag-region` attribute on an element; for specialized drag behavior, `window.startDragging()` gives programmatic control instead [1].

## Platform-specific styling

Window styling has platform caveats. On macOS specifically, developers can create a transparent titlebar with a custom window background color using the `TitleBarStyle::Transparent` setting together with native Cocoa APIs to set the background color [1]. Beyond that, the platform supports transparent windows and size constraints among other styling options [1].

## Windows are a security boundary

Windows are not just presentation — they are a unit of the security model. Capabilities, defined as JSON files in `src-tauri/capabilities/`, target specific windows by label through a `windows` field, so different windows can be granted different permissions; multiple windows can also share a capability [2]. The same capability files can be gated to specific operating systems via a `platforms` field (`linux`, `windows`, `macos`, `android`, `ios`) [2]. The recommended posture is least privilege: "only give the necessary capabilities to each window" [2]. This means window creation and permission design are intertwined — adding a window with a distinct trust level generally means authoring a capability scoped to its label. (The permission mechanics themselves are covered in [Plugin system](/capabilities/plugin-system.md).)

# Citations

1. [Window Customization | Tauri](/sources/tauri-window-customization.md)
2. [Capabilities for Different Windows and Platforms](/sources/tauri-learn-capabilities-multiwindow.md)
3. [Tauri Architecture | Tauri](/sources/tauri-architecture.md)
