# src-tauri/ — Tauri 2 desktop app

Wraps `agent-server` in a desktop shell. **A separate Cargo workspace** from
`agent/` — run cargo commands from this directory; `-p` from `agent/` cannot
reach these crates.

## Commands

From the **repo root**:

```bash
npm run desktop:dev
npm run desktop:build
```

## Gotchas

- CI (`scripts/ci.sh`) runs src-tauri clippy + tests only when GTK/WebKitGTK dev
  deps are present. Its fmt is never checked — src-tauri is hand-formatted by
  convention (compact hand-format, no `cargo fmt`).
- End-to-end GUI driving goes through WebDriver (tauri-driver + selenium) — see
  the `auto-drive-tauri` skill (`.agents/skills/auto-drive-tauri/`).
