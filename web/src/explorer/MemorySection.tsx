export function MemorySection({ recalled }: { recalled: string[] }) {
  return (
    <div className="px-3 py-2 text-xs" style={{ color: "var(--text-muted)" }}>
      <div className="font-semibold" style={{ color: "var(--text-strong)" }}>Memory</div>
      {recalled.length === 0 ? <div>No recall this turn.</div>
        : recalled.map((t, i) => <div key={i}>· {t}</div>)}
    </div>
  );
}
