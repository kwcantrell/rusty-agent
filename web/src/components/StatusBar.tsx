import type { ConnectionStatus } from "../state";

export function StatusBar({ online, status, onSignOut, onOpenSettings }:
  { online: boolean; status: ConnectionStatus; onSignOut: () => void; onOpenSettings?: () => void }) {
  return (
    <div className="flex items-center justify-between border-b border-zinc-800 bg-zinc-950 px-4 py-2 text-sm">
      <div className="flex items-center gap-2">
        <span className={`h-2 w-2 rounded-full ${online ? "bg-green-400" : "bg-zinc-600"}`} />
        <span className="text-zinc-300">{online ? "agent online" : "agent offline"}</span>
        <span className="text-zinc-600">· {status}</span>
      </div>
      <div className="flex items-center gap-3">
        {onOpenSettings && (
          <button onClick={onOpenSettings} className="text-zinc-400 hover:text-zinc-200" aria-label="settings">⚙</button>
        )}
        <button onClick={onSignOut} className="text-zinc-400 hover:text-zinc-200">sign out</button>
      </div>
    </div>
  );
}
