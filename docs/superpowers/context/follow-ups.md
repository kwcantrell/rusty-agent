# Review Follow-ups Ledger (project-wide)

Durable, committed home for review findings from each subagent-driven-development cycle.
Populated at the end of every `/next-spec` subsystem cycle (the `.superpowers/sdd/` progress
ledger is gitignored scratch — findings live HERE). Convention mirrors
[`claude-cli-inference.md`](./claude-cli-inference.md) → "Follow-ups / known limitations":
each item has a file:line ref, a status (**Open** / **Accepted (won't-fix)** / **Resolved**),
and a one-line reason. The per-backend claude-cli list remains the source of truth for
claude-cli detail; this file is the project-wide index.

---

## 2026-06-23 — Settings capability (browser-driven live daemon reconfiguration)

Spec: [`../specs/2026-06-23-settings-capability-design.md`](../specs/2026-06-23-settings-capability-design.md) ·
Plan: [`../plans/2026-06-23-settings-capability.md`](../plans/2026-06-23-settings-capability.md) ·
Merged to `main` at `6133ff4`. Final whole-branch review (opus): "Ready to merge, with fixes" — all findings Minor, no Critical/Important.

### Accepted (won't-fix / deferred)
- **`effective_denylist` dedup is O(n²)** — `agent/crates/agent-runtime-config/src/runtime_config.rs` (`effective_denylist`). Uses `Vec::contains` per insert; fine for config-size lists, not a hot path. Reason: not worth an `IndexSet` for a handful of entries.
- **`validate()` doesn't independently reject `claude-cli` + `native`** — `agent/crates/agent-runtime-config/src/runtime_config.rs` (`validate`). Relies on the documented `normalized()`-first contract (`apply`/`new` always normalize before validating). Reason: contract is enforced at the only call sites + doc-commented.
- **`load_over` swallows all I/O errors like a missing file** — `agent/crates/agent-runtime-config/src/runtime_config.rs` (`load_over`). Permission/other I/O errors return the flag base, same as ENOENT (brief-mandated). Reason: acceptable; a future hardening pass could split `ErrorKind::NotFound` from other errors.
- **`SettingsPanel` keeps its opened snapshot if the server pushes a new `settings_state` while open** — `web/src/components/SettingsPanel.tsx` (`useState(settings)` init, no re-sync effect). Reason: closing and reopening the panel refreshes; standard modal-form pattern; not a spec requirement.
- **`App.tsx` uses an inline `import("./wire").RuntimeSettings` type** — `web/src/App.tsx` (`saveSettings` param). Reason: style only; a top-level `import type` would be cleaner; tsc accepts it, no runtime difference.
- **Hard-floor / user-denylist overlap not visually flagged** — `web/src/components/SettingsPanel.tsx` (hard-floor display). If a user denylist entry also appears in the hard floor, there's no visual indication of overlap. Reason: cosmetic; not a spec requirement.

### Resolved (during the cycle, kept for context)
- **Daemon `user_input` arm lost integration coverage** after the forced `daemon_roundtrip.rs` rewrite → added a model-free `user_input` smoke test (fail-fast base_url + then `settings_get`→`settings_state` proves the read loop survived). Commit `c9a6e5a`.
- **Settings gear not disabled when daemon offline** (spec §7) → `StatusBar` gained `settingsDisabled?`; `App` passes `!(connected && state.online)`. Commit `b72c1fb`.
- **Daemon catch-all stamped the session id for every unhandled frame** → scoped stamp + `handle()` to `SettingsGet | SettingsUpdate`, `_ => {}` otherwise. Commit `b72c1fb`.
- **`settings_state` wire test didn't round-trip-deserialize** → added deserialize + `matches!` assertion mirroring the error half. Commit `b72c1fb`.
- **Duplicate `import` in `web/test/wire.test.ts`** (caused `tsc TS2300`, blocking `npm run build`) → merged the imports. Commit `45709d6`.
