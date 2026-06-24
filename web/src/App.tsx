import { useEffect, useReducer, useRef, useState } from "react";
import { connect } from "./socket";
import { resolveTransport, isTauri } from "./transport";
import { initialState, reduce, useAnimatedItems, artifactsFrom } from "./state";
import type { Decision, RuntimeSettings } from "./wire";
import { PairingScreen } from "./components/PairingScreen";
import { SettingsPanel } from "./components/SettingsPanel";
import { TopBar } from "./components/TopBar";
import { AgentColumn } from "./components/AgentColumn";
import { WorkspacePane } from "./components/workspace/WorkspacePane";
import { resolveInitialTheme, applyTheme, type Theme } from "./theme";
import { appendUserMsg, clearSession, loadSessionId, loadTheme, loadToken, loadUserMsgs, saveSession, saveTheme } from "./storage";

function wsUrl(token: string): string {
  return `${location.origin.replace(/^http/, "ws")}/browser?token=${encodeURIComponent(token)}`;
}

export default function App() {
  const [sessionId, setSessionId] = useState<string | null>(loadSessionId());
  const [token, setToken] = useState<string | null>(loadToken());
  const [state, dispatch] = useReducer(reduce, loadUserMsgs(sessionId ?? ""), initialState);
  const [showSettings, setShowSettings] = useState(false);
  const [theme, setTheme] = useState<Theme>(() =>
    resolveInitialTheme(loadTheme(), window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false));
  const [activeArtifactKey, setActiveArtifactKey] = useState<string | null>(null);
  const [workspaceOpen, setWorkspaceOpen] = useState(false);
  const sock = useRef<ReturnType<typeof connect> | null>(null);
  const [localUrl, setLocalUrl] = useState<string | null>(null);
  const tauri = isTauri();

  useEffect(() => {
    if (!tauri) return;
    let active = true;
    resolveTransport().then((t) => {
      if (!active) return;
      setLocalUrl(t.wsUrl);
      setSessionId(t.sessionId); // satisfies the existing sessionId gate
    });
    return () => { active = false; };
  }, [tauri]);

  useEffect(() => { applyTheme(theme); }, [theme]);
  const toggleTheme = () => setTheme((t) => { const next = t === "dark" ? "light" : "dark"; saveTheme(next); return next; });

  const animatedItems = useAnimatedItems(state.items);
  const artifacts = artifactsFrom(state.items);
  const toolCount = state.items.filter((it) => it.kind === "tool").length;
  // Called before any early return so hook order stays stable across the
  // pairing/loading → connected transition (React: no conditional hooks).
  const narrow = useNarrow();

  useEffect(() => {
    if (artifacts.length > 0) { setActiveArtifactKey(artifacts[artifacts.length - 1].key); }
  }, [artifacts.length]);

  useEffect(() => {
    if (!sessionId) return;
    if (tauri ? !localUrl : !token) return;
    dispatch({ type: "reset", userMsgs: loadUserMsgs(sessionId) });
    const WebSocketImpl = (window as unknown as { __WS__?: typeof WebSocket }).__WS__;
    const url = tauri ? (localUrl as string) : wsUrl(token as string);
    sock.current = connect(
      url,
      { onFrame: (f) => dispatch({ type: "frame", frame: f }), onStatus: (s) => dispatch({ type: "status", status: s }) },
      WebSocketImpl ? { WebSocketImpl } : undefined,
    );
    return () => { sock.current?.close(); sock.current = null; };
  }, [token, sessionId, tauri, localUrl]);

  if (!tauri && (!token || !sessionId)) {
    return (
      <div className="h-screen" style={{ background: "var(--surface-base)" }}>
        <PairingScreen onPaired={({ sessionId, token }) => { saveSession(sessionId, token); setSessionId(sessionId); setToken(token); }} />
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
  const signOut = () => { sock.current?.close(); clearSession(); setToken(null); setSessionId(null); };
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
        showWorkspaceToggle={narrow} onToggleWorkspace={() => setWorkspaceOpen((o) => !o)} />
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
