import type { Item } from "../state";
import { AssistantMessage } from "./AssistantMessage";
import { ToolCall } from "./ToolCall";

export function MessageList({ items }: { items: Item[] }) {
  return (
    <div className="flex-1 overflow-y-auto px-4">
      {items.map((it, i) => {
        switch (it.kind) {
          case "user":
            return <div key={i} className="my-2 ml-auto max-w-[80%] rounded bg-zinc-800 px-3 py-2 text-zinc-100">{it.text}</div>;
          case "assistant":
            return <AssistantMessage key={i} item={it} />;
          case "tool":
            return <ToolCall key={i} item={it} />;
          case "error":
            return <div key={i} className="my-2 rounded border border-red-700 bg-red-950 px-3 py-2 text-red-300">✗ {it.message}</div>;
        }
      })}
    </div>
  );
}
