// One-time transcript-top banner (replaces the old sticky AgentHeader).
export function SessionBanner({ projectLabel, model }: { projectLabel: string; model?: string }) {
  return (
    <div className="mx-4 my-3 rounded-md px-3 py-2"
      style={{ border: "1px solid var(--cli-border)", color: "var(--cli-dim)" }}>
      <span style={{ color: "var(--cli-accent)" }}>✻</span> {projectLabel}{model ? ` · ${model}` : ""}
    </div>
  );
}
