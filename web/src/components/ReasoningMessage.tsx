import { useState } from "react";

export function ReasoningMessage({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="my-2 max-w-[80%] rounded-lg px-3 py-2 text-xs"
      style={{ border: "1px solid var(--border)", background: "var(--surface-raised)", color: "var(--text-muted)" }}>
      <button onClick={() => setOpen((o) => !o)} className="font-medium" style={{ color: "var(--text)" }}>
        {open ? "▾" : "▸"} Thinking
      </button>
      {open && <pre className="mt-1 whitespace-pre-wrap break-words">{text}</pre>}
    </div>
  );
}
