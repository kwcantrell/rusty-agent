---
name: tauri
description: >-
  Use when building, modifying, or debugging a Tauri v2 desktop application
  (Windows/macOS/Linux) — a cross-platform app with a web frontend and a Rust
  backend rendered in the OS system webview. Covers prerequisites, scaffolding
  with create-tauri-app / tauri init, the src-tauri layout and tauri.conf.json,
  the tauri CLI (dev/build/info/add/migrate), the Rust↔JS IPC model
  (#[tauri::command] + invoke + events + State), the v2 capabilities/permissions
  security model, configuration, and bundling/code-signing. Trigger on mentions
  of Tauri, tauri.conf.json, #[tauri::command], invoke(), capabilities, or
  "desktop app in Rust + web".
---

# Building Tauri v2 desktop apps

Tauri v2 builds a desktop app from a **web frontend** (any framework, or plain
HTML/JS) plus a **Rust backend**. The frontend renders in the OS-provided
webview (WebView2 on Windows, WKWebView on macOS, WebKitGTK on Linux) — Tauri
does **not** bundle Chromium, which keeps binaries small. The Rust side exposes
native capability through a typed IPC bridge.

## When to use this skill

- Use when the repo is (or is becoming) a Tauri desktop app: there's a
  `src-tauri/` directory, a `tauri.conf.json`, `@tauri-apps/*` deps, or the user
  asks to "wrap this web app as a desktop app in Rust."
- **Do not** use for a pure web app (no native shell) or a pure Rust CLI/server.
- Mobile note: Tauri v2 also targets Android/iOS from the same project, but
  mobile is **out of scope** for this skill — this skill is desktop-only.

This file is the hub. For depth, read the matching file in `references/` (see the
decision table at the bottom). Pull in only what the current task needs.

## 1. Prerequisites (check first)

Run `tauri info` in an existing project to audit the toolchain. For a fresh
machine:

- **Rust** — install via <https://rustup.rs> (`rustup`); this provides `cargo`.
- **Node.js** — only if the frontend is a JS/TS project (most are). Any package
  manager works (npm / pnpm / yarn / bun).
- **System dependencies (the part people miss):**
  - **Linux (Debian/Ubuntu):** `libwebkit2gtk-4.1-dev`, `build-essential`,
    `curl`, `wget`, `file`, `libxdo-dev`, `libssl-dev`,
    `libayatana-appindicator3-dev`, `librsvg2-dev`. Other distros have
    equivalents (e.g. `webkit2gtk4.1` on Fedora/Arch).
  - **macOS:** Xcode Command Line Tools — `xcode-select --install`.
  - **Windows:** Microsoft C++ Build Tools (MSVC toolchain) and WebView2
    Runtime (preinstalled on Windows 10/11; otherwise install Evergreen WebView2).

Linux webview deps are the #1 first-build failure. If `tauri dev` fails to
compile with a `webkit2gtk` / `glib` / `soup` error, the system packages are
missing.

## 2. Scaffolding

**New project** — `create-tauri-app` scaffolds frontend + `src-tauri/` with a
chosen template (vanilla, React, Vue, Svelte, Solid, Angular, Yew, Leptos, …):

```bash
npm create tauri-app@latest        # or: pnpm create tauri-app
# yarn create tauri-app  |  bun create tauri-app
# Rust-only path:
cargo install create-tauri-app --locked && cargo create-tauri-app
```

**Add Tauri to an existing frontend** — install the CLI and run `init`:

```bash
npm install -D @tauri-apps/cli@latest
npx tauri init                     # interactive: app name, window title, devUrl, frontendDist
npm install @tauri-apps/api@latest # JS bindings for invoke/events/etc.
```

### Project layout

```
my-app/
├─ <frontend at the repo root>      # package.json, index.html, src/, vite.config, …
└─ src-tauri/                       # the Rust application
   ├─ Cargo.toml                    # Rust deps (tauri, plugins) + bin/lib config
   ├─ build.rs                      # runs tauri-build (codegen for context/permissions)
   ├─ tauri.conf.json               # THE Tauri config (see references/configuration.md)
   ├─ src/
   │  ├─ main.rs                    # desktop entry point; calls the lib's run()
   │  └─ lib.rs                     # app setup (Builder, invoke_handler); mobile entry too
   ├─ capabilities/                 # security: which windows may call which commands
   │  └─ default.json
   ├─ icons/                        # app icons (generate with `tauri icon`)
   └─ gen/                          # generated schemas/bindings — do not hand-edit
```

The frontend and the Rust backend are two halves of one app. The frontend builds
to static assets that Tauri serves; **server-side rendering is not supported** —
the app is a static web host plus native Rust.

## 3. The dev loop

```bash
npm run tauri dev      # or: npx tauri dev / cargo tauri dev
npm run tauri build    # production build + platform bundles
```

- `tauri dev` runs `beforeDevCommand` (your frontend dev server), waits for
  `build.devUrl`, then launches the app with hot-reload of the webview and
  recompile-on-change of Rust.
- `tauri build` runs `beforeBuildCommand`, expects static output at
  `build.frontendDist`, compiles Rust in release, and produces installers.

If the window is blank, `devUrl`/`frontendDist` is almost always wrong — see
`references/troubleshooting.md`.

## 4. Core IPC pattern (the one thing to internalize)

The frontend calls Rust through **commands**. Define, register, invoke:

```rust
// src-tauri/src/lib.rs
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {name}! You've been greeted from Rust.")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![greet]) // register every command here
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

```js
// frontend
import { invoke } from '@tauri-apps/api/core';
const message = await invoke('greet', { name: 'World' }); // JS camelCase → Rust snake_case
```

Key rules: arguments are passed as a JS object and map **camelCase → snake_case**;
return any `serde::Serialize` type; return `Result<T, E>` (with a `Serialize`
error) to reject the promise; mark `async fn` for non-blocking work. Full model
(events, `State`, injected `AppHandle`/`Window`) → `references/ipc.md`.

## 5. The #1 security gotcha

In Tauri v2 a command — including every plugin API — is **inert from the
frontend until a capability grants its permission.** A correctly written command
that compiles fine will be rejected at runtime ("not allowed") if no capability
covers it. Grant it in a capabilities file:

```json
// src-tauri/capabilities/default.json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Capabilities for the main window",
  "windows": ["main"],
  "permissions": ["core:default", "shell:allow-open"]
}
```

Your **own** `#[tauri::command]`s registered via `invoke_handler` are callable by
default in dev, but plugin and core-API access is gated by permissions like
`fs:allow-read-text-file` or `shell:allow-open`. Full ACL model → `references/security.md`.

## 6. Decision table — where to read next

| Task | Read |
|------|------|
| Look up a `tauri` CLI command or flag | `references/cli.md` |
| Add/return data between JS and Rust, events, app state | `references/ipc.md` |
| Fix "not allowed", grant permissions, set CSP, scopes | `references/security.md` |
| Edit `tauri.conf.json` (windows, bundle, identifier, plugins) | `references/configuration.md` |
| Add an official plugin (fs, dialog, store, http, …) | `references/security.md` + `references/cli.md` (`tauri add`) |
| Bundle / code-sign / ship an installer / updater | `references/distribution.md` |
| Something is broken | `references/troubleshooting.md` |

## 7. Verify your work

- `tauri info` — toolchain + project diagnostics are sane.
- `tauri build` (or `tauri dev`) compiles and launches without error.
- A round-trip `invoke('your_command', {...})` returns the expected value (and a
  failing case rejects), confirming the command is registered **and** permitted.
