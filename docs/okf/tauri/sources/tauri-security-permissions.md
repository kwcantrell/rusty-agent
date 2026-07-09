---
type: Source
title: "Permissions | Tauri"
description: "Tauri permissions system for controlling explicit privileges of commands and frontend access to system resources."
resource: https://v2.tauri.app/security/permissions/
tags: [security]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Tauri permissions are "descriptions of explicit privileges of commands" that establish a security model where each capability must be explicitly granted. The system enables commands to be accessible in the frontend while maintaining security boundaries through flexible configuration and grouping mechanisms.

## Core Concepts

Permissions control what frontend code can access by establishing explicit privileges for each command. The system is flexible, supporting:

- Enabling or denying specific commands
- Defining path scopes
- Combining both approaches

Developers can group related permissions into sets under new identifiers called "permission sets," enabling reuse and simplification of configuration.

## Permission Identifiers

Permission identifiers follow a naming convention:
- `<name>:default` for default plugin/app permissions
- `<name>:<command-name>` for individual commands

The plugin prefix "tauri-plugin-" is automatically prepended at compile time. Identifiers are limited to ASCII lowercase letters [a-z] with a maximum length of 116 characters.

## Configuration Structure

**Plugins** place permission definitions in `permissions/<identifier>.json/toml`

**Applications** use `src-tauri/permissions/<identifier>.toml` (TOML format only for permissions)

Permissions must be referenced in capability files to be granted to app windows or webviews.

## Use Cases

- **Plugin developers** ship pre-defined, well-named permissions for exposed commands
- **Application developers** extend existing plugin permissions or define custom ones
- **Configuration simplification** through grouping or bundling platform-specific rules

## Example Configuration Pattern

A basic permission structure includes an identifier, description, and command allowlists or scope definitions with path patterns such as `$HOME/*`, enabling developers to define granular access control for their applications.
