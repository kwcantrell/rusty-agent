# Design Spec — UI Redesign: "Workbench" + render-anything Inspector

**Date:** 2026-06-23
**Status:** Approved design. Next step: `writing-plans` → implementation plan.
**Builds on:** [`2026-06-23-react-frontend-design.md`](./2026-06-23-react-frontend-design.md) (the existing React frontend) and [`2026-06-23-event-animation-rich-text-design.md`](./2026-06-23-event-animation-rich-text-design.md).
**Spans:** `web/` (frontend), `agent/crates/agent-tools` + `agent/crates/agent-server` (wire). `cloud/` (Worker) is **untouched** — verified transparent relay.

## 1. Purpose & scope

The current browser UI is functional but visually undesigned: a flat single `zinc-950` column, plain gray boxes, tiny emoji glyphs, minimal hierarchy. This redesign gives it a deliberate, modern, "editorial light + dark" visual system and restructures it into a **three-pane Workbench**, and adds a first-class **render-anything Inspector**: a surface the agent can deliberately render arbitrary artifacts into (markdown, code, HTML, diagrams, tables, images), beyond today's diff/terminal.

### In scope

- A coherent **design system** ("Ink & Sage") with light **and** dark themes, applied to **every** surface.
- A **three-pane Workbench shell** (Activity rail · conversation · Inspector) replacing the single column + horizontal `TimelineView`.
- A **render-anything Inspector** with one renderer per artifact kind.
- A **full-stack artifact channel (v1)**: extend the `Display` enum with rich kinds + a builtin `render` tool, mirrored on the wire and in the frontend.
- Restyle of all existing components; retire `TimelineView`.

### Out of scope (deferred)

- **Artifact channel v2** (§5.4): a first-class streaming `AgentEvent::Artifact` decoupled from tool results, with an `EventSink` in `ToolCtx`. Specced here as the target contract; **not built** this cycle.
- New agent capabilities beyond the `render` tool; auth/pairing changes; multi-session management UI; conversation persistence.
- A density/compact toggle (comfortable density only for v1).

### Invariants

- **Worker is behavior-preserving.** `cloud/` is unchanged. The relay forwards raw frames and only inspects `kind === "event"`; new `Display` variants ride inside `tool_result.display` as opaque JSON. (Verified in `cloud/src/session.ts`.)
- **Wire envelope is unchanged.** The `{ v, session_id, id?, kind, ... }` envelope and event `payload: { type, ... }` shape stay. Only the `Display` enum (a leaf payload) gains variants. `PROTOCOL_VERSION` stays `1` (additive, backward-compatible: older clients ignore unknown `display` variants and fall back to `content`).

## 2. Architecture

```
web/ (React SPA, redesigned)
  ├─ design system (CSS vars via Tailwind v4 @theme; [data-theme] light/dark)
  ├─ AppShell ── ActivityRail │ Conversation │ Inspector
  └─ artifact renderers (markdown, code, html, mermaid, table, image, diff, terminal)
        ▲
        │ tool_result.display : Display   (opaque to the Worker)
cloud/ Worker  ── unchanged transparent relay ──▶ Durable Object ──▶ daemon
        ▲
agent-server/wire.rs  ── Display mirrored in WireEvent::ToolResult
agent-tools/types.rs  ── Display enum extended + builtin `render` tool
```

Data flow is unchanged from today: a tool returns `ToolOutput { content, display }` → the agent loop emits `AgentEvent::ToolResult` → `wire_event_from` maps it to `WireEvent::ToolResult { name, content, display }` → relayed → `parseInbound` in `wire.ts` → reducer → the Inspector renders `display` by kind. The only new producer is the builtin `render` tool, whose entire job is to return a `ToolOutput` with a chosen `Display` variant.

## 3. Design system — "Ink & Sage" (light + dark)

Single source of truth in `web/src/index.css` using Tailwind v4 `@theme` + CSS custom properties, switched by `[data-theme="light"|"dark"]` on `<html>`.

