# Skills in `RuntimeConfig` — Design

**Status:** approved design (brainstorming → spec). Next step: `writing-plans`.
**Date:** 2026-06-23
**Attaches via:** `RuntimeConfig` (daemon disk + wire surface) + the existing `build_loop` reconfigure path + one additive `ContextManager`/`WindowContext` core method + web Settings UI.
**Core crates touched:** **`agent-core` only** — one additive method (`ContextManager::set_system` / `WindowContext::set_system`). This is the first deliberate core change since the core was frozen through #1/#5/#6; the missing seam *is* the feature (see §4). `agent-model`/`agent-tools`/`agent-policy` stay frozen.

---

## 1. Purpose & scope

Make the daemon's two skill settings — **`skills_dirs`** (the ordered skill roots) and **`active_skills`** (the preset skills folded into the system prompt) — part of `RuntimeConfig`, so they:

1. **Persist to disk** across restarts (via the existing `load_over`/`save`), and
2. **Round-trip on the wire** as part of `SettingsState`/`SettingsUpdate`, and
3. **Live-apply** mid-session (next-turn) when changed from the browser, and
4. are **editable from a browser Settings UI** (a discovered-skills checklist + a directories textarea).

This is the **full Settings capability for skills** — the ambition chosen during brainstorming over the smaller "persist + safe round-trip only" option.

### The follow-up this closes

From `docs/superpowers/context/follow-ups.md` (2026-06-23 skills-subsystem):

> **Persisting `skills_dirs`/`active_skills` into `RuntimeConfig`** + a browser Settings UI to edit them — Open — deferred to the Settings capability cycle. Persisting now (without matching web round-trip support) would let a browser settings-save silently wipe skill config, since `WireBody::SettingsUpdate` carries a full `RuntimeConfig`.

This cycle delivers exactly the "matching web round-trip support" that the deferral was waiting on, so persisting is now safe.

### In / out of scope

| Item | Status |
| --- | --- |
| `skills_dirs`/`active_skills` in `RuntimeConfig` (disk + wire) | **In scope** — the foundation. |
| Live-apply both fields mid-session (next-turn) | **In scope.** |
| Browser Settings UI: skill-dirs textarea + discovered-skills checklist | **In scope.** |
| `ContextManager::set_system` (the one core addition) | **In scope** — required for live system-prompt recompose. |
| **CLI** persistence of skills | **Out** — the CLI never uses `RuntimeConfig`; its `--skills-dir`/`--skill` stay launch-only and untouched. |
| Live skill **body** reload | **Already works** — `SkillRegistry::scan()` is per-invocation, so edited skill bodies are picked up live regardless. This cycle is about the configured *roots* and active *presets*. |
| New execution authority | **None** — unchanged from the skills subsystem; nothing here runs anything. |
| Sub-agent skills / OS-sandboxing | **Untouched** — remain deferred. |

---

## 2. Background — why this is daemon-only, and why the system prompt is the hard part

Established by reading the code:

- **The CLI builds everything straight from flags** (`agent-cli/src/main.rs`): registry, model, policy, and the composed system prompt are constructed inline from `cli.*`. The CLI **never constructs a `RuntimeConfig`** and never calls `load_over`/`save`. So "persist into `RuntimeConfig`" is entirely a **daemon** concern.
- **`RuntimeConfig` is the daemon's editable surface** — persisted (`load_over`/`save`, `runtime_config.rs`) *and* mirrored on the wire as `SettingsState`/`SettingsUpdate` (`agent-server/src/runtime.rs`). On every `SettingsUpdate`, `RuntimeState::apply` validates → persists → calls `build_loop` to swap the live `AgentLoop` atomically ("next-turn apply, no interrupt": an in-flight run keeps the loop `Arc` it already cloned).
- **`build_loop` already rebuilds the loop's `ToolRegistry`** on every reconfigure — but today it **ignores skills**: the 4 skill tools are pre-built once in `main.rs` and folded into the `mcp_tools` slice, and the composed system prompt (with presets) is pre-built once and stored in the per-session `WindowContext` created in `daemon::run`.
- **The system prompt lives inside the core's `WindowContext`** (`agent-core/src/context.rs`), in a **private `system` field with no setter**; the `ContextManager` trait exposes only `append` + `build`. The `ctx` is owned by `daemon::run` behind a `tokio::sync::Mutex`, created once and locked per turn. **There is no seam today to replace a session's system prompt** — that gap is what §4 fills.

Consequence: `skills_dirs` live-applies with no core change (rebuild the registry/tools in `build_loop`), but `active_skills` live-apply *requires* mutating the session system prompt → one additive core method.

