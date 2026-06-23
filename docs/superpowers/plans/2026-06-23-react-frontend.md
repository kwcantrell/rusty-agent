# React Frontend (Subsystem #6) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Claude Code–style React chat UI (new top-level `web/` package) that is a pure WebSocket client of the existing Cloudflare Worker — rendering the local agent's streamed tokens, tool calls (diffs + terminal), approval prompts, and presence — served same-origin by the Worker via static assets.

**Architecture:** A standalone Vite + React + TS app. A typed socket client feeds inbound frames to a single reducer producing `ConversationState`; components render purely from that state. The existing `cloud/` Worker gains only an `assets` config block (`run_worker_first` for the four API/WS routes) so it serves the SPA same-origin (no CORS). Zero Rust/daemon/wire-protocol changes.

**Tech Stack:** React 19, Vite 7, TypeScript 5, Tailwind CSS v4 (`@tailwindcss/vite`), Vitest + React Testing Library (jsdom), the `diff` library. Cloudflare Workers static assets (Wrangler 4.103).

## Global Constraints

- **Behavior-preserving on the wire:** the Rust daemon and the wire protocol (`{ v, session_id, id?, kind, ... }`, event `payload: { type, ... }`) are UNCHANGED. The UI speaks the existing contract.
- **New code lives in top-level `web/`.** The only `cloud/` change is `wrangler.jsonc` config + a committed `web/dist/index.html` placeholder. No `cloud/src/*` change.
- **`cloud/` tests must stay green:** `cd cloud && npm test` (9 tests) must keep passing — the `assets.directory` placeholder makes this so.
- **Wire shapes (exact):** event frame `{v,session_id,kind:"event",payload:{type,...}}`; `payload.type ∈ token|tool_start|tool_result|error|done`; `approval_request` frame `{v,session_id,id,kind:"approval_request",summary,command?,display?}`; `presence` `{v,session_id,kind:"presence",online}`. Outbound: `user_input {text}`, `approval_response {decision}` (decision ∈ `approve|approve_always|deny`) echoing the request `id`. `Display` is externally-tagged serde: `{Text:string}` | `{Diff:{path,before,after}}` | `{Terminal:{command,stdout,stderr,exit_code}}`.
- **Same-origin:** the app connects to `${location.origin.replace("http","ws")}/browser?token=...` and `POST /pair` (relative) — works in dev (Vite proxy) and prod (Worker-served).
- Node 22 + npm available; npm registry reachable. Commands run from `/home/kalen/rust-agent-runtime/web` unless stated.

---

## File Structure

- `web/package.json`, `web/tsconfig.json`, `web/tsconfig.node.json`, `web/vite.config.ts`, `web/index.html`, `web/vitest.config.ts`, `web/test/setup.ts` — toolchain (Task 1).
- `web/src/main.tsx`, `web/src/index.css`, `web/src/App.tsx` — app shell (Task 1 stub, Task 7 real).
- `web/src/wire.ts` — wire types + `parseInbound` (Task 2).
- `web/src/state.ts` — `ConversationState`, `Item`, reducer + `initialState` (Task 3).
- `web/src/storage.ts` — localStorage helpers (Task 3).
- `web/src/socket.ts` — typed WS client (Task 4).
- `web/src/components/DiffView.tsx`, `TerminalBlock.tsx`, `ToolCall.tsx` — tool rendering (Task 5).
- `web/src/components/StatusBar.tsx`, `AssistantMessage.tsx`, `MessageList.tsx`, `ApprovalPrompt.tsx`, `Composer.tsx` — chat shell (Task 6).
- `web/src/components/PairingScreen.tsx` + `web/src/App.tsx` (real) — integration (Task 7).
- `cloud/wrangler.jsonc` (+ `web/dist/index.html` placeholder) — Worker config (Task 1).
- `cloud/RUNNING.md` — UI dev/build flow (Task 8).

---

## Task 1: Scaffold `web/` package + Worker assets config

Stand up the Vite/React/TS/Tailwind/Vitest toolchain, a trivial App, the Vite dev proxy, and the `cloud/` `assets` config with a placeholder dist — gated on `npm run build`, a smoke test, and `cloud/`'s tests staying green.

**Files:**
- Create: `web/package.json`, `web/tsconfig.json`, `web/tsconfig.node.json`, `web/vite.config.ts`, `web/vitest.config.ts`, `web/index.html`, `web/.gitignore`, `web/test/setup.ts`, `web/test/smoke.test.ts`, `web/src/main.tsx`, `web/src/index.css`, `web/src/App.tsx`, `web/dist/index.html`
- Modify: `cloud/wrangler.jsonc`

**Interfaces:**
- Produces: a buildable `web/` package and a Worker that serves `web/dist` same-origin with API routes via `run_worker_first`.

- [ ] **Step 1: package.json**

`web/package.json`:

```json
{
  "name": "web",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview",
    "test": "vitest run",
    "typecheck": "tsc -b --noEmit"
  },
  "dependencies": {
    "diff": "^7.0.0",
    "react": "^19.0.0",
    "react-dom": "^19.0.0"
  },
  "devDependencies": {
    "@tailwindcss/vite": "^4.0.0",
    "@testing-library/jest-dom": "^6.6.0",
    "@testing-library/react": "^16.1.0",
    "@testing-library/user-event": "^14.5.0",
    "@types/diff": "^7.0.0",
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "@vitejs/plugin-react": "^5.0.0",
    "jsdom": "^25.0.0",
    "tailwindcss": "^4.0.0",
    "typescript": "^5.6.0",
    "vite": "^7.0.0",
    "vitest": "^3.0.0"
  }
}
```

> After `npm install`, if any peer conflict appears, set the offending package to the version named in the error and reinstall; record resolved versions in your report.

- [ ] **Step 2: TypeScript + Vite + Vitest config**

`web/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "useDefineForClassFields": true,
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "types": ["vitest/globals", "@testing-library/jest-dom"]
  },
  "include": ["src", "test"]
}
```

`web/tsconfig.node.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2023"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowSyntheticDefaultImports": true,
    "strict": true,
    "noEmit": true
  },
  "include": ["vite.config.ts", "vitest.config.ts"]
}
```

