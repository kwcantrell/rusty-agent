import type { RightTab } from "../storage";
import { saveRightTab } from "../storage";

const LABELS: Record<RightTab, string> = { workspace: "Workspace", context: "Context", design: "Design" };

export function RightPaneTabs(
  { rightTab, setRightTab }: { rightTab: RightTab; setRightTab: (t: RightTab) => void },
) {
  return (
    <div className="flex gap-1 px-2 pt-2" role="tablist" style={{ borderBottom: "1px solid var(--border)" }}>
      {(["workspace", "context", "design"] as const).map((t) => (
        <button key={t} role="tab" aria-selected={rightTab === t}
          onClick={() => { setRightTab(t); saveRightTab(t); }}
          className="rounded-t-lg px-3 py-1.5 text-xs"
          style={{ color: rightTab === t ? "var(--text-strong)" : "var(--text-muted)",
            fontWeight: rightTab === t ? 600 : 400 }}>
          {LABELS[t]}
        </button>
      ))}
    </div>
  );
}
