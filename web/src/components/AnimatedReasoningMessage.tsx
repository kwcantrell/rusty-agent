import { useState } from "react";
import { motion } from "framer-motion";
import { useStreamingText } from "../hooks/useStreamingText";
import { MarkdownText } from "./MarkdownText";
import type { AnimatedItem } from "../state";

interface Props {
  item: Extract<AnimatedItem, { kind: "reasoning" }>;
}

export function AnimatedReasoningMessage({ item }: Props) {
  const [open, setOpen] = useState(false);
  const streaming = item.streaming;
  const visibleText = useStreamingText(item.text, streaming);

  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, height: 0 }}
      className="my-2 max-w-[80%] rounded border border-zinc-700 bg-zinc-900/60 px-3 py-2 text-xs"
    >
      <button onClick={() => setOpen((o) => !o)} className="mb-1 font-medium text-zinc-300">
        {open ? "▾" : "▸"} Thinking
      </button>
      {open && (
        <motion.div
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          transition={{ duration: 0.15 }}
        >
          <MarkdownText text={visibleText} />
          {streaming && <span className="inline-block h-4 w-[1ch] animate-pulse text-cyan-400">|</span>}
        </motion.div>
      )}
    </motion.div>
  );
}
