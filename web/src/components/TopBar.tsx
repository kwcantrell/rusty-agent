import { invoke } from "@tauri-apps/api/core";
import type { ConnectionStatus } from "../state";
import type { Theme } from "../theme";
import { ThemeToggle } from "./ThemeToggle";

export function TopBar({ projectLabel, online, status, theme, onToggleTheme,
  onOpenSettings, settingsDisabled, onSignOut, onToggleWorkspace, showWorkspaceToggle,
  tauriWorkspace, onWorkspaceChanged, llamaOk, llamaModel }:
  { projectLabel: string; online: boolean; status: ConnectionStatus;
    theme: Theme; onToggleTheme: () => void;
    onOpenSettings?: () => void; settingsDisabled?: boolean; onSignOut: () => void;
    onToggleWorkspace?: () => void; showWorkspaceToggle?: boolean;
    tauriWorkspace?: string; onWorkspaceChanged?: (path: string) => void;
    llamaOk?: boolean; llamaModel?: string }) {
  return (
    <div className="flex items-center justify-between px-4 py-2.5"
      style={{ background: "var(--surface-base)", borderBottom: "1px solid var(--border)" }}>
      <div className="flex items-center gap-2">
        <span className="h-2 w-2 rounded-full"
          style={{ background: online ? "var(--state-done)" : "var(--text-muted)" }}
          title={online ? "agent online" : "agent offline"} />
        <span className="font-display text-base" style={{ color: "var(--text-strong)" }}>{projectLabel}</span>
        <span className="text-xs" style={{ color: "var(--text-muted)" }}>· {status}</span>
      </div>
      <div className="flex items-center gap-3 text-sm">
        {llamaOk !== undefined && (
          <span className="flex items-center gap-1 text-xs" style={{ color: "var(--text-muted)" }}
            title={llamaOk ? `llama-server ready: ${llamaModel ?? "model loaded"}` : "llama-server offline (localhost:8080)"}>
            <span className="h-2 w-2 rounded-full"
              style={{ background: llamaOk ? "var(--state-done)" : "var(--text-muted)" }} />
            LLM
          </span>
        )}
        {tauriWorkspace !== undefined && (
          <div className="flex items-center gap-2 text-xs" style={{ color: "var(--text-muted)" }}>
            <span title={tauriWorkspace} className="max-w-[28ch] truncate">{tauriWorkspace}</span>
            <button
              type="button"
              className="rounded-full px-3 py-1 hover:opacity-80"
              style={{ border: "1px solid var(--border)", color: "var(--text)" }}
              onClick={async () => {
                const picked = await invoke<string | null>("pick_workspace");
                if (picked) onWorkspaceChanged?.(picked);
              }}
            >
              Change…
            </button>
          </div>
        )}
        {showWorkspaceToggle && (
          <button onClick={onToggleWorkspace} aria-label="toggle workspace"
            className="rounded-full px-3 py-1 text-xs hover:opacity-80"
            style={{ border: "1px solid var(--border)", color: "var(--text)" }}>Workspace</button>
        )}
        <ThemeToggle theme={theme} onToggle={onToggleTheme} />
        {onOpenSettings && (
          <button onClick={onOpenSettings} disabled={settingsDisabled} aria-label="settings"
            className="disabled:opacity-40 disabled:cursor-not-allowed hover:opacity-80"
            style={{ color: "var(--text-muted)" }}>⚙</button>
        )}
        <button onClick={onSignOut} className="hover:opacity-80" style={{ color: "var(--text-muted)" }}>sign out</button>
      </div>
    </div>
  );
}
