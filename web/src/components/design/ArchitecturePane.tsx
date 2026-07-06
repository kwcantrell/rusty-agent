import { useCallback, useEffect, useRef, useState } from "react";
import { fetchArchitecture, type ArchitectureSnapshot, type BlockId } from "./architecture";
import { ArchDiagram } from "./ArchDiagram";
import { ArchDetail } from "./ArchDetail";

/** Self-contained: fetches on mount; staleness is the enemy, so no caching. */
export function ArchitecturePane() {
  const [snapshot, setSnapshot] = useState<ArchitectureSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<BlockId>("loop");

  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; };
  }, []);
  const load = useCallback(() => {
    setError(null);
    setSnapshot(null);
    fetchArchitecture()
      .then((s) => { if (alive.current) setSnapshot(s); })
      .catch((e) => { if (alive.current) setError(String(e)); });
  }, []);
  useEffect(() => { load(); }, [load]);

  if (error) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-sm"
        style={{ color: "var(--text-muted)" }}>
        <p>Could not read the runtime architecture: {error}</p>
        <button onClick={load} className="rounded px-3 py-1 text-xs"
          style={{ background: "var(--accent)", color: "var(--accent-fg)" }}>Retry</button>
      </div>
    );
  }
  if (!snapshot) {
    return <p className="p-4 text-sm" style={{ color: "var(--text-muted)" }}>Loading architecture…</p>;
  }
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center justify-end px-3 pt-2">
        <button onClick={load} className="rounded px-2 py-0.5 text-xs"
          style={{ color: "var(--text-muted)", border: "1px solid var(--border)" }}>Refresh</button>
      </div>
      <ArchDiagram snapshot={snapshot} selected={selected} onSelect={setSelected} />
      <div className="min-h-0 flex-1" style={{ borderTop: "1px solid var(--border)" }}>
        <ArchDetail snapshot={snapshot} block={selected} />
      </div>
    </div>
  );
}
