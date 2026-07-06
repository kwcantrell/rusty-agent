export function VersionBar({ current, total, compare, renderableFlags, onSelect, onLatest, onToggleCompare }: {
  current: number; total: number; compare: boolean; renderableFlags: boolean[];
  onSelect: (i: number) => void; onLatest: () => void; onToggleCompare: () => void;
}) {
  const btn = "rounded px-2 py-0.5 text-xs disabled:opacity-40 disabled:cursor-not-allowed";
  return (
    <div className="flex items-center gap-2 px-3 py-2" style={{ borderBottom: "1px solid var(--border)" }}>
      <button aria-label="previous version" className={btn} disabled={current === 0}
        onClick={() => onSelect(current - 1)} style={{ color: "var(--text-muted)" }}>←</button>
      <span className="text-xs" style={{ color: "var(--text-strong)" }}>
        v{current + 1} / {total}{renderableFlags[current] ? "" : " (unsupported)"}
      </span>
      <button aria-label="next version" className={btn} disabled={current >= total - 1}
        onClick={() => onSelect(current + 1)} style={{ color: "var(--text-muted)" }}>→</button>
      <button className={btn} disabled={current >= total - 1} onClick={onLatest}
        style={{ color: "var(--text-muted)" }}>latest</button>
      <div className="flex-1" />
      <button aria-pressed={compare} className={btn} disabled={total < 2} onClick={onToggleCompare}
        style={{ background: compare ? "var(--accent)" : "transparent",
          color: compare ? "var(--accent-fg)" : "var(--text-muted)",
          border: "1px solid var(--border)" }}>Compare</button>
    </div>
  );
}
