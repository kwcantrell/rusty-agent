import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ArchDetail } from "./ArchDetail";
import { archFixture as fixture } from "./archFixture";

describe("ArchDetail", () => {
  it("tools block renders a table grouped with kind labels", () => {
    render(<ArchDetail snapshot={fixture} block="tools" />);
    expect(screen.getByText("render")).toBeInTheDocument();
    expect(screen.getByText("Render an artifact")).toBeInTheDocument();
    expect(screen.getByText("skills")).toBeInTheDocument(); // kind chip for `use_skill`
  });

  it("policy block marks hard-floor entries", () => {
    render(<ArchDetail snapshot={fixture} block="policy" />);
    expect(screen.getByText("rm -rf /")).toBeInTheDocument();
    expect(screen.getByText("hard floor")).toBeInTheDocument();
    expect(screen.getByText("ls")).toBeInTheDocument();
  });

  it("model block lists backend, host, and sampling", () => {
    render(<ArchDetail snapshot={fixture} block="model" />);
    expect(screen.getByText("http://localhost:8080")).toBeInTheDocument();
    expect(screen.getByText(/0.6/)).toBeInTheDocument();
  });

  it("sandbox block shows degraded reason when present", () => {
    const s = { ...fixture, sandbox: { ...fixture.sandbox, degraded: "no docker daemon" } };
    render(<ArchDetail snapshot={s} block="sandbox" />);
    expect(screen.getByText("no docker daemon")).toBeInTheDocument();
  });

  it("prompt block never renders prompt text, only stats", () => {
    render(<ArchDetail snapshot={fixture} block="prompt" />);
    expect(screen.getByText(/97/)).toBeInTheDocument();
    expect(screen.getByText(/built-in/)).toBeInTheDocument(); // override inactive wording
  });
});
