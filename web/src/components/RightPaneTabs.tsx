import type { RightTab } from "../storage";
import { saveRightTab } from "../storage";
import { isTauri } from "../transport";

const LABELS: Record<RightTab, string> = {
  workspace: "Workspace", context: "Context", design: "Design",
  architecture: "Architecture", config: "Config",
};
const BASE: readonly RightTab[] = ["workspace", "context", "design"];
const TAURI_TABS: readonly RightTab[] = ["architecture", "config"];

export function RightPaneTabs(
  { rightTab, setRightTab }: { rightTab: RightTab; setRightTab: (t: RightTab) => void },
) {
  const tabs = isTauri() ? [...BASE, ...TAURI_TABS] : BASE;
  return (
    <div className="flex gap-1 px-2 pt-2" role="tablist" style={{ borderBottom: "1px solid var(--border)" }}>
      {tabs.map((t) => (
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
