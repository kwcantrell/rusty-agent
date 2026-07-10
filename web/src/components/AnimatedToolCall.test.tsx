import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AnimatedToolCall } from "./AnimatedToolCall";
import type { AnimatedItem, SubagentCard } from "../state";

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

const subagentCard = (over: Partial<SubagentCard>): SubagentCard => ({
  subagentType: "researcher",
  status: "running",
  text: "",
  reasoning: "",
  textElided: 0,
  reasoningElided: 0,
  promptTokens: 0,
  completionTokens: 0,
  costUsd: 0,
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
  it("renders a running subagent card with badge, status, and live transcript", () => {
    render(
      <AnimatedToolCall
        item={toolItem({
          status: "running",
          subagent: subagentCard({
            subagentType: "researcher",
            status: "running",
            text: "partial answer",
            reasoning: "",
            textElided: 0,
          }),
        })}
      />
    );
    expect(screen.getByTestId("subagent-card")).toBeInTheDocument();
    expect(screen.getByText("agent[researcher]")).toBeInTheDocument();
    expect(screen.getByText("running")).toBeInTheDocument();
    expect(screen.getByTestId("subagent-transcript")).toHaveTextContent("partial answer");
  });
  it("renders outcome footer with detail, stats, and elision marker when done", () => {
    render(
      <AnimatedToolCall
        item={toolItem({
          status: "done",
          subagent: subagentCard({
            status: "done",
            outcome: "timeout",
            detail: "sub-agent timed out after 5s",
            stop: "stop",
            turns: 2,
            toolCalls: 3,
            durationMs: 5000,
            textElided: 42,
            promptTokens: 100,
            completionTokens: 20,
            costUsd: 0.0123,
          }),
        })}
      />
    );
    // outcome word shows in the status pill, not the footer text
    expect(screen.getByText("timeout")).toBeInTheDocument();
    expect(screen.getByText(/sub-agent timed out after 5s/)).toBeInTheDocument();
    expect(screen.getByText(/2 turns/)).toBeInTheDocument();
    expect(screen.getByText(/3 tools/)).toBeInTheDocument();
    expect(screen.getByText(/5\.0s/)).toBeInTheDocument();
    expect(screen.getByText(/120 tok/)).toBeInTheDocument();
    expect(screen.getByText(/\$0\.0123/)).toBeInTheDocument();
    expect(screen.getByTestId("subagent-transcript")).toHaveTextContent("…(42 chars elided)");
  });
  it("renders no card block when item.subagent is undefined", () => {
    render(<AnimatedToolCall item={toolItem({ subagent: undefined })} />);
    expect(screen.queryByTestId("subagent-card")).not.toBeInTheDocument();
  });
});
