import type { InspectorArtifact } from "../../state";
import { ArtifactRenderer } from "./ArtifactRenderer";

export function Inspector({ artifacts, activeKey, onSelect, onClose }:
  { artifacts: InspectorArtifact[]; activeKey: string | null;
    onSelect: (key: string) => void; onClose: () => void }) {
  if (artifacts.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-6 text-sm"
        style={{ color: "var(--text-muted)" }}>
        Nothing to inspect yet.
      </div>
    );
  }
  const active = artifacts.find((a) => a.key === activeKey) ?? artifacts[artifacts.length - 1];
  return (
    <div className="flex h-full flex-col" style={{ background: "var(--surface-raised)" }}>
      <div className="flex items-center gap-1 px-2 pt-2" role="tablist"
        style={{ borderBottom: "1px solid var(--border)" }}>
        {artifacts.map((a) => {
          const on = a.key === active.key;
          return (
            <button key={a.key} role="tab" aria-selected={on} onClick={() => onSelect(a.key)}
              className="rounded-t px-3 py-1 text-xs"
              style={{
                background: on ? "var(--surface-overlay)" : "transparent",
                color: on ? "var(--text-strong)" : "var(--text-muted)",
                fontWeight: on ? 600 : 400,
                border: on ? "1px solid var(--border)" : "1px solid transparent",
                borderBottom: "none",
              }}>
              {a.title}
            </button>
          );
        })}
        <button onClick={onClose} aria-label="close inspector"
          className="ml-auto px-2 text-xs hover:opacity-80" style={{ color: "var(--text-muted)" }}>✕</button>
      </div>
      <div className="flex-1 overflow-auto" style={{ background: "var(--surface-overlay)" }}>
        <ArtifactRenderer display={active.display} />
      </div>
    </div>
  );
}
