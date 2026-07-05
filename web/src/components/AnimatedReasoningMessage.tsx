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
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="my-1.5 italic"
      style={{ color: "var(--cli-dim)" }}
    >
      <button onClick={() => setOpen((o) => !o)} style={{ color: "var(--cli-dim)" }}>
        ✻ Thinking… {open ? "▾" : "▸"}
      </button>
      {open && (
        <motion.div
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          transition={{ duration: 0.15 }}
          className="pl-5"
        >
          <MarkdownText text={visibleText} />
          {streaming && <span className="inline-block h-4 w-[1ch] animate-pulse" style={{ color: "var(--cli-accent)" }}>|</span>}
        </motion.div>
      )}
    </motion.div>
  );
}
