# `tauri.conf.json`

The single source of truth for the Tauri side of the app, at
`src-tauri/tauri.conf.json`. JSON5 (`tauri.conf.json5`) and TOML
(`Tauri.toml`) variants are also supported. Keep the `$schema` line for editor
autocomplete and validation.

Platform overrides (`tauri.linux.conf.json`, `tauri.windows.conf.json`,
`tauri.macos.conf.json`) and `--config` values are merged via JSON Merge Patch
(RFC 7396) — see `cli.md`.

## Top-level shape

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "My App",
  "version": "0.1.0",
  "identifier": "com.example.myapp",
  "build": { /* frontend integration */ },
  "app": { /* windows + security + runtime behavior */ },
  "bundle": { /* packaging / installers */ },
  "plugins": { /* per-plugin config */ }
}
```

- **`productName`** — display name (window title default, installer name).
- **`version`** — app version; if omitted, taken from `Cargo.toml`. Can point to
  a `package.json` path to inherit the frontend version.
- **`identifier`** — reverse-DNS unique id (see `security.md`). Required; must
  not stay `com.tauri.dev`.

## `build` — frontend integration

```json
"build": {
  "frontendDist": "../dist",
  "devUrl": "http://localhost:1420",
  "beforeDevCommand": "npm run dev",
  "beforeBuildCommand": "npm run build",
  "beforeBundleCommand": ""
}
```

- **`frontendDist`** — path to built static assets used by `tauri build` (relative
  to `src-tauri/`), **or** a URL to load remotely.
- **`devUrl`** — URL of the dev server `tauri dev` waits for and loads.
- **`beforeDevCommand` / `beforeBuildCommand`** — shell commands run before
  dev/build (start the frontend dev server / produce `frontendDist`).

A `frontendDist`/`devUrl` mismatch with the actual frontend tool is the usual
cause of a blank window — see `troubleshooting.md`.

## `app` — windows, security, behavior

```json
"app": {
  "windows": [
    {
      "label": "main",
      "title": "My App",
      "width": 1000,
      "height": 700,
      "minWidth": 600,
      "resizable": true,
      "fullscreen": false,
      "center": true,
      "decorations": true,
      "transparent": false
    }
  ],
  "security": {
    "csp": "default-src 'self'",
    "assetProtocol": { "enable": true, "scope": ["$RESOURCE/**"] }
  },
  "withGlobalTauri": false,
  "trayIcon": { "iconPath": "icons/icon.png" }
}
```

- **`windows[]`** — initial windows. `label` is the identity referenced by
  capabilities and event targeting. Many window options exist (size limits,
  position, `alwaysOnTop`, `skipTaskbar`, `theme`, `titleBarStyle`, …).
- **`security`** — CSP, asset protocol scope, capability controls (see
  `security.md`).
- **`withGlobalTauri`** — expose `window.__TAURI__` for bundler-free frontends.
- **`trayIcon`**, **`macOSPrivateApi`**, etc. — runtime features.

Windows can also be created at runtime in Rust (`WebviewWindowBuilder`) instead
of declaring them here.

## `bundle` — packaging

```json
"bundle": {
  "active": true,
  "targets": "all",
  "icon": [
    "icons/32x32.png",
    "icons/128x128.png",
    "icons/icon.icns",
    "icons/icon.ico"
  ],
  "category": "Productivity",
  "shortDescription": "...",
  "longDescription": "...",
  "resources": [],
  "externalBin": [],
  "copyright": "",
  "linux": { "deb": { "depends": [] } },
  "macOS": { "minimumSystemVersion": "10.13", "signingIdentity": null },
  "windows": { "wix": {}, "nsis": {} },
  "createUpdaterArtifacts": false
}
```

- **`targets`** — `"all"`, or an array like `["deb", "appimage", "nsis", "dmg"]`.
  Tauri only builds targets valid for the host OS.
- **`icon`** — icon set used by installers (generate with `tauri icon`).
- **`resources`** — extra files bundled and accessible at runtime via
  `$RESOURCE`.
- **`externalBin`** — sidecar executables shipped with the app.
- **`createUpdaterArtifacts`** — emit signed update artifacts for the updater
  plugin.
- Platform sub-objects (`linux`/`macOS`/`windows`) hold packaging + signing
  options (see `distribution.md`).

## `plugins` — per-plugin config

Plugins read their settings from here, keyed by plugin name:

```json
"plugins": {
  "updater": {
    "endpoints": ["https://releases.example.com/{{target}}/{{current_version}}"],
    "pubkey": "<updater public key>"
  },
  "deep-link": { "desktop": { "schemes": ["myapp"] } }
}
```

Adding a plugin = `tauri add <name>` (or manual Cargo + JS install + `register`
call in `lib.rs`) + config here (if any) + permissions in a capability file.
