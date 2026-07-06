import { useEffect } from "react";
import type { RuntimeSettings } from "../../wire";
import { SettingsForm, type SettingsMeta } from "../SettingsForm";

export interface ConfigPanelProps {
  settings: RuntimeSettings | null;
  meta: SettingsMeta | null;
  error: string | null;
  disabled: boolean;
  onSave: (s: RuntimeSettings) => void;
  onLoad?: () => void;
}

export function ConfigPanel({ settings, meta, error, disabled, onSave, onLoad }: ConfigPanelProps) {
  // Fetch fresh settings once when the panel opens (was the subtab's click handler).
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => { onLoad?.(); }, []);
  if (!settings) {
    return <p className="p-4 text-sm" style={{ color: "var(--text-muted)" }}>Loading settings…</p>;
  }
  return (
    <div className="min-h-0 flex-1 overflow-y-auto p-4">
      <p className="mb-3 text-xs" style={{ color: "var(--text-muted)" }}>
        Changes apply from the next turn; an in-flight turn finishes on the old config.
      </p>
      {/* remount when fresh settings arrive so the form re-seeds */}
      <SettingsForm key={JSON.stringify(settings)} settings={settings} meta={meta}
        error={error} disabled={disabled} onSave={onSave} />
    </div>
  );
}
