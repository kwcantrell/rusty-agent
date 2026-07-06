import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ArchDiagram } from "./ArchDiagram";
import { archFixture as fixture } from "./archFixture";

describe("ArchDiagram", () => {
  it("renders all seven blocks", () => {
    render(<ArchDiagram snapshot={fixture} selected={null} onSelect={() => {}} />);
    for (const label of ["Model", "Agent Loop", "Tools", "Policy", "Sandbox", "Context", "Prompt"]) {
      expect(screen.getByRole("button", { name: new RegExp(label) })).toBeInTheDocument();
    }
  });

  it("shows dynamic badges", () => {
    render(<ArchDiagram snapshot={fixture} selected={null} onSelect={() => {}} />);
    expect(screen.getByText("3 tools")).toBeInTheDocument();
    expect(screen.getByText("memory on")).toBeInTheDocument();
    expect(screen.queryByText("degraded")).not.toBeInTheDocument();
    expect(screen.queryByText("override")).not.toBeInTheDocument();
    expect(screen.getByText("subagents")).toBeInTheDocument();
  });

  it("3-tool fixture sub-label has no ellipsis", () => {
    render(<ArchDiagram snapshot={fixture} selected={null} onSelect={() => {}} />);
    expect(screen.getByRole("button", { name: /Tools/ }).textContent).not.toContain("…");
  });

  it("shows degraded and override badges when present", () => {
    const s = { ...fixture,
      sandbox: { ...fixture.sandbox, degraded: "no docker daemon" },
      prompt: { est_tokens: 20, override_active: true, override_chars: 40 } };
    render(<ArchDiagram snapshot={s} selected={null} onSelect={() => {}} />);
    expect(screen.getByText("degraded")).toBeInTheDocument();
    expect(screen.getByText("override")).toBeInTheDocument();
  });

  it("fires onSelect with the block id and marks selection", () => {
    const picked: string[] = [];
    render(<ArchDiagram snapshot={fixture} selected="tools" onSelect={(b) => picked.push(b)} />);
    fireEvent.click(screen.getByRole("button", { name: /Policy/ }));
    expect(picked).toEqual(["policy"]);
    expect(screen.getByRole("button", { name: /Tools/ })).toHaveAttribute("aria-pressed", "true");
  });

  it("arrows sit between rows, not inside them", () => {
    render(<ArchDiagram snapshot={fixture} selected={null} onSelect={() => {}} />);
    const loopBtn = screen.getByRole("button", { name: /Agent Loop/ });
    const arrows = screen.getAllByText("↓");
    expect(arrows).toHaveLength(2);
    for (const a of arrows) expect(a.closest("div")).not.toBe(loopBtn.parentElement);
  });
});
