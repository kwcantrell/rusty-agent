# Claude Design Tab — Design

**Date:** 2026-07-05
**Status:** Approved (brainstorm complete)
**Scope:** Stage 1 of the "Claude design" hub tab — design canvas + prompt/config designer

## Context

The GUI (web SPA + Tauri desktop, one React app) has a right pane with two tabs:
**Workspace** (artifacts) and **Context**. This spec adds a third tab, **Design** —
a hub where the user iterates on visual designs with the agent and shapes the
agent's own configuration.

The full vision has four parts; this round decomposed them:

1. **Design canvas** (this spec) — agent renders mockups/diagrams; user iterates.
2. **Prompt/config designer** (this spec) — edit system prompt + runtime knobs from the GUI.
3. **Architecture viewer** — later stage, same hub tab, own spec.
4. **Claude-style visual theme** — separate app-wide effort, own spec.

## Decisions (made during brainstorm)

| decision | choice |
|----------|--------|
| Structure | One hub tab with sub-sections, built in stages |
| Stage 1 | Canvas + config designer together |
| Canvas interaction | **Annotate-on-canvas**: versioned surface + click-to-pin comments sent as structured feedback |
| Config apply model | **Live where safe**: sampling knobs + system prompt hot-apply at next turn boundary; structural changes take effect next session |
| Remote exposure | **Desktop-only config**: canvas works everywhere; config mutation exists only in Tauri — the Worker path never gains config endpoints |
| Architecture | **Approach A (artifact-native)** now, with clean seams for a later migration to **Approach B (first-class design channel with server-persisted versions + structured feedback tool results)** once V1 proves the UX |

## Architecture overview

The Design tab is added to `RightPaneTabs.tsx` (`workspace | context | design`);
`RightTab` in `storage.ts` is extended. Inside the tab a sub-nav switches between:

- **Canvas** — versioned design surface. The agent uses the existing `render`
  tool with a reserved id convention `design:<name>`. The client intercepts
  `design:`-prefixed artifacts: each re-render of an id appends a **version**
  to that design's stack instead of replacing in place. Rendering reuses
  `ArtifactRenderer` (HTML, SVG, Mermaid, image already supported). An
  annotation overlay provides click-to-pin comments; feedback returns to the
  agent as a structured chat message.
- **Config** (Tauri-only) — form editor over `RuntimeConfig` + system prompt,
  backed by new Tauri commands into the embedded `agent-server` runtime.
  The SPA build shows the canvas but renders no config UI at all.

No changes to agent-core, the wire protocol, or the Cloudflare Worker.

### B-migration seams (designed in now)

- All version logic lives behind a **`DesignStore` interface** in the client
  (v1 impl: in-memory + localStorage). B swaps in a server-backed store
  without touching canvas components.
- The feedback message is a **fixed JSON schema** in a fenced block; the same
  schema becomes the `DesignFeedback` tool-result payload in B. Pinned by a
  golden test.
- No component reads `Display` off the socket directly; everything goes
  through the store, so a future `DesignUpdate` wire event lands in one place.

## Components

### Web (`web/src/`)

| unit | responsibility |
|------|----------------|
| `components/design/DesignPane.tsx` | Hub pane; sub-nav Canvas/Config; hides Config outside Tauri |
| `components/design/DesignCanvas.tsx` | Renders active version via `ArtifactRenderer`; hosts version bar + overlay |
| `components/design/VersionBar.tsx` | Version stepper, jump-to-latest, side-by-side compare toggle |
| `components/design/AnnotationOverlay.tsx` | Click-to-pin layer; draft pins (pct coords + comment); Send feedback |
| `components/design/ConfigPanel.tsx` | Tauri-only form; "applies live" vs "next session" groups; per-field state |
| `designStore.ts` | `DesignStore` interface + v1 impl: intercept `design:` artifacts, per-design version stacks, localStorage per session |
| `designFeedback.ts` | Builds structured feedback message; hands to existing chat-send path |

Existing files touched: `RightPaneTabs.tsx`, `storage.ts`, `App.tsx`,
`state.ts` (artifact reducer routes `design:` ids to the store).

Boundaries: canvas talks only to `DesignStore`; overlay talks only to
`designFeedback`; config panel talks only to Tauri commands.

### Rust

- **`src-tauri`** — new commands: `get_runtime_config`, `set_runtime_config(partial)`,
  `get_system_prompt`, `set_system_prompt`, `apply_live(settings)`. Call into the
  embedded agent-server runtime. (Separate workspace — mind `-p` targets.)
- **`agent-server/src/runtime.rs`** — narrow config-mutation API used only by the
  Tauri commands, never exposed on the Worker path: validate → write config file
  (atomic temp+rename) → for the live subset, stage updates the loop picks up at
  the next turn boundary.
- **`agent-tools/src/render.rs`** — description text extended to document the
  `design:<id>` convention. Behavior unchanged.

## Data flow

