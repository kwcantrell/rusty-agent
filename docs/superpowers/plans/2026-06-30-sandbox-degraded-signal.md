# Sandbox-Degraded First-Class Signal — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface a silently-degraded sandbox (auto-mode, Docker down → host execution) as a first-class signal on CLI, web, and desktop — at connect time and run start.

**Architecture:** A new `AgentEvent::SandboxDegraded` is emitted once at run start when `sandbox.describe().degraded.is_some()`. The CLI renders a loud line; the server maps it to a new `ServerEvent::SandboxDegraded` and also carries the posture in the `settings_state` snapshot so the web/desktop banner shows the moment a client connects. The web reducer tracks the posture and a `<SandboxBanner>` renders it, dismissibly.

**Tech Stack:** Rust (Cargo workspace `agent/`), TypeScript/React 19 + Vite + vitest (`web/`).

## Global Constraints

- Signal fires ONLY when isolation was requested but not delivered: `SandboxDescriptor.degraded == Some(_)`. Explicit host mode (`degraded == None`) and no-sandbox (`config.sandbox == None`) stay silent.
- `describe()` reads cached `Availability` — never re-probe Docker in the read paths.
- `AgentEvent`, `ServerEvent`, and `WireEvent` matches are exhaustive (no wildcard arms); adding a variant REQUIRES updating every match site or the build breaks. The Rust variant therefore lands as one atomic, compiles-green commit across `agent-core`, `agent-cli`, `agent-server` (Task 1).
- Conventional commits: `type(scope): summary`.
- Run from `agent/` for cargo; `source ~/.cargo/env` first if `cargo` is not on PATH. Run from `web/` for npm.
- Branch is already `fix/sandbox-degraded-signal`.

---

### Task 1: Backend degraded-sandbox signal (all Rust crates)

The enum variant couples these crates; they land together so the workspace stays green.

**Files:**
- Modify: `agent/crates/agent-core/src/event.rs:25` (add variant after `Context(ContextEvent)`)
- Modify: `agent/crates/agent-core/src/loop_.rs:76` (add `sandbox_descriptor()` accessor) and `:155-157` (emit at start of `run_with_cancel`)
- Modify: `agent/crates/agent-core/src/testkit.rs:123` (add `CollectingSink` arm)
- Modify: `agent/crates/agent-cli/src/render.rs:83` (add arm before `AgentEvent::Error(e)`)
- Modify: `agent/crates/agent-server/src/wire.rs` (new `SandboxDegraded` struct; new `ServerEvent::SandboxDegraded`; `SettingsState.sandbox_degraded`; `sandbox_degraded_from` helper; map in `server_event_from`)
- Modify: `agent/crates/agent-server/src/runtime.rs:119-125` (populate `sandbox_degraded` in `settings_state()`)
- Test: inline `#[cfg(test)]` in `loop_.rs` and `wire.rs`

**Interfaces:**
- Produces: `AgentEvent::SandboxDegraded { mechanism: &'static str, reason: String }`
- Produces: `AgentLoop::sandbox_descriptor(&self) -> Option<agent_tools::SandboxDescriptor>`
- Produces (wire, serde `tag="type"`, snake_case): `ServerEvent::SandboxDegraded { mechanism: String, reason: String }` → JSON type `"sandbox_degraded"`
- Produces: `pub struct SandboxDegraded { pub mechanism: String, pub reason: String }` (Serialize/Deserialize)
- Produces: `SettingsState.sandbox_degraded: Option<SandboxDegraded>`
- Produces: `pub fn sandbox_degraded_from(desc: Option<agent_tools::SandboxDescriptor>) -> Option<SandboxDegraded>`

- [ ] **Step 1: Write the failing loop test**

Add to the `#[cfg(test)] mod tests` block in `agent/crates/agent-core/src/loop_.rs` (it already imports `registry()`, `policy()`, `ScriptedModel`, `Scripted`, `PassthroughProtocol`, `AlwaysApprove`, `WindowContext`, `Message`, `CollectingSink`):

