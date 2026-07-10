import { useEffect, useState } from "react";
import type { Decision } from "../wire";
import type { PendingApproval } from "../state";

const OPTIONS: { key: string; label: string; decision: Decision }[] = [
  { key: "1", label: "Yes", decision: "approve" },
  { key: "2", label: "Yes, don't ask again", decision: "approve_always" },
  { key: "3", label: "No", decision: "deny" },
];

// Claude Code-style permission box: numbered plain-text options, answerable
// with the 1/2/3 keys. Keystrokes originating in the composer (or any other
// text field, including the feedback input below) are ignored so typing
// digits never answers the approval.
export function ApprovalPrompt({ approval, onDecide }: { approval: PendingApproval; onDecide: (d: Decision) => void }) {
  const [feedback, setFeedback] = useState("");
  const decide = (d: Decision) =>
    onDecide(d === "deny" && feedback.trim() ? { deny: { feedback: feedback.trim() } } : d);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target ?? document.activeElement;
      if (t instanceof HTMLElement && (t.tagName === "TEXTAREA" || t.tagName === "INPUT" || t.isContentEditable)) return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const opt = OPTIONS.find((o) => o.key === e.key);
      if (opt) decide(opt.decision);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [decide]);

  return (
    <div className="mx-4 my-2 rounded-md p-3" style={{ border: "1px solid var(--cli-border)" }}>
      {approval.origin && (
        <div className="mb-1" style={{ color: "var(--cli-accent)" }}>
          Sub-agent <b>{approval.origin.subagent}</b>
          {approval.origin.depth > 1 ? ` (depth ${approval.origin.depth})` : ""} wants to run:
        </div>
      )}
      <div className="mb-2" style={{ color: "var(--cli-text)" }}>Allow: {approval.summary}</div>
      {approval.command && (
        <pre className="mb-2 overflow-x-auto" style={{ color: "var(--cli-accent)" }}>{approval.command}</pre>
      )}
      <input
        value={feedback}
        onChange={(e) => setFeedback(e.target.value)}
        placeholder="optional feedback if denying"
        className="mb-1 w-full rounded px-2 py-1 text-sm"
        style={{ background: "var(--surface-raised)", border: "1px solid var(--border)", color: "var(--cli-text)" }}
      />
      <div className="flex flex-wrap gap-x-8 gap-y-1">
        {OPTIONS.map((o) => (
          <button key={o.key} type="button" onClick={() => decide(o.decision)}
            className="text-left hover:underline" style={{ color: "var(--cli-text)" }}>
            <span style={{ color: "var(--cli-dim)" }}>{o.key}.</span> {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}
