export function WorkspaceEmptyState() {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 px-8 text-center"
      style={{ color: "var(--text-muted)" }}>
      <div className="font-display text-2xl" style={{ color: "var(--text-strong)" }}>Workspace</div>
      <div className="text-sm">Rendered output from the agent will appear here.</div>
    </div>
  );
}
