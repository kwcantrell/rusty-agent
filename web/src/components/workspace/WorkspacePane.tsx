import { useState } from "react";
import type { InspectorArtifact } from "../../state";
import { ArtifactRenderer } from "../inspector/ArtifactRenderer";
import { MarkdownText } from "../MarkdownText";
import { artifactSource } from "./artifactSource";
import { loadWorkspaceView, saveWorkspaceView, type WorkspaceView } from "../../storage";
import { WorkspaceEmptyState } from "./WorkspaceEmptyState";

const VIEWPORTS: { id: WorkspaceView["viewport"]; label: string; maxWidth: string }[] = [
  { id: "desktop", label: "Desktop", maxWidth: "100%" },
  { id: "tablet", label: "Tablet", maxWidth: "820px" },
  { id: "mobile", label: "Mobile", maxWidth: "390px" },
];

export function WorkspacePane({ artifacts, activeKey, onSelect }:
  { artifacts: InspectorArtifact[]; activeKey: string | null; onSelect: (key: string) => void }) {
  const [view, setView] = useState<WorkspaceView>(() => loadWorkspaceView());
  const setMode = (mode: WorkspaceView["mode"]) => { const v = { ...view, mode }; setView(v); saveWorkspaceView(v); };
  const setViewport = (viewport: WorkspaceView["viewport"]) => { const v = { ...view, viewport }; setView(v); saveWorkspaceView(v); };

  if (artifacts.length === 0) {
    return (
      <div className="flex h-full flex-col" style={{ background: "var(--surface-overlay)" }}>
        <WorkspaceEmptyState />
      </div>
    );
  }

  const active = artifacts.find((a) => a.key === activeKey) ?? artifacts[artifacts.length - 1];
  const source = artifactSource(active.display);
  const codeDisabled = source === null;
  // If Code is selected but this artifact has no source, fall back to Preview for rendering.
  const mode = view.mode === "code" && !codeDisabled ? "code" : "preview";
  const vp = VIEWPORTS.find((v) => v.id === view.viewport) ?? VIEWPORTS[0];

  return (
    <div className="flex h-full flex-col" style={{ background: "var(--surface-overlay)" }}>
      {/* header: artifact tabs */}
      <div className="flex items-center gap-1 overflow-x-auto px-2 pt-2" role="tablist"
        style={{ borderBottom: "1px solid var(--border)" }}>
        {artifacts.map((a) => {
          const on = a.key === active.key;
          return (
            <button key={a.key} role="tab" aria-selected={on} onClick={() => onSelect(a.key)}
              className="whitespace-nowrap rounded-t-lg px-3 py-1.5 text-xs"
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
      </div>
      {/* header: mode toggle + viewport */}
      <div className="flex items-center justify-between gap-2 px-3 py-2"
        style={{ borderBottom: "1px solid var(--border)" }}>
        <div className="inline-flex rounded-full p-0.5" style={{ border: "1px solid var(--border)" }}>
          {(["preview", "code"] as const).map((m) => {
            const on = mode === m;
            const disabled = m === "code" && codeDisabled;
            const label = m === "preview" ? "Preview" : "Code";
            return (
              <button key={m} onClick={() => setMode(m)} disabled={disabled}
                title={disabled ? "This artifact has no source to show" : undefined}
                className="rounded-full px-3 py-1 text-xs disabled:opacity-40 disabled:cursor-not-allowed"
                style={{ background: on ? "var(--accent)" : "transparent", color: on ? "var(--accent-fg)" : "var(--text-muted)" }}>
                {label}
              </button>
            );
          })}
        </div>
        <div className="inline-flex gap-1">
          {VIEWPORTS.map((v) => {
            const on = view.viewport === v.id;
            return (
              <button key={v.id} onClick={() => setViewport(v.id)} disabled={mode === "code"}
                className="rounded-full px-2.5 py-1 text-xs disabled:opacity-40 disabled:cursor-not-allowed"
                style={{ background: on ? "var(--surface-raised)" : "transparent",
                         color: on ? "var(--text-strong)" : "var(--text-muted)",
                         border: "1px solid " + (on ? "var(--border)" : "transparent") }}>
                {v.label}
              </button>
            );
          })}
        </div>
      </div>
      {/* body */}
      <div className="min-h-0 flex-1 overflow-auto p-3">
        {mode === "preview" ? (
          <div data-testid="preview-frame" className="mx-auto h-full"
            style={{ maxWidth: vp.maxWidth, width: "100%" }}>
            <ArtifactRenderer display={active.display} />
          </div>
        ) : (
          <div data-testid="code-view">
            <MarkdownText text={"```" + (source?.lang ?? "text") + "\n" + (source?.source ?? "") + "\n```"} />
          </div>
        )}
      </div>
    </div>
  );
}