```rust
#[tokio::test]
async fn run_emits_sandbox_degraded_even_without_tool_calls() {
    use agent_tools::{CommandSpec, SandboxStrategy, SandboxedChild, SandboxError,
        SandboxDescriptor, Mode, HostExecutor};
    use std::sync::Arc;

    struct DegradedFake;
    impl SandboxStrategy for DegradedFake {
        fn launch(&self, spec: CommandSpec) -> Result<SandboxedChild, SandboxError> {
            HostExecutor.launch(spec)
        }
        fn describe(&self) -> SandboxDescriptor {
            SandboxDescriptor { mode: Mode::Auto, mechanism: "docker",
                image: Some("debian:stable-slim".into()), network: false,
                degraded: Some("no daemon".into()) }
        }
    }

    let ws = std::env::temp_dir();
    // A plain text turn: no tool calls, so no approval prompt is ever hit.
    let model = Arc::new(ScriptedModel::new(vec![Scripted::Text("hi".into())]));
    let sink = Arc::new(CollectingSink::default());
    let agent = AgentLoop::new(
        model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
        Arc::new(AlwaysApprove), sink.clone(),
        LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 1,
            temperature: 0.0, max_tokens: None, workspace: ws,
            tool_timeout: std::time::Duration::from_secs(5),
            stream_idle_timeout: std::time::Duration::from_secs(60),
            sandbox: Some(Arc::new(DegradedFake)), ..Default::default() });

    let mut ctx = WindowContext::new(Message::system("sys"));
    agent.run(&mut ctx, "hello".into()).await.unwrap();

    let events = sink.events.lock().unwrap();
    assert!(events.iter().any(|e| e == "sandbox_degraded:docker"),
        "degraded sandbox must be surfaced even with no tool calls: {events:?}");
}
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p agent-core run_emits_sandbox_degraded_even_without_tool_calls 2>&1 | tail -20`
Expected: compile error — no variant `AgentEvent::SandboxDegraded`, no `sandbox_degraded:` label. (Compile failure counts as the red state here.)

- [ ] **Step 3: Add the event variant**

In `agent/crates/agent-core/src/event.rs`, change the tail of `enum AgentEvent` (currently ending `Context(ContextEvent),` then `}` at line 25-26) to:

```rust
    Context(ContextEvent),
    /// Emitted once at run start when the configured sandbox has silently
    /// degraded to unsandboxed host execution (e.g. Docker unavailable in
    /// `auto` mode). The run is NOT isolated despite being configured to be;
    /// surfaces that "we thought we were sandboxed" hole loudly to every
    /// observer instead of leaving it in a single `tracing::warn!` line.
    SandboxDegraded { mechanism: &'static str, reason: String },
}
```

- [ ] **Step 4: Add the `sandbox_descriptor()` accessor**

In `agent/crates/agent-core/src/loop_.rs`, immediately after the `new()` method (the line `Self { model, protocol, tools, policy, approval, sink, config, retriever: None }` closes at :76-77), add a method inside `impl AgentLoop`:

```rust
    /// The live sandbox posture (cached; never re-probes Docker). `None` when
    /// no sandbox is wired (host executor installed directly).
    pub fn sandbox_descriptor(&self) -> Option<agent_tools::SandboxDescriptor> {
        self.config.sandbox.as_ref().map(|s| s.describe())
    }
```

- [ ] **Step 5: Emit at run start**

In `agent/crates/agent-core/src/loop_.rs`, at the very top of `run_with_cancel` (before `if let Some(retriever) = &self.retriever {` at :157):

```rust
        // Surface a silently-degraded sandbox loudly at run start. If the
        // configured strategy fell back to unsandboxed host execution (e.g.
        // Docker unavailable in `auto` mode), every tool call runs with ambient
        // host access despite the config asking for isolation. The per-approval
        // posture string already carries this, but a run may never hit an
        // approval prompt — emit it unconditionally, once, here.
        if let Some(d) = self.sandbox_descriptor() {
            if let Some(reason) = d.degraded {
                self.sink.emit(AgentEvent::SandboxDegraded { mechanism: d.mechanism, reason });
            }
        }
```