**Tokens**
- **Surfaces / elevation:** `--surface-base`, `--surface-raised`, `--surface-overlay` (+ borders), giving depth without heavy shadows.
- **Text:** `--text-strong`, `--text`, `--text-muted`.
- **Accent:** sage `--accent` (`#4f7a52` light / lifted `#6fae72` dark); clay `--accent-2` (`#c4622d`) for secondary/warning.
- **Semantic tool states:** running (clay/amber), done (sage/green), error (red), idle (neutral).
- **Scales:** type (xs→2xl, with mono = `ui-monospace` for code/terminal/tool names and an optional light serif accent on headings), spacing, radius, shadow, focus-ring.

**Palettes**
- **Light:** warm white `#fcfcfa` base, near-black ink `#16181a` text, sage accent. Editorial, calm.
- **Dark:** warm charcoal `#16181a` base (not pure black), off-white `#f3f4ee` text, lifted sage. Keeps warmth.

**Theme control:** default to `prefers-color-scheme`; manual toggle in the status bar; persisted in `localStorage` (extend `storage.ts`). All color comes from tokens — no hardcoded `zinc-*` in components.

## 4. Layout — three-pane Workbench shell

New `AppShell` composes three regions between a slim top status bar and a bottom Composer:

- **Left — `ActivityRail` (collapsible):** session name + a live **Activity log** listing every tool call vertically with running/done/idle status dots; settings entry pinned at the bottom. This **absorbs and replaces** `TimelineView`. Collapses to an icon strip on narrow widths.
- **Center — Conversation:** the message list. Tool calls render as compact **chips** (name + kind pill + status); the chip whose artifact is open in the Inspector is highlighted ("viewing →"). Cleaner user/assistant/reasoning rendering on the token system.
- **Right — `Inspector` (collapsible, resizable):** the render-anything surface (§5). Remembers width; resizable divider.

**Responsive:** below a breakpoint, the rail and inspector become slide-over drawers so the conversation stays primary on narrow screens.

## 5. The render-anything Inspector + artifact channel

### 5.1 Display enum extension (`agent-tools/src/types.rs`)

Existing `Text` / `Diff` / `Terminal` stay. Add:

- `Markdown(String)` — GFM document.
- `Code { lang: String, filename: Option<String>, text: String }` — full-file syntax-highlighted view (distinct from `Diff`).
- `Html(String)` — sandboxed HTML.
- `Mermaid(String)` — diagram source.
- `Table { columns: Vec<String>, rows: Vec<Vec<String>> }`.
- `Image { mime: String, data: String }` — base64 (`data`) or URL; `mime` distinguishes.

Each artifact optionally carries an **`id: Option<String>`** and **`title: Option<String>`** so the Inspector can title tabs and **replace-by-id** (re-render the same logical artifact in place). Concretely, `Display` becomes a struct-ish payload or each variant gains `id`/`title` — implementation plan decides the exact shape; the contract is "every artifact may carry an id + title."

Serde: additive, `#[serde(rename_all)]` consistent with existing variants. Round-trip unit tests per variant.

### 5.2 Builtin `render` tool (`agent-tools`)

A new builtin tool the agent calls to deliberately push an artifact:

- **Schema:** `render(kind, title, content, id?)` where `kind ∈ {markdown, code, html, mermaid, table, image}` (+ optional `lang`/`filename` for code, `mime` for image).
- **Behavior:** returns `ToolOutput { content: <short text ack for the model>, display: Some(<Display variant>) }`. No `EventSink`, no `ToolCtx` change — it flows through the existing `tool_result` path.
- **Registration:** alongside existing builtin tools; covered by the tool registry tests.

### 5.3 Wire + frontend

