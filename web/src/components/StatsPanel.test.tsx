import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { StatsPanel } from "./StatsPanel";

describe("StatsPanel", () => {
  it("renders nothing without stats", () => {
    const { container } = render(<StatsPanel stats={null} />);
    expect(container.firstChild).toBeNull();
  });
  it("shows failure count and cost", () => {
    render(<StatsPanel stats={{ turns: 2, prompt_tokens: 300, completion_tokens: 90,
      reasoning_tokens: 0, cached_tokens: 0, cost_usd: 0.05, tool_calls: 3,
      tools_ok: 2, tools_denied: 0, tools_error: 1, tools_timeout: 0, tools_panic: 0,
      tool_time_ms: 900, turn_time_ms: 1200, context_events: 1, errors: 1 }} />);
    expect(screen.getByText("3 (1 failed)")).toBeInTheDocument();
    expect(screen.getByText("$0.0500")).toBeInTheDocument();
  });
});
