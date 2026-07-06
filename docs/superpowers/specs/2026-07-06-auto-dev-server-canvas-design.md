# Auto-launched dev server in the Design canvas

**Date:** 2026-07-06
**Status:** Design approved, pending spec review
**Surface:** Tauri desktop app (`src-tauri/`) + web SPA (`web/`)

## Problem

The Design tab can already render a running localhost dev server in an iframe, but
only if the user *manually* types the URL into a field (`DesignPane.tsx`) or the
agent points the canvas at it via `render kind=url`. In both cases the user is
responsible for starting the dev server themselves in a separate terminal.

We want the harness to do that work: detect a dev script in the workspace's
`package.json`, launch the dev server on a single click, and render the resulting
URL into the canvas automatically — then let the user annotate the live preview
without leaving "drive the app" mode.

## What already exists (reused, not rebuilt)

- **Live-URL canvas** — a `Display::Url` variant + a guarded live-preview iframe.
  The manual URL field calls `store.addUrlVersion(url)` (`designStore.ts`), which
  creates/extends the `design:live-preview` design. All version-bar, compare, and
  pin plumbing already flows from there.
- **Feedback layer** — `DesignCanvas.tsx` has an **Interact / Pin feedback** mode
  toggle. Interact makes the iframe drivable (`AnnotationOverlay` `passthrough`);
  Pin captures annotation pins → a frozen `design-feedback` chat message that
  carries `{design_id, version, pins, url?}`.
- **Two-layer localhost guard** — `isLocalUrl` (`urlGuard.ts`) is the authoritative
  WHATWG-parser guard on the only iframe choke-point; a coarser Rust check backs it.

**This feature adds the automation** — detect → spawn → discover URL → feed the
existing `addUrlVersion` seam — plus a shift-to-annotate affordance. Everything
downstream of the discovered URL is existing, tested code.

## Scope & platform

Desktop-only (Tauri). In the browser-via-Cloudflare-Worker path, "localhost" is the
*user's browser machine*, not where the agent/workspace lives, so there is nowhere
reachable to spawn a server. The launcher is `isTauri()`-gated on the web side and
the commands exist only in the Tauri handler set; the browser path exposes no spawn
surface, keeping the remote-exposure question closed.

The harness can spawn processes directly with `tokio::process` (already a full-feature
dependency in `src-tauri/`) — no `tauri-plugin-shell` needed. We control the exact
command in Rust.

## Design decisions (locked)

| Decision | Choice |
|----------|--------|
| Launch trigger | **One-click, auto-detected.** Detection + prefill are automatic; nothing spawns until a single explicit click. No arbitrary repo code runs without consent. |
| URL discovery | **Parse the server's stdout** for the announced `http(s)://localhost:<port>` line. |
| Multiple candidates | **Show a ranked picker** (labeled by dir + script), best guess first. |
| Lifecycle | **One managed server at a time.** Explicit Stop/Restart; force-killed on app quit and workspace change; a new start replaces the old. Survives tab switches. |
| Feedback affordance | **Shift-click always drops a pin**, even while driving the live app in Interact mode. The mode toggle stays. |

## Architecture & components

### Rust — `DevServerManager` (`src-tauri/src/devserver.rs`)

A struct held in `AppState`, guarding at most one child:

```
DevServerManager {
  current: Mutex<Option<Running>>,   // Running { child, candidate, url, log_tail }
}
```

- **`detect(workspace) -> Vec<DevScriptCandidate>`** — walk the workspace for
  `package.json` files (skip `node_modules`, bounded depth), read each `scripts`
  map, emit a candidate per server-ish script (`dev`, `start`, `serve`,
  `storybook`, `preview`). Package manager inferred from the nearest lockfile
  (`pnpm-lock.yaml` → pnpm, `yarn.lock` → yarn, else npm).
  Ranking: script literally named `dev` first, then shallowest `dir`, then
  alphabetical.

  ```
  DevScriptCandidate { dir: String, script: String, package_manager: String, label: String }
  ```

- **`start(candidate) -> Result<DevServerStatus, String>`** — run
  `<pm> run <script>` in `candidate.dir` via `tokio::process::Command` in its own
  **process group** (`process_group(0)`) so the whole `npm → node` tree is killable
  together. Stream stdout+stderr line-by-line into a bounded log tail. As lines
  arrive, match a small regex set for `https?://(localhost|127\.0\.0\.1):<port>`;
  the first hit resolves with the URL. A ~30s timeout resolves to an error carrying
  the captured log tail. A `start` first `stop()`s any existing server.

  ```
  DevServerStatus { url: String, candidate: DevScriptCandidate, state: "running" }
  ```

- **`stop()`** — kill the process group. Called on a new `start`, on workspace
  change (hook in `bridge.set_workspace`), and on app exit (`Drop` + window-close).

- **`status() -> Option<DevServerStatus>`** — the current running server, if any.

