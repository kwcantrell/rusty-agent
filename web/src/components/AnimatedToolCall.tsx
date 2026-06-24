import { motion } from "framer-motion";
import { useState } from "react";
import { DiffView } from "./DiffView";
import { TerminalBlock } from "./TerminalBlock";
import type { AnimatedItem } from "../state";

interface Props {
  item: Extract<AnimatedItem, { kind: "tool" }>;
}

export function AnimatedToolCall({ item }: Props) {
  const [expanded, setExpanded] = useState(true);
  const isRunning = item.status === "running";
  const statusIcon = isRunning ? "…" : "✓";
  const d = item.display;
  const hasKnownDisplay = !!d && ("Diff" in d || "Terminal" in d || "Text" in d);

  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, height: 0 }}
      className="my-2"
    >
      <div
        className="flex cursor-pointer items-center gap-2 font-mono text-cyan-400 hover:text-cyan-300"
        onClick={() => setExpanded((e) => !e)}
      >
        <span>⚙ {item.name}</span>
        <span className="inline-flex items-center justify-center">
          {isRunning && (
            <motion.span
              animate={{ scale: [1, 1.15, 1] }}
              transition={{ repeat: Infinity, duration: 1.5 }}
              className="text-cyan-400"
            >
              {statusIcon}
            </motion.span>
          )}
          {!isRunning && <span className="text-green-400">{statusIcon}</span>}
        </span>
        {!isRunning && <span className="text-xs text-zinc-500">{expanded ? "▾" : "▸"}</span>}
      </div>

      {expanded && !isRunning && (
        <motion.div
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          transition={{ duration: 0.15 }}
        >
          {d && "Diff" in d && <DiffView path={d.Diff.path} before={d.Diff.before} after={d.Diff.after} />}
          {d && "Terminal" in d && (
            <TerminalBlock
              command={d.Terminal.command}
              stdout={d.Terminal.stdout}
              stderr={d.Terminal.stderr}
              exitCode={d.Terminal.exit_code}
            />
          )}
          {d && "Text" in d && <pre className="whitespace-pre-wrap p-2 font-mono text-sm text-zinc-300">{d.Text}</pre>}
          {!hasKnownDisplay && item.content && (
            <pre className="whitespace-pre-wrap p-2 font-mono text-sm text-zinc-400">{item.content}</pre>
          )}
        </motion.div>
      )}

      {isRunning && (
        <motion.div
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          transition={{ duration: 0.15 }}
          className="text-xs text-zinc-500"
        >
          running…
        </motion.div>
      )}
    </motion.div>
  );
}
