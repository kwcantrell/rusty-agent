import { useState } from "react";
import { motion } from "framer-motion";
import type { AnimatedItem } from "../state";
import { argSummary, resultSummary } from "./cliFormat";

interface Props {
  item: Extract<AnimatedItem, { kind: "tool" }>;
  /** Inspector key for this tool's artifact, if it produced one. */
  artifactKey?: string;
  /** True when this tool's artifact is the one open in the Inspector. */
  active?: boolean;
  onSelect?: (key: string) => void;
}

const EXPAND_LINES = 20;

// A tool call renders as a Claude Code-style transcript group:
//   ⏺ Name(arg-summary)
//     ⎿ result-summary        view →
// Clicking the ⎿ line toggles a raw-content preview (≤20 lines). `view →`
// focuses the Inspector artifact when the tool produced a display.
export function AnimatedToolCall({ item, artifactKey, active, onSelect }: Props) {
  const [expanded, setExpanded] = useState(false);
  const isRunning = item.status === "running";
  const failed = !!item.resultStatus && item.resultStatus !== "ok";
  const clickable = !!artifactKey && !!onSelect;
  // Attributed sub-agent tool rows nest under their dispatch parent: indent,
  // prefix a ↳, and strip the `sub:` display prefix from the tool name.
  const nested = !!item.parentId;
  const displayName = nested && item.name.startsWith("sub:") ? item.name.slice(4) : item.name;
  const arg = argSummary(item.args);
  const dot = isRunning ? "var(--cli-accent)" : failed ? "var(--cli-err)" : "var(--cli-ok)";
  const preview = (item.content ?? "").split("\n").slice(0, EXPAND_LINES).join("\n");

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="my-1.5"
      style={{ marginLeft: nested ? "1.25rem" : undefined }}
    >
      <div className="flex items-baseline gap-2">
        {nested && <span style={{ color: "var(--cli-dim)" }}>↳</span>}
        {isRunning ? (
          <motion.span animate={{ opacity: [1, 0.3, 1] }} transition={{ repeat: Infinity, duration: 1.2 }}
            style={{ color: dot }}>⏺</motion.span>
        ) : (
          <span style={{ color: dot }}>⏺</span>
        )}
        <span style={{ color: "var(--cli-text)" }}>
          {displayName}
          {arg && <span style={{ color: "var(--cli-dim)" }}>({arg})</span>}
        </span>
      </div>
      {!isRunning && (
        <div className="flex items-baseline gap-2 pl-5">
          <button type="button" onClick={() => setExpanded((e) => !e)} className="text-left"
            style={{ color: failed ? "var(--cli-err)" : "var(--cli-dim)" }}>
            ⎿ {resultSummary(item.content, item.resultStatus)}
            {failed && ` · ${item.resultStatus} · ${item.durationMs}ms`}
          </button>
          {clickable && (
            <button type="button" onClick={() => onSelect!(artifactKey!)}
              style={{ color: "var(--cli-accent)" }}>
              {active ? "viewing →" : "view →"}
            </button>
          )}
        </div>
      )}
      {expanded && preview && (
        <pre className="mt-1 overflow-x-auto whitespace-pre-wrap pl-5"
          style={{ color: "var(--cli-dim)" }}>{preview}</pre>
      )}
    </motion.div>
  );
}
