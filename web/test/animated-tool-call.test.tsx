import { describe, it, expect, vi } from "vitest";
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
    expect(screen.getByText("⏺")).toBeInTheDocument();
  });

  it("renders tool name and done status", () => {
    const item = {
      kind: "tool", name: "read_file", args: { path: "a.txt" }, status: "done", content: "file contents",
      ts: Date.now(), streaming: false, progress: 1,
    } as ToolItem;
    render(<AnimatedToolCall item={item} />);
    expect(screen.getByText(/read_file/)).toBeInTheDocument();
    expect(screen.getByText("⏺")).toBeInTheDocument();
    expect(screen.getByText(/⎿/)).toBeInTheDocument();
  });

  it("shows a failure badge with status and duration for a non-ok result", () => {
    const item = {
      kind: "tool", name: "read_file", args: { path: "a.txt" }, status: "done",
      content: "ERROR: …", resultStatus: "timeout", durationMs: 60000,
      ts: Date.now(), streaming: false, progress: 1,
    } as ToolItem;
    render(<AnimatedToolCall item={item} />);
    expect(screen.getByText(/timeout · 60000ms/)).toBeInTheDocument();
  });

  it("shows no failure badge for an ok result", () => {
    const item = {
      kind: "tool", name: "read_file", args: { path: "a.txt" }, status: "done",
      content: "ok", resultStatus: "ok", durationMs: 5,
      ts: Date.now(), streaming: false, progress: 1,
    } as ToolItem;
    render(<AnimatedToolCall item={item} />);
    expect(screen.queryByText(/ok · 5ms/)).not.toBeInTheDocument();
  });

  it("is a clickable chip that focuses its artifact, without rendering output inline", () => {
    const onSelect = vi.fn();
    const item = {
      kind: "tool", name: "write_file", args: { path: "a.txt" }, status: "done",
      display: { Diff: { path: "a.txt", before: "foo\nbar\n", after: "foo\nbaz\n" } },
      ts: Date.now(), streaming: false, progress: 1,
    } as ToolItem;
    render(<AnimatedToolCall item={item} artifactKey="art-3" active={false} onSelect={onSelect} />);
    expect(screen.getByText(/write_file/)).toBeInTheDocument();
    // diff content is shown in the Inspector, not inline:
    expect(screen.queryByText(/-\s*bar/)).not.toBeInTheDocument();
    screen.getByText("view →").click();
    expect(onSelect).toHaveBeenCalledWith("art-3");
  });

  it("shows a 'viewing' affordance when its artifact is active", () => {
    const item = {
      kind: "tool", name: "execute_command", args: { command: "echo hi" }, status: "done",
      display: { Terminal: { command: "echo hi", stdout: "hi\n", stderr: "", exit_code: 0 } },
      ts: Date.now(), streaming: false, progress: 1,
    } as ToolItem;
    render(<AnimatedToolCall item={item} artifactKey="art-2" active={true} onSelect={() => {}} />);
    expect(screen.getByText(/execute_command/)).toBeInTheDocument();
    expect(screen.getByText(/viewing/)).toBeInTheDocument();
  });

  it("renders a non-clickable chip when there is no artifact", () => {
    const item = {
      kind: "tool", name: "read_file", args: { path: "a.txt" }, status: "done", content: "file contents",
      ts: Date.now(), streaming: false, progress: 1,
    } as ToolItem;
    render(<AnimatedToolCall item={item} />);
    expect(screen.getByText(/read_file/)).toBeInTheDocument();
    // raw content is not dumped into the conversation:
    expect(screen.queryByText("file contents")).not.toBeInTheDocument();
    // view → button is not shown without an artifact key
    expect(screen.queryByText("view →")).not.toBeInTheDocument();
  });
});
