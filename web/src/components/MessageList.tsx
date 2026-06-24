import type { AnimatedItem } from "../state";
import { AnimatedAssistantMessage } from "./AnimatedAssistantMessage";
import { AnimatedReasoningMessage } from "./AnimatedReasoningMessage";
import { AnimatedToolCall } from "./AnimatedToolCall";
import { AnimatedError } from "./AnimatedError";

export function MessageList({ items }: { items: AnimatedItem[] }) {
  return (
    <div className="flex-1 overflow-y-auto px-4">
      {items.map((it, i) => {
        switch (it.kind) {
          case "user":
            return <div key={i} className="my-2 ml-auto max-w-[80%] rounded bg-zinc-800 px-3 py-2 text-zinc-100">{it.text}</div>;
          case "assistant":
            return <AnimatedAssistantMessage key={i} item={it} />;
          case "reasoning":
            return <AnimatedReasoningMessage key={i} item={it} />;
          case "tool":
            return <AnimatedToolCall key={i} item={it} />;
          case "error":
            return <AnimatedError key={i} item={it} />;
        }
      })}
    </div>
  );
}
