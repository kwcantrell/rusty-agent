import { useState } from "react";

export function Composer({ disabled, onSend }: { disabled: boolean; onSend: (text: string) => void }) {
  const [text, setText] = useState("");
  const submit = () => {
    const t = text.trim();
    if (!t || disabled) return;
    onSend(t);
    setText("");
  };
  return (
    <div className="flex gap-2 p-3" style={{ background: "var(--surface-base)", borderTop: "1px solid var(--border)" }}>
      <textarea
        className="flex-1 resize-none rounded-lg p-2 outline-none disabled:opacity-50"
        style={{ background: "var(--surface-overlay)", color: "var(--text-strong)", border: "1px solid var(--border)" }}
        rows={2}
        value={text}
        disabled={disabled}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); submit(); } }}
        placeholder={disabled ? "disconnected…" : "Message the agent…"}
      />
      <button onClick={submit} disabled={disabled}
        className="rounded-lg px-4 disabled:opacity-50 hover:opacity-90"
        style={{ background: "var(--accent)", color: "var(--accent-fg)" }}>Send</button>
    </div>
  );
}
