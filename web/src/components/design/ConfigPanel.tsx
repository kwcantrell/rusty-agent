import type { RuntimeSettings, DiscoveredSkill } from "../../wire";

export interface ConfigPanelProps {
  settings: RuntimeSettings | null;
  meta: { workspace: string; apiKeySet: boolean; hardFloor: string[];
    discoveredSkills: DiscoveredSkill[] } | null;
  error: string | null;
  disabled: boolean;
  onSave: (s: RuntimeSettings) => void;
}

export function ConfigPanel({ settings }: ConfigPanelProps) {
  return (
    <p className="p-4 text-sm" style={{ color: "var(--text-muted)" }}>
      {settings ? "Config editor coming in Task 10." : "Loading settings…"}
    </p>
  );
}
