import type { AnimatedItem } from "../state";
import { AnimatedAssistantMessage } from "./AnimatedAssistantMessage";
import { AnimatedReasoningMessage } from "./AnimatedReasoningMessage";
import { AnimatedToolCall } from "./AnimatedToolCall";
import { AnimatedError } from "./AnimatedError";

export function MessageList({ items, activeArtifactKey, onSelectArtifact }:
  { items: AnimatedItem[]; activeArtifactKey?: string | null; onSelectArtifact?: (key: string) => void }) {
  return (
    <div className="flex-1 overflow-y-auto px-4">
      {items.map((it, i) => {
        switch (it.kind) {
          case "user":
            return <div key={i} className="my-2 ml-auto max-w-[80%] rounded-2xl px-4 py-2"
              style={{ background: "var(--text-strong)", color: "var(--surface-base)" }}>{it.text}</div>;
          case "assistant":
            return <AnimatedAssistantMessage key={i} item={it} />;
          case "reasoning":
            return <AnimatedReasoningMessage key={i} item={it} />;
          case "tool": {
            const artifactKey = it.display ? `art-${i}` : undefined;
            return <AnimatedToolCall key={i} item={it} artifactKey={artifactKey}
              active={!!artifactKey && artifactKey === activeArtifactKey} onSelect={onSelectArtifact} />;
          }
          case "error":
            return <AnimatedError key={i} item={it} />;
        }
      })}
    </div>
  );
}
