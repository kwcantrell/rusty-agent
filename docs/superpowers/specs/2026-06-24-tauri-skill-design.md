# Design: generic `tauri` agent skill

**Date:** 2026-06-24
**Status:** Approved (design), pending spec review

## Goal

Create a reusable, repo-agnostic agent skill under `.agents/skills/` that equips
any agent to build, modify, and debug **Tauri v2 desktop apps**
(Windows/macOS/Linux) in **any** repository. No references to this repo or any
specific application.

Grounded in a verified sweep of the official Tauri v2 docs (`v2.tauri.app`):
prerequisites, `create-tauri-app`/`tauri init`, project structure, the `tauri`
CLI surface, the Rust↔JS IPC model, the capabilities/permissions/ACL security
model, `tauri.conf.json`, and desktop bundling/signing.

## Scope

- **In scope:** desktop development (Windows/macOS/Linux) end to end —
  prerequisites, scaffolding, dev loop, IPC, security/capabilities,
  configuration, bundling + code signing, troubleshooting.
- **Out of scope (brief mention only):** mobile (android/iOS) dev/build. Noted
  as "Tauri v2 also targets mobile; out of scope for this skill" without detail.
- Frontend-framework-agnostic and package-manager-agnostic (show npm / pnpm /
  cargo variants where they differ).

## Structure

One comprehensive skill with progressive disclosure via a `references/` subdir:

```
.agents/skills/tauri/
  SKILL.md                  # hub: when-to-use, prereqs, scaffold, dev loop,
                            #   core IPC pattern, the #1 security gotcha, build —
                            #   plus a decision table linking to references/
  references/
    cli.md                  # full `tauri` CLI: dev/build/init/info/icon/signer/
                            #   migrate/plugin/permission/add + common flags
    ipc.md                  # commands (#[tauri::command]), invoke, args,
                            #   return/errors, async, events, State management
    security.md             # capabilities/, permissions, ACL, CSP, identifiers
    configuration.md        # tauri.conf.json: windows, bundle, security, app id
    distribution.md         # desktop bundling + code signing per platform, updater
    troubleshooting.md      # common pitfalls/gotchas
```

Each reference is self-contained and loaded only when the task needs it.

## SKILL.md content shape

- **Frontmatter:** `name: tauri`; `description` written for retrieval — mentions
  Tauri, desktop app, Rust + system webview, `tauri.conf.json`, `invoke`/IPC,
  capabilities. The terms a triggering task would contain.
- **When to use / when not to** — use when building/modifying a Tauri desktop
  app; not for pure-web or pure-Rust projects.
- **Prerequisites** quick-check per OS (Rust toolchain; system webview deps such
  as `libwebkit2gtk-4.1-dev` + `build-essential` on Debian/Ubuntu; Xcode CLT on
  macOS; MSVC Build Tools + WebView2 on Windows).
- **Scaffolding** — `create-tauri-app` (new) vs `tauri init` (add to existing
  frontend); the two-part layout: frontend at root + `src-tauri/` (Rust).
- **The dev loop** — `tauri dev` / `tauri build`, what each does, dev-server
  expectation (`devUrl` / `frontendDist`).
- **Core IPC pattern** — one minimal end-to-end example (Rust
  `#[tauri::command]` registered in `invoke_handler` → `invoke()` from JS), then
  "see references/ipc.md".
- **The #1 security gotcha** — in v2 a command/plugin API is inert from the
  frontend until a **capability** grants its **permission**; short example, then
  "see references/security.md".
- **Decision table** — "want to do X → read references/Y.md".
- **Verification** — confirm with `tauri info`, a successful `tauri build`, and a
  round-trip `invoke` call.

## Reference file contents (summary)

- **cli.md** — every subcommand with purpose + key flags; `tauri.conf.json` +
  platform-override merge (RFC 7396); `--` passthrough to the frontend; `info`
  for diagnostics; `add` for plugins; `migrate` for v1→v2; `permission`/`signer`.
- **ipc.md** — command signature, JSON arg passing (camelCase ↔ snake_case),
  `Result<T, E>` error handling with `serde::Serialize` errors, async commands,
  injected types (`AppHandle`, `Window`, `State`), events (`emit`/`listen`),
  and `State` management with `manage`.
- **security.md** — the ACL model: permissions (allow/deny on commands),
  capabilities files in `capabilities/`, assigning windows/webviews, scopes,
  enabling plugin permissions, CSP in `tauri.conf.json`, the app identifier.
- **configuration.md** — `tauri.conf.json` top-level keys (`identifier`,
  `build`, `app`, `bundle`, `plugins`), window options, bundle targets,
  `security`/CSP, and where platform overrides live.
- **distribution.md** — `tauri build` outputs per platform (`.app`/`.dmg`,
  `.msi`/`.exe` (NSIS), `.deb`/`.rpm`/`.AppImage`), code signing on macOS
  (notarization) and Windows, and the updater at a high level.
- **troubleshooting.md** — frequent failures: command "not allowed" (missing
  capability/permission), CSP blocking assets, blank window (wrong
  `frontendDist`/`devUrl`), missing Linux webkit deps, dev-vs-build asset paths,
  v1 allowlist removed in favor of capabilities.

## Non-goals / YAGNI

- No mobile detail, no CI/CD recipes, no framework-specific tutorials, no
  scripts/tooling — pure markdown knowledge + actionable steps.

## Verification

- Skill files exist under `.agents/skills/tauri/` with valid frontmatter.
- No occurrence of this repo's name or app-specific paths in any skill file.
- Commands, file names, and config keys match the verified Tauri v2 docs.
