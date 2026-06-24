# Security: capabilities, permissions, ACL, CSP

Tauri v2 replaces v1's global `allowlist` with a fine-grained **Access Control
List (ACL)**. The mental model, smallest to largest:

- **Permission** — a named rule that *allows* or *denies* one or more commands,
  optionally with a **scope** (e.g. which paths a filesystem read may touch).
  Identified as `<plugin>:<name>`, e.g. `core:default`, `fs:allow-read-text-file`,
  `shell:allow-open`.
- **Permission set** — a bundle of permissions a plugin ships for convenience,
  e.g. `fs:default`.
- **Capability** — a JSON file in `src-tauri/capabilities/` that grants a list of
  permissions to specific **windows/webviews** (and, for remote content, specific
  origins). This is what actually turns access on.

> The core gotcha: a command or plugin API does nothing from the frontend until a
> **capability** grants a **permission** for it. "Command not allowed" almost
> always means a missing permission.

## A capability file

```json
// src-tauri/capabilities/default.json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Permissions for the main window",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "shell:allow-open",
    "dialog:allow-open",
    {
      "identifier": "fs:allow-read-text-file",
      "allow": [{ "path": "$APPDATA/*" }]
    }
  ]
}
```

- `windows` — labels of the windows this capability applies to (glob allowed,
  e.g. `"main-*"`). Match the window `label` from `tauri.conf.json`.
- `permissions` — strings (whole permission) or objects (permission **with
  scope**). The scoped object form restricts a permission to specific resources
  using `allow`/`deny` lists.
- `platforms` (optional) — limit a capability to certain OSes, e.g.
  `["macOS", "windows", "linux"]`.
- `remote` (optional) — allow listed remote origins to use these permissions
  (off by default; only for windows that load remote URLs — do this carefully).

You can have multiple capability files; they're additive. Generate one with
`tauri capability new <name>` and add permissions with
`tauri permission add <identifier> --capability <name>`. List what a plugin
offers with `tauri permission ls`.

## Your own commands vs plugin/core APIs

- **Your `#[tauri::command]`s** registered in `invoke_handler` are callable from
  the frontend without a custom permission during development. (For hardening you
  can still define permissions for them.)
- **Core APIs and plugin APIs** (fs, shell, dialog, http, store, …) are gated:
  add the relevant `core:*` / `<plugin>:*` permission to a capability, or the
  call is rejected.

`core:default` enables a sensible baseline of core permissions; scaffolded apps
include it.

## Scopes

Scopes constrain *what* a permitted command may act on. Filesystem and HTTP are
the common cases:

```json
{ "identifier": "fs:allow-read-text-file", "allow": [{ "path": "$APPCONFIG/*" }] }
{ "identifier": "http:default", "allow": [{ "url": "https://api.example.com/*" }] }
```

`deny` takes precedence over `allow`. Path variables like `$APPDATA`,
`$APPCONFIG`, `$RESOURCE`, `$HOME` resolve at runtime to the right per-OS
location — prefer them over hardcoded paths.

## Content Security Policy (CSP)

Set the webview CSP in `tauri.conf.json` under `app.security.csp`. A strict CSP
is the main defense against injected/remote script in the webview:

```json
{
  "app": {
    "security": {
      "csp": "default-src 'self'; img-src 'self' asset: https://asset.localhost; script-src 'self'"
    }
  }
}
```

- `csp: null` (the scaffold default) disables CSP — fine for early dev, tighten
  before shipping.
- To load app-bundled files via `convertFileSrc`/the `asset:` protocol, include
  `asset:` (and on Windows `https://asset.localhost`) in the relevant directive.
- Tauri injects nonces/hashes for its own scripts when CSP is enabled.

Related `app.security` keys: `dangerousDisableAssetCspModification`,
`freezePrototype`, `assetProtocol` (enable + scope the `asset:` protocol),
`capabilities` (inline capabilities / restrict which capability files load).

## App identifier

`identifier` (top level of `tauri.conf.json`) is a reverse-DNS string, e.g.
`com.example.myapp`. It's the OS-level unique app id (bundle id / data dir / deep
links). Set it once; changing it later orphans existing user data. It must not be
the default `com.tauri.dev` for a real build.

## Migrating from v1 allowlist

`tauri migrate` converts the v1 `tauri.allowlist` config into v2 capability
files and updates plugin deps. After migrating, review the generated
`capabilities/` to ensure it grants only what the app needs.
