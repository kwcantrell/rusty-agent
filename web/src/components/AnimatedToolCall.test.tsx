import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AnimatedToolCall } from "./AnimatedToolCall";
import type { AnimatedItem } from "../state";

type ToolItem = Extract<AnimatedItem, { kind: "tool" }>;

const toolItem = (over: Partial<ToolItem>): ToolItem => ({
  kind: "tool",
  id: "c1",
  name: "read_file",
  args: {},
  status: "done",
  resultStatus: "ok",
  ts: 0,
  streaming: false,
  progress: 1,
  ...over,
});

describe("AnimatedToolCall", () => {
  it("renders a nested child row: ↳ marker and the sub: prefix stripped", () => {
    render(<AnimatedToolCall item={toolItem({ parentId: "d1", name: "sub:read_file" })} />);
    expect(screen.getByText("↳")).toBeInTheDocument();
    expect(screen.getByText("read_file")).toBeInTheDocument();
    expect(screen.queryByText("sub:read_file")).not.toBeInTheDocument();
  });
  it("renders a top-level row: no ↳ marker and the full name is shown", () => {
    render(<AnimatedToolCall item={toolItem({ parentId: undefined, name: "sub:read_file" })} />);
    expect(screen.queryByText("↳")).not.toBeInTheDocument();
    expect(screen.getByText("sub:read_file")).toBeInTheDocument();
  });
  it("shows the arg summary in the header", () => {
    render(<AnimatedToolCall item={toolItem({ name: "Bash", args: { command: "npm test" } })} />);
    expect(screen.getByText("(npm test)")).toBeInTheDocument();
  });
  it("renders a ⎿ result summary and expands raw content on click", () => {
    render(<AnimatedToolCall item={toolItem({ content: "42 passed\n0 failed" })} />);
    const summary = screen.getByText(/⎿ 42 passed \(\+1 lines\)/);
    expect(screen.queryByText(/0 failed/)).not.toBeInTheDocument();
    fireEvent.click(summary);
    expect(screen.getByText(/0 failed/)).toBeInTheDocument();
  });
  it("shows no ⎿ line while running", () => {
    render(<AnimatedToolCall item={toolItem({ status: "running" })} />);
    expect(screen.queryByText(/⎿/)).not.toBeInTheDocument();
  });
  it("marks failed results with the resultStatus and duration", () => {
    render(<AnimatedToolCall item={toolItem({ resultStatus: "error", durationMs: 42, content: "boom" })} />);
    expect(screen.getByText(/boom · error · 42ms/)).toBeInTheDocument();
  });
  it("calls onSelect from the view → affordance", () => {
    const onSelect = vi.fn();
    render(<AnimatedToolCall item={toolItem({})} artifactKey="art-1" onSelect={onSelect} />);
    fireEvent.click(screen.getByText("view →"));
    expect(onSelect).toHaveBeenCalledWith("art-1");
  });
});
