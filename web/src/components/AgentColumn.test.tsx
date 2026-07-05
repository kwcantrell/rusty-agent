import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { AgentColumn } from "./AgentColumn";

const base = {
  items: [], activeArtifactKey: null, onSelectArtifact: () => {},
  projectLabel: "studio-x", model: "qwen3", pendingApproval: null,
  onDecide: () => {}, composerDisabled: false, onSend: vi.fn(),
  usage: null as null | { promptTokens: number; contextLimit: number; turn: number; maxTurns: number },
  settings: null, toolCount: 0, artifactCount: 0, stats: null,
};

describe("AgentColumn", () => {
  it("renders the session banner (project + model)", () => {
    render(<AgentColumn {...base} />);
    expect(screen.getByText(/studio-x · qwen3/)).toBeInTheDocument();
  });
  it("disables the composer when asked", () => {
    render(<AgentColumn {...base} composerDisabled />);
    expect(screen.getByPlaceholderText(/disconnected/)).toBeDisabled();
  });
  it("sends a message", () => {
    const onSend = vi.fn();
    render(<AgentColumn {...base} onSend={onSend} />);
    const ta = screen.getByPlaceholderText(/Message the agent/);
    fireEvent.change(ta, { target: { value: "hello" } });
    fireEvent.keyDown(ta, { key: "Enter" });
    expect(onSend).toHaveBeenCalledWith("hello");
  });
  it("renders the context dashboard gauge above the composer", () => {
    render(<AgentColumn {...base} usage={{ promptTokens: 4000, contextLimit: 8000, turn: 1, maxTurns: 20 }} />);
    expect(screen.getByLabelText("context usage")).toBeInTheDocument();
    expect(screen.getByText(/4k\s*\/\s*8k/)).toBeInTheDocument();
    expect(screen.getByText(/50%/)).toBeInTheDocument();
  });
});
