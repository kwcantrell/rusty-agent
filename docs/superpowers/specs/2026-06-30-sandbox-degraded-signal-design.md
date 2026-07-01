# Surface Degraded-Sandbox Posture as a First-Class Signal — Design

**Date:** 2026-06-30
**Status:** Approved (brainstorming) → ready for plan
**Source:** Finding 1 (HIGH) of the harness-engineering audit re-run
(`.agents/skills/harness-engineering/audit.md`, 2026-06-30). The sibling HIGH,
Finding 2 (symlink-escape in `resolve_in_workspace`), is a self-contained fix
shipped separately and is out of scope here. Anchors re-verified against current
`main` on 2026-06-30.

## Principle

When the sandbox is configured to isolate but silently falls back to
unsandboxed host execution, that posture is a **security fact the operator must
see**, not a log line. Surface it as a first-class signal on every surface
(CLI, web, desktop), at both connect time and run start, so a
"we thought we were sandboxed" state cannot pass unnoticed. The signal fires
**only** when isolation was requested and not delivered — explicit host mode
(`degraded: None`) is a deliberate choice and stays silent.

## Finding being addressed (HIGH)

**Where:** `agent/crates/agent-sandbox/src/strategy.rs:57-65`.

In `auto` mode, when `DockerSandbox::probe()` reported the daemon unavailable,
`DockerSandbox::launch` degrades to `HostExecutor.launch(spec)` with only a
`tracing::warn!`. Every subsequent tool call then runs with ambient host
filesystem/network access while `RuntimeConfig` still claims isolation. The
`SandboxDescriptor.degraded: Option<String>` concept already exists and is
consumed at `loop_.rs` only inside the per-approval posture string — which never
fires on a run that hits no approval prompt. There is no run-level or
connect-level signal.

`enforce` mode already errors on unavailable Docker
(`strategy.rs:58`, `SandboxError::Unavailable`); it is correct and unchanged.

## Goal

Make the degraded posture impossible to miss:

- **CLI:** a loud line at run start.
- **Web / desktop:** a persistent, dismissible banner that appears the instant a
  client connects (from the settings snapshot) and is reinforced at run start
  (from a streamed event).

Non-goal: changing the sandbox fallback behavior itself. `auto`-degrades-to-host
is the intended resilience behavior; this design makes it *visible*, not
different. OS-level isolation remains the authoritative boundary
(`2026-06-23-os-sandboxing-docker-design.md`).

## Architecture & data flow

```
DockerSandbox (auto, docker down)
  describe().degraded = Some(reason), mechanism = "docker"
        │
        ├─ connect time ──▶ RuntimeState::settings_state()
        │                     reads current_loop().sandbox_descriptor()
        │                     → SettingsState.sandbox_degraded: Option<SandboxDegraded>
        │                     → wire → web reducer sets state → <SandboxBanner/>
        │
        └─ run start ─────▶ AgentLoop::run_with_cancel (emit once)
                              → AgentEvent::SandboxDegraded { mechanism, reason }
                              ├─ CLI  render.rs → loud yellow line
                              └─ wire server_event_from → ServerEvent::SandboxDegraded
                                   → web reducer sets state → <SandboxBanner/>
```

The Docker availability probe runs **once** at `build_sandbox` time
(`agent-runtime-config/src/lib.rs:220`, `DockerSandbox::probe()`).
`SandboxStrategy::describe()` only reads the cached `Availability`, so both the
connect-time and run-start reads are cheap and cannot re-block on `docker`.

## Section 1 — Core event model (`agent-core`)

**`src/event.rs`** — new variant on `AgentEvent`:

```rust
/// Emitted once at run start when the configured sandbox has silently
/// degraded to unsandboxed host execution (e.g. Docker unavailable in
/// `auto` mode). The run is NOT isolated despite being configured to be.
SandboxDegraded { mechanism: &'static str, reason: String },
```

**`src/loop_.rs`** — new accessor on `AgentLoop`:

```rust
/// The live sandbox posture (cached; never re-probes Docker). `None` when no
/// sandbox is wired (host executor is installed directly).
pub fn sandbox_descriptor(&self) -> Option<agent_tools::SandboxDescriptor> {
    self.config.sandbox.as_ref().map(|s| s.describe())
}
```

`mechanism` is `&'static str` to match `SandboxDescriptor.mechanism`; the wire
and settings layers convert to `String` at their boundary.

## Section 2 — Emit site (`agent-core/src/loop_.rs`)

At the top of `run_with_cancel`, before the retriever/goal setup and the turn
loop:

```rust
if let Some(d) = self.sandbox_descriptor() {
    if let Some(reason) = d.degraded {
        self.sink.emit(AgentEvent::SandboxDegraded { mechanism: d.mechanism, reason });
    }
}
```

Emitted once per run, unconditionally on degraded posture — not gated on hitting
an approval prompt. The existing per-approval posture string
(`loop_.rs:358-363`) is unchanged.

## Section 3 — CLI rendering (`agent-cli/src/render.rs`)

New exhaustive-match arm (the `AgentEvent` match has no wildcard, so this is
required and intentional):

```rust
AgentEvent::SandboxDegraded { mechanism, reason } => {
    let _ = writeln!(out,
        "\n\x1b[33m⚠ sandbox degraded: {mechanism} unavailable ({reason}); \
         tools run UNSANDBOXED on the host\x1b[0m");
}
```

Yellow (warning), distinct from the red `✗` used for `Error`.

## Section 4 — Wire + connect-time posture (`agent-server`)

**Run-start event — `src/wire.rs`:**

- New `ServerEvent::SandboxDegraded { mechanism: String, reason: String }`
  (serde `tag = "type"`, `rename_all = "snake_case"` → wire type
  `"sandbox_degraded"`).
