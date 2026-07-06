import { useState } from "react";
import type { Item } from "../../state";
import { useDesignStore, LIVE_PREVIEW_ID } from "../../designStore";
import { isLocalUrl } from "../inspector/urlGuard";
import { buildFeedbackMessage } from "../../designFeedback";
import { DesignCanvas } from "./DesignCanvas";

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
