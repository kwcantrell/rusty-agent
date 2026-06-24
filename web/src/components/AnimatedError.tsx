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
      className="my-2 rounded border border-red-700 bg-red-950 px-3 py-2 text-red-300"
    >
      ✗ {item.message}
    </motion.div>
  );
}