- [ ] **Step 6: Add the `CollectingSink` arm**

In `agent/crates/agent-core/src/testkit.rs`, add after the `CompactionFailed` arm (:123), before the closing `};`:

```rust
            AgentEvent::SandboxDegraded { mechanism, .. } => format!("sandbox_degraded:{mechanism}"),
```

- [ ] **Step 7: Add the CLI render arm**

In `agent/crates/agent-cli/src/render.rs`, immediately before the `AgentEvent::Error(e) =>` arm (:83):

```rust
            AgentEvent::SandboxDegraded { mechanism, reason } => {
                let _ = writeln!(out,
                    "\n\x1b[33m⚠ sandbox degraded: {mechanism} unavailable ({reason}); \
                     tools run UNSANDBOXED on the host\x1b[0m");
            }
```

- [ ] **Step 8: Add the wire struct, helper, ServerEvent variant, and mapping**

In `agent/crates/agent-server/src/wire.rs`:

(a) After the `SettingsState` struct (ends :49), add:

```rust
/// Degraded-sandbox posture carried in `SettingsState` (connect-time) and as a
/// streamed `ServerEvent` (run-start). Present only when isolation was requested
/// but not delivered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SandboxDegraded { pub mechanism: String, pub reason: String }

/// Extract the degraded posture from a sandbox descriptor, if any. Pure so the
/// daemon's `settings_state()` stays trivial and this stays unit-testable.
pub fn sandbox_degraded_from(desc: Option<agent_tools::SandboxDescriptor>) -> Option<SandboxDegraded> {
    desc.and_then(|d| d.degraded.map(|reason| SandboxDegraded {
        mechanism: d.mechanism.to_string(), reason }))
}
```

(b) Add the field to `SettingsState` (after `pub discovered_skills: Vec<DiscoveredSkill>,` at :48):

```rust
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_degraded: Option<SandboxDegraded>,
```

(c) Add the wire variant to `enum ServerEvent` (after `ApprovalRequest { .. }`, before the closing `}` at :39):

```rust
    SandboxDegraded { mechanism: String, reason: String },
```

(d) Map it in `server_event_from` (add before the closing `})` at :106):

```rust
        AgentEvent::SandboxDegraded { mechanism, reason } =>
            ServerEvent::SandboxDegraded { mechanism: mechanism.to_string(), reason },
```

- [ ] **Step 9: Populate `sandbox_degraded` in `settings_state()`**

In `agent/crates/agent-server/src/runtime.rs`, in `settings_state()` (the `SettingsState { .. }` literal at :119-125), add before it:

```rust
        let sandbox_degraded = crate::wire::sandbox_degraded_from(
            self.current_loop().sandbox_descriptor());
```

and add the field inside the struct literal (after `discovered_skills: discovered,` at :124):

```rust
            sandbox_degraded,
```

- [ ] **Step 10: Add the wire unit tests**

In `agent/crates/agent-server/src/wire.rs` `#[cfg(test)] mod tests`, add:

