import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { AnimatedToolCall } from "../src/components/AnimatedToolCall";
import type { AnimatedItem } from "../src/state";

type ToolItem = Extract<AnimatedItem, { kind: "tool" }>;

describe("AnimatedToolCall", () => {
  it("renders tool name and running status", () => {
    const item = {
      kind: "tool", name: "read_file", args: { path: "a.txt" }, status: "running",
      ts: Date.now(), streaming: true, progress: 0,
    } as ToolItem;
    render(<AnimatedToolCall item={item} />);
    expect(screen.getByText(/read_file/)).toBeInTheDocument();
    expect(screen.getByText("…")).toBeInTheDocument();
  });

  it("renders tool name and done status", () => {
    const item = {
      kind: "tool", name: "read_file", args: { path: "a.txt" }, status: "done", content: "file contents",
      ts: Date.now(), streaming: false, progress: 1,
    } as ToolItem;
    render(<AnimatedToolCall item={item} />);
    expect(screen.getByText(/read_file/)).toBeInTheDocument();
    expect(screen.getByText("✓")).toBeInTheDocument();
  });

  it("renders diff display", () => {
    const item = {
      kind: "tool", name: "write_file", args: { path: "a.txt" }, status: "done",
      display: { Diff: { path: "a.txt", before: "foo\nbar\n", after: "foo\nbaz\n" } },
      ts: Date.now(), streaming: false, progress: 1,
    } as ToolItem;
    render(<AnimatedToolCall item={item} />);
    expect(screen.getByText(/write_file/)).toBeInTheDocument();
    expect(screen.getByText("a.txt")).toBeInTheDocument();
    expect(screen.getByText(/-\s*bar/)).toBeInTheDocument();
    expect(screen.getByText(/\+\s*baz/)).toBeInTheDocument();
  });

  it("renders terminal display", () => {
    const item = {
      kind: "tool", name: "execute_command", args: { command: "echo hi" }, status: "done",
      display: { Terminal: { command: "echo hi", stdout: "hi\n", stderr: "", exit_code: 0 } },
      ts: Date.now(), streaming: false, progress: 1,
    } as ToolItem;
    render(<AnimatedToolCall item={item} />);
    expect(screen.getByText(/execute_command/)).toBeInTheDocument();
    expect(screen.getByText(/echo hi/)).toBeInTheDocument();
  });

  it("renders raw content when no display", () => {
    const item = {
      kind: "tool", name: "read_file", args: { path: "a.txt" }, status: "done", content: "file contents",
      ts: Date.now(), streaming: false, progress: 1,
    } as ToolItem;
    render(<AnimatedToolCall item={item} />);
    expect(screen.getByText(/read_file/)).toBeInTheDocument();
    expect(screen.getByText("file contents")).toBeInTheDocument();
  });
});
