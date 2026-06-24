# Context Dashboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a compact, expandable "context dashboard" above the chat composer that shows live context-window usage plus active config and session stats.

**Architecture:** The Rust agent gains one additive wire event (`Usage`) emitted per tool-loop turn, computed from the existing server-side token estimator. The web client stores the latest usage on its reducer state and renders a new `ContextDashboard` component in the existing slot between the approval prompt and the composer. No change to the token/reasoning/tool streaming path.

**Tech Stack:** Rust (workspace crates `agent-core`, `agent-server`), React + TypeScript + Tailwind + Vitest (`web/`).

## Global Constraints

- **No new dependencies** in either the Rust workspace or `web/`.
- **Wire protocol stays version 1** — the `Usage` event is purely additive.
- **Cargo is not on PATH**: prefix every cargo command with `source ~/.cargo/env &&` (run from the repo root or `cd agent` as noted).
- **Web styling** uses existing CSS variables only (`--surface-base`, `--surface-overlay`, `--border`, `--text`, `--text-muted`, `--text-strong`, `--accent`, `--state-run`, `--state-done`, `--state-error`) and Tailwind utility classes — match `AgentHeader.tsx` / `Composer.tsx`.
- **TDD**: write the failing test first, watch it fail, implement, watch it pass, commit.
- Token estimate semantics: `prompt_tokens` is the built-context estimate (`~4 chars/token`), authoritative for what was actually sent. `turn`/`max_turns` are the per-run tool-loop counters, NOT the client's per-exchange `turnIndex`.

---

## File Structure

**Rust (backend telemetry):**
- `agent/crates/agent-core/src/context.rs` — add `built_tokens(&[Message]) -> usize` helper (Task 1).
- `agent/crates/agent-core/src/event.rs` — add `AgentEvent::Usage { .. }` variant (Task 1).
- `agent/crates/agent-core/src/loop_.rs` — emit `Usage` per turn (Task 1).
- `agent/crates/agent-core/src/testkit.rs` — label the new variant in `CollectingSink` (Task 1).
- `agent/crates/agent-server/src/wire.rs` — add `WireEvent::Usage` + mapping (Task 2).

**TypeScript (frontend):**
- `web/src/wire.ts` — add `usage` to the `WireEvent` union (Task 3).
- `web/src/state.ts` — add `usage` state field + reducer handling (Task 3).
- `web/src/storage.ts` — persist dashboard expanded/collapsed (Task 4).
- `web/src/components/ContextDashboard.tsx` — new component (Task 5).
- `web/src/components/AgentColumn.tsx` + `web/src/App.tsx` — thread props + render (Task 6).

---

## Task 1: Backend token telemetry (agent-core)

**Files:**
- Modify: `agent/crates/agent-core/src/context.rs`
- Modify: `agent/crates/agent-core/src/event.rs`
- Modify: `agent/crates/agent-core/src/loop_.rs:122-134`
- Modify: `agent/crates/agent-core/src/testkit.rs:80-92`
- Test: `agent/crates/agent-core/src/context.rs` (inline `#[cfg(test)]`) and `agent/crates/agent-core/src/loop_.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces: `agent_core::built_tokens(messages: &[Message]) -> usize` (re-exported via `pub use context::*`).
- Produces: `AgentEvent::Usage { prompt_tokens: usize, context_limit: usize, turn: usize, max_turns: usize }`.

- [ ] **Step 1: Write the failing test for `built_tokens`**

Add to the `#[cfg(test)] mod tests` block in `agent/crates/agent-core/src/context.rs`:

```rust
    #[test]
    fn built_tokens_sums_per_message_estimate() {
        let msgs = vec![Message::system("SYS"), Message::user("hello world")];
        let expected = message_tokens(&msgs[0]) + message_tokens(&msgs[1]);
        assert_eq!(built_tokens(&msgs), expected);
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-core built_tokens_sums`
Expected: FAIL — `cannot find function 'built_tokens'`.

- [ ] **Step 3: Implement `built_tokens`**