**New Tauri commands** (mirror the `architecture_get` pattern; added to the
`all_handlers!` macro so the production and test handler lists cannot drift):
`dev_scripts_detect`, `dev_server_start`, `dev_server_stop`, `dev_server_status`.

### Web — launcher in `DesignPane.tsx`

On mount (Tauri only, `isTauri()`-gated), call `dev_scripts_detect`. Render a
launcher row above the canvas:

- **0 candidates** → nothing new; the existing manual URL field remains the fallback.
- **1 candidate** → a single **Start dev server** button (labeled with dir/script).
- **≥2** → a ranked picker (chips or dropdown) + Start.
- **Running** → a status pill (url + candidate) with **Stop** and **Restart**.

On start: `dev_server_start` returns the URL → `store.addUrlVersion(url)` (the
existing seam) → canvas renders the guarded iframe. Non-Tauri browsers see no
launcher and keep the manual field.

## Data flow

```
Design tab mounts (Tauri)
  → dev_scripts_detect(workspace)  ──▶ [DevScriptCandidate...] (ranked)
User clicks Start on a candidate
  → dev_server_start(candidate)
       spawn `<pm> run <script>` in candidate.dir (own process group)
       stream stdout/stderr → parse first localhost URL (≤30s)
  ──▶ DevServerStatus { url, candidate, state }
  → store.addUrlVersion(url)        (existing seam)
  ──▶ canvas renders guarded iframe (isLocalUrl still gates it)
User drives the live app (Interact mode)
  holds Shift + clicks ──▶ pin dropped
  releases Shift       ──▶ back to driving
Send feedback ──▶ existing `design-feedback` message (carries url + pins)
Stop / Restart / workspace-change / app-quit ──▶ process-group kill
```

## Shift-click feedback

Today `passthrough` (iframe drivable, overlay ignores clicks) is
`!!liveUrl && interact`. Add a window-level `shiftHeld` tracker (keydown/keyup on
`Shift`) and derive:

```
passthrough = !!liveUrl && interact && !shiftHeld
```

While Shift is held the annotation overlay re-captures clicks → a click drops a pin;
releasing Shift returns to driving the live app. The **Interact / Pin feedback**
toggle is unchanged, for people who prefer explicit modes and for static (non-URL)
designs. `AnnotationOverlay` is untouched except for receiving the derived
`passthrough` — no new pin logic.

**Stuck-modifier guard:** a `keyup` fired while focus is inside the iframe won't reach
the parent window, so also clear `shiftHeld` on window `blur` to avoid getting stuck
in pin mode.

## Lifecycle & error handling

- **No candidates / no scripts** → launcher hidden; manual URL field is the fallback.
- **Spawn failure** (pm missing, script exits immediately) → `dev_server_start`
  returns an error carrying the captured log tail; launcher shows the message + Retry;
  state returns to stopped.
- **URL never appears within timeout** → same error path, log tail shown; manual field
  remains available.
- **Server crashes after boot** → iframe goes blank (a post-boot crash isn't reliably
  detectable); **Restart** + the visible log tail are the recovery affordances. Not
  auto-restarted.
- **Orphan prevention** → process-group kill on every teardown path plus `Drop` on the
  manager. A new `start` always stops the previous server first (the one-server rule).
  Workspace change stops it (the old server points at a stale dir).

## Security

- **Consent gate** — nothing spawns without the single explicit click; no auto-execution
  of repo code on tab open.
- **Constrained command** — only `<pm> run <script>` where `script` was detected in a
  `package.json` under the current workspace; never an arbitrary string from the UI.
  Package manager comes from the lockfile, not user input.
- **Guard unchanged** — `isLocalUrl` still gates the iframe even though we produced the
  URL ourselves (defense in depth).
- **Desktop-only** — `isTauri()`-gated web side; commands only in the Tauri handler set;
  no spawn surface on the browser-via-Worker path.

## Testing

**Rust (`src-tauri`)**
- Detection/ranking over temp-dir fixtures (root + `web/` monorepo shape; lockfile → pm
  inference; script filtering; ranking order).
- URL parsing from captured sample stdout lines (Vite `Local:`, Next, CRA,
  `127.0.0.1` variants, no-match → timeout).
- Spawn + kill using a fake server script
  (`node -e "console.log('Local: http://localhost:5199'); setInterval(()=>{},1e9)"`)
  so tests need no real framework; assert URL resolves and process-group kill leaves no
  surviving child.
- Workspace change stops the running server.

**Web (vitest)**
- Launcher rendering for 0 / 1 / many candidates; Start → `addUrlVersion` called with the
  returned URL; Stop/Restart wiring.
- `shiftHeld` toggles `passthrough`: pin placement works while Shift held in Interact
  mode; window `blur` clears the stuck state; guard still blocks a non-local URL.

## Out of scope (YAGNI)

- Multiple concurrent managed servers.
- Auto-restart on crash.
- A full streaming console (only a bounded error-time log tail).
- Persisting the per-workspace candidate choice across sessions.
