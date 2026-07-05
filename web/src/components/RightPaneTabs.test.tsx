import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { RightPaneTabs } from "./RightPaneTabs";

describe("RightPaneTabs", () => {
  it("renders Workspace, Context, and Design tabs", () => {
    render(<RightPaneTabs rightTab="workspace" setRightTab={() => {}} />);
    expect(screen.getByRole("tab", { name: "Workspace" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Context" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Design" })).toBeInTheDocument();
  });

  it("selects Design on click", () => {
    const picked: string[] = [];
    render(<RightPaneTabs rightTab="workspace" setRightTab={(t) => picked.push(t)} />);
    fireEvent.click(screen.getByRole("tab", { name: "Design" }));
    expect(picked).toEqual(["design"]);
    expect(screen.getByRole("tab", { name: "Workspace" })).toHaveAttribute("aria-selected", "true");
  });
});