```rust
    #[test]
    fn sandbox_degraded_event_serializes_with_type_tag() {
        let ev = server_event_from(AgentEvent::SandboxDegraded {
            mechanism: "docker", reason: "no daemon".into() }).unwrap();
        let j = serde_json::to_string(&ev).unwrap();
        assert!(j.contains(r#""type":"sandbox_degraded""#), "missing type tag: {j}");
        assert!(j.contains(r#""mechanism":"docker""#), "missing mechanism: {j}");
        assert!(j.contains(r#""reason":"no daemon""#), "missing reason: {j}");
    }

    #[test]
    fn sandbox_degraded_from_maps_only_when_degraded() {
        use agent_tools::{SandboxDescriptor, Mode};
        let degraded = SandboxDescriptor { mode: Mode::Auto, mechanism: "docker",
            image: None, network: false, degraded: Some("no daemon".into()) };
        assert_eq!(sandbox_degraded_from(Some(degraded)),
            Some(SandboxDegraded { mechanism: "docker".into(), reason: "no daemon".into() }));

        let healthy = SandboxDescriptor { mode: Mode::Off, mechanism: "host",
            image: None, network: true, degraded: None };
        assert_eq!(sandbox_degraded_from(Some(healthy)), None);
        assert_eq!(sandbox_degraded_from(None), None);
    }
```

- [ ] **Step 11: Build the workspace and run the affected suites**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished` — no errors (all exhaustive matches updated).

Run: `cargo test -p agent-core -p agent-cli -p agent-server 2>&1 | tail -25`
Expected: all pass, including `run_emits_sandbox_degraded_even_without_tool_calls`, `sandbox_degraded_event_serializes_with_type_tag`, `sandbox_degraded_from_maps_only_when_degraded`.

- [ ] **Step 12: Commit**

```bash
git add agent/crates/agent-core/src/event.rs agent/crates/agent-core/src/loop_.rs \
  agent/crates/agent-core/src/testkit.rs agent/crates/agent-cli/src/render.rs \
  agent/crates/agent-server/src/wire.rs agent/crates/agent-server/src/runtime.rs
git commit -m "feat(core): first-class SandboxDegraded signal across loop, cli, wire

Emit AgentEvent::SandboxDegraded once at run start when the configured
sandbox has degraded to host execution; render loudly in the CLI; map to
ServerEvent::SandboxDegraded and carry posture in settings_state for the
connect-time web banner. Audit Finding 1 (HIGH)."
```

---

### Task 2: Web wire types + reducer state

**Files:**
- Modify: `web/src/wire.ts:39-54` (add `WireEvent` variant + `Inbound` settings_state field)
- Modify: `web/src/state.ts` (`Item` unchanged; add state field :20-34, `Action` :36-41, reducer :73-142, `initialState` :43-46)
- Test: create `web/src/sandbox-banner.test.ts`

**Interfaces:**
- Consumes: wire type `"sandbox_degraded"` and `settings_state.sandbox_degraded` from Task 1.
- Produces: `ConversationState.sandboxDegraded: { mechanism: string; reason: string } | null`
- Produces: `Action` variant `{ type: "dismiss_sandbox_banner" }`

- [ ] **Step 1: Write the failing reducer test**

Create `web/src/sandbox-banner.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { reduce, initialState } from "./state";
import type { Inbound } from "./wire";

const ev = (payload: unknown): Inbound =>
  ({ v: 1, session_id: "s", kind: "event", payload } as Inbound);

const settings = (sandbox_degraded: unknown): Inbound =>
  ({ v: 1, session_id: "s", kind: "settings_state", settings: {}, workspace: "/w",
     api_key_set: true, hard_floor: [], discovered_skills: [], sandbox_degraded } as Inbound);

describe("sandbox degraded banner state", () => {
  it("sets posture from a settings_state frame (connect time)", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: settings({ mechanism: "docker", reason: "no daemon" }) });
    expect(s.sandboxDegraded).toEqual({ mechanism: "docker", reason: "no daemon" });
  });

  it("clears posture when settings_state reports healthy", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: settings({ mechanism: "docker", reason: "x" }) });
    s = reduce(s, { type: "frame", frame: settings(undefined) });
    expect(s.sandboxDegraded).toBeNull();
  });

  it("sets posture from a run-start sandbox_degraded event", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: ev({ type: "sandbox_degraded", mechanism: "docker", reason: "no daemon" }) });
    expect(s.sandboxDegraded).toEqual({ mechanism: "docker", reason: "no daemon" });
  });

  it("dismiss clears the banner", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: ev({ type: "sandbox_degraded", mechanism: "docker", reason: "no daemon" }) });
    s = reduce(s, { type: "dismiss_sandbox_banner" });
    expect(s.sandboxDegraded).toBeNull();
  });

  it("reset clears the banner", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: ev({ type: "sandbox_degraded", mechanism: "docker", reason: "no daemon" }) });
    s = reduce(s, { type: "reset", userMsgs: [] });
    expect(s.sandboxDegraded).toBeNull();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run (from `web/`): `npm test -- sandbox-banner 2>&1 | tail -25`
