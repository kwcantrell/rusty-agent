import { useState } from "react";
import type { Design, Pin } from "../../designStore";
import { ArtifactRenderer } from "../inspector/ArtifactRenderer";
import { VersionBar } from "./VersionBar";
import { AnnotationOverlay } from "./AnnotationOverlay";

export function DesignCanvas({ design, sentPins, onSendFeedback, sendDisabled }: {
  design: Design;
  sentPins: (version: number) => Pin[];
  onSendFeedback: (version: number, pins: Pin[]) => void;
  sendDisabled: boolean;
}) {
  const [viewed, setViewed] = useState<number | null>(null); // null = follow latest
  const [compare, setCompare] = useState(false);
  const [interact, setInteract] = useState(false);
  const total = design.versions.length;
  const cur = Math.min(viewed ?? total - 1, total - 1);
  const behind = viewed !== null && cur < total - 1;
  const curDisplay = design.versions[cur].display;
  const liveUrl = "Url" in curDisplay ? curDisplay.Url.url : null;
  const modeBtn = (on: boolean) => ({
    background: on ? "var(--accent)" : "transparent",
    color: on ? "var(--accent-fg)" : "var(--text-muted)",
    border: "1px solid var(--border)",
  });
  return (
    <div className="flex h-full min-h-0 flex-col">
      <VersionBar current={cur} total={total} compare={compare}
        renderableFlags={design.versions.map((v) => v.renderable)}
        onSelect={setViewed} onLatest={() => setViewed(null)}
        onToggleCompare={() => setCompare((c) => !c)} />
      {behind && (
        <button onClick={() => setViewed(null)}
          className="mx-3 mt-2 rounded px-2 py-1 text-xs"
          style={{ background: "var(--surface-raised)", color: "var(--text-strong)",
            border: "1px solid var(--border)" }}>
          v{total} available — jump to latest
        </button>
      )}
      {liveUrl && !compare && (
        <div className="flex gap-1 px-3 pt-2" role="group" aria-label="canvas mode">
          <button aria-pressed={interact} onClick={() => setInteract(true)}
            className="rounded px-2 py-0.5 text-xs" style={modeBtn(interact)}>Interact</button>
          <button aria-pressed={!interact} onClick={() => setInteract(false)}
            className="rounded px-2 py-0.5 text-xs" style={modeBtn(!interact)}>Pin feedback</button>
        </div>
      )}
      <div className="min-h-0 flex-1 overflow-auto p-3">
        {compare && cur > 0 ? (
          <div className="flex h-full gap-2">
            <div className="min-w-0 flex-1" data-testid="compare-left">
              <ArtifactRenderer display={design.versions[cur - 1].display} />
            </div>
            <div className="min-w-0 flex-1" data-testid="compare-right">
              <ArtifactRenderer display={design.versions[cur].display} />
            </div>
          </div>
        ) : (
          <AnnotationOverlay sent={sentPins(cur + 1)} disabled={sendDisabled}
            passthrough={!!liveUrl && interact}
            onSend={(pins) => onSendFeedback(cur + 1, pins)}>
            <ArtifactRenderer display={design.versions[cur].display} />
          </AnnotationOverlay>
        )}
      </div>
    </div>
  );
}
