# Troubleshooting & common pitfalls

Start with `tauri info` — it surfaces version mismatches and missing tooling
fast. Then match the symptom below.

## "Command <name> not allowed" / permission denied

The ACL is blocking the call. Either:
- It's a **plugin/core API** with no permission granted → add the permission to a
  capability file (`<plugin>:<name>` or `<plugin>:default`) and ensure the
  capability's `windows` includes the calling window's `label`. See `security.md`.
- It's **your own command** but not registered → add it to
  `tauri::generate_handler![...]` in `invoke_handler`.
- A **scope** is too narrow → the permission is granted but the path/URL isn't in
  its `allow` list (or is in `deny`).

## Blank/white window

- `build.devUrl` (dev) or `build.frontendDist` (build) doesn't match where the
  frontend actually serves/outputs. Confirm the dev server port equals `devUrl`
  and that `frontendDist` points at the real build output dir.
- Frontend uses **absolute** asset paths that don't resolve under Tauri. Set the
  frontend base to relative (e.g. Vite `base: './'`).
- App expects **SSR** — Tauri serves static files only; configure the framework
  for static export / SPA mode.
- Open the webview devtools (right-click → Inspect in a debug build, or
  `window.open_devtools()`) and check the console for load errors.

## Assets/images blocked, or "Refused to … Content Security Policy"

- The CSP in `app.security.csp` is rejecting a source. Add the needed directive
  (e.g. `img-src`, `connect-src`, `script-src`).
- Loading local files via `convertFileSrc`/`asset:` requires `asset:` in the CSP
  directive **and** `app.security.assetProtocol.enable: true` with a matching
  `scope`. On Windows also allow `https://asset.localhost`.
- During early dev you can set `csp: null` to unblock, then tighten before
  release.

## Linux build fails: `webkit2gtk` / `glib-2.0` / `javascriptcoregtk` not found

Missing system webview/build deps. Install them (Debian/Ubuntu):
`libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev
libayatana-appindicator3-dev librsvg2-dev`. Note v2 uses webkit2gtk **4.1**
(v1 used 4.0).

## `invoke` argument is `undefined` / `null` in Rust

camelCase ↔ snake_case mismatch. JS `{ filePath }` maps to Rust `file_path`. Make
the names line up, or use `#[tauri::command(rename_all = "snake_case")]` and pass
snake_case keys.

## Command compiles but the promise never resolves / blocks the UI

Long-running synchronous command blocking the runtime. Make it `async fn` (runs
off the main thread). For `State` held across `.await`, the async form is
required.

## `manage()` panics at startup

`.manage::<T>()` was called twice for the same type `T`. Register each state type
once; combine fields into one struct if needed.

## Works in `dev` but broken in `build` (or vice versa)

- Dev loads from `devUrl` (a live server); build loads `frontendDist` (static
  files). Asset paths, env vars, and API base URLs can differ — test both.
- `beforeBuildCommand` may not be producing `frontendDist`; verify the static
  output exists after it runs.

## Windows: app won't launch on another machine

Missing WebView2 Runtime (older Windows). Ship the Evergreen bootstrapper or use
Tauri's `bundle.windows.webviewInstallMode`.

## Upgrading from Tauri v1

Run `tauri migrate`. It updates deps, rewrites config, and converts the v1
`allowlist` to v2 capability files. The biggest conceptual change: there is **no
allowlist** in v2 — frontend access to APIs is granted by capabilities +
permissions. Re-audit the generated `capabilities/` afterward.

## General triage order

1. `tauri info` — toolchain sane?
2. Reproduce with `tauri dev` and open webview devtools — frontend or backend?
3. If backend: check the terminal running `tauri dev` for Rust panics/logs.
4. If "not allowed": it's the ACL — go to `security.md`.
5. If asset/CSP: it's `app.security` — go to `configuration.md`.
