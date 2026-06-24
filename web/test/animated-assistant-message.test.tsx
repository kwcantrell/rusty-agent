import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { AnimatedAssistantMessage } from "../src/components/AnimatedAssistantMessage";
import type { AnimatedItem } from "../src/state";

type AssistantItem = Extract<AnimatedItem, { kind: "assistant" }>;

describe("AnimatedAssistantMessage", () => {
  it("renders assistant text", () => {
    const item = { kind: "assistant", text: "Hello world", ts: Date.now(), streaming: false, progress: 1 } as AssistantItem;
    render(<AnimatedAssistantMessage item={item} />);
    expect(screen.getByText("Hello world")).toBeInTheDocument();
  });

  it("renders markdown headings", () => {
    const item = { kind: "assistant", text: "# Heading", ts: Date.now(), streaming: false, progress: 1 } as AssistantItem;
    render(<AnimatedAssistantMessage item={item} />);
    expect(screen.getByText("Heading")).toBeInTheDocument();
  });

  it("renders code blocks with syntax highlighting", () => {
    // rehype-highlight tokenizes code into <span> nodes, so assert on the pre's full
    // textContent and the highlight.js class rather than a single-text-node getByText.
    const item = { kind: "assistant", text: "```js\nconsole.log('hi');\n```", ts: Date.now(), streaming: false, progress: 1 } as AssistantItem;
    const { container } = render(<AnimatedAssistantMessage item={item} />);
    const pre = container.querySelector("pre");
    expect(pre?.textContent).toContain("console.log('hi');");
    expect(pre?.querySelector("code")?.className).toMatch(/hljs|language-/);
  });

  it("renders inline code", () => {
    const item = { kind: "assistant", text: "Use `console.log` to debug", ts: Date.now(), streaming: false, progress: 1 } as AssistantItem;
    render(<AnimatedAssistantMessage item={item} />);
    expect(screen.getByText(/Use/)).toBeInTheDocument();
    expect(screen.getByText(/console\.log/)).toBeInTheDocument();
  });

  it("shows done reason when present", () => {
    const item = { kind: "assistant", text: "done", done: "stop", ts: Date.now(), streaming: false, progress: 1 } as AssistantItem;
    render(<AnimatedAssistantMessage item={item} />);
    expect(screen.getByText("done")).toBeInTheDocument();
  });
});
