import type { Item } from "../state";

type ToolItem = Extract<Item, { kind: "tool" }>;

export function ActivityRail({ items, sessionLabel, onOpenSettings, collapsed, onToggleCollapse }:
  { items: Item[]; sessionLabel: string; onOpenSettings?: () => void;
    collapsed: boolean; onToggleCollapse: () => void }) {
  const tools = items.filter((i): i is ToolItem => i.kind === "tool");
  return (
    <div className="flex h-full flex-col gap-2 p-2"
      style={{ width: collapsed ? 44 : 168, background: "var(--surface-raised)", borderRight: "1px solid var(--border)" }}>
      <div className="flex items-center justify-between">
        {!collapsed && <span className="text-xs font-semibold" style={{ color: "var(--text-strong)" }}>{sessionLabel}</span>}
        <button onClick={onToggleCollapse} aria-label="toggle activity rail"
          className="text-xs hover:opacity-80" style={{ color: "var(--text-muted)" }}>{collapsed ? "»" : "«"}</button>
      </div>
      {!collapsed && <div className="text-[10px] uppercase tracking-wide" style={{ color: "var(--text-muted)" }}>Activity</div>}
      <div className="flex flex-1 flex-col gap-1 overflow-y-auto">
        {tools.map((t, i) => (
          <div key={i} className="flex items-center gap-2 rounded px-1.5 py-1 text-xs" style={{ color: "var(--text)" }}>
            <span className="h-1.5 w-1.5 flex-none rounded-full"
              style={{ background: t.status === "running" ? "var(--state-run)" : "var(--state-done)" }} />
            {!collapsed && <span className="truncate font-mono">{t.name}</span>}
          </div>
        ))}
      </div>
      {onOpenSettings && (
        <button onClick={onOpenSettings} className="flex items-center gap-2 rounded px-1.5 py-1 text-xs hover:opacity-80"
          style={{ color: "var(--text-muted)" }} aria-label="settings">
          <span>⚙</span>{!collapsed && <span>Settings</span>}
        </button>
      )}
    </div>
  );
}