---

## 3. Data model — `RuntimeConfig` (`agent-runtime-config/src/runtime_config.rs`)

Two new fields, additive, `#[serde(default)]` so older on-disk files load cleanly:

```rust
#[serde(default)] pub skills_dirs: Vec<String>,    // ordered roots; first = writable root
#[serde(default)] pub active_skills: Vec<String>,  // preset skill names folded into the system prompt
```

- **`PartialRuntimeConfig`** gains matching `Option<Vec<String>>` arms; `merge` sets each when `Some` (per-field fallback, like every other field).
- **`from_launch`** seeds both empty.
- **Validation:** `validate()` adds **nothing** for these fields. `skills_dirs` needs no check (`scan()` tolerates missing/unreadable roots → empty). `active_skills` validity is *registry-relative* (depends on what's discoverable), so it cannot be checked by a standalone `validate()`; it is validated in `apply()` where the registry is in hand (§5, validation split).
- **Seeding from flags (daemon `main.rs`):** mirror the existing `base.http_allow_hosts = allow_host;` with `base.skills_dirs = skills_dir;` and `base.active_skills = skill;`. Then `load_over` overlays the persisted file, so **persisted skills win over launch flags on restart** — consistent with every other field.

### Why the wipe risk is closed

`skills_dirs`/`active_skills` travel *inside* `RuntimeConfig` (the `settings` payload), so they round-trip like any other field. The wipe risk the deferral flagged is closed three independent ways: (a) `#[serde(default)]` on the Rust side; (b) the web form holds the received settings object and spreads it on save, so even an *old* web client that doesn't know the fields preserves them (the same mechanism by which `http_allow_hosts` survives today despite not being in the TS type); (c) the new UI explicitly edits and re-sends them.

---

## 4. Core change (`agent-core/src/context.rs`) — the one additive seam

```rust
// trait ContextManager
fn set_system(&mut self, system: Message);

// impl for WindowContext — replaces the private `system` field; history untouched.
fn set_system(&mut self, system: Message) { self.system = system; }
```

A few lines, fully additive. The inability to re-`system` a live session is a genuine `WindowContext` limitation, not just a skills concern — this is the kind of targeted improvement to the code we're working in that the seam-first architecture invites. It is called out in the spec as a **deliberate, minimal core extension** so the "zero core changes since #4" streak is broken knowingly, not silently.

**Test:** `set_system` swaps the system message; `build()` still emits it first; existing history is preserved across the swap.

---

## 5. Daemon wiring (`agent-server`)

### `build_loop` becomes the single place skills are built

- **`main.rs`** stops pre-building the skill tools and the composed prompt. The `mcp_tools` slice reverts to **MCP-only**. `DaemonParams.system_prompt` becomes the **base** prompt (`daemon::SYSTEM_PROMPT`), not a pre-composed one.
- **`build_loop`** (the one function that turns a `RuntimeConfig` into a loop):
  1. builds the registry as today (`build_registry` + register `mcp_tools`),
  2. additionally calls `build_skills(&cfg.skills_dirs, workspace)` → registers the 4 skill tools into that same registry,
  3. composes the system prompt via `compose_system_prompt(base, &registry, &cfg.active_skills)`.
  It returns the loop **and** the composed prompt (e.g. `(Arc<AgentLoop>, String)`, or a small struct) so the caller can store the prompt.
- **`RuntimeState`** stores the current composed system prompt (behind its existing mutex) and exposes `current_system_prompt() -> String`. `RuntimeState::new` and `apply` both update it from `build_loop`.

### Next-turn system-prompt apply (no cross-locking)

`apply()` runs **synchronously in the read loop** and must not touch the async `ctx` mutex. So `apply()` only updates `RuntimeState`'s stored loop + composed prompt. The **per-turn `UserInput` handler** in `daemon::run` — which already locks `ctx` — calls `ctx.set_system(Message::system(runtime.current_system_prompt()))` **before** `agent.run(...)`, every turn, unconditionally (a cheap `Message` clone; no generation tracking needed).

This reuses the **exact** "next-turn apply, no interrupt" semantics the loop swap already uses: an in-flight run keeps its old loop *and* old prompt; the next turn picks up both fresh. The model/temperature/etc. swap is already handled by `current_loop()`; the system prompt now rides the same discipline.

### Validation split — strict on the wire, lenient at startup

A deliberate asymmetry, to give good UI feedback without ever bricking the daemon:

- **On the wire (`apply`) — strict.** `apply()` builds the registry from `skills_dirs` and runs the compose with `active_skills`; an unknown preset name → `SettingsError`, **nothing changes** (config, loop, and prompt all unchanged). The user gets immediate feedback (and the checklist UI makes an unknown name hard to produce).
- **At startup (`RuntimeState::new` → `build_loop`) — lenient.** A *persisted* `active_skills` entry that is no longer discoverable (e.g. its dir was removed between runs) must **not** prevent the daemon from booting. Startup drops unknown presets with a `tracing::warn!` and boots with the base prompt + whatever presets resolve. (Implementation: a lenient compose variant, or pre-filter `active_skills` against `scan()` before a strict compose.)

---

## 6. Wire protocol (`agent-server/src/wire.rs` ↔ `web/src/wire.ts`)

Two carriers, two different homes:

- **`skills_dirs` + `active_skills`** ride **inside `RuntimeConfig`** (the `settings` payload of `SettingsState`/`SettingsUpdate`). No new wire variant; they round-trip like any other editable field.
- **`discovered_skills`** is a **read-only sibling field in the `SettingsState` frame**, alongside `workspace`/`api_key_set`/`hard_floor` — **not** in `RuntimeConfig`. `state_body()` computes it fresh each time: `SkillRegistry::from_config(&cfg.skills_dirs, &workspace).scan()` → a `Vec<{ name: String, description: String }>`. It is daemon-derived truth the browser cannot edit, so it never enters the persisted/round-tripped config (no wipe surface, no validation needed).

---

## 7. Web UI (`web/src`)

- **`wire.ts`** — `RuntimeSettings` gains `skills_dirs: string[]` and `active_skills: string[]`. The `SettingsState` inbound type (and the `Meta` mapping in `App.tsx`) gains `discovered_skills: { name: string; description: string }[]`.
- **`SettingsPanel.tsx`** — a new **"Skills"** `<section>`:
  - **Skill directories** — a textarea, one dir per line, using the existing `toLines`/`fromLines` helpers (identical pattern to the allow/deny lists). Empty lines filtered.
  - **Active skills** — a checklist rendered from `meta.discoveredSkills`. Each row: a checkbox + the skill `name` + its muted `description`. Checked ⇔ the name is in `active_skills`; toggling adds/removes the name in the form's `active_skills` array. A short note explains the save-then-appears two-step for newly added dirs ("save directories, then the skills they contain appear here to activate"). If `discoveredSkills` is empty, show a hint ("no skills found in the configured directories").
  - **Save** folds `skills_dirs` (from the textarea) and `active_skills` (from the checklist) into the outbound object alongside the existing `command_allowlist`/`command_denylist` handling.

---

## 8. Testing

- **`agent-runtime-config`** — extend the round-trip test so `skills_dirs`/`active_skills` save → `load_over` wins; a partial file falls back per-field; `from_launch` seeds both empty.
- **`agent-core`** — `set_system` unit test (swaps the system message; `build()` keeps it first; history intact).
- **`agent-server`** —
  - `apply()` rejects an unknown `active_skill` with `SettingsError` and leaves config/loop/prompt unchanged;
  - `apply()` with valid skills swaps the loop and updates `current_system_prompt()`;
  - startup is lenient — a persisted unknown preset boots with a warning, not a panic;
  - `state_body()` includes `discovered_skills` scanned from a temp skills dir;
  - a daemon round-trip proving a `SettingsUpdate` that changes `active_skills` reaches the **next** turn's `WindowContext` (the next-turn apply).
- **`web`** — `SettingsPanel.test.tsx`: renders the discovered-skills checklist; toggling a checkbox puts/removes the name in the saved `active_skills`; editing the dirs textarea round-trips; a save preserves all fields (including ones the UI doesn't edit).

---

## 9. Summary of seams touched

| Seam | Change |
| --- | --- |
| `RuntimeConfig` | +2 fields (`skills_dirs`, `active_skills`), `#[serde(default)]`; `PartialRuntimeConfig` + `merge`; flag seeding in daemon `main.rs`. |
| `ContextManager` / `WindowContext` | +`set_system` (the one additive core method). |
| `build_loop` (daemon) | becomes the single skills-build site: builds registry skill tools + composes the system prompt; returns the composed prompt. |
| `daemon::run` per-turn handler | applies `current_system_prompt()` to `ctx` before each run (next-turn discipline). |
| `SettingsState` wire frame | +`discovered_skills` (read-only, daemon-computed). |
| web `wire.ts` / `App.tsx` / `SettingsPanel.tsx` | +types; +Skills section (dirs textarea + discovered-skills checklist). |

The CLI's `--skills-dir`/`--skill` flags and the `agent-skills` crate internals are unchanged.