In `agent/crates/agent-core/src/context.rs`, directly below the `message_tokens` fn (around line 11):

```rust
/// Total estimated tokens for a built context (system + kept history),
/// using the same per-message estimate the window manager evicts against.
pub fn built_tokens(messages: &[Message]) -> usize {
    messages.iter().map(message_tokens).sum()
}
```

- [ ] **Step 4: Run it to verify it passes**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-core built_tokens_sums`
Expected: PASS.

- [ ] **Step 5: Add the `Usage` event variant**

In `agent/crates/agent-core/src/event.rs`, add to the `AgentEvent` enum (after `Reasoning(String)`):

```rust
    Usage { prompt_tokens: usize, context_limit: usize, turn: usize, max_turns: usize },
```

- [ ] **Step 6: Label `Usage` in the test sink**

In `agent/crates/agent-core/src/testkit.rs`, inside `CollectingSink::emit`'s match, add an arm (after the `Reasoning` arm):

```rust
            AgentEvent::Usage { prompt_tokens, .. } => format!("usage:{prompt_tokens}"),
```

- [ ] **Step 7: Write the failing test for loop emission**

Add a new test to the `#[cfg(test)] mod tests` block in `agent/crates/agent-core/src/loop_.rs`. Model it on `transport_error_then_success_via_retry` (a single text turn → one completion → done):

```rust
    #[tokio::test]
    async fn emits_usage_event_before_completing() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![Scripted::Text("hi".into())]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), Arc::new(DenyAll),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 2, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60), ..Default::default() });
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let events = sink.events.lock().unwrap().clone();
        // A usage event is emitted, and it precedes the terminal done.
        let usage_idx = events.iter().position(|e| e.starts_with("usage:")).expect("usage event present");
        let done_idx = events.iter().rposition(|e| e == "done").expect("done present");
        assert!(usage_idx < done_idx);
    }
```

> If `DenyAll`, `ScriptedModel`, `Scripted`, `PassthroughProtocol`, `registry`, `policy`, or `AlwaysApprove` are not in scope at this location, copy the `use super::*;` / helper imports already present at the top of the existing `mod tests` block — do not redefine them.

