import type { RuntimeSettings } from "../wire";
import { SettingsForm, type SettingsMeta } from "./SettingsForm";

interface Props {
  settings: RuntimeSettings;
  meta: SettingsMeta | null;
  error: string | null;
  disabled: boolean;
  onSave: (s: RuntimeSettings) => void;
  onClose: () => void;
}

export function SettingsPanel({ settings, meta, error, disabled, onSave, onClose }: Props) {
  return (
    <div className="absolute inset-0 z-10 flex justify-end" style={{ background: "rgba(0,0,0,0.5)" }}>
      <div className="h-full w-96 overflow-y-auto p-4 shadow-xl"
        style={{ background: "var(--surface-overlay)", color: "var(--text)" }}>
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-lg font-semibold" style={{ color: "var(--text-strong)" }}>Settings</h2>
          <button onClick={onClose} className="hover:opacity-80" style={{ color: "var(--text-muted)" }}>close</button>
        </div>
        <SettingsForm settings={settings} meta={meta} error={error} disabled={disabled} onSave={onSave} />
      </div>
    </div>
  );
}
