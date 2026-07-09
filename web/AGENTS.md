# web/ — browser SPA

React 19 / Vite / Tailwind single-page app (the Context Explorer UI) — the Tauri
desktop app's frontend. Reaches the *local* agent over **Tauri IPC** (`agent-server`)
via `invoke()`/`Channel` (see `src/socket.ts`, gated on `isTauri()` in `src/transport.ts`).
Outside Tauri the app shows a "use the desktop app" notice; there is no browser-direct
or Cloudflare Worker transport.

## Commands

```bash
npm test            # vitest
npm run typecheck
npm run build       # tsc -b && vite build
```
