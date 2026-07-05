import { useRef, useState } from "react";

const MAX_ROWS = 6;
const ROW_PX = 22; // 13px * 1.65 line-height, rounded

// Claude Code-style prompt box: bordered, `>` prefix, Enter sends,
// Shift+Enter newlines, ↑/↓ walk the persisted prompt history.
export function Composer({ disabled, onSend, history }:
  { disabled: boolean; onSend: (text: string) => void; history: () => string[] }) {
  const [text, setText] = useState("");
  // null = editing a fresh draft; otherwise an index into history().
  const cursor = useRef<number | null>(null);
  const draft = useRef("");

  const submit = () => {
    const t = text.trim();
    if (!t || disabled) return;
    onSend(t);
    setText("");
    cursor.current = null;
  };

  const autogrow = (ta: HTMLTextAreaElement) => {
    ta.style.height = "auto";
    ta.style.height = `${Math.min(ta.scrollHeight, MAX_ROWS * ROW_PX)}px`;
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); submit(); return; }
    const ta = e.currentTarget;
    if (e.key === "ArrowUp" && !ta.value.slice(0, ta.selectionStart).includes("\n")) {
      const h = history();
      if (h.length === 0) return;
      e.preventDefault();
      if (cursor.current === null) { draft.current = text; cursor.current = h.length - 1; }
      else if (cursor.current > 0) { cursor.current -= 1; }
      setText(h[cursor.current]);
    } else if (e.key === "ArrowDown" && !ta.value.slice(ta.selectionEnd).includes("\n")) {
      if (cursor.current === null) return;
      const h = history();
      e.preventDefault();
      if (cursor.current < h.length - 1) { cursor.current += 1; setText(h[cursor.current]); }
      else { cursor.current = null; setText(draft.current); }
    }
  };

  return (
    <div className="p-3" style={{ borderTop: "1px solid var(--cli-border)" }}>
      <div className="cli-promptbox flex items-start gap-2 rounded-md px-3 py-2"
        style={{ border: "1px solid var(--cli-border)", opacity: disabled ? 0.5 : 1 }}>
        <span aria-hidden style={{ color: "var(--cli-dim)" }}>&gt;</span>
        <textarea
          aria-label="prompt"
          className="flex-1 resize-none bg-transparent outline-none disabled:opacity-50"
          style={{ color: "var(--cli-text)", font: "inherit", height: `${ROW_PX}px` }}
          rows={1}
          value={text}
          disabled={disabled}
          onChange={(e) => { setText(e.target.value); cursor.current = null; autogrow(e.target); }}
          onKeyDown={onKeyDown}
          placeholder={disabled ? "disconnected…" : ""}
        />
      </div>
    </div>
  );
}
