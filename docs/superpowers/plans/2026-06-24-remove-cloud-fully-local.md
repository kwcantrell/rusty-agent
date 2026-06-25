# Remove Cloud Control Plane (Fully-Local Desktop) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete the Cloudflare control plane and every cloud-only code path so the app has exactly one execution model — the Tauri desktop app with its embedded local WebSocket bridge.

**Architecture:** The desktop app already runs fully locally: `src-tauri/src/bridge.rs` embeds the runtime via `agent_server::daemon::serve` and the webview connects to `ws://127.0.0.1:<port>/agent`. This plan removes the parallel, now-dead cloud path (Worker + `agent-serverd` outbound client + web pairing screen) and the vestigial Vite proxy that logs `ECONNREFUSED 127.0.0.1:8787`.

**Tech Stack:** Rust (Cargo workspace + standalone `src-tauri`), React 19 / Vite 7 / TypeScript / Vitest, Cloudflare Workers (being deleted).

**Spec:** `docs/superpowers/specs/2026-06-24-remove-cloud-fully-local-design.md`

## Global Constraints

- **cargo and node are on PATH** — do NOT `source ~/.cargo/env`.
- **The build is the gate.** Every Rust task ends with `cargo build` + `cargo test` green; every web task ends with `npm run typecheck` + `npm test` green.
- **Keep the desktop path (`daemon::serve`, `setup::local_params`, `bridge.rs`) intact.** Only the outbound/cloud surface is removed.
- **Non-Tauri behavior:** `resolveTransport()` returns `{ wsUrl: "" }` when not under Tauri; `App` renders a "desktop app" notice instead of connecting.
- **llama-server on :8080** is required only for the ignored live e2e (`e2e_live.rs`), not for the default test suites.
- Leave `storage.ts`, `storage.test.ts`, `socket.ts`, `socket.test.ts` untouched (their token/`/browser` strings are generic localStorage/URL fixtures, not cloud infrastructure).

---

### Task 1: Delete the Cloudflare control plane and its launcher

**Files:**
- Delete: `cloud/` (entire directory)
- Delete: `scripts/launch-web-ui.sh` (then remove `scripts/` if empty)
- Delete: `agent-server.json` (repo root — stale cloud enrollment credentials)

**Interfaces:**
- Consumes: nothing.
- Produces: nothing. These deletions are independent of the Rust/web build graphs (the `cloud/` Worker is its own npm project; `launch-web-ui.sh` and `agent-server.json` are only read at runtime by the cloud daemon, which Task 2 removes).

- [ ] **Step 1: Confirm the build is green before any change (baseline)**

Run: `cd agent && cargo build && cd ../src-tauri && cargo build && cd ../web && npm run build`
Expected: all three succeed.

- [ ] **Step 2: Delete cloud directory, launcher, and stale config**

```bash
cd /home/kalen/rust-agent-runtime
git rm -r cloud
git rm scripts/launch-web-ui.sh
git rm agent-server.json
rmdir scripts 2>/dev/null || true   # remove the dir only if now empty
```

- [ ] **Step 3: Verify nothing in the build graph referenced them**

