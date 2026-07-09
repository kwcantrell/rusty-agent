---
type: Source
title: "Plugin Development | Tauri"
description: "Tauri plugins extend core functionality via Rust, Kotlin, or Swift code exposed to the webview."
resource: https://v2.tauri.app/develop/plugins/
tags: [core]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Tauri plugins extend the framework's core functionality by hooking into the application lifecycle and exposing Rust, Kotlin, or Swift code to the webview. By design, the Tauri core does not contain features not needed by everyone. Instead it offers a mechanism to add external functionalities into a Tauri application called plugins.

## Plugin Structure

A Tauri plugin consists of:
- A Cargo crate (Rust)
- An optional NPM package for JavaScript bindings
- Optional Android library (Kotlin) and iOS (Swift) projects

Plugins follow a standardized naming pattern. The Rust crate uses the format `tauri-plugin-{name}`, while the JavaScript package follows either `tauri-plugin-{name}-api` or the recommended scoped format `@scope-name/plugin-{name}`.

New plugins are created using the CLI command `npx @tauri-apps/cli plugin new [name]`. The resulting directory structure includes:
- `src/` - Rust implementation files
- `permissions/` - Permission configuration files
- `android/` and `ios/` - Native mobile projects
- `guest-js/` - JavaScript API bindings

## Lifecycle Events

Plugins can hook into five core lifecycle stages:

**setup**: Plugin is being initialized — Used to register mobile plugins, manage state, and run background tasks.

**on_navigation**: Triggered when the webview attempts navigation. Returning `false` cancels the operation.

**on_webview_ready**: New window has been created — Executes initialization scripts for each window.

**on_event**: Handles core events such as window events, menu events and application exit requested.

**on_drop**: Executes code during plugin deconstruction.

## Exposing APIs

The plugin's public API is exported as a struct matching the plugin name (in PascalCase). Users access it through an extension trait via a `Manager` instance.

Commands are defined in `commands.rs` and registered through `invoke_handler()`. They support dependency injection of `AppHandle` and `Window`, state access, and input parameters.

Example registration:
```rust
Builder::new("<plugin-name>")
    .invoke_handler(tauri::generate_handler![commands::upload])
```

JavaScript binding example:
```typescript
export async function upload(url: string, onProgressHandler: (progress: number) => void): Promise<void> {
  const onProgress = new Channel<number>()
  onProgress.onmessage = onProgressHandler
  await invoke('plugin:<plugin-name>|upload', { url, onProgress })
}
```

## Permissions & Security

Commands require explicit permission definitions. Two permission types exist:

**Command-specific permissions**: Use `CommandScope<'_, Entry>` to restrict individual commands.

**Global scope permissions**: Applied plugin-wide using `GlobalScope<'_, Entry>`.

Permissions are defined in TOML/JSON files within the `permissions/` directory. The documentation recommends checking both global and command scopes for flexibility.

## Configuration

Plugin configuration is specified in `tauri.conf.json`:

```json
{
  "plugins": {
    "plugin-name": {
      "timeout": 30
    }
  }
}
```

The config struct is accessible during setup via the `Builder` API.

## State Management

Plugins manage state identically to Tauri applications, using the same state management patterns and `Manager::manage()` methods.
