---
type: Source
title: "Upgrade from Tauri 1.0 | Tauri"
description: "Comprehensive migration guide from Tauri 1.0 to 2.0 covering configuration, API restructuring, and breaking changes."
resource: https://v2.tauri.app/start/migrate/from-tauri-1/
tags: [core]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

This comprehensive guide details the migration path from Tauri 1.0 to Tauri 2.0, covering configuration changes, API restructuring, and breaking changes across the framework.

## Configuration Changes

The configuration structure underwent significant reorganization:

- **Package metadata** moved from nested `package` object to top-level (productName, version)
- **mainBinaryName** now required at top-level to match productName
- The `tauri` key was renamed to `"app"` throughout configurations
- Allowlist system completely replaced with new permissions model
- Build paths changed: `distDir` → `"frontendDist"`, `devPath` → `"devUrl"`

## Cargo Features Modified

**New features:**
- `"linux-protocol-body"` for custom protocol request body parsing

**Removed features:**
- `"reqwest-client"`, `"process-command-api"`, `"shell-open-api"`, `"windows7-compat"`
- `"updater"` and `"system-tray"` (now plugins)

## Major API Restructuring

The framework extracted numerous APIs into dedicated plugins:

- **CLI functionality** moved to `"@tauri-apps/plugin-cli"`
- **Dialog, Clipboard, HTTP, Notification** operations require their respective plugins
- **File system operations** use standard Rust `std::fs` or `"@tauri-apps/plugin-fs"`
- **Global shortcuts** operate through `"@tauri-apps/plugin-global-shortcut"`
- **Shell commands** handled by `"@tauri-apps/plugin-shell"`

## JavaScript API Changes

The `"@tauri-apps/api"` package now only exports core modules:

- Rename `"@tauri-apps/api/tauri"` → `"@tauri-apps/api/core"`
- Import specific functionality from individual plugins
- Window API relocated to `"@tauri-apps/api/webviewWindow"`

## Rust Type Renamings

- `"Window"` → `"WebviewWindow"`
- `"WindowBuilder"` → `"WebviewWindowBuilder"`
- `"get_window()"` → `"get_webview_window()"`

## Menu System Overhaul

Replace the old API with new builders:

- Use `"tauri::menu::MenuBuilder"` instead of `"tauri::Menu"`
- Use `"tauri::menu::MenuItemBuilder"` instead of `"tauri::CustomMenuItem"`
- Use `"tauri::menu::SubmenuBuilder"` instead of `"tauri::Submenu"`
- Use `"tauri::menu::PredefinedMenuItem"` instead of `"tauri::MenuItem"`

Menu event handling shifted from `"Builder::on_menu_event"` to `"App::on_menu_event"` or window-level handlers.

## System Tray Restructuring

Rebranded as `"TrayIcon"` with updated builders:

- Replace `"SystemTray"` with `"tauri::tray::TrayIconBuilder"`
- Split event handling into `"on_menu_event"` and `"on_tray_icon_event"`

## Permissions System Transformation

The v1 allowlist completely replaced with an access control list (ACL) approach:

- Create capability files in `"src-tauri/capabilities"` folder
- Run `"tauri migrate"` command to automatically convert v1 allowlist to v2 capabilities
- Permissions now support per-window and per-domain configuration

## Path Management Changes

Access path utilities through the Manager:

```rust
let home_dir = app.path().home_dir()?;
let resolved = app.path().resolve("path", BaseDirectory::Config)?;
```

## File System API Changes

JavaScript functions renamed:
- `"createDir"` → `"mkdir"`
- `"readBinaryFile"` → `"readFile"`
- `"removeDir"`, `"removeFile"` → `"remove"`
- `"renameFile"` → `"rename"`
- `"writeBinaryFile"` → `"writeFile"`

## Environment Variables

Renamed for consistency:
- `"TAURI_PRIVATE_KEY"` → `"TAURI_SIGNING_PRIVATE_KEY"`
- `"TAURI_KEY_PASSWORD"` → `"TAURI_SIGNING_PRIVATE_KEY_PASSWORD"`
- CLI-related variables prefixed with `"TAURI_CLI_"`

## Mobile Preparation

To support mobile targets, modify your Cargo manifest:

```toml
[lib]
name = "app_lib"
crate-type = ["staticlib", "cdylib", "rlib"]
```

Refactor `"main.rs"` into `"lib.rs"` with a `"run()"` function decorated with `"#[cfg_attr(mobile, tauri::mobile_entry_point)]"`.

## Windows Origin URL Change

Production apps now serve on `"http://tauri.localhost"` instead of `"https://tauri.localhost"`. Set `"app > windows > useHttpsScheme"` to `"true"` to preserve the HTTPS scheme and prevent IndexedDB/LocalStorage loss.

## Event System Redesign

- `"emit()"` now broadcasts to all listeners
- New `"emit_to()"` targets specific EventTargets
- `"listen_global"` renamed to `"listen_any"`
- Event filtering changed from window-based to EventTarget-based approach

## Automated Migration Tool

Run `"tauri migrate"` with your package manager to automatically convert configurations and generate capability files. However, manual review of all changes remains necessary.
