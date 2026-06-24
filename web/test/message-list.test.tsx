import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { MessageList } from "../src/components/MessageList";
import type { AnimatedItem } from "../src/state";

function makeAnimated(kind: string, props: Record<string, unknown>): AnimatedItem {
  return { kind, ts: Date.now(), streaming: false, progress: 1, ...props } as AnimatedItem;
}

describe("MessageList", () => {
  it("renders user items", () => {
    const items: AnimatedItem[] = [makeAnimated("user", { text: "hello" })];
    render(<MessageList items={items} />);
    expect(screen.getByText("hello")).toBeInTheDocument();
  });

  it("renders assistant items with animated component", () => {
    const items: AnimatedItem[] = [makeAnimated("assistant", { text: "hi there" })];
    render(<MessageList items={items} />);
    expect(screen.getByText("hi there")).toBeInTheDocument();
  });

  it("renders reasoning items with animated component", () => {
    const items: AnimatedItem[] = [makeAnimated("reasoning", { text: "thinking" })];
    render(<MessageList items={items} />);
    expect(screen.getByText("▸ Thinking")).toBeInTheDocument();
  });

  it("renders tool items with animated component", () => {
    const items: AnimatedItem[] = [makeAnimated("tool", { name: "read_file", args: {}, status: "running" })];
    render(<MessageList items={items} />);
    expect(screen.getByText(/read_file/)).toBeInTheDocument();
  });

  it("renders error items with animated component", () => {
    const items: AnimatedItem[] = [makeAnimated("error", { message: "fail" })];
    render(<MessageList items={items} />);
    expect(screen.getByText(/✗/)).toBeInTheDocument();
    // message shares a text container with the "✗ " glyph, so match as a substring
    expect(screen.getByText(/fail/)).toBeInTheDocument();
  });
});
