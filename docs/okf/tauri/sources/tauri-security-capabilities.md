---
type: Source
title: "Capabilities | Tauri"
description: "Tauri capabilities system for granularly controlling permissions granted to windows and webviews."
resource: https://v2.tauri.app/security/capabilities/
tags: [security]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Capabilities "granularly enable and constrain the core exposure to the application frontend running in the system WebView." The system allows developers to control which permissions are granted to different windows and webviews within their applications through JSON or TOML configuration files.

## Definition Structure

Capabilities are defined through configuration files located in the `src-tauri/capabilities` directory. Each capability file contains:

- An identifier
- Description
- Target windows or webviews
- Associated permissions

This structure enables developers to express which frontend contexts receive which permissions.

## Configuration Methods

The framework documents three approaches for defining capabilities:

1. **File-based**: Individual capability files referenced in `tauri.conf.json`
2. **Inline configuration**: Capabilities directly embedded in the configuration file
3. **Hybrid approach**: Mixing pre-defined and inline capabilities

Developers can select the approach that best fits their organizational and architectural needs.

## Platform-Specific Features

Capabilities support platform targeting through a `platforms` array, enabling different permission sets for:

- Desktop environments (Linux, macOS, Windows)
- Mobile environments (iOS, Android)

This allows developers to tailor the permission surface for each deployment target.

## Security Scope

The documentation notes that capabilities address frontend compromise risks by "minimizing impact" and preventing "exposure of local system interfaces and data." However, capabilities do not protect against:

- Malicious or insecure Rust code
- Compromised development environments

Developers must apply defense-in-depth practices beyond capabilities alone.

## Additional Configuration

Custom Rust commands can be restricted through `tauri-build` configuration. The page references schema files for IDE autocompletion support, enabling developers to use type-aware tooling when authoring capability definitions.
