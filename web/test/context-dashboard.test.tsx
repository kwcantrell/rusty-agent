import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ContextDashboard } from "../src/components/ContextDashboard";
import type { RuntimeSettings } from "../src/wire";

const settings = { model: "qwen3", temperature: 0.7, active_skills: ["search", "files"] } as unknown as RuntimeSettings;
const usage = { promptTokens: 12400, contextLimit: 128000, turn: 3, maxTurns: 20 };

describe("ContextDashboard", () => {
  beforeEach(() => localStorage.clear());

  it("renders the collapsed gauge with a formatted figure and percent", () => {
    render(<ContextDashboard usage={usage} settings={settings} toolCount={5} artifactCount={2} />);
    expect(screen.getByText(/12\.4k\s*\/\s*128k/)).toBeInTheDocument();
    expect(screen.getByText(/10%/)).toBeInTheDocument(); // 12400/128000 ≈ 10%
  });

  it("shows a muted placeholder when usage is null", () => {
    render(<ContextDashboard usage={null} settings={settings} toolCount={0} artifactCount={0} />);
    expect(screen.getByText(/—/)).toBeInTheDocument();
  });

  it("expands to reveal config and session stats", () => {
    render(<ContextDashboard usage={usage} settings={settings} toolCount={5} artifactCount={2} />);
    fireEvent.click(screen.getByRole("button", { name: /context/i }));
    expect(screen.getByText(/qwen3/)).toBeInTheDocument();
    expect(screen.getByText(/turns 3\/20/)).toBeInTheDocument();
    expect(screen.getByText(/5 tools/)).toBeInTheDocument();
    expect(screen.getByText(/2 art/)).toBeInTheDocument();
    expect(screen.getByText(/search, files/)).toBeInTheDocument();
  });
});
