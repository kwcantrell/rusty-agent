---
type: Practice
title: Capabilities, permissions, and scopes discipline
description: Configure Tauri's access-control system for least privilege — granting only necessary permissions per window and platform, scoping commands to exact paths and hosts, and pruning defaults rather than accepting them.
tags: [security, ipc-architecture]
timestamp: 2026-07-09T00:00:00Z
---

# Capabilities, permissions, and scopes discipline

Tauri's security rests on a strict trust boundary: the Rust core has full,
unconstrained system access, while the WebView frontend is limited to exactly the
resources you expose over IPC [1]. What you expose is governed by three layered
mechanisms — permissions, command scopes, and capabilities — and the whole point
is to grant the *minimum* needed [1]. Everything is disabled by default and must
be explicitly enabled [2], so the failure mode is not accidental exposure but
over-granting through carelessly wide configuration.

## The three layers, and how they compose

**Permissions** are descriptions of the explicit privileges of commands — they
enable or deny specific commands and can carry path scopes [3]. Identifiers follow
`<name>:default` or `<name>:<command-name>`, are ASCII-lowercase, and the
`tauri-plugin-` prefix is prepended at compile time [3]. Application permissions
live in `src-tauri/permissions/<identifier>.toml` (TOML only); plugins ship theirs
under `permissions/<identifier>.json|toml` [3]. Group related permissions into
*permission sets* under a new identifier for reuse [3].

**Command scopes** narrow *what a command may act on* — with `allow` and `deny`
lists where **deny always wins over allow** [4]. Scopes are serialized via `serde`
and passed to the command, which is responsible for enforcing them [4]. The fs
plugin scopes directory/file access with glob paths; the HTTP plugin scopes which
URLs are reachable [4].

**Capabilities** bind permissions to *frontend contexts* — specific windows and
platforms [5]. They live in `src-tauri/capabilities/` (file-based, inline in
`tauri.conf.json`, or hybrid) and each declares an identifier, target `windows`,
and `permissions` [5]. Permissions only take effect once referenced by a
capability [3].

## Least privilege per window and per platform

Give each window only the capabilities it needs [6]. Target windows by label with
the `windows` field, and gate a capability to specific operating systems with
`platforms` (any of `linux`, `windows`, `macos`, `android`, `ios`) [6]:

```json
{
  "identifier": "fs-read-home",
  "windows": ["first"],
  "platforms": ["linux", "windows"],
  "permissions": ["fs:allow-home-read"]
}
```

A splash window, a settings window, and a main window should not share one broad
capability just because it is convenient — that hands a frontend compromise in the
least-trusted window the union of everyone's privileges. Capabilities exist
precisely to minimize the blast radius of a compromised frontend and to prevent
exposure of local system interfaces and data [5].

## Scope precisely; prune defaults

The sharpest discipline is at the scope level. Rather than granting blanket access
like `fs:allow-write-text-file`, scope it to the exact path the feature needs [7]:

```json
{ "identifier": "fs:allow-write-text-file", "allow": [{ "path": "$HOME/test.txt" }] }
```

Base-directory variables (`$HOME`, `$APP`, `$RESOURCE`) keep paths portable [7].
Treat each plugin's `default` permission set as a starting point to *review and
trim*, not to accept wholesale — default fs permissions, for instance, grant all
read operations and `$APP` access [7]. The workflow: add the plugin, read its
default permissions, identify the specific commands you actually call, define
custom scopes, and reference them in a capability [7]. Attempting an unpermitted
operation fails loudly (`fs.write_text_file not allowed`), which is your signal
that a scope is correctly tight, not that something is broken [7].

## Know the boundaries of this model

Two limits matter. First, scope enforcement is only as good as the command's
implementation — "command developers need to ensure that there are no scope
bypasses possible," so custom scoped commands need careful auditing [4]. Second,
capabilities protect against a *compromised frontend*; they do not protect against
malicious or insecure Rust code or a compromised build environment [5]. For those,
layer on the CSP and isolation pattern
([/practices/csp-and-isolation.md](/practices/csp-and-isolation.md)) and treat the
whole lifecycle as in scope
([/practices/security-threat-model.md](/practices/security-threat-model.md)).
Tauri 2.4+ can even strip unused commands from the binary via
`removeUnusedCommands`, which pushes you toward listing only necessary commands in
ACL files — a security *and* size win
([/practices/binary-size-and-footprint.md](/practices/binary-size-and-footprint.md)).

# Citations

1. [Security overview](/sources/tauri-security-overview.md)
2. [Beyond Electron: Attacking Alternative Desktop Application Frameworks](/sources/bishopfox-beyond-electron.md)
3. [Permissions](/sources/tauri-security-permissions.md)
4. [Command Scopes](/sources/tauri-security-scope.md)
5. [Capabilities](/sources/tauri-security-capabilities.md)
6. [Capabilities for Different Windows and Platforms](/sources/tauri-learn-capabilities-multiwindow.md)
7. [Using Plugin Permissions](/sources/tauri-learn-plugin-permissions.md)
