import { useEffect, useState } from "react";
import type { MemoryRow } from "./types";
import { listMemories, deleteMemory, updateMemory } from "./api";

export function MemorySection({ recalled }: { recalled: string[] }) {
  const [rows, setRows] = useState<MemoryRow[]>([]);
  const [q, setQ] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  const refresh = () => listMemories(50, 0).then(setRows).catch(() => {});
  useEffect(() => { refresh(); }, []);

  const onDelete = async (id: string) => { await deleteMemory(id); refresh(); };
  const onSave = async (id: string) => {
    await updateMemory(id, draft); setEditing(null); refresh();
  };

  const shown = rows.filter((r) => r.text.toLowerCase().includes(q.toLowerCase()));

  return (
    <div className="px-3 py-2 text-xs">
      <div className="font-semibold" style={{ color: "var(--text-strong)" }}>Memory</div>

      <div className="mt-1" style={{ color: "var(--text-muted)" }}>Recalled this turn</div>
      {recalled.length === 0 ? <div style={{ color: "var(--text-muted)" }}>— none —</div>
        : recalled.map((t, i) => <div key={i} style={{ color: "var(--text-strong)" }}>· {t}</div>)}

      <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="filter memories…"
        className="mt-2 w-full rounded px-2 py-1"
        style={{ background: "var(--surface-base)", color: "var(--text-strong)",
          border: "1px solid var(--border)" }} />

      <div className="mt-1 space-y-1">
        {shown.map((r) => (
          <div key={r.id} className="rounded p-1" style={{ border: "1px solid var(--border)" }}>
            {editing === r.id ? (
              <div className="flex gap-1">
                <input value={draft} onChange={(e) => setDraft(e.target.value)}
                  className="flex-1 rounded px-1"
                  style={{ background: "var(--surface-base)", color: "var(--text-strong)" }} />
                <button onClick={() => onSave(r.id)} style={{ color: "var(--accent)" }}>save</button>
              </div>
            ) : (
              <div className="flex items-start justify-between gap-2">
                <span style={{ color: "var(--text-strong)" }}>{r.text}</span>
                <span className="flex shrink-0 gap-2">
                  <span style={{ color: "var(--text-muted)" }}>{r.scope_kind}</span>
                  <button aria-label={`edit ${r.id}`}
                    onClick={() => { setEditing(r.id); setDraft(r.text); }}
                    style={{ color: "var(--text-muted)" }}>edit</button>
                  <button aria-label={`delete ${r.id}`} onClick={() => onDelete(r.id)}
                    style={{ color: "var(--state-error)" }}>del</button>
                </span>
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
