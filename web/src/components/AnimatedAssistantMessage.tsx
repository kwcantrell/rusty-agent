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
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, height: 0 }}
      className="whitespace-pre-wrap py-2 text-zinc-100"
    >
      <MarkdownText text={visibleText} />
      {streaming && <span className="inline-block h-4 w-[1ch] animate-pulse text-cyan-400">|</span>}
    </motion.div>
  );
}
