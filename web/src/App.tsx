import { useEffect, useReducer, useRef, useState } from "react";
import { connect } from "./socket";
import { initialState, reduce, useAnimatedItems, artifactsFrom } from "./state";
import type { Decision, RuntimeSettings } from "./wire";
import { PairingScreen } from "./components/PairingScreen";
import { StatusBar } from "./components/StatusBar";
import { MessageList } from "./components/MessageList";
import { ApprovalPrompt } from "./components/ApprovalPrompt";
import { Composer } from "./components/Composer";
import { SettingsPanel } from "./components/SettingsPanel";
import { ActivityRail } from "./components/ActivityRail";
import { Inspector } from "./components/inspector/Inspector";
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
  const [railCollapsed, setRailCollapsed] = useState(false);
  const [inspectorOpen, setInspectorOpen] = useState(true);
  const sock = useRef<ReturnType<typeof connect> | null>(null);
  const messageListRef = useRef<HTMLDivElement>(null);

  useEffect(() => { applyTheme(theme); }, [theme]);
  const toggleTheme = () => setTheme((t) => { const next = t === "dark" ? "light" : "dark"; saveTheme(next); return next; });

  const animatedItems = useAnimatedItems(state.items);
  const artifacts = artifactsFrom(state.items);

  useEffect(() => {
    if (artifacts.length > 0) { setActiveArtifactKey(artifacts[artifacts.length - 1].key); setInspectorOpen(true); }
  }, [artifacts.length]);

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
      <div className="h-screen" style={{ background: "var(--surface-base)" }}>
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
  const openSettings = () => {
    setShowSettings(true);
    sock.current?.send({ v: 1, session_id: sessionId, kind: "settings_get" });
  };
  const saveSettings = (s: RuntimeSettings) => {
    sock.current?.send({ v: 1, session_id: sessionId, kind: "settings_update", settings: s });
  };

  const connected = state.status === "open";
  return (
    <div className="flex h-screen flex-col" style={{ background: "var(--surface-base)" }}>
      <StatusBar online={state.online} status={state.status} onSignOut={signOut} onOpenSettings={openSettings} settingsDisabled={!(connected && state.online)} theme={theme} onToggleTheme={toggleTheme} />
      {showSettings && state.settings && (
        <SettingsPanel
          settings={state.settings}
          meta={state.settingsMeta}
          error={state.settingsError}
          disabled={!connected}
          onSave={saveSettings}
          onClose={() => setShowSettings(false)}
        />
      )}
      <div className="flex min-h-0 flex-1">
        <ActivityRail items={state.items} sessionLabel={sessionId.slice(0, 8)}
          onOpenSettings={openSettings} collapsed={railCollapsed}
          onToggleCollapse={() => setRailCollapsed((c) => !c)} />
        <div ref={messageListRef} className="flex min-w-0 flex-1 flex-col overflow-y-auto">
          <MessageList items={animatedItems} activeArtifactKey={activeArtifactKey}
            onSelectArtifact={(key) => { setActiveArtifactKey(key); setInspectorOpen(true); }} />
        </div>
        {inspectorOpen && (
          <div style={{ width: 360, borderLeft: "1px solid var(--border)" }} className="min-h-0">
            <Inspector artifacts={artifacts} activeKey={activeArtifactKey}
              onSelect={setActiveArtifactKey} onClose={() => setInspectorOpen(false)} />
          </div>
        )}
      </div>
      {state.pendingApproval && <ApprovalPrompt approval={state.pendingApproval} onDecide={decide} />}
      <Composer disabled={!connected} onSend={send} />
    </div>
  );
}
