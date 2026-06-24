import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { TimelineView } from "../src/components/TimelineView";
import type { TurnGroup, AnimatedItem } from "../src/state";

function makeTurn(items: AnimatedItem[]): TurnGroup {
  return {
    items,
    startTs: items[0].ts,
    endTs: items[items.length - 1].ts,
    duration: items[items.length - 1].ts - items[0].ts,
  };
}

describe("TimelineView", () => {
  it("renders user message pills", () => {
    const now = Date.now();
    const turns: TurnGroup[] = [
      makeTurn([
        { kind: "user", text: "hello", ts: now, streaming: false, progress: 1 } as AnimatedItem,
        { kind: "assistant", text: "hi", ts: now + 100, streaming: false, progress: 1, done: "stop" } as AnimatedItem,
      ]),
    ];
    render(<TimelineView turns={turns} messageListRef={{ current: null }} />);
    expect(screen.getByText("hello")).toBeInTheDocument();
  });

  it("renders thinking bar", () => {
    const now = Date.now();
    const turns: TurnGroup[] = [
      makeTurn([
        { kind: "user", text: "q", ts: now, streaming: false, progress: 1 } as AnimatedItem,
        { kind: "reasoning", text: "thinking", ts: now + 50, streaming: false, progress: 1 } as AnimatedItem,
        { kind: "assistant", text: "a", ts: now + 100, streaming: false, progress: 1, done: "stop" } as AnimatedItem,
      ]),
    ];
    render(<TimelineView turns={turns} messageListRef={{ current: null }} />);
    // The thinking bar carries the reasoning text in its title (tooltip), not as visible text.
    expect(screen.getByTitle("thinking")).toBeInTheDocument();
  });

  it("renders tool call bars", () => {
    const now = Date.now();
    const turns: TurnGroup[] = [
      makeTurn([
        { kind: "user", text: "run", ts: now, streaming: false, progress: 1 } as AnimatedItem,
        { kind: "tool", name: "read_file", args: {}, status: "done", ts: now + 50, streaming: false, progress: 1 } as AnimatedItem,
        { kind: "assistant", text: "done", ts: now + 100, streaming: false, progress: 1, done: "stop" } as AnimatedItem,
      ]),
    ];
    render(<TimelineView turns={turns} messageListRef={{ current: null }} />);
    // The tool bar carries the tool name in its title (tooltip).
    expect(screen.getByTitle("read_file")).toBeInTheDocument();
  });

  it("renders done dot", () => {
    const now = Date.now();
    const turns: TurnGroup[] = [
      makeTurn([
        { kind: "user", text: "q", ts: now, streaming: false, progress: 1 } as AnimatedItem,
        { kind: "assistant", text: "a", ts: now + 100, streaming: false, progress: 1, done: "stop" } as AnimatedItem,
      ]),
    ];
    render(<TimelineView turns={turns} messageListRef={{ current: null }} />);
    expect(screen.getByTitle("Done: stop")).toBeInTheDocument();
  });

  it("renders multiple turns", () => {
    const now = Date.now();
    const turns: TurnGroup[] = [
      makeTurn([
        { kind: "user", text: "q1", ts: now, streaming: false, progress: 1 } as AnimatedItem,
        { kind: "assistant", text: "a1", ts: now + 100, streaming: false, progress: 1, done: "stop" } as AnimatedItem,
      ]),
      makeTurn([
        { kind: "user", text: "q2", ts: now + 200, streaming: false, progress: 1 } as AnimatedItem,
        { kind: "assistant", text: "a2", ts: now + 300, streaming: false, progress: 1, done: "stop" } as AnimatedItem,
      ]),
    ];
    render(<TimelineView turns={turns} messageListRef={{ current: null }} />);
    expect(screen.getByText("q1")).toBeInTheDocument();
    expect(screen.getByText("q2")).toBeInTheDocument();
  });

  it("renders nothing when no turns", () => {
    const { container } = render(<TimelineView turns={[]} messageListRef={{ current: null }} />);
    expect(container.firstChild).toBeNull();
  });
});
