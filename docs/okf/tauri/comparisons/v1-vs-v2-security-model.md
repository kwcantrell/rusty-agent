---
type: Comparison
title: "Tauri v1 allowlist vs v2 capabilities: two security models"
description: "How Tauri's v1 global allowlist was replaced by the v2 capabilities/permissions ACL, what the new model buys you, and what the migration actually entails."
tags: [security, distribution, core]
timestamp: 2026-07-09T00:00:00Z
---

Between Tauri 1.0 and 2.0 the mechanism that decides what the frontend is allowed to touch changed shape entirely. The v1 allowlist was "completely replaced with an access control list (ACL) approach" [1]. This is not a rename — it is a different model with different granularity, and it forces a migration.

## v1: the global allowlist

In Tauri 1.0, access was governed by an allowlist: a global configuration switching whole API areas on or off for the application as a whole [1]. It was coarse. Turning on filesystem or shell access enabled that surface for the entire app, with no built-in way to say *this* window gets it and *that* one does not, or that a capability applies on desktop but not mobile.

## v2: permissions, scopes, and capabilities

Tauri 2.0 replaces the single allowlist with three layered concepts that compose:

- **Permissions** are "descriptions of explicit privileges of commands" [3]. Each command's exposure to the frontend is described explicitly, and related permissions can be grouped into reusable "permission sets" under a single identifier [3]. Identifiers follow a convention — `<name>:default` for defaults, `<name>:<command-name>` for individual commands — and the `tauri-plugin-` prefix is prepended at compile time [3].
- **Command scopes** narrow *what a command may act on*, using a hierarchical allow/deny structure in which "denial rules always take precedence over allowance rules" [4]. The fs plugin scopes directory and file access with glob path strings; the HTTP plugin scopes which URLs may be reached [4]. Crucially, scopes are handed to the command, and "command developers need to ensure that there are no scope bypasses possible" — enforcement lives in the command implementation, not the framework [4].
- **Capabilities** tie permissions to frontend contexts. They "granularly enable and constrain the core exposure to the application frontend running in the system WebView" [2], defined as JSON or TOML files in `src-tauri/capabilities/`, each naming target windows/webviews and their associated permissions [2].

Together these enable what the allowlist could not: "per-window and per-domain configuration" [1], plus platform targeting via a `platforms` array so desktop and mobile can receive different permission sets [2]. The whole scheme sits inside Tauri's trust-boundary model, where the WebView frontend reaches system resources only through IPC and access is enforced at the command implementation level [5].

## What the new model buys you — and its limits

The gain is least-privilege granularity: rather than a blanket `allow-write-text-file`, you scope it to a precise path such as `$HOME/test.txt`, and a specific capability decides which window even sees that permission [6][2]. Denials always win over allowances, so a broad grant cannot silently re-open something an explicit deny closed [4].

The limits are stated plainly. Capabilities "address frontend compromise risks by minimizing impact" but "do not protect against malicious or insecure Rust code" or a compromised development environment — defense-in-depth is still the developer's job [2]. And because scope enforcement lives in command code, a buggy scope check is a real bypass risk that requires careful auditing [4].

## Migration implications

The move is mandatory when upgrading — the allowlist is gone, so there is no v1-compatible path forward [1]. Concretely:

- **Config restructuring.** The `tauri` config key is renamed to `app`, and the allowlist configuration is replaced by the permissions model [1].
- **Capability files.** You create capability files in `src-tauri/capabilities/`, and application-level permissions live in `src-tauri/permissions/<identifier>.toml` — TOML only for app permissions [1][3].
- **APIs became plugins.** Much of what the allowlist gated — dialog, clipboard, HTTP, notification, shell, global shortcut, CLI — moved into dedicated plugins, each shipping its own default permission set you must opt into [1]. Adding a plugin with `tauri add <plugin>` brings its permissions, which you then reference in a capability [6].
- **Automated but not hands-off.** The `tauri migrate` command auto-converts v1 allowlist configuration into v2 capability files, but "manual review of all changes remains necessary" [1].

The practical shift for a maintainer: where v1 asked "which API areas does my app use?", v2 asks, per window and per platform, "which commands, scoped to which resources, does *this* context need?" — and answers it in reviewable capability files rather than one global switchboard [2][3][4].

# Citations

1. [Upgrade from Tauri 1.0 | Tauri](/sources/tauri-migrate-from-v1.md)
2. [Capabilities | Tauri](/sources/tauri-security-capabilities.md)
3. [Permissions | Tauri](/sources/tauri-security-permissions.md)
4. [Command Scopes | Tauri](/sources/tauri-security-scope.md)
5. [Security | Tauri](/sources/tauri-security-overview.md)
6. [Using Plugin Permissions](/sources/tauri-learn-plugin-permissions.md)
