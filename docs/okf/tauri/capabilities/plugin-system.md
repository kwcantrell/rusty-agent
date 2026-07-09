---
type: Capability
title: Plugin system
description: Tauri's plugin architecture — the lifecycle hooks, cross-language structure, permission integration, and the official/community ecosystem — as the framework's primary extension mechanism.
tags: [core, security, mobile]
timestamp: 2026-07-09T00:00:00Z
---
# Plugin system

Plugins are Tauri's primary extension mechanism. By design the core deliberately omits features not needed by everyone, and plugins are how external functionality is added back in [1]. They are also central to the architecture rather than bolted on: plugins enable Rust functionality, provide the integration glue between the core and specific features, and expose JavaScript APIs for reaching backend operations [2].

## Structure

A plugin is a polyglot package: a Cargo crate (Rust), an optional NPM package for JavaScript bindings, and optional Android (Kotlin) and iOS (Swift) native projects [1]. Naming follows a standardized pattern — the Rust crate is `tauri-plugin-{name}`, and the JS package is `tauri-plugin-{name}-api` or, preferably, a scoped `@scope-name/plugin-{name}` [1]. Scaffolding a new plugin (`npx @tauri-apps/cli plugin new [name]`) produces `src/` (Rust), `permissions/`, `android/` and `ios/` (native mobile), and `guest-js/` (JS bindings) [1].

## Lifecycle

Plugins hook into the application lifecycle at five stages [1]:

- **setup** — plugin initialization; where mobile plugins are registered, state is managed, and background tasks start.
- **on_navigation** — fired when the webview attempts navigation; returning `false` cancels it.
- **on_webview_ready** — a new window was created; runs per-window initialization scripts.
- **on_event** — handles core events such as window events, menu events, and application-exit-requested.
- **on_drop** — runs during plugin deconstruction.

A plugin exposes its Rust API as a struct named after the plugin (PascalCase), reached through an extension trait on a `Manager` instance, and it manages state using the same `Manager::manage()` patterns as an application [1]. Commands live in `commands.rs`, are registered via `invoke_handler()`, and support the same dependency injection as application commands (`AppHandle`, `Window`, state, parameters) [1].

## Permission integration

Plugin commands are not callable by default — they require explicit permission definitions, and this is where the plugin system meets Tauri's access-control model [1]. Permissions are declared in TOML/JSON files in the plugin's `permissions/` directory, and two scope types exist: command-specific permissions via `CommandScope<'_, Entry>` and plugin-wide permissions via `GlobalScope<'_, Entry>` [1]. Each plugin ships a `default` permission set giving a reasonable baseline — for official plugins documented in the Tauri docs, for community plugins in `permissions/default.toml` [3]. To use a plugin, its permissions are wired into a capability file under `src-tauri/capabilities/` (or `tauri.conf.json`), and custom scopes should be narrowed to specific paths rather than granting blanket access, following least privilege [3]. Calling a command without the needed permission fails at runtime with an error such as `fs.write_text_file not allowed` [3].

## Ecosystem

The official ecosystem is substantial: the catalog lists 34 official plugins covering filesystem access, notifications, HTTP clients, biometric authentication and more, alongside 60-plus community plugins and separate community integrations for tools like Deno, Angular, and Svelte [4]. Not every plugin works everywhere — a compatibility matrix records which official plugins support Android, iOS, Linux, macOS, and Windows [4]. The updater is one such official plugin, delivering signed automatic updates. Mobile plugin development — Kotlin/`@TauriPlugin` classes on Android, Swift `Plugin` subclasses on iOS — is a first-class part of this system; see [Mobile](/capabilities/mobile.md).

# Citations

1. [Plugin Development | Tauri](/sources/tauri-plugins-develop.md)
2. [Tauri Architecture | Tauri](/sources/tauri-architecture.md)
3. [Using Plugin Permissions](/sources/tauri-learn-plugin-permissions.md)
4. [Features & Recipes | Tauri](/sources/tauri-plugin-catalog.md)
