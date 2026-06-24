# Design Spec — Web Frontend Redesign (two-pane builder layout, light editorial)

**Date:** 2026-06-24
**Status:** Approved design — ready for `writing-plans`.
**Scope:** The `web/` React SPA only (view layer). **No daemon / cloud / wire-protocol changes.**
**Reference:** an AI web-builder IDE screenshot (two-pane: agent column + large Preview/Code workspace, light editorial chrome).

---

## 1. Goal

Restructure and restyle the existing `web/` client into a **two-pane "builder" layout** with a **light editorial aesthetic**, matching the reference's *chrome* (not its dark previewed content). Left = the agent (project header + conversation + composer); right = a dominant **Preview/Code workspace** for the agent's artifacts.

This is a **restructure-and-restyle, not a rewrite**: `socket.ts`, `wire.ts`, `state.ts`, `storage.ts` are untouched; existing message/tool/artifact components are reused and re-composed under a new shell + design-token system.

### Non-goals / out of scope
- No backend, daemon, Cloudflare Worker, or wire-protocol change.
- No `Share` action; no browser-style multi-tab top bar (single session).
- No new agent capabilities — the workspace renders only what `state.items` already produces (`artifactsFrom`).
- No conversation virtualization or other perf rework beyond what exists.
- `Download` is **not** in this slice (deferred; see §9).

---

## 2. Layout architecture

A new top-level shell replaces today's `StatusBar + (ActivityRail | MessageList | Inspector) + Composer`:

```
TopBar   project/session label · theme toggle · settings ⚙ · sign out
├── AgentColumn (~38%, min ~360px)            WorkspacePane (~62%)
│   • AgentHeader: project title (serif),      • WorkspaceHeader: artifact tabs ·
│     model name, working-dir/session id         [Preview│Code] segmented · viewport ▼
│   • Conversation: existing MessageList       • WorkspaceBody:
│     (AssistantMessage, ToolCall,                – Preview → ArtifactRenderer (HtmlArtifact/
│      ReasoningMessage, DiffView, …)               Mermaid) in a viewport-sized frame
│   • Composer pinned at bottom                  – Code → active artifact source (highlight.js)
│   • ApprovalPrompt surfaces inline             – Empty state when no artifacts
```

Proportions: AgentColumn ~38% (min ~360px), WorkspacePane ~62%, on desktop.

### 2.1 Component fate
- **Reused (logic unchanged) / restyled:** `MessageList`, `AssistantMessage`, `AnimatedAssistantMessage`, `ToolCall`, `AnimatedToolCall`, `ReasoningMessage`, `AnimatedReasoningMessage`, `DiffView`, `TerminalBlock`, `MarkdownText`, `AnimatedError`, `ApprovalPrompt`, `Composer`, `SettingsPanel`, `PairingScreen`, and inspector internals `ArtifactRenderer`, `HtmlArtifact`, `MermaidArtifact`.
- **Repurposed:** `StatusBar` → `TopBar` (slim top chrome); `ActivityRail`'s controls (settings, sign-out, theme, session label) fold into `TopBar` / `AgentHeader`. The icon rail is **dropped**.
- **New (thin shells):** `AgentColumn`, `WorkspacePane` (replaces the 360px `Inspector` wrapper), `TopBar`, `AgentHeader`, `WorkspaceEmptyState`.
- **Unchanged:** `socket.ts`, `wire.ts`, `state.ts`, `storage.ts`. `App.tsx` slims to compose `TopBar` + `AgentColumn` + `WorkspacePane`, keeping the socket/state wiring it already owns (`send`, `decide`, `openSettings`, `saveSettings`, `activeArtifactKey`).

---

## 3. Workspace pane behavior

### 3.1 Artifact tabs
Driven by the existing `artifactsFrom(state.items)`. One tab per artifact, label = title/filename + a type glyph. Click selects (`activeArtifactKey`, already in `App`). New artifacts auto-select the latest (current behavior). Overflow scrolls horizontally. Empty list → no tabs; body shows the empty state.

### 3.2 Preview │ Code segmented toggle
A per-workspace mode (local state, not per-artifact):
- **Preview** → renders the active artifact via the existing `ArtifactRenderer` (`HtmlArtifact` sandboxed iframe, `MermaidArtifact`, …).
- **Code** → the artifact's raw source in a read-only, syntax-highlighted view using the already-present `highlight.js`. If an artifact has no meaningful source, the Code toggle is **disabled** with a tooltip.

### 3.3 Viewport selector (Desktop / Tablet / Mobile)
Applies to **Preview only**. Constrains the preview frame to a centered max-width: Desktop = 100%, Tablet ≈ 820px, Mobile ≈ 390px, with the editorial background around it. **Disabled** in Code mode.

