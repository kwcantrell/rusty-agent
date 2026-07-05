import type { AnimatedItem } from "../state";
import { AnimatedAssistantMessage } from "./AnimatedAssistantMessage";
import { AnimatedReasoningMessage } from "./AnimatedReasoningMessage";
import { AnimatedToolCall } from "./AnimatedToolCall";
import { AnimatedError } from "./AnimatedError";

export function MessageList({ items, activeArtifactKey, onSelectArtifact }:
  { items: AnimatedItem[]; activeArtifactKey?: string | null; onSelectArtifact?: (key: string) => void }) {
  return (
    <div className="px-4">
      {items.map((it, i) => {
        switch (it.kind) {
          case "user":
            return <div key={i} className="my-2 whitespace-pre-wrap" style={{ color: "var(--cli-dim)" }}>
              <span className="mr-2">&gt;</span>{it.text}</div>;
          case "assistant":
            return <AnimatedAssistantMessage key={i} item={it} />;
          case "reasoning":
            return <AnimatedReasoningMessage key={i} item={it} />;
          case "tool": {
            const artifactKey = it.display ? `art-${i}` : undefined;
            return <AnimatedToolCall key={i} item={it} artifactKey={artifactKey}
              active={!!artifactKey && artifactKey === activeArtifactKey} onSelect={onSelectArtifact} />;
          }
          case "context":
            return <div key={i} className="my-1" style={{ color: "var(--cli-dim)" }}>✻ {it.text}</div>;
          case "error":
            return <AnimatedError key={i} item={it} />;
        }
      })}
    </div>
  );
}
