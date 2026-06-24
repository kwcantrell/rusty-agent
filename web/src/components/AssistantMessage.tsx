import type { Item } from "../state";
import { MarkdownText } from "./MarkdownText";

export function AssistantMessage({ item }: { item: Extract<Item, { kind: "assistant" }> }) {
  return <div className="py-2" style={{ color: "var(--text)" }}><MarkdownText text={item.text} /></div>;
}