Run: `grep -rniE "launch-web-ui|agent-server\.json" --include=*.rs --include=*.ts --include=*.tsx --include=*.toml --include=*.json . | grep -vE "/docs/|node_modules"`
Expected: no output (docs may still mention them historically — that's fine).

- [ ] **Step 4: Re-verify builds still pass (cloud deletion touches nothing in agent/web/src-tauri)**

Run: `cd agent && cargo build && cd ../web && npm run build`
Expected: both succeed.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "chore: remove Cloudflare control plane, launcher, and stale enrollment config"
```

---

### Task 2: Strip the cloud client from `agent-server` (Rust)

**Files:**
- Delete: `agent/crates/agent-server/src/config.rs`
- Delete: `agent/crates/agent-server/src/main.rs`
- Delete: `agent/crates/agent-server/tests/daemon_roundtrip.rs`
- Modify: `agent/crates/agent-server/src/lib.rs` (remove `pub mod config;`)
- Modify: `agent/crates/agent-server/src/daemon.rs` (remove `run()`, the `IntoClientRequest` import, and the `ws_url`/`agent_token` fields)
- Modify: `agent/crates/agent-server/src/setup.rs` (drop the two empty-string initializers)
- Modify: `agent/crates/agent-server/src/wire.rs` (remove the `Presence` variant)
- Modify: `agent/crates/agent-server/Cargo.toml` (remove the `[[bin]]` and `reqwest`)
- Keep: `tests/serve_inbound.rs`, `tests/bridge.rs` (desktop path)

**Interfaces:**
- Consumes: nothing new.
- Produces: a slimmed `pub struct DaemonParams` (no `ws_url`, no `agent_token`) with fields `config, api_key, claude_binary, config_path, workspace, system_prompt, mcp_tools, memory_tools`; `pub async fn serve<S>(ws, params)` unchanged in signature; `agent-server` is now **library-only** (no `agent-serverd` binary). `setup::local_params(workspace, config_path, base_url, model) -> DaemonParams` unchanged in signature.

- [ ] **Step 1: Delete the cloud-only files**

```bash
cd /home/kalen/rust-agent-runtime/agent/crates/agent-server
git rm src/config.rs src/main.rs tests/daemon_roundtrip.rs
```

- [ ] **Step 2: Remove the `config` module from `lib.rs`**

In `src/lib.rs`, delete the line:

```rust
pub mod config;
```

Resulting file:

```rust
pub mod approval;
pub mod daemon;
pub mod runtime;
pub mod setup;
pub mod sink;
pub mod wire;
```

- [ ] **Step 3: Remove `run()`, its import, and the cloud fields in `daemon.rs`**

In `src/daemon.rs`, delete this import line:

```rust
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
```

Change the `DaemonParams` struct from:

```rust
pub struct DaemonParams {
    pub ws_url: String, // ws://host/agent
    pub agent_token: String,
    pub config: RuntimeConfig, // flag-derived base; the file at config_path overlays it
    pub api_key: Option<String>,
    pub claude_binary: String,
    pub config_path: PathBuf,
    pub workspace: PathBuf,
    pub system_prompt: String,
    pub mcp_tools: Arc<[Arc<dyn Tool>]>,
    pub memory_tools: Arc<[Arc<dyn Tool>]>,
}
```

to:

```rust
pub struct DaemonParams {
    pub config: RuntimeConfig, // flag-derived base; the file at config_path overlays it
    pub api_key: Option<String>,
    pub claude_binary: String,
    pub config_path: PathBuf,
    pub workspace: PathBuf,
    pub system_prompt: String,
    pub mcp_tools: Arc<[Arc<dyn Tool>]>,
    pub memory_tools: Arc<[Arc<dyn Tool>]>,
}
```

Delete the entire `run()` function:

```rust
pub async fn run(params: DaemonParams) -> Result<(), DynErr> {
    // Cloud path: dial the Worker, then hand the socket to the transport-agnostic
    // serve() below.
    let mut req = params.ws_url.clone().into_client_request()?;
    req.headers_mut().insert("Authorization",
        format!("Bearer {}", params.agent_token).parse()?);
    let (ws, _resp) = tokio_tungstenite::connect_async(req).await?;
    serve(ws, params).await
}
```

Update the `serve` doc comment from:

```rust
/// Drive the runtime over an already-established WebSocket. Transport-agnostic:
/// the cloud path (`run`) dials the Worker; the desktop bridge accepts a local
/// connection. Everything from here down is the original `run()` body.
```

to:

```rust
/// Drive the runtime over an already-established WebSocket. The desktop bridge
/// (`src-tauri/src/bridge.rs`) accepts a local connection and hands the socket here.
```

- [ ] **Step 4: Drop the dead initializers in `setup.rs`**

In `src/setup.rs`, change the `DaemonParams { ... }` literal from:

```rust
    DaemonParams {
        ws_url: String::new(),
        agent_token: String::new(),
        config,
```

to:

```rust
    DaemonParams {
        config,
```

- [ ] **Step 5: Remove the `Presence` variant in `wire.rs`**

In `src/wire.rs`, delete this line from the `WireBody` enum:

```rust
    Presence { online: bool },
```

- [ ] **Step 6: Remove the binary and `reqwest` from `Cargo.toml`**

In `Cargo.toml`, delete the `[[bin]]` block:

```toml
[[bin]]
name = "agent-serverd"
path = "src/main.rs"
```

and delete the dependency line:

```toml
reqwest.workspace = true
```

- [ ] **Step 7: Build the workspace**

Run: `cd /home/kalen/rust-agent-runtime/agent && cargo build`
Expected: PASS (no `agent-serverd` target, no unused-import/dead-field warnings for the removed items).

- [ ] **Step 8: Confirm no dangling references remain**

Run: `grep -rnE "daemon::run|agent_server::config|ws_url:|agent_token|WireBody::Presence|agent-serverd" agent/crates --include=*.rs`
Expected: no output.

- [ ] **Step 9: Run the agent-server tests (desktop path still green)**

Run: `cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-server`
Expected: PASS, including `bridge`-style `serve_inbound` and `wire` round-trip tests; `daemon_roundtrip` no longer exists.

- [ ] **Step 10: Build the Tauri crate against the slimmed `DaemonParams`**

Run: `cd /home/kalen/rust-agent-runtime/src-tauri && cargo build && cargo test`
Expected: PASS (`bridge.rs` / `llama_health.rs` tests green; `e2e_live` is `#[ignore]`).

- [ ] **Step 11: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add -A
git commit -m "refactor(agent-server): remove cloud client (enroll/run/Presence); library-only crate"
```

---

### Task 3: Remove the pairing/browser path from the web app

**Files:**
- Delete: `web/src/components/PairingScreen.tsx`
- Modify: `web/src/transport.ts` (drop `needsPairing`)
- Modify: `web/src/App.tsx` (remove pairing route + token machinery; add non-Tauri notice)
- Modify: `web/vite.config.ts` (delete the `server.proxy` block)

**Interfaces:**
- Consumes: `isTauri()`, `resolveTransport()` from `transport.ts`.
- Produces: `Transport = { wsUrl: string; sessionId: string }`; `App` renders the non-Tauri notice containing the text "desktop app" when `isTauri()` is false.

- [ ] **Step 1: Delete the PairingScreen component**

```bash
cd /home/kalen/rust-agent-runtime/web
git rm src/components/PairingScreen.tsx
```

- [ ] **Step 2: Simplify `transport.ts`**

Replace the entire contents of `web/src/transport.ts` with:

```ts
import { invoke } from "@tauri-apps/api/core";

export interface Transport {
  wsUrl: string;
  sessionId: string;
}

// Tauri v2 with withGlobalTauri=false still injects __TAURI_INTERNALS__.
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

const SESSION_KEY = "local_session_id";

function localSessionId(): string {
  let id = localStorage.getItem(SESSION_KEY);
  if (!id) {
    id = crypto.randomUUID();
    localStorage.setItem(SESSION_KEY, id);
  }
  return id;
}

export async function resolveTransport(): Promise<Transport> {
  if (isTauri()) {
    const wsUrl = await invoke<string>("get_local_ws_url");
    return { wsUrl, sessionId: localSessionId() };
  }
  return { wsUrl: "", sessionId: "" };
}
```

- [ ] **Step 3: Rewrite `App.tsx` for pure-desktop**

Replace the entire contents of `web/src/App.tsx` with:

```tsx
import { useEffect, useReducer, useRef, useState } from "react";
import { connect } from "./socket";
import { resolveTransport, isTauri } from "./transport";
import { initialState, reduce, useAnimatedItems, artifactsFrom } from "./state";
import type { Decision, RuntimeSettings } from "./wire";
import { SettingsPanel } from "./components/SettingsPanel";
import { TopBar } from "./components/TopBar";
import { AgentColumn } from "./components/AgentColumn";
import { WorkspacePane } from "./components/workspace/WorkspacePane";
import { resolveInitialTheme, applyTheme, type Theme } from "./theme";
import { appendUserMsg, loadSessionId, loadTheme, loadUserMsgs, saveTheme } from "./storage";

export default function App() {
  const [sessionId, setSessionId] = useState<string | null>(loadSessionId());
  const [state, dispatch] = useReducer(reduce, loadUserMsgs(sessionId ?? ""), initialState);
  const [showSettings, setShowSettings] = useState(false);
  const [theme, setTheme] = useState<Theme>(() =>
    resolveInitialTheme(loadTheme(), window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false));
  const [activeArtifactKey, setActiveArtifactKey] = useState<string | null>(null);
  const [workspaceOpen, setWorkspaceOpen] = useState(false);
  const sock = useRef<ReturnType<typeof connect> | null>(null);
  const [localUrl, setLocalUrl] = useState<string | null>(null);
  const [workspace, setWorkspace] = useState<string | undefined>(undefined);
  const [llama, setLlama] = useState<{ ok: boolean; model?: string } | null>(null);
  const tauri = isTauri();

  useEffect(() => {
    if (!tauri) return;
    let active = true;
    const poll = async () => {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        const h = await invoke<{ ok: boolean; model?: string } | null>("llama_health");
        if (active) setLlama(h ?? { ok: false });
      } catch { if (active) setLlama({ ok: false }); }
    };
    poll();
    const id = setInterval(poll, 10000);
    return () => { active = false; clearInterval(id); };
  }, [tauri]);

  useEffect(() => {
    if (!tauri) return;
    let active = true;
    resolveTransport().then((t) => {
      if (!active) return;
      setLocalUrl(t.wsUrl);
      setSessionId(t.sessionId);
    });
    import("@tauri-apps/api/core")
      .then(({ invoke }) => invoke<string | null>("get_workspace"))
      .then((w) => { if (active && w) setWorkspace(w); })
      .catch(() => { /* no workspace yet; TopBar simply won't show it */ });
    return () => { active = false; };
  }, [tauri]);

  useEffect(() => { applyTheme(theme); }, [theme]);
  const toggleTheme = () => setTheme((t) => { const next = t === "dark" ? "light" : "dark"; saveTheme(next); return next; });

  const animatedItems = useAnimatedItems(state.items);
  const artifacts = artifactsFrom(state.items);
  const toolCount = state.items.filter((it) => it.kind === "tool").length;
  // Called before any early return so hook order stays stable across the
  // loading → connected transition (React: no conditional hooks).
  const narrow = useNarrow();

  useEffect(() => {
    if (artifacts.length > 0) { setActiveArtifactKey(artifacts[artifacts.length - 1].key); }
  }, [artifacts.length]);

  useEffect(() => {
    if (!sessionId) return;
    if (!localUrl) return;
    dispatch({ type: "reset", userMsgs: loadUserMsgs(sessionId) });
    const WebSocketImpl = (window as unknown as { __WS__?: typeof WebSocket }).__WS__;
    sock.current = connect(
      localUrl,
      { onFrame: (f) => dispatch({ type: "frame", frame: f }), onStatus: (s) => dispatch({ type: "status", status: s }) },
      WebSocketImpl ? { WebSocketImpl } : undefined,
    );
    return () => { sock.current?.close(); sock.current = null; };
  }, [sessionId, localUrl]);

  // Not running inside the desktop shell (plain browser / tests): there is no
  // local bridge to connect to, so render a notice instead of hanging.
  if (!tauri) {
    return (
      <div className="flex h-screen items-center justify-center"
        style={{ background: "var(--surface-base)", color: "var(--text-strong)" }}>
        <p className="font-display text-lg">
          This is the desktop app — launch it from the rust-agent-runtime window.
        </p>
      </div>
    );
  }

  // Tauri mode: brief window before resolveTransport() sets the local session id.
  if (!sessionId) {
    return <div className="h-screen" style={{ background: "var(--surface-base)" }} />;
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
  // Local desktop has no account/session token; "sign out" resets the local
  // session id and reconnects to the bridge with a fresh session.
  const signOut = () => {
    sock.current?.close();
    localStorage.removeItem("local_session_id");
    location.reload();
  };
  const openSettings = () => {
    setShowSettings(true);
    sock.current?.send({ v: 1, session_id: sessionId, kind: "settings_get" });
  };
  const saveSettings = (s: RuntimeSettings) => {
    sock.current?.send({ v: 1, session_id: sessionId, kind: "settings_update", settings: s });
  };

  const connected = state.status === "open";
  const projectLabel = `session ${sessionId.slice(0, 8)}`;
  const model = state.settings?.model;

  return (
    <div className="flex h-screen flex-col" style={{ background: "var(--surface-base)" }}>
      <TopBar projectLabel={projectLabel} online={state.online} status={state.status}
        theme={theme} onToggleTheme={toggleTheme}
        onOpenSettings={openSettings} settingsDisabled={!(connected && state.online)}
        onSignOut={signOut}
        showWorkspaceToggle={narrow} onToggleWorkspace={() => setWorkspaceOpen((o) => !o)}
        tauriWorkspace={workspace}
        onWorkspaceChanged={(p) => setWorkspace(p)}
        llamaOk={llama?.ok ?? false}
        llamaModel={llama?.model} />
      {showSettings && state.settings && (
        <SettingsPanel settings={state.settings} meta={state.settingsMeta} error={state.settingsError}
          disabled={!connected} onSave={saveSettings} onClose={() => setShowSettings(false)} />
      )}
      <div className="relative flex min-h-0 flex-1">
        <div className="min-w-0 flex-1" style={!narrow ? { flexBasis: "38%", maxWidth: "42%", borderRight: "1px solid var(--border)" } : undefined}>
          <AgentColumn items={animatedItems} activeArtifactKey={activeArtifactKey}
            onSelectArtifact={(key) => { setActiveArtifactKey(key); setWorkspaceOpen(true); }}
            projectLabel={projectLabel} model={model}
            pendingApproval={state.pendingApproval} onDecide={decide}
            composerDisabled={!connected} onSend={send}
            usage={state.usage} settings={state.settings}
            toolCount={toolCount} artifactCount={artifacts.length} />
        </div>
        {!narrow && (
          <div className="min-w-0 flex-1">
            <WorkspacePane artifacts={artifacts} activeKey={activeArtifactKey} onSelect={setActiveArtifactKey} />
          </div>
        )}
        {narrow && workspaceOpen && (
          <div className="absolute inset-0 z-20" style={{ background: "var(--surface-overlay)" }}>
            <div className="flex items-center justify-end p-2" style={{ borderBottom: "1px solid var(--border)" }}>
              <button onClick={() => setWorkspaceOpen(false)} aria-label="close workspace"
                className="px-2 text-sm" style={{ color: "var(--text-muted)" }}>✕</button>
            </div>
            <div className="h-[calc(100%-2.5rem)]">
              <WorkspacePane artifacts={artifacts} activeKey={activeArtifactKey} onSelect={setActiveArtifactKey} />
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function useNarrow(): boolean {
  const [narrow, setNarrow] = useState(() => window.matchMedia?.("(max-width: 900px)").matches ?? false);
  useEffect(() => {
    const mq = window.matchMedia?.("(max-width: 900px)");
    if (!mq) return;
    const on = () => setNarrow(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  return narrow;
}
```

- [ ] **Step 4: Remove the Vite proxy (the `ECONNREFUSED` fix)**

Replace the entire contents of `web/vite.config.ts` with:

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Pure-desktop: the webview connects directly to the Tauri local bridge
// (ws://127.0.0.1:<port>/agent), so no dev-server proxy is needed.
export default defineConfig({
  plugins: [react(), tailwindcss()],
});
```

- [ ] **Step 5: Typecheck (tests come in Task 4)**

Run: `cd /home/kalen/rust-agent-runtime/web && npm run typecheck`
Expected: PASS (no references to `PairingScreen`, `needsPairing`, `loadToken`, `saveSession`, or `wsUrl()` remain in `src/`).

- [ ] **Step 6: Confirm no cloud references remain in `src/`**

Run: `grep -rnE "PairingScreen|needsPairing|/browser\?token|8787|/pair\b|/enroll" web/src`
Expected: no output.

- [ ] **Step 7: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add -A
git commit -m "feat(web): remove pairing/browser path and Vite cloud proxy; desktop-only transport"
```

---

### Task 4: Update web tests for pure-desktop

**Files:**
- Delete: `web/test/pairing.test.tsx`
- Modify: `web/test/smoke.test.tsx` (assert the non-Tauri notice)
- Modify: `web/test/app.test.tsx` (Tauri-mocked connected render)
- Modify: `web/src/App.tauri.test.tsx` (drop `needsPairing` from the transport mock)
- Modify: `web/src/transport.test.ts` (new `{ wsUrl }` contract)

**Interfaces:**
- Consumes: `App` default export; `resolveTransport` from `transport.ts`.
- Produces: nothing (test-only).

- [ ] **Step 1: Delete the obsolete pairing test**

```bash
cd /home/kalen/rust-agent-runtime/web
git rm test/pairing.test.tsx
```

- [ ] **Step 2: Rewrite `test/smoke.test.tsx` to assert the notice**

Replace the entire contents of `web/test/smoke.test.tsx` with:

```tsx
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import App from "../src/App";

beforeEach(() => localStorage.clear());

describe("App", () => {
  it("shows the desktop-app notice when not running under Tauri", () => {
    render(<App />);
    expect(screen.getByText(/desktop app/i)).toBeInTheDocument();
  });
});
```

- [ ] **Step 3: Rewrite `test/app.test.tsx` to drive a Tauri-mode connection**

Replace the entire contents of `web/test/app.test.tsx` with:

```tsx
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, act, waitFor } from "@testing-library/react";

// Force Tauri mode and a fixed bridge URL; the connected render is what we test.
vi.mock("../src/transport", () => ({
  isTauri: () => true,
  resolveTransport: async () => ({
    wsUrl: "ws://127.0.0.1:5/agent",
    sessionId: "11111111-1111-1111-1111-111111111111",
  }),
}));
// App dynamically imports invoke() for llama_health/get_workspace; stub it.
vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => null }));

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
  TestWS.last = null;
  (window as unknown as { __WS__?: unknown }).__WS__ = TestWS;
});

describe("App (Tauri mode)", () => {
  it("connects to the local bridge and renders streamed frames", async () => {
    const App = (await import("../src/App")).default;
    render(<App />);
    // resolveTransport() resolves a microtask later, then the connect effect runs.
    await waitFor(() => expect(TestWS.last).not.toBeNull());
    const SID = "11111111-1111-1111-1111-111111111111";
    act(() => { TestWS.last!.onopen?.(); });
    act(() => {
      TestWS.last!.onmessage?.({ data: JSON.stringify({ v: 1, session_id: SID, kind: "event", payload: { type: "token", text: "hello world" } }) });
      // Complete the stream so the assistant text is fully revealed.
      TestWS.last!.onmessage?.({ data: JSON.stringify({ v: 1, session_id: SID, kind: "event", payload: { type: "done", reason: "stop" } }) });
    });
    expect(await screen.findByText("hello world")).toBeInTheDocument();
  });
});
```

- [ ] **Step 4: Update the transport mock in `src/App.tauri.test.tsx`**

In `web/src/App.tauri.test.tsx`, change the mock's returned object from:

```tsx
  resolveTransport: async () => ({
    wsUrl: "ws://127.0.0.1:5/agent",
    sessionId: "11111111-1111-1111-1111-111111111111",
    needsPairing: false,
  }),
```

to:

```tsx
  resolveTransport: async () => ({
    wsUrl: "ws://127.0.0.1:5/agent",
    sessionId: "11111111-1111-1111-1111-111111111111",
  }),
```

(Leave the rest of the file as-is; its `queryByText(/pair with your agent/i)` assertions still hold — that text no longer exists anywhere.)

- [ ] **Step 5: Rewrite `src/transport.test.ts` for the new contract**

Replace the entire contents of `web/src/transport.test.ts` with:

```ts
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

describe("resolveTransport", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    localStorage.clear();
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("uses the local bridge URL in Tauri mode", async () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValue("ws://127.0.0.1:54321/agent");
    const { resolveTransport } = await import("./transport");
    const t = await resolveTransport();
    expect(invokeMock).toHaveBeenCalledWith("get_local_ws_url");
    expect(t.wsUrl).toBe("ws://127.0.0.1:54321/agent");
    expect(t.sessionId).toMatch(/[0-9a-f-]{36}/);
  });

  it("returns an empty wsUrl outside Tauri", async () => {
    const { resolveTransport } = await import("./transport");
    const t = await resolveTransport();
    expect(t.wsUrl).toBe("");
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 6: Run the full web suite**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test`
Expected: PASS — `pairing.test.tsx` gone; `smoke` asserts the notice; `app` renders streamed text in Tauri mode; `transport`/`App.tauri` green; untouched `storage`/`socket` suites still green.

- [ ] **Step 7: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add -A
git commit -m "test(web): pure-desktop tests — notice for non-Tauri, Tauri-mode connected render"
```

---

### Task 5: Docs note, optional dependency, and full-system verification

**Files:**
- Modify: `docs/superpowers/context/README.md` (mark the Cloudflare control-plane subsystem removed)

**Interfaces:**
- Consumes: nothing.
- Produces: nothing.

- [ ] **Step 1: Add the removal note to the context README**

In `docs/superpowers/context/README.md`, find the table row for the Cloudflare control plane (subsystem #5, marked "✅ built & merged") and append to its status cell: `— **removed 2026-06-24** (superseded by [remove-cloud-fully-local](../specs/2026-06-24-remove-cloud-fully-local-design.md)); the app is now desktop-only.` Do not delete the historical row.

- [ ] **Step 2: Full clean build across all three projects**

Run:
```bash
cd /home/kalen/rust-agent-runtime/agent && cargo build && cargo test \
  && cd ../src-tauri && cargo build && cargo test \
  && cd ../web && npm run typecheck && npm test && npm run build
```
Expected: every command PASS. (`src-tauri`'s `e2e_live` stays ignored.)

- [ ] **Step 3: Repo-wide grep — no cloud references outside historical docs**

Run: `grep -rniE "wrangler|:8787|/enroll\b|pairing|control plane|agent-serverd" --include=*.rs --include=*.ts --include=*.tsx --include=*.toml --include=*.json --include=*.html . | grep -vE "/docs/|node_modules|/dist/"`
Expected: no output.

- [ ] **Step 4: Live desktop smoke — boots with no `ECONNREFUSED`**

Pre-req: `curl -s -m 2 localhost:8080/health` returns `{"status":"ok"}` (start llama-server if not).
Run: `cd /home/kalen/rust-agent-runtime && npm run desktop:dev` (launch, observe logs, then quit).
Expected: the window boots and connects to the local bridge; the logs contain **no** `ws proxy error` / `ECONNREFUSED 127.0.0.1:8787` line.

Optional (out of critical path): install the one missing Tauri Linux dep — `sudo apt-get install -y libxdo-dev`. Not required for the current build.

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add -A
git commit -m "docs: mark Cloudflare control plane removed; app is desktop-only"
```

---

## Self-Review

**Spec coverage** (against `2026-06-24-remove-cloud-fully-local-design.md`):
- §3.1 delete cloud outright → Task 1 ✓
- §3.2 Rust strip (config.rs, main.rs/`[[bin]]`, `daemon::run`, `DaemonParams` fields, `setup.rs`, `wire::Presence`, `daemon_roundtrip.rs`, `reqwest`) → Task 2 ✓
- §3.3 web strip (PairingScreen, transport, App, vite proxy, tests) → Tasks 3–4 ✓
- §3.4 docs note + libxdo optional → Task 5 ✓ (libxdo as optional step)
- §4 non-Tauri notice → Task 3 Step 3 + Task 4 Step 2 ✓
- §6 verification checklist → Task 5 Steps 2–4 ✓
- **Added beyond spec:** `src/transport.test.ts` and `src/App.tauri.test.tsx` (both reference `needsPairing`) — covered in Task 4 Steps 4–5. `storage.ts`/`socket.ts` deliberately untouched (generic helpers, independently tested) — noted in Global Constraints.

**Placeholder scan:** none — every code step shows full file or exact before/after.

**Type consistency:** `Transport = { wsUrl, sessionId }` is produced in Task 3 Step 2 and consumed identically in App (Task 3 Step 3) and all mocks (Task 4). `DaemonParams` slimmed identically in `daemon.rs` (Task 2 Step 3) and `setup.rs` (Task 2 Step 4); no remaining producer sets `ws_url`/`agent_token`. `serve`/`local_params` signatures unchanged, so `bridge.rs` and `e2e_live.rs` compile without edits.
