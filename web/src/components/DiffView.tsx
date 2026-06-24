import { diffLines } from "diff";

export function DiffView({ path, before, after }: { path: string; before: string; after: string }) {
  const parts = diffLines(before, after);
  return (
    <div className="rounded text-sm" style={{ border: "1px solid var(--border)", background: "var(--surface-overlay)" }}>
      <div className="px-2 py-1 font-mono" style={{ borderBottom: "1px solid var(--border)", color: "var(--accent-2)" }}>{path}</div>
      <pre className="overflow-x-auto p-2 font-mono leading-tight">
        {parts.flatMap((part, pi) => {
          const sign = part.added ? "+" : part.removed ? "-" : " ";
          const color = part.added ? "var(--state-done)" : part.removed ? "var(--state-error)" : "var(--text-muted)";
          return part.value.replace(/\n$/, "").split("\n").map((line, li) => (
            <div key={`${pi}-${li}`} style={{ color }}>{sign} {line}</div>
          ));
        })}
      </pre>
    </div>
  );
}