`web/vite.config.ts`:

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// In dev, the browser hits Vite (:5173); proxy the Worker's API/WS routes to wrangler dev (:8787)
// so the app is same-origin from the browser's perspective (no CORS) and WebSockets work.
const target = "http://127.0.0.1:8787";
export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    proxy: {
      "/enroll": { target, changeOrigin: true },
      "/pair": { target, changeOrigin: true },
      "/agent": { target, ws: true, changeOrigin: true },
      "/browser": { target, ws: true, changeOrigin: true },
    },
  },
});
```

`web/vitest.config.ts`:

```ts
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./test/setup.ts"],
  },
});
```

`web/test/setup.ts`:

```ts
import "@testing-library/jest-dom/vitest";
```

- [ ] **Step 3: HTML, entry, styles, trivial App**

`web/index.html`:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>agent</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

`web/src/index.css`:

```css
@import "tailwindcss";
```

`web/src/main.tsx`:

```tsx
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./index.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
```

`web/src/App.tsx` (stub; Task 7 replaces it):

```tsx
export default function App() {
  return <div className="p-4 text-zinc-100">agent UI</div>;
}
```

`web/.gitignore`:

```
node_modules/
dist/
```

> Note: `dist/` is gitignored EXCEPT the committed placeholder below — force-add it in Step 6.

- [ ] **Step 4: Committed dist placeholder (keeps cloud tests green)**

`web/dist/index.html`:

```html
<!doctype html><html><head><meta charset="utf-8"><title>agent</title></head>
<body><div id="root">build placeholder — run `vite build` in web/</div></body></html>
```

- [ ] **Step 5: Worker assets config**

In `cloud/wrangler.jsonc`, add a top-level `"assets"` key (sibling of `"main"`, `"durable_objects"`, etc.):

```jsonc
  "assets": {
    "directory": "../web/dist",
    "not_found_handling": "single-page-application",
    "run_worker_first": ["/enroll", "/pair", "/agent", "/browser"]
  },
```

- [ ] **Step 6: Smoke test, install, build, verify both suites**

`web/test/smoke.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import App from "../src/App";

describe("App", () => {
  it("renders", () => {
    render(<App />);
    expect(screen.getByText(/agent UI/)).toBeInTheDocument();
  });
});
```

Run:

```bash
cd /home/kalen/rust-agent-runtime/web && npm install && npm run build && npm test
```
Expected: install OK; `vite build` writes `web/dist/assets/...` + overwrites `index.html`; 1 smoke test passes.

Then confirm cloud is unaffected:

```bash
cd /home/kalen/rust-agent-runtime/cloud && npm test
```
Expected: 9 tests still pass (the `assets.directory` now resolves to a real `web/dist`).

- [ ] **Step 7: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/package.json web/package-lock.json web/tsconfig.json web/tsconfig.node.json web/vite.config.ts web/vitest.config.ts web/index.html web/.gitignore web/test web/src cloud/wrangler.jsonc
git add -f web/dist/index.html
git status --short   # confirm web/dist/index.html staged, web/node_modules NOT staged
git commit -m "feat(web): scaffold React+Vite+Tailwind SPA; serve same-origin via Worker assets"
```

---

## Task 2: Wire types + frame parsing

**Files:**
- Create: `web/src/wire.ts`, `web/test/wire.test.ts`

**Interfaces:**
- Produces: `Display`, `WireEvent`, `Inbound`, `Outbound`, `Decision` types; `parseInbound(raw: string): Inbound | null`.

- [ ] **Step 1: Write failing tests with real frame shapes**

`web/test/wire.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { parseInbound } from "../src/wire";

describe("parseInbound", () => {
  it("parses a token event", () => {
    const f = parseInbound(JSON.stringify({ v: 1, session_id: "s", kind: "event", payload: { type: "token", text: "hi" } }));
    expect(f).toEqual({ v: 1, session_id: "s", kind: "event", payload: { type: "token", text: "hi" } });
  });
  it("parses a tool_result with a Terminal display", () => {
    const f = parseInbound(JSON.stringify({
      v: 1, session_id: "s", kind: "event",
      payload: { type: "tool_result", name: "execute_command", content: "exit=0",
        display: { Terminal: { command: "echo hi", stdout: "hi\n", stderr: "", exit_code: 0 } } } }));
    expect(f?.kind).toBe("event");
    if (f?.kind === "event" && f.payload.type === "tool_result") {
      expect(f.payload.display).toEqual({ Terminal: { command: "echo hi", stdout: "hi\n", stderr: "", exit_code: 0 } });
    } else throw new Error("wrong shape");
  });
  it("parses a Diff display", () => {
    const f = parseInbound(JSON.stringify({
      v: 1, session_id: "s", kind: "event",
      payload: { type: "tool_result", name: "edit_file", content: "ok",
        display: { Diff: { path: "a.txt", before: "x\n", after: "y\n" } } } }));
    if (f?.kind === "event" && f.payload.type === "tool_result") {
      expect(f.payload.display).toEqual({ Diff: { path: "a.txt", before: "x\n", after: "y\n" } });
    } else throw new Error("wrong shape");
  });
  it("parses approval_request and presence", () => {
    const a = parseInbound(JSON.stringify({ v: 1, session_id: "s", id: "c0", kind: "approval_request", summary: "run x", command: "x" }));
    expect(a).toMatchObject({ kind: "approval_request", id: "c0", summary: "run x", command: "x" });
    const p = parseInbound(JSON.stringify({ v: 1, session_id: "s", kind: "presence", online: true }));
    expect(p).toMatchObject({ kind: "presence", online: true });
  });
  it("returns null on malformed json or unknown kind", () => {
    expect(parseInbound("{not json")).toBeNull();
    expect(parseInbound(JSON.stringify({ v: 1, session_id: "s", kind: "mystery" }))).toBeNull();
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd web && npm test -- wire`
Expected: FAIL (module `../src/wire` not found).

- [ ] **Step 3: Implement `wire.ts`**

`web/src/wire.ts`:

```ts
export const PROTOCOL_VERSION = 1;

export type Display =
  | { Text: string }
  | { Diff: { path: string; before: string; after: string } }
  | { Terminal: { command: string; stdout: string; stderr: string; exit_code: number } };

export type WireEvent =
  | { type: "token"; text: string }
  | { type: "tool_start"; name: string; args: unknown }
  | { type: "tool_result"; name: string; content: string; display?: Display }
  | { type: "error"; message: string }
  | { type: "done"; reason: string };

export type Inbound =
  | { v: number; session_id: string; kind: "event"; payload: WireEvent }
  | { v: number; session_id: string; id: string; kind: "approval_request"; summary: string; command?: string; display?: Display }
  | { v: number; session_id: string; kind: "presence"; online: boolean };

export type Decision = "approve" | "approve_always" | "deny";

export type Outbound =
  | { v: number; session_id: string; kind: "user_input"; text: string }
  | { v: number; session_id: string; id: string; kind: "approval_response"; decision: Decision };

/** Parse a raw WS text frame into an Inbound, or null if malformed/unknown. */
export function parseInbound(raw: string): Inbound | null {
  let v: unknown;
  try {
    v = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!v || typeof v !== "object") return null;
  const o = v as Record<string, unknown>;
  if (o.kind === "event" || o.kind === "approval_request" || o.kind === "presence") {
    return o as unknown as Inbound;
  }
  return null;
}
```

- [ ] **Step 4: Run to verify pass + typecheck**

