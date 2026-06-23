import type { Item } from "../state";

export function AssistantMessage({ item }: { item: Extract<Item, { kind: "assistant" }> }) {
  return <div className="whitespace-pre-wrap py-2 text-zinc-100">{item.text}</div>;
}