Expected: FAIL — `sandboxDegraded` is undefined / action type not handled.

- [ ] **Step 3: Extend the wire types**

In `web/src/wire.ts`, add to the `WireEvent` union (after the `done` line at :47):

```ts
  | { type: "sandbox_degraded"; mechanism: string; reason: string };
```

(Move the trailing `;` — the `done` line becomes `| { type: "done"; reason: string }` with no semicolon, and the new line ends the union.)

And add to the `settings_state` arm of `Inbound` (:53), before its closing `}`:

```ts
 sandbox_degraded?: { mechanism: string; reason: string } | null;
```

- [ ] **Step 4: Add reducer state, action, and handling**

In `web/src/state.ts`:

(a) In `ConversationState` (:20-34), add:

```ts
  sandboxDegraded: { mechanism: string; reason: string } | null;
```

(b) In `initialState` return (:44-45), add `sandboxDegraded: null,`.

(c) In the `Action` union (:36-41), add:

```ts
  | { type: "dismiss_sandbox_banner" }
```

(d) In `reduce` (:56-71) add a case:

```ts
    case "dismiss_sandbox_banner":
      return { ...state, sandboxDegraded: null };
```

(e) In `reduceFrame`, in the `settings_state` branch (:75-80), add to the returned object:

```ts
      sandboxDegraded: frame.sandbox_degraded ?? null,
```

(f) In `reduceFrame`'s `switch (p.type)` (:90), add a case:

```ts
    case "sandbox_degraded":
      return { ...s, sandboxDegraded: { mechanism: p.mechanism, reason: p.reason } };
```

- [ ] **Step 5: Run the tests to verify they pass**

Run (from `web/`): `npm test -- sandbox-banner 2>&1 | tail -15`
Expected: PASS (5 tests).

- [ ] **Step 6: Typecheck**