Run: `cd web && npm test -- wire && npm run typecheck`
Expected: 5 tests PASS; no type errors.

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/wire.ts web/test/wire.test.ts
git commit -m "feat(web): wire envelope types + parseInbound"
```

---

## Task 3: State reducer + localStorage

**Files:**
- Create: `web/src/state.ts`, `web/src/storage.ts`, `web/test/state.test.ts`, `web/test/storage.test.ts`

**Interfaces:**
- Consumes: `Inbound`, `Display`, `Decision` (Task 2).
- Produces: `ConnectionStatus`, `Item`, `ConversationState`, `Action`, `initialState(userMsgs: string[]): ConversationState`, `reduce(state, action): ConversationState`; storage `loadToken/saveSession/clearSession/loadUserMsgs/appendUserMsg`.

- [ ] **Step 1: Write failing reducer tests**

`web/test/state.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { initialState, reduce, type ConversationState } from "../src/state";
import type { Inbound } from "../src/wire";

const ev = (payload: Inbound extends { kind: "event" } ? never : never): never => payload; // unused helper guard
function frame(f: Inbound) { return { type: "frame", frame: f } as const; }
function run(actions: Parameters<typeof reduce>[1][], userMsgs: string[] = []): ConversationState {
  return actions.reduce(reduce, initialState(userMsgs));
}

describe("reducer", () => {
  it("accumulates streamed tokens into one assistant item", () => {
    const s = run([
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "token", text: "Hel" } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "token", text: "lo" } }),
    ]);
    expect(s.items).toEqual([{ kind: "assistant", text: "Hello" }]);
  });

  it("correlates tool_result to the running tool of the same name", () => {
    const s = run([
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "tool_start", name: "execute_command", args: {} } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "tool_result", name: "execute_command", content: "exit=0" } }),
    ]);
    expect(s.items).toEqual([{ kind: "tool", name: "execute_command", args: {}, status: "done", content: "exit=0", display: undefined }]);
  });

  it("sets and clears the pending approval", () => {
    let s = run([frame({ v: 1, session_id: "s", id: "c0", kind: "approval_request", summary: "run x", command: "x" })]);
    expect(s.pendingApproval).toMatchObject({ id: "c0", summary: "run x" });
    s = reduce(s, { type: "approval_sent" });
    expect(s.pendingApproval).toBeNull();
  });

  it("tracks presence and closes a turn on done", () => {
    const s = run([
      frame({ v: 1, session_id: "s", kind: "presence", online: true }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "token", text: "ok" } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "done", reason: "stop" } }),
    ]);
    expect(s.online).toBe(true);
    expect(s.items).toEqual([{ kind: "assistant", text: "ok", done: "stop" }]);
  });

  it("reset-and-replay reconstructs history with user messages interleaved by turn", () => {
    // Two stored user messages -> they head turn 0 and turn 1.
    const s = run([
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "token", text: "A" } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "done", reason: "stop" } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "token", text: "B" } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "done", reason: "stop" } }),
    ], ["q1", "q2"]);
    expect(s.items).toEqual([
      { kind: "user", text: "q1" },
      { kind: "assistant", text: "A", done: "stop" },
      { kind: "user", text: "q2" },
      { kind: "assistant", text: "B", done: "stop" },
    ]);
  });

  it("user_send pushes the user item and is not double-emitted by the following turn", () => {
    const s = run([
      { type: "user_send", text: "hello" },
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "token", text: "hi back" } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "done", reason: "stop" } }),
    ]);
    expect(s.items).toEqual([
      { kind: "user", text: "hello" },
      { kind: "assistant", text: "hi back", done: "stop" },
    ]);
  });
});
```

> The `ev` helper above is unused scaffolding from an earlier draft — delete it; it is not referenced.

- [ ] **Step 2: Run to verify it fails**

Run: `cd web && npm test -- state`
Expected: FAIL (module `../src/state` not found).

- [ ] **Step 3: Implement `state.ts`**

`web/src/state.ts`:

```ts
import type { Display, Inbound } from "./wire";

export type ConnectionStatus = "connecting" | "open" | "closed" | "error";

export type Item =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string; done?: string }
  | { kind: "tool"; name: string; args: unknown; status: "running" | "done"; content?: string; display?: Display }
  | { kind: "error"; message: string };

export interface PendingApproval {
  id: string;
  summary: string;
  command?: string;
  display?: Display;
}

export interface ConversationState {
  items: Item[];
  pendingApproval: PendingApproval | null;
  online: boolean;
  status: ConnectionStatus;
  // replay scaffolding (not rendered):
  userMsgs: string[]; // stored user messages for this session; index = turn
  turnIndex: number; // turns started so far
  inTurn: boolean; // has the current turn's user item been emitted?
}

export type Action =
  | { type: "reset"; userMsgs: string[] }
  | { type: "frame"; frame: Inbound }
  | { type: "user_send"; text: string }
  | { type: "approval_sent" }
  | { type: "status"; status: ConnectionStatus };

export function initialState(userMsgs: string[]): ConversationState {
  return { items: [], pendingApproval: null, online: false, status: "connecting", userMsgs, turnIndex: 0, inTurn: false };
}

/** Emit the stored user message that heads the current turn, if not already emitted. */
function startTurn(s: ConversationState): ConversationState {
  if (s.inTurn) return s;
  const text = s.userMsgs[s.turnIndex];
  const items = text !== undefined ? [...s.items, { kind: "user", text } as Item] : s.items;
  return { ...s, items, inTurn: true };
}

export function reduce(state: ConversationState, action: Action): ConversationState {
  switch (action.type) {
    case "reset":
      return initialState(action.userMsgs);
    case "status":
      return { ...state, status: action.status };
    case "user_send": {
      // Live send: the user item is emitted now, so the upcoming turn must not re-emit it.
      return { ...state, items: [...state.items, { kind: "user", text: action.text }], inTurn: true };
    }
    case "approval_sent":
      return { ...state, pendingApproval: null };
    case "frame":
      return reduceFrame(state, action.frame);
  }
}

