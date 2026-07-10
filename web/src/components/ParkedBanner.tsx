import type { ParkedRun } from "../wire";

export function ParkedBanner(
  { runs, onDismiss }: { runs: ParkedRun[]; onDismiss: () => void },
) {
  return (
    <div role="alert" data-testid="parked-banner"
      className="flex items-center justify-between gap-3 px-4 py-2 text-sm"
      style={{ background: "var(--accent)", color: "var(--accent-fg)",
        borderBottom: "1px solid var(--border)" }}>
      <div className="flex flex-col gap-0.5">
        {runs.map((r) => (
          <span key={r.session_id}>
            Parked run {r.session_id} · {r.workspace} —{" "}
            {r.error ? r.error : `${r.asks} approval${r.asks === 1 ? "" : "s"} waiting`}
          </span>
        ))}
      </div>
      <button type="button" onClick={onDismiss} aria-label="Dismiss"
        className="shrink-0 rounded px-2 py-0.5 opacity-80 hover:opacity-100"
        style={{ border: "1px solid var(--border)" }}>
        Dismiss
      </button>
    </div>
  );
}
