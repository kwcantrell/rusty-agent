import { useRef, useState, type ReactNode, type MouseEvent } from "react";
import type { Pin } from "../../designStore";

/**
 * Click-to-pin layer over a rendered artifact. Drafts are local until "Send
 * feedback"; pct coordinates are relative to the artifact box so they survive
 * pane resizes. The layer sits ABOVE iframe-hosted HTML, so no cross-frame
 * event wrangling (and mockups are intentionally non-interactive).
 */
export function AnnotationOverlay({ children, sent, disabled, onSend, passthrough = false }: {
  children: ReactNode; sent: Pin[]; disabled: boolean; onSend: (pins: Pin[]) => void;
  passthrough?: boolean;
}) {
  const [drafts, setDrafts] = useState<Pin[]>([]);
  const box = useRef<HTMLDivElement>(null);

  const addPin = (e: MouseEvent) => {
    const r = box.current?.getBoundingClientRect();
    if (!r || r.width === 0 || r.height === 0) return;
    const x = Math.round(((e.clientX - r.left) / r.width) * 1000) / 1000;
    const y = Math.round(((e.clientY - r.top) / r.height) * 1000) / 1000;
    setDrafts((d) => [...d, { x_pct: x, y_pct: y, comment: "" }]);
  };
  const setComment = (i: number, comment: string) =>
    setDrafts((d) => d.map((p, j) => (j === i ? { ...p, comment } : p)));
  const remove = (i: number) => setDrafts((d) => d.filter((_, j) => j !== i));
  const ready = drafts.filter((p) => p.comment.trim().length > 0);
  const send = () => { onSend(ready); setDrafts([]); };

  return (
    <div className="flex h-full flex-col">
      <div ref={box} className="relative min-h-0 flex-1">
        {children}
        <div data-testid="pin-layer" className="absolute inset-0 cursor-crosshair" onClick={addPin}
          style={passthrough ? { pointerEvents: "none" } : undefined}>
          {sent.map((p, i) => <Marker key={`s${i}`} pin={p} kind="sent" n={i + 1} />)}
          {drafts.map((p, i) => <Marker key={`d${i}`} pin={p} kind="draft" n={sent.length + i + 1} />)}
        </div>
      </div>
      <div className="space-y-1 p-2" style={{ borderTop: "1px solid var(--border)" }}>
        {drafts.map((p, i) => (
          <div key={i} className="flex items-center gap-2">
            <span className="text-xs" style={{ color: "var(--text-muted)" }}>#{sent.length + i + 1}</span>
            <input aria-label={`pin ${sent.length + i + 1} comment`} value={p.comment}
              placeholder="what should change here?"
              className="min-w-0 flex-1 rounded px-2 py-1 text-xs"
              style={{ background: "var(--surface-base)", color: "var(--text-strong)",
                border: "1px solid var(--border)" }}
              onChange={(e) => setComment(i, e.target.value)} />
            <button aria-label={`delete pin ${sent.length + i + 1}`} onClick={() => remove(i)}
              className="text-xs" style={{ color: "var(--text-muted)" }}>✕</button>
          </div>
        ))}
        <button onClick={send} disabled={disabled || ready.length === 0}
          className="w-full rounded px-3 py-1.5 text-xs font-medium hover:opacity-90 disabled:opacity-40"
          style={{ background: "var(--accent)", color: "var(--accent-fg)" }}>
          Send feedback{ready.length > 0 ? ` (${ready.length})` : ""}
        </button>
      </div>
    </div>
  );
}

function Marker({ pin, kind, n }: { pin: Pin; kind: "sent" | "draft"; n: number }) {
  return (
    <span data-testid={`pin-${kind}`} onClick={(e) => e.stopPropagation()} title={pin.comment}
      className="absolute flex h-5 w-5 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full text-[10px] font-bold"
      style={{ left: `${pin.x_pct * 100}%`, top: `${pin.y_pct * 100}%`,
        background: kind === "draft" ? "var(--accent)" : "var(--surface-raised)",
        color: kind === "draft" ? "var(--accent-fg)" : "var(--text-muted)",
        border: "1px solid var(--border)" }}>{n}</span>
  );
}
