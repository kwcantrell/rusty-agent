import { useEffect, useReducer, useRef, useState } from "react";
import { connect } from "./socket";
import { initialState, reduce, useAnimatedItems, useTurnGrouping } from "./state";
import type { Decision, RuntimeSettings } from "./wire";
import { PairingScreen } from "./components/PairingScreen";
import { StatusBar } from "./components/StatusBar";
import { MessageList } from "./components/MessageList";
import { ApprovalPrompt } from "./components/ApprovalPrompt";
import { Composer } from "./components/Composer";
import { SettingsPanel } from "./components/SettingsPanel";
import { TimelineView } from "./components/TimelineView";
import { appendUserMsg, clearSession, loadSessionId, loadToken, loadUserMsgs, saveSession } from "./storage";

function wsUrl(token: string): string {
  return `${location.origin.replace(/^http/, "ws")}/browser?token=${encodeURIComponent(token)}`;
}

export default function App() {
  const [sessionId, setSessionId] = useState<string | null>(loadSessionId());
  const [token, setToken] = useState<string | null>(loadToken());
  const [state, dispatch] = useReducer(reduce, loadUserMsgs(sessionId ?? ""), initialState);
  const [showSettings, setShowSettings] = useState(false);
  const sock = useRef<ReturnType<typeof connect> | null>(null);
  const messageListRef = useRef<HTMLDivElement>(null);

  const animatedItems = useAnimatedItems(state.items);
  const turns = useTurnGrouping(animatedItems);

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
  const openSettings = () => {
    setShowSettings(true);
    sock.current?.send({ v: 1, session_id: sessionId, kind: "settings_get" });
  };
  const saveSettings = (s: RuntimeSettings) => {
    sock.current?.send({ v: 1, session_id: sessionId, kind: "settings_update", settings: s });
  };

  const connected = state.status === "open";
  return (
    <div className="flex h-screen flex-col bg-zinc-950">
      <StatusBar online={state.online} status={state.status} onSignOut={signOut} onOpenSettings={openSettings} settingsDisabled={!(connected && state.online)} />
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
      <div ref={messageListRef} className="flex flex-1 flex-col overflow-y-auto">
        <MessageList items={animatedItems} />
      </div>
      <TimelineView turns={turns} messageListRef={messageListRef} />
      {state.pendingApproval && <ApprovalPrompt approval={state.pendingApproval} onDecide={decide} />}
      <Composer disabled={!connected} onSend={send} />
    </div>
  );
}