- [ ] **Step 8: Run it to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-core emits_usage_event_before_completing`
Expected: FAIL — `usage event present` panic (no usage emitted yet).

- [ ] **Step 9: Emit `Usage` in the loop**

In `agent/crates/agent-core/src/loop_.rs`, change the loop header and request construction (currently lines 122-134). Rename `_turn` to `turn`, build messages into a local, emit usage, then move `messages` into the request:

```rust
        for turn in 0..self.config.max_turns {
            let messages = ctx.build(self.config.model_limit);
            self.sink.emit(AgentEvent::Usage {
                prompt_tokens: built_tokens(&messages),
                context_limit: self.config.model_limit,
                turn: turn + 1,
                max_turns: self.config.max_turns,
            });
            let base = CompletionRequest {
                messages,
                tools: self.tools.schemas(),
                temperature: self.config.temperature,
                max_tokens: self.config.max_tokens,
                top_p: self.config.top_p,
                top_k: self.config.top_k,
                min_p: self.config.min_p,
                presence_penalty: self.config.presence_penalty,
                repeat_penalty: self.config.repeat_penalty,
                enable_thinking: self.config.enable_thinking,
            };
```

Then add `built_tokens` to the imports at the top of `loop_.rs` line 1:

```rust
use crate::{built_tokens, AgentEvent, ContextManager, EventSink};
```

- [ ] **Step 10: Run it to verify it passes**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-core emits_usage_event_before_completing`
Expected: PASS.

- [ ] **Step 11: Run the full crate test suite (no regressions)**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-core`
Expected: PASS (all existing tests still green; the new `AgentEvent::Usage` arm is exhaustively matched in `testkit.rs`).

- [ ] **Step 12: Commit**

```bash
git add agent/crates/agent-core/src/context.rs agent/crates/agent-core/src/event.rs agent/crates/agent-core/src/loop_.rs agent/crates/agent-core/src/testkit.rs
git commit -m "feat(agent-core): emit per-turn Usage event (context tokens, turn budget)"
```

---

## Task 2: Wire the `Usage` event to the protocol (agent-server)

**Files:**
- Modify: `agent/crates/agent-server/src/wire.rs:60-73` (the `WireEvent` enum)
- Modify: `agent/crates/agent-server/src/wire.rs:100-114` (`wire_event_from`)
- Test: `agent/crates/agent-server/src/wire.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: `AgentEvent::Usage { prompt_tokens, context_limit, turn, max_turns }` from Task 1.
- Produces: `WireEvent::Usage { prompt_tokens: usize, context_limit: usize, turn: usize, max_turns: usize }` serializing as `{"type":"usage", ...}`.

- [ ] **Step 1: Write the failing round-trip test**

Add to the `#[cfg(test)] mod tests` block in `agent/crates/agent-server/src/wire.rs`:

```rust
    #[test]
    fn usage_event_maps_to_wire_and_serializes() {
        let payload = wire_event_from(AgentEvent::Usage {
            prompt_tokens: 1234, context_limit: 128_000, turn: 2, max_turns: 20,
        }).unwrap();
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"type\":\"usage\""));
        assert!(json.contains("\"prompt_tokens\":1234"));
        assert!(json.contains("\"context_limit\":128000"));
        assert!(json.contains("\"turn\":2"));
        assert!(json.contains("\"max_turns\":20"));
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-server usage_event_maps_to_wire`
Expected: FAIL — no `Usage` variant on `AgentEvent`/`WireEvent` (compile error).

- [ ] **Step 3: Add the `WireEvent::Usage` variant**

In `agent/crates/agent-server/src/wire.rs`, add to the `WireEvent` enum (after `Reasoning { text: String },`):

```rust
    Usage { prompt_tokens: usize, context_limit: usize, turn: usize, max_turns: usize },
```

- [ ] **Step 4: Add the mapping arm**

In `wire_event_from`, add an arm (after the `Reasoning` arm):

```rust
        AgentEvent::Usage { prompt_tokens, context_limit, turn, max_turns } =>
            WireEvent::Usage { prompt_tokens, context_limit, turn, max_turns },
```

- [ ] **Step 5: Run it to verify it passes**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-server usage_event_maps_to_wire`
Expected: PASS.

- [ ] **Step 6: Build the whole workspace (catch other non-exhaustive matches)**

Run: `source ~/.cargo/env && cd agent && cargo test --workspace`
Expected: PASS. (If another crate matches `AgentEvent` non-exhaustively, add a `Usage` arm there; none is expected — the CLI sink in `agent-cli` is the only other consumer, verify it compiles.)

- [ ] **Step 7: Commit**

```bash
git add agent/crates/agent-server/src/wire.rs
git commit -m "feat(agent-server): relay Usage as a wire event (type=usage)"
```

---

## Task 3: Client wire type + reducer state (web)

**Files:**
- Modify: `web/src/wire.ts:38-44` (the `WireEvent` union)
- Modify: `web/src/state.ts` (state field, `initialState`, `reduceFrame`)
- Test: `web/test/state.test.ts`, `web/test/wire.test.ts`

**Interfaces:**
- Produces: `ConversationState.usage: { promptTokens: number; contextLimit: number; turn: number; maxTurns: number } | null`.
- Consumes (later tasks): the same `usage` object shape, passed to `ContextDashboard`.

- [ ] **Step 1: Write the failing reducer test**

Add to `web/test/state.test.ts` inside `describe("reducer", ...)`:

```ts
  it("stores the latest usage event and clears it on reset", () => {
    let s = run([
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "usage", prompt_tokens: 1200, context_limit: 8000, turn: 1, max_turns: 20 } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "usage", prompt_tokens: 1500, context_limit: 8000, turn: 2, max_turns: 20 } }),
    ]);
    expect(s.usage).toEqual({ promptTokens: 1500, contextLimit: 8000, turn: 2, maxTurns: 20 });
    s = reduce(s, { type: "reset", userMsgs: [] });
    expect(s.usage).toBeNull();
  });

  it("does not add usage events to the transcript", () => {
    const s = run([
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "usage", prompt_tokens: 100, context_limit: 8000, turn: 1, max_turns: 20 } }),
    ]);
    expect(s.items).toEqual([]);
  });
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd web && npx vitest run test/state.test.ts -t usage`
Expected: FAIL — `usage` is not a valid event type (TS) / `s.usage` undefined.

- [ ] **Step 3: Add `usage` to the wire union**

In `web/src/wire.ts`, add to the `WireEvent` union (after the `reasoning` member):

```ts
  | { type: "usage"; prompt_tokens: number; context_limit: number; turn: number; max_turns: number }
