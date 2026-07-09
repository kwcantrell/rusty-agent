---
type: Source
title: "Window Customization | Tauri"
description: "Tauri provides configuration, JavaScript, and Rust APIs for customizing window appearance and behavior."
resource: https://v2.tauri.app/learn/window-customization/
tags: [core]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Tauri provides several approaches for customizing window appearance and behavior through configuration methods, custom titlebars, and platform-specific features.

## Configuration Methods

Changes can be made via three routes: the `tauri.conf.json` file, JavaScript API, or Rust Window implementation.

## Custom Titlebars

Developers can disable native decorations by setting `decorations` to `false`, then build custom titlebar interfaces with HTML, CSS, and JavaScript controls for minimize, maximize, and close functions.

## Manual Drag Implementation

For specialized drag behaviors, the `window.startDragging()` method provides programmatic control instead of using the `data-tauri-drag-region` attribute.

## macOS-Specific Features

On macOS, developers can create a transparent titlebar with custom window background colors using the `TitleBarStyle::Transparent` setting and native Cocoa APIs to set custom background colors.

## Additional Customization

The platform supports transparent windows, size constraints, and various window styling options through configuration.
