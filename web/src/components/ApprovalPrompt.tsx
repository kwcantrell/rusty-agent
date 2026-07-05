import { useEffect } from "react";
import type { Decision } from "../wire";
import type { PendingApproval } from "../state";

const OPTIONS: { key: string; label: string; decision: Decision }[] = [
  { key: "1", label: "Yes", decision: "approve" },
  { key: "2", label: "Yes, don't ask again", decision: "approve_always" },
  { key: "3", label: "No", decision: "deny" },
];

// Claude Code-style permission box: numbered plain-text options, answerable
// with the 1/2/3 keys. Keystrokes originating in the composer (or any other
// text field) are ignored so typing digits never answers the approval.
export function ApprovalPrompt({ approval, onDecide }: { approval: PendingApproval; onDecide: (d: Decision) => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target;
      if (t instanceof HTMLElement && (t.tagName === "TEXTAREA" || t.tagName === "INPUT" || t.isContentEditable)) return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const opt = OPTIONS.find((o) => o.key === e.key);
      if (opt) onDecide(opt.decision);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onDecide]);

  return (
    <div className="mx-4 my-2 rounded-md p-3" style={{ border: "1px solid var(--cli-border)" }}>
      <div className="mb-2" style={{ color: "var(--cli-text)" }}>Allow: {approval.summary}</div>
      {approval.command && (
        <pre className="mb-2 overflow-x-auto" style={{ color: "var(--cli-accent)" }}>{approval.command}</pre>
      )}
      <div className="flex flex-wrap gap-x-8 gap-y-1">
        {OPTIONS.map((o) => (
          <button key={o.key} type="button" onClick={() => onDecide(o.decision)}
            className="text-left hover:underline" style={{ color: "var(--cli-text)" }}>
            <span style={{ color: "var(--cli-dim)" }}>{o.key}.</span> {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}