```

- [ ] **Step 4: Add the state field + reset**

In `web/src/state.ts`:

In the `ConversationState` interface (after `pendingApproval`):

```ts
  usage: { promptTokens: number; contextLimit: number; turn: number; maxTurns: number } | null;
```

In `initialState`, add `usage: null` to the returned object:

```ts
  return { items: [], pendingApproval: null, usage: null, online: false, status: "connecting", userMsgs, turnIndex: 0, inTurn: false,
    settings: null, settingsMeta: null, settingsError: null };
```

- [ ] **Step 5: Handle the `usage` event in the reducer**

In `web/src/state.ts`, inside `reduceFrame`'s `switch (p.type)` (Task note: `usage` arrives as an `event`-kind frame, so it flows through `startTurn` like the others — that is fine, it does not emit a user item by itself). Add a case **before** `case "token":`:

```ts
    case "usage":
      return { ...s, usage: { promptTokens: p.prompt_tokens, contextLimit: p.context_limit, turn: p.turn, maxTurns: p.max_turns } };
```

> Note: returning `s` (the `startTurn` result) is intentional and matches the other event arms.

- [ ] **Step 6: Write a wire parse test**

Add to `web/test/wire.test.ts` inside `describe("parseInbound", ...)`:

```ts
  it("parses a usage event", () => {
    const f = parseInbound(JSON.stringify({ v: 1, session_id: "s", kind: "event", payload: { type: "usage", prompt_tokens: 1200, context_limit: 8000, turn: 1, max_turns: 20 } }));
    expect(f).toEqual({ v: 1, session_id: "s", kind: "event", payload: { type: "usage", prompt_tokens: 1200, context_limit: 8000, turn: 1, max_turns: 20 } });
  });
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cd web && npx vitest run test/state.test.ts test/wire.test.ts`
Expected: PASS.

- [ ] **Step 8: Typecheck**

Run: `cd web && npm run typecheck`
Expected: no errors.

- [ ] **Step 9: Commit**

```bash
git add web/src/wire.ts web/src/state.ts web/test/state.test.ts web/test/wire.test.ts
git commit -m "feat(web): store latest usage telemetry on reducer state"
```

---

## Task 4: Persist dashboard expanded/collapsed (web storage)

**Files:**
- Modify: `web/src/storage.ts`
- Test: `web/test/storage.test.ts`

**Interfaces:**
- Produces: `loadDashExpanded(): boolean` (default `false`), `saveDashExpanded(v: boolean): void`.

- [ ] **Step 1: Write the failing test**

Add to `web/test/storage.test.ts` (match the file's existing import style; if it imports from `../src/storage`, add these to that import):

```ts
import { loadDashExpanded, saveDashExpanded } from "../src/storage";

describe("context dashboard persistence", () => {
  it("defaults to collapsed and round-trips the expanded flag", () => {
    localStorage.clear();
    expect(loadDashExpanded()).toBe(false);
    saveDashExpanded(true);
    expect(loadDashExpanded()).toBe(true);
    saveDashExpanded(false);
    expect(loadDashExpanded()).toBe(false);
  });
});
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd web && npx vitest run test/storage.test.ts -t "context dashboard"`
Expected: FAIL — `loadDashExpanded` is not exported.

- [ ] **Step 3: Implement the helpers**

Append to `web/src/storage.ts`:

```ts
const DASH_EXPANDED = "agent.contextDashExpanded";

