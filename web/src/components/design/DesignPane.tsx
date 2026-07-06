import { useState } from "react";
import type { Item } from "../../state";
import type { RuntimeSettings, DiscoveredSkill } from "../../wire";
import { isTauri } from "../../transport";
import { useDesignStore } from "../../designStore";
import { buildFeedbackMessage } from "../../designFeedback";
import { DesignCanvas } from "./DesignCanvas";
import { ConfigPanel } from "./ConfigPanel";

export interface DesignPaneProps {
  items: Item[];
  sessionId: string;
  onSend: (text: string) => void;
  sendDisabled: boolean;
  settings: RuntimeSettings | null;
  settingsMeta: { workspace: string; apiKeySet: boolean; hardFloor: string[];
    discoveredSkills: DiscoveredSkill[] } | null;
  settingsError: string | null;
  onSaveSettings: (s: RuntimeSettings) => void;
  onLoadSettings: () => void;
}

export function DesignPane({ items, sessionId, onSend, sendDisabled,
  settings, settingsMeta, settingsError, onSaveSettings, onLoadSettings }: DesignPaneProps) {
  const [section, setSection] = useState<"canvas" | "config">("canvas");
  const store = useDesignStore(items, sessionId);
  const [activeId, setActiveId] = useState<string | null>(null);
  const tauri = isTauri();
  const active = store.designs.find((d) => d.id === activeId) ?? store.designs[store.designs.length - 1];
  const sub = (on: boolean) => ({
    color: on ? "var(--text-strong)" : "var(--text-muted)", fontWeight: on ? 600 : 400,
  });

  return (
    <div className="flex h-full flex-col" style={{ background: "var(--surface-overlay)" }}>
      <div className="flex gap-1 px-2 pt-1" role="tablist" style={{ borderBottom: "1px solid var(--border)" }}>
        <button role="tab" aria-selected={section === "canvas"} onClick={() => setSection("canvas")}
          className="rounded-t-lg px-3 py-1 text-xs" style={sub(section === "canvas")}>Canvas</button>
        {tauri && (
          <button role="tab" aria-selected={section === "config"}
            onClick={() => { setSection("config"); onLoadSettings(); }}
            className="rounded-t-lg px-3 py-1 text-xs" style={sub(section === "config")}>Config</button>
        )}
      </div>
      {section === "config" && tauri ? (
        <ConfigPanel settings={settings} meta={settingsMeta} error={settingsError}
          disabled={sendDisabled} onSave={onSaveSettings} />
      ) : !active ? (
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
            onSendFeedback={(v, pins) => {
              onSend(buildFeedbackMessage(active.id, v, pins));
              store.recordSent(active.id, v, pins);
            }}
            sendDisabled={sendDisabled} />
        </>
      )}
    </div>
  );
}
