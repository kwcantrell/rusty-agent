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
    <div className="flex gap-2 border-t border-zinc-800 bg-zinc-950 p-3">
      <textarea
        className="flex-1 resize-none rounded bg-zinc-900 p-2 text-zinc-100 outline-none disabled:opacity-50"
        rows={2}
        value={text}
        disabled={disabled}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); submit(); } }}
        placeholder={disabled ? "disconnected…" : "Message the agent…"}
      />
      <button onClick={submit} disabled={disabled} className="rounded bg-zinc-700 px-4 text-zinc-100 hover:bg-zinc-600 disabled:opacity-50">Send</button>
    </div>
  );
}
