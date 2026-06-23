import { useState } from "react";

export function ReasoningMessage({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="my-2 max-w-[80%] rounded border border-zinc-700 bg-zinc-900/60 px-3 py-2 text-xs text-zinc-400">
      <button onClick={() => setOpen((o) => !o)} className="font-medium text-zinc-300">
        {open ? "▾" : "▸"} Thinking
      </button>
      {open && <pre className="mt-1 whitespace-pre-wrap break-words">{text}</pre>}
    </div>
  );
}
