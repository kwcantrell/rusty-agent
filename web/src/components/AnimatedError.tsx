import { motion } from "framer-motion";
import type { AnimatedItem } from "../state";

interface Props {
  item: Extract<AnimatedItem, { kind: "error" }>;
}

export function AnimatedError({ item }: Props) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, height: 0 }}
      className="my-2 rounded-lg px-3 py-2"
      style={{ border: "1px solid var(--state-error)", background: "var(--surface-raised)", color: "var(--state-error)" }}
    >
      ✗ {item.message}
    </motion.div>
  );
}
