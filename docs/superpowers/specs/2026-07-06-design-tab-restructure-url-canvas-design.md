# Design: Right-pane tab restructure + live-URL canvas rendering

**Date:** 2026-07-06
**Status:** Approved (brainstorm complete)

## Problem

1. The Design tab nests three subtabs (Canvas | Config | Architecture). Config and
   Architecture are runtime introspection surfaces, not design surfaces — they don't
   belong under Design.
2. The agent's `render` tool has no way to point the Design canvas at a running dev
   server. Its only live-UI option is `kind=html` with an inlined standalone document,
   so the model authors throwaway HTML files instead of using the real Vite-served
   app. Feedback pins then annotate the standalone copy, and fixes drift away from
   the actual code.

## Decisions made during brainstorm

- Config and Architecture become **top-level right-pane tabs** (Tauri-only, as today).
- Live rendering is an **iframe of the real dev server**, not a screencast of the
  sandbox Chromium. Screencast (CDP frame streaming + input forwarding, element-level
  click resolution) is explicitly deferred as its own future project.
- URL placement is **agent tool + manual field**: the agent gets a `url` render kind,
  and the canvas gets a small URL input as fallback.
- URL policy is **localhost-only** (any port), enforced in the Rust tool and
  re-validated client-side.

## Part 1 — Promote Config and Architecture to top-level tabs

- `RightTab` (`web/src/storage.ts`) becomes
  `"workspace" | "context" | "design" | "architecture" | "config"`.
- `RightPaneTabs.tsx` renders five tabs: Workspace | Context | Design | Architecture
  | Config. Architecture and Config appear only when `isTauri()` — the same gating
  `DesignPane` applies today, moved up one level.
- `loadRightTab` falls back to `workspace` when the stored value is a Tauri-only tab
  and the app is not running under Tauri.
- `DesignPane` loses its subtab strip and every settings-related prop
  (`settings`, `settingsMeta`, `settingsError`, `onSaveSettings`, `onLoadSettings`);
  it becomes pure canvas (design list + `DesignCanvas`).
- `App.tsx` renders `ConfigPanel` / `ArchitecturePane` directly for the new tabs and
  sends `settings_get` when the Config tab is selected (matching the current
  subtab's `onLoadSettings` behavior).

## Part 2 — `url` render kind, end to end

### Rust (`agent/`)

- New `Display::Url { url: String, title: Option<String>, id: Option<String> }` in
  the shared `Display` enum (`agent-tools/src/types.rs` — the single definition; it
  serializes straight onto the wire).
- `render` tool (`agent-tools/src/render.rs`): accept `kind: "url"` with `content`
  as the URL. Validation inside `execute`:
  - scheme must be `http` or `https`;
  - host must be `localhost`, `127.0.0.1`, or `[::1]` (any port);
  - anything else → `ToolError::InvalidArgs`.
- **Steering (the drift fix):** rewrite the tool description and the `kind`/`content`
  schema descriptions so the model learns: when a real dev server is running (e.g.
  Vite), render `kind=url` pointing at it; `kind=html` is for one-off static mockups
  only. A test pins this guidance the same way
  `description_documents_the_design_canvas_convention` pins the `design:` convention.

### Web (`web/`)

- `wire.ts`: add `{ Url: { url: string; title?: string; id?: string } }` to `Display`.
- `designStore.ts`: add `"Url"` to the `RENDERABLE` set.
- New `inspector/UrlArtifact.tsx`: an iframe with
  `sandbox="allow-scripts allow-same-origin"` (a live app needs scripts and its own
  origin; the target is the user's own localhost server). Client-side re-validation
  of the localhost rule — a non-localhost URL renders an inline error, never an
  iframe.
- **Mixed-content notice:** if the SPA itself is served over `https:` (cloud path),
  the browser blocks an `http://localhost` iframe. Detect
  (`window.location.protocol === "https:"` and target is `http:`) and render a
  clear notice instead of a silently broken frame. Live-URL rendering is effectively
  a Tauri-app / locally-served-SPA feature.
- **Interact vs. pin toggle:** for URL versions the canvas gets a two-state toggle —
  *Interact* (annotation overlay's pin layer gets `pointer-events: none`; the user
  drives the app normally) and *Pin feedback* (overlay captures clicks as today).
  Non-URL versions keep today's always-on pin layer.
- **Pins on live apps are viewport-relative:** the iframe's scroll position is
  invisible cross-origin, so `x_pct`/`y_pct` describe the visible viewport at the
  moment of pinning. Accepted limitation, documented in code.
- **Feedback payload:** `buildFeedbackMessage` gains an optional `url` field in the
  `design-feedback` JSON payload, included when the annotated version is a URL
  render, so the agent knows exactly which page/route the pins refer to. Existing
  fields are untouched; the golden test gains a case for the extended payload.
- **Manual URL field:** a small URL input on the canvas. Submitting a URL
  creates/updates a local design with id `design:live-preview` in the design store
  (localStorage-backed like any other design), so versioning, pins, and feedback
  work identically whether the agent or the user placed the URL. Same localhost
  validation, same inline error on violation.
- **Versioning:** `url` renders append versions through the existing `design:`
  mechanics. Content is live, so stepping versions only matters when the URL itself
  changed (e.g. different routes); compare mode simply shows two iframes.

## Out of scope

- **Chromium screencast to the canvas** (CDP `Page.startScreencast` frame streaming,
  input forwarding, element-level click resolution). Deferred; revisit as its own
  project if viewport-relative pins prove too coarse.
- **Sandbox networking for agent-side browsing.** The agent can already drive
  Playwright/Chromium in the sandbox image against the same dev server; a sandboxed
  container reaching the host's Vite needs host-gateway networking, which this
  design does not touch. Sandbox containers are one-shot with `--network
  none|bridge` and no published ports, so the assumed topology is: **Vite runs on
  the host**; canvas and agent both point at it.

## Testing

Rust (`cargo test -p agent-tools`):
- `kind=url` with `http://localhost:5173` → `Display::Url` output;
- external host and non-http(s) scheme → `InvalidArgs`;
- steering text pinned (description mentions preferring the dev server URL over
  standalone html).

Web (`npm test`):
- `RightPaneTabs`: five tabs, Tauri gating of Architecture/Config;
- `storage`: Tauri-only stored tab falls back to `workspace` outside Tauri;
- `DesignPane`: subtabs gone, pure canvas;
- `App`: Config tab triggers `settings_get`;
- `UrlArtifact`: localhost guard, non-localhost inline error, mixed-content notice;
- canvas: interact/pin toggle switches overlay pointer-events; URL versions render
  `UrlArtifact`;
- `designStore`: `Url` displays are renderable and version normally;
- `designFeedback`: golden test for payload with `url` field; existing golden
  unchanged.
