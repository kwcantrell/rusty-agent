import { motion } from "framer-motion";
import type { AnimatedItem } from "../state";

interface Props {
  item: Extract<AnimatedItem, { kind: "tool" }>;
  /** Inspector key for this tool's artifact, if it produced one. */
  artifactKey?: string;
  /** True when this tool's artifact is the one open in the Inspector. */
  active?: boolean;
  onSelect?: (key: string) => void;
}

// A tool call renders as a compact chip in the conversation. Rich output
// (diff/terminal/etc.) lives in the Inspector; clicking the chip focuses it.
export function AnimatedToolCall({ item, artifactKey, active, onSelect }: Props) {
  const isRunning = item.status === "running";
  const failed = !!item.resultStatus && item.resultStatus !== "ok";
  const clickable = !!artifactKey && !!onSelect;
  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, height: 0 }}
      className="my-1"
    >
      <button
        type="button"
        onClick={clickable ? () => onSelect!(artifactKey!) : undefined}
        disabled={!clickable}
        className="inline-flex items-center gap-2 rounded-md px-2 py-1 font-mono text-xs"
        style={{
          background: "var(--surface-raised)",
          border: `1px solid ${active ? "var(--accent)" : "var(--border)"}`,
          color: "var(--text)",
          cursor: clickable ? "pointer" : "default",
        }}
      >
        <span className="rounded-full px-1.5 text-[10px]"
          style={{ background: "var(--accent)", color: "var(--accent-fg)" }}>{item.name}</span>
        {isRunning ? (
          <motion.span animate={{ scale: [1, 1.15, 1] }} transition={{ repeat: Infinity, duration: 1.5 }}
            style={{ color: "var(--state-run)" }}>…</motion.span>
        ) : (
          <span style={{ color: "var(--state-done)" }}>✓</span>
        )}
        {failed && (
          <span className="rounded-full px-1.5 text-[10px]"
            style={{ border: "1px solid var(--state-error)", color: "var(--state-error)" }}>
            {item.resultStatus} · {item.durationMs}ms
          </span>
        )}
        {clickable && <span style={{ color: "var(--accent)" }}>{active ? "viewing →" : "view →"}</span>}
      </button>
    </motion.div>
  );
}