### 3.4 State source
`mode` (`preview|code`) and `viewport` (`desktop|tablet|mobile`) are local `useState` in `WorkspacePane`, optionally persisted via an additive `storage.ts` key so a reload restores the last view. Everything else reads existing app state. **No new wire/state shape.**

---

## 4. Aesthetic system (light editorial)

Centralized as **design tokens** in `theme.ts` + `index.css` (Tailwind v4 `@theme`). Light is primary; dark is a parallel token set (existing `applyTheme` / `ThemeToggle` keep working).

**Typography:**
- **Display serif** (recommend **Fraunces**, fallback **Instrument Serif**) for the project title, workspace empty-state headline, and section headings — *accent/display only, never body*. **Self-hosted** vendored `woff2` (e.g. via `@fontsource`); **no runtime Google-CDN fetch** (served same-origin by the Worker; must stay local-first/offline-safe).
- **Body sans** (**Inter**, self-hosted) for conversation, controls, labels. Existing mono stack for code/terminal/diff.

**Surfaces & color (light):** warm off-white base (~`#FBFAF8`), white raised cards, **hairline `1px` low-contrast borders**, soft shadows used sparingly. A single restrained **ink/charcoal accent** for primary actions (echoing the reference's near-black pill) — no saturated brand color.

**Controls:** `rounded-full` pills for primary buttons + the segmented Preview│Code toggle; `rounded-xl` for cards/tabs/inputs; quiet hover; visible focus rings. `Composer` becomes a soft rounded input block.

**Spacing & density:** generous padding, comfortable conversation line-height, clear message-group separation. Editorial calm over density.

**Guardrail:** token + class pass over existing components — no logic changes, no unrelated refactors.

---

## 5. Responsiveness

Desktop-first. Below ~900px width: **AgentColumn goes full-width** (conversation is primary) and **WorkspacePane becomes a toggle/slide-over** opened from a `TopBar` button — same components, re-composed. No separate mobile visual design.

---

## 6. Accessibility

- Keyboard-navigable artifact tabs + Preview│Code segmented control (arrow keys, `role="tab"`/`aria-selected`).
- Visible focus rings on editorial controls.
- **AA contrast** verified for the light palette (warm off-white + ink accent).
- `prefers-reduced-motion` respected — gate the existing `framer-motion` animations.
- `HtmlArtifact` preview stays sandboxed (unchanged).

---

## 7. Failure modes (frontend — all sourced from existing state)

- **Disconnected** (`state.status`/`state.online`): `TopBar` shows offline; `Composer` disables; workspace keeps last artifacts inspectable.
- **No artifacts:** editorial empty state (serif headline + one supporting line).
- **Preview render error:** inline error card inside the sandboxed frame (reuse `AnimatedError` styling); pane never blanks.
- **Code mode, no source:** toggle disabled with tooltip.
- **Settings error:** existing `SettingsPanel` error surface.
- **Pre-auth:** `PairingScreen` full-screen (restyled).

---

## 8. Performance / footprint

- Self-host **subsetted** `woff2` (display weights only) — keep bundle lean.
- **No new heavy runtime deps** — reuse `highlight.js`, `mermaid`, `framer-motion`.
- `mode`/`viewport` are local state — no extra re-renders of the conversation.

---

## 9. Deferred (intentional)

- **Download** active artifact to a file — useful, but its own slice.
- **Share** — meaningless for a local paired session; omitted.
- **Multi-tab / multi-session** browser-style top bar — single session today.
- Conversation virtualization / large-history perf.

---

## 10. Testing

Existing **Vitest + jsdom + Testing Library**; `npm test` + `npm run build` stay green.
- Shell renders both panes; `TopBar` controls work (settings opens via `settings_get`, sign-out clears session, theme toggle).
- Artifact tabs list + select set the active artifact; a new artifact auto-selects the latest.
- Preview│Code toggle switches the body; Code is disabled when the active artifact has no source.
- Viewport selector changes the preview frame's constrained width (Desktop/Tablet/Mobile); disabled in Code mode.
- Empty-state renders with no artifacts; `Composer` disables when offline.
- `prefers-reduced-motion` gates animation (smoke).
- Existing `wire` / `state` / `reasoning` / `SettingsPanel` tests remain unaffected (no logic changes).

---

## 11. Definition of done

The `web/` client renders as a two-pane builder — a light, editorial **AgentColumn** (serif project header + reused conversation + composer) beside a dominant **WorkspacePane** with artifact tabs, a Preview│Code toggle, and a Desktop/Tablet/Mobile viewport selector that resizes the Preview — with the icon rail removed and its controls folded into the TopBar. Light + dark themes work; fonts are self-hosted; no backend/wire change. `npm test` + `npm run build` green; the new layout validated in the running app against a live session producing at least one artifact.
