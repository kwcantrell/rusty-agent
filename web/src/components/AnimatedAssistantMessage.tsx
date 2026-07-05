import { motion } from "framer-motion";
import { useStreamingText } from "../hooks/useStreamingText";
import { MarkdownText } from "./MarkdownText";
import type { AnimatedItem } from "../state";

interface Props {
  item: Extract<AnimatedItem, { kind: "assistant" }>;
}

export function AnimatedAssistantMessage({ item }: Props) {
  const streaming = item.streaming && item.done === undefined;
  const visibleText = useStreamingText(item.text, streaming);

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="flex gap-2 py-1.5"
      style={{ color: "var(--cli-text)" }}
    >
      <span aria-hidden>⏺</span>
      <div className="min-w-0 flex-1 whitespace-pre-wrap">
        <MarkdownText text={visibleText} />
        {streaming && <span className="inline-block h-4 w-[1ch] animate-pulse" style={{ color: "var(--cli-accent)" }}>|</span>}
      </div>
    </motion.div>
  );
}
