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
  const [cardOpen, setCardOpen] = useState(true);
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
      {item.subagent && (
        <div className="mt-1 pl-5" data-testid="subagent-card">
          <div className="flex items-baseline gap-2">
            <span className="rounded px-1 text-xs"
              style={{ background: "var(--cli-accent)", color: "var(--cli-bg, #000)" }}>
              agent[{item.subagent.subagentType}]
            </span>
            {(() => {
              const waiting = item.subagent.status === "running" && item.subagent.waitingApproval;
              return (
                <span className="text-xs" style={{
                  color: item.subagent.status === "running" ? "var(--cli-accent)"
                    : item.subagent.outcome === "completed" || item.subagent.outcome === undefined
                      ? "var(--cli-ok)" : "var(--cli-err)" }}>
                  {waiting ? "waiting approval"
                    : item.subagent.status === "running" ? "running" : (item.subagent.outcome ?? "done")}
                </span>
              );
            })()}
            {(item.subagent.text || item.subagent.reasoning) && (
              <button type="button" onClick={() => setCardOpen((o) => !o)}
                style={{ color: "var(--cli-dim)" }} className="text-xs">
                {cardOpen ? "hide ▴" : "transcript ▾"}
              </button>
            )}
          </div>
          {cardOpen && (
            <pre data-testid="subagent-transcript"
              className="mt-1 max-h-64 overflow-y-auto whitespace-pre-wrap text-xs">
              {item.subagent.textElided > 0 && (
                <span style={{ color: "var(--cli-dim)" }}>
                  …({item.subagent.textElided} chars elided){"\n"}
                </span>
              )}
              {item.subagent.reasoning && (
                <span style={{ color: "var(--cli-dim)" }}>{item.subagent.reasoning}{"\n"}</span>
              )}
              <span style={{ color: "var(--cli-text)" }}>{item.subagent.text}</span>
            </pre>
          )}
          {item.subagent.status === "done" && (
            <div className="text-xs" style={{ color: "var(--cli-dim)" }}>
              {item.subagent.detail && <span style={{ color: "var(--cli-err)" }}>{item.subagent.detail} · </span>}
              {item.subagent.stop && `${item.subagent.stop} · `}
              {item.subagent.turns !== undefined && `${item.subagent.turns} turns · `}
              {item.subagent.toolCalls !== undefined && `${item.subagent.toolCalls} tools · `}
              {item.subagent.durationMs !== undefined && `${(item.subagent.durationMs / 1000).toFixed(1)}s`}
              {item.subagent.promptTokens > 0 &&
                ` · ${item.subagent.promptTokens + item.subagent.completionTokens} tok`}
              {item.subagent.costUsd > 0 && ` · $${item.subagent.costUsd.toFixed(4)}`}
            </div>
          )}
        </div>
      )}
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