function reduceFrame(state: ConversationState, frame: Inbound): ConversationState {
  if (frame.kind === "presence") return { ...state, online: frame.online };
  if (frame.kind === "approval_request") {
    return { ...state, pendingApproval: { id: frame.id, summary: frame.summary, command: frame.command, display: frame.display } };
  }
  // frame.kind === "event"
  const s = startTurn(state);
  const p = frame.payload;
  switch (p.type) {
    case "token": {
      const items = [...s.items];
      const last = items[items.length - 1];
      if (last && last.kind === "assistant" && last.done === undefined) {
        items[items.length - 1] = { ...last, text: last.text + p.text };
      } else {
        items.push({ kind: "assistant", text: p.text });
      }
      return { ...s, items };
    }
    case "tool_start":
      return { ...s, items: [...s.items, { kind: "tool", name: p.name, args: p.args, status: "running" }] };
    case "tool_result": {
      const items = [...s.items];
      for (let i = items.length - 1; i >= 0; i--) {
        const it = items[i];
        if (it.kind === "tool" && it.status === "running" && it.name === p.name) {
          items[i] = { ...it, status: "done", content: p.content, display: p.display };
          break;
        }
      }
      return { ...s, items };
    }
    case "error":
      return { ...s, items: [...s.items, { kind: "error", message: p.message }] };
    case "done": {
      const items = [...s.items];
      const last = items[items.length - 1];
      if (last && last.kind === "assistant" && last.done === undefined) {
        items[items.length - 1] = { ...last, done: p.reason };
      }
      // Close the turn: next event starts a new one and re-emits the next user message.
      return { ...s, items, turnIndex: s.turnIndex + 1, inTurn: false };
    }
  }
}
```

- [ ] **Step 4: Run reducer tests**

Run: `cd web && npm test -- state`
Expected: all PASS. (Remove the unused `ev` helper from the test file if it causes a `noUnusedLocals` error under typecheck.)

- [ ] **Step 5: Write storage tests + implementation**

`web/test/storage.test.ts`:

```ts
import { describe, it, expect, beforeEach } from "vitest";
import { loadToken, saveSession, clearSession, loadUserMsgs, appendUserMsg, loadSessionId } from "../src/storage";

beforeEach(() => localStorage.clear());

describe("storage", () => {
  it("saves and loads a session, and clears it", () => {
    saveSession("sess-1", "tok-abc");
    expect(loadToken()).toBe("tok-abc");
    expect(loadSessionId()).toBe("sess-1");
    clearSession();
    expect(loadToken()).toBeNull();
    expect(loadSessionId()).toBeNull();
  });
  it("appends and loads per-session user messages", () => {
    appendUserMsg("sess-1", "q1");
    appendUserMsg("sess-1", "q2");
    appendUserMsg("sess-2", "other");
    expect(loadUserMsgs("sess-1")).toEqual(["q1", "q2"]);
    expect(loadUserMsgs("sess-2")).toEqual(["other"]);
    expect(loadUserMsgs("nope")).toEqual([]);
  });
});
```

`web/src/storage.ts`:

```ts
const TOKEN = "agent.sessionToken";
const SID = "agent.sessionId";
const MSGS = (sid: string) => `agent.userMsgs.${sid}`;

export function saveSession(sessionId: string, token: string): void {
  localStorage.setItem(SID, sessionId);
  localStorage.setItem(TOKEN, token);
}
export function loadToken(): string | null {
  return localStorage.getItem(TOKEN);
}
export function loadSessionId(): string | null {
  return localStorage.getItem(SID);
}
export function clearSession(): void {
  localStorage.removeItem(TOKEN);
  localStorage.removeItem(SID);
}
export function loadUserMsgs(sessionId: string): string[] {
  const raw = localStorage.getItem(MSGS(sessionId));
  if (!raw) return [];
  try {
    const v = JSON.parse(raw);
    return Array.isArray(v) ? (v as string[]) : [];
  } catch {
    return [];
  }
}
export function appendUserMsg(sessionId: string, text: string): void {
  const arr = loadUserMsgs(sessionId);
  arr.push(text);
  localStorage.setItem(MSGS(sessionId), JSON.stringify(arr));
}
```

- [ ] **Step 6: Run all Task-3 tests + typecheck**

Run: `cd web && npm test -- state storage && npm run typecheck`
Expected: all PASS; no type errors.

- [ ] **Step 7: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/state.ts web/src/storage.ts web/test/state.test.ts web/test/storage.test.ts
git commit -m "feat(web): conversation reducer (reset-and-replay, turn interleaving) + localStorage"
```

---

## Task 4: Typed socket client

**Files:**
- Create: `web/src/socket.ts`, `web/test/socket.test.ts`

**Interfaces:**
- Consumes: `Inbound`, `Outbound`, `parseInbound` (Task 2); `ConnectionStatus` (Task 3).
- Produces: `connect(url: string, handlers: { onFrame: (f: Inbound) => void; onStatus: (s: ConnectionStatus) => void }, opts?: { WebSocketImpl?: typeof WebSocket; backoffMs?: number }): { send(o: Outbound): void; close(): void }`.

- [ ] **Step 1: Write failing tests with a mock WebSocket**

`web/test/socket.test.ts`:

```ts
import { describe, it, expect, vi } from "vitest";
import { connect } from "../src/socket";
import type { Inbound } from "../src/wire";
import type { ConnectionStatus } from "../src/state";

class FakeWS {
  static instances: FakeWS[] = [];
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  readyState = 0;
  sent: string[] = [];
  url: string;
  constructor(url: string) { this.url = url; FakeWS.instances.push(this); }
  send(d: string) { this.sent.push(d); }
  close() { this.readyState = 3; this.onclose?.(); }
  open() { this.readyState = 1; this.onopen?.(); }
  message(o: unknown) { this.onmessage?.({ data: JSON.stringify(o) }); }
}

describe("socket", () => {
  it("reports status open and delivers parsed frames", () => {
    FakeWS.instances = [];
    const frames: Inbound[] = []; const statuses: ConnectionStatus[] = [];
    connect("ws://x/browser?token=t", { onFrame: (f) => frames.push(f), onStatus: (s) => statuses.push(s) },
      { WebSocketImpl: FakeWS as unknown as typeof WebSocket });
    const ws = FakeWS.instances[0];
    ws.open();
    ws.message({ v: 1, session_id: "s", kind: "presence", online: true });
    expect(statuses).toContain("open");
    expect(frames).toEqual([{ v: 1, session_id: "s", kind: "presence", online: true }]);
  });

  it("reconnects on unexpected close but not after a deliberate close()", () => {
    vi.useFakeTimers();
    FakeWS.instances = [];
    const handle = connect("ws://x/browser?token=t", { onFrame: () => {}, onStatus: () => {} },
      { WebSocketImpl: FakeWS as unknown as typeof WebSocket, backoffMs: 10 });
    FakeWS.instances[0].open();
    FakeWS.instances[0].close(); // unexpected
    vi.advanceTimersByTime(10);
    expect(FakeWS.instances.length).toBe(2); // reconnected
    handle.close(); // deliberate
    vi.advanceTimersByTime(1000);
    expect(FakeWS.instances.length).toBe(2); // no further reconnect
    vi.useRealTimers();
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd web && npm test -- socket`
Expected: FAIL (module not found).

- [ ] **Step 3: Implement `socket.ts`**

`web/src/socket.ts`:

```ts
import { parseInbound, type Inbound, type Outbound } from "./wire";
import type { ConnectionStatus } from "./state";

interface Handlers {
  onFrame: (f: Inbound) => void;
  onStatus: (s: ConnectionStatus) => void;
}
interface Opts {
  WebSocketImpl?: typeof WebSocket;
  backoffMs?: number;
}

export function connect(url: string, handlers: Handlers, opts: Opts = {}) {
  const WS = opts.WebSocketImpl ?? WebSocket;
  const baseBackoff = opts.backoffMs ?? 500;
  let ws: WebSocket;
  let closed = false;
  let backoff = baseBackoff;

  const open = () => {
    handlers.onStatus("connecting");
    ws = new WS(url);
    ws.onopen = () => { backoff = baseBackoff; handlers.onStatus("open"); };
    ws.onmessage = (e: MessageEvent) => {
      const f = parseInbound(typeof e.data === "string" ? e.data : "");
      if (f) handlers.onFrame(f);
    };
    ws.onerror = () => handlers.onStatus("error");
    ws.onclose = () => {
      handlers.onStatus("closed");
      if (closed) return;
      setTimeout(open, backoff);
      backoff = Math.min(backoff * 2, 30000);
    };
  };
  open();

  return {
    send(o: Outbound) {
      ws.send(JSON.stringify(o));
    },
    close() {
      closed = true;
      ws.close();
    },
  };
}
```

- [ ] **Step 4: Run to verify pass + typecheck**

Run: `cd web && npm test -- socket && npm run typecheck`
Expected: 2 tests PASS; no type errors.

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/socket.ts web/test/socket.test.ts
git commit -m "feat(web): typed WebSocket client with reconnect/backoff"
```

---

## Task 5: Tool-rendering components (DiffView, TerminalBlock, ToolCall)

**Files:**
- Create: `web/src/components/DiffView.tsx`, `web/src/components/TerminalBlock.tsx`, `web/src/components/ToolCall.tsx`, `web/test/tool-components.test.tsx`

**Interfaces:**
- Consumes: `Item`, `Display` (Task 3 / Task 2).
- Produces: `DiffView({ path, before, after })`, `TerminalBlock({ command, stdout, stderr, exitCode })`, `ToolCall({ item })` where `item` is the `tool` Item variant.

- [ ] **Step 1: Write failing component tests**

`web/test/tool-components.test.tsx`:

```tsx
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { DiffView } from "../src/components/DiffView";
import { TerminalBlock } from "../src/components/TerminalBlock";
import { ToolCall } from "../src/components/ToolCall";

