import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { AnimatedReasoningMessage } from "../src/components/AnimatedReasoningMessage";
import type { AnimatedItem } from "../src/state";

type ReasoningItem = Extract<AnimatedItem, { kind: "reasoning" }>;

describe("AnimatedReasoningMessage", () => {
  it("renders reasoning text once expanded", () => {
    // Collapsed by default (like the existing ReasoningMessage), so expand first.
    const item = { kind: "reasoning", text: "let me think about this", ts: Date.now(), streaming: false, progress: 1 } as ReasoningItem;
    render(<AnimatedReasoningMessage item={item} />);
    fireEvent.click(screen.getByText(/✻ Thinking…/));
    expect(screen.getByText("let me think about this")).toBeInTheDocument();
  });

  it("collapses by default", () => {
    const item = { kind: "reasoning", text: "hidden reasoning", ts: Date.now(), streaming: false, progress: 1 } as ReasoningItem;
    render(<AnimatedReasoningMessage item={item} />);
    expect(screen.queryByText("hidden reasoning")).not.toBeInTheDocument();
  });

  it("expands when clicked", () => {
    const item = { kind: "reasoning", text: "visible reasoning", ts: Date.now(), streaming: false, progress: 1 } as ReasoningItem;
    render(<AnimatedReasoningMessage item={item} />);
    fireEvent.click(screen.getByText(/✻ Thinking…/));
    expect(screen.getByText("visible reasoning")).toBeInTheDocument();
  });

  it("shows the Thinking label", () => {
    const item = { kind: "reasoning", text: "test", ts: Date.now(), streaming: false, progress: 1 } as ReasoningItem;
    render(<AnimatedReasoningMessage item={item} />);
    expect(screen.getByText(/✻ Thinking…/)).toBeInTheDocument();
  });
});
