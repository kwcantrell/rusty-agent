# Design Spec — Settings Capability (browser-driven live daemon reconfiguration)

**Date:** 2026-06-23
**Status:** Approved design. Next step: `writing-plans` → implementation plan.
**Primer:** none — this subsystem was deferred *out of* the React frontend (#6); see that spec's
"Out of scope" note in [`2026-06-23-react-frontend-design.md`](./2026-06-23-react-frontend-design.md) §1.
**Depends on:** the merged Cloudflare control plane (`cloud/`) and React frontend (`web/`) being live.
**Touches:** `agent-server` (daemon), `agent-runtime-config` (shared wiring), `web/` (UI).
**Does NOT touch:** the core crates (`agent-core`/`agent-model`/`agent-tools`/`agent-policy`) or `cloud/`.

## 1. Purpose & scope

A **browser-driven settings channel** that lets a paired operator reconfigure the *running* local
daemon — its model/inference backend, its command policy, and its loop tuning — **without restarting
the daemon or dropping the WebSocket**. Today every one of these is a launch-time flag on
`agent-serverd run` (`--backend`, `--base-url`, `--model`, `--protocol`, `--context-limit`, the
`RulePolicy` allow/deny lists, the `LoopConfig` knobs); changing any of them means killing and
relaunching the daemon. This subsystem adds a new **inbound daemon-config channel** so the browser
can edit them live.

### Scope (what is editable)

- **Model & inference:** `backend` (`openai` | `claude-cli`), `base_url`/endpoint, `model` name,
  `protocol` (`native` | `prompted`).
- **Command policy:** the `RulePolicy` command **allowlist** and the **editable portion** of the
  **denylist** (see §5 — a hard floor is always enforced on top).
- **Loop tuning:** `temperature`, `max_tokens`, `max_turns`, `context_limit` (the `LoopConfig`
  `model_limit`).

### Exposed as read-only metadata (shown, never edited here)

- **workspace** path (display only).
- **hard-floor denylist** (so the operator can see what is *always* blocked — see §5).
- **`api_key_set`** — a masked boolean indicator that an API key is configured server-side.

### Out of scope (deferred, each its own future cycle)

- **Workspace editing.** Changing the workspace live interacts badly with the lexical path guard,
  a `WindowContext` already populated against the old workspace, and in-flight runs. It deserves its
  own treatment.
- **MCP settings.** The MCP client (subsystem #3) is not built; there is nothing to configure yet.
- **Secrets over the wire.** The API key stays server-side (`AGENT_API_KEY` env / launch). The
  channel never sends or accepts it (see §5).

### Invariants (hold the #5/#6 bar)

1. **Zero `cloud/` changes.** The `AgentSession` Durable Object relays browser↔daemon frames
   transparently (`cloud/src/session.ts` fans agent→browser to *all* browsers and forwards
   browser→daemon for *any* frame, special-casing only `event` for R2 persistence and `presence`).
   New `settings_*` frame kinds flow through untouched. Because they are **not** `kind:"event"`, they
   are neither persisted to R2 nor replayed — exactly right (settings are fetched fresh, not replayed).
2. **Zero changes to the core crates.** `agent-core`/`agent-model`/`agent-tools`/`agent-policy` are
   unmodified. Live reconfiguration is achieved by *rebuilding* an `AgentLoop` from the existing seams
   and atomically swapping it — `AgentLoop::new` only stores `Arc`s, so this is cheap.

## 2. Architecture

```
web/ SettingsPanel ──▶ Worker ──relay──▶ Durable Object ──WS──▶ daemon (agent-server)
   settings_get / settings_update            (transparent; settings_* are not "event" frames)
   ◀── settings_state / settings_error ◀──── broadcast to all browsers
```

The daemon stops holding a single immutable `Arc<AgentLoop>`. Instead it holds a small **runtime
holder**: a `Mutex<Arc<AgentLoop>>` (the current loop) plus the current `RuntimeConfig` and the
*persistent* pieces that survive a reconfigure — `WsEventSink`, `WsApprovalChannel`, the
`WindowContext`, the workspace path, and the runtime-config file path. A settings change rebuilds the
loop's *parts* (model, protocol, registry, policy, `LoopConfig`) from the new config and swaps the
`Arc` in the cell. The next `user_input` picks up the new loop; any in-flight run keeps the `Arc` it
already cloned and finishes on its old config (see §6, "Apply timing").

## 3. Wire additions (`agent-server/src/wire.rs`)

Additive variants on the existing `#[serde(tag="kind", rename_all="snake_case")]` `WireBody` enum and
a small payload struct. The protocol version stays `1` (purely additive; older browsers simply never
send the new kinds).

```rust
// New inbound variants (browser → daemon)
SettingsGet,                                   // request current effective settings
SettingsUpdate { settings: RuntimeSettings },  // apply a new full settings object

// New outbound variants (daemon → browser)
SettingsState {
    settings: RuntimeSettings,                 // current effective editable settings
    workspace: String,                         // read-only display
    api_key_set: bool,                         // masked indicator
    hard_floor: Vec<String>,                   // read-only, always-enforced denials
},
SettingsError { message: String },             // validation/apply failure; no swap happened
```

`RuntimeSettings` is the **full editable surface** (not a patch): `backend`, `base_url`, `model`,
`protocol`, `command_allowlist`, `command_denylist` (the editable portion only), `temperature`,
`max_tokens`, `max_turns`, `context_limit`. The UI always holds the full current state, so
full-object replace is simpler than patch-merge and avoids partial-update ambiguity (YAGNI for a
single-operator tool).

`settings_state` is sent (a) in reply to `settings_get`, and (b) broadcast after every successful
apply — the DO fans agent→browser frames to *all* browsers, so multiple tabs stay in sync for free.

## 4. Config model & persistence (`agent-runtime-config`)

A new **`RuntimeConfig`** type owns the editable surface, its serde, validation, defaults, and
on-disk persistence. (`RuntimeSettings` on the wire mirrors its editable fields; `RuntimeConfig` is
the daemon-side home of the same data plus load/save.)

- **New flag:** `--runtime-config <path>` on `agent-serverd run`, default `agent-runtime.json`,
  **separate** from the enrollment `agent-server.json` (which keeps holding only credentials and is
  untouched).
- **Startup precedence (flags seed, file wins):** build a `RuntimeConfig` from the launch flags
  (`--backend`, `--base-url`, `--model`, `--protocol`, `--context-limit`, plus loop-tuning defaults
  and the default allow/deny lists), then if the file exists, deserialize it and merge **per field**
  over the flag defaults (file wins). Per-field merge gives graceful schema evolution — a file written
  by an older build is missing new fields, which fall back to the flag/default value.
- **Apply (persist-then-swap):** on `settings_update`, `validate()` the incoming `RuntimeSettings`;
  on success, write the file, then swap the rebuilt loop into the cell and broadcast `settings_state`.
  Persist-then-swap keeps disk and memory consistent: if the file write fails, emit `settings_error`
  and do **not** swap.
- **`validate()` rules:** `backend ∈ {openai, claude-cli}`; `protocol ∈ {native, prompted}`;
  `backend == claude-cli` ⇒ protocol is forced to `prompted` (the daemon already does this at launch —
  validation centralizes it); `base_url` non-empty when `backend == openai`; numeric bounds
  `0.0 ≤ temperature ≤ 2.0`, `max_tokens > 0`, `max_turns ≥ 1`, `context_limit ≥` a sane floor
  (e.g. 1024). Invalid input → `settings_error`, no persist, no swap.

## 5. Security posture

The auth model is pairing-based: whoever holds the pairing code already drives the agent and can
`ApproveAlways` arbitrary commands. So an operator editing the policy is **not** a new privilege.
The denylist's real job is protecting against **the model** (or an injected settings frame) doing
something catastrophic, not against the operator. Two rules encode that:

- **Hard-floor denylist (immutable).** `agent-runtime-config` defines a `HARD_FLOOR_DENYLIST` const —
  `rm -rf /`, `sudo`, `:(){`, `mkfs`, `dd if=`. The effective denylist handed to `RulePolicy` is
  `HARD_FLOOR_DENYLIST ∪ user_denylist` (deduped). The browser only ever edits the **non-floor**
  portion; even a cleared user denylist still enforces the floor. `settings_state.hard_floor` surfaces
  it read-only so the operator sees what is always blocked.
- **Secrets never traverse the wire.** The API key stays `AGENT_API_KEY`/launch-only. `settings_*`
  frames never carry it; `settings_state.api_key_set` is a masked boolean only.

## 6. Daemon integration (`agent-server`)

`daemon.rs` plus a new module (e.g. `runtime.rs`) that owns the holder + rebuild/swap logic.

- **Runtime holder:** replace the single `Arc<AgentLoop>` with a struct holding
  `current: Mutex<Arc<AgentLoop>>`, the current `RuntimeConfig`, and the persistent pieces
  (`WsEventSink`, `WsApprovalChannel`, `workspace`, `WindowContext`, runtime-config path). Uses
  `std::sync::Mutex` — the lock is only held to clone an `Arc` out (or store one in), never across an
  `.await`, mirroring the existing `Arc<Mutex<String>>` session pattern.
- **`build_loop(config, persistent…) -> Arc<AgentLoop>`:** assembles `build_model` + `pick_protocol`
  + `build_registry` + `RulePolicy { workspace, command_allowlist, effective_denylist }` +
  `LoopConfig` from a `RuntimeConfig`, reusing the persistent sink/approval. This is the one place
  that turns config into a loop; called once at startup and again on each apply.
- **Read-loop arms (additive to the existing `user_input` / `approval_response`):**
  - `settings_get` → build `settings_state` from the current `RuntimeConfig` + metadata; send via the
    writer channel.
  - `settings_update` → `validate()` → persist → rebuild → store into the cell → broadcast
    `settings_state`; on any failure send `settings_error` and leave the current loop intact.
  - `user_input` → lock the cell, clone the current `Arc<AgentLoop>`, unlock, spawn the run with that
    clone (so a concurrent swap never affects an in-flight run).
- **Apply timing — next turn, no interrupt.** A swap replaces the cell's `Arc`; the in-flight run, if
  any, already holds its own clone and finishes on its old config. The `WindowContext` is persistent,
  so conversation continuity is preserved across swaps (a model/protocol change just applies to
  subsequent turns over the same history).

## 7. Frontend (`web/`)

- **`wire.ts`:** add the two outbound (`settings_get`, `settings_update`) and two inbound
  (`settings_state`, `settings_error`) frame types, mirroring the Rust shapes.
- **`state.ts`:** `ConversationState` gains `settings: SettingsView | null` and
  `settingsError: string | null`. Reducer: `settings_state` → store settings + metadata, clear error;
  `settings_error` → set error. (Settings frames do not affect the message `items` stream.)
- **`SettingsPanel` component** (modal/drawer), opened from a gear in `StatusBar`:
  - On open, sends `settings_get`; renders once `settings_state` arrives.
  - Three grouped sections — **Model & inference**, **Command policy** (allowlist + editable
    denylist), **Loop tuning** — plus read-only **hard-floor denylist**, **workspace**, and an
    **"API key set"** badge.
  - Light client-side validation (mirrors §4 bounds) for snappy feedback; the **server is
    authoritative**. Save sends `settings_update`; `settings_error` renders inline; the broadcast
    `settings_state` echo confirms the applied state.
  - Disabled while the daemon is offline (presence) or the socket is not open.

## 8. Testing

**Rust (`agent/`, `cargo test --workspace`; `clippy --all-targets -- -D warnings` clean):**
- Wire round-trip for the four new `WireBody` variants (tag names, optional fields).
- `RuntimeConfig::validate` — enum/bounds rejection; `claude-cli` ⇒ protocol forced to `prompted`;
  `base_url` required for `openai`.
- Effective denylist always contains the hard floor, even when the user denylist is empty/cleared.
- `RuntimeConfig` load/save round-trip and **file-overrides-flags-per-field** precedence (incl. a file
  missing a field falling back to the flag default).
- Cell-swap unit test: after a swap, the `Arc` a subsequent `user_input` would clone is the new loop,
  and a previously-cloned `Arc` is unaffected.

**Web (`web/`, Vitest + jsdom):**
- Reducer: `settings_state` stores settings/metadata + clears error; `settings_error` sets error.
- `SettingsPanel` (RTL): renders from `settings_state`, edits fields, emits `settings_update` on save,
  shows `settings_error`, disabled when offline; hard-floor + workspace render read-only.
- Wire parse of captured sample `settings_state`/`settings_error` frames (lock the TS↔Rust contract).

**Manual chrome E2E (deferred to the human, reusing the documented bring-up):**
live-switch `openai`(llama.cpp)↔`claude-cli` and confirm the next turn uses the new backend; edit the
allowlist and watch a previously-gated command auto-approve; tune a loop param; restart the daemon and
confirm settings persisted; confirm the hard floor cannot be removed.

## 9. Definition of done

From the browser, a paired operator opens Settings, switches backend/model/endpoint, edits the
allow/denylist, and tunes loop params; changes take effect on the **next turn** without dropping the
socket, **persist across a daemon restart**, the **hard floor is always enforced**, and **secrets
never leave the server**. `agent/` and `web/` test suites green; `cloud/` tests still green (**zero
`cloud/` changes**); **core crates untouched**; manual chrome E2E verified.