export function loadDashExpanded(): boolean {
  return localStorage.getItem(DASH_EXPANDED) === "1";
}
export function saveDashExpanded(v: boolean): void {
  try { localStorage.setItem(DASH_EXPANDED, v ? "1" : "0"); } catch { /* ignore */ }
}
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cd web && npx vitest run test/storage.test.ts -t "context dashboard"`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src/storage.ts web/test/storage.test.ts
git commit -m "feat(web): persist context dashboard expanded state"
```

---

## Task 5: The `ContextDashboard` component (web)

**Files:**
- Create: `web/src/components/ContextDashboard.tsx`
- Test: `web/test/context-dashboard.test.tsx`

**Interfaces:**
- Consumes: `usage` (Task 3 shape), `settings: RuntimeSettings | null` (`web/src/wire.ts`), `toolCount: number`, `artifactCount: number`.
- Produces: `ContextDashboard` React component (default export not used; named export to match siblings).

- [ ] **Step 1: Write the failing component test**

Create `web/test/context-dashboard.test.tsx`:

```tsx
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ContextDashboard } from "../src/components/ContextDashboard";
import type { RuntimeSettings } from "../src/wire";

const settings = { model: "qwen3", temperature: 0.7, active_skills: ["search", "files"] } as unknown as RuntimeSettings;
const usage = { promptTokens: 12400, contextLimit: 128000, turn: 3, maxTurns: 20 };

describe("ContextDashboard", () => {
  beforeEach(() => localStorage.clear());

  it("renders the collapsed gauge with a formatted figure and percent", () => {
    render(<ContextDashboard usage={usage} settings={settings} toolCount={5} artifactCount={2} />);
    expect(screen.getByText(/12\.4k\s*\/\s*128k/)).toBeInTheDocument();
    expect(screen.getByText(/10%/)).toBeInTheDocument(); // 12400/128000 ≈ 10%
  });

  it("shows a muted placeholder when usage is null", () => {
    render(<ContextDashboard usage={null} settings={settings} toolCount={0} artifactCount={0} />);
    expect(screen.getByText(/—/)).toBeInTheDocument();
  });

  it("expands to reveal config and session stats", () => {
    render(<ContextDashboard usage={usage} settings={settings} toolCount={5} artifactCount={2} />);
    fireEvent.click(screen.getByRole("button", { name: /context/i }));
    expect(screen.getByText(/qwen3/)).toBeInTheDocument();
    expect(screen.getByText(/turns 3\/20/)).toBeInTheDocument();
    expect(screen.getByText(/5 tools/)).toBeInTheDocument();
    expect(screen.getByText(/2 art/)).toBeInTheDocument();
    expect(screen.getByText(/search, files/)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd web && npx vitest run test/context-dashboard.test.tsx`
Expected: FAIL — cannot resolve `../src/components/ContextDashboard`.

- [ ] **Step 3: Implement the component**

Create `web/src/components/ContextDashboard.tsx`:

```tsx
import { useState } from "react";
import type { RuntimeSettings } from "../wire";
import { loadDashExpanded, saveDashExpanded } from "../storage";

function fmt(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1).replace(/\.0$/, "")}k` : `${n}`;
}

