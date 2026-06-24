import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { Inspector } from "../src/components/inspector/Inspector";
import type { InspectorArtifact } from "../src/state";

const arts: InspectorArtifact[] = [
  { key: "art-1", title: "Plan", display: { Markdown: { text: "# Plan body" } } },
  { key: "art-2", title: "token.rs", display: { Code: { lang: "rust", filename: "token.rs", text: "fn x(){}" } } },
];

describe("Inspector", () => {
  it("shows an empty state when there are no artifacts", () => {
    render(<Inspector artifacts={[]} activeKey={null} onSelect={() => {}} onClose={() => {}} />);
    expect(screen.getByText(/nothing to inspect/i)).toBeInTheDocument();
  });
  it("renders a tab per artifact and shows the active one", () => {
    render(<Inspector artifacts={arts} activeKey="art-1" onSelect={() => {}} onClose={() => {}} />);
    expect(screen.getByRole("tab", { name: "Plan" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "token.rs" })).toBeInTheDocument();
    expect(screen.getByText("Plan body")).toBeInTheDocument();
  });
  it("fires onSelect when a tab is clicked", () => {
    const onSelect = vi.fn();
    render(<Inspector artifacts={arts} activeKey="art-1" onSelect={onSelect} onClose={() => {}} />);
    screen.getByRole("tab", { name: "token.rs" }).click();
    expect(onSelect).toHaveBeenCalledWith("art-2");
  });
});
