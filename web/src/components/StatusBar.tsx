import type { ConnectionStatus } from "../state";
import type { Theme } from "../theme";
import { ThemeToggle } from "./ThemeToggle";

export function StatusBar({ online, status, onSignOut, onOpenSettings, settingsDisabled, theme, onToggleTheme }:
  { online: boolean; status: ConnectionStatus; onSignOut: () => void;
    onOpenSettings?: () => void; settingsDisabled?: boolean;
    theme: Theme; onToggleTheme: () => void }) {
  return (
    <div className="flex items-center justify-between px-4 py-2 text-sm"
      style={{ background: "var(--surface-raised)", borderBottom: "1px solid var(--border)" }}>
      <div className="flex items-center gap-2">
        <span className="h-2 w-2 rounded-full"
          style={{ background: online ? "var(--state-done)" : "var(--text-muted)" }} />
        <span style={{ color: "var(--text)" }}>{online ? "agent online" : "agent offline"}</span>
        <span style={{ color: "var(--text-muted)" }}>· {status}</span>
      </div>
      <div className="flex items-center gap-3">
        <ThemeToggle theme={theme} onToggle={onToggleTheme} />
        {onOpenSettings && (
          <button onClick={onOpenSettings} disabled={settingsDisabled}
            className="disabled:opacity-40 disabled:cursor-not-allowed hover:opacity-80"
            style={{ color: "var(--text-muted)" }} aria-label="settings">⚙</button>
        )}
        <button onClick={onSignOut} className="hover:opacity-80"
          style={{ color: "var(--text-muted)" }}>sign out</button>
      </div>
    </div>
  );
}
