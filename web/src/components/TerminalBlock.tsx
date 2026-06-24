export function TerminalBlock({ command, stdout, stderr, exitCode }: { command: string; stdout: string; stderr: string; exitCode: number }) {
  return (
    <div className="rounded text-sm" style={{ border: "1px solid var(--border)", background: "var(--surface-overlay)" }}>
      <div className="flex items-center justify-between px-2 py-1" style={{ borderBottom: "1px solid var(--border)" }}>
        <span className="font-mono" style={{ color: "var(--accent)" }}>$ {command}</span>
        <span style={{ color: exitCode === 0 ? "var(--state-done)" : "var(--state-error)" }}>exit {exitCode}</span>
      </div>
      <pre className="overflow-x-auto p-2 font-mono leading-tight" style={{ color: "var(--text)" }}>{stdout}<span style={{ color: "var(--state-error)" }}>{stderr}</span></pre>
    </div>
  );
}
