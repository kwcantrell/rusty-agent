import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ContextDashboard } from "../src/components/ContextDashboard";
import type { RuntimeSettings } from "../src/wire";

const settings = { model: "qwen3", temperature: 0.7, active_skills: ["search", "files"] } as unknown as RuntimeSettings;
const usage = { promptTokens: 12400, contextLimit: 128000, turn: 3, maxTurns: 20 };

describe("ContextDashboard", () => {
  beforeEach(() => localStorage.clear());

  it("renders the collapsed gauge with a formatted figure and percent", () => {
    render(<ContextDashboard usage={usage} settings={settings} toolCount={5} artifactCount={2} stats={null} />);
    expect(screen.getByText(/12\.4k\s*\/\s*128k/)).toBeInTheDocument();
    expect(screen.getByText(/10%/)).toBeInTheDocument(); // 12400/128000 ≈ 10%
  });

  it("shows a muted placeholder when usage is null", () => {
    render(<ContextDashboard usage={null} settings={settings} toolCount={0} artifactCount={0} stats={null} />);
    expect(screen.getByText(/—/)).toBeInTheDocument();
  });

  it("expands to reveal config and session stats", () => {
    render(<ContextDashboard usage={usage} settings={settings} toolCount={5} artifactCount={2} stats={null} />);
    fireEvent.click(screen.getByRole("button", { name: /context/i }));
    expect(screen.getByText(/qwen3/)).toBeInTheDocument();
    expect(screen.getByText(/turns 3\/20/)).toBeInTheDocument();
    expect(screen.getByText(/5 tools/)).toBeInTheDocument();
    expect(screen.getByText(/2 art/)).toBeInTheDocument();
    expect(screen.getByText(/search, files/)).toBeInTheDocument();
  });

  it("shows the session stats panel when expanded and stats exist", () => {
    const stats = { turns: 2, prompt_tokens: 300, completion_tokens: 90,
      reasoning_tokens: 0, cached_tokens: 0, cost_usd: 0, tool_calls: 3,
      tools_ok: 3, tools_denied: 0, tools_error: 0, tools_timeout: 0, tools_panic: 0,
      tool_time_ms: 900, turn_time_ms: 1200, context_events: 1, errors: 0 };
    render(<ContextDashboard usage={usage} settings={settings} toolCount={5} artifactCount={2} stats={stats} />);
    expect(screen.queryByLabelText("Session stats")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /context/i }));
    expect(screen.getByLabelText("Session stats")).toBeInTheDocument();
    expect(screen.getByText("3 (0 failed)")).toBeInTheDocument();
  });
});
