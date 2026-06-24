export function AgentHeader({ projectLabel, model }: { projectLabel: string; model?: string }) {
  return (
    <div className="px-4 pb-3 pt-4" style={{ borderBottom: "1px solid var(--border)" }}>
      <div className="font-display text-xl leading-tight" style={{ color: "var(--text-strong)" }}>{projectLabel}</div>
      {model && (
        <div className="mt-0.5 font-mono text-xs" style={{ color: "var(--text-muted)" }}>model {model}</div>
      )}
    </div>
  );
}
