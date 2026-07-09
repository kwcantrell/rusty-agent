---
type: Source
title: "Capabilities for Different Windows and Platforms"
description: "Capability-based access control for assigning different permissions to windows and platforms."
resource: https://v2.tauri.app/learn/security/capabilities-for-windows-and-platforms/
tags: [security]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

## Overview

Tauri enables granular security through capability-based access control, allowing developers to assign different permissions to different windows and restrict them to specific platforms.

## Core Security Model

The capability system implements principle of least privilege: "for better security it is recommended to only give the necessary capabilities to each window."

Capabilities are defined in JSON files within `src-tauri/capabilities/` and specify:
- **Identifier**: unique name for the capability
- **Permissions**: specific actions allowed (e.g., `fs:allow-home-read`)
- **Windows**: target window labels receiving these permissions
- **Platforms**: operating systems where capability is active

## Window-Specific Capabilities

Capabilities use a `windows` field to target specific windows by label:

```json
{
  "identifier": "fs-read-home",
  "description": "Allow file access to home directory",
  "local": true,
  "windows": ["first"],
  "permissions": ["fs:allow-home-read"]
}
```

Multiple windows can share capabilities:

```json
{
  "identifier": "dialog",
  "windows": ["first", "second"],
  "permissions": ["dialog:allow-ask"]
}
```

## Platform-Dependent Capabilities

Capabilities activate selectively using the `platforms` field:

```json
{
  "identifier": "fs-read-home",
  "windows": ["first"],
  "platforms": ["linux", "windows"],
  "permissions": ["fs:allow-home-read"]
}
```

Available platforms: `linux`, `windows`, `macos`, `android`, `ios`

## Window Creation Methods

**Configuration file** (`tauri.conf.json`):
```json
"windows": [
  {"label": "first", "title": "First"},
  {"label": "second", "title": "Second"}
]
```

**Programmatic** (Rust):
```rust
tauri::WebviewWindowBuilder::new(app, "first", webview_url)
  .title("First")
  .build()?;
```