export function ContextDashboard(
  { usage, settings, toolCount, artifactCount }:
  { usage: { promptTokens: number; contextLimit: number; turn: number; maxTurns: number } | null;
    settings: RuntimeSettings | null; toolCount: number; artifactCount: number },
) {
  const [expanded, setExpanded] = useState(loadDashExpanded);
  const toggle = () => setExpanded((e) => { const next = !e; saveDashExpanded(next); return next; });

  const pct = usage ? Math.min(100, Math.round((usage.promptTokens / usage.contextLimit) * 100)) : 0;
  const over = pct >= 80;
  const fill = over ? "var(--state-error)" : "var(--accent)";

  return (
    <div style={{ background: "var(--surface-base)", borderTop: "1px solid var(--border)" }}>
      <button onClick={toggle} aria-label="context usage" aria-expanded={expanded}
        className="flex w-full items-center gap-2 px-3 py-2 text-left">
        <span className="h-2 w-2 shrink-0 rounded-full"
          style={{ background: usage ? fill : "var(--text-muted)" }} />
        <span className="font-mono text-xs shrink-0" style={{ color: "var(--text-strong)" }}>
          {usage ? `${fmt(usage.promptTokens)} / ${fmt(usage.contextLimit)}` : "— / —"}
        </span>
        <span className="relative h-1.5 flex-1 overflow-hidden rounded-full"
          style={{ background: "var(--surface-overlay)" }}>
          <span className="absolute inset-y-0 left-0 rounded-full"
            style={{ width: `${pct}%`, background: fill }} />
        </span>
        <span className="font-mono text-xs shrink-0" style={{ color: "var(--text-muted)" }}>
          {usage ? `${pct}%` : ""}
        </span>
        <span className="shrink-0 text-xs" style={{ color: "var(--text-muted)" }}>{expanded ? "▾" : "▸"}</span>
      </button>

      {expanded && (
        <div className="space-y-1 px-3 pb-2 font-mono text-xs" style={{ color: "var(--text-muted)" }}>
          {settings && (
            <div>model {settings.model} · temp {settings.temperature}</div>
          )}
          {usage && (
            <div>turns {usage.turn}/{usage.maxTurns} · {toolCount} tools · {artifactCount} art</div>
          )}
          {settings && settings.active_skills.length > 0 && (
            <div>skills: {settings.active_skills.join(", ")}</div>
          )}
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cd web && npx vitest run test/context-dashboard.test.tsx`
Expected: PASS.

- [ ] **Step 5: Typecheck**

Run: `cd web && npm run typecheck`
Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add web/src/components/ContextDashboard.tsx web/test/context-dashboard.test.tsx
git commit -m "feat(web): ContextDashboard component (expandable gauge + config/stats)"
```

---

## Task 6: Integrate into AgentColumn + App (web)

**Files:**
- Modify: `web/src/components/AgentColumn.tsx`
- Modify: `web/src/App.tsx:95-100` (the `<AgentColumn .../>` call) and the `artifacts`/derived-count area
- Test: `web/src/components/AgentColumn.test.tsx`

**Interfaces:**
- Consumes: `ContextDashboard` (Task 5), `ConversationState.usage` (Task 3), `state.settings`, `state.items`.

- [ ] **Step 1: Write the failing integration test**

Add to `web/src/components/AgentColumn.test.tsx`. First extend the `base` fixture with the new props, then add a test:

Replace the `base` object with:

```tsx
const base = {
  items: [], activeArtifactKey: null, onSelectArtifact: () => {},
  projectLabel: "studio-x", model: "qwen3", pendingApproval: null,
  onDecide: () => {}, composerDisabled: false, onSend: vi.fn(),
  usage: null as null | { promptTokens: number; contextLimit: number; turn: number; maxTurns: number },
  settings: null, toolCount: 0, artifactCount: 0,
};
```

Add a test inside the `describe`:

```tsx
  it("renders the context dashboard gauge above the composer", () => {
    render(<AgentColumn {...base} usage={{ promptTokens: 4000, contextLimit: 8000, turn: 1, maxTurns: 20 }} />);
    expect(screen.getByLabelText("context usage")).toBeInTheDocument();
    expect(screen.getByText(/4k\s*\/\s*8k/)).toBeInTheDocument();
    expect(screen.getByText(/50%/)).toBeInTheDocument();
  });
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd web && npx vitest run src/components/AgentColumn.test.tsx -t "context dashboard"`
Expected: FAIL — `context usage` label not found (and TS error on unknown props).

- [ ] **Step 3: Thread props through `AgentColumn`**

In `web/src/components/AgentColumn.tsx`:

Add the import:

```tsx
import { ContextDashboard } from "./ContextDashboard";
import type { RuntimeSettings } from "../wire";
```

Extend the props destructure and type signature — replace the function signature with:

```tsx
export function AgentColumn({ items, activeArtifactKey, onSelectArtifact, projectLabel, model,
  pendingApproval, onDecide, composerDisabled, onSend, usage, settings, toolCount, artifactCount }:
  { items: AnimatedItem[]; activeArtifactKey: string | null; onSelectArtifact: (key: string) => void;
    projectLabel: string; model?: string; pendingApproval: PendingApproval | null;
    onDecide: (d: Decision) => void; composerDisabled: boolean; onSend: (text: string) => void;
    usage: { promptTokens: number; contextLimit: number; turn: number; maxTurns: number } | null;
    settings: RuntimeSettings | null; toolCount: number; artifactCount: number }) {
```

Render the dashboard between the approval prompt and the composer — replace those two lines with:

```tsx
      {pendingApproval && <ApprovalPrompt approval={pendingApproval} onDecide={onDecide} />}
      <ContextDashboard usage={usage} settings={settings} toolCount={toolCount} artifactCount={artifactCount} />
      <Composer disabled={composerDisabled} onSend={onSend} />
```

- [ ] **Step 4: Pass the real data from `App.tsx`**

In `web/src/App.tsx`, compute the derived counts near the existing `artifacts` line (after `const artifacts = artifactsFrom(state.items);`):

```tsx
  const toolCount = state.items.filter((it) => it.kind === "tool").length;
```

Then extend the `<AgentColumn .../>` call (around line 95) to pass the new props:

```tsx
          <AgentColumn items={animatedItems} activeArtifactKey={activeArtifactKey}
            onSelectArtifact={(key) => { setActiveArtifactKey(key); setWorkspaceOpen(true); }}
            projectLabel={projectLabel} model={model}
            pendingApproval={state.pendingApproval} onDecide={decide}
            composerDisabled={!connected} onSend={send}
            usage={state.usage} settings={state.settings}
            toolCount={toolCount} artifactCount={artifacts.length} />
```

- [ ] **Step 5: Run the integration test to verify it passes**

Run: `cd web && npx vitest run src/components/AgentColumn.test.tsx`
Expected: PASS (the new test plus the existing three).

- [ ] **Step 6: Run the full web suite + typecheck**

Run: `cd web && npm run typecheck && npx vitest run`
Expected: no type errors; all tests pass.

- [ ] **Step 7: Commit**

```bash
git add web/src/components/AgentColumn.tsx web/src/components/AgentColumn.test.tsx web/src/App.tsx
git commit -m "feat(web): mount ContextDashboard above the composer with live data"
```

---

## Final verification

- [ ] **Backend:** `source ~/.cargo/env && cd agent && cargo test --workspace` → all green.
- [ ] **Frontend:** `cd web && npm run typecheck && npx vitest run` → all green.
- [ ] **Build:** `cd web && npm run build` → succeeds (tsc + vite).
- [ ] **Manual smoke (optional):** run the stack, send a message, confirm the gauge appears above the composer, climbs across tool-loop turns, and the expand toggle persists across reload.

---

## Self-Review (completed during planning)

- **Spec coverage:** backend telemetry (Tasks 1–2), wire type + state (Task 3), persistence (Task 4), component with collapsed/expanded + edge cases (Task 5), placement in the existing slot + derived stats (Task 6). Over-limit threshold (80%, `--state-error`), no-usage muted state, and settings-not-loaded placeholders are all in Task 5. ✔
- **Type consistency:** `{ promptTokens, contextLimit, turn, maxTurns }` (client) and `{ prompt_tokens, context_limit, turn, max_turns }` (wire/Rust) are used consistently across Tasks 1–6; `built_tokens` named identically in definition (Task 1 Step 3) and use (Task 1 Step 9). ✔
- **Placeholder scan:** every code step shows complete code and an exact command with expected output. ✔
