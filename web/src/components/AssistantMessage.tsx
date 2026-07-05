import type { Item } from "../state";
import { MarkdownText } from "./MarkdownText";

export function AssistantMessage({ item }: { item: Extract<Item, { kind: "assistant" }> }) {
  return (
    <div className="flex gap-2 py-1.5" style={{ color: "var(--cli-text)" }}>
      <span aria-hidden>⏺</span>
      <div className="min-w-0 flex-1"><MarkdownText text={item.text} /></div>
    </div>
  );
}
