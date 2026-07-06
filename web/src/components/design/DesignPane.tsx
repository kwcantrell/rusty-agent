import { useEffect, useState } from "react";
import type { Item } from "../../state";
import { useDesignStore, LIVE_PREVIEW_ID } from "../../designStore";
import { isLocalUrl } from "../inspector/urlGuard";
import { buildFeedbackMessage } from "../../designFeedback";
import { DesignCanvas } from "./DesignCanvas";
import { isTauri } from "../../transport";
import {
  detectDevScripts, startDevServer, stopDevServer,
  type DevScriptCandidate, type DevServerStatus,
} from "./devServer";

export interface DesignPaneProps {
  items: Item[];
  sessionId: string;
  onSend: (text: string) => void;
  sendDisabled: boolean;
}

export function DesignPane({ items, sessionId, onSend, sendDisabled }: DesignPaneProps) {
  const store = useDesignStore(items, sessionId);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [urlDraft, setUrlDraft] = useState("");
  const [urlError, setUrlError] = useState<string | null>(null);

  const [candidates, setCandidates] = useState<DevScriptCandidate[]>([]);
  const [picked, setPicked] = useState(0);
  const [running, setRunning] = useState<DevServerStatus | null>(null);
  const [devError, setDevError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!isTauri()) return;
    detectDevScripts().then(setCandidates).catch(() => setCandidates([]));
  }, []);

  const launch = async (cand: DevScriptCandidate) => {
    setBusy(true); setDevError(null);
    try {
      const status = await startDevServer(cand);
      store.addUrlVersion(status.url);
      setActiveId(LIVE_PREVIEW_ID);
      setRunning(status);
    } catch (e) {
      setDevError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const stop = async () => { await stopDevServer().catch(() => {}); setRunning(null); };

  const preview = () => {
    if (!isLocalUrl(urlDraft)) {
      setUrlError("Only localhost URLs (e.g. http://localhost:5173) can be previewed.");
      return;
    }
    store.addUrlVersion(urlDraft);
    setActiveId(LIVE_PREVIEW_ID);
    setUrlError(null);
  };
  const active = store.designs.find((d) => d.id === activeId) ?? store.designs[store.designs.length - 1];
  const sub = (on: boolean) => ({
    color: on ? "var(--text-strong)" : "var(--text-muted)", fontWeight: on ? 600 : 400,
  });

  return (
    <div className="flex h-full flex-col" style={{ background: "var(--surface-overlay)" }}>
      <div className="px-2 pt-2">
        {candidates.length > 0 && !running && (
          <div className="flex gap-1 pb-1">
            {candidates.length > 1 && (
              <select aria-label="dev script" value={picked}
                onChange={(e) => setPicked(Number(e.target.value))}
                className="min-w-0 flex-1 rounded px-2 py-1 text-xs"
                style={{ background: "var(--surface-base)", color: "var(--text-strong)",
                  border: "1px solid var(--border)" }}>
                {candidates.map((c, i) => <option key={c.dir + c.script} value={i}>{c.label}</option>)}
              </select>
            )}
            <button onClick={() => launch(candidates[picked])} disabled={busy}
              className="rounded px-2 py-1 text-xs disabled:opacity-40"
              style={{ background: "var(--accent)", color: "var(--accent-fg)" }}>
              {busy ? "Starting…" : candidates.length > 1
                ? "Start dev server" : `Start dev server (${candidates[0].label})`}
            </button>
          </div>
        )}
        {running && (
          <div className="flex items-center gap-2 pb-1 text-xs" style={{ color: "var(--text-muted)" }}>
            <span className="min-w-0 flex-1 truncate">▶ {running.candidate.label} — {running.url}</span>
            <button onClick={() => launch(running.candidate)} disabled={busy}
              className="rounded px-2 py-0.5" style={{ border: "1px solid var(--border)" }}>Restart</button>
            <button onClick={stop} className="rounded px-2 py-0.5"
              style={{ border: "1px solid var(--border)" }}>Stop</button>
          </div>
        )}
        {devError && <p className="pb-1 text-xs" style={{ color: "var(--text-muted)" }}>{devError}</p>}
        <div className="flex gap-1">
          <input aria-label="preview url" value={urlDraft} placeholder="http://localhost:5173"
            onChange={(e) => setUrlDraft(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") preview(); }}
            className="min-w-0 flex-1 rounded px-2 py-1 text-xs"
            style={{ background: "var(--surface-base)", color: "var(--text-strong)",
              border: "1px solid var(--border)" }} />
          <button onClick={preview} className="rounded px-2 py-1 text-xs"
            style={{ background: "var(--surface-raised)", color: "var(--text-strong)",
              border: "1px solid var(--border)" }}>Preview</button>
        </div>
        {urlError && <p className="pt-1 text-xs" style={{ color: "var(--text-muted)" }}>{urlError}</p>}
      </div>
      {!active ? (
        <div className="flex flex-1 items-center justify-center p-6 text-center text-sm"
          style={{ color: "var(--text-muted)" }}>
          <p>No designs yet. Ask the agent to render one with id &quot;design:&lt;name&gt;&quot;.</p>
        </div>
      ) : (
        <>
          {store.designs.length > 1 && (
            <div className="flex gap-1 overflow-x-auto px-2 pt-1" role="tablist">
              {store.designs.map((d) => (
                <button key={d.id} role="tab" aria-selected={d.id === active.id}
                  onClick={() => setActiveId(d.id)}
                  className="whitespace-nowrap rounded-t px-2 py-1 text-xs" style={sub(d.id === active.id)}>
                  {d.title}
                </button>
              ))}
            </div>
          )}
          <DesignCanvas key={active.id} design={active}
            sentPins={(v) => store.sentPins(active.id, v)}
            onSendFeedback={(v, pins, url) => {
              onSend(buildFeedbackMessage(active.id, v, pins, undefined, url));
              store.recordSent(active.id, v, pins);
            }}
            sendDisabled={sendDisabled} />
        </>
      )}
    </div>
  );
}
