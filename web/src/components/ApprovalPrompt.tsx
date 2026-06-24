import type { Decision } from "../wire";
import type { PendingApproval } from "../state";

export function ApprovalPrompt({ approval, onDecide }: { approval: PendingApproval; onDecide: (d: Decision) => void }) {
  return (
    <div className="mx-4 my-2 rounded-lg p-3 text-sm"
      style={{ border: "1px solid var(--accent-2)", background: "var(--surface-raised)" }}>
      <div className="mb-2" style={{ color: "var(--text-strong)" }}>Allow: {approval.summary}</div>
      {approval.command && (
        <pre className="mb-2 overflow-x-auto font-mono" style={{ color: "var(--accent-2)" }}>{approval.command}</pre>
      )}
      <div className="flex gap-2">
        <button onClick={() => onDecide("approve")} className="rounded px-3 py-1 hover:opacity-90"
          style={{ background: "var(--state-done)", color: "var(--accent-fg)" }}>Approve</button>
        <button onClick={() => onDecide("approve_always")} className="rounded px-3 py-1 hover:opacity-90"
          style={{ background: "var(--surface-overlay)", color: "var(--text)", border: "1px solid var(--border)" }}>Approve always</button>
        <button onClick={() => onDecide("deny")} className="rounded px-3 py-1 hover:opacity-90"
          style={{ background: "var(--state-error)", color: "#fff" }}>Deny</button>
      </div>
    </div>
  );
}
