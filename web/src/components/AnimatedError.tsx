import { motion } from "framer-motion";
import type { AnimatedItem } from "../state";

interface Props {
  item: Extract<AnimatedItem, { kind: "error" }>;
}

export function AnimatedError({ item }: Props) {
  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="my-1.5 flex gap-2"
      style={{ color: "var(--cli-err)" }}
    >
      <span aria-hidden>⏺</span>
      <div className="min-w-0 flex-1 whitespace-pre-wrap">{item.message}</div>
    </motion.div>
  );
}