Run (from `web/`): `npm run typecheck 2>&1 | tail -10`
Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add web/src/wire.ts web/src/state.ts web/src/sandbox-banner.test.ts
git commit -m "feat(web): track degraded-sandbox posture in reducer state"
```

---

### Task 3: SandboxBanner component + mount

**Files:**
- Create: `web/src/components/SandboxBanner.tsx`
- Modify: `web/src/App.tsx:151-152` (mount above the conversation split) and `:4` (import)
- Test: create `web/src/components/SandboxBanner.test.tsx`

**Interfaces:**
- Consumes: `ConversationState.sandboxDegraded` and the `dismiss_sandbox_banner` action from Task 2.
- Produces: `<SandboxBanner info={{mechanism,reason}} onDismiss={() => void} />`

- [ ] **Step 1: Write the failing component test**

Create `web/src/components/SandboxBanner.test.tsx`:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { SandboxBanner } from "./SandboxBanner";

describe("SandboxBanner", () => {
  it("shows mechanism and reason and warns about host execution", () => {
    render(<SandboxBanner info={{ mechanism: "docker", reason: "no daemon" }} onDismiss={() => {}} />);
    expect(screen.getByRole("alert").textContent).toMatch(/unsandboxed/i);
    expect(screen.getByRole("alert").textContent).toMatch(/docker/);
    expect(screen.getByRole("alert").textContent).toMatch(/no daemon/);
  });

  it("calls onDismiss when the dismiss control is clicked", () => {
    const onDismiss = vi.fn();
    render(<SandboxBanner info={{ mechanism: "docker", reason: "no daemon" }} onDismiss={onDismiss} />);
    fireEvent.click(screen.getByRole("button", { name: /dismiss/i }));
    expect(onDismiss).toHaveBeenCalledOnce();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run (from `web/`): `npm test -- SandboxBanner 2>&1 | tail -15`
Expected: FAIL — module `./SandboxBanner` not found.

- [ ] **Step 3: Create the component**

Create `web/src/components/SandboxBanner.tsx` (follows the `AnimatedError` warning palette; distinct, non-scrolling banner):

```tsx
export function SandboxBanner(
  { info, onDismiss }: { info: { mechanism: string; reason: string }; onDismiss: () => void },
) {
  return (
    <div role="alert"
      className="flex items-center justify-between gap-3 px-4 py-2 text-sm"
      style={{ background: "var(--warning-surface, #3a2f00)", color: "var(--warning-text, #ffd24a)",
        borderBottom: "1px solid var(--border)" }}>
      <span>
        ⚠ <strong>Sandbox degraded</strong> — tools run unsandboxed on the host
        {" "}({info.mechanism}: {info.reason}).
      </span>
      <button type="button" onClick={onDismiss} aria-label="Dismiss"
        className="shrink-0 rounded px-2 py-0.5 opacity-80 hover:opacity-100"
        style={{ border: "1px solid var(--border)" }}>
        Dismiss
      </button>
    </div>
  );
}
```

- [ ] **Step 4: Run the component test to verify it passes**

Run (from `web/`): `npm test -- SandboxBanner 2>&1 | tail -15`
Expected: PASS (2 tests).

- [ ] **Step 5: Mount it in App.tsx**

In `web/src/App.tsx`, add to the import from `./components/...` near :8:

```tsx
import { SandboxBanner } from "./components/SandboxBanner";
```

Then between the settings-panel block close (`)}` at :151) and the conversation split (`<div className="relative flex min-h-0 flex-1">` at :152), insert:

```tsx
      {state.sandboxDegraded && (
        <SandboxBanner info={state.sandboxDegraded}
          onDismiss={() => dispatch({ type: "dismiss_sandbox_banner" })} />
      )}
```

- [ ] **Step 6: Typecheck, full test, and build**

Run (from `web/`): `npm run typecheck 2>&1 | tail -10`
Expected: no errors.

Run (from `web/`): `npm test 2>&1 | tail -15`
Expected: all suites pass (existing + new).

Run (from `web/`): `npm run build 2>&1 | tail -5`
Expected: build succeeds.

- [ ] **Step 7: Commit**

```bash
git add web/src/components/SandboxBanner.tsx web/src/components/SandboxBanner.test.tsx web/src/App.tsx
git commit -m "feat(web): SandboxBanner surfaces degraded-sandbox posture"
```

---

## Final verification

- [ ] From `agent/`: `cargo build && cargo test -p agent-core -p agent-cli -p agent-server -p agent-tools 2>&1 | tail -20` — all green.
- [ ] From `web/`: `npm run typecheck && npm test && npm run build` — all green.
- [ ] Manual sanity (optional): with Docker stopped and `sandbox_mode="auto"`, launch the CLI (`cargo run -p agent-cli -- --backend claude-cli --model sonnet --workspace .`) and confirm the yellow `⚠ sandbox degraded` line appears on the first run.

## Notes for the implementer

- The audit's sibling HIGH (Finding 2, symlink escape) is already fixed and committed on this branch (`agent-tools/src/fs/paths.rs`) — do not touch it.
- If `@testing-library/react` is not yet a dependency, check `web/package.json`; the existing `*.test.tsx` files (e.g. `SettingsPanel.test.tsx`, `AgentColumn.test.tsx`) already use it, so it is present.
- `SandboxDescriptor` fields, for constructing test doubles: `{ mode: Mode, mechanism: &'static str, image: Option<String>, network: bool, degraded: Option<String> }`.