**Render → version stack.** `render` with `id: "design:landing-page"` flows the
existing tool-result path. The artifact reducer routes `design:` ids to
`DesignStore`: new id → stack `[v1]`; existing id → append `vN+1` and auto-jump,
unless the user is viewing an older version, in which case show a "new version"
badge instead of yanking the view. Non-`design:` artifacts are untouched.
Store mirrors to localStorage keyed by session id — reload restores stacks
within a session; **cross-session persistence is out of scope for v1** (that is
the B migration).

**Annotate → feedback turn.** Pins are local drafts; nothing sends until
"Send feedback". `designFeedback.ts` serializes one chat message: a short
human-readable header + a fenced `design-feedback` JSON block:

```json
{ "design_id": "...", "version": 3,
  "pins": [{ "x_pct": 0.42, "y_pct": 0.10, "comment": "..." }],
  "note": "optional overall note" }
```

Sent as a normal user turn via the existing composer path — works on any
backend, mid-conversation, and lands in the session trace. Sent pins remain
visible on that version, marked "sent".

**Config, live path.** Panel loads via `get_runtime_config`/`get_system_prompt`.
Live-safe subset — `temperature`, `top_p`, `top_k`, `min_p`, penalties,
`max_tokens`, system prompt — goes through `apply_live`: staged in the runtime,
picked up at the next **turn boundary** (never mid-turn), and also persisted so
it survives restart. Note: the sampling knobs are `RuntimeConfig` fields and
persist to the config file; the system prompt is assembled in
`agent-runtime-config/src/prompts.rs` (`LoopParts.base_system_prompt`), so its
persisted home (config field vs prompt override file) is resolved during
planning — the UI contract (edit, hot-apply at turn boundary, survives restart)
is fixed either way.

**Config, next-session path.** Structural fields — `backend`, `model`,
`base_url`, `protocol`, tool allow/deny lists, `http_allow_hosts`, context and
subagent settings — write the config file only, validated Rust-side with the
same validation CLI startup uses. UI labels them "takes effect next session"
with a dirty-vs-active indicator.

## Error handling & edge cases

- **Bad payloads still version:** a `design:` artifact that fails to render
  (bad HTML/Mermaid) still becomes a version — `ArtifactRenderer` degrades per
  kind — so the user can flip back; the version bar marks it failed.
- **Bounded stacks:** keep last **N=20** versions per design, oldest dropped.
  localStorage writes use the existing storage wrapper (tolerates
  `SecurityError`/quota → in-memory fallback, per `storage.securityerror.test.ts`).
- **Pins:** percentage coordinates of the rendered artifact box — survive pane
  resizes. Versions are immutable snapshots, so pinned content never shifts
  under a pin. Overlay is a div above iframe-hosted HTML — no cross-frame events.
- **Feedback send failure:** draft pins are kept; existing composer retry/error
  UX applies. Feedback is never silently lost. The JSON block is generated,
  never hand-edited; if a weak model ignores it, the human-readable header
  still conveys the gist.
- **Config validation:** validate-then-write; rejected edits return per-field
  errors shown inline; nothing partially applied; file written atomically.
- **External edits:** `set_runtime_config` carries a content hash of the loaded
  base; if the file changed on disk (e.g. CLI edit), refuse with
  "config changed externally — reload".
- **Dead session:** `apply_live` with no live session degrades to
  file-write-only and reports so.
- **Non-Tauri:** Config sub-nav entry does not render at all (same
  `window.__TAURI__` probe as `get_workspace`).

## Testing

Web (vitest):

- `designStore.test.ts` — interception vs plain artifacts, version append
  (immutable), auto-jump vs pinned badge, N=20 bound, localStorage round-trip +
  SecurityError fallback, session-key isolation.
- `designFeedback.test.ts` — **golden test of the JSON schema** (B-migration
  contract), pct serialization, draft/sent transitions.
- `DesignCanvas.test.tsx` / `VersionBar.test.tsx` — active-version render,
  stepping, compare mode, failed-version marker.
- `AnnotationOverlay.test.tsx` — click→draft at pct coords, edit/delete,
  send disabled with zero pins, retained sent pins.
- `ConfigPanel.test.tsx` — live vs next-session grouping, dirty indicators,
  inline errors, hidden when not Tauri (mock per `App.tauri.test.tsx`).
- `RightPaneTabs`/`storage` — third tab value + stale stored value migration.

Rust:

- `agent-server` — validation rejects with field errors; atomic write;
  stale-hash refusal; `apply_live` staging consumed at turn boundary;
  dead-session degradation.
- `src-tauri` — command wiring smoke tests (its own workspace).
- `agent-tools` — `render` description mentions `design:` (snapshot assertion).

Manual/e2e: drive the real app — render `design:demo` twice → two versions →
pin → send → feedback turn visible in session JSONL; edit temperature →
visible in next turn's request.

## Out of scope (v1)

- Cross-session / server-persisted version history (Approach B).
- Structured tool-result delivery of feedback (Approach B).
- Architecture viewer sub-section (later stage, own spec).
- Claude-style visual theme (separate effort, own spec).
- Config editing over the Worker/browser path (deliberately never).