- `server_event_from` maps `AgentEvent::SandboxDegraded { mechanism, reason }` →
  `ServerEvent::SandboxDegraded { mechanism: mechanism.into(), reason }`.

**Connect-time — `src/wire.rs` + `src/runtime.rs`:**

- New wire struct:
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct SandboxDegraded { pub mechanism: String, pub reason: String }
  ```
- `SettingsState` gains
  `pub sandbox_degraded: Option<SandboxDegraded>`.
- `RuntimeState::settings_state()` populates it:
  ```rust
  let sandbox_degraded = self.current_loop().sandbox_descriptor()
      .and_then(|d| d.degraded.map(|reason| SandboxDegraded {
          mechanism: d.mechanism.to_string(), reason }));
  ```
  `settings_get` and `settings_update` both return `SettingsState`, so the
  banner state stays correct after a settings change too.

## Section 5 — Web + desktop

**`web/src/wire.ts`:**

- `WireEvent` gains `| { type: "sandbox_degraded"; mechanism: string; reason: string }`.
- The `settings_state` arm of `Inbound` gains
  `sandbox_degraded?: { mechanism: string; reason: string } | null`.

**`web/src/state.ts`:**

- `ConversationState` gains
  `sandboxDegraded: { mechanism: string; reason: string } | null` (init `null`).
- `Action` gains `{ type: "dismiss_sandbox_banner" }`.
- Reducer:
  - `settings_state` frame → set `sandboxDegraded` from
    `frame.sandbox_degraded ?? null`.
  - `event` frame, `p.type === "sandbox_degraded"` → set `sandboxDegraded`.
  - `dismiss_sandbox_banner` → set `sandboxDegraded: null` (client-only; a later
    degraded run re-emits and re-shows).
  - `reset` → cleared via `initialState`.

**New component `web/src/components/SandboxBanner.tsx`:** rendered above the
conversation; visible whenever `sandboxDegraded !== null`; shows
`⚠ Sandbox degraded — tools run unsandboxed on the host (<mechanism>: <reason>)`
with a dismiss control dispatching `dismiss_sandbox_banner`. Styling follows the
existing warning/error affordance (`AnimatedError` palette) but is a distinct,
non-scrolling banner, not a message-list item.

**Desktop (`src-tauri`):** consumes `ServerEvent` as serde JSON over the IPC
channel and forwards frames to the same web frontend. Adding an enum variant is
additive; there is no exhaustive Rust `ServerEvent` match in `src-tauri` to
update (verified). The desktop app picks up the new event and settings field
through the shared `web/` code.

## Error handling & edge cases

- **No sandbox wired** (`config.sandbox == None`, host executor installed
  directly): `sandbox_descriptor()` returns `None` → no signal. Correct: nothing
  was promised.
- **Explicit host mode** (`Mode::Off`): `HostExecutor::describe().degraded ==
  None` → no signal. Correct: deliberate choice.
- **Enforce mode, Docker down:** launch errors before any run reaches tools;
  unchanged.
- **Dismiss then re-run while still degraded:** run-start event re-emits, banner
  re-appears. Intended — the posture is still unsafe.

## Testing

**Rust:**
- `agent-core` (`loop_.rs`): a degraded fake sandbox on a **no-tool-call** run
  still yields a `sandbox_degraded:<mechanism>` event (proves it is not gated on
  approvals). Extend the existing `degraded_posture_*` test to also assert the
  event.
- `agent-server` (`wire.rs`): `server_event_from(AgentEvent::SandboxDegraded …)`
  → `ServerEvent::SandboxDegraded`, serializes with `"type":"sandbox_degraded"`.
- `agent-server` (`runtime.rs`): `settings_state()` carries
  `sandbox_degraded: Some(..)` under a degraded fake sandbox and `None` under a
  host/available sandbox.

**Web (vitest):**
- reducer sets `sandboxDegraded` from a `settings_state` frame.
- reducer sets `sandboxDegraded` from a `sandbox_degraded` event frame.
- `dismiss_sandbox_banner` clears it; `reset` clears it.

**CLI:** render arm produces the warning line (follows existing `render.rs`
test pattern).

## Alternatives considered

Both resolved with the user during brainstorming:

1. **Wire representation.** Reuse `ServerEvent::Error` with a `⚠` prefix (zero
   protocol/TS churn) — **rejected**: conflates a safety-posture signal with real
   failures and is not type-distinct. Chose a first-class event + a persistent,
   dismissible banner.
2. **Signal timing.** Run-start event only — **rejected**: a user idling on a
   freshly-opened UI would not see the warning until they sent a message. Chose
   connect-time (`settings_state`) **and** run-start, matching the audit's
   "startup check" wording.

## Files touched

- `agent/crates/agent-core/src/event.rs` — new `AgentEvent::SandboxDegraded`.
- `agent/crates/agent-core/src/loop_.rs` — `sandbox_descriptor()` + emit at run
  start; test.
- `agent/crates/agent-core/src/testkit.rs` — `CollectingSink` arm.
- `agent/crates/agent-cli/src/render.rs` — CLI arm.
- `agent/crates/agent-server/src/wire.rs` — `ServerEvent::SandboxDegraded`,
  `SandboxDegraded` struct, `SettingsState.sandbox_degraded`, mapping; tests.
- `agent/crates/agent-server/src/runtime.rs` — populate `sandbox_degraded` in
  `settings_state()`.
- `web/src/wire.ts` — `WireEvent` + `Inbound` settings_state field.
- `web/src/state.ts` — state field, action, reducer; tests.
- `web/src/components/SandboxBanner.tsx` — new component; wired into the
  conversation view.