- **`agent-server/src/wire.rs`:** the `Display` re-export already flows through `WireEvent::ToolResult.display`; add serde coverage for new variants (round-trip tests mirroring the existing `event_envelope_round_trips`).
- **`web/src/wire.ts`:** extend the `Display` TS union to mirror the new variants. `parseInbound` is unchanged (variants are leaf data).
- **`web/src/state.ts`:** reducer routes any `tool_result` carrying a `display` to the Inspector — open/focus a tab keyed by `id` (or tool-call id), replacing same-id content. Tool chips link to their artifact.
- **Inspector renderers (`web/src/components/inspector/`):** one component per kind.
  - `Markdown`, `Code` → reuse existing `react-markdown` + `rehype-highlight`/`highlight.js`.
  - `Diff`, `Terminal` → reuse existing `DiffView` / `TerminalBlock`.
  - `Html` → **sandboxed `<iframe sandbox>`** (no `allow-same-origin`, no scripts unless explicitly needed) to contain script/style bleed.
  - `Mermaid` → add the `mermaid` dependency; render to SVG.
  - `Table` → semantic `<table>` on the token system.
  - `Image` → `<img>` from data-URI or URL with `mime`.

### 5.4 v2 (documented target, deferred — NOT built this cycle)

A first-class streaming artifact channel, specified here as the contract so v1 doesn't paint us into a corner:

- `AgentEvent::Artifact { id, title, kind, payload }` in `agent-core`, decoupled from tool results.
- `WireEvent::Artifact` + a `WireBody`/event mapping in `agent-server`; Worker remains transparent.
- An `EventSink` handle added to `ToolCtx` so **any** tool can push/update artifacts mid-run (live-updating panes, streaming previews), not only via a tool's final `ToolOutput`.
- Frontend Inspector already keys tabs by artifact `id`, so v2 events update existing tabs with no UI rework.

v1's `id`/`title` on `Display` is the forward-compatible seam: v2 reuses the same identity model.

### 5.5 Security

Artifacts originate from the user's own trusted local daemon. Even so, `Html` is rendered in a sandboxed iframe to contain script/style/style-leak; `Image` data is size-bounded; markdown/code go through the existing sanitizing markdown pipeline. No artifact is given same-origin privileges.

## 6. Component inventory (old → new)

Restyled to tokens: `PairingScreen`, `StatusBar` (+ theme toggle), `Composer`, `MessageList`, `AssistantMessage`, `ReasoningMessage`, `ToolCall` (→ chip + Inspector trigger), `ApprovalPrompt`, `SettingsPanel`.
New: `AppShell`, `ActivityRail`, `Inspector` + `inspector/*` renderers, `ThemeToggle`.
Reused inside Inspector: `DiffView`, `TerminalBlock`, `MarkdownText`.
Retired: `TimelineView` (function → `ActivityRail`).
Animations: keep framer-motion (`Animated*` components), re-tune transitions.

## 7. Motion, states, testing

- **Motion:** refine enter/expand/collapse transitions for messages, chips, rail, and Inspector; gate all on `prefers-reduced-motion`.
- **States (all themed):** empty (no session), connecting/reconnecting, agent-offline, error, approval, and Inspector-empty.
- **Testing:**
  - Frontend (vitest + RTL): a render test per artifact kind; theme switch; rail/inspector collapse + resize persistence; reducer routing of `display` → Inspector tab (incl. replace-by-id); tool-chip ↔ artifact focus.
  - Rust: serde round-trip per new `Display` variant; `render` tool produces the expected `ToolOutput.display`; tool registry includes `render`.
  - Regression: existing wire round-trip tests stay green (envelope unchanged).

## 8. Sequencing & risks

**Sequencing** (one shippable slice; frontend + Display + render tool land together):
1. Design system tokens + theme switch (`index.css`, `storage.ts`, `ThemeToggle`).
2. `AppShell` + `ActivityRail` + `Inspector` scaffolding; retire `TimelineView`; restyle existing components.
3. `Display` enum extension + serde tests (`agent-tools`, `agent-server`).
4. Builtin `render` tool + tests.
5. `wire.ts` + reducer routing + Inspector renderers (markdown/code/diff/terminal first, then html/mermaid/table/image).
6. Motion/state polish; full test pass.

**Risks / mitigations:**
- *Mermaid bundle size* — lazy-load the renderer so it doesn't bloat first paint.
- *HTML safety* — strict iframe sandbox; never same-origin.
- *Theme regressions* — enforce "tokens only, no raw `zinc-*`" so both themes stay consistent.
- *Wire compatibility* — additive variants + `content` fallback keep older clients/daemons working without a version bump.
