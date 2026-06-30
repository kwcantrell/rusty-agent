import { useState } from "react";
import type { SkillDto } from "./types";
import { getSkill, saveSkill } from "./api";

export function SkillSection({ skills }: { skills: { name: string; description: string }[] }) {
  const [open, setOpen] = useState<SkillDto | null>(null);
  const [body, setBody] = useState("");
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const onOpen = async (name: string) => {
    try {
      const s = await getSkill(name);
      setOpen(s); setBody(s.body); setSaved(false); setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  const onSave = async () => {
    if (!open) return;
    try {
      await saveSkill(open.name, body);
      setSaved(true); setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="px-3 py-2 text-xs">
      <div className="font-semibold" style={{ color: "var(--text-strong)" }}>Skills</div>
      {error && <div style={{ color: "var(--state-error)" }}>{error}</div>}
      <div className="mt-1 space-y-0.5">
        {skills.map((s) => (
          <button key={s.name} aria-label={`open ${s.name}`} onClick={() => onOpen(s.name)}
            className="block w-full text-left" style={{ color: "var(--text-strong)" }}>
            {s.name} <span style={{ color: "var(--text-muted)" }}>— {s.description}</span>
          </button>
        ))}
      </div>
      {open && (
        <div className="mt-2 rounded p-1" style={{ border: "1px solid var(--border)" }}>
          <div className="mb-1" style={{ color: "var(--text-muted)" }}>{open.name}/SKILL.md</div>
          <textarea value={body} onChange={(e) => { setBody(e.target.value); setSaved(false); }}
            rows={8} className="w-full rounded px-2 py-1 font-mono"
            style={{ background: "var(--surface-base)", color: "var(--text-strong)",
              border: "1px solid var(--border)" }} />
          <div className="mt-1 flex items-center gap-2">
            <button onClick={onSave} style={{ color: "var(--accent)" }}>save</button>
            {saved && <span style={{ color: "var(--text-muted)" }}>saved ✓</span>}
            <button onClick={() => setOpen(null)} style={{ color: "var(--text-muted)" }}>close</button>
          </div>
        </div>
      )}
    </div>
  );
}
