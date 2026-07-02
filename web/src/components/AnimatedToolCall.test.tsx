import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
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
});
