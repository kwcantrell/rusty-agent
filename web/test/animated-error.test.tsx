import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { AnimatedError } from "../src/components/AnimatedError";
import type { AnimatedItem } from "../src/state";

type ErrorItem = Extract<AnimatedItem, { kind: "error" }>;

describe("AnimatedError", () => {
  it("renders error message", () => {
    const item = { kind: "error", message: "something went wrong", ts: Date.now(), streaming: false, progress: 1 } as ErrorItem;
    render(<AnimatedError item={item} />);
    expect(screen.getByText(/✗/)).toBeInTheDocument();
    // message shares a text container with the "✗ " glyph, so match as a substring
    expect(screen.getByText(/something went wrong/)).toBeInTheDocument();
  });

  it("has red border styling", () => {
    const item = { kind: "error", message: "fail", ts: Date.now(), streaming: false, progress: 1 } as ErrorItem;
    const { container } = render(<AnimatedError item={item} />);
    const el = container.firstChild as HTMLElement;
    expect(el.className).toContain("border-red-700");
    expect(el.className).toContain("bg-red-950");
  });
});