describe("tool components", () => {
  it("DiffView shows added and removed lines", () => {
    render(<DiffView path="a.txt" before={"foo\nbar\n"} after={"foo\nbaz\n"} />);
    expect(screen.getByText("a.txt")).toBeInTheDocument();
    expect(screen.getByText(/-\s*bar/)).toBeInTheDocument();
    expect(screen.getByText(/\+\s*baz/)).toBeInTheDocument();
  });
  it("TerminalBlock shows the command, output, and exit code", () => {
    render(<TerminalBlock command="echo hi" stdout={"hi\n"} stderr="" exitCode={0} />);
    expect(screen.getByText(/echo hi/)).toBeInTheDocument();
    expect(screen.getByText(/hi/)).toBeInTheDocument();
    expect(screen.getByText(/exit 0/)).toBeInTheDocument();
  });
  it("ToolCall renders a Terminal display for execute_command", () => {
    render(<ToolCall item={{ kind: "tool", name: "execute_command", args: {}, status: "done", content: "exit=0",
      display: { Terminal: { command: "ls", stdout: "file\n", stderr: "", exit_code: 0 } } }} />);
    expect(screen.getByText(/execute_command/)).toBeInTheDocument();
    expect(screen.getByText(/ls/)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd web && npm test -- tool-components`
Expected: FAIL (modules not found).

- [ ] **Step 3: Implement the three components**

`web/src/components/DiffView.tsx`:

```tsx
import { diffLines } from "diff";

export function DiffView({ path, before, after }: { path: string; before: string; after: string }) {
  const parts = diffLines(before, after);
  return (
    <div className="rounded border border-zinc-700 bg-zinc-900 text-sm">
      <div className="border-b border-zinc-700 px-2 py-1 font-mono text-amber-400">{path}</div>
      <pre className="overflow-x-auto p-2 font-mono leading-tight">
        {parts.flatMap((part, pi) => {
          const sign = part.added ? "+" : part.removed ? "-" : " ";
          const cls = part.added ? "text-green-400" : part.removed ? "text-red-400" : "text-zinc-400";
          return part.value.replace(/\n$/, "").split("\n").map((line, li) => (
            <div key={`${pi}-${li}`} className={cls}>{sign} {line}</div>
          ));
        })}
      </pre>
    </div>
  );
}
```

`web/src/components/TerminalBlock.tsx`:

```tsx
export function TerminalBlock({ command, stdout, stderr, exitCode }: { command: string; stdout: string; stderr: string; exitCode: number }) {
  return (
    <div className="rounded border border-zinc-700 bg-black text-sm">
      <div className="flex items-center justify-between border-b border-zinc-700 px-2 py-1">
        <span className="font-mono text-zinc-300">$ {command}</span>
        <span className={exitCode === 0 ? "text-green-400" : "text-red-400"}>exit {exitCode}</span>
      </div>
      <pre className="overflow-x-auto p-2 font-mono leading-tight text-zinc-200">{stdout}{stderr}</pre>
    </div>
  );
}
```

`web/src/components/ToolCall.tsx`:

```tsx
import type { Item } from "../state";
import { DiffView } from "./DiffView";
import { TerminalBlock } from "./TerminalBlock";

type ToolItem = Extract<Item, { kind: "tool" }>;

export function ToolCall({ item }: { item: ToolItem }) {
  const statusIcon = item.status === "running" ? "…" : "✓";
  const d = item.display;
  return (
    <div className="my-2">
      <div className="font-mono text-cyan-400">⚙ {item.name} <span className="text-zinc-500">{statusIcon}</span></div>
      {d && "Diff" in d && <DiffView path={d.Diff.path} before={d.Diff.before} after={d.Diff.after} />}
      {d && "Terminal" in d && (
        <TerminalBlock command={d.Terminal.command} stdout={d.Terminal.stdout} stderr={d.Terminal.stderr} exitCode={d.Terminal.exit_code} />
      )}
      {d && "Text" in d && <pre className="whitespace-pre-wrap p-2 font-mono text-sm text-zinc-300">{d.Text}</pre>}
      {!d && item.content && <pre className="whitespace-pre-wrap p-2 font-mono text-sm text-zinc-400">{item.content}</pre>}
    </div>
  );
}
```

- [ ] **Step 4: Run to verify pass + typecheck**

Run: `cd web && npm test -- tool-components && npm run typecheck`
Expected: 3 tests PASS; no type errors.

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/components/DiffView.tsx web/src/components/TerminalBlock.tsx web/src/components/ToolCall.tsx web/test/tool-components.test.tsx
git commit -m "feat(web): DiffView, TerminalBlock, ToolCall components"
```

---

## Task 6: Chat-shell components (StatusBar, AssistantMessage, MessageList, ApprovalPrompt, Composer)

**Files:**
- Create: `web/src/components/StatusBar.tsx`, `web/src/components/AssistantMessage.tsx`, `web/src/components/MessageList.tsx`, `web/src/components/ApprovalPrompt.tsx`, `web/src/components/Composer.tsx`, `web/test/shell-components.test.tsx`

**Interfaces:**
- Consumes: `Item`, `ConnectionStatus`, `PendingApproval` (Task 3); `Decision` (Task 2); `ToolCall` (Task 5).
- Produces: `StatusBar({ online, status, onSignOut })`, `AssistantMessage({ item })`, `MessageList({ items })`, `ApprovalPrompt({ approval, onDecide })`, `Composer({ disabled, onSend })`.

- [ ] **Step 1: Write failing component tests**

`web/test/shell-components.test.tsx`:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StatusBar } from "../src/components/StatusBar";
import { MessageList } from "../src/components/MessageList";
import { ApprovalPrompt } from "../src/components/ApprovalPrompt";
import { Composer } from "../src/components/Composer";

describe("shell components", () => {
  it("StatusBar shows presence and triggers sign-out", async () => {
    const onSignOut = vi.fn();
    render(<StatusBar online={true} status="open" onSignOut={onSignOut} />);
    expect(screen.getByText(/online/i)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /sign out/i }));
    expect(onSignOut).toHaveBeenCalled();
  });

  it("MessageList renders items in order by type", () => {
    render(<MessageList items={[
      { kind: "user", text: "hi" },
      { kind: "assistant", text: "hello", done: "stop" },
      { kind: "error", message: "boom" },
    ]} />);
    expect(screen.getByText("hi")).toBeInTheDocument();
    expect(screen.getByText("hello")).toBeInTheDocument();
    expect(screen.getByText(/boom/)).toBeInTheDocument();
  });

  it("ApprovalPrompt emits the chosen decision", async () => {
    const onDecide = vi.fn();
    render(<ApprovalPrompt approval={{ id: "c0", summary: "run x", command: "x" }} onDecide={onDecide} />);
    await userEvent.click(screen.getByRole("button", { name: /^approve$/i }));
    expect(onDecide).toHaveBeenCalledWith("approve");
  });

  it("Composer sends text and is disabled when offline", async () => {
    const onSend = vi.fn();
    const { rerender } = render(<Composer disabled={false} onSend={onSend} />);
    await userEvent.type(screen.getByRole("textbox"), "do it");
    await userEvent.click(screen.getByRole("button", { name: /send/i }));
    expect(onSend).toHaveBeenCalledWith("do it");
    rerender(<Composer disabled={true} onSend={onSend} />);
    expect(screen.getByRole("button", { name: /send/i })).toBeDisabled();
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd web && npm test -- shell-components`
Expected: FAIL (modules not found).

- [ ] **Step 3: Implement the five components**

`web/src/components/StatusBar.tsx`:

```tsx
import type { ConnectionStatus } from "../state";

export function StatusBar({ online, status, onSignOut }: { online: boolean; status: ConnectionStatus; onSignOut: () => void }) {
  return (
    <div className="flex items-center justify-between border-b border-zinc-800 bg-zinc-950 px-4 py-2 text-sm">
      <div className="flex items-center gap-2">
        <span className={`h-2 w-2 rounded-full ${online ? "bg-green-400" : "bg-zinc-600"}`} />
        <span className="text-zinc-300">{online ? "agent online" : "agent offline"}</span>
        <span className="text-zinc-600">· {status}</span>
      </div>
      <button onClick={onSignOut} className="text-zinc-400 hover:text-zinc-200">sign out</button>
    </div>
  );
}
```

`web/src/components/AssistantMessage.tsx`:

```tsx
import type { Item } from "../state";

export function AssistantMessage({ item }: { item: Extract<Item, { kind: "assistant" }> }) {
  return <div className="whitespace-pre-wrap py-2 text-zinc-100">{item.text}</div>;
}
```

`web/src/components/MessageList.tsx`:

```tsx
import type { Item } from "../state";
import { AssistantMessage } from "./AssistantMessage";
import { ToolCall } from "./ToolCall";

export function MessageList({ items }: { items: Item[] }) {
  return (
    <div className="flex-1 overflow-y-auto px-4">
      {items.map((it, i) => {
        switch (it.kind) {
          case "user":
            return <div key={i} className="my-2 ml-auto max-w-[80%] rounded bg-zinc-800 px-3 py-2 text-zinc-100">{it.text}</div>;
          case "assistant":
            return <AssistantMessage key={i} item={it} />;
          case "tool":
            return <ToolCall key={i} item={it} />;
          case "error":
            return <div key={i} className="my-2 rounded border border-red-700 bg-red-950 px-3 py-2 text-red-300">✗ {it.message}</div>;
        }
      })}
    </div>
  );
}
```

`web/src/components/ApprovalPrompt.tsx`:

```tsx
import type { Decision } from "../wire";
import type { PendingApproval } from "../state";

export function ApprovalPrompt({ approval, onDecide }: { approval: PendingApproval; onDecide: (d: Decision) => void }) {
  return (
    <div className="mx-4 my-2 rounded border border-amber-700 bg-amber-950 p-3 text-sm">
      <div className="mb-2 text-amber-200">Allow: {approval.summary}</div>
      {approval.command && <pre className="mb-2 overflow-x-auto font-mono text-amber-300">{approval.command}</pre>}
      <div className="flex gap-2">
        <button onClick={() => onDecide("approve")} className="rounded bg-green-700 px-3 py-1 text-white hover:bg-green-600">Approve</button>
        <button onClick={() => onDecide("approve_always")} className="rounded bg-green-900 px-3 py-1 text-green-200 hover:bg-green-800">Approve always</button>
        <button onClick={() => onDecide("deny")} className="rounded bg-red-800 px-3 py-1 text-white hover:bg-red-700">Deny</button>
      </div>
    </div>
  );
}
```

`web/src/components/Composer.tsx`:

```tsx
import { useState } from "react";

export function Composer({ disabled, onSend }: { disabled: boolean; onSend: (text: string) => void }) {
  const [text, setText] = useState("");
  const submit = () => {
    const t = text.trim();
    if (!t || disabled) return;
    onSend(t);
    setText("");
  };
  return (
    <div className="flex gap-2 border-t border-zinc-800 bg-zinc-950 p-3">
      <textarea
        className="flex-1 resize-none rounded bg-zinc-900 p-2 text-zinc-100 outline-none disabled:opacity-50"
        rows={2}
        value={text}
        disabled={disabled}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); submit(); } }}
        placeholder={disabled ? "disconnected…" : "Message the agent…"}
      />
      <button onClick={submit} disabled={disabled} className="rounded bg-zinc-700 px-4 text-zinc-100 hover:bg-zinc-600 disabled:opacity-50">Send</button>
    </div>
  );
}
```

- [ ] **Step 4: Run to verify pass + typecheck**

Run: `cd web && npm test -- shell-components && npm run typecheck`
Expected: 4 tests PASS; no type errors.

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/components/StatusBar.tsx web/src/components/AssistantMessage.tsx web/src/components/MessageList.tsx web/src/components/ApprovalPrompt.tsx web/src/components/Composer.tsx web/test/shell-components.test.tsx
git commit -m "feat(web): chat-shell components (StatusBar, MessageList, ApprovalPrompt, Composer)"
```

---

## Task 7: PairingScreen + App integration

Wire pairing, the socket, the reducer, storage, and the components into the working app.

**Files:**
- Create: `web/src/components/PairingScreen.tsx`, `web/test/pairing.test.tsx`, `web/test/app.test.tsx`
- Modify: `web/src/App.tsx` (replace the stub)

**Interfaces:**
- Consumes: everything from Tasks 2–6.
- Produces: `PairingScreen({ onPaired })` (calls `POST /pair`, then `onPaired({ sessionId, token })`); the real `App` that routes on a stored token and drives the conversation.

- [ ] **Step 1: Write the PairingScreen test (mocked fetch)**

`web/test/pairing.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PairingScreen } from "../src/components/PairingScreen";

beforeEach(() => localStorage.clear());

describe("PairingScreen", () => {
  it("pairs with a code and reports the session", async () => {
    const onPaired = vi.fn();
    vi.stubGlobal("fetch", vi.fn(async () => ({
      ok: true,
      json: async () => ({ session_id: "sess-1", session_token: "tok-1", agent_id: "a1" }),
    })) as unknown as typeof fetch);

    render(<PairingScreen onPaired={onPaired} />);
    await userEvent.type(screen.getByRole("textbox"), "123456");
    await userEvent.click(screen.getByRole("button", { name: /pair/i }));

    expect(onPaired).toHaveBeenCalledWith({ sessionId: "sess-1", token: "tok-1" });
    vi.unstubAllGlobals();
  });

  it("shows an error on a bad code", async () => {
    const onPaired = vi.fn();
    vi.stubGlobal("fetch", vi.fn(async () => ({ ok: false, status: 404, json: async () => ({ error: "invalid pairing code" }) })) as unknown as typeof fetch);
    render(<PairingScreen onPaired={onPaired} />);
    await userEvent.type(screen.getByRole("textbox"), "000000");
    await userEvent.click(screen.getByRole("button", { name: /pair/i }));
    expect(await screen.findByText(/invalid/i)).toBeInTheDocument();
    expect(onPaired).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});
```

- [ ] **Step 2: Implement PairingScreen**

`web/src/components/PairingScreen.tsx`:

```tsx
import { useState } from "react";

export function PairingScreen({ onPaired }: { onPaired: (s: { sessionId: string; token: string }) => void }) {
  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const pair = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await fetch("/pair", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ pairing_code: code.trim() }),
      });
      if (!r.ok) {
        const body = (await r.json().catch(() => ({}))) as { error?: string };
        setError(body.error ?? `pairing failed (${r.status})`);
        return;
      }
      const body = (await r.json()) as { session_id: string; session_token: string };
      onPaired({ sessionId: body.session_id, token: body.session_token });
    } catch {
      setError("could not reach the control plane");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 text-zinc-100">
      <h1 className="text-lg">Pair with your agent</h1>
      <input
        className="rounded bg-zinc-900 px-3 py-2 text-center font-mono tracking-widest outline-none"
        value={code}
        onChange={(e) => setCode(e.target.value)}
        placeholder="pairing code"
        onKeyDown={(e) => { if (e.key === "Enter") pair(); }}
      />
      <button onClick={pair} disabled={busy || !code.trim()} className="rounded bg-zinc-700 px-4 py-2 hover:bg-zinc-600 disabled:opacity-50">
        {busy ? "Pairing…" : "Pair"}
      </button>
      {error && <div className="text-red-400">{error}</div>}
    </div>
  );
}
```

- [ ] **Step 3: Write the App integration test (mock socket via injected WebSocket)**

`web/test/app.test.tsx`:

```tsx
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, act } from "@testing-library/react";
import App from "../src/App";
import { saveSession } from "../src/storage";

// A controllable WebSocket the App will use (App reads a window-injected impl in tests).
class TestWS {
  static last: TestWS | null = null;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  readyState = 1;
  sent: string[] = [];
  constructor(public url: string) { TestWS.last = this; }
  send(d: string) { this.sent.push(d); }
  close() { this.readyState = 3; this.onclose?.(); }
}

beforeEach(() => {
  localStorage.clear();
  (window as unknown as { __WS__?: unknown }).__WS__ = TestWS;
});

describe("App", () => {
  it("with a stored token, connects and renders streamed frames", () => {
    saveSession("sess-1", "tok-1");
    render(<App />);
    act(() => { TestWS.last!.onopen?.(); });
    act(() => {
      TestWS.last!.onmessage?.({ data: JSON.stringify({ v: 1, session_id: "sess-1", kind: "presence", online: true }) });
      TestWS.last!.onmessage?.({ data: JSON.stringify({ v: 1, session_id: "sess-1", kind: "event", payload: { type: "token", text: "hello world" } }) });
    });
    expect(screen.getByText(/agent online/i)).toBeInTheDocument();
    expect(screen.getByText("hello world")).toBeInTheDocument();
  });

  it("without a token, shows the pairing screen", () => {
    render(<App />);
    expect(screen.getByRole("button", { name: /pair/i })).toBeInTheDocument();
  });
});
```

- [ ] **Step 4: Implement the real App**

`web/src/App.tsx` (replace the stub). The socket impl is read from `window.__WS__` when present so tests can inject a fake; production uses the global `WebSocket`.

```tsx
import { useEffect, useReducer, useRef, useState } from "react";
import { connect } from "./socket";
import { initialState, reduce } from "./state";
import type { Decision } from "./wire";
import { PairingScreen } from "./components/PairingScreen";
import { StatusBar } from "./components/StatusBar";
import { MessageList } from "./components/MessageList";
import { ApprovalPrompt } from "./components/ApprovalPrompt";
import { Composer } from "./components/Composer";
import { appendUserMsg, clearSession, loadSessionId, loadToken, loadUserMsgs, saveSession } from "./storage";

function wsUrl(token: string): string {
  return `${location.origin.replace(/^http/, "ws")}/browser?token=${encodeURIComponent(token)}`;
}

export default function App() {
  const [sessionId, setSessionId] = useState<string | null>(loadSessionId());
  const [token, setToken] = useState<string | null>(loadToken());
  const [state, dispatch] = useReducer(reduce, loadUserMsgs(sessionId ?? ""), initialState);
  const sock = useRef<ReturnType<typeof connect> | null>(null);

  useEffect(() => {
    if (!token || !sessionId) return;
    dispatch({ type: "reset", userMsgs: loadUserMsgs(sessionId) });
    const WebSocketImpl = (window as unknown as { __WS__?: typeof WebSocket }).__WS__;
    sock.current = connect(
      wsUrl(token),
      { onFrame: (f) => dispatch({ type: "frame", frame: f }), onStatus: (s) => dispatch({ type: "status", status: s }) },
      WebSocketImpl ? { WebSocketImpl } : undefined,
    );
    return () => { sock.current?.close(); sock.current = null; };
  }, [token, sessionId]);

  if (!token || !sessionId) {
    return (
      <div className="h-screen bg-zinc-950">
        <PairingScreen onPaired={({ sessionId, token }) => { saveSession(sessionId, token); setSessionId(sessionId); setToken(token); }} />
      </div>
    );
  }

  const send = (text: string) => {
    appendUserMsg(sessionId, text);
    dispatch({ type: "user_send", text });
    sock.current?.send({ v: 1, session_id: sessionId, kind: "user_input", text });
  };
  const decide = (d: Decision) => {
    if (!state.pendingApproval) return;
    sock.current?.send({ v: 1, session_id: sessionId, id: state.pendingApproval.id, kind: "approval_response", decision: d });
    dispatch({ type: "approval_sent" });
  };
  const signOut = () => { sock.current?.close(); clearSession(); setToken(null); setSessionId(null); };

  const connected = state.status === "open";
  return (
    <div className="flex h-screen flex-col bg-zinc-950">
      <StatusBar online={state.online} status={state.status} onSignOut={signOut} />
      <MessageList items={state.items} />
      {state.pendingApproval && <ApprovalPrompt approval={state.pendingApproval} onDecide={decide} />}
      <Composer disabled={!connected} onSend={send} />
    </div>
  );
}
```

> Note the `useReducer(reduce, loadUserMsgs(...), initialState)` three-arg form: the third arg is the init function, so initial state is `initialState(loadUserMsgs(...))`.

- [ ] **Step 5: Run all tests, typecheck, and a production build**

Run:

```bash
cd /home/kalen/rust-agent-runtime/web && npm test && npm run typecheck && npm run build
```
Expected: all tests across all files PASS; no type errors; `vite build` succeeds and writes `web/dist/`.

- [ ] **Step 6: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/components/PairingScreen.tsx web/src/App.tsx web/test/pairing.test.tsx web/test/app.test.tsx
git commit -m "feat(web): PairingScreen + App integration (socket + reducer + components)"
```

---

## Task 8: Docs + full-suite verification

**Files:**
- Modify: `cloud/RUNNING.md`

- [ ] **Step 1: Document the UI dev/build flow**

Append a section to `cloud/RUNNING.md`:

```markdown
## 4. The web UI (subsystem #6)

Dev (HMR, two processes):
- terminal A: `cd cloud && npx wrangler dev`            # API + WS on :8787
- terminal B: `cd web && npm run dev`                   # UI on :5173, proxies /pair,/agent,/browser to :8787
- browse http://localhost:5173 — same-origin via the Vite proxy (no CORS). Enter the daemon's pairing code.

Production-like (single origin, served by the Worker):
- `cd web && npm run build`                             # writes web/dist
- `cd cloud && npx wrangler dev`                        # serves the SPA + API on :8787 (run_worker_first routes the API)
- browse http://localhost:8787

Deploy ships both together: `cd web && npm run build && cd ../cloud && npx wrangler deploy`.
```

- [ ] **Step 2: Run the full automated suites**

Run:

```bash
cd /home/kalen/rust-agent-runtime/web && npm test && npm run build
cd /home/kalen/rust-agent-runtime/cloud && npm test
```
Expected: all `web/` tests pass + build succeeds; all 9 `cloud/` tests still pass (assets dir present).

- [ ] **Step 3: Manual E2E (deferred to the human)**

Follow the new RUNNING.md §4 against the live stack (daemon + model up): pair, send a prompt, watch streamed tokens, approve a command tool (diff/terminal render), reload the page → reconnect-replay rebuilds both sides, stop the daemon → presence goes offline. (Drive via chrome-devtools same-origin if automating.)

- [ ] **Step 4: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add cloud/RUNNING.md
git commit -m "docs(cloud): document the web UI dev/build/deploy flow"
```

---

## Self-Review (completed during planning)

**Spec coverage:**
- §2 hosting / same-origin assets + §4 Worker config → Task 1 (Steps 5) + Task 8 (build/serve flow).
- §3 repo structure / build tooling (Vite/React/TS/Tailwind/Vitest, dev proxy) → Task 1.
- §5 wire types (incl. externally-tagged `Display`) → Task 2.
- §6 socket client (reconnect/backoff, deliberate-close) → Task 4.
- §7 state model (reducer, reset-and-replay, optimistic + turn-interleaved user messages) → Task 3.
- §8 components (DiffView, TerminalBlock, ToolCall; StatusBar, AssistantMessage, MessageList, ApprovalPrompt, Composer; PairingScreen) → Tasks 5, 6, 7.
- §9 testing (reducer, wire parse, components, socket, manual E2E) → Tasks 2–8.
- §10 DoD (paired session driving streamed work, both suites green, daemon/protocol untouched) → Tasks 7, 8.

**Placeholder scan:** the only environment-resolved values are npm peer versions (Task 1 Step 1) — handled with concrete `^` floors plus a resolution procedure. The `ev` helper in Task 3's test draft is explicitly flagged for deletion. No TODO/TBD or "add error handling" placeholders; every code step shows complete code.

**Type consistency:** `Inbound`/`Outbound`/`Display`/`WireEvent`/`Decision` (Task 2) are consumed unchanged in Tasks 3, 4, 6, 7. `Item`/`ConversationState`/`ConnectionStatus`/`PendingApproval`/`Action`/`initialState`/`reduce` (Task 3) match their uses in Tasks 5–7 (`ToolCall` takes `Extract<Item,{kind:"tool"}>`; `App` uses `initialState` as the `useReducer` init fn and dispatches `reset`/`frame`/`user_send`/`approval_sent`/`status`). `connect(...)`'s signature (Task 4) matches the `App` call (Task 7), including the `WebSocketImpl` opt used by both the socket test and the app test. Storage fn names (`saveSession`/`loadToken`/`loadSessionId`/`clearSession`/`loadUserMsgs`/`appendUserMsg`) match between Task 3 and Task 7.
