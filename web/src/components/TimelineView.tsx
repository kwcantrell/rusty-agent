import { useRef, useState } from "react";
import type { AnimatedItem, TurnGroup } from "../state";

interface Props {
  turns: TurnGroup[];
  onTurnClick?: (index: number) => void;
  messageListRef: React.RefObject<HTMLDivElement | null>;
}

type UserItem = Extract<AnimatedItem, { kind: "user" }>;
type ReasoningItem = Extract<AnimatedItem, { kind: "reasoning" }>;
type ToolItem = Extract<AnimatedItem, { kind: "tool" }>;
type AssistantItem = Extract<AnimatedItem, { kind: "assistant" }>;

export function TimelineView({ turns, onTurnClick, messageListRef }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [hoveredTurn, setHoveredTurn] = useState<number | null>(null);

  if (turns.length === 0) return null;

  const handleTurnClick = (index: number) => {
    onTurnClick?.(index);
    if (messageListRef.current) {
      messageListRef.current.scrollIntoView({ behavior: "smooth", block: "nearest" });
    }
  };

  return (
    <div className="relative border-t border-zinc-800 bg-zinc-900/80">
      <div className="overflow-x-auto overflow-y-hidden px-4">
        <div ref={scrollRef} className="flex h-12 items-center gap-2 whitespace-nowrap">
          {turns.map((turn, turnIndex) => {
            const isHovered = hoveredTurn === turnIndex;
            const users = turn.items.filter((i): i is UserItem => i.kind === "user");
            const reasonings = turn.items.filter((i): i is ReasoningItem => i.kind === "reasoning");
            const tools = turn.items.filter((i): i is ToolItem => i.kind === "tool");
            const dones = turn.items.filter((i): i is AssistantItem => i.kind === "assistant" && i.done !== undefined);
            return (
              <div key={turnIndex} className="flex items-center" onClick={() => handleTurnClick(turnIndex)}>
                {turnIndex > 0 && <div className="mx-1 h-px w-3 bg-zinc-700" />}

                {/* User message pill */}
                {users.map((userItem, ui) => (
                  <div
                    key={`user-${ui}`}
                    className={`rounded-full border border-zinc-700 bg-zinc-800 px-2 py-0.5 text-xs text-zinc-200 transition-colors hover:border-zinc-500 ${isHovered ? "cursor-pointer" : ""}`}
                    onMouseEnter={() => setHoveredTurn(turnIndex)}
                    onMouseLeave={() => setHoveredTurn(null)}
                    title={userItem.text}
                  >
                    {userItem.text.length > 30 ? userItem.text.slice(0, 30) + "…" : userItem.text}
                  </div>
                ))}

                {/* Thinking bar */}
                {reasonings.map((reasoningItem, ri) => (
                  <div
                    key={`reasoning-${ri}`}
                    className="mx-1 flex h-3 items-center"
                    title={reasoningItem.text.length > 50 ? reasoningItem.text.slice(0, 50) + "…" : reasoningItem.text}
                    onMouseEnter={() => setHoveredTurn(turnIndex)}
                    onMouseLeave={() => setHoveredTurn(null)}
                  >
                    <div className="h-1 w-12 rounded-full bg-purple-500/60" />
                  </div>
                ))}

                {/* Tool call bars */}
                {tools.map((toolItem, ti) => {
                  const isRunning = toolItem.status === "running";
                  return (
                    <div
                      key={`tool-${ti}`}
                      className="mx-1 flex h-3 items-center"
                      title={toolItem.name}
                      onMouseEnter={() => setHoveredTurn(turnIndex)}
                      onMouseLeave={() => setHoveredTurn(null)}
                    >
                      <div className={`h-1 w-16 rounded-full ${isRunning ? "bg-amber-400/80" : "bg-green-400/80"}`} />
                    </div>
                  );
                })}

                {/* Done dot */}
                {dones.map((doneItem, di) => (
                  <div key={`done-${di}`} className="mx-1 flex h-3 items-center" title={`Done: ${doneItem.done}`}>
                    <div className="h-2 w-2 rounded-full bg-green-400" />
                  </div>
                ))}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
