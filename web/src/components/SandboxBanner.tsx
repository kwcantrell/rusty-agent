export function SandboxBanner(
  { info, onDismiss }: { info: { mechanism: string; reason: string }; onDismiss: () => void },
) {
  return (
    <div role="alert"
      className="flex items-center justify-between gap-3 px-4 py-2 text-sm"
      style={{ background: "var(--warning-surface, #3a2f00)", color: "var(--warning-text, #ffd24a)",
        borderBottom: "1px solid var(--border)" }}>
      <span>
        ⚠ <strong>Sandbox degraded</strong> — tools run unsandboxed on the host
        {" "}({info.mechanism}: {info.reason}).
      </span>
      <button type="button" onClick={onDismiss} aria-label="Dismiss"
        className="shrink-0 rounded px-2 py-0.5 opacity-80 hover:opacity-100"
        style={{ border: "1px solid var(--border)" }}>
        Dismiss
      </button>
    </div>
  );
}
